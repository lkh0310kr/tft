//! The workspace shell: docks, tab groups, the reading/editor area, overlays and popouts.
//! Commands run by id; hotkeys come from `hotkeys.json`; the vault is indexed in the background.

use crate::assets::{icon, logo};
use crate::canvas::{CanvasEvent, CanvasView};
use crate::chooser::{Chooser, ChooserEvent, Row as ChooserRow};
use crate::copy_results::{CopyResults, CopyResultsEvent};
use crate::debug_info::{DebugInfo, DebugInfoEvent};
use crate::editor::{Editor, EditorEvent, IncomingAttachments};
use crate::explorer::{
    self, display_name, kind_name, ExplorerEvent, FileExplorer, FileTree, SortOrder,
};
use crate::export_modals::{ImageExport, ImageExportEvent, PdfExport, PdfExportEvent};
use crate::file_menu::{self, Pick};
use crate::fold;
use crate::graph::{GraphEvent, GraphView};
use crate::inline_field;
use crate::pdf::view::{PdfEvent, PdfView};
use crate::popout::PopoutWindow;
use crate::prompt::{Prompt, PromptAction, PromptButton, PromptEvent, PromptSpec};
use crate::reading::{
    self, LinkTarget, PropertyItemRemoved, ReadingEvent, ReadingView, TaskMarker, TaskToggled,
};
use crate::search::{CopyEntry, SearchEvent, SearchPane};
use crate::settings::{self, SettingChange, SettingsEvent, SettingsView, SettingsWindow};
use crate::switcher::{QuickSwitcher, SwitcherEvent};
use crate::theme::{
    Theme, FONT_UI_SMALL, FONT_UI_SMALLER, HEADER_HEIGHT, ICON_SIZE, RIBBON_WIDTH, SIDEBAR_WIDTH,
    TAB_CURVE,
};
use crate::vaults::VaultManager;
use crate::watch;
use crate::zoom;
use gpui::{
    actions, anchored, canvas, deferred, div, point, prelude::*, px, relative, rgb, rgba,
    transparent_black, Action, Anchor, App, Bounds, BoxShadow, ClipboardItem, Context, CursorStyle,
    Decorations, DragMoveEvent, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, Global, HitboxBehavior, Hsla, KeyBinding, MouseButton, MouseDownEvent, PathBuilder,
    Pixels, ResizeEdge, SharedString, Subscription, Task, Tiling, UTF16Selection, Window,
    WindowControlArea,
};
use rayon::prelude::*;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};
use tft_cache::{basename, index_markdown, CachedMetadata, LinkFormat, MetadataCache};
use tft_commands::{hotkey_label, keystroke, CommandRegistry};
use tft_formats::config::{
    AppConfig, AppearanceConfig, PagePreviewConfig, PropertyTypes, SwitcherConfig,
};
use tft_formats::properties::PropertiesInDocument;
use tft_formats::{validate_renamed_file, FileKind};
use tft_markdown::Subpath;
use tft_search::ExcludeFilters;
use tft_text::table::TableCommand;
use tft_text::{BlockInsert, LinePrefix, LinkSyntax};
use tft_vault::{FileChange, TrashOption, Vault, VaultError, VaultRegistry};

// Files from outside the vault: insert attachment, download attachments, open file.
mod attachments;
mod download_modal;

// The app's own commands: debug info and release notes.
mod app_info;
mod release_notes;

// Frontmatter migration.
mod migrate_frontmatter;

// Export to PDF and Export as image.
mod exports;

actions!(
    workspace,
    [ToggleMode, GoBack, GoForward, OpenSearch, OpenGraph]
);

actions!(
    dock_tree,
    [TreeUp, TreeDown, TreeLeft, TreeRight, TreeActivate]
);

/// Runs the command with this Obsidian id (bound from the registry's hotkeys).
#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct RunCommand {
    pub id: String,
}

/// The command registry with the vault's `hotkeys.json` applied.
pub struct Commands(pub Arc<CommandRegistry>);

impl Global for Commands {}

/// The local civil time, which is what Obsidian's dates are in.
fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The ids that need an editor on screen with the keyboard in it (Obsidian's
/// `editorCallback`); [`Workspace::editing_or_report`] guards the hotkey path too.
pub const EDITOR_ONLY: &[&str] = &[
    "editor:add-cursor-above",
    "editor:add-cursor-below",
    "editor:clear-formatting",
    "editor:context-menu",
    "editor:cycle-list-checklist",
    "editor:delete-paragraph",
    "editor:fold-all",
    "editor:fold-less",
    "editor:fold-more",
    "editor:follow-link",
    "editor:indent-list",
    "editor:insert-callout",
    "editor:insert-codeblock",
    "editor:insert-embed",
    "editor:insert-footnote",
    "editor:insert-horizontal-rule",
    "editor:insert-link",
    "editor:insert-mathblock",
    "editor:insert-table",
    "editor:insert-tag",
    "editor:insert-wikilink",
    "editor:open-link-in-new-leaf",
    "editor:open-link-in-new-split",
    "editor:open-link-in-new-window",
    "editor:rename-heading",
    "editor:set-heading",
    "editor:set-heading-0",
    "editor:set-heading-1",
    "editor:set-heading-2",
    "editor:set-heading-3",
    "editor:set-heading-4",
    "editor:set-heading-5",
    "editor:set-heading-6",
    "editor:swap-line-down",
    "editor:swap-line-up",
    "editor:toggle-blockquote",
    "editor:toggle-bold",
    "editor:toggle-bullet-list",
    "editor:toggle-checklist-status",
    "editor:toggle-code",
    "editor:toggle-comments",
    "editor:toggle-fold",
    "editor:toggle-highlight",
    "editor:toggle-highlight-blue",
    "editor:toggle-highlight-green",
    "editor:toggle-highlight-orange",
    "editor:toggle-highlight-purple",
    "editor:toggle-highlight-red",
    "editor:toggle-highlight-yellow",
    "editor:toggle-inline-math",
    "editor:toggle-italics",
    "editor:toggle-numbered-list",
    "editor:toggle-strikethrough",
    "editor:unfold-all",
    "editor:unindent-list",
    "markdown:add-alias",
];

/// The ids that act on the active file, offered only while one is open
/// (Obsidian 1.15.0's empty-tab palette listed none of them).
pub const ACTIVE_FILE_ONLY: &[&str] = &[
    "workspace:copy-path",
    "workspace:copy-full-path",
    "workspace:copy-url",
    "open-with-default-app:open",
    "open-with-default-app:show",
];

/// Command ids the workspace can run; the palette lists only these, as Obsidian hides failed checks.
pub const RUNNABLE: &[&str] = &[
    "editor:save-file",
    "editor:toggle-source",
    "editor:open-search",
    "editor:open-search-replace",
    "editor:add-cursor-above",
    "editor:add-cursor-below",
    "editor:toggle-bold",
    "editor:toggle-italics",
    "editor:toggle-strikethrough",
    "editor:toggle-highlight",
    "editor:toggle-code",
    "editor:toggle-inline-math",
    "editor:toggle-comments",
    "editor:toggle-blockquote",
    "editor:toggle-bullet-list",
    "editor:toggle-numbered-list",
    "editor:toggle-checklist-status",
    "editor:cycle-list-checklist",
    "editor:toggle-highlight-red",
    "editor:toggle-highlight-orange",
    "editor:toggle-highlight-yellow",
    "editor:toggle-highlight-green",
    "editor:toggle-highlight-blue",
    "editor:toggle-highlight-purple",
    "editor:clear-formatting",
    "editor:insert-callout",
    "editor:insert-codeblock",
    "editor:insert-mathblock",
    "editor:insert-horizontal-rule",
    "editor:insert-tag",
    "editor:insert-footnote",
    "editor:set-heading",
    "editor:toggle-fold",
    "editor:fold-all",
    "editor:unfold-all",
    "editor:fold-more",
    "editor:fold-less",
    "editor:set-heading-0",
    "editor:set-heading-1",
    "editor:set-heading-2",
    "editor:set-heading-3",
    "editor:set-heading-4",
    "editor:set-heading-5",
    "editor:set-heading-6",
    "editor:indent-list",
    "editor:unindent-list",
    "editor:delete-paragraph",
    "editor:swap-line-up",
    "editor:swap-line-down",
    "editor:insert-link",
    "editor:insert-wikilink",
    "editor:insert-embed",
    "editor:follow-link",
    "editor:open-link-in-new-leaf",
    "switcher:open",
    "global-search:open",
    "graph:open",
    "graph:open-local",
    "graph:animate",
    "canvas:new-file",
    "canvas:jump-to-group",
    "canvas:convert-to-file",
    "markdown:toggle-preview",
    "markdown:add-metadata-property",
    "markdown:add-alias",
    "markdown:clear-metadata-properties",
    "editor:toggle-fold-properties",
    "editor:context-menu",
    "editor:focus",
    "editor:insert-table",
    "editor:rename-heading",
    "editor:table-row-before",
    "editor:table-row-after",
    "editor:table-row-up",
    "editor:table-row-down",
    "editor:table-row-copy",
    "editor:table-row-delete",
    "editor:table-col-before",
    "editor:table-col-after",
    "editor:table-col-left",
    "editor:table-col-right",
    "editor:table-col-copy",
    "editor:table-col-delete",
    "editor:table-col-align-left",
    "editor:table-col-align-center",
    "editor:table-col-align-right",
    "app:go-back",
    "app:go-forward",
    "app:open-settings",
    "app:open-vault",
    "app:switch-vault",
    "app:open-another-vault",
    "app:open-sandbox-vault",
    "app:toggle-left-sidebar",
    "file-explorer:open",
    "file-explorer:reveal-active-file",
    "file-explorer:new-file",
    "file-explorer:new-file-in-current-tab",
    "file-explorer:new-file-in-new-pane",
    "file-explorer:new-folder",
    "file-explorer:move-file",
    "file-explorer:duplicate-file",
    "workspace:edit-file-title",
    "workspace:new-tab",
    "workspace:close",
    "workspace:close-others",
    "workspace:toggle-pin",
    "workspace:next-tab",
    "workspace:previous-tab",
    "workspace:goto-tab-1",
    "workspace:goto-tab-2",
    "workspace:goto-tab-3",
    "workspace:goto-tab-4",
    "workspace:goto-tab-5",
    "workspace:goto-tab-6",
    "workspace:goto-tab-7",
    "workspace:goto-tab-8",
    "workspace:goto-last-tab",
    "workspace:undo-close-pane",
    "workspace:split-vertical",
    "workspace:split-horizontal",
    "workspace:move-to-new-window",
    "workspace:open-in-new-window",
    "workspace:new-window",
    "workspace:close-window",
    "workspace:close-tab-group",
    "workspace:close-others-tab-group",
    "editor:focus-left",
    "editor:focus-right",
    "editor:focus-top",
    "editor:focus-bottom",
    "editor:open-link-in-new-split",
    "editor:open-link-in-new-window",
    "app:delete-file",
    "workspace:copy-path",
    "workspace:copy-full-path",
    "workspace:copy-url",
    "workspace:show-trash",
    "open-with-default-app:open",
    "open-with-default-app:show",
    "theme:toggle-light-dark",
    "editor:toggle-spellcheck",
    "editor:attach-file",
    "editor:download-attachments",
    "app:open-file",
    "app:show-debug-info",
    "app:show-release-notes",
    "app:migrate-frontmatter",
    "workspace:export-pdf",
    "canvas:export-as-image",
    "window:zoom-in",
    "window:zoom-out",
    "window:reset-zoom",
    "window:toggle-always-on-top",
    "app:reload",
];

/// Binds every runnable command's hotkeys in the Workspace context and the reading
/// view's scroll keys; call once at startup with `hotkeys.json` applied.
pub fn bind_keys(cx: &mut App, registry: &CommandRegistry) {
    let ctx = Some("Workspace");
    let bindings: Vec<KeyBinding> = RUNNABLE
        .iter()
        .filter_map(|id| registry.get(id))
        .flat_map(|c| {
            c.hotkeys.iter().filter_map(|h| {
                let keys = keystroke(h)?;
                Some(KeyBinding::new(&keys, RunCommand { id: c.id.clone() }, ctx))
            })
        })
        .collect();
    cx.bind_keys(bindings);
    // The view zoom keys are Obsidian's View-menu accelerators (Ctrl+=/-/0),
    // not command hotkeys; bound here so the registry shows them unbound, and
    // a canvas or graph keeps these keys for its own zoom.
    let zoom = |key: &str, id: &str| KeyBinding::new(key, RunCommand { id: id.into() }, ctx);
    cx.bind_keys([
        zoom("secondary-=", "window:zoom-in"),
        zoom("secondary-shift-=", "window:zoom-in"),
        zoom("secondary--", "window:zoom-out"),
        zoom("secondary-0", "window:reset-zoom"),
    ]);
    // The Tags, All properties and Outline trees: arrows walk and fold,
    // Enter and Space act like a click. Home, End, Escape, F2 and Delete are not theirs.
    let tree = Some("DockTree");
    cx.bind_keys([
        KeyBinding::new("up", TreeUp, tree),
        KeyBinding::new("down", TreeDown, tree),
        KeyBinding::new("left", TreeLeft, tree),
        KeyBinding::new("right", TreeRight, tree),
        KeyBinding::new("enter", TreeActivate, tree),
        KeyBinding::new("space", TreeActivate, tree),
    ]);
    crate::reading::view::bind_keys(cx);
    download_modal::bind_keys(cx);
}

/// Everything derived from the vault folder, built off the UI thread and kept current by `apply`.
pub struct VaultState {
    pub root: PathBuf,
    vault: RwLock<Vault>,
    /// The same vault as a node graph; the desktop still reads `vault`, cache
    /// and texts by path while it moves over.
    data: RwLock<tft_app::VaultData>,
    cache: RwLock<MetadataCache>,
    /// Note text as indexed, for search and mentions.
    texts: RwLock<HashMap<String, Arc<String>>>,
    /// Config files Settings writes, locked because a control changes one while the vault is open.
    app: RwLock<AppConfig>,
    /// `app.json`'s Excluded files, compiled whenever `app` is set.
    excluded: RwLock<ExcludeFilters>,
    appearance: RwLock<AppearanceConfig>,
    switcher: RwLock<SwitcherConfig>,
    /// `page-preview.json`, read by the hover preview's sources.
    page_preview: RwLock<PagePreviewConfig>,
    /// `types.json`: the type assigned to each property name.
    property_types: RwLock<PropertyTypes>,
    /// Config files that would not parse; the app runs on defaults for them
    /// and refuses to write over what the user had.
    pub unreadable_configs: Vec<String>,
    pub index_ms: f64,
}

/// One re-read note: its path and, for markdown, the new record and text.
type Reindexed = (String, Option<(CachedMetadata, Arc<String>)>);

/// What a batch of filesystem events changed.
#[derive(Debug, Default)]
pub struct Changes {
    pub files: Vec<FileChange>,
    pub notes: Vec<String>,
    pub removed_notes: Vec<String>,
}

impl Changes {
    fn structural(&self) -> bool {
        self.files
            .iter()
            .any(|c| !matches!(c, FileChange::Modified(_)))
    }
}

/// What a rename did: the files that moved, the links rewritten, and the index changes.
#[derive(Debug, Default)]
pub struct RenameOutcome {
    pub changes: Changes,
    pub renamed: Vec<(String, String)>,
    pub rewritten_files: usize,
    pub rewritten_links: usize,
    /// Notes whose links were left alone because the file changed underneath the rewrite.
    pub stale: Vec<String>,
}

impl VaultState {
    pub fn load(root: &PathBuf) -> Result<VaultState, String> {
        let started = Instant::now();
        let vault = Vault::open(root).map_err(|e| e.to_string())?;
        // A config that will not parse falls back to defaults so one broken
        // file cannot stop a vault opening; the window reports it and writes are refused.
        let mut unreadable = Vec::new();
        // Only a parse failure counts: an unreadable file has nothing on disk to lose.
        let mut note = |name: &'static str, err: Option<&VaultError>| {
            if matches!(err, Some(VaultError::Json { .. })) {
                unreadable.push(name.to_string());
            }
        };
        let app = vault.app_config();
        note("app.json", app.as_ref().err());
        let app = app.unwrap_or_default();
        let appearance = vault.appearance_config();
        note("appearance.json", appearance.as_ref().err());
        let appearance = appearance.unwrap_or_default();
        let switcher = vault.switcher_config();
        note("switcher.json", switcher.as_ref().err());
        let switcher = switcher.unwrap_or_default();
        let page_preview = vault.page_preview_config();
        note("page-preview.json", page_preview.as_ref().err());
        let page_preview = page_preview.unwrap_or_default();
        let property_types = vault.property_types();
        note("types.json", property_types.as_ref().err());
        let property_types = property_types.unwrap_or_default();
        let notes: Vec<String> = vault.markdown_files().map(|f| f.path.clone()).collect();
        let loaded: Vec<(String, CachedMetadata, Arc<String>)> = notes
            .par_iter()
            .map(|path| {
                let text = std::fs::read_to_string(vault.absolute(path)).unwrap_or_default();
                let meta = index_markdown(&text);
                (path.clone(), meta, Arc::new(text))
            })
            .collect();
        let mut texts = HashMap::with_capacity(loaded.len());
        let metas: Vec<(String, CachedMetadata)> = loaded
            .into_iter()
            .map(|(path, meta, text)| {
                texts.insert(path.clone(), text);
                (path, meta)
            })
            .collect();
        let cache = MetadataCache::new(vault.link_resolver(), metas);
        let mut data = tft_app::VaultData::new(vault.root().to_string_lossy());
        vault.apply_snapshot(&mut data);
        let state = VaultState {
            root: vault.root().to_path_buf(),
            vault: RwLock::new(vault),
            data: RwLock::new(data),
            cache: RwLock::new(cache),
            texts: RwLock::new(texts),
            excluded: RwLock::new(ExcludeFilters::new(&[] as &[String])),
            app: RwLock::new(app),
            appearance: RwLock::new(appearance),
            switcher: RwLock::new(switcher),
            page_preview: RwLock::new(page_preview),
            property_types: RwLock::new(property_types),
            unreadable_configs: unreadable,
            index_ms: started.elapsed().as_secs_f64() * 1000.0,
        };
        state.verify_data()?;
        Ok(state)
    }

    pub fn property_types(&self) -> RwLockReadGuard<'_, PropertyTypes> {
        self.property_types.read().expect("property types lock")
    }

    // Every core plugin is on; `core-plugins.json` is not read.
    pub fn core_plugin_enabled(&self, _id: &str) -> bool {
        true
    }

    pub fn app(&self) -> RwLockReadGuard<'_, AppConfig> {
        self.app.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The Excluded files rules: what search, the graph, unlinked mentions and
    /// the tags pane leave out, and what the switcher puts last.
    pub fn excluded(&self) -> RwLockReadGuard<'_, ExcludeFilters> {
        self.excluded.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_excluded(&self, path: &str) -> bool {
        self.excluded().is_excluded(path)
    }

    pub fn appearance(&self) -> RwLockReadGuard<'_, AppearanceConfig> {
        self.appearance.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Re-reads `appearance.json` from disk into the config in hand and
    /// answers it: an edit made outside the app counts as much as one of
    /// its own.
    pub fn reload_appearance(&self) -> AppearanceConfig {
        let fresh = self
            .vault
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .appearance_config()
            .ok();
        let mut cur = self.appearance.write().unwrap_or_else(|e| e.into_inner());
        if let Some(fresh) = fresh {
            *cur = fresh;
        }
        cur.clone()
    }

    pub fn config_dir(&self) -> String {
        self.vault
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .config_dir()
            .to_string()
    }

    pub fn switcher(&self) -> RwLockReadGuard<'_, SwitcherConfig> {
        self.switcher.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn page_preview(&self) -> RwLockReadGuard<'_, PagePreviewConfig> {
        self.page_preview.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes a setting the Settings window changed and keeps the config in step;
    /// the file goes first, since a change that cannot reach disk does not count.
    pub fn apply_setting(&self, change: &SettingChange) -> Result<(), String> {
        match change {
            SettingChange::App(config) => {
                self.write_config("app.json", config)?;
                *self.app.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
            SettingChange::Appearance(config) => {
                self.write_config("appearance.json", config)?;
                *self.appearance.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
        }
        Ok(())
    }

    fn write_config<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        self.vault()
            .write_config(name, value)
            .map_err(|e| e.to_string())
    }

    /// Records `key`'s type in `types.json`, as Obsidian's type menu does, and returns the table.
    pub fn assign_property_type(
        &self,
        key: &str,
        ty: tft_formats::properties::PropertyType,
    ) -> Result<PropertyTypes, String> {
        let mut types = self.property_types.write().expect("property types lock");
        // Obsidian seeds the built-in keys on every write, before the key being assigned.
        types.seed_defaults();
        types.types.insert(key.to_string(), ty.as_str().to_string());
        self.vault()
            .write_config("types.json", &*types)
            .map_err(|e| e.to_string())?;
        Ok(types.clone())
    }

    pub fn vault(&self) -> RwLockReadGuard<'_, Vault> {
        self.vault.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn data(&self) -> RwLockReadGuard<'_, tft_app::VaultData> {
        self.data.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Rebuilds the node graph from the path vault, reusing backend IDs so
    /// nodes that did not move keep their NodeId.
    fn sync_data(&self) {
        let vault = self.vault();
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        vault.apply_snapshot(&mut data);
    }

    /// The desktop migration's ratchet: every unique path the path vault
    /// reports must have exactly one node, and the projections must agree.
    pub fn verify_data(&self) -> Result<(), String> {
        let vault = self.vault();
        let data = self.data();
        let expected_files: std::collections::BTreeSet<&str> = vault
            .files()
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| data.lookup(p).is_ok())
            .collect();
        let actual_files: std::collections::BTreeSet<String> =
            data.files().into_iter().map(|f| f.path).collect();
        if expected_files.len() != actual_files.len()
            || !expected_files.iter().all(|p| actual_files.contains(*p))
        {
            return Err(format!(
                "files differ: {} in the vault, {} in the node graph",
                expected_files.len(),
                actual_files.len()
            ));
        }
        let expected_folders: std::collections::BTreeSet<&str> = vault
            .folders()
            .iter()
            .map(String::as_str)
            .filter(|p| data.lookup(p).is_ok())
            .collect();
        let actual_folders: std::collections::BTreeSet<String> =
            data.folders().into_iter().collect();
        if expected_folders.len() != actual_folders.len()
            || !expected_folders.iter().all(|p| actual_folders.contains(*p))
        {
            return Err(format!(
                "folders differ: {} in the vault, {} in the node graph",
                expected_folders.len(),
                actual_folders.len()
            ));
        }
        data.validate()
            .map_err(|e| format!("node graph invalid: {e:?}"))
    }

    pub fn cache(&self) -> RwLockReadGuard<'_, MetadataCache> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn texts(&self) -> RwLockReadGuard<'_, HashMap<String, Arc<String>>> {
        self.texts.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn build_options(&self) -> reading::BuildOptions {
        reading::BuildOptions {
            strict_line_breaks: crate::fixed::STRICT_LINE_BREAKS,
        }
    }

    /// Applies filesystem events for `paths`: refreshes the file list, re-indexes
    /// changed notes and re-resolves links when files came or went.
    pub fn apply(&self, paths: &[PathBuf]) -> Changes {
        let mut files = Vec::new();
        {
            let mut vault = self.vault.write().unwrap_or_else(|e| e.into_inner());
            for path in paths {
                files.extend(vault.refresh_path(path));
            }
        }
        self.absorb(files)
    }

    /// Re-reads and re-indexes the notes among `files` and re-resolves links when files came or went.
    fn absorb(&self, files: Vec<FileChange>) -> Changes {
        if files.iter().any(|c| !matches!(c, FileChange::Modified(_))) {
            self.sync_data();
        }
        let mut changes = Changes::default();
        // Keep one change per path, the last one seen.
        let mut last: HashMap<String, FileChange> = HashMap::new();
        for c in files {
            last.insert(c.path().to_string(), c);
        }
        let mut files: Vec<FileChange> = last.into_values().collect();
        files.sort_by(|a, b| a.path().cmp(b.path()));
        if files.is_empty() {
            return changes;
        }
        let reread: Vec<Reindexed> = files
            .par_iter()
            .map(|c| match c {
                FileChange::Added(p) | FileChange::Modified(p)
                    if FileKind::from_path(p) == FileKind::Markdown =>
                {
                    let text = std::fs::read_to_string(self.root.join(p)).unwrap_or_default();
                    let meta = index_markdown(&text);
                    (p.clone(), Some((meta, Arc::new(text))))
                }
                other => (other.path().to_string(), None),
            })
            .collect();
        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
            for (c, (path, indexed)) in files.iter().zip(reread) {
                match (c, indexed) {
                    (FileChange::Removed(_), _) => {
                        if texts.remove(&path).is_some() {
                            changes.removed_notes.push(path.clone());
                        }
                        cache.forget_path(&path);
                    }
                    (_, Some((meta, text))) => {
                        cache.insert_file(path.clone(), meta);
                        texts.insert(path.clone(), text);
                        changes.notes.push(path);
                    }
                    (_, None) => cache.add_path(&path),
                }
            }
            if files.iter().any(|c| !matches!(c, FileChange::Modified(_))) {
                cache.reresolve();
            }
        }
        changes.files = files;
        changes
    }

    pub fn link_format(&self) -> LinkFormat {
        LinkFormat::Shortest
    }

    pub fn trash_option(&self) -> TrashOption {
        TrashOption::System
    }

    /// `alwaysUpdateLinks`, **off** by default: a rename asks before rewriting.
    /// Obsidian's row says "Turn off to be prompted to update links after renaming
    /// a file", and the toggle rendered off on a vault whose `app.json` was `{}`.
    pub fn update_links_automatically(&self) -> bool {
        false
    }

    /// "Confirm before deleting files" left the Settings window, fixed on.
    pub fn confirm_delete(&self) -> bool {
        crate::fixed::PROMPT_DELETE
    }

    /// Where a note created for the link text `name` goes (`newFileLocation`, `newFileFolderPath`).
    pub fn new_note_path(&self, name: &str, _current: Option<&str>) -> String {
        self.vault().new_note_path(name, "")
    }

    /// Creates a note (and its folders) and indexes it.
    pub fn create_note(&self, path: &str, text: &str) -> Result<Changes, String> {
        let files = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .create_note(path, text)
            .map_err(|e| e.to_string())?;
        Ok(self.absorb(files))
    }

    pub fn create_folder(&self, path: &str) -> Result<(), String> {
        self.vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .create_folder(path)
            .map_err(|e| e.user_message())?;
        self.sync_data();
        Ok(())
    }

    /// Copies a file or folder next to itself (`Name 1.ext`, `Name 1`) and indexes the copy.
    pub fn duplicate(&self, path: &str) -> Result<(String, Changes), String> {
        let (target, files) = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .duplicate(path)
            .map_err(|e| e.user_message())?;
        Ok((target, self.absorb(files)))
    }

    /// Whether the rename would go through, without doing it.
    pub fn check_rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.vault()
            .check_rename(from, to)
            .map_err(|e| e.user_message())
    }

    /// The rewrites a rename would need, without doing anything: `(files, links)`.
    pub fn rename_preview(&self, from: &str, to: &str) -> (usize, usize) {
        let plan = self.vault().rename_plan(from, to);
        let texts = self.texts();
        let rewrites = self
            .cache()
            .plan_rename_rewrites(&plan, self.link_format(), &|p| {
                texts.get(p).map(|t| t.to_string())
            });
        (
            rewrites.len(),
            rewrites.iter().map(|r| r.links).sum::<usize>(),
        )
    }

    /// Writes a block id into `path`, a note the user is not editing. The bytes
    /// on disk must be the bytes the insertion was planned against; the watcher re-indexes.
    pub fn write_block_id(
        &self,
        path: &str,
        before: &str,
        insertion: &tft_suggest::blocks::IdInsertion,
    ) -> Result<(), String> {
        let absolute = self.vault().write_path(path).map_err(|e| e.to_string())?;
        let on_disk = std::fs::read_to_string(&absolute)
            .map_err(|e| format!("Could not read {path}: {e}"))?;
        if on_disk != before {
            return Err(format!("{path} changed on disk; no id was written"));
        }
        tft_vault::write_no_follow(&absolute, insertion.applied(&on_disk).as_bytes())
            .map_err(|e| format!("Could not write {path}: {e}"))
    }

    /// Renames or moves a file or folder, rewrites links to it when `update_links`,
    /// and brings the index up to date.
    pub fn rename(
        &self,
        from: &str,
        to: &str,
        update_links: bool,
    ) -> Result<RenameOutcome, String> {
        let moved = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .rename(from, to)
            // Both of these end up in a modal the user typed into, so they
            // get the sentence rather than the path and the errno.
            .map_err(|e| e.user_message())?;
        {
            let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
            data.rebase_local_prefix(from, to);
            for (old, new) in &moved {
                data.rebase_local_prefix(old, new);
            }
        }
        self.sync_data();
        let plan = if update_links {
            let texts = self.texts();
            self.cache()
                .plan_rename_rewrites(&moved, self.link_format(), &|p| {
                    texts.get(p).map(|t| t.to_string())
                })
        } else {
            Vec::new()
        };
        // Each rewrite was planned from the indexed text, so the file must still
        // be that text: a note written underneath keeps its own bytes and its
        // links are left alone rather than overwritten.
        let mut stale: Vec<String> = Vec::new();
        let texts = self.texts();
        for rw in &plan {
            let absolute = self
                .vault()
                .write_path(&rw.new_path)
                .map_err(|e| e.to_string())?;
            let planned_from = texts.get(&rw.old_path).map(|t| t.to_string());
            match (std::fs::read_to_string(&absolute), planned_from) {
                (Ok(on_disk), Some(was)) if on_disk != was => {
                    stale.push(rw.new_path.clone());
                    continue;
                }
                _ => {}
            }
            tft_vault::write_no_follow(&absolute, rw.text.as_bytes())
                .map_err(|e| format!("Could not update links in {}: {e}", rw.new_path))?;
        }
        drop(texts);
        let moved_from: HashMap<&str, &str> = moved
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for (old, new) in &moved {
            let old_text = texts.remove(old);
            cache.forget_path(old);
            changes.files.push(FileChange::Removed(old.clone()));
            if old_text.is_some() {
                changes.removed_notes.push(old.clone());
            }
            if FileKind::from_path(new) == FileKind::Markdown {
                let text = plan
                    .iter()
                    .find(|r| r.old_path == *old)
                    .map(|r| Arc::new(r.text.clone()))
                    .or(old_text)
                    .unwrap_or_else(|| {
                        Arc::new(std::fs::read_to_string(self.root.join(new)).unwrap_or_default())
                    });
                cache.insert_file(new.clone(), index_markdown(&text));
                texts.insert(new.clone(), text);
                changes.notes.push(new.clone());
            } else {
                cache.add_path(new);
            }
            changes.files.push(FileChange::Added(new.clone()));
        }
        for rw in plan
            .iter()
            .filter(|r| !moved_from.contains_key(r.old_path.as_str()))
        {
            let text = Arc::new(rw.text.clone());
            cache.insert_file(rw.new_path.clone(), index_markdown(&text));
            texts.insert(rw.new_path.clone(), text);
            changes
                .files
                .push(FileChange::Modified(rw.new_path.clone()));
            changes.notes.push(rw.new_path.clone());
        }
        cache.reresolve();
        Ok(RenameOutcome {
            changes,
            renamed: moved,
            rewritten_files: plan.len() - stale.len(),
            rewritten_links: plan
                .iter()
                .filter(|r| !stale.contains(&r.new_path))
                .map(|r| r.links)
                .sum(),
            stale,
        })
    }

    /// Writes the notes a heading rename rewrites and indexes them, leaving a note
    /// that changed underneath alone; returns the changes and the stale paths.
    pub fn write_link_rewrites(
        &self,
        plan: &[tft_cache::Rewrite],
    ) -> Result<(Changes, Vec<String>), String> {
        let mut stale: Vec<String> = Vec::new();
        let mut written: Vec<&tft_cache::Rewrite> = Vec::new();
        for rw in plan {
            let absolute = self
                .vault()
                .write_path(&rw.new_path)
                .map_err(|e| e.to_string())?;
            let planned_from = self.texts().get(&rw.old_path).map(|t| t.to_string());
            match (std::fs::read_to_string(&absolute), planned_from) {
                (Ok(on_disk), Some(was)) if on_disk == was => {}
                _ => {
                    stale.push(rw.new_path.clone());
                    continue;
                }
            }
            tft_vault::write_no_follow(&absolute, rw.text.as_bytes())
                .map_err(|e| format!("Could not update links in {}: {e}", rw.new_path))?;
            written.push(rw);
        }
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for rw in written {
            let text = Arc::new(rw.text.clone());
            cache.insert_file(rw.new_path.clone(), index_markdown(&text));
            texts.insert(rw.new_path.clone(), text);
            changes
                .files
                .push(FileChange::Modified(rw.new_path.clone()));
            changes.notes.push(rw.new_path.clone());
        }
        cache.reresolve();
        Ok((changes, stale))
    }

    /// Deletes a file or folder per `trashOption` and drops it from the index.
    pub fn trash(&self, path: &str) -> Result<Changes, String> {
        let removed = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .trash(path, self.trash_option())
            .map_err(|e| e.to_string())?;
        self.sync_data();
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for p in removed {
            if texts.remove(&p).is_some() {
                changes.removed_notes.push(p.clone());
            }
            cache.forget_path(&p);
            changes.files.push(FileChange::Removed(p));
        }
        cache.reresolve();
        Ok(changes)
    }
}

impl reading::EmbedSource for VaultState {
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String> {
        self.cache()
            .resolve(linktext, source_path)
            .map(str::to_string)
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.vault().absolute(path)).ok()
    }

    fn absolute(&self, path: &str) -> PathBuf {
        self.vault().absolute(path)
    }
}

pub use tft_app::ViewMode;

/// What the main area paints: the mode asks for a view, the open file decides whether
/// it can show one, and `Empty` stands in when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainView {
    Reading,
    Editor,
    Graph,
    Canvas,
    Pdf,
    /// A `release-notes` leaf: this build's notes in a reading view of their own.
    ReleaseNotes,
    Empty,
}

impl MainView {
    /// The name the socket's `focus` and `state` use for this view.
    fn as_str(self) -> &'static str {
        match self {
            MainView::Reading => "reading",
            MainView::Editor => "editor",
            MainView::Graph => "graph",
            MainView::Canvas => "canvas",
            MainView::Pdf => "pdf",
            MainView::ReleaseNotes => "release-notes",
            MainView::Empty => "nothing",
        }
    }
}

use tft_app::{FileOperationRequest, LeftTab};

/// How long a Notice stays: Obsidian fades its after a few seconds, exact duration unprobed.
const NOTICE_LIFETIME: Duration = Duration::from_secs(5);

/// A message on screen, with the id its expiry timer names it by.
struct Notice {
    id: u64,
    text: SharedString,
    /// The window it shows in (`None` for the vault window), the active one when it went up.
    window: Option<u64>,
    /// A button after the message (Obsidian's `.notice-cta`), as the migration's Undo.
    action: Option<NoticeAction>,
}

/// What a notice's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeAction {
    /// "Undo" after a Frontmatter migration: every file written back to its prior bytes.
    UndoMigration,
}

impl NoticeAction {
    fn label(self) -> &'static str {
        match self {
            NoticeAction::UndoMigration => "Undo",
        }
    }
}

impl Notice {
    /// The window it draws in: its own while open, else the vault window.
    fn home(&self, open: &impl Fn(u64) -> bool) -> Option<u64> {
        self.window.filter(|id| open(*id))
    }
}

/// The transient messages, oldest first. The status bar keeps what stays true;
/// everything a command has to say goes here and expires on its own.
#[derive(Default)]
struct Notices {
    live: Vec<Notice>,
    last_id: u64,
}

impl Notices {
    /// Shows `text` in `window` (`None` vault) and returns its id; a message already
    /// on screen is renewed rather than stacked, since the autosave retries on a timer.
    #[cfg(test)]
    fn push(&mut self, text: SharedString, window: Option<u64>) -> u64 {
        self.push_with(text, window, None)
    }

    /// [`Notices::push`] with a button after the message.
    fn push_with(
        &mut self,
        text: SharedString,
        window: Option<u64>,
        action: Option<NoticeAction>,
    ) -> u64 {
        self.live.retain(|n| n.text != text);
        self.last_id += 1;
        self.live.push(Notice {
            id: self.last_id,
            text,
            window,
            action,
        });
        self.last_id
    }

    /// The messages window `here` draws, oldest first.
    fn in_window<'a>(
        &'a self,
        here: Option<u64>,
        open: &'a impl Fn(u64) -> bool,
    ) -> impl Iterator<Item = &'a Notice> + 'a {
        self.live.iter().filter(move |n| n.home(open) == here)
    }

    /// The window each message draws in, in the order of `texts`.
    fn windows(&self, open: &impl Fn(u64) -> bool) -> Vec<Option<u64>> {
        self.live.iter().map(|n| n.home(open)).collect()
    }

    /// Drops the message with this id, if it is still the one on screen.
    fn expire(&mut self, id: u64) {
        self.live.retain(|n| n.id != id);
    }

    fn clear(&mut self) {
        self.live.clear();
    }

    /// The newest message, which is what the last command said.
    fn latest(&self) -> Option<&SharedString> {
        self.live.last().map(|n| &n.text)
    }

    fn texts(&self) -> Vec<String> {
        self.live.iter().map(|n| n.text.to_string()).collect()
    }
}

/// Which overlay is up, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modal {
    None,
    Switcher,
    Prompt,
    /// The template chooser.
    Chooser,
    /// The Search pane's "Copy search results".
    CopyResults,
    /// "Download attachments" (`editor:download-attachments`).
    Downloads,
    /// "Debug info" (`app:show-debug-info`).
    DebugInfo,
    /// "Frontmatter migration" (`app:migrate-frontmatter`).
    MigrateFrontmatter,
    /// "Export to PDF" (`workspace:export-pdf`).
    PdfExport,
    /// "Export as image" (`canvas:export-as-image`).
    ImageExport,
}

/// The file operation the open prompt will finish.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingOp {
    Rename(String),
    Move(String),
    Delete(String),
    NewFolder {
        parent: String,
    },
    /// `alwaysUpdateLinks` is off: asking whether to rewrite links.
    UpdateLinks {
        from: String,
        to: String,
    },
    /// `editor:rename-heading`: the note and the zero-based line of the ATX heading.
    RenameHeading {
        path: String,
        line: u32,
    },
}

/// One place back and forward can land: the note and the state the view was left
/// in, so back to a note read in Reading returns it in Reading.
use tft_app::{place_new_tab, HistoryEntry, Tab};

trait TabTitle {
    fn title(&self) -> SharedString;
}

impl TabTitle for Tab {
    fn title(&self) -> SharedString {
        match release_notes::version_of(self) {
            Some(version) => release_notes::title(&version).into(),
            None => leaf_title(self.path.as_deref(), self.mode),
        }
    }
}

/// What the chooser is open for: a vault file for a canvas card, or a group to jump to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ChooserPurpose {
    #[default]
    CanvasFile,
    CanvasGroup,
}

use tft_app::SplitDirection;

/// The main area's shape: a tree of splits over the tab groups, the way
/// `workspace.json`'s `main` records it. The root is always a split, even around a single group.
#[derive(Debug, Clone, PartialEq)]
enum Layout {
    Group(u64),
    Split {
        direction: SplitDirection,
        children: Vec<Layout>,
        /// The split's own `workspace.json` keys, kept so the node is written back as read.
        node: Map<String, Value>,
    },
}

impl Layout {
    fn split(direction: SplitDirection, children: Vec<Layout>) -> Layout {
        let mut node = Map::new();
        node.insert("id".into(), json!(leaf_id()));
        node.insert("type".into(), json!("split"));
        Layout::Split {
            direction,
            children,
            node,
        }
    }

    /// Every group, depth first, in the order Obsidian lays them out.
    fn groups(&self, out: &mut Vec<u64>) {
        match self {
            Layout::Group(id) => out.push(*id),
            Layout::Split { children, .. } => children.iter().for_each(|c| c.groups(out)),
        }
    }

    /// The group in the main area's top right corner: the last child of every
    /// side-by-side split and the first of every stacked one; its strip pads for the window controls.
    fn top_right(&self) -> Option<u64> {
        match self {
            Layout::Group(id) => Some(*id),
            Layout::Split {
                direction,
                children,
                ..
            } => match direction {
                SplitDirection::Vertical => children.last()?.top_right(),
                SplitDirection::Horizontal => children.first()?.top_right(),
            },
        }
    }

    fn contains(&self, id: u64) -> bool {
        match self {
            Layout::Group(g) => *g == id,
            Layout::Split { children, .. } => children.iter().any(|c| c.contains(id)),
        }
    }

    /// The child indexes from the root down to group `id`.
    fn path_to(&self, id: u64) -> Option<Vec<usize>> {
        match self {
            Layout::Group(g) => (*g == id).then(Vec::new),
            Layout::Split { children, .. } => children.iter().enumerate().find_map(|(ix, c)| {
                let mut path = c.path_to(id)?;
                path.insert(0, ix);
                Some(path)
            }),
        }
    }

    fn node_at(&self, path: &[usize]) -> Option<&Layout> {
        match path.split_first() {
            None => Some(self),
            Some((ix, rest)) => match self {
                Layout::Split { children, .. } => children.get(*ix)?.node_at(rest),
                Layout::Group(_) => None,
            },
        }
    }

    fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut Layout> {
        match path.split_first() {
            None => Some(self),
            Some((ix, rest)) => match self {
                Layout::Split { children, .. } => children.get_mut(*ix)?.node_at_mut(rest),
                Layout::Group(_) => None,
            },
        }
    }

    /// Puts `new` beside group `id` along `direction` ("Split right"/"Split down"):
    /// a sibling when the enclosing split already runs that way, else wrapped in a
    /// new split. `after` says right/below vs left/above; `Some(true)` means wrapped.
    fn insert_beside(
        &mut self,
        id: u64,
        new: u64,
        direction: SplitDirection,
        after: bool,
    ) -> Option<bool> {
        let Layout::Split {
            direction: own,
            children,
            ..
        } = self
        else {
            return None;
        };
        let ix = children.iter().position(|c| c.contains(id))?;
        if matches!(children[ix], Layout::Group(g) if g == id) {
            if *own == direction {
                children.insert(if after { ix + 1 } else { ix }, Layout::Group(new));
                return Some(false);
            }
            let pair = if after {
                vec![Layout::Group(id), Layout::Group(new)]
            } else {
                vec![Layout::Group(new), Layout::Group(id)]
            };
            children[ix] = Layout::split(direction, pair);
            return Some(true);
        }
        children[ix].insert_beside(id, new, direction, after)
    }

    /// Takes group `id` out; a split left with one child keeps it (Obsidian keeps the nesting).
    fn remove(&mut self, id: u64) -> bool {
        let Layout::Split { children, .. } = self else {
            return false;
        };
        let Some(ix) = children.iter().position(|c| c.contains(id)) else {
            return false;
        };
        if matches!(children[ix], Layout::Group(g) if g == id) {
            children.remove(ix);
            return true;
        }
        let removed = children[ix].remove(id);
        if removed && matches!(&children[ix], Layout::Split { children, .. } if children.is_empty())
        {
            children.remove(ix);
        }
        removed
    }

    /// The groups to step to from `id` along `direction`: the nearest sibling set that runs that way.
    fn neighbors(&self, id: u64, direction: SplitDirection, forward: bool) -> Vec<u64> {
        let Some(path) = self.path_to(id) else {
            return Vec::new();
        };
        for depth in (0..path.len()).rev() {
            let Some(Layout::Split {
                direction: own,
                children,
                ..
            }) = self.node_at(&path[..depth])
            else {
                continue;
            };
            if *own != direction {
                continue;
            }
            let ix = path[depth];
            let sibling = if forward {
                children.get(ix + 1)
            } else {
                ix.checked_sub(1).and_then(|i| children.get(i))
            };
            if let Some(sibling) = sibling {
                let mut out = Vec::new();
                sibling.groups(&mut out);
                return out;
            }
        }
        Vec::new()
    }
}

/// One popout window, as `workspace.json`'s `floating` split records it: a root
/// split over tab groups, like `main`, with the window's place on the node. A
/// restored node has no gpui window until it is opened (see [[Specification#Observed popout windows]]).
#[derive(Debug, Clone, PartialEq)]
struct WindowNode {
    id: u64,
    /// The groups and how they are split. Its own `node` map goes unwritten: a
    /// `window` node's keys live in `node` below.
    tree: Layout,
    /// The node's own keys as read (bounds, `maximize`, `zoom`, anything unknown),
    /// less `children` and `direction`, which the tree supplies.
    node: Map<String, Value>,
    /// The gpui window showing this node, while one is open.
    handle: Option<gpui::WindowHandle<PopoutWindow>>,
    /// What that window's title bar says, set only when it changes.
    title: SharedString,
    /// The inner size the window was opened for, written back while the window measures
    /// within a pixel: a fractional display scale rounds the surface.
    opened_for: Option<(i32, i32)>,
}

impl WindowNode {
    /// A window TFT opened: Obsidian's keys in Obsidian's order, not maximized, at zoom 0.
    fn new(id: u64, tree: Layout, bounds: WindowBounds) -> WindowNode {
        let mut node = Map::new();
        node.insert("id".into(), json!(leaf_id()));
        node.insert("type".into(), json!("window"));
        node.insert("x".into(), json!(bounds.x));
        node.insert("y".into(), json!(bounds.y));
        node.insert("width".into(), json!(bounds.width));
        node.insert("height".into(), json!(bounds.height));
        node.insert("maximize".into(), json!(false));
        node.insert("zoom".into(), json!(0));
        WindowNode::from_node(id, tree, node)
    }

    /// A window as the file described it, not yet shown.
    fn from_node(id: u64, tree: Layout, node: Map<String, Value>) -> WindowNode {
        WindowNode {
            id,
            tree,
            node,
            handle: None,
            title: SharedString::default(),
            opened_for: None,
        }
    }

    /// Records where the window is now: the bounds as whole numbers and `maximize`,
    /// in the slots they have or at the end for a node that never had one.
    fn set_bounds(&mut self, bounds: WindowBounds, maximized: bool) {
        for (key, value) in [
            ("x", json!(bounds.x)),
            ("y", json!(bounds.y)),
            ("width", json!(bounds.width)),
            ("height", json!(bounds.height)),
            ("maximize", json!(maximized)),
        ] {
            self.node.insert(key.into(), value);
        }
    }

    /// Whether the gpui window is still up.
    fn is_open(&self, cx: &App) -> bool {
        self.handle.is_some_and(|h| h.read(cx).is_ok())
    }

    /// The recorded position and inner size, when all four are there.
    fn bounds(&self) -> Option<WindowBounds> {
        let number = |key: &str| {
            self.node
                .get(key)
                .and_then(Value::as_f64)
                .map(|v| v.round() as i32)
        };
        Some(WindowBounds {
            x: number("x")?,
            y: number("y")?,
            width: number("width")?,
            height: number("height")?,
        })
    }
}

/// A popout's place as `workspace.json` records it: position and inner size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// One tab group: a strip of tabs with its own editor and reading view. The active
/// group's fields are on the workspace; an inactive group keeps its own here.
struct TabGroup {
    id: u64,
    tabs: Vec<Tab>,
    active_tab: usize,
    mode_before_graph: ViewMode,
    detached: Option<SharedString>,
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    canvas: Entity<CanvasView>,
    pdf: Entity<PdfView>,
    /// The `tabs` node this group was restored from, less its children.
    node: Map<String, Value>,
}

impl TabGroup {
    fn new(
        id: u64,
        editor: Entity<Editor>,
        reading: Entity<ReadingView>,
        canvas: Entity<CanvasView>,
        pdf: Entity<PdfView>,
        mode: ViewMode,
    ) -> TabGroup {
        let mut node = Map::new();
        node.insert("id".into(), json!(leaf_id()));
        node.insert("type".into(), json!("tabs"));
        TabGroup {
            id,
            tabs: vec![Tab::empty(mode)],
            active_tab: 0,
            mode_before_graph: ViewMode::Reading,
            detached: None,
            editor,
            reading,
            canvas,
            pdf,
            node,
        }
    }

    /// The group's share of its split, Obsidian's `dimension` (a percentage).
    fn dimension(&self) -> Option<f64> {
        self.node.get("dimension").and_then(Value::as_f64)
    }

    fn set_dimension(&mut self, dimension: Option<f64>) {
        set_dimension(&mut self.node, dimension);
    }
}

/// Writes or drops a node's `dimension`, in the slot Obsidian gives it: after `type`.
fn set_dimension(node: &mut Map<String, Value>, dimension: Option<f64>) {
    match dimension {
        Some(d) => {
            if node.contains_key("dimension") {
                node["dimension"] = json!(d);
            } else {
                let at = node
                    .keys()
                    .position(|k| k == "type")
                    .map(|i| i + 1)
                    .unwrap_or(node.len());
                node.shift_insert(at, "dimension".into(), json!(d));
            }
        }
        None => {
            node.shift_remove("dimension");
        }
    }
}

/// What one group's rendering reads: live fields for the active group, stashed for others.
struct GroupView {
    id: u64,
    active: bool,
    tabs: Vec<Tab>,
    active_tab: usize,
    current: Option<String>,
    mode: ViewMode,
    history_len: usize,
    history_ix: usize,
    detached: Option<SharedString>,
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    canvas: Entity<CanvasView>,
    pdf: Entity<PdfView>,
}

/// The view-header title being renamed in place. Obsidian's `.view-header-title`
/// is editable at rest, so a click starts typing and only Enter renames the file.
/// The text is the stem, shared with the rename prompt's validator.
struct TitleEdit {
    /// The group whose header holds the field.
    group: u64,
    /// The file the field opened on; the edit is dropped if that leaf moves on.
    path: String,
    /// The typed text and the input method's composition, as the file tree's row rename holds.
    editing: inline_field::Editing,
}

/// The payload of a divider drag: which split and the boundary between two children.
#[derive(Debug, Clone)]
struct DividerDrag {
    /// The popout window whose tree the split is in; `None` for the main area's.
    window: Option<u64>,
    path: Vec<usize>,
    before: usize,
}

/// The payload of a tab drag: the group's strip and tab index. The tab stays until the drop.
#[derive(Debug, Clone)]
struct TabDrag {
    group: u64,
    tab: usize,
    title: SharedString,
}

/// What a tab drag measured, each in its own window's coordinates, for the hit test across windows.
#[derive(Default)]
struct DragBounds {
    strips: HashMap<u64, gpui::Bounds<Pixels>>,
    contents: HashMap<u64, gpui::Bounds<Pixels>>,
    tabs: HashMap<(u64, usize), gpui::Bounds<Pixels>>,
}

/// Which of [`DragBounds`]' maps a measured element goes in.
#[derive(Clone, Copy)]
enum DragPart {
    Strip(u64),
    Content(u64),
    Tab(u64, usize),
}

/// An invisible child that writes where its parent was laid out into `bounds`, for cross-window drags.
fn measure_for_drag(bounds: &Rc<RefCell<DragBounds>>, part: DragPart) -> impl IntoElement {
    let bounds = bounds.clone();
    canvas(
        move |b, _, _| {
            let mut all = bounds.borrow_mut();
            match part {
                DragPart::Strip(g) => all.strips.insert(g, b),
                DragPart::Content(g) => all.contents.insert(g, b),
                DragPart::Tab(g, ix) => all.tabs.insert((g, ix), b),
            };
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Where a tab dragged over a group's content would land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    /// The zone `position` falls in: an outer band on each side splits, the middle
    /// joins the strip; the nearer edge wins a corner.
    fn at(bounds: gpui::Bounds<gpui::Pixels>, position: gpui::Point<gpui::Pixels>) -> DropZone {
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        let x = f32::from(position.x - bounds.origin.x);
        let y = f32::from(position.y - bounds.origin.y);
        let band = w.min(h) * DROP_EDGE_SHARE;
        let edges = [
            (DropZone::Left, x),
            (DropZone::Right, w - x),
            (DropZone::Top, y),
            (DropZone::Bottom, h - y),
        ];
        let (zone, distance) = edges
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((DropZone::Center, 0.0));
        if distance < band {
            zone
        } else {
            DropZone::Center
        }
    }

    /// The split a side zone makes, and whether the new group goes after the dropped-on one.
    fn split(self) -> Option<(SplitDirection, bool)> {
        match self {
            DropZone::Center => None,
            DropZone::Left => Some((SplitDirection::Vertical, false)),
            DropZone::Right => Some((SplitDirection::Vertical, true)),
            DropZone::Top => Some((SplitDirection::Horizontal, false)),
            DropZone::Bottom => Some((SplitDirection::Horizontal, true)),
        }
    }
}

/// How much of a group's shorter side, from each edge, drops as a split.
const DROP_EDGE_SHARE: f32 = 0.3;

/// Where a tab dragged over a strip would land: before tab `at` at the nearest
/// boundary, with an accent bar that slides there from `from`.
#[derive(Debug, Clone, Copy)]
struct DropBar {
    group: u64,
    at: usize,
    x: f32,
    from: f32,
    since: Instant,
    /// The strip's left edge, which the bar's position is drawn from.
    strip_x: f32,
}

impl DropBar {
    /// Where the bar is drawn at `now`, easing toward `x` so it lands softly.
    fn shown_x(&self, now: Instant) -> f32 {
        let t =
            now.saturating_duration_since(self.since).as_secs_f32() / DROP_BAR_SLIDE.as_secs_f32();
        if t >= 1. {
            return self.x;
        }
        let eased = 1. - (1. - t) * (1. - t);
        self.from + (self.x - self.from) * eased
    }

    fn settled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= DROP_BAR_SLIDE
    }
}

/// How long the drop bar takes to slide to a new boundary.
const DROP_BAR_SLIDE: Duration = Duration::from_millis(120);
/// The drop bar's width; it straddles the boundary between two tabs.
const DROP_BAR_WIDTH: f32 = 3.;

/// The ghost that follows the pointer while a tab is dragged: the tab's title on a
/// card placed at the cursor's bottom right, so it covers neither strip nor bar.
/// gpui draws it where the drag started less `grab`; the card is offset past that.
struct TabDragPreview {
    title: SharedString,
    /// Where in the tab the press landed.
    grab: gpui::Point<gpui::Pixels>,
    /// The window it was picked up in; gpui paints the active drag in every window,
    /// so others would show a frozen ghost (seen on X11).
    source: gpui::AnyWindowHandle,
}

/// From the pointer's hotspot to the ghost's top left: the grabbing hand's bottom right corner.
const DRAG_GHOST_GAP: f32 = 8.;

impl Render for TabDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.window_handle() != self.source {
            return div().into_any_element();
        }
        let theme = cx.global::<Theme>().clone();
        // Padding on a wrapper, not a margin: the layout drops a root element's margin.
        div()
            .pl(self.grab.x + px(DRAG_GHOST_GAP))
            .pt(self.grab.y + px(DRAG_GHOST_GAP))
            // The drag's view paints last, and a cursor asked for without a hitbox
            // wins; this is where the grabbing hand comes from.
            .child(canvas(
                |_, _, _| (),
                |_, _, window, _| window.set_window_cursor_style(CursorStyle::ClosedHand),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(34.))
                    .px(px(10.))
                    .max_w(px(200.))
                    .rounded(px(6.))
                    .bg(theme.tokens.background_primary)
                    .border_1()
                    .border_color(theme.tokens.background_modifier_border)
                    .shadow_md()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.tokens.text_normal)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.title.clone()),
            )
            .into_any_element()
    }
}

/// The drag's on-screen stand-in: nothing, the divider moves in place.
struct NoDragPreview;

impl Render for NoDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The payload of a sidebar resize: which dock's inner edge is being dragged.
#[derive(Debug, Clone)]
struct DockDrag {
    left: bool,
}

/// The narrowest a sidebar can be dragged, and the narrowest recorded `width` is believed.
const MIN_SIDEBAR_WIDTH: f64 = 150.;

/// The margin around the content when TFT draws the frame: the shadow lives in it
/// and the pointer resizes from it. Obsidian's hidden frame measures 23 px on the
/// sides; gpui's inset is one value, so this is the side margin.
const FRAME_SHADOW: Pixels = px(24.);

/// The frame shadow's gaussian sigma (`blur_radius`). Obsidian fits sigma 13 with a
/// 12 px downward offset; this is as dark at the edge and fades before the margin,
/// since a tail cut off by the margin reads as a band.
const FRAME_SHADOW_BLUR: Pixels = px(11.);

/// How far the frame shadow sits below the content.
const FRAME_SHADOW_OFFSET: Pixels = px(6.);

/// The frame shadow's colour: black at 32 %, Obsidian's at the sides.
const FRAME_SHADOW_COLOR: u32 = 0x00000052;

/// The radius of a window corner TFT draws; Obsidian's hidden frame is about 10 px here.
const WINDOW_RADIUS: Pixels = px(10.);

/// Obsidian's `--frame-left-space` on macOS: room for the traffic lights (19, 12)
/// past the ribbon in the window's top-left header.
const MAC_FRAME_LEFT_SPACE: f32 = 80.;

/// Which window corners are rounded: each that meets no tiled or maximised edge,
/// when the frame is TFT's. gpui clips children to rectangles, so every element
/// reaching a corner rounds itself with [`WindowCorners::apply`].
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct WindowCorners {
    tl: bool,
    tr: bool,
    bl: bool,
    br: bool,
}

impl WindowCorners {
    pub(crate) fn of(window: &Window) -> WindowCorners {
        match window.window_decorations() {
            Decorations::Client { tiling } => WindowCorners {
                tl: !(tiling.top || tiling.left),
                tr: !(tiling.top || tiling.right),
                bl: !(tiling.bottom || tiling.left),
                br: !(tiling.bottom || tiling.right),
            },
            Decorations::Server => WindowCorners::default(),
        }
    }

    /// The left edge's corners only, for the ribbon or the left sidebar.
    pub(crate) fn left(self) -> WindowCorners {
        WindowCorners {
            tr: false,
            br: false,
            ..self
        }
    }

    /// The right edge's corners only, for the right sidebar.
    pub(crate) fn right(self) -> WindowCorners {
        WindowCorners {
            tl: false,
            bl: false,
            ..self
        }
    }

    /// The top edge's corners only, for a title row.
    pub(crate) fn top(self) -> WindowCorners {
        WindowCorners {
            bl: false,
            br: false,
            ..self
        }
    }

    /// The bottom edge's corners only, for whatever sits under a title row.
    pub(crate) fn bottom(self) -> WindowCorners {
        WindowCorners {
            tl: false,
            tr: false,
            ..self
        }
    }

    pub(crate) fn apply<T: Styled>(self, mut el: T) -> T {
        if self.tl {
            el = el.rounded_tl(WINDOW_RADIUS);
        }
        if self.tr {
            el = el.rounded_tr(WINDOW_RADIUS);
        }
        if self.bl {
            el = el.rounded_bl(WINDOW_RADIUS);
        }
        if self.br {
            el = el.rounded_br(WINDOW_RADIUS);
        }
        el
    }
}

/// A 28 by 24 icon button of the kind Obsidian calls a clickable icon:
/// 4 px above and below the icon, 6 px either side, a 4 px radius and a
/// wash behind it on hover.
/// Makes the sandbox vault at `root`: a welcome note that links to a
/// second, so the editor, the links and the graph have something to show.
/// The folder is new each time, so nothing already there is written over.
fn write_sandbox(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::write(
        root.join("Welcome.md"),
        "---\ntags:\n  - sandbox\n---\n# Welcome to the sandbox\n\nThis vault is a scratch folder: try things here and nothing you keep is \
touched. Link to [[Another note]], press Ctrl+O to switch notes, Ctrl+P \
for the command palette, and Ctrl+E to flip between editing and reading.\n\n\
- [ ] A task to tick\n- A list item with **bold** and *italic* text\n\n\
> [!tip] Callouts\n> Obsidian's callouts render here too.\n",
    )?;
    std::fs::write(
        root.join("Another note.md"),
        "# Another note\n\nIt links back to [[Welcome]], which the backlinks pane shows.\n",
    )
}

/// What an empty tab offers, in Obsidian's order: element id, pill label and command.
/// What must match is the text and that each pill runs the command it names.
const EMPTY_TAB_ACTIONS: [(&str, &str, &str); 3] = [
    ("empty-new-note", "New note", "file-explorer:new-file"),
    ("empty-new-canvas", "New canvas", "canvas:new-file"),
    ("empty-go-to-file", "Go to file", "switcher:open"),
];

/// The path a file or folder at `from` takes when a rename field's `value` becomes
/// its name, or why the filesystem would refuse it. A field holds the stem, so both
/// the stem and the full name are validated; one validator for both rename paths.
fn renamed_path(from: &str, value: &str, is_folder: bool) -> Result<String, String> {
    let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let name = from.rsplit('/').next().unwrap_or(from);
    let ext = if is_folder {
        None
    } else {
        name.rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|_| basename(name) != name)
    };
    validate_renamed_file(value, ext).map_err(|err| err.to_string())?;
    let new_name = match ext {
        Some(ext) => format!("{value}.{ext}"),
        None => value.to_string(),
    };
    Ok(if folder.is_empty() {
        new_name
    } else {
        format!("{folder}/{new_name}")
    })
}

/// Whether the view header carries the mode switch: no on an empty tab (nothing to
/// read or edit), and the graph has its own toggle.
fn header_shows_mode_switch(current: &Option<String>, mode: ViewMode) -> bool {
    current.is_some() && mode != ViewMode::Graph
}

/// What the status bar reads on an empty tab; it stays and reads as zero.
const EMPTY_TAB_STATUS: [&str; 1] = ["0 properties"];

/// One pill of the empty tab.
struct EmptyAction {
    id: &'static str,
    label: &'static str,
    /// The command's first hotkey as `hotkeys.json` writes it, from the registry,
    /// or `None` for a command with none.
    hotkey: Option<String>,
    command: &'static str,
}

impl EmptyAction {
    /// The pill's text: the label, then the hotkey when there is one.
    fn text(&self) -> String {
        match &self.hotkey {
            Some(keys) => format!("{} {keys}", self.label),
            None => self.label.to_string(),
        }
    }
}

/// The pills an empty tab draws; a command the registry does not have is left out
/// rather than drawn dead.
fn empty_tab_actions(registry: &CommandRegistry) -> Vec<EmptyAction> {
    EMPTY_TAB_ACTIONS
        .iter()
        .filter_map(|(id, label, command)| {
            let known = registry.get(command)?;
            Some(EmptyAction {
                id,
                label,
                hotkey: known.hotkeys.first().map(hotkey_label),
                command,
            })
        })
        .collect()
}

/// Makes `area` a handle on the window when the frame is TFT's: a press drags,
/// a double click maximises or restores, a right click opens the window menu.
/// Windows uses the hit test (`WindowControlArea::Drag`) instead, since asking for
/// both would stop the system dragging at all.
/// A press on a macOS drag area that has not moved yet; a window has one pointer.
static MAC_DRAG_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn window_handle(
    area: gpui::Stateful<gpui::Div>,
    frameless: bool,
) -> gpui::Stateful<gpui::Div> {
    if !frameless {
        return area;
    }
    if cfg!(target_os = "windows") {
        return area.window_control_area(WindowControlArea::Drag);
    }
    if cfg!(target_os = "macos") {
        // AppKit's drag session swallows the click it starts from, so the drag
        // begins on the first move and a double click still arrives as one; macOS has no window menu.
        return area
            .on_mouse_down(MouseButton::Left, |_, _, _| {
                MAC_DRAG_PENDING.store(true, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_up(MouseButton::Left, |_, _, _| {
                MAC_DRAG_PENDING.store(false, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_down_out(|_, _, _| {
                MAC_DRAG_PENDING.store(false, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_move(|_, window, _| {
                if MAC_DRAG_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    window.start_window_move();
                }
            })
            .on_click(|e, window, _| {
                if e.click_count() == 2 {
                    window.titlebar_double_click();
                }
            });
    }
    area.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .on_click(|e, window, _| {
            if e.click_count() == 2 {
                window.zoom_window();
            }
        })
        .on_mouse_down(MouseButton::Right, |e, window, _| {
            window.show_window_menu(e.position)
        })
}

/// Which edge or corner of the window `pos` is over, in the shadow margin of a non-tiled side.
fn resize_edge(
    pos: gpui::Point<Pixels>,
    size: gpui::Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let m = FRAME_SHADOW;
    let top = pos.y < m && !tiling.top;
    let bottom = pos.y > size.height - m && !tiling.bottom;
    let left = pos.x < m && !tiling.left;
    let right = pos.x > size.width - m && !tiling.right;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (true, _, _, _) => ResizeEdge::Top,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (_, true, _, _) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

/// Whether TFT draws the window's title row itself. On Linux this is whatever the
/// compositor granted (`Decorations::Client`); Windows and macOS answer from the
/// transparent-title-bar request. On macOS the row holds the system traffic lights.
pub(crate) fn frameless(window: &Window) -> bool {
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        crate::hidden_frame()
    } else {
        matches!(window.window_decorations(), Decorations::Client { .. })
    }
}

/// The strip at the head of a column running into the window's top-left corner,
/// beside a [`window_title_row`]: the title row's height on macOS where the
/// traffic lights sit, `None` elsewhere or in fullscreen. It drags the window.
pub(crate) fn traffic_light_strip(window: &Window) -> Option<gpui::Stateful<gpui::Div>> {
    (cfg!(target_os = "macos") && frameless(window) && !window.is_fullscreen()).then(|| {
        window_handle(
            div()
                .id("traffic-light-strip")
                .flex_shrink_0()
                .w_full()
                .h(px(HEADER_HEIGHT)),
            true,
        )
    })
}

/// TFT's own frame for a desktop-bare window: a margin of [`FRAME_SHADOW`] holding
/// a soft shadow and resize handles, a one pixel border, and no margin on tiled or
/// maximised edges. With the desktop's frame the content passes through untouched.
pub(crate) fn client_frame(
    content: gpui::AnyElement,
    theme: &Theme,
    window: &mut Window,
) -> gpui::Stateful<gpui::Div> {
    let decorations = window.window_decorations();
    let tiling = match decorations {
        Decorations::Client { tiling } => tiling,
        Decorations::Server => Tiling::default(),
    };
    let client = matches!(decorations, Decorations::Client { .. });
    window.set_client_inset(if client { FRAME_SHADOW } else { px(0.) });
    let border = theme.tokens.background_modifier_border;
    let shadow: Hsla = rgba(FRAME_SHADOW_COLOR).into();
    let corners = WindowCorners::of(window);
    div()
        .id("window-backdrop")
        .size_full()
        .bg(transparent_black())
        .map(|d| corners.apply(d))
        .when(client, |d| {
            d.when(!tiling.top, |d| d.pt(FRAME_SHADOW))
                .when(!tiling.bottom, |d| d.pb(FRAME_SHADOW))
                .when(!tiling.left, |d| d.pl(FRAME_SHADOW))
                .when(!tiling.right, |d| d.pr(FRAME_SHADOW))
                .child(
                    canvas(
                        |_, window, _| {
                            window.insert_hitbox(
                                gpui::Bounds::new(
                                    point(px(0.), px(0.)),
                                    crate::zoom::window_size(window),
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_, hitbox, window, _| {
                            let size = crate::zoom::window_size(window);
                            let Some(edge) = resize_edge(window.mouse_position(), size, tiling)
                            else {
                                return;
                            };
                            let style = match edge {
                                ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                ResizeEdge::Left | ResizeEdge::Right => {
                                    CursorStyle::ResizeLeftRight
                                }
                                ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                    CursorStyle::ResizeUpLeftDownRight
                                }
                                ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                    CursorStyle::ResizeUpRightDownLeft
                                }
                            };
                            window.set_cursor_style(style, &hitbox);
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .on_mouse_move(|_, window, _| window.refresh())
                .on_mouse_down(MouseButton::Left, move |e, window, _| {
                    let size = crate::zoom::window_size(window);
                    if let Some(edge) = resize_edge(e.position, size, tiling) {
                        window.start_window_resize(edge);
                    }
                })
        })
        .child(
            div()
                .size_full()
                .cursor(CursorStyle::Arrow)
                .bg(theme.tokens.background_primary)
                .map(|d| corners.apply(d))
                .when(client, |d| {
                    d.border_color(border)
                        .when(!tiling.top, |d| d.border_t_1())
                        .when(!tiling.bottom, |d| d.border_b_1())
                        .when(!tiling.left, |d| d.border_l_1())
                        .when(!tiling.right, |d| d.border_r_1())
                        .when(!tiling.is_tiled(), |d| {
                            d.shadow(vec![BoxShadow {
                                color: shadow,
                                blur_radius: FRAME_SHADOW_BLUR,
                                spread_radius: px(0.),
                                offset: point(px(0.), FRAME_SHADOW_OFFSET),
                            }])
                        })
                })
                // The margin's own listeners see only what the content
                // lets through, so the content keeps the pointer's moves.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
}

/// The frame's margin a popout's surface carries on each side beyond its
/// inner size: with TFT's own frame on Linux the shadow is drawn inside
/// the window, so the window is asked for that much more than the inner
/// size `workspace.json` records, and the recorder takes the same margin
/// back off (see [`Workspace::render_popout`]). It follows what TFT
/// asked for, not the frame the compositor settled on: that is not known
/// until some frames in, and a window opened and recorded by the same
/// rule reopens at the size it closed at whichever frame it got.
pub(crate) fn popout_margin() -> Pixels {
    if crate::hidden_frame() && cfg!(target_os = "linux") {
        FRAME_SHADOW
    } else {
        px(0.)
    }
}

/// The surface asked for a popout whose inner size is `width` × `height`.
fn popout_outer_size(width: f32, height: f32) -> gpui::Size<Pixels> {
    let margin = popout_margin() * 2.;
    gpui::size(px(width) + margin, px(height) + margin)
}

/// Where a popout opens: at the place `workspace.json` recorded for it,
/// maximized when it was, and centered at that size when the recorded
/// position is on no display any more, as after a monitor was unplugged.
/// With nothing recorded it is Obsidian's popout size, centered.
fn popout_placement(recorded: Option<(WindowBounds, bool)>, cx: &App) -> gpui::WindowBounds {
    let Some((recorded, maximized)) = recorded else {
        let (width, height) = crate::popout::INNER_SIZE;
        return gpui::WindowBounds::Windowed(Bounds::centered(
            None,
            popout_outer_size(width, height),
            cx,
        ));
    };
    let size = popout_outer_size(recorded.width as f32, recorded.height as f32);
    let origin = gpui::point(px(recorded.x as f32), px(recorded.y as f32));
    let on_screen = cx.displays().iter().any(|d| d.bounds().contains(&origin));
    let bounds = if on_screen {
        Bounds::new(origin, size)
    } else {
        Bounds::centered(None, size, cx)
    };
    if maximized {
        gpui::WindowBounds::Maximized(bounds)
    } else {
        gpui::WindowBounds::Windowed(bounds)
    }
}

/// The frame-related window options every window of TFT's opens with,
/// the main one included in spirit: the desktop's frame when the frame
/// style is `native`, TFT's own otherwise, with the transparent margin
/// the shadow needs on Linux and the hidden caption on Windows and macOS.
/// A caller sets its bounds and minimum size over these.
pub(crate) fn frame_window_options(title: &'static str) -> gpui::WindowOptions {
    let hidden = crate::hidden_frame();
    let decorations = if hidden {
        gpui::WindowDecorations::Client
    } else {
        gpui::WindowDecorations::Server
    };
    gpui::WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(title.into()),
            // What hides the caption on Windows and macOS, where
            // `window_decorations` is not read at all; see `main.rs`.
            appears_transparent: (cfg!(target_os = "windows") || cfg!(target_os = "macos"))
                && hidden,
            traffic_light_position: (cfg!(target_os = "macos") && hidden)
                .then(|| point(px(19.), px(12.))),
        }),
        // On macOS the hidden frame keeps the system's title bar, see-through,
        // over the top 28 px of the content, and a movable window is dragged
        // by any press there: the tab under it never saw its own drag start.
        // TFT moves the window itself from its drag areas
        // (`window_handle`), which a window that is not movable still obeys.
        is_movable: !(cfg!(target_os = "macos") && hidden),
        // KWin ties the activation token to the application by this id.
        app_id: Some("tft".into()),
        window_decorations: Some(decorations),
        // The shadow's margin has to be see-through, and only Linux draws
        // one: Windows keeps the system's shadow outside the window.
        window_background: match decorations {
            gpui::WindowDecorations::Client if !cfg!(target_os = "windows") => {
                gpui::WindowBackgroundAppearance::Transparent
            }
            _ => gpui::WindowBackgroundAppearance::Opaque,
        },
        ..Default::default()
    }
}

/// The three window buttons at the top right of a frameless window:
/// minimise, maximise or restore, and close, each `RIBBON_WIDTH` wide
/// and `HEADER_HEIGHT` tall, the close one red on hover and rounded with
/// the window's corner. `close` is what the close button does, since a
/// window with an open text field has a write to make on the way out.
/// On Windows the buttons name their areas instead and the system acts.
pub(crate) fn window_controls(
    theme: &Theme,
    window: &Window,
    corners: WindowCorners,
    close: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::Div {
    /// What a window button does when clicked.
    type Act = Box<dyn Fn(&mut Window, &mut App)>;
    let maximized = window.is_maximized();
    let system = cfg!(target_os = "windows");
    let button =
        |id: &'static str, name: &'static str, close: bool, area: WindowControlArea, act: Act| {
            let hover_bg: Hsla = if close {
                rgb(0xe93147).into()
            } else {
                theme.tokens.background_modifier_hover
            };
            let color = theme.tokens.text_muted;
            div()
                .id(id)
                .group(id)
                .w(px(RIBBON_WIDTH))
                .h(px(HEADER_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(close && corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
                .hover(move |s| s.bg(hover_bg))
                .when(system, |d| d.occlude().window_control_area(area))
                .when(!system, |d| {
                    d.on_click(move |_, window, cx| act(window, cx))
                })
                .child(
                    div()
                        .when(close, |d| {
                            d.group_hover(id, |s| s.text_color(gpui::white()))
                        })
                        .text_color(color)
                        .child(icon(name, 14., color)),
                )
        };
    div()
        .absolute()
        .top_0()
        .right_0()
        .flex()
        .flex_row()
        .child(button(
            "win-minimize",
            "minus",
            false,
            WindowControlArea::Min,
            Box::new(|w, _| w.minimize_window()),
        ))
        .child(button(
            "win-maximize",
            if maximized { "copy" } else { "square" },
            false,
            WindowControlArea::Max,
            Box::new(|w, _| w.zoom_window()),
        ))
        .child(button(
            "win-close",
            "x",
            true,
            WindowControlArea::Close,
            Box::new(close),
        ))
}

/// A title row for a window that is not the main one - the vault manager,
/// Settings - when the frame is TFT's: `HEADER_HEIGHT` of `bg`
/// that drags the window, with `title` centred on it the way Obsidian's
/// `.titlebar-text` is, the window buttons at its right, or on macOS the
/// desktop's traffic lights at its left. `bg` is `--titlebar-background`
/// where the row is meant to read as a bar, and the window's own ground
/// where it is not. With the desktop's frame there is no row: the
/// desktop's title bar, with the same title, is above the window instead.
pub(crate) fn window_title_row(
    title: &'static str,
    bg: Hsla,
    theme: &Theme,
    window: &Window,
    close: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<gpui::Stateful<gpui::Div>> {
    if !frameless(window) {
        return None;
    }
    let corners = WindowCorners::of(window);
    let mac = cfg!(target_os = "macos");
    // The handle is a sibling of the buttons: a press inside a button would move
    // the window and swallow the click.
    let handle = window_handle(
        div()
            .id("window-title-drag")
            .flex_1()
            .h_full()
            .when(mac, |d| d.pl(px(MAC_FRAME_LEFT_SPACE))),
        true,
    );
    Some(
        div()
            .id("window-title-row")
            .relative()
            .flex_shrink_0()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .flex()
            .flex_row()
            .bg(bg)
            .map(|d| corners.top().apply(d))
            // The title is centred on the row and painted under the handle and
            // buttons, with no hitbox, so a press still drags the window.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.tokens.text_muted)
                    .child(title),
            )
            .child(handle)
            .when(!mac, |d| {
                d.child(
                    div()
                        .relative()
                        .flex_shrink_0()
                        .w(px(3. * RIBBON_WIDTH))
                        .h_full()
                        .child(window_controls(theme, window, corners, close)),
                )
            }),
    )
}

/// How many closed tabs `workspace:undo-close-pane` can bring back.
const CLOSED_TABS_KEPT: usize = 20;

/// How many paths `workspace.json`'s `lastOpenFiles` keeps.
const LAST_OPEN_FILES_KEPT: usize = 10;

pub struct Workspace {
    focus_handle: FocusHandle,
    /// The keyboard of the dock's Tags, All properties and Outline trees (Obsidian's active side leaf).
    dock_tree_focus: FocusHandle,
    /// Whether that handle held focus at the last draw, when the focused row wears its ring.
    dock_tree_keyboard: bool,
    /// The desktop's colour scheme as the window last reported it, which `theme: "system"` follows.
    system_dark: bool,
    /// `theme:toggle-light-dark` under `theme: "system"`: Obsidian writes nothing and
    /// flips only the session's scheme, so this turns the desktop's answer over.
    scheme_flipped: bool,
    /// Set when the main area changed and the keyboard has to follow; honoured in `render`.
    refocus_main: bool,
    state: Option<Arc<VaultState>>,
    vault_name: SharedString,
    /// Whether the vault goes on the switcher's list: a sandbox does not.
    record_vault: bool,
    /// The vault switcher's menu, dropped up from the sidebar's footer.
    vault_menu_open: bool,
    /// Whether the pointer is over the switcher row: the menu closes on mouse-down
    /// outside, and the row's own press would otherwise reopen it.
    vault_switcher_hovered: bool,
    /// The vault manager while one is up, the same way.
    vault_window: Option<gpui::WindowHandle<VaultManager>>,
    /// The Settings window while one is up. Settings is a window, not a modal, so
    /// the vault behind stays live.
    settings_window: Option<gpui::WindowHandle<SettingsWindow>>,
    loading: Option<SharedString>,
    explorer: Entity<FileExplorer>,
    search: Entity<SearchPane>,
    switcher: Entity<QuickSwitcher>,
    chooser: Entity<Chooser>,
    /// What the chooser's pick means while it is open.
    chooser_purpose: ChooserPurpose,
    /// The "Download attachments" modal while it is up.
    downloads: Option<attachments::DownloadsModal>,
    /// "Frontmatter migration" while it is up.
    migrate: Option<migrate_frontmatter::MigrateState>,
    /// What the last migration wrote, for its notice's Undo.
    migration_undo: Vec<migrate_frontmatter::Migrated>,
    /// A canvas's note/media picker request; the next render opens it with the window.
    pending_canvas_pick: Option<crate::canvas::FilePick>,
    settings: Entity<SettingsView>,
    prompt: Entity<Prompt>,
    copy_results: Entity<CopyResults>,
    /// "Debug info", filled each time it opens.
    debug_info: Entity<DebugInfo>,
    /// "Export to PDF", filled each time it opens.
    pdf_export: Entity<PdfExport>,
    /// "Export as image", at its defaults each time it opens.
    image_export: Entity<ImageExport>,
    /// The reading view release-notes tabs draw, and the version whose notes it holds.
    release_notes: Entity<ReadingView>,
    release_notes_built: RefCell<Option<String>>,
    pending: Option<PendingOp>,
    modal: Modal,
    /// Command ids run this session, most recent first.
    recent_commands: Vec<String>,
    /// The notes' fold records, kept outside the vault and opened on first use.
    fold_store: Option<(PathBuf, fold::FoldStore)>,
    left_visible: bool,
    /// The sidebar's width in logical pixels (`left.width`); a new vault's is 300.
    left_width: f64,
    /// Whether the window has TFT's own frame, read at each render for the window
    /// buttons and drag areas.
    frameless: bool,
    /// Whether the window fills the screen, read with `frameless`: macOS then drops the traffic lights' space.
    fullscreen: bool,
    /// The window corners rounded this frame, read at the top of `render`.
    corners: WindowCorners,
    left_tab: LeftTab,
    reading: Entity<ReadingView>,
    editor: Entity<Editor>,
    /// The active group's canvas view, showing the open file when it is a `.canvas`.
    canvas: Entity<CanvasView>,
    /// The active group's PDF view, showing the open file when it is a `.pdf`.
    pdf: Entity<PdfView>,
    graph: Entity<GraphView>,
    /// The mode to return to when the graph closes.
    mode_before_graph: ViewMode,
    mode: ViewMode,
    current: Option<String>,
    /// The note whose file went away while its buffer was on screen: the text stays
    /// (it may hold unsaved edits) but nothing writes it back.
    detached: Option<SharedString>,
    history: Vec<HistoryEntry>,
    history_ix: usize,
    /// The main area's tabs; the active one's fields are the live ones above.
    tabs: Vec<Tab>,
    active_tab: usize,
    /// Tabs closed this session with the index they had, oldest first.
    closed_tabs: Vec<(usize, Tab)>,
    /// Every tab group, the active one's slot holding what it last stashed.
    groups: Vec<TabGroup>,
    active_group: usize,
    /// How the groups are split, as `workspace.json`'s `main` records it.
    layout_tree: Layout,
    /// The popout windows `floating` records; their groups are in `groups` but not `layout_tree`.
    windows: Vec<WindowNode>,
    /// The main window: where `layout_tree`'s groups are, and where the keyboard goes for them.
    main_window: gpui::AnyWindowHandle,
    /// The vault window's zoom level (see [[Specification#Window zoom]]), stepped by the
    /// three zoom commands and kept in TFT's window file.
    zoom_level: f32,
    /// The layout `app:reload` took down, restored once the vault is read again.
    reload_layout: Option<Value>,
    /// The popout whose groups are being drawn, while [`Workspace::render_popout`]
    /// runs; `None` while the main window renders. The group renderers
    /// read it for the tree their group is in.
    rendering_window: Option<u64>,
    /// The next group's id; a window takes its id from the same counter.
    next_group_id: u64,
    /// Group ids, most recently active first: where focus goes when a group
    /// closes, and which group a focus command lands in.
    group_mru: Vec<u64>,
    /// A tab is being dragged, so every group paints its drop zones. Set
    /// when the drag starts and cleared when the next frame finds no drag
    /// under way, however the drag ended.
    dragging_tab: bool,
    /// The tab being dragged, for a drop in another window: the release
    /// reaches only the window the drag started in, where gpui's own drop
    /// targets are not under the pointer.
    tab_drag: Option<TabDrag>,
    /// Where every strip, tab and group content was laid out during a drag, for the
    /// cross-window hit test (see [`Workspace::cross_window_drag_moved`]).
    drag_bounds: Rc<RefCell<DragBounds>>,
    /// Which group's content the dragged tab is over, and where in it.
    tab_drop: Option<(u64, DropZone)>,
    /// The accent bar between two tabs of the strip the dragged tab is over.
    drop_bar: Option<DropBar>,
    /// Where each tab was on the last drag move, to find the nearest boundary.
    tab_bounds: HashMap<(u64, usize), gpui::Bounds<gpui::Pixels>>,
    /// The keyboard while the view-header title is typed into; its own handle, so the
    /// title only takes keys while a click has put the caret in it.
    title_focus: FocusHandle,
    /// The header title being renamed in place, if any.
    title_edit: Option<TitleEdit>,
    /// A name the inline title asked for, waiting for a frame: the subscription has
    /// no window and a rename may raise the "Update links?" prompt.
    pending_title_rename: Option<String>,
    /// What a slash popup asked for, waiting for a frame: the palette and commands need the window.
    pending_slash: Vec<(Entity<Editor>, Option<String>)>,
    /// Files and images pasted or dropped onto an editor, waiting: the embeds need the window.
    pending_attachments: Vec<(Entity<Editor>, IncomingAttachments)>,
    /// A slash popup's command is running, so it is not recorded as a recent command.
    running_from_slash: bool,
    /// Where each group's header title was painted and shaped, for measuring a click.
    title_layout: HashMap<u64, inline_field::Layout>,
    /// `workspace.json` as read at load (`None` if absent), which the quit write is merged into.
    layout: Option<Value>,
    /// A `workspace.json` was read, so startup flags leave the first note unopened.
    layout_restored: bool,
    /// `lastOpenFiles`: the notes opened, most recent first.
    last_open_files: Vec<String>,
    /// What the window's title bar says, set only when it changes.
    window_title: SharedString,
    /// The next link followed opens in a new tab (Ctrl+click or `editor:open-link-in-new-leaf`).
    link_in_new_tab: bool,
    /// Page preview: the link under the pointer and the popover up for it.
    hover_preview: crate::hover_preview::HoverPreview,
    /// An editing popover's autosave, pending.
    preview_save_task: Option<gpui::Task<()>>,
    /// The file menu a click on a tab, the editor or a link opened: where, and on what.
    file_menu: Option<file_menu::Menu>,
    /// The keyboard the open menu holds, and where it goes back to when the menu closes.
    menu_focus: file_menu::MenuFocus,
    /// The window that took the menu's keyboard (`None` vault), whose render gives it back.
    menu_window: Option<u64>,
    /// The window the modal or file menu draws in, latched when it goes up; `None`
    /// while nothing is up, `Some(None)` for the vault window.
    overlay_window: Option<Option<u64>>,
    /// What the status bar says: it stays true until something makes it untrue.
    status: SharedString,
    notices: Notices,
    _watcher: Option<watch::Watcher>,
    /// Pending autosave, two seconds after the last edit.
    save_task: Option<Task<()>>,
    /// Whether the startup flags have been applied: a control client needs the window
    /// to have stopped arranging itself.
    started: bool,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What the workspace listens to on a group's views; a click makes its group active
/// before the view handles it, so the views that send events are the live ones.
fn view_subscriptions(
    editor: &Entity<Editor>,
    reading: &Entity<ReadingView>,
    canvas: &Entity<CanvasView>,
    pdf: &Entity<PdfView>,
    cx: &mut Context<Workspace>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe(pdf, |this, pdf, event: &PdfEvent, cx| match event {
            PdfEvent::Report(message) => this.report(message.clone(), cx),
            PdfEvent::CopyLink {
                subpath,
                alias,
                quote,
            } => {
                let Some(path) = pdf.read(cx).path().map(str::to_string) else {
                    return;
                };
                this.copy_pdf_link(&path, subpath, alias, quote.as_deref(), cx);
            }
        }),
        cx.subscribe(
            canvas,
            |this, canvas, event: &CanvasEvent, cx| match event {
                CanvasEvent::Edited => this.schedule_save(cx),
                CanvasEvent::OpenLink(target) => this.follow_link(target.clone(), cx),
                CanvasEvent::Viewport(view_state) => {
                    this.record_canvas_view_state(&canvas, view_state.clone(), cx)
                }
                // The chooser needs a window to take focus, which this subscription
                // lacks; the next render opens it.
                CanvasEvent::PickFile(kind) => {
                    this.pending_canvas_pick = Some(*kind);
                    cx.notify();
                }
            },
        ),
        cx.subscribe(
            reading,
            |this, reading, event: &ReadingEvent, cx| match event {
                ReadingEvent::Hover(hover) => this.link_hover(
                    crate::hover_preview::Source::Reading,
                    reading.entity_id(),
                    hover.clone(),
                    cx,
                ),
                ReadingEvent::OpenLink(target) => this.follow_link(target.clone(), cx),
                ReadingEvent::OpenLinkInNewTab(target) => {
                    this.link_in_new_tab = true;
                    this.follow_link(target.clone(), cx);
                }
                ReadingEvent::ContextMenu { target, position } => {
                    this.open_reading_menu(target.clone(), *position, cx)
                }
            },
        ),
        cx.subscribe(reading, |this, _, event: &TaskToggled, cx| {
            this.toggle_task(&event.path, event.marker, cx)
        }),
        cx.subscribe(reading, |this, _, event: &PropertyItemRemoved, cx| {
            this.remove_property_item(&event.path, &event.key, event.index, cx)
        }),
        cx.subscribe(
            editor,
            |this, editor, event: &EditorEvent, cx| match event {
                EditorEvent::Hover(hover) => this.link_hover(
                    crate::hover_preview::Source::Editor,
                    editor.entity_id(),
                    hover.clone(),
                    cx,
                ),
                EditorEvent::Edited => {
                    this.schedule_save(cx);
                }
                EditorEvent::OpenLink(target) => this.follow_link(target.clone(), cx),
                EditorEvent::OpenLinkInNewTab(target) => {
                    this.link_in_new_tab = true;
                    this.follow_link(target.clone(), cx);
                }
                EditorEvent::PropertyTypeAssigned { key, ty } => {
                    this.assign_property_type(key.clone(), *ty, cx)
                }
                EditorEvent::ContextMenu {
                    position,
                    selection,
                    selected,
                    link,
                    spelling,
                } => this.open_editor_menu(
                    *position,
                    selection.clone(),
                    *selected,
                    link.clone(),
                    spelling.clone(),
                    cx,
                ),
                EditorEvent::RenameTo(value) => {
                    this.pending_title_rename = Some(value.clone());
                    cx.notify();
                }
                EditorEvent::Reported(message) => this.report(message.clone(), cx),
                EditorEvent::SlashCommands => {
                    this.pending_slash.push((editor.clone(), None));
                    cx.notify();
                }
                EditorEvent::RunCommand(id) => {
                    this.pending_slash.push((editor.clone(), Some(id.clone())));
                    cx.notify();
                }
                EditorEvent::Attachments(incoming) => {
                    this.pending_attachments
                        .push((editor.clone(), incoming.clone()));
                    cx.notify();
                }
            },
        ),
    ]
}

impl Workspace {
    pub fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Workspace {
        let explorer = cx.new(FileExplorer::new);
        let search = cx.new(SearchPane::new);
        let switcher = cx.new(QuickSwitcher::new);
        let chooser = cx.new(Chooser::new);
        let settings = cx.new(SettingsView::new);
        let prompt = cx.new(Prompt::new);
        let copy_results = cx.new(CopyResults::new);
        let debug_info = cx.new(DebugInfo::new);
        let pdf_export = cx.new(PdfExport::new);
        let image_export = cx.new(ImageExport::new);
        let release_notes = cx.new(ReadingView::new);
        let reading = cx.new(ReadingView::new);
        let editor = cx.new(|cx| crate::new_editor(window, cx));
        let canvas = cx.new(CanvasView::new);
        let pdf = cx.new(PdfView::new);
        let graph = cx.new(GraphView::new);
        let subscriptions = vec![
            cx.subscribe_in(&graph, window, |this, _, event: &GraphEvent, window, cx| {
                match event {
                    GraphEvent::Open(path) => {
                        let back = this.mode_before_graph;
                        this.set_mode(back, window, cx);
                        this.open_file(path.clone(), None, cx);
                    }
                    // A graph node previews its file on Mod by default (observed).
                    GraphEvent::Hover(Some(hover)) => {
                        let where_ = this.window_of_group(this.groups[this.active_group].id);
                        this.row_hover(
                            crate::hover_preview::Source::Graph,
                            where_,
                            crate::hover_preview::exact(&hover.path),
                            hover.path.clone(),
                            true,
                            hover.anchor,
                            hover.modifiers,
                            cx,
                        )
                    }
                    GraphEvent::Hover(None) => {
                        this.source_left(crate::hover_preview::Source::Graph, cx)
                    }
                }
            }),
            cx.subscribe_in(
                &explorer,
                window,
                |this, _, event: &ExplorerEvent, window, cx| match event {
                    ExplorerEvent::Open(path) => {
                        // A row's click or Enter hands the keyboard to the note it opened.
                        this.open_file(path.clone(), None, cx);
                        this.focus_main(window, cx);
                    }
                    ExplorerEvent::OpenInNewTab(path) => {
                        this.open_link_in_new_tab(path.clone(), None, cx)
                    }
                    ExplorerEvent::NewNote { folder } => this.dispatch_file_operation(
                        FileOperationRequest::CreateNote {
                            parent: folder.clone(),
                        },
                        window,
                        cx,
                    ),
                    ExplorerEvent::NewFolder { parent } => {
                        this.start_new_folder(parent, window, cx)
                    }
                    // Obsidian renames a tree row on the row, not in a
                    // dialog, and F2 and the menu's "Rename..." both do it
                    // (the 2026-10-25 pass, steps `s09` and `s11`).
                    ExplorerEvent::Rename(path) => {
                        let path = path.clone();
                        this.explorer.update(cx, |e, cx| e.start_rename(&path, cx));
                    }
                    ExplorerEvent::RenameTo { path, name } => {
                        this.commit_row_rename(path.clone(), name, window, cx)
                    }
                    ExplorerEvent::Move(path) => this.start_move(path, window, cx),
                    ExplorerEvent::MoveInto { path, folder } => {
                        this.drop_into_folder(path, folder, window, cx)
                    }
                    ExplorerEvent::Duplicate(path) => this.duplicate(path, cx),
                    ExplorerEvent::Delete(path) => this.start_delete(path, window, cx),
                    ExplorerEvent::NewCanvas { folder } => this.new_canvas(folder, cx),
                    ExplorerEvent::OpenToRight(path) => {
                        this.open_to_right(path.clone(), window, cx)
                    }
                    ExplorerEvent::OpenInNewWindow(path) => {
                        this.open_path_in_new_window(path.clone(), window, cx)
                    }
                    ExplorerEvent::CopyPath(path) => this.copy_path(path, cx),
                    ExplorerEvent::OpenInDefaultApp(path) => this.open_in_default_app(path, cx),
                    ExplorerEvent::ShowInSystemExplorer(path) => {
                        this.show_in_system_explorer(path, cx)
                    }
                    ExplorerEvent::SearchInFolder(folder) => {
                        this.search_in_folder(folder, window, cx)
                    }
                    ExplorerEvent::LeaveFocus => this.focus_main(window, cx),
                    ExplorerEvent::Hover {
                        path,
                        over,
                        anchor,
                        modifiers,
                    } => this.row_hover(
                        crate::hover_preview::Source::Files,
                        None,
                        crate::hover_preview::exact(path),
                        path.clone(),
                        *over,
                        *anchor,
                        *modifiers,
                        cx,
                    ),
                },
            ),
            cx.subscribe_in(
                &prompt,
                window,
                |this, _, event: &PromptEvent, window, cx| match event {
                    PromptEvent::Submit(value) => this.prompt_submitted(value, window, cx),
                    PromptEvent::Secondary => this.prompt_secondary(window, cx),
                    PromptEvent::Tertiary => this.prompt_tertiary(window, cx),
                    PromptEvent::Cancel => this.prompt_cancelled(window, cx),
                },
            ),
            cx.subscribe_in(
                &copy_results,
                window,
                |this, _, event: &CopyResultsEvent, window, cx| match event {
                    CopyResultsEvent::Copy(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        this.report("Results copied to your clipboard.", cx);
                    }
                    CopyResultsEvent::Close => this.close_modal(window, cx),
                },
            ),
            cx.subscribe_in(
                &debug_info,
                window,
                |this, _, event: &DebugInfoEvent, window, cx| match event {
                    DebugInfoEvent::Copy(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        this.close_modal(window, cx);
                        this.report("Debug info copied to your clipboard", cx);
                    }
                    DebugInfoEvent::Sandbox => {
                        this.close_modal(window, cx);
                        this.open_sandbox_vault(window, cx);
                    }
                    DebugInfoEvent::Close => this.close_modal(window, cx),
                },
            ),
            cx.subscribe_in(
                &pdf_export,
                window,
                |this, _, event: &PdfExportEvent, window, cx| {
                    this.on_pdf_export_event(event, window, cx)
                },
            ),
            cx.subscribe_in(
                &image_export,
                window,
                |this, _, event: &ImageExportEvent, window, cx| {
                    this.on_image_export_event(event, window, cx)
                },
            ),
            // The release notes' links: only ones to the web, which open
            // in the browser as a note's do.
            cx.subscribe(
                &release_notes,
                |this, _, event: &ReadingEvent, cx| match event {
                    ReadingEvent::OpenLink(target) | ReadingEvent::OpenLinkInNewTab(target) => {
                        this.follow_link(target.clone(), cx)
                    }
                    // Nothing there links to a note, so nothing previews.
                    ReadingEvent::ContextMenu { .. } | ReadingEvent::Hover(_) => {}
                },
            ),
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &SearchEvent, window, cx| match event {
                    SearchEvent::Open(path, offset) => {
                        this.open_file(path.clone(), None, cx);
                        if let Some(offset) = *offset {
                            this.reveal_offset(offset, cx);
                        }
                    }
                    SearchEvent::CopyResults(entries) => {
                        this.open_copy_results(entries.clone(), window, cx);
                    }
                    SearchEvent::Dismiss => this.focus_main(window, cx),
                    SearchEvent::StateChanged => {}
                    SearchEvent::ContextMenu {
                        path,
                        file_row,
                        position,
                    } => {
                        this.file_menu = Some(file_menu::Menu {
                            position: *position,
                            submenu: false,
                            anchor: Anchor::TopLeft,
                            target: file_menu::Target::Search {
                                path: path.clone(),
                                file_row: *file_row,
                            },
                        });
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &switcher,
                window,
                |this, _, event: &SwitcherEvent, window, cx| match event {
                    SwitcherEvent::Open(path) => {
                        this.close_modal(window, cx);
                        this.open_file(path.clone(), None, cx);
                    }
                    SwitcherEvent::Create(name) => {
                        this.close_modal(window, cx);
                        this.create_from_link(name, window, cx);
                    }
                    SwitcherEvent::Close => this.close_modal(window, cx),
                },
            ),
            cx.subscribe_in(
                &settings,
                window,
                |this, _, event: &SettingsEvent, _window, cx| match event {
                    SettingsEvent::Close => this.close_settings(cx),
                    SettingsEvent::Changed(change) => {
                        this.change_setting(change.as_ref().clone(), cx)
                    }
                },
            ),
            cx.subscribe_in(
                &chooser,
                window,
                |this, _, event: &ChooserEvent, window, cx| match event {
                    ChooserEvent::Chosen(value) => match this.chooser_purpose {
                        ChooserPurpose::CanvasFile => {
                            this.modal = Modal::None;
                            this.canvas
                                .update(cx, |c, cx| c.add_file_card_at_center(value, cx));
                            this.focus_main(window, cx);
                            cx.notify();
                        }
                        ChooserPurpose::CanvasGroup => {
                            this.modal = Modal::None;
                            this.canvas.update(cx, |c, cx| c.jump_to_group(value, cx));
                            this.focus_main(window, cx);
                            cx.notify();
                        }
                    },
                    ChooserEvent::Close => {
                        this.modal = Modal::None;
                        // Whatever the main area paints, not the editor:
                        // its handle is outside the dispatch tree in
                        // reading view and would swallow every key.
                        this.focus_main(window, cx);
                        cx.notify();
                    }
                },
            ),
        ];
        let mut subscriptions = subscriptions;
        // The desktop's focus moving to this window takes the active group
        // with it, as it does for a popout (see `window_activated`).
        subscriptions.push(cx.observe_window_activation(window, |ws, window, cx| {
            if window.is_window_active() {
                ws.window_activated(None, cx);
            }
        }));
        subscriptions.extend(view_subscriptions(&editor, &reading, &canvas, &pdf, cx));
        let groups = vec![TabGroup::new(
            0,
            editor.clone(),
            reading.clone(),
            canvas.clone(),
            pdf.clone(),
            ViewMode::Reading,
        )];
        // Closing the window must not lose edits waiting for the autosave.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(ws) = this.upgrade() {
                ws.update(cx, |ws, cx| ws.on_quit(cx));
            }
            true
        });
        let mut ws = Workspace {
            focus_handle: cx.focus_handle(),
            dock_tree_focus: cx.focus_handle(),
            dock_tree_keyboard: false,
            system_dark: true,
            scheme_flipped: false,
            refocus_main: false,
            state: None,
            vault_name: "No vault".into(),
            record_vault: true,
            vault_menu_open: false,
            vault_switcher_hovered: false,
            vault_window: None,
            settings_window: None,
            loading: None,
            explorer,
            search,
            switcher,
            chooser,
            chooser_purpose: ChooserPurpose::default(),
            downloads: None,
            migrate: None,
            migration_undo: Vec::new(),
            pending_canvas_pick: None,
            settings,
            prompt,
            copy_results,
            debug_info,
            pdf_export,
            image_export,
            release_notes,
            release_notes_built: RefCell::new(None),
            pending: None,
            modal: Modal::None,
            recent_commands: Vec::new(),
            fold_store: None,
            left_visible: true,
            left_width: SIDEBAR_WIDTH as f64,
            frameless: false,
            fullscreen: false,
            corners: WindowCorners::default(),
            left_tab: LeftTab::Files,
            reading,
            editor,
            canvas,
            pdf,
            graph,
            mode_before_graph: ViewMode::Reading,
            mode: ViewMode::Reading,
            current: None,
            detached: None,
            history: Vec::new(),
            history_ix: 0,
            tabs: vec![Tab::empty(ViewMode::Reading)],
            active_tab: 0,
            closed_tabs: Vec::new(),
            groups,
            active_group: 0,
            layout_tree: Layout::split(SplitDirection::Vertical, vec![Layout::Group(0)]),
            windows: Vec::new(),
            main_window: window.window_handle(),
            zoom_level: 0.0,
            reload_layout: None,
            rendering_window: None,
            next_group_id: 1,
            group_mru: vec![0],
            dragging_tab: false,
            tab_drag: None,
            drag_bounds: Rc::default(),
            tab_drop: None,
            drop_bar: None,
            tab_bounds: HashMap::new(),
            title_focus: cx.focus_handle(),
            title_edit: None,
            pending_title_rename: None,
            pending_slash: Vec::new(),
            pending_attachments: Vec::new(),
            running_from_slash: false,
            title_layout: HashMap::new(),
            layout: None,
            layout_restored: false,
            last_open_files: Vec::new(),
            window_title: SharedString::default(),
            link_in_new_tab: false,
            hover_preview: Default::default(),
            preview_save_task: None,
            file_menu: None,
            menu_focus: file_menu::MenuFocus::new(cx),
            menu_window: None,
            overlay_window: None,
            status: "Pass a vault folder as the first argument".into(),
            notices: Notices::default(),
            _watcher: None,
            save_task: None,
            started: false,
            _subscriptions: subscriptions,
        };
        if let Some(root) = root {
            ws.load_vault(root, window, cx);
        }
        ws
    }

    pub(crate) fn load_vault(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_vault_with(root, true, window, cx);
    }

    /// [`Self::load_vault`], saying whether the vault goes on the switcher's list.
    fn load_vault_with(
        &mut self,
        root: PathBuf,
        record: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_vault(cx);
        self.record_vault = record;
        self.vault_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into();
        self.loading = Some(format!("Indexing {}…", root.display()).into());
        self.status = "Indexing vault".into();
        // Restoring may make tab groups, and a group's editor needs the window, so the
        // vault comes back in it.
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { VaultState::load(&root) })
                .await;
            this.update_in(cx, |ws, window, cx| match result {
                Ok(state) => ws.set_state(state, window, cx),
                Err(err) => {
                    ws.loading = None;
                    ws.status = format!("Vault failed to open: {err}").into();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Puts the open vault away before another loads: saves its edits and layout,
    /// leaves one empty tab, and forgets `workspace.json` so the next vault starts blank.
    fn leave_vault(&mut self, cx: &mut Context<Self>) {
        if self.state.is_none() {
            return;
        }
        self.on_quit(cx);
        self.forget_vault(cx);
    }

    /// [`Self::leave_vault`] less the writes: every group and popout closed, the vault's state dropped.
    fn forget_vault(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.groups.iter().map(|g| g.id).collect();
        for id in ids {
            self.close_group(id, false, cx);
        }
        self.windows.clear();
        self.closed_tabs.clear();
        self.detached = None;
        self.layout = None;
        self.layout_restored = false;
        self.last_open_files.clear();
        self._watcher = None;
        self.state = None;
    }

    /// Records a property type in `types.json` and redraws the views from the new table.
    fn assign_property_type(
        &mut self,
        key: String,
        ty: tft_formats::properties::PropertyType,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        match state.assign_property_type(&key, ty) {
            Ok(types) => {
                self.reading
                    .update(cx, |r, _| r.set_property_types(types.clone()));
                self.editor.update(cx, |e, _| e.set_property_types(types));
                cx.notify();
            }
            Err(err) => {
                self.report(format!("types.json could not be written: {err}"), cx);
            }
        }
    }

    /// Pushes what `app.json` says into the open views; called when the vault loads
    /// and whenever Settings writes.
    fn apply_app_config(&mut self, state: &VaultState, cx: &mut Context<Self>) {
        let readable = crate::fixed::READABLE_LINE_LENGTH;
        let inline_title = crate::fixed::INLINE_TITLE;
        let properties = PropertiesInDocument::default();
        let (smart, use_tab, tab_size) = (
            crate::fixed::SMART_LISTS,
            crate::fixed::USE_TAB,
            crate::fixed::TAB_SIZE,
        );
        let build_options = state.build_options();
        let link_options = tft_suggest::Options {
            format: state.link_format(),
            markdown_links: false,
        };
        let slash = state.core_plugin_enabled("slash-command");
        // The editors watch the spellchecker and recheck when it changes.
        crate::spellcheck::set_enabled(state.app().spellcheck.unwrap_or(true), cx);
        let chrome = crate::gutter::Chrome::default();
        // Every group's views, the active one's among them.
        for group in &self.groups {
            group.reading.update(cx, |r, cx| {
                r.set_readable_line_length(readable);
                r.set_show_inline_title(inline_title);
                r.set_property_types(state.property_types().clone());
                r.set_properties_in_document(properties);
                r.set_fold_headings(crate::fixed::FOLD_HEADINGS);
                r.set_chrome(chrome);
                cx.notify();
            });
            group.editor.update(cx, |e, cx| {
                e.set_slash_enabled(slash, cx);
                e.set_link_options(link_options);
                e.set_readable_line_length(readable);
                e.set_show_inline_title(inline_title);
                e.set_indent_options(smart, use_tab, tab_size);
                e.set_property_types(state.property_types().clone());
                e.set_properties_in_document(properties);
                e.set_build_options(build_options, cx);
                e.set_chrome(chrome, cx);
            });
        }
    }

    /// What `app.json` says an editor does, for an editor that is not a group's.
    fn configure_editor(editor: &Entity<Editor>, state: &VaultState, cx: &mut App) {
        let properties = PropertiesInDocument::default();
        let link_options = tft_suggest::Options {
            format: state.link_format(),
            markdown_links: false,
        };
        let (smart, use_tab, tab_size) = (
            crate::fixed::SMART_LISTS,
            crate::fixed::USE_TAB,
            crate::fixed::TAB_SIZE,
        );
        let build_options = state.build_options();
        let chrome = crate::gutter::Chrome::default();
        editor.update(cx, |e, cx| {
            e.set_chrome(chrome, cx);
            e.set_link_options(link_options);
            e.set_indent_options(smart, use_tab, tab_size);
            e.set_property_types(state.property_types().clone());
            e.set_properties_in_document(properties);
            e.set_build_options(build_options, cx);
        });
    }

    /// Rebuilds the command registry and every key bound from it, with `hotkeys.json` applied.
    fn rebuild_commands(&mut self, state: &VaultState, cx: &mut Context<Self>) {
        let mut registry = CommandRegistry::with_known_ids();
        if let Ok(hotkeys) = state.vault().hotkeys() {
            registry.apply_overrides(&hotkeys);
        }
        let registry = Arc::new(registry);
        crate::bind_all_keys(cx, &registry);
        cx.set_global(Commands(registry));
    }

    fn change_setting(&mut self, change: SettingChange, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if let Err(err) = state.apply_setting(&change) {
            self.report(format!("{} could not be written: {err}", change.file()), cx);
            return;
        }
        // The base color scheme lives here: the theme is rebuilt at once
        // rather than when the write comes back through the watcher.
        if let SettingChange::Appearance(_) = &change {
            self.reload_theme(cx);
        }
        self.apply_app_config(&state, cx);
        self.refresh_vault_views(cx);
        cx.notify();
    }

    fn set_state(&mut self, state: VaultState, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = None;
        self.status = format!(
            "{} files, {} notes indexed in {:.0} ms",
            state.vault().files().len(),
            state.cache().len(),
            state.index_ms
        )
        .into();
        // A config that would not parse is being stood in for by its
        // defaults, and Settings will refuse to write it. Say so once, on
        // the way in, rather than leaving a tab that looks merely wrong.
        if !state.unreadable_configs.is_empty() {
            let names = state.unreadable_configs.join(", ");
            let one = state.unreadable_configs.len() == 1;
            let (verb, noun) = if one { ("is", "it") } else { ("are", "them") };
            self.report(
                format!(
                    "{names} {verb} not valid JSON; TFT is using defaults and will not overwrite {noun}"
                ),
                cx,
            );
        }
        let state = Arc::new(state);
        self.state = Some(state.clone());
        // The vault window comes back at the zoom it was left at.
        let level = zoom::load_level(&state.root);
        if level != self.zoom_level {
            self.zoom_level = level;
            self.apply_main_zoom(cx);
        }
        if self.record_vault {
            let root = state.root.clone();
            self.remember_vault(&root, cx);
        }
        for group in &self.groups {
            Self::give_editor_the_vault(&group.editor, &state, cx);
        }
        self.apply_app_config(&state, cx);
        self.refresh_vault_views(cx);
        self.start_watching(&state, cx);
        // The theme from the vault's files and the desktop's scheme: the
        // window opened on a theme read before it could say whether the
        // desktop is dark, and `set_system_dark`, run before the vault was
        // read, had no vault to rebuild it from.
        self.reload_theme(cx);
        if let Some(layout) = self.reload_layout.take() {
            // A reload: the commands and keys from the files as they are
            // now, and the layout as it was left.
            self.rebuild_commands(&state, cx);
            self.apply_layout(layout, &state, window, cx);
            self.focus_main(window, cx);
        } else if let Some(current) = self.current.clone() {
            self.open_file(current, None, cx);
        } else {
            self.restore_layout(&state, window, cx);
        }
        cx.notify();
    }

    /// Points an editor at the vault: what its embeds render from and what
    /// its autocompletion offers. Every group's editor gets this, the first
    /// when the vault opens and the rest as they are made.
    fn give_editor_the_vault(editor: &Entity<Editor>, state: &Arc<VaultState>, cx: &mut App) {
        let embed_source: Arc<dyn reading::EmbedSource + Send + Sync> = state.clone();
        let build_options = state.build_options();
        editor.update(cx, |e, cx| {
            e.set_vault(Some(embed_source), build_options, cx);
            e.set_suggestion_source(Some(Arc::new(
                crate::suggestion_source::DesktopSuggestionSource::new(state.clone()),
            )));
        });
    }

    /// "Add note from vault" and "Add media from vault": the chooser over
    /// the vault's notes, or its images, PDFs, audio and video; the pick
    /// becomes a file card at the middle of the canvas.
    fn open_canvas_file_chooser(
        &mut self,
        kind: crate::canvas::FilePick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let wanted = |k: FileKind| match kind {
            crate::canvas::FilePick::Note => k == FileKind::Markdown,
            crate::canvas::FilePick::Media => matches!(
                k,
                FileKind::Image | FileKind::Pdf | FileKind::Audio | FileKind::Video
            ),
        };
        let mut rows: Vec<ChooserRow> = state
            .vault()
            .files()
            .iter()
            .filter(|f| wanted(f.kind))
            .map(|f| ChooserRow {
                label: match kind {
                    crate::canvas::FilePick::Note => basename(&f.path).to_string(),
                    crate::canvas::FilePick::Media => f.path.clone(),
                },
                value: f.path.clone(),
                flair: None,
            })
            .collect();
        rows.sort_by(|a, b| a.label.cmp(&b.label));
        let (placeholder, empty) = match kind {
            crate::canvas::FilePick::Note => ("Type name of a note...", "No notes in the vault"),
            crate::canvas::FilePick::Media => ("Type name of a file...", "No media in the vault"),
        };
        self.chooser_purpose = ChooserPurpose::CanvasFile;
        self.modal = Modal::Chooser;
        self.chooser
            .update(cx, |c, cx| c.open(rows, placeholder, empty, window, cx));
        cx.notify();
    }

    /// `canvas:jump-to-group`: a chooser over the open canvas's labelled
    /// groups; the pick is framed and selected. Obsidian hides the command
    /// when no canvas is active; here it says so.
    fn jump_to_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Canvas {
            self.report("Canvas: Jump to group needs an open canvas", cx);
            return;
        }
        let rows: Vec<ChooserRow> = self
            .canvas
            .read(cx)
            .group_targets()
            .into_iter()
            .map(|(label, id)| ChooserRow {
                label,
                value: id,
                flair: None,
            })
            .collect();
        self.chooser_purpose = ChooserPurpose::CanvasGroup;
        self.modal = Modal::Chooser;
        self.chooser.update(cx, |c, cx| {
            c.open(
                rows,
                "Type name of a group...",
                "No groups in this canvas",
                window,
                cx,
            )
        });
        cx.notify();
    }

    /// `canvas:convert-to-file`: the selected text card's text becomes a
    /// note named after its first line (`Untitled` when blank) beside the
    /// canvas, and the card embeds it. Obsidian asks for the name in a
    /// dialog first; here the first line names it.
    fn convert_card_to_file(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if self.main_view(cx) != MainView::Canvas {
            self.report("Canvas: Convert to file needs a selected text card", cx);
            return;
        }
        let Some((id, text)) = self.canvas.read(cx).selected_text_card() else {
            self.report("Canvas: Convert to file needs a selected text card", cx);
            return;
        };
        let folder = self
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/'))
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let first = text
            .lines()
            .map(|l| l.trim().trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .unwrap_or("Untitled");
        let name: String = first
            .chars()
            .filter(|c| {
                !matches!(
                    c,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#' | '^'
                )
            })
            .take(60)
            .collect::<String>()
            .trim()
            .to_string();
        let name = if name.is_empty() {
            "Untitled".to_string()
        } else {
            name
        };
        let path = state.vault().available_path(folder, &name, "md");
        match state.create_note(&path, &text) {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.canvas
                    .update(cx, |c, cx| c.convert_to_file(&id, &path, cx));
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Rebuilds everything that depends on the file list or the whole cache.
    fn refresh_vault_views(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let show_unsupported = crate::fixed::SHOW_UNSUPPORTED_FILES;
        let tree = {
            let vault = state.vault();
            FileTree::new(vault.files(), vault.folders(), show_unsupported)
        };
        let vault_name = state.vault().name();
        self.explorer.update(cx, |e, cx| {
            e.set_vault_name(vault_name);
            e.set_tree(tree, cx)
        });
        self.switcher
            .update(cx, |s, cx| s.set_state(state.clone(), cx));
        self.search
            .update(cx, |s, cx| s.set_state(state.clone(), cx));
        self.graph
            .update(cx, |g, cx| g.set_state(state.clone(), cx));
        self.settings
            .update(cx, |s, cx| s.set_state(state.clone(), cx));
        cx.notify();
    }

    fn start_watching(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let (watcher, rx) = match watch::start(&state.root) {
            Ok(w) => w,
            Err(err) => {
                self.report(format!("File watching unavailable: {err}"), cx);
                return;
            }
        };
        self._watcher = Some(watcher);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            if this.upgrade().is_none() {
                break;
            }
            let mut paths: Vec<PathBuf> = Vec::new();
            while let Ok(p) = rx.try_recv() {
                paths.push(p);
            }
            if paths.is_empty() {
                continue;
            }
            paths.sort();
            paths.dedup();
            if this
                .update(cx, |ws, cx| ws.files_changed(paths, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn files_changed(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // `appearance.json` changed on disk: read the base color scheme again.
        if paths.iter().any(|p| Self::is_appearance_file(&state, p)) {
            self.reload_theme(cx);
        }
        cx.spawn(async move |this, cx| {
            let changes = cx
                .background_executor()
                .spawn(async move { state.apply(&paths) })
                .await;
            this.update(cx, |ws, cx| ws.after_changes(changes, cx)).ok();
        })
        .detach();
    }

    /// Whether `path` is the appearance file the base color scheme is read from.
    fn is_appearance_file(state: &VaultState, path: &Path) -> bool {
        let cfg = state.root.join(state.config_dir());
        let Ok(rel) = path.strip_prefix(&cfg) else {
            return false;
        };
        rel.as_os_str() == "appearance.json"
    }

    /// The window's answer to the desktop's scheme, passed in once the window exists.
    pub fn set_system_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.system_dark = dark;
        self.reload_theme(cx);
    }

    /// `theme:toggle-light-dark`: with a named base theme write the other; under
    /// `system` write nothing and flip only the session's scheme.
    fn toggle_light_dark(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // Its own statement: `appearance()` is a read guard and `change_setting` takes the write lock.
        let swapped = settings::toggle_light_dark(&state.appearance());
        match swapped {
            Some(next) => {
                self.scheme_flipped = false;
                self.change_setting(SettingChange::Appearance(next), cx);
            }
            None => {
                self.scheme_flipped = !self.scheme_flipped;
                self.reload_theme(cx);
            }
        }
    }

    /// Rebuilds the theme global from `appearance.json`'s base color scheme and repaints.
    pub fn reload_theme(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let config = state.reload_appearance();
        let system_dark = self.system_dark != self.scheme_flipped;
        let theme = Theme::from_appearance(&config, system_dark);
        cx.set_global(theme);
        cx.notify();
    }

    /// Whether `changes` touched `path` only as a modification whose bytes the open
    /// buffer already holds: TFT's own save echoing back. A content check, not an
    /// mtime, so a genuine external write still reloads.
    fn is_own_write(&self, path: &str, changes: &Changes, state: &VaultState, cx: &App) -> bool {
        let modified_only = changes
            .files
            .iter()
            .filter(|c| c.path() == path)
            .all(|c| matches!(c, FileChange::Modified(_)));
        if !modified_only {
            return false;
        }
        let editor = self.editor.read(cx);
        if editor.name.as_ref() != path {
            return false;
        }
        // Typing can resume before the echo arrives: the buffer has moved on, but
        // the file is what it last wrote.
        state.texts().get(path).is_some_and(|text| {
            editor.text() == text.as_str() || (editor.is_dirty() && editor.disk_unchanged(text))
        })
    }

    fn after_changes(&mut self, changes: Changes, cx: &mut Context<Self>) {
        if changes.files.is_empty() {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        if changes.structural() {
            self.refresh_vault_views(cx);
        } else {
            self.search.update(cx, |s, cx| s.rerun(cx));
        }
        let n = changes.files.len();
        self.status = if n == 1 {
            format!("{} changed", changes.files[0].path()).into()
        } else {
            format!("{n} files changed").into()
        };
        // A card showing one of these files is rebuilt, whichever group's canvas holds it.
        for change in &changes.files {
            let path = change.path().to_string();
            for group in &self.groups {
                group.canvas.update(cx, |c, cx| c.invalidate(&path, cx));
            }
        }
        // A changed audio or video file is read again and its player stopped.
        for change in &changes.files {
            let path = change.path();
            if matches!(FileKind::from_path(path), FileKind::Audio | FileKind::Video) {
                crate::media::players(cx).invalidate(&state.vault().absolute(path));
            }
        }
        // A changed `.pdf` is forgotten by the shared cache and every view holding
        // it reloads, keeping its page and zoom.
        for change in &changes.files {
            let path = change.path();
            if FileKind::from_path(path) != FileKind::Pdf {
                continue;
            }
            crate::pdf::documents(cx).invalidate(&state.vault().absolute(path), cx);
            for group in &self.groups {
                if group.pdf.read(cx).path() == Some(path) {
                    group.pdf.update(cx, |p, cx| p.reload(cx));
                }
            }
        }
        // Back and forward must not land on a removed file; an atomic external save
        // arrives as a removal too, so only a truly gone path is forgotten.
        for change in &changes.files {
            if let FileChange::Removed(path) = change {
                if !state.vault().absolute(path).exists() {
                    self.forget_history(path);
                    for tab in self.stashed_tabs_mut() {
                        if tab.path.as_deref() == Some(path.as_str()) {
                            tab.path = None;
                        }
                    }
                }
            }
        }
        if let Some(current) = self.current.clone() {
            let removed = changes
                .files
                .iter()
                .any(|c| matches!(c, FileChange::Removed(p) if *p == current));
            if removed && !state.vault().absolute(&current).exists() {
                // The open note is gone: the buffer stays (it may hold unsaved edits)
                // but is no longer a file, and the panes stop describing it.
                self.detach_current(true, &state, cx);
                self.status =
                    format!("{current} was deleted; this buffer is no longer saved anywhere")
                        .into();
            } else if changes.files.iter().any(|c| c.path() == current) {
                // TFT's own autosave echoes back a moment later; rebuilding would
                // re-enter `show_file` and re-expand the Files list every few seconds.
                if !self.is_own_write(&current, &changes, &state, cx) {
                    self.show_file(&current, &state, cx);
                }
            }
        } else if let Some(detached) = self.detached.clone() {
            // The detached buffer's file is back: a slow external unlink-then-rewrite
            // arrives as two changes; reattach and let `show_file` keep unsaved edits.
            if changes.files.iter().any(|c| c.path() == detached.as_ref())
                && state.vault().absolute(&detached).exists()
            {
                self.show_file(&detached, &state, cx);
            }
        }
        cx.notify();
    }

    /// Called once the startup flags have run, so `wait_loaded` can answer for a finished window.
    pub fn mark_started(&mut self) {
        self.started = true;
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    pub fn is_loaded(&self) -> bool {
        self.state.is_some()
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Shows the Search tab with `query` and focuses the field; the dock comes back
    /// first, since an unpainted handle takes every key.
    pub fn show_search(
        &mut self,
        query: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.left_visible = true;
        self.left_tab = LeftTab::Search;
        self.search.update(cx, |s, cx| s.set_query(query, cx));
        if let Some(window) = window {
            window.focus(&self.search.focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// Opens the quick switcher pre-filled with `query`.
    pub fn open_switcher(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut recent: Vec<String> = Vec::new();
        for entry in self.history.iter().rev() {
            if !recent.contains(&entry.path) {
                recent.push(entry.path.clone());
            }
        }
        self.close_editor_suggest(cx);
        self.modal = Modal::Switcher;
        self.switcher
            .update(cx, |s, cx| s.open(recent, query, window, cx));
        cx.notify();
    }

    /// What a slash popup lists: the commands as they stand, ordered by this session's recent ones.
    fn slash_commands(&self, window: &Window, cx: &App) -> crate::editor::SlashCommands {
        let registry = cx.global::<Commands>().0.clone();
        let entries = registry
            .iter()
            .filter(|c| self.can_run(&c.id, window, cx))
            .map(|c| tft_suggest::CommandEntry {
                id: c.id.clone(),
                name: c.name.clone(),
                hotkeys: c.hotkeys.iter().map(hotkey_label).collect(),
            })
            .collect();
        crate::editor::SlashCommands {
            entries,
            pinned: Vec::new(),
            recent: self.recent_commands.clone(),
        }
    }

    /// Runs the command a slash popup row named once its `/query` is out; it is not
    /// recorded as recent, and the removal plus the command's edits are one undo step.
    fn run_slash_command(
        &mut self,
        editor: &Entity<Editor>,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.running_from_slash = true;
        self.run_command(id, window, cx);
        self.running_from_slash = false;
        editor.update(cx, |e, _| e.finish_slash_command());
    }

    /// Opens a Settings dropdown's options, for `--menu`.
    pub fn open_settings_menu(&mut self, key: &str, cx: &mut Context<Self>) {
        self.settings.update(cx, |s, cx| s.open_menu(key, cx));
    }

    /// Opens the Settings window, and brings it forward if it is already up.
    ///
    /// Obsidian's Settings is a modal; TFT's is a window of its own, so
    /// this takes nothing away from the workspace: the vault behind it
    /// keeps its keys and its focus.
    pub fn open_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_editor_suggest(cx);
        self.vault_menu_open = false;
        let view = self.settings.clone();
        if let Some(handle) = self.settings_window {
            let raised = handle
                .update(cx, |_, window, cx| {
                    view.update(cx, |s, cx| s.open(window, cx));
                    window.activate_window();
                })
                .is_ok();
            if raised {
                cx.notify();
                return;
            }
        }
        match SettingsWindow::open(view, cx.weak_entity(), cx) {
            Ok(handle) => self.settings_window = Some(handle),
            Err(err) => self.report(format!("Could not open Settings: {err}"), cx),
        }
        cx.notify();
    }

    /// Takes the Settings window down, from its own Escape or its ×.
    fn close_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        cx.notify();
    }

    /// Whether the Settings window is up. A handle left behind by a window
    /// the desktop closed reads as gone, which is what makes this the test
    /// rather than `settings_window.is_some()`.
    pub fn settings_up(&self, cx: &App) -> bool {
        self.settings_window
            .is_some_and(|handle| handle.read(cx).is_ok())
    }

    /// The Settings window, for the control socket: input goes there while
    /// it is up, since that is the window a caller is looking at.
    pub fn settings_window(&self, cx: &App) -> Option<gpui::AnyWindowHandle> {
        self.settings_up(cx)
            .then(|| self.settings_window.map(Into::into))
            .flatten()
    }

    /// A modal going up takes the autocompletion popup down with it: the
    /// popup would otherwise stay painted under the modal, still holding
    /// the keys it binds.
    fn close_editor_suggest(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.close_suggest(cx));
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal != Modal::None {
            self.modal = Modal::None;
            let mode = self.mode;
            self.set_mode(mode, window, cx);
        }
    }

    fn run_command_action(
        &mut self,
        action: &RunCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.id.clone();
        self.run_command(&id, window, cx);
    }

    /// Runs a command by its Obsidian id; unknown or not-yet-implemented
    /// ids are reported in the status bar.
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if RUNNABLE.contains(&id) && !self.running_from_slash {
            self.recent_commands.retain(|r| r != id);
            self.recent_commands.insert(0, id.to_string());
            self.recent_commands.truncate(20);
        }
        match id {
            "editor:save-file" => self.flush_save(cx),
            "editor:open-search" => self.open_find("", false, window, cx),
            "editor:open-search-replace" => self.open_find("", true, window, cx),
            "editor:add-cursor-above" | "editor:add-cursor-below" => {
                if self.editing_or_report(id, window, cx) {
                    let above = id == "editor:add-cursor-above";
                    self.editor
                        .update(cx, |e, cx| e.add_cursor(above, window, cx));
                }
            }
            "editor:toggle-source" => {
                let mode = match self.mode {
                    ViewMode::LivePreview => ViewMode::Source,
                    _ => ViewMode::LivePreview,
                };
                self.set_mode(mode, window, cx);
            }
            "switcher:open" => self.open_switcher("", window, cx),
            "global-search:open" => self.open_search_action(&OpenSearch, window, cx),
            "graph:open" => self.open_graph_action(&OpenGraph, window, cx),
            // Asked for by id with no file open -- the palette does not offer
            // it, and `can_run` says so -- this opens nothing, which is what
            // Obsidian's `checkCallback` does rather than falling back to the
            // global graph.
            "graph:open-local" if self.current.is_none() => {}
            "graph:open-local" => self.show_graph(true, window, cx),
            // Both check callbacks: asked for by id where the palette would
            // not offer them, they do nothing.
            "graph:animate" if !self.global_graph_active(cx) => {}
            "graph:animate" => self.graph.update(cx, |g, cx| g.animate(cx)),
            "canvas:new-file" => self.new_canvas("", cx),
            "canvas:jump-to-group" => self.jump_to_group(window, cx),
            "canvas:convert-to-file" => self.convert_card_to_file(cx),
            "markdown:toggle-preview" => self.toggle_mode(&ToggleMode, window, cx),
            // The properties block is the same editable widget in reading
            // view, and `editor:toggle-fold-properties` was observed running
            // there with the preview leaf active (Specification.md, "Folding"),
            // so these three ask only that the note hold the keyboard.
            //
            // Obsidian opens an unnamed property row and writes nothing until
            // it is named, which is what `add_property_row` does.
            "markdown:add-metadata-property" => {
                if self.note_has_keys(window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.add_property_row(window, cx));
                }
            }
            "markdown:add-alias" => self.add_alias(window, cx),
            "editor:rename-heading" => {
                if self.editing_or_report(id, window, cx) {
                    self.start_rename_heading(window, cx);
                }
            }
            "markdown:clear-metadata-properties" => {
                if self.note_has_keys(window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.clear_properties(window, cx));
                }
            }
            // The Editor tab's Spellcheck row as a command: the same
            // `app.json` write a click on the row makes, applied the same
            // way, so the open Settings window and the views follow it.
            "editor:toggle-spellcheck" => {
                // Built in a statement of its own: the config is read under
                // a guard, and `change_setting` takes the write lock.
                let change = self
                    .state
                    .as_ref()
                    .and_then(|state| settings::toggle_command(id, state));
                if let Some(change) = change {
                    self.change_setting(change, cx);
                }
            }
            "theme:toggle-light-dark" => self.toggle_light_dark(cx),
            // "Show context menu under cursor": a right click's menu, at the
            // caret. In the reading view Obsidian 1.15.0 neither offered it
            // nor opened anything, so a hotkey there is silent too.
            "editor:context-menu" => {
                if self.mode.is_editing() && self.note_has_keys(window, cx) {
                    self.editor.update(cx, |e, cx| e.context_menu_at_caret(cx));
                }
            }
            // "Focus on last note": the keyboard back on the note's editor
            // the main area shows, wherever it was -- a dock tree, the
            // search field. It never switches tabs: over a canvas, an empty
            // tab or a note in the reading view Obsidian 1.15.0 did nothing
            // visible, so only an editor on screen takes it.
            "editor:focus" => {
                if self.main_view(cx) == MainView::Editor {
                    self.focus_main(window, cx);
                }
            }
            "editor:toggle-fold-properties" => {
                if self.note_has_keys(window, cx) {
                    if self.mode.is_editing() {
                        self.editor.update(cx, |e, cx| e.toggle_properties_fold(cx));
                    } else {
                        self.reading
                            .update(cx, |r, cx| r.toggle_properties_fold(cx));
                    }
                }
            }
            // The five fold commands pass their check only with a markdown
            // view focused in an editing mode, as observed; none has a
            // default hotkey.
            "editor:toggle-fold" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.toggle_fold(cx));
                }
            }
            "editor:fold-all" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.fold_all(cx));
                }
            }
            "editor:unfold-all" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.unfold_all(cx));
                }
            }
            "editor:fold-more" | "editor:fold-less" => {
                if self.editing_or_report(id, window, cx) {
                    let more = id == "editor:fold-more";
                    self.editor.update(cx, |e, cx| e.fold_step(more, cx));
                }
            }
            "editor:insert-table" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.insert_table(window, cx));
                }
            }
            // The wrapping toggles write the pair of markers Obsidian's markup uses around the selection.
            "editor:toggle-bold"
            | "editor:toggle-italics"
            | "editor:toggle-strikethrough"
            | "editor:toggle-highlight"
            | "editor:toggle-code"
            | "editor:toggle-inline-math"
            | "editor:toggle-comments" => {
                if self.editing_or_report(id, window, cx) {
                    let marker = match id {
                        "editor:toggle-bold" => "**",
                        "editor:toggle-italics" => "*",
                        "editor:toggle-strikethrough" => "~~",
                        "editor:toggle-highlight" => "==",
                        "editor:toggle-code" => "`",
                        "editor:toggle-inline-math" => "$",
                        _ => "%%",
                    };
                    self.editor
                        .update(cx, |e, cx| e.toggle_wrap(marker, window, cx));
                }
            }
            "editor:toggle-bullet-list"
            | "editor:toggle-numbered-list"
            | "editor:toggle-blockquote" => {
                if self.editing_or_report(id, window, cx) {
                    let kind = match id {
                        "editor:toggle-bullet-list" => LinePrefix::Bullet,
                        "editor:toggle-numbered-list" => LinePrefix::Ordered,
                        _ => LinePrefix::Quote,
                    };
                    self.editor
                        .update(cx, |e, cx| e.toggle_line_prefix(kind, window, cx));
                }
            }
            "editor:toggle-checklist-status" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.toggle_checklist(window, cx));
                }
            }
            "editor:cycle-list-checklist" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.cycle_checklist(window, cx));
                }
            }
            // The six colours write one emoji after the opening `==`.
            "editor:toggle-highlight-red"
            | "editor:toggle-highlight-orange"
            | "editor:toggle-highlight-yellow"
            | "editor:toggle-highlight-green"
            | "editor:toggle-highlight-blue"
            | "editor:toggle-highlight-purple" => {
                if self.editing_or_report(id, window, cx) {
                    let color = id
                        .rsplit('-')
                        .next()
                        .and_then(tft_markdown::HighlightColor::from_name);
                    self.editor
                        .update(cx, |e, cx| e.toggle_highlight(color, window, cx));
                }
            }
            "editor:clear-formatting" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.clear_formatting(window, cx));
                }
            }
            "editor:insert-callout"
            | "editor:insert-codeblock"
            | "editor:insert-mathblock"
            | "editor:insert-horizontal-rule"
            | "editor:insert-tag"
            | "editor:insert-footnote" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(kind) = BlockInsert::from_id(id) {
                        self.editor
                            .update(cx, |e, cx| e.insert_block(kind, window, cx));
                    }
                }
            }
            // "Toggle heading" is a real palette entry whose check passes and changes
            // nothing (Specification.md, "Observed editing command behavior"); doing
            // nothing is parity, not a bug.
            "editor:set-heading" => {
                self.editing_or_report(id, window, cx);
            }
            "editor:set-heading-0"
            | "editor:set-heading-1"
            | "editor:set-heading-2"
            | "editor:set-heading-3"
            | "editor:set-heading-4"
            | "editor:set-heading-5"
            | "editor:set-heading-6" => {
                if self.editing_or_report(id, window, cx) {
                    let level = id
                        .rsplit('-')
                        .next()
                        .and_then(|n| n.parse::<u8>().ok())
                        .unwrap_or(0);
                    self.editor
                        .update(cx, |e, cx| e.set_heading(level, window, cx));
                }
            }
            "editor:indent-list" | "editor:unindent-list" => {
                if self.editing_or_report(id, window, cx) {
                    let indent = id == "editor:indent-list";
                    self.editor
                        .update(cx, |e, cx| e.indent_list(indent, window, cx));
                }
            }
            "editor:delete-paragraph" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.delete_paragraph(window, cx));
                }
            }
            "editor:swap-line-up" | "editor:swap-line-down" => {
                if self.editing_or_report(id, window, cx) {
                    let up = id == "editor:swap-line-up";
                    self.editor.update(cx, |e, cx| e.swap_lines(up, window, cx));
                }
            }
            "editor:insert-link" | "editor:insert-wikilink" | "editor:insert-embed" => {
                if self.editing_or_report(id, window, cx) {
                    let kind = match id {
                        "editor:insert-link" => LinkSyntax::Markdown,
                        "editor:insert-wikilink" => LinkSyntax::Wikilink,
                        _ => LinkSyntax::Embed,
                    };
                    self.editor
                        .update(cx, |e, cx| e.insert_link(kind, window, cx));
                }
            }
            "editor:follow-link" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.follow_link_at_cursor(cx));
                }
            }
            "editor:open-link-in-new-leaf" => {
                if self.editing_or_report(id, window, cx) {
                    self.link_in_new_tab = true;
                    let found = self.editor.update(cx, |e, cx| e.follow_link_at_cursor(cx));
                    if !found {
                        self.link_in_new_tab = false;
                    }
                }
            }
            "editor:attach-file" => self.attach_file(window, cx),
            "editor:download-attachments" => self.start_download_attachments(window, cx),
            "app:open-file" => self.open_file_dialog(window, cx),
            _ if TableCommand::from_id(id).is_some() => {
                let cmd = TableCommand::from_id(id).expect("checked above");
                if self.editing_or_report(id, window, cx) {
                    let ran = self
                        .editor
                        .update(cx, |e, cx| e.table_command(cmd, window, cx));
                    if !ran {
                        self.report("The cursor is not in a table", cx);
                    }
                }
            }
            "app:go-back" => self.go_back(&GoBack, window, cx),
            "app:go-forward" => self.go_forward(&GoForward, window, cx),
            "app:open-settings" => self.open_settings(window, cx),
            "app:toggle-left-sidebar" => {
                self.left_visible = !self.left_visible;
                if !self.left_visible {
                    self.leave_search_field(window, cx);
                }
                cx.notify();
            }
            "file-explorer:open" => {
                self.leave_search_field(window, cx);
                self.left_visible = true;
                self.left_tab = LeftTab::Files;
                self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
                cx.notify();
            }
            "file-explorer:reveal-active-file" => match self.current.clone() {
                Some(current) => self.reveal_in_navigation(&current, window, cx),
                None => {
                    self.leave_search_field(window, cx);
                    self.left_visible = true;
                    self.left_tab = LeftTab::Files;
                    self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
                    cx.notify();
                }
            },
            "file-explorer:new-file"
            | "file-explorer:new-file-in-current-tab"
            | "file-explorer:new-file-in-new-pane" => {
                // Obsidian's "Create new note" goes to a new tab unless the active one
                // is empty; "to the right" is a new tab in TFT's one strip.
                let fresh_tab = match id {
                    "file-explorer:new-file-in-current-tab" => false,
                    "file-explorer:new-file-in-new-pane" => true,
                    _ => self.current.is_some(),
                };
                // New notes go to the vault folder.
                let folder = String::new();
                if fresh_tab {
                    self.new_tab(cx);
                }
                self.new_note(&folder, window, cx);
            }
            "file-explorer:new-folder" => self.start_new_folder("", window, cx),
            "file-explorer:move-file" => {
                if let Some(current) = self.current.clone() {
                    self.start_move(&current, window, cx);
                }
            }
            "file-explorer:duplicate-file" => {
                if let Some(current) = self.current.clone() {
                    self.duplicate(&current, cx);
                }
            }
            "workspace:edit-file-title" => {
                if let Some(current) = self.current.clone() {
                    self.start_rename(&current, window, cx);
                }
            }
            "workspace:new-tab" => self.new_tab(cx),
            // A pinned tab is unpinned by the close, not closed (observed).
            "workspace:close" if self.active_tab_pinned() => self.toggle_pin(cx),
            "workspace:close" => self.close_tab(self.active_tab, cx),
            "workspace:close-others" => self.close_other_tabs(cx),
            "workspace:toggle-pin" => self.toggle_pin(cx),
            "workspace:move-to-new-window" => self.move_to_new_window(window, cx),
            "workspace:open-in-new-window" => self.open_in_new_window(window, cx),
            "workspace:new-window" => {
                self.new_window(window, cx);
            }
            "workspace:close-window" => self.close_window(window, cx),
            "workspace:split-vertical" => self.split(SplitDirection::Vertical, window, cx),
            "workspace:split-horizontal" => self.split(SplitDirection::Horizontal, window, cx),
            "workspace:close-tab-group" => {
                let id = self.active_group_id();
                self.close_group(id, true, cx);
            }
            // Obsidian's "Close others in tab group" closes the group's other tabs, as `close-others` does.
            "workspace:close-others-tab-group" => self.close_other_tabs(cx),
            "editor:focus-left" => self.focus_neighbor(SplitDirection::Vertical, false, cx),
            "editor:focus-right" => self.focus_neighbor(SplitDirection::Vertical, true, cx),
            "editor:focus-top" => self.focus_neighbor(SplitDirection::Horizontal, false, cx),
            "editor:focus-bottom" => self.focus_neighbor(SplitDirection::Horizontal, true, cx),
            // "Open link under cursor to the right": split right, then follow the link there.
            "editor:open-link-in-new-split" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.editor.read(cx).link_target_at_cursor() {
                        self.split(SplitDirection::Vertical, window, cx);
                        self.follow_link(target, cx);
                    }
                }
            }
            "editor:open-link-in-new-window" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.editor.read(cx).link_target_at_cursor() {
                        self.open_link_in_new_window(target, window, cx);
                    }
                }
            }
            "workspace:next-tab" => self.cycle_tab(1, cx),
            "workspace:previous-tab" => self.cycle_tab(self.tabs.len().saturating_sub(1), cx),
            "workspace:goto-last-tab" => self.activate_tab(self.tabs.len().saturating_sub(1), cx),
            "workspace:undo-close-pane" => self.undo_close_tab(cx),
            _ if id.starts_with("workspace:goto-tab-") => {
                let n: usize = id
                    .rsplit('-')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                if n >= 1 && n <= self.tabs.len() {
                    self.activate_tab(n - 1, cx);
                }
            }
            "app:delete-file" => {
                if let Some(current) = self.current.clone() {
                    self.start_delete(&current, window, cx);
                }
            }
            // The file menu's rows for the active file: Copy path, Copy full path,
            // Open in default app, Show in explorer. The menu is silent, but each
            // command reports what it copied.
            "workspace:copy-path" => {
                if let Some(current) = self.current.clone() {
                    self.copy_path(&current, cx);
                    self.report("Copied to your clipboard", cx);
                }
            }
            "workspace:copy-full-path" => {
                if let (Some(state), Some(current)) = (self.state.clone(), self.current.clone()) {
                    let text = full_path_text(state.vault().absolute(&current));
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.report("Copied to your clipboard", cx);
                }
            }
            "workspace:copy-url" => {
                if let (Some(state), Some(current)) = (self.state.clone(), self.current.clone()) {
                    let name = vault_folder_name(&state.root);
                    let url = crate::uri::open_url(&name, &current);
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    self.report("URL copied to your clipboard", cx);
                }
            }
            // "Show trash", offered only under `trashOption: "local"`: the `.trash`
            // folder, or Obsidian's notice when it is missing.
            "workspace:show-trash" => {
                if let Some(state) = self.state.clone() {
                    if state.trash_option() == TrashOption::Local {
                        let trash = state.vault().absolute(TRASH_FOLDER);
                        if trash.exists() {
                            self.show_in_system_explorer(TRASH_FOLDER, cx);
                        } else {
                            self.report(missing_file_notice(&full_path_text(trash)), cx);
                        }
                    }
                }
            }
            "open-with-default-app:open" => {
                if let Some(current) = self.current.clone() {
                    self.open_in_default_app(&current, cx);
                }
            }
            "open-with-default-app:show" => {
                if let Some(current) = self.current.clone() {
                    self.show_in_system_explorer(&current, cx);
                }
            }
            // "Manage vaults" raises the vault manager.
            "app:open-vault" => self.open_vault_manager(window, cx),
            "app:switch-vault" => self.toggle_vault_menu(cx),
            "app:open-another-vault" => self.open_another_vault(window, cx),
            "app:open-sandbox-vault" => self.open_sandbox_vault(window, cx),
            "app:show-debug-info" => self.show_debug_info(window, cx),
            "app:migrate-frontmatter" => self.open_migrate_frontmatter(window, cx),
            "workspace:export-pdf" | "canvas:export-as-image" => {
                self.run_export_command(id, window, cx);
            }
            "app:show-release-notes" => self.show_release_notes(cx),
            // Zoom in/out/reset: half a level between -2.5 and +3, on the vault
            // window (see [[Specification#Window zoom]]).
            "app:reload" => self.reload_app(cx),
            "window:zoom-in" => self.zoom_main(zoom::stepped(self.zoom_level, 1), cx),
            "window:zoom-out" => self.zoom_main(zoom::stepped(self.zoom_level, -1), cx),
            "window:reset-zoom" => self.zoom_main(0.0, cx),
            // A popout's command only (`can_run`): that window above others, a
            // second run back; nothing is written.
            "window:toggle-always-on-top" => {
                if self.can_run(id, window, cx) {
                    let on = !zoom::always_on_top(window);
                    if !zoom::set_always_on_top(window, on) {
                        self.report(
                            "This desktop does not let a window keep itself on top".to_string(),
                            cx,
                        );
                    }
                }
            }
            _ => {
                let name = self.command_name(id, cx);
                self.report(format!("{name}: not available in TFT yet"), cx);
            }
        }
    }

    /// "Change vault..." and the footer's name: the switcher's menu, listing every
    /// vault this machine has opened.
    fn toggle_vault_menu(&mut self, cx: &mut Context<Self>) {
        self.vault_menu_open = !self.vault_menu_open;
        cx.notify();
    }

    /// "Manage vaults": the vault manager window, brought forward if already up.
    fn open_vault_manager(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.vault_menu_open = false;
        if let Some(handle) = self.vault_window {
            let up = handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok();
            if up {
                cx.notify();
                return;
            }
        }
        let main = window.window_handle();
        match VaultManager::open(cx.weak_entity(), main, cx) {
            Ok(handle) => self.vault_window = Some(handle),
            Err(err) => self.report(format!("Could not open the vault manager: {err}"), cx),
        }
        cx.notify();
    }

    /// "Open sandbox vault": a fresh temp folder with a note or two, loaded here and
    /// left off the switcher's list.
    pub(crate) fn open_sandbox_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.vault_menu_open = false;
        let root = std::env::temp_dir().join(format!("TFT Sandbox {}", now_millis()));
        if let Err(err) = write_sandbox(&root) {
            self.report(format!("Could not make the sandbox vault: {err}"), cx);
            return;
        }
        self.load_vault_with(root, false, window, cx);
        // Opened once the vault is in, the way `--note` is.
        self.current = Some("Welcome.md".into());
    }

    /// Puts `root` at the head of the switcher's list. The list lives
    /// outside every vault; a machine with nowhere to keep it has no list.
    fn remember_vault(&mut self, root: &Path, cx: &mut Context<Self>) {
        let outcome = (|| -> std::io::Result<()> {
            let Some(mut registry) = VaultRegistry::open()? else {
                return Ok(());
            };
            registry.record_open(root, now_millis());
            registry.save()
        })();
        if let Err(err) = outcome {
            self.report(
                format!("Could not record the vault for the switcher: {err}"),
                cx,
            );
        }
    }

    /// "Open vault...": asks the desktop for a folder and opens it as the vault;
    /// cancelling changes nothing.
    fn open_another_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.vault_menu_open = false;
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open vault".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let root = match picked.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.remove(0),
                Ok(Ok(_)) => return,
                Ok(Err(err)) => {
                    this.update(cx, |ws, cx| {
                        ws.report(format!("Could not open the folder dialog: {err}"), cx)
                    })
                    .ok();
                    return;
                }
                Err(_) => return,
            };
            this.update_in(cx, |ws, window, cx| ws.load_vault(root, window, cx))
                .ok();
        })
        .detach();
    }

    /// The command's name as the palette shows it, for a message about it.
    fn command_name(&self, id: &str, cx: &App) -> String {
        cx.global::<Commands>()
            .0
            .get(id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// "Reload app without saving", in the vault window: the vault, config, theme,
    /// hotkeys and plugins are read again; unsaved edits are dropped and the layout
    /// kept ([[Specification#Observed app reload]]).
    fn reload_app(&mut self, cx: &mut Context<Self>) {
        if self.state.is_none() {
            return;
        }
        let this = cx.entity();
        let main = self.main_window;
        cx.defer(move |cx| {
            main.update(cx, |_, window, cx| {
                this.update(cx, |ws, cx| ws.reload_now(window, cx))
            })
            .ok();
        });
    }

    fn reload_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let layout = self.layout_json(cx);
        // Without saving: the pending autosave is dropped and the closing groups write nothing.
        self.save_task = None;
        self.current = None;
        self.forget_vault(cx);
        self.history.clear();
        self.history_ix = 0;
        self.reload_layout = Some(layout);
        let record = self.record_vault;
        self.load_vault_with(state.root.clone(), record, window, cx);
    }

    /// Sets the vault window's zoom level, records it in TFT's window file and applies it.
    fn zoom_main(&mut self, level: f32, cx: &mut Context<Self>) {
        if level == self.zoom_level {
            return;
        }
        self.zoom_level = level;
        if let Some(state) = &self.state {
            zoom::save_level(&state.root, level);
        }
        self.apply_main_zoom(cx);
    }

    /// Applies [`Self::zoom_level`] after this update: the command may have run in a
    /// popout while this workspace is being updated.
    fn apply_main_zoom(&self, cx: &mut Context<Self>) {
        let main = self.main_window;
        let level = self.zoom_level;
        cx.defer(move |cx| {
            main.update(cx, |_, window, cx| {
                zoom::set_level(window, cx, level);
            })
            .ok();
        });
    }

    /// Whether a command can run now, which is what the palette lists and the socket
    /// reports; Obsidian hides commands whose check callback fails.
    pub fn can_run(&self, id: &str, window: &Window, cx: &App) -> bool {
        if !RUNNABLE.contains(&id) {
            return false;
        }
        // A command whose core plugin is off is gone from the registry; this only
        // matters for a direct id.
        if let Some(plugin) = tft_commands::plugin_of(id) {
            if !self
                .state
                .as_ref()
                .is_none_or(|state| state.core_plugin_enabled(plugin))
            {
                return false;
            }
        }
        // Obsidian's table commands are check callbacks: hidden outside Live Preview
        // and when the command has nothing to do.
        if let Some(cmd) = TableCommand::from_id(id) {
            return self.mode.is_editing() && self.editor.read(cx).table_command_applies(cmd);
        }
        // `graph:open-local` is a checkCallback not listed with no file open; only
        // that case was observed (see [[Specification#`graph:open-local` is not offered with no file open]]).
        if id == "graph:open-local" {
            return self.current.is_some();
        }
        // Offered only while a global graph is the active leaf, not with a note active
        // or a local graph.
        if id == "graph:animate" {
            return self.global_graph_active(cx);
        }
        // "Toggle window always on top" is offered only in a popout that is not maximized.
        if id == "window:toggle-always-on-top" {
            return window.window_handle().downcast::<PopoutWindow>().is_some()
                && !window.is_maximized();
        }
        // The active file's commands are check callbacks: offered only with a note or image open.
        if ACTIVE_FILE_ONLY.contains(&id) {
            return self.current.is_some();
        }
        // "Show trash" only where there is a trash of the vault's own.
        if id == "workspace:show-trash" {
            return self
                .state
                .as_ref()
                .is_some_and(|state| state.trash_option() == TrashOption::Local);
        }
        // "Focus on last note" whenever a Markdown leaf exists, in any mode or tab.
        if id == "editor:focus" {
            return self.has_markdown_leaf();
        }
        // Both are check callbacks: "Rename this heading..." needs the caret on an ATX
        // heading while editing; "Add alias" is left out of reading view, whose block is
        // read-only in TFT.
        if id == "editor:rename-heading" {
            return self.mode.is_editing()
                && self.note_has_keys(window, cx)
                && self.editor.read(cx).heading_to_rename().is_some();
        }
        if let Some(can) = self.attachments_can_run(id) {
            return can;
        }
        if let Some(can) = self.exports_can_run(id, cx) {
            return can;
        }
        if EDITOR_ONLY.contains(&id) {
            return self.mode.is_editing() && self.note_has_keys(window, cx);
        }
        true
    }

    /// Whether any tab of any group, popouts included, is a Markdown leaf.
    fn has_markdown_leaf(&self) -> bool {
        let live = std::iter::once((self.current.as_deref(), self.mode == ViewMode::Graph));
        let mine = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix != self.active_tab)
            .map(|(_, t)| (t.path.as_deref(), t.mode == ViewMode::Graph));
        let others = self
            .groups
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix != self.active_group)
            .flat_map(|(_, g)| g.tabs.iter())
            .map(|t| (t.path.as_deref(), t.mode == ViewMode::Graph));
        any_markdown_leaf(live.chain(mine).chain(others))
    }

    /// True when the note holds the keyboard: no modal and focus on its pane. An editor
    /// hotkey otherwise fires wherever the keyboard is and writes into a note behind it.
    fn note_has_keys(&self, window: &Window, cx: &App) -> bool {
        self.modal == Modal::None
            && (self.editor.read(cx).focus_handle_ref().is_focused(window)
                || self.reading.focus_handle(cx).is_focused(window))
    }

    /// True when an editor command can run. Obsidian hides a failed check callback; TFT
    /// says why it did nothing instead, but only when the note had the keys -- a command
    /// arriving in a dock field or modal is addressed there.
    fn editing_or_report(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.note_has_keys(window, cx) {
            return false;
        }
        if self.mode.is_editing() {
            return true;
        }
        let name = self.command_name(id, cx);
        self.report(format!("{name}: only available while editing"), cx);
        false
    }

    fn open_search_action(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).query(cx);
        self.show_search(&query, Some(window), cx);
    }

    /// Whether the open file is a Markdown note.
    fn active_note_is_markdown(&self) -> bool {
        self.current
            .as_deref()
            .is_some_and(|path| FileKind::from_path(path) == FileKind::Markdown)
    }

    /// `graph:animate`'s check: a global graph is the active leaf.
    fn global_graph_active(&self, cx: &App) -> bool {
        self.main_view(cx) == MainView::Graph && !self.graph.read(cx).is_local()
    }

    /// Shows the graph (`graph:open`); a second Ctrl+G returns to the note.
    pub fn show_graph(&mut self, local: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.graph.update(cx, |g, cx| g.set_local(local, cx));
        self.set_mode(ViewMode::Graph, window, cx);
    }

    fn open_graph_action(&mut self, _: &OpenGraph, window: &mut Window, cx: &mut Context<Self>) {
        // "Open graph view" toggles the *global* graph. Asked for it with a
        // local graph on screen, Obsidian shows the global one rather than
        // leaving graph mode: `graph:open-local` is not a toggle either, so
        // treating any graph as the one to close made the pair inconsistent.
        if self.mode == ViewMode::Graph && !self.graph.read(cx).is_local() {
            let back = self.mode_before_graph;
            self.set_mode(back, window, cx);
        } else {
            self.show_graph(false, window, cx);
        }
    }

    pub fn set_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        if mode == ViewMode::Graph && self.mode != ViewMode::Graph {
            self.mode_before_graph = self.mode;
        }
        // The history entry tracks the current state, so navigating away and back restores the mode.
        self.record_history_state(cx);
        if !mode.is_editing() && self.mode.is_editing() {
            // Reading shows what the editor has: save and rebuild now rather than waiting
            // for the autosave.
            if self.editor.read(cx).is_dirty() {
                self.flush_save(cx);
                if let (Some(state), Some(current)) = (self.state.clone(), self.current.clone()) {
                    self.show_file(&current, &state, cx);
                }
            }
        }
        // The fold is shared: editor and reading view collapse the same sections.
        if mode.is_editing() != self.mode.is_editing() {
            self.sync_folds(mode.is_editing(), cx);
        }
        self.mode = mode;
        if mode.is_editing() {
            self.editor.update(cx, |e, cx| {
                e.set_live_preview(mode == ViewMode::LivePreview, cx)
            });
        }
        self.focus_main(window, cx);
        cx.notify();
    }

    /// Hands the folds across to the editor when `to_editor`, else to the reading view.
    fn sync_folds(&mut self, to_editor: bool, cx: &mut Context<Self>) {
        if to_editor {
            let (keys, props) = {
                let r = self.reading.read(cx);
                (r.fold_keys(), r.properties_folded())
            };
            self.editor.update(cx, |e, cx| {
                e.set_fold_keys(keys, cx);
                e.set_properties_folded(props, cx);
            });
        } else {
            let (keys, props) = {
                let e = self.editor.read(cx);
                (e.fold_keys(), e.properties_folded())
            };
            self.reading.update(cx, |r, cx| {
                r.set_fold_keys(keys, cx);
                r.set_properties_folded(props, cx);
            });
        }
    }

    /// The vault's fold store, opened on first use. `root` is passed because the first
    /// note opens while `self.state` is still being installed.
    fn fold_store(&mut self, root: &Path) -> Option<&mut fold::FoldStore> {
        if self.fold_store.as_ref().is_none_or(|(r, _)| r != root) {
            self.fold_store = Some((root.to_path_buf(), fold::FoldStore::open(root)));
        }
        self.fold_store.as_mut().map(|(_, s)| s)
    }

    /// Writes the open note's folds to the store: on leaving and on quit; the reading
    /// view's go to the editor first, which knows the extents.
    fn save_folds(&mut self, path: &str, cx: &mut Context<Self>) {
        if FileKind::from_path(path) != FileKind::Markdown {
            return;
        }
        if self.editor.read(cx).name.as_ref() != path {
            return;
        }
        if !self.mode.is_editing() {
            self.sync_folds(true, cx);
        }
        let Some(record) = self.editor.read(cx).fold_record() else {
            return;
        };
        let Some(root) = self.state.as_ref().map(|s| s.root.clone()) else {
            return;
        };
        if let Some(store) = self.fold_store(&root) {
            store.put(path, record);
        }
    }

    /// Restores a note's folds as it opens: the editor takes the `from` lines, reading
    /// the keys they come to.
    fn restore_folds(
        &mut self,
        path: &str,
        text: &str,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self
            .fold_store(&state.root)
            .and_then(|s| s.get(path).cloned())
        else {
            return;
        };
        let (headings, lists) = (crate::fixed::FOLD_HEADINGS, crate::fixed::FOLD_INDENTS);
        let doc = tft_markdown::parse(text);
        let foldables = fold::foldables(&doc, text, fold::FoldOptions { headings, lists });
        let froms = record
            .folds
            .iter()
            .filter(|r| **r != fold::PROPERTIES_RANGE)
            .map(|r| r.from);
        let keys = fold::Folds::from_record(&foldables, froms).keys();
        let props = record.folds.contains(&fold::PROPERTIES_RANGE);
        self.editor
            .update(cx, |e, cx| e.set_fold_record(&record, cx));
        self.reading.update(cx, |r, cx| {
            r.set_fold_keys(keys, cx);
            r.set_properties_folded(props, cx);
        });
    }

    /// Asks for the keyboard to go back to the main area at the next paint. Most callers
    /// have no `Window`, so they record the request and `render` carries it out.
    fn request_focus_main(&mut self, cx: &mut Context<Self>) {
        self.refocus_main = true;
        cx.notify();
    }

    /// Whether the keyboard is somewhere nothing can use it: the workspace root (where a
    /// dock click leaves it), no element, or a main view the area is not painting. Anything
    /// else holds it on purpose and must keep it.
    fn keyboard_is_parked(&self, window: &Window, cx: &App) -> bool {
        if window.focused(cx).is_none() || self.focus_handle.is_focused(window) {
            return true;
        }
        // The dock's tree handle is out of the dispatch tree once its pane is hidden or
        // another tab took the dock.
        let painted = self.main_view(cx);
        let unpainted =
            |view: MainView, handle: &FocusHandle| painted != view && handle.is_focused(window);
        if unpainted(MainView::Editor, self.editor.read(cx).focus_handle_ref())
            || unpainted(MainView::Reading, &self.reading.focus_handle(cx))
            || unpainted(MainView::Graph, &self.graph.focus_handle(cx))
            || unpainted(MainView::Canvas, &self.canvas.focus_handle(cx))
            || unpainted(MainView::Pdf, &self.pdf.focus_handle(cx))
            || unpainted(MainView::ReleaseNotes, &self.release_notes.focus_handle(cx))
        {
            return true;
        }
        // A view of a group that is no longer active is painted, but the keyboard
        // belongs with the active group's note.
        self.groups.iter().enumerate().any(|(ix, g)| {
            ix != self.active_group
                && (g.editor.read(cx).focus_handle_ref().is_focused(window)
                    || g.reading.focus_handle(cx).is_focused(window)
                    || g.canvas.focus_handle(cx).is_focused(window)
                    || g.pdf.focus_handle(cx).is_focused(window))
        })
    }

    /// Puts the keyboard on whatever the main area paints; a handle on an absent view reaches nothing.
    fn focus_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refocus_main = false;
        let handle = match self.main_view(cx) {
            MainView::Reading => self.reading.focus_handle(cx),
            MainView::Editor => self.editor.focus_handle(cx),
            MainView::Graph => self.graph.focus_handle(cx),
            MainView::Canvas => self.canvas.focus_handle(cx),
            MainView::Pdf => self.pdf.focus_handle(cx),
            MainView::ReleaseNotes => self.release_notes.focus_handle(cx),
            MainView::Empty => self.focus_handle.clone(),
        };
        // gpui's focus is per window, and the active group's views paint in one window:
        // the keyboard goes there. A window cannot be updated from within its own update.
        let target = self.active_window_handle();
        if target == window.window_handle() {
            window.focus(&handle, cx);
        } else {
            target
                .update(cx, |_, window, cx| window.focus(&handle, cx))
                .ok();
        }
    }

    /// Hands the keyboard back to the main area before the left dock stops painting the
    /// search field; focus on an unmounted input would take every key.
    fn leave_search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.focus_handle(cx).is_focused(window) {
            self.focus_main(window, cx);
        }
    }

    /// Opens the editor's find panel, switching to editing first; an empty `query`
    /// keeps the panel's own or the selection.
    pub fn open_find(
        &mut self,
        query: &str,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mode.is_editing() {
            let mode = self.editing_mode();
            self.set_mode(mode, window, cx);
        }
        self.editor
            .update(cx, |e, cx| e.open_find(query, replace, window, cx));
    }

    /// The editing mode a note opens in: Editing view in Live Preview.
    pub fn editing_mode(&self) -> ViewMode {
        ViewMode::LivePreview
    }

    pub fn default_mode(&self) -> ViewMode {
        self.editing_mode()
    }

    /// Opens a vault path by exact match or suffix (`--note`); `Name#Heading` or
    /// `Name#^id` also scrolls to that section.
    pub fn open_by_name(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.state.clone() else {
            return false;
        };
        let link = tft_markdown::parse_linktext(name);
        let wanted = link.path;
        let found = state
            .vault()
            .files()
            .iter()
            .map(|f| f.path.as_str())
            .find(|p| {
                *p == wanted
                    || p.ends_with(wanted)
                    || p.trim_end_matches(".md") == wanted
                    || p.trim_end_matches(".md").ends_with(&format!("/{wanted}"))
            })
            .map(str::to_string);
        match found {
            Some(path) => {
                self.open_file(path, link.subpath, cx);
                true
            }
            None => false,
        }
    }

    /// Opens the first markdown note, if any.
    pub fn open_first(&mut self, cx: &mut Context<Self>) {
        let first = self
            .state
            .as_ref()
            .and_then(|s| s.vault().markdown_files().next().map(|f| f.path.clone()));
        if let Some(path) = first {
            self.open_file(path, None, cx);
        }
    }

    /// Opens a vault-relative path in the main area and records it in history.
    pub fn open_file(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        self.reset_preview(cx);
        let Some(state) = self.state.clone() else {
            self.current = Some(path);
            return;
        };
        // A pinned tab keeps its file: anything opened from it — a link
        // followed, a switcher pick — goes to a new tab (observed).
        if self.active_tab_pinned() && self.current.as_deref() != Some(path.as_str()) {
            self.new_tab(cx);
        }
        // The graph is a toggle, not a place a note can open in: a note
        // asked for from anywhere shows the note.
        if self.mode == ViewMode::Graph {
            self.mode = self.mode_before_graph;
        }
        if self.current.as_deref() != Some(path.as_str()) {
            self.flush_save(cx);
            // The entry being left keeps the state it is being left in, so
            // coming back to it comes back to where the reader was.
            self.record_history_state(cx);
            self.history.truncate(self.history_ix);
            self.history
                .push(HistoryEntry::new(path.clone(), self.mode));
            self.history_ix = self.history.len();
            self.last_open_files.retain(|p| *p != path);
            self.last_open_files.insert(0, path.clone());
            self.last_open_files.truncate(LAST_OPEN_FILES_KEPT);
        }
        self.show_file(&path, &state, cx);
        // Opening a note is a move into the main area, wherever it was asked
        // for: an explorer row, a search hit, the switcher, a link, the
        // socket. Obsidian hands the keyboard to the note it just opened, and
        // without this the click that opened it leaves focus on the
        // workspace root, where typing and the arrow keys reach nothing.
        self.request_focus_main(cx);
        if self.current.as_deref() != Some(path.as_str()) {
            // It did not open, so it is not somewhere to go back to either.
            self.forget_history(&path);
            return;
        }
        if let Some(sub) = subpath {
            self.scroll_to_subpath(&path, &sub, &state, cx);
        }
    }

    fn show_file(&mut self, path: &str, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let previous = self.current.replace(path.to_string());
        if let Some(prev) = previous.clone() {
            if prev != path {
                self.save_folds(&prev, cx);
            }
        }
        if let Some(gone) = self.detached.take() {
            // The kept detached buffer's edits exist nowhere else; name what is dropped
            // rather than dropping it silently.
            let lost = {
                let e = self.editor.read(cx);
                e.is_dirty() && e.name.as_ref() != path
            };
            if lost {
                self.report(
                    format!(
                        "Discarded the unsaved edits in {gone}; nothing was saving that buffer"
                    ),
                    cx,
                );
            }
        }
        self.explorer.update(cx, |e, cx| e.follow(path, cx));
        self.graph
            .update(cx, |g, cx| g.set_current(Some(path.to_string()), cx));
        let kind = FileKind::from_path(path);
        if kind != FileKind::Canvas {
            self.release_canvas(cx);
        }
        if kind != FileKind::Pdf {
            self.release_pdf(cx);
        }
        match kind {
            FileKind::Canvas => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    // Nothing of a note may show through: the reading view is emptied, the editor let go.
                    self.reading.update(cx, |r, cx| {
                        r.set_note(
                            None,
                            reading::build_with("", "", state.as_ref(), state.build_options()),
                            cx,
                        )
                    });
                    self.release_buffer(cx);
                    // The view keeps its scene when the file is what it last read or wrote,
                    // keeps unsaved edits, and reloads otherwise.
                    let (same, matches, dirty) = {
                        let c = self.canvas.read(cx);
                        (c.path() == Some(path), c.matches_file(&text), c.is_dirty())
                    };
                    if same && matches {
                        // Own write or an unrelated event.
                    } else if same && dirty {
                        self.report(format!("{path} changed on disk; unsaved edits kept"), cx);
                    } else if same {
                        self.canvas.update(cx, |c, cx| c.reload(&text, cx));
                    } else {
                        // Where the tab last looked, when restored from the layout for this file.
                        let view_state = self
                            .tabs
                            .get(self.active_tab)
                            .filter(|t| t.path.as_deref() == Some(path))
                            .and_then(|t| CanvasView::viewport_from_json(t.view_state.as_ref()));
                        let state = state.clone();
                        self.canvas
                            .update(cx, |c, cx| c.load(path, &text, view_state, state, cx));
                    }
                    self.status = path.to_string().into();
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            FileKind::Markdown => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    // In an editing mode the reading model is built lazily -- on a switch
                    // to reading, a fold sync or a state dump -- not on every open.
                    let build_ms = if self.mode.is_editing() && !debug_status() {
                        let (path, text, state) = (path.to_string(), text.clone(), state.clone());
                        self.reading.update(cx, |r, cx| {
                            let build_options = state.build_options();
                            let state = state.clone();
                            r.set_note_later(
                                path,
                                text,
                                move |path, text| {
                                    crate::reading::build_with(
                                        path,
                                        text,
                                        state.as_ref(),
                                        build_options,
                                    )
                                },
                                cx,
                            )
                        });
                        0.0
                    } else {
                        let started = Instant::now();
                        let model =
                            reading::build_with(path, &text, state.as_ref(), state.build_options());
                        let build_ms = started.elapsed().as_secs_f64() * 1000.0;
                        self.reading
                            .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
                        build_ms
                    };
                    // The editor keeps its buffer when the file is what it last saved (or
                    // still holds unsaved edits), and keeps its place when the note changed.
                    let (same, matches, dirty) = {
                        let e = self.editor.read(cx);
                        // A save's echo can arrive after the next edit: the buffer is dirty
                        // but the file is what it last wrote, so nothing changed underneath
                        // it (seen 2026-11-10).
                        let echo = e.is_dirty() && e.disk_unchanged(&text);
                        (
                            e.name.as_ref() == path,
                            e.matches_file(&text) || echo,
                            e.is_dirty(),
                        )
                    };
                    if same && matches {
                        // Own write or an unrelated event.
                    } else if same && dirty {
                        self.report(format!("{path} changed on disk; unsaved edits kept"), cx);
                        cx.notify();
                        return;
                    } else if same {
                        self.editor.update(cx, |e, cx| e.reload(text, cx));
                    } else {
                        self.editor
                            .update(cx, |e, cx| e.set_text(path.to_string(), text.clone(), cx));
                        self.restore_folds(path, &text, state, cx);
                    }
                    // Build times stay off the status bar unless `TFT_DEBUG_STATUS` is set.
                    self.status = if debug_status() {
                        format!("{path}  ·  reading view built in {build_ms:.1} ms").into()
                    } else {
                        path.to_string().into()
                    };
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            // A PDF is drawn by its own view and never writes; a tab switch back to the
            // open file keeps its page and zoom, and the view remembers a file it showed
            // earlier this session.
            FileKind::Pdf => {
                // Nothing of a note may show through: reading emptied, editor let go.
                self.reading.update(cx, |r, cx| {
                    r.set_note(
                        None,
                        reading::build_with("", "", state.as_ref(), state.build_options()),
                        cx,
                    )
                });
                self.release_buffer(cx);
                if self.pdf.read(cx).path() != Some(path) {
                    let absolute = state.vault().absolute(path);
                    self.pdf.update(cx, |p, cx| p.load(path, absolute, cx));
                }
                self.status = path.to_string().into();
            }
            // An image, audio or video file is drawn as its own embed.
            FileKind::Image | FileKind::Audio | FileKind::Video => {
                let model = reading::build_with(
                    path,
                    &format!("![[{path}]]"),
                    state.as_ref(),
                    state.build_options(),
                );
                self.reading
                    .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
                self.release_buffer(cx);
                self.status = path.to_string().into();
            }
            other => {
                // Nothing of the previous note may survive into a file no view can draw,
                // and the file's bytes must not reach the editor.
                self.reading.update(cx, |r, cx| {
                    r.set_note(
                        None,
                        reading::build_with("", "", state.as_ref(), state.build_options()),
                        cx,
                    )
                });
                self.release_buffer(cx);
                self.status = format!("{path}: {other:?} files are not supported yet").into();
            }
        }
        cx.notify();
    }

    /// A file `show_file` could not read. `current` must not name it while the editor
    /// holds another note, or edits get silently dropped; go back to the real note, or
    /// let the buffer go when there is none.
    fn unreadable(
        &mut self,
        path: &str,
        previous: Option<String>,
        err: &std::io::Error,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        self.report(format!("Could not read {path}: {err}"), cx);
        self.current = previous.filter(|p| p.as_str() != path);
        match self.current.clone() {
            Some(back) => {
                self.explorer.update(cx, |e, cx| e.follow(&back, cx));
                self.graph.update(cx, |g, cx| g.set_current(Some(back), cx));
            }
            None => self.detach_current(true, state, cx),
        }
    }

    /// Lets go of the canvas on screen; its edits were flushed already, and otherwise
    /// `main_view` would keep painting a file that is no longer shown.
    fn release_canvas(&mut self, cx: &mut Context<Self>) {
        if self.canvas.read(cx).path().is_some() {
            self.canvas.update(cx, |c, cx| c.release(cx));
        }
    }

    /// Lets go of the PDF on screen, if any, the same way. Nothing is
    /// flushed first: the view never writes.
    fn release_pdf(&mut self, cx: &mut Context<Self>) {
        if self.pdf.read(cx).path().is_some() {
            self.pdf.update(cx, |p, cx| p.release(cx));
        }
    }

    /// Plain text of a cached heading (markup stripped).
    fn heading_text(raw: &str) -> String {
        let doc = tft_markdown::parse(raw);
        match doc.blocks.first().map(|b| &b.kind) {
            Some(tft_markdown::BlockKind::Paragraph { content }) => {
                tft_markdown::plain_text(content)
            }
            _ => raw.to_string(),
        }
        .trim()
        .to_string()
    }

    /// Lets go of a buffer for a file the editor cannot save. A dirty buffer stays on
    /// screen as belonging to no file; a clean one goes.
    fn release_buffer(&mut self, cx: &mut Context<Self>) {
        let (dirty, name) = {
            let editor = self.editor.read(cx);
            (editor.is_dirty(), editor.name.clone())
        };
        if dirty && !name.is_empty() {
            self.save_task = None;
            self.detached = Some(name);
            return;
        }
        self.clear_buffer(cx);
    }

    /// Empties the editor; keeping a canvas's or PDF's bytes would let the autosave
    /// write them through the markdown writer.
    fn clear_buffer(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        self.detached = None;
        self.editor
            .update(cx, |e, cx| e.set_text("", String::new(), cx));
    }

    /// Lets go of the open note: no pane, graph or history may keep describing it.
    /// `keep_buffer` keeps a buffer whose edits exist nowhere else.
    fn detach_current(
        &mut self,
        keep_buffer: bool,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        self.save_task = None;
        if keep_buffer {
            let name = self.editor.read(cx).name.clone();
            self.detached = (!name.is_empty()).then_some(name);
        } else {
            self.clear_buffer(cx);
        }
        self.release_canvas(cx);
        self.release_pdf(cx);
        self.reading.update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.current = None;
        // Nothing is open, so no row is the open file's; otherwise the next `set_tree`
        // would paint a path the vault no longer has.
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
    }

    /// Drops every row that describes the open note; panes are not refreshed on the way
    /// out, so a gone note's outline, backlinks and outgoing links would stay clickable.
    fn forget_history(&mut self, path: &str) {
        self.history_ix = prune_history(&mut self.history, self.history_ix, path);
        for tab in self.stashed_tabs_mut() {
            tab.history_ix = prune_history(&mut tab.history, tab.history_ix, path);
        }
    }

    fn reveal_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reading
            .update(cx, |r, cx| r.scroll_to_offset(offset, cx));
        self.editor.update(cx, |e, cx| e.reveal_offset(offset, cx));
        // An outline row or backlink moved the caret; the keyboard goes where the caret went.
        self.request_focus_main(cx);
    }

    fn scroll_to_subpath(
        &mut self,
        path: &str,
        subpath: &Subpath<'_>,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        // `[[doc.pdf#page=3]]`: the link parser reads `page=3` as a heading, the PDF
        // view as a page; every `#` part is put back and parsed as pdf.js's hash.
        if FileKind::from_path(path) == FileKind::Pdf {
            if let Subpath::Headings(names) = subpath {
                let raw = names.join("#");
                // `#page=99`, `#page=0` and `#page=abc` leave the view put;
                // `selection=` marks the text and brings it near the top.
                let parsed = tft_pdf::parse_subpath(&raw);
                if let Some(page) = parsed.page {
                    if self.pdf.read(cx).path() == Some(path) {
                        self.pdf.update(cx, |p, cx| match parsed.selection {
                            Some(selection) => p.go_to_selection(page, selection, cx),
                            None => p.go_to_page(page, false, cx),
                        });
                    }
                }
            }
            return;
        }
        let offset = Self::subpath_offset(state, path, subpath);
        if let Some(offset) = offset {
            self.reveal_offset(offset, cx);
        }
    }

    /// Copy link/quote in a PDF: a wikilink to the file at the vault's link format,
    /// subpath and alias after it; the quote is `> text`, a blank line, the link.
    fn copy_pdf_link(
        &mut self,
        path: &str,
        subpath: &str,
        alias: &str,
        quote: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let target =
            tft_cache::links::link_path(state.cache().resolver(), path, path, state.link_format());
        let link = format!("[[{target}#{subpath}|{alias}]]");
        let text = match quote {
            Some(text) => crate::pdf::selection::quote(text, &link),
            None => link,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Where a `#Heading` or `#^block` subpath starts in a note, by the cache.
    fn subpath_offset(state: &VaultState, path: &str, subpath: &Subpath<'_>) -> Option<usize> {
        let cache = state.cache();
        let meta = cache.file(path)?;
        match subpath {
            Subpath::Headings(names) => {
                let last = names.last()?;
                meta.headings
                    .iter()
                    .find(|h| Self::heading_text(&h.heading).eq_ignore_ascii_case(last.trim()))
                    .map(|h| h.position.start.offset)
            }
            Subpath::Block(id) => meta
                .blocks
                .get(&id.to_lowercase())
                .or_else(|| meta.blocks.get(*id))
                .map(|b| b.position.start.offset),
        }
    }

    // ----- page preview ------------------------------------------------------------

    /// The window and note of the group whose reading view or editor is `view`.
    fn view_origin(&self, view: gpui::EntityId) -> Option<(Option<u64>, String)> {
        let ix = self
            .groups
            .iter()
            .position(|g| g.reading.entity_id() == view || g.editor.entity_id() == view)?;
        let group = &self.groups[ix];
        let path = if ix == self.active_group {
            self.tabs.get(self.active_tab)?.path.clone()
        } else {
            group.tabs.get(group.active_tab)?.path.clone()
        }?;
        Some((self.window_of_group(group.id), path))
    }

    /// The pointer came onto or left a link. A link that may preview arms the show
    /// timer; leaving arms the hide timer, defused by reaching the popover.
    fn link_hover(
        &mut self,
        source: crate::hover_preview::Source,
        view: gpui::EntityId,
        hover: Option<crate::reading::LinkHover>,
        cx: &mut Context<Self>,
    ) {
        let Some((window, from)) = self.view_origin(view) else {
            return;
        };
        self.nested_link_hover(source, window, from, 0, hover, cx);
    }

    /// [`Self::link_hover`] for a link `depth` popovers deep; leaving clears it only
    /// if it is still one at this depth, since arrival can precede departure.
    fn nested_link_hover(
        &mut self,
        source: crate::hover_preview::Source,
        window: Option<u64>,
        from: String,
        depth: usize,
        hover: Option<crate::reading::LinkHover>,
        cx: &mut Context<Self>,
    ) {
        if hover.is_none()
            && self
                .hover_preview
                .pending
                .as_ref()
                .is_some_and(|p| p.depth != depth)
        {
            return;
        }
        self.hover_preview.generation += 1;
        let pending = hover.and_then(|h| {
            self.hover_preview.modifiers = h.modifiers;
            let LinkTarget::Internal(target) = h.target else {
                return None;
            };
            Some(crate::hover_preview::Pending {
                window,
                source,
                target,
                from,
                anchor: h.anchor,
                depth,
            })
        });
        self.set_preview_pending(pending, cx);
    }

    fn set_preview_pending(
        &mut self,
        pending: Option<crate::hover_preview::Pending>,
        cx: &mut Context<Self>,
    ) {
        self.hover_preview.pending = pending;
        if self.hover_preview.pending.is_some() {
            self.arm_preview_show(cx);
        }
        // Either way something may have been left: a link for nothing, or
        // for another link whose popover goes ahead of this one's.
        self.arm_preview_hide(cx);
    }

    /// The pointer came onto a previewing row (a pane's row, a tab) or left it.
    /// Leaving clears the link only if it is still this row's; the next arrival can precede departure.
    #[allow(clippy::too_many_arguments)]
    fn row_hover(
        &mut self,
        source: crate::hover_preview::Source,
        window: Option<u64>,
        target: String,
        from: String,
        over: bool,
        anchor: Bounds<Pixels>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        if !over {
            let ours =
                self.hover_preview.pending.as_ref().is_some_and(|p| {
                    p.source == source && p.anchor == anchor && p.target == target
                });
            if !ours {
                return;
            }
        }
        self.hover_preview.generation += 1;
        self.hover_preview.modifiers = modifiers;
        let pending = over.then_some(crate::hover_preview::Pending {
            window,
            source,
            target,
            from,
            anchor,
            depth: 0,
        });
        self.set_preview_pending(pending, cx);
    }

    /// The pointer left whatever `source` last reported, for sources that do not say which.
    fn source_left(&mut self, source: crate::hover_preview::Source, cx: &mut Context<Self>) {
        if self
            .hover_preview
            .pending
            .as_ref()
            .is_some_and(|p| p.source == source && p.depth == 0)
        {
            self.hover_preview.generation += 1;
            self.set_preview_pending(None, cx);
        }
    }

    /// Mod down with the pointer on a link raises its popover after ~300 ms; Mod up leaves one up.
    fn preview_modifiers(&mut self, modifiers: gpui::Modifiers, cx: &mut Context<Self>) {
        let was = crate::hover_preview::mod_held(&self.hover_preview.modifiers);
        self.hover_preview.modifiers = modifiers;
        if !was
            && crate::hover_preview::mod_held(&modifiers)
            && self.hover_preview.pending.is_some()
        {
            self.hover_preview.generation += 1;
            self.arm_preview_show(cx);
        }
    }

    /// Starts the show timer if the link may preview: the plugin on and Mod held
    /// where its source needs it.
    fn arm_preview_show(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some(pending) = &self.hover_preview.pending else {
            return;
        };
        if !state.core_plugin_enabled("page-preview") {
            return;
        }
        if pending.source.requires_mod(&state.page_preview())
            && !crate::hover_preview::mod_held(&self.hover_preview.modifiers)
        {
            return;
        }
        let generation = self.hover_preview.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::hover_preview::SHOW_DELAY)
                .await;
            this.update(cx, |this, cx| {
                if this.hover_preview.generation == generation {
                    this.show_preview(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Starts a hide timer, if a popover is up; [`HoverPreview::kept`] reads where the
    /// pointer is when it fires, so no generation is needed.
    fn arm_preview_hide(&mut self, cx: &mut Context<Self>) {
        if self.hover_preview.shown.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::hover_preview::HIDE_DELAY)
                .await;
            this.update(cx, |this, cx| {
                let kept = this.hover_preview.kept();
                this.truncate_previews(kept, cx);
            })
            .ok();
        })
        .detach();
    }

    fn show_preview(&mut self, cx: &mut Context<Self>) {
        let (Some(state), Some(pending)) = (self.state.clone(), self.hover_preview.pending.clone())
        else {
            return;
        };
        let (content, from) = crate::hover_preview::content(
            state.as_ref(),
            state.build_options(),
            &pending.target,
            &pending.from,
        );
        // An editing popover goes only on Escape or a press outside, so a link under it raises nothing.
        if self
            .hover_preview
            .shown
            .iter()
            .skip(pending.depth)
            .any(|s| s.editing.is_some())
        {
            return;
        }
        let note =
            crate::hover_preview::editable_note(state.as_ref(), &pending.target, &pending.from);
        // The new popover replaces whatever was up above the link's own.
        self.truncate_previews(pending.depth, cx);
        // The properties block starts folded, as Obsidian's popover had it.
        self.hover_preview
            .shown
            .push(crate::hover_preview::Shown::new(
                pending.window,
                pending.anchor,
                pending.target.clone(),
                from,
                content,
                note,
                [crate::properties::PROPERTIES_FOLD_ID]
                    .into_iter()
                    .collect(),
            ));
        cx.notify();
    }

    /// Keeps the first `keep` popovers and takes down the rest, writing an editing one's edits first.
    fn truncate_previews(&mut self, keep: usize, cx: &mut Context<Self>) {
        if self.hover_preview.shown.len() <= keep {
            return;
        }
        self.flush_preview_save(cx);
        self.hover_preview.shown.truncate(keep);
        // A view that is gone never reports the pointer leaving its link.
        if self
            .hover_preview
            .pending
            .as_ref()
            .is_some_and(|p| p.depth > keep)
        {
            self.hover_preview.pending = None;
        }
        cx.notify();
    }

    fn hide_preview(&mut self, cx: &mut Context<Self>) {
        self.truncate_previews(0, cx);
    }

    /// Drops the link under the pointer and the non-editing popovers, and makes every
    /// view forget its link: called when the screen changes under the pointer.
    fn reset_preview(&mut self, cx: &mut Context<Self>) {
        let hp = &mut self.hover_preview;
        if hp.pending.is_none() && hp.shown.is_empty() {
            return;
        }
        hp.generation += 1;
        hp.pending = None;
        let kept = hp
            .shown
            .iter()
            .rposition(|s| s.editing.is_some())
            .map_or(0, |ix| ix + 1);
        self.truncate_previews(kept, cx);
        for group in &self.groups {
            group.reading.update(cx, |r, _| r.clear_hover());
            group.editor.update(cx, |e, _| e.clear_hover());
        }
    }

    /// The pointer came onto the `ix`th popover or left it.
    fn popover_hover(&mut self, ix: usize, over: bool, cx: &mut Context<Self>) {
        let Some(shown) = self.hover_preview.shown.get_mut(ix) else {
            return;
        };
        shown.over = over;
        if !over {
            self.hover_preview.generation += 1;
            self.arm_preview_hide(cx);
        }
    }

    /// A link clicked inside the `ix`th popover opens against the shown note; the popovers go.
    fn preview_link(&mut self, ix: usize, target: LinkTarget, cx: &mut Context<Self>) {
        let from = self.hover_preview.shown.get(ix).map(|s| s.from.clone());
        self.hide_preview(cx);
        let target = match (target, from, self.state.clone()) {
            (LinkTarget::Internal(body), Some(from), Some(state)) => {
                match state.cache().resolve(&body, &from) {
                    Some(path) => {
                        let sub = body.find('#').map(|i| &body[i..]).unwrap_or("");
                        LinkTarget::Internal(format!("{path}{sub}"))
                    }
                    None => LinkTarget::Internal(body),
                }
            }
            (target, _, _) => target,
        };
        self.follow_link(target, cx);
    }

    /// A click on the `ix`th popover's text makes it a Live Preview editor in the same
    /// box, caret where clicked, until Escape or a press outside.
    fn edit_preview(
        &mut self,
        ix: usize,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // "Click to create": the note is made, empty, and opens in the
        // active tab in its editing mode with the keyboard in it; the
        // popover goes (observed, `s11` `missingClick`).
        if let Some(shown) = self.hover_preview.shown.get(ix) {
            if matches!(shown.content, crate::hover_preview::Content::Missing(_)) {
                let name = tft_markdown::parse_linktext(&shown.target).path.to_string();
                self.hide_preview(cx);
                self.create_from_link(&name, window, cx);
                self.request_focus_main(cx);
                return;
            }
        }
        let Some((path, subpath)) = self
            .hover_preview
            .shown
            .get(ix)
            .filter(|s| s.editing.is_none())
            .and_then(|s| s.note.clone())
        else {
            return;
        };
        // The popovers over this one go; the open note's edits are written first so the
        // popover starts from what is on disk.
        self.truncate_previews(ix + 1, cx);
        self.flush_save(cx);
        let Ok(text) = std::fs::read_to_string(state.vault().absolute(&path)) else {
            return;
        };
        let editor = cx.new(|cx| crate::new_editor(window, cx));
        Self::give_editor_the_vault(&editor, &state, cx);
        Self::configure_editor(&editor, &state, cx);
        editor.update(cx, |e, cx| {
            e.set_readable_line_length(false);
            e.set_show_inline_title(false);
            e.set_live_preview(true, cx);
            e.set_text(path.clone(), text, cx);
        });
        let subscription = cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
            this.preview_editor_event(&editor, event, cx)
        });
        if let Some(shown) = self.hover_preview.shown.get_mut(ix) {
            shown.editing = Some(editor.clone());
            shown.editing_subscription = Some(subscription);
            shown.hover_current.set(None);
        }
        window.focus(&editor.focus_handle(cx), cx);
        // The caret at the click once a frame maps the point; for a section, at its start.
        let section = (!subpath.is_empty())
            .then(|| tft_markdown::parse_linktext(&subpath).subpath)
            .flatten()
            .and_then(|sub| Self::subpath_offset(&state, &path, &sub));
        match section {
            Some(offset) => editor.update(cx, |e, cx| {
                e.set_selection_range(offset, offset, window, cx);
                e.reveal_offset(offset, cx);
            }),
            None => {
                let editor = editor.downgrade();
                Self::caret_when_laid_out(editor, position, 4, window);
            }
        }
        cx.notify();
    }

    /// Puts an editor's caret at a window point once it has a layout, trying for `tries` more frames.
    fn caret_when_laid_out(
        editor: gpui::WeakEntity<Editor>,
        position: gpui::Point<Pixels>,
        tries: usize,
        window: &mut Window,
    ) {
        window.on_next_frame(move |window, cx| {
            let placed = editor
                .update(cx, |e, cx| e.caret_at_point(position, window, cx))
                .unwrap_or(true);
            if !placed && tries > 0 {
                Self::caret_when_laid_out(editor, position, tries - 1, window);
            }
        });
    }

    /// What an editing popover's editor reports.
    fn preview_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.hover_preview.shown.iter().position(|s| {
            s.editing
                .as_ref()
                .is_some_and(|e| e.entity_id() == editor.entity_id())
        }) else {
            return;
        };
        match event {
            EditorEvent::Edited => self.schedule_preview_save(cx),
            EditorEvent::OpenLink(target) => {
                let target = target.clone();
                self.preview_link(ix, target, cx)
            }
            EditorEvent::OpenLinkInNewTab(target) => {
                let target = target.clone();
                self.link_in_new_tab = true;
                self.preview_link(ix, target, cx)
            }
            // Its links raise popovers on the editor's rules (Mod).
            EditorEvent::Hover(hover) => {
                let shown = &self.hover_preview.shown[ix];
                let (window, from) = (shown.window, shown.from.clone());
                self.nested_link_hover(
                    crate::hover_preview::Source::Editor,
                    window,
                    from,
                    ix + 1,
                    hover.clone(),
                    cx,
                )
            }
            EditorEvent::Reported(message) => self.report(message.clone(), cx),
            _ => {}
        }
    }

    /// An editing popover's edits are written two seconds after the last one, as the open note's are.
    fn schedule_preview_save(&mut self, cx: &mut Context<Self>) {
        self.preview_save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |ws, cx| ws.flush_preview_save(cx)).ok();
        }));
    }

    /// Writes every editing popover's unsaved edits now.
    fn flush_preview_save(&mut self, cx: &mut Context<Self>) {
        self.preview_save_task = None;
        let editors: Vec<Entity<Editor>> = self
            .hover_preview
            .shown
            .iter()
            .filter_map(|s| s.editing.clone())
            .collect();
        for editor in editors {
            if editor.read(cx).is_dirty() {
                self.write_buffer(&editor, cx);
            }
        }
    }

    /// Escape or a press outside the `ix`th editing popover: it goes with those over
    /// it. Escape returns the keyboard to the main view; a press keeps what it hit.
    fn close_preview(
        &mut self,
        ix: usize,
        escape: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .hover_preview
            .shown
            .get(ix)
            .is_some_and(|s| s.editing.is_some())
        {
            self.hover_preview.generation += 1;
            self.truncate_previews(ix, cx);
            // The ones under it go too, unless the pointer is over them.
            self.arm_preview_hide(cx);
            if escape {
                self.focus_main(window, cx);
            }
        }
    }

    /// The popovers up in the window being drawn.
    fn render_hover_preview(
        &mut self,
        here: Option<u64>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let Some(state) = self.state.clone() else {
            return Vec::new();
        };
        let properties = PropertiesInDocument::default();
        let types = state.property_types().clone();
        let mut out = Vec::new();
        for (ix, shown) in self.hover_preview.shown.iter().enumerate() {
            if shown.window != here {
                continue;
            }
            let this = cx.entity().downgrade();
            let (window_id, from) = (shown.window, shown.from.clone());
            let handlers = crate::hover_preview::Handlers {
                on_link: Rc::new({
                    let this = this.clone();
                    move |target, _window, cx| {
                        this.update(cx, |this, cx| this.preview_link(ix, target, cx))
                            .ok();
                    }
                }),
                // A link in a popover previews on the reading view's rules.
                on_hover_link: Rc::new({
                    let this = this.clone();
                    move |hover, _window, cx| {
                        let from = from.clone();
                        this.update(cx, |this, cx| {
                            this.nested_link_hover(
                                crate::hover_preview::Source::Reading,
                                window_id,
                                from,
                                ix + 1,
                                hover,
                                cx,
                            )
                        })
                        .ok();
                    }
                }),
                on_hover: Rc::new({
                    let this = this.clone();
                    move |over, _window, cx| {
                        this.update(cx, |this, cx| this.popover_hover(ix, over, cx))
                            .ok();
                    }
                }),
                on_edit: Rc::new({
                    let this = this.clone();
                    move |position, window, cx| {
                        this.update(cx, |this, cx| this.edit_preview(ix, position, window, cx))
                            .ok();
                    }
                }),
                on_close: Rc::new(move |escape, window, cx| {
                    this.update(cx, |this, cx| this.close_preview(ix, escape, window, cx))
                        .ok();
                }),
            };
            out.push(crate::hover_preview::render(
                shown, ix, theme, &types, properties, handlers, window,
            ));
        }
        out
    }

    fn follow_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        let new_tab = std::mem::take(&mut self.link_in_new_tab);
        match target {
            LinkTarget::Internal(body) => {
                let Some(state) = self.state.clone() else {
                    return;
                };
                let source = self.current.clone().unwrap_or_default();
                let parsed = tft_markdown::parse_linktext(&body);
                let resolved = state.cache().resolve(&body, &source).map(str::to_string);
                match resolved {
                    Some(path) if new_tab => self.open_link_in_new_tab(path, parsed.subpath, cx),
                    Some(path) => {
                        if path == source {
                            if let Some(sub) = parsed.subpath {
                                self.scroll_to_subpath(&path, &sub, &state, cx);
                            }
                        } else {
                            self.open_file(path, parsed.subpath, cx);
                        }
                    }
                    None => {
                        // Obsidian creates the note a dangling link points at.
                        if new_tab {
                            self.new_tab(cx);
                        }
                        let name = parsed.path.to_string();
                        self.create_note_for(&name, None, cx);
                    }
                }
            }
            LinkTarget::External(url) => cx.open_url(&url),
            LinkTarget::Tag(tag) => self.show_search(&format!("tag:#{tag}"), None, cx),
            LinkTarget::Footnote(n) => {
                // Obsidian's reference anchors to that one definition, not the note's
                // end; reveal it the same way a heading jump does.
                let at = self
                    .reading
                    .read(cx)
                    .model()
                    .footnotes
                    .get(n.saturating_sub(1))
                    .copied();
                match at {
                    Some(offset) => self.reveal_offset(offset, cx),
                    None => self.reading.update(cx, |r, cx| r.scroll_to_end(cx)),
                }
            }
        }
    }

    fn toggle_mode(&mut self, _: &ToggleMode, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.mode {
            ViewMode::Reading => self.editing_mode(),
            ViewMode::LivePreview | ViewMode::Source | ViewMode::Graph => ViewMode::Reading,
        };
        self.set_mode(mode, window, cx);
    }

    fn go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.record_history_state(cx);
        if self.history_ix > 1 {
            self.history_ix -= 1;
            self.navigate_history(cx);
        }
    }

    fn go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        self.record_history_state(cx);
        if self.history_ix < self.history.len() {
            self.history_ix += 1;
            self.navigate_history(cx);
        }
    }

    fn navigate_history(&mut self, cx: &mut Context<Self>) {
        let (Some(state), Some(entry)) = (
            self.state.clone(),
            self.history.get(self.history_ix.wrapping_sub(1)).cloned(),
        ) else {
            return;
        };
        if self.mode == ViewMode::Graph {
            self.mode = self.mode_before_graph;
        }
        self.flush_save(cx);
        // Obsidian's history carries the view state, so back/forward restore the note's mode too.
        self.mode = entry.mode;
        self.show_file(&entry.path, &state, cx);
        if entry.offset > 0 {
            self.reveal_offset(entry.offset, cx);
        }
        self.request_focus_main(cx);
    }

    /// Writes the view state into the entry the history stands on: the caret while
    /// editing, the top block in Reading, where there is no caret.
    fn record_history_state(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            return;
        };
        let mode = if self.mode == ViewMode::Graph {
            self.mode_before_graph
        } else {
            self.mode
        };
        let offset = if mode == ViewMode::Reading {
            self.reading.read(cx).top_offset().unwrap_or(0)
        } else {
            self.editor.read(cx).selection().start
        };
        let Some(entry) = self.history.get_mut(self.history_ix.wrapping_sub(1)) else {
            return;
        };
        if entry.path == current {
            entry.mode = mode;
            entry.offset = offset;
        }
    }

    /// A task checkbox clicked in Reading takes the editor's path: one undo step, the
    /// autosave writes it, and the reader rebuilds from the buffer.
    fn toggle_task(&mut self, path: &str, marker: TaskMarker, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // A click that outlived its note must write nothing; the marker's line and
        // column belong to that note's bytes.
        if self.current.as_deref() != Some(path) || self.editor.read(cx).name.as_ref() != path {
            return;
        }
        if !self.editor.update(cx, |e, cx| {
            e.toggle_task_at(marker.line, marker.col, None, cx)
        }) {
            return;
        }
        self.rebuild_reading_from_editor(path, &state, cx);
    }

    /// A pill's remove control in Reading takes [`Self::toggle_task`]'s path, then
    /// rebuilds the reader from the buffer.
    fn remove_property_item(
        &mut self,
        path: &str,
        key: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // A click that outlived its note must write nothing; the key and index belong
        // to that note's block.
        if self.current.as_deref() != Some(path) || self.editor.read(cx).name.as_ref() != path {
            return;
        }
        if !self
            .editor
            .update(cx, |e, cx| e.remove_property_item(key, index, None, cx))
        {
            return;
        }
        self.rebuild_reading_from_editor(path, &state, cx);
    }

    /// Rebuilds the reading view from the editor's buffer after a reader's edit the
    /// autosave has not written yet.
    fn rebuild_reading_from_editor(
        &mut self,
        path: &str,
        state: &VaultState,
        cx: &mut Context<Self>,
    ) {
        let text = self.editor.read(cx).text().to_string();
        let model = reading::build_with(path, &text, state, state.build_options());
        self.reading
            .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
        cx.notify();
    }

    /// Writes the editor's buffer back two seconds after the last edit, as Obsidian's autosave does.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |ws, cx| ws.flush_save(cx)).ok();
        }));
    }

    /// Writes the editor's buffer now if dirty, with the file's own line endings and
    /// BOM around the edited text.
    pub fn flush_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        let Some(state) = self.state.clone() else {
            return;
        };
        self.flush_canvas_save(&state, cx);
        let (dirty, name) = {
            let editor = self.editor.read(cx);
            (editor.is_dirty(), editor.name.to_string())
        };
        // Only the open note is written back; a buffer that is not a vault file
        // (the synthetic stress note) never touches disk.
        if !dirty || self.current.as_deref() != Some(name.as_str()) {
            return;
        }
        let editor = self.editor.clone();
        self.write_buffer(&editor, cx);
    }

    /// Writes an editor's buffer to its file, unless the file changed or went away underneath it.
    fn write_buffer(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let name = editor.read(cx).name.to_string();
        let contents = editor.read(cx).file_contents();
        // The one place a note's bytes are written; the path is checked for links,
        // so a save can never land outside the vault.
        let absolute = match state.vault().write_path(&name) {
            Ok(path) => path,
            Err(err) => {
                self.report(format!("{name} not saved: {err}"), cx);
                return;
            }
        };
        // The vault is the source of truth: if the file is not what this note loaded
        // or last saved, someone else wrote it, and overwriting would throw that away.
        // A deleted note is not recreated; the buffer keeps its edits and says so.
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if editor.read(cx).disk_unchanged(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{name} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.report(
                    format!("{name} was deleted; not recreated, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {name}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        match tft_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                editor.update(cx, |e, _| e.mark_saved(&contents));
                self.report(format!("{name} saved"), cx);
            }
            Err(err) => {
                self.report(format!("Could not save {name}: {err}"), cx);
            }
        }
        cx.notify();
    }

    /// "Copy search results" from the Search pane's count menu.
    fn open_copy_results(
        &mut self,
        entries: Vec<CopyEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_editor_suggest(cx);
        self.copy_results.update(cx, |m, cx| m.open(entries, cx));
        self.modal = Modal::CopyResults;
        window.focus(&self.copy_results.focus_handle(cx), cx);
        cx.notify();
    }

    // ----- file operations -------------------------------------------------------

    fn open_prompt(
        &mut self,
        op: PendingOp,
        spec: PromptSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = Some(op);
        self.close_editor_suggest(cx);
        self.modal = Modal::Prompt;
        self.prompt.update(cx, |p, cx| p.open(spec, window, cx));
        cx.notify();
    }

    /// The explorer menu's "Copy path": the vault-relative path, the one the app
    /// speaks. Silent, as Obsidian shows no notice for it.
    fn copy_path(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
    }

    /// The explorer menu's "Open in default app": one command per platform, no crate.
    /// The child is spawned and let go; only a spawn failure is reported.
    fn open_in_default_app(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        let mut command = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else if cfg!(target_os = "windows") {
            // `start` is a shell builtin, and its first quoted argument is the window
            // title, not the file.
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        } else {
            std::process::Command::new("xdg-open")
        };
        if let Err(err) = command
            .arg(&absolute)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            self.report(format!("Could not open {path}: {err}"), cx);
        }
    }

    /// The file menu's "Show in system explorer", through [`file_menu::reveal_commands`].
    /// The D-Bus call waits, so the whole run is off the UI thread.
    fn show_in_system_explorer(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { file_menu::show_in_system_explorer(&absolute) })
                .await;
            if let Err(err) = result {
                this.update(cx, |ws, cx| {
                    ws.report(format!("Could not show {path}: {err}"), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// The file menu's "Reveal file in navigation", and `file-explorer:reveal-active-file`:
    /// show Files expanded to `path`, ring its row and give it the keyboard.
    fn reveal_in_navigation(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_search_field(window, cx);
        self.left_visible = true;
        self.left_tab = LeftTab::Files;
        self.explorer
            .update(cx, |e, cx| e.reveal_and_focus(path, window, cx));
        cx.notify();
    }

    /// The explorer menu's "Search in folder": scoped with `path:`; the trailing slash
    /// keeps `Notes/` from matching `Notes archive/`.
    fn search_in_folder(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = match folder.is_empty() {
            true => String::new(),
            false => format!("path:\"{folder}/\""),
        };
        self.show_search(&query, Some(window), cx);
    }

    /// Shows a message that expires on its own, the way Obsidian's Notice does; the
    /// status bar keeps only what stays true.
    fn report(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.report_with(message, None, cx);
    }

    /// A notice's button pressed.
    fn run_notice_action(
        &mut self,
        action: NoticeAction,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            NoticeAction::UndoMigration => self.undo_migration(cx),
        }
    }

    /// [`Workspace::report`] with a button after the message.
    fn report_with(
        &mut self,
        message: impl Into<SharedString>,
        action: Option<NoticeAction>,
        cx: &mut Context<Self>,
    ) {
        let home = self.window_of_group(self.active_group_id());
        let id = self.notices.push_with(message.into(), home, action);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_LIFETIME).await;
            this.update(cx, |ws, cx| {
                ws.notices.expire(id);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Creates `Untitled.md` (or the next free number), opens it for editing and asks
    /// for its name, as Ctrl+N does.
    fn dispatch_file_operation(
        &mut self,
        request: FileOperationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            FileOperationRequest::CreateNote { parent } => self.new_note(&parent, window, cx),
        }
    }

    fn new_note(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "md");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                let mode = self.editing_mode();
                self.set_mode(mode, window, cx);
                self.start_rename(&path, window, cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `canvas:new-file`: an empty `Untitled.canvas` (or next free number), opened like a
    /// new note; Obsidian writes it empty (see [[Specification#Observed canvas write behavior]]).
    fn new_canvas(&mut self, folder: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "canvas");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                if self.current.is_some() {
                    self.new_tab(cx);
                }
                self.open_file(path.clone(), None, cx);
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Whether any group's editor holds `path` with bytes other than `on_disk` --
    /// unsaved edits a write would discard.
    fn buffer_differs(&self, path: &str, on_disk: &str, cx: &mut Context<Self>) -> bool {
        let mine = {
            let editor = self.editor.read(cx);
            editor.name.as_ref() == path && editor.text() != on_disk
        };
        mine || self.groups.iter().any(|group| {
            let editor = group.editor.read(cx);
            editor.name.as_ref() == path && editor.text() != on_disk
        })
    }

    /// Writes the canvas file now if dirty, through the editor save's checks: the path
    /// is checked for links, and a file written by someone else is left alone.
    fn flush_canvas_save(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let (dirty, path) = {
            let canvas = self.canvas.read(cx);
            (canvas.is_dirty(), canvas.path().map(str::to_string))
        };
        let Some(path) = path.filter(|_| dirty) else {
            return;
        };
        let absolute = match state.vault().write_path(&path) {
            Ok(absolute) => absolute,
            Err(err) => {
                self.report(format!("{path} not saved: {err}"), cx);
                return;
            }
        };
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if self.canvas.read(cx).matches_file(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{path} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {path}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        let contents = self.canvas.read(cx).to_json();
        match tft_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                self.canvas.update(cx, |c, _| c.mark_saved(contents));
                self.report(format!("{path} saved"), cx);
            }
            Err(err) => self.report(format!("Could not save {path}: {err}"), cx),
        }
        cx.notify();
    }

    /// A canvas view moved: the tab records where it looks, so `workspace.json` gets it
    /// whether or not the tab is live. The wheel reaches an inactive pane.
    fn record_canvas_view_state(
        &mut self,
        canvas: &Entity<CanvasView>,
        view_state: Value,
        cx: &mut Context<Self>,
    ) {
        let path = canvas.read(cx).path().map(str::to_string);
        if path.is_none() {
            return;
        }
        if *canvas == self.canvas {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                if self.current == path {
                    tab.view_state = Some(view_state);
                }
            }
            return;
        }
        if let Some(group) = self.groups.iter_mut().find(|g| g.canvas == *canvas) {
            if let Some(tab) = group.tabs.get_mut(group.active_tab) {
                if tab.path == path {
                    tab.view_state = Some(view_state);
                }
            }
        }
    }

    /// Creates the note a link or the switcher names and opens it.
    fn create_note_for(&mut self, name: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let mut path = state.new_note_path(name, self.current.as_deref());
        // A dangling link naming a canvas makes it, empty, as the embed's "Click to
        // create" offers (the click itself was not observed).
        if FileKind::from_path(name) == FileKind::Canvas {
            if let Some(canvas) = path.strip_suffix(".md") {
                path = canvas.to_string();
            }
        }
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                if let Some(window) = window {
                    let mode = self.editing_mode();
                    self.set_mode(mode, window, cx);
                }
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    fn create_from_link(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.create_note_for(name, Some(window), cx);
    }

    /// Enter in a tree row's own label: the same validator and rename the prompt's box
    /// uses. A name the filesystem refuses leaves the field open and says nothing, as
    /// Obsidian's does -- the row is where the answer belongs.
    fn commit_row_rename(
        &mut self,
        from: String,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(to) = self.renamed_path(&from, value) else {
            return;
        };
        // The filesystem's refusals, a taken name above all, must be known before the
        // field closes, since staying open is how it reports them.
        let allowed = self
            .state
            .as_ref()
            .map(|s| s.check_rename(&from, &to))
            .unwrap_or(Ok(()));
        if allowed.is_err() {
            return;
        }
        self.explorer.update(cx, |e, cx| e.end_rename(cx));
        self.rename_with_links(&from, &to, window, cx);
    }

    fn start_rename(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let is_folder = self
            .state
            .as_ref()
            .map(|s| s.vault().absolute(path).is_dir())
            .unwrap_or(false);
        let name = path.rsplit('/').next().unwrap_or(path);
        let initial = if is_folder { name } else { basename(name) };
        self.open_prompt(
            PendingOp::Rename(path.to_string()),
            PromptSpec {
                title: if is_folder {
                    "Rename folder".into()
                } else {
                    "Rename file".into()
                },
                detail: vec![path.to_string().into()],
                input: Some(initial.to_string()),
                placeholder: "Name".into(),
                confirm: "Rename".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// A row dropped on a folder in the Files view: it moves there under the same
    /// name, numbered `Name 1` if taken. Links follow the rename rules; a drag-move
    /// has no undo.
    fn drop_into_folder(
        &mut self,
        path: &str,
        folder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let to = {
            let vault = state.vault();
            let is_folder = vault.absolute(path).is_dir();
            if !explorer::drop_moves(path, is_folder, folder) {
                return;
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            let (stem, ext) = match name.rsplit_once('.') {
                Some((stem, ext)) if !is_folder && !stem.is_empty() => (stem, ext),
                _ => (name, ""),
            };
            vault.available_path(folder, stem, ext)
        };
        self.rename_with_links(path, &to, window, cx);
    }

    fn start_move(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let is_folder = state.vault().absolute(path).is_dir();
        let own_prefix = format!("{path}/");
        let parent = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut suggestions = vec!["/".to_string()];
        suggestions.extend(
            state
                .vault()
                .folders()
                .iter()
                .filter(|f| *f != path && !f.starts_with(&own_prefix) && f.as_str() != parent)
                .cloned(),
        );
        if parent.is_empty() {
            suggestions.remove(0);
        }
        self.open_prompt(
            PendingOp::Move(path.to_string()),
            PromptSpec {
                title: if is_folder {
                    "Move folder to...".into()
                } else {
                    "Move file to...".into()
                },
                detail: vec![path.to_string().into()],
                input: Some(String::new()),
                placeholder: "Type a folder".into(),
                suggestions,
                confirm: "Move".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    fn start_new_folder(&mut self, parent: &str, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self
            .state
            .as_ref()
            .map(|s| s.vault().available_path(parent, "Untitled", ""))
            .unwrap_or_else(|| "Untitled".to_string());
        let initial = initial.rsplit('/').next().unwrap_or(&initial).to_string();
        self.open_prompt(
            PendingOp::NewFolder {
                parent: parent.to_string(),
            },
            PromptSpec {
                title: "New folder".into(),
                detail: vec![if parent.is_empty() {
                    "In the vault folder".into()
                } else {
                    format!("In {parent}").into()
                }],
                input: Some(initial),
                placeholder: "Folder name".into(),
                confirm: "Create".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    fn start_delete(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if !state.confirm_delete() {
            self.delete_now(path, window, cx);
            return;
        }
        let where_to = match state.trash_option() {
            TrashOption::System => "It will be moved to the system trash.",
            TrashOption::Local => "It will be moved to the vault's .trash folder.",
            TrashOption::None => "It will be deleted permanently.",
        };
        let name = path.rsplit('/').next().unwrap_or(path);
        self.open_prompt(
            PendingOp::Delete(path.to_string()),
            PromptSpec {
                title: format!("Delete \"{name}\"?").into(),
                detail: vec![where_to.into()],
                input: None,
                confirm: "Delete".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    fn delete_now(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let was_current = self.current.as_deref() == Some(path)
            || self
                .current
                .as_deref()
                .is_some_and(|c| c.starts_with(&format!("{path}/")));
        if was_current {
            // Nothing may write the buffer back into a gone file, and no pane may keep describing it.
            self.detach_current(false, &state, cx);
            // The main area may stop painting the view; focus left on it would swallow
            // every key, so `focus_main` picks the right place instead.
            self.focus_main(window, cx);
        }
        self.forget_history(path);
        match state.trash(path) {
            // Obsidian shows nothing for a delete that worked.
            Ok(changes) => self.after_changes(changes, cx),
            Err(err) => self.report(err, cx),
        }
    }

    fn duplicate(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let is_folder = state.vault().absolute(path).is_dir();
        match state.duplicate(path) {
            // Nothing is reported for a copy that worked; the new file or folder is the feedback.
            Ok((copy, changes)) if is_folder => {
                // An empty folder's copy changes no file, so `after_changes` would leave the tree as it was.
                if changes.files.is_empty() {
                    self.refresh_vault_views(cx);
                } else {
                    self.after_changes(changes, cx);
                }
                self.left_tab = LeftTab::Files;
                self.explorer.update(cx, |e, cx| e.reveal_folder(&copy, cx));
                cx.notify();
            }
            Ok((copy, changes)) => {
                self.after_changes(changes, cx);
                self.open_file(copy, None, cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Enter in the prompt: finish the pending operation with `value`.
    fn prompt_submitted(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(op) = self.pending.clone() else {
            self.close_modal(window, cx);
            return;
        };
        let value = value.trim();
        match op {
            PendingOp::RenameHeading { path, line } => {
                self.finish_rename_heading(&path, line, value, window, cx);
            }
            PendingOp::Rename(from) => {
                // The box holds the stem, the same path and validator the view-header inline field uses.
                let to = match self.renamed_path(&from, value) {
                    Ok(to) => to,
                    Err(err) => {
                        self.prompt.update(cx, |p, cx| p.set_error(err, cx));
                        return;
                    }
                };
                self.rename_with_links(&from, &to, window, cx);
            }
            PendingOp::Move(from) => {
                let folder = value.trim_matches('/');
                let name = from.rsplit('/').next().unwrap_or(&from);
                let to = if folder.is_empty() {
                    name.to_string()
                } else {
                    format!("{folder}/{name}")
                };
                self.rename_with_links(&from, &to, window, cx);
            }
            PendingOp::Delete(path) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.delete_now(&path, window, cx);
            }
            PendingOp::NewFolder { parent } => {
                let path = if parent.is_empty() {
                    value.to_string()
                } else {
                    format!("{parent}/{value}")
                };
                let result = self
                    .state
                    .as_ref()
                    .map(|s| s.create_folder(&path))
                    .unwrap_or(Ok(()));
                match result {
                    Ok(()) => {
                        self.pending = None;
                        self.close_modal(window, cx);
                        self.refresh_vault_views(cx);
                        self.explorer.update(cx, |e, cx| e.reveal_folder(&path, cx));
                        self.report(format!("Created folder {path}"), cx);
                    }
                    Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
                }
            }
            PendingOp::UpdateLinks { from, to } => self.finish_rename(&from, &to, true, window, cx),
        }
    }

    /// The prompt's secondary button: "Do not update" the links.
    fn prompt_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(PendingOp::UpdateLinks { from, to }) = self.pending.clone() {
            self.finish_rename(&from, &to, false, window, cx);
        }
    }

    /// The prompt's "Always update": writes `alwaysUpdateLinks: true` first and then
    /// renames. A write that cannot reach disk takes the rename with it.
    fn prompt_tertiary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PendingOp::UpdateLinks { from, to }) = self.pending.clone() else {
            return;
        };
        let Some(state) = self.state.clone() else {
            return;
        };
        let mut app = state.app().clone();
        app.always_update_links = Some(true);
        if let Err(err) = state.apply_setting(&SettingChange::App(app)) {
            self.prompt
                .update(cx, |p, cx| p.set_error(format!("app.json: {err}"), cx));
            return;
        }
        self.finish_rename(&from, &to, true, window, cx);
    }

    /// Escape or a click outside: on the rename prompt this is "Do not update", leaving
    /// the rename standing and the links alone.
    fn prompt_cancelled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(PendingOp::UpdateLinks { from, to }) = self.pending.clone() {
            self.finish_rename(&from, &to, false, window, cx);
            return;
        }
        self.pending = None;
        self.close_modal(window, cx);
    }

    fn rename_with_links(
        &mut self,
        from: &str,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if from == to {
            self.pending = None;
            self.close_modal(window, cx);
            return;
        }
        if state.update_links_automatically() {
            self.finish_rename(from, to, true, window, cx);
            return;
        }
        let (files, links) = state.rename_preview(from, to);
        if links == 0 {
            self.finish_rename(from, to, false, window, cx);
            return;
        }
        self.open_prompt(
            PendingOp::UpdateLinks {
                from: from.to_string(),
                to: to.to_string(),
            },
            update_links_prompt(files, links),
            window,
            cx,
        );
    }

    fn finish_rename(
        &mut self,
        from: &str,
        to: &str,
        update_links: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // The open note's edits must be on disk before its file moves or links are rewritten.
        self.flush_save(cx);
        match state.rename(from, to, update_links) {
            Ok(outcome) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.apply_rename(outcome, cx);
            }
            Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
        }
    }

    /// Follows the open note and the history to their new paths, then refreshes like a filesystem event.
    fn apply_rename(&mut self, outcome: RenameOutcome, cx: &mut Context<Self>) {
        let renamed: HashMap<&str, &str> = outcome
            .renamed
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        // A note's fold record follows it to its new path.
        if let Some(root) = self.state.as_ref().map(|s| s.root.clone()) {
            if let Some(store) = self.fold_store(&root) {
                for (from, to) in &renamed {
                    store.rename(from, to);
                }
            }
        }
        for entry in &mut self.history {
            if let Some(new) = renamed.get(entry.path.as_str()) {
                entry.path = new.to_string();
            }
        }
        for tab in self.stashed_tabs_mut() {
            for entry in &mut tab.history {
                if let Some(new) = renamed.get(entry.path.as_str()) {
                    entry.path = new.to_string();
                }
            }
            if let Some(path) = tab.path.as_mut() {
                if let Some(new) = renamed.get(path.as_str()) {
                    *path = new.to_string();
                }
            }
        }
        // Every group's views follow the rename, the active one's included.
        for group in &self.groups {
            group.editor.update(cx, |e, _| {
                if let Some(new) = renamed.get(e.name.as_ref()) {
                    e.set_name(new.to_string());
                }
            });
            group.reading.update(cx, |r, _| {
                if let Some(new) = r.path.as_deref().and_then(|p| renamed.get(p)) {
                    r.path = Some(new.to_string());
                }
            });
        }
        if let Some(current) = self.current.clone() {
            if let Some(new) = renamed.get(current.as_str()) {
                self.current = Some(new.to_string());
            }
        }
        // Obsidian says nothing about the rename; it shows one Notice only when links
        // were rewritten -- exactly `Updated 1 link in 1 file.`
        let links = (outcome.rewritten_links > 0).then(|| {
            format!(
                "Updated {} link{} in {} file{}.",
                outcome.rewritten_links,
                plural(outcome.rewritten_links),
                outcome.rewritten_files,
                plural(outcome.rewritten_files),
            )
        });
        // A note that changed underneath keeps its own bytes; Obsidian never skips, so this notice stays.
        let skipped = match outcome.stale.len() {
            0 => None,
            n => Some(format!(
                "{n} file{} changed on disk and {} left alone",
                plural(n),
                if n == 1 { "was" } else { "were" }
            )),
        };
        self.after_changes(outcome.changes, cx);
        if let Some(current) = self.current.clone() {
            self.explorer.update(cx, |e, cx| e.follow(&current, cx));
        }
        let notice = match (links, skipped) {
            (None, None) => None,
            (Some(l), None) => Some(l),
            (None, Some(s)) => Some(s),
            (Some(l), Some(s)) => Some(format!("{l} {s}")),
        };
        if let Some(notice) = notice {
            self.report(notice, cx);
        }
    }

    /// `editor:rename-heading`: the file-rename modal's shape, titled "Rename heading",
    /// Save then Cancel. Off an ATX heading line it opens nothing, as Obsidian's run on
    /// a setext heading did.
    fn start_rename_heading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.current.clone() else {
            return;
        };
        let (heading, line) = {
            let e = self.editor.read(cx);
            (e.heading_to_rename(), e.cursor().line as u32)
        };
        let Some(heading) = heading else {
            return;
        };
        self.open_prompt(
            PendingOp::RenameHeading { path, line },
            PromptSpec {
                title: "Rename heading".into(),
                input: Some(heading.text),
                buttons: vec![
                    PromptButton::new("Save", PromptAction::Submit).cta(),
                    PromptButton::new("Cancel", PromptAction::Cancel),
                ],
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// Save in "Rename heading": an empty name writes nothing. Otherwise the note takes
    /// the heading line and its links as one undo step and is written at once; other
    /// linking notes go through [`VaultState::write_link_rewrites`].
    fn finish_rename_heading(
        &mut self,
        path: &str,
        line: u32,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if name.is_empty() {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        self.pending = None;
        self.close_modal(window, cx);
        let own_text = {
            let e = self.editor.read(cx);
            (e.name.as_ref() == path && self.current.as_deref() == Some(path))
                .then(|| e.text().to_string())
        };
        let Some(own_text) = own_text else {
            return;
        };
        // Both guards are dropped before anything takes a write lock.
        let plan = {
            let texts = state.texts();
            state
                .cache()
                .plan_heading_rename(path, &own_text, line, name, &|p| {
                    texts.get(p).map(|t| t.to_string())
                })
        };
        let Some(plan) = plan else {
            return;
        };
        if !plan.own.is_empty() {
            let heading = plan.heading.clone();
            let edits = plan.own.clone();
            self.editor.update(cx, |e, cx| {
                e.apply_heading_rename(edits, &heading, window, cx)
            });
            self.flush_save(cx);
        }
        match state.write_link_rewrites(&plan.rewrites) {
            Ok((changes, stale)) => {
                self.after_changes(changes, cx);
                if !stale.is_empty() {
                    let n = stale.len();
                    self.report(
                        format!(
                            "{n} file{} changed on disk and {} left alone",
                            plural(n),
                            if n == 1 { "was" } else { "were" }
                        ),
                        cx,
                    );
                }
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `markdown:add-alias`: Live Preview writes the `aliases` key at once; Source mode
    /// reveals the File properties pane's aliases input. Reading view is not offered
    /// (see `can_run`).
    fn add_alias(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode.is_editing() || !self.note_has_keys(window, cx) {
            return;
        }
        let in_table =
            self.mode == ViewMode::LivePreview && self.editor.read(cx).draws_properties_table();
        let _ = in_table;
        let changed = self
            .editor
            .update(cx, |e, cx| e.add_alias(true, window, cx));
        // The key reached disk within 0.3 s in Obsidian, not on the autosave's debounce.
        if changed {
            self.flush_save(cx);
        }
        cx.notify();
    }

    /// Renames the open note (same folder and extension) right away; `--rename` for smoke tests.
    pub fn rename_current(&mut self, new_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.current.clone() else {
            return;
        };
        let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let name = from.rsplit('/').next().unwrap_or(&from);
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("md");
        let to = if folder.is_empty() {
            format!("{new_name}.{ext}")
        } else {
            format!("{folder}/{new_name}.{ext}")
        };
        self.finish_rename(&from, &to, true, window, cx);
    }

    /// Opens a file prompt for the open note (`--prompt`): `rename`, `move`, `delete` or `folder`.
    pub fn open_file_prompt(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            return;
        };
        match kind {
            "move" => self.start_move(&current, window, cx),
            "delete" => self.start_delete(&current, window, cx),
            "folder" => self.start_new_folder("", window, cx),
            _ => self.start_rename(&current, window, cx),
        }
    }

    /// Which view the main area paints. A view gets it only when it has this file: an
    /// editor with no file takes unsaved edits, and a stale reading view keeps a
    /// deleted note's links clickable.
    fn main_view(&self, cx: &App) -> MainView {
        if self.active_release_notes().is_some() {
            return MainView::ReleaseNotes;
        }
        let has_note = self.reading.read(cx).path.is_some();
        let image = self
            .current
            .as_deref()
            .is_some_and(|p| shown_as_embed(FileKind::from_path(p)));
        let has_buffer = !self.editor.read(cx).name.is_empty();
        let canvas =
            self.current.is_some() && self.canvas.read(cx).path() == self.current.as_deref();
        let pdf = self.current.is_some() && self.pdf.read(cx).path() == self.current.as_deref();
        Self::main_view_for(self.mode, has_note, image, has_buffer, canvas, pdf)
    }

    /// The rule over what the views have: `has_note`, `image`, `has_buffer`, `canvas`,
    /// `pdf`. A canvas or PDF has no reading/source mode, so it paints in every mode
    /// but the graph's.
    fn main_view_for(
        mode: ViewMode,
        has_note: bool,
        image: bool,
        has_buffer: bool,
        canvas: bool,
        pdf: bool,
    ) -> MainView {
        match mode {
            ViewMode::Graph => MainView::Graph,
            _ if canvas => MainView::Canvas,
            _ if pdf => MainView::Pdf,
            ViewMode::Reading if has_note => MainView::Reading,
            // An image has no source to edit, so it reads the same in every mode.
            ViewMode::LivePreview | ViewMode::Source if image && has_note => MainView::Reading,
            ViewMode::LivePreview | ViewMode::Source if has_buffer => MainView::Editor,
            _ => MainView::Empty,
        }
    }

    /// What one group's rendering reads: live fields for the active group, stashed for others.
    fn group_view(&self, ix: usize) -> GroupView {
        let g = &self.groups[ix];
        if ix == self.active_group {
            return GroupView {
                id: g.id,
                active: true,
                tabs: self.tab_snapshots(),
                active_tab: self.active_tab,
                current: self.current.clone(),
                mode: self.mode,
                history_len: self.history.len(),
                history_ix: self.history_ix,
                detached: self.detached.clone(),
                editor: self.editor.clone(),
                reading: self.reading.clone(),
                canvas: self.canvas.clone(),
                pdf: self.pdf.clone(),
            };
        }
        let tab = g.tabs.get(g.active_tab);
        GroupView {
            id: g.id,
            active: false,
            tabs: g.tabs.clone(),
            active_tab: g.active_tab,
            current: tab.and_then(|t| t.path.clone()),
            mode: tab.map(|t| t.mode).unwrap_or(ViewMode::Reading),
            history_len: tab.map(|t| t.history.len()).unwrap_or(0),
            history_ix: tab.map(|t| t.history_ix).unwrap_or(0),
            detached: g.detached.clone(),
            editor: g.editor.clone(),
            reading: g.reading.clone(),
            canvas: g.canvas.clone(),
            pdf: g.pdf.clone(),
        }
    }

    /// The main area: the layout tree drawn as nested rows and columns, with draggable dividers.
    fn render_layout(
        &self,
        node: &Layout,
        path: &mut Vec<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match node {
            Layout::Group(id) => match self.group_ix(*id) {
                Some(ix) => self.render_group(ix, theme, cx),
                None => div().into_any_element(),
            },
            Layout::Split {
                direction,
                children,
                ..
            } => {
                let row = *direction == SplitDirection::Vertical;
                // A split whose children lack a `dimension` shares its room equally, as
                // Obsidian draws one.
                let dims: Option<Vec<f64>> =
                    children.iter().map(|c| self.dimension_of(c)).collect();
                let split_path = path.clone();
                let drag_path = split_path.clone();
                let mut container = div()
                    .id(SharedString::from(format!("split-{split_path:?}")))
                    .flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .when(row, |d| d.flex_row())
                    .when(!row, |d| d.flex_col())
                    .on_drag_move(cx.listener(
                        move |this, e: &DragMoveEvent<DividerDrag>, _, cx| {
                            let drag = e.drag(cx).clone();
                            if drag.path == drag_path {
                                this.drag_divider(&drag, e.bounds, e.event.position, cx);
                            }
                        },
                    ));
                for (ix, child) in children.iter().enumerate() {
                    path.push(ix);
                    let body = self.render_layout(child, path, theme, cx);
                    path.pop();
                    let grow = dims.as_ref().map(|d| d[ix] as f32).unwrap_or(1.0);
                    let mut cell = div()
                        .relative()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex_basis(px(0.))
                        .child(body);
                    cell.style().flex_grow = Some(grow.max(0.01));
                    if ix > 0 {
                        let drag = DividerDrag {
                            window: self.rendering_window,
                            path: split_path.clone(),
                            before: ix - 1,
                        };
                        let handle = div()
                            .id(("divider", ix as u64))
                            .absolute()
                            .when(row, |d| {
                                d.left(px(0.))
                                    .top_0()
                                    .h_full()
                                    .w(px(6.))
                                    .cursor_col_resize()
                            })
                            .when(!row, |d| {
                                d.top(px(0.))
                                    .left_0()
                                    .w_full()
                                    .h(px(6.))
                                    .cursor_row_resize()
                            })
                            .occlude()
                            .on_drag(drag, |_, _, _, cx| cx.new(|_| NoDragPreview));
                        cell = cell
                            .when(row, |d| d.border_l_1())
                            .when(!row, |d| d.border_t_1())
                            .border_color(theme.tokens.background_modifier_border)
                            .child(handle);
                    }
                    container = container.child(cell);
                }
                container.into_any_element()
            }
        }
    }

    /// One group: its tab strip, view header and view. A mouse down in a non-active
    /// group makes it active before the view handles the click.
    fn render_group(&self, ix: usize, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = self.group_view(ix);
        let id = view.id;
        // The tree being drawn: the main area's, or the popout's whose window is rendering.
        let tree = self.tree(self.rendering_window);
        let mut order = Vec::new();
        tree.groups(&mut order);
        // Top left is the first group however the splits run; top right needs the tree's shape.
        let first = order.first() == Some(&id);
        let last = tree.top_right() == Some(id);
        div()
            .id(("group", id))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(!view.active, |d| {
                d.capture_any_mouse_down(
                    cx.listener(move |this, _, _, cx| this.activate_group(id, cx)),
                )
            })
            .child(self.render_tab_strip(&view, first, last, theme, cx))
            .map(|d| {
                // Every tab draws its view header, in the main area or a popout.
                d.when(self.show_view_header(), |d| {
                    d.child(self.render_header(&view, theme, cx))
                })
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.render_group_main(&view, theme, cx))
                        .when(self.dragging_tab, |d| {
                            d.child(self.render_drop_zones(id, theme, cx))
                        }),
                )
            })
            .into_any_element()
    }

    /// The drop target over a group's content while a tab is dragged: it lights the
    /// zone the tab would land in and takes the drop, covering the view.
    fn render_drop_zones(
        &self,
        id: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let zone = self.tab_drop.filter(|(g, _)| *g == id).map(|(_, z)| z);
        let wash = theme.tokens.interactive_accent.opacity(0.15);
        let edge = theme.tokens.interactive_accent.opacity(0.6);
        div()
            .id(("drop-zones", id))
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.tab_drag_moved(id, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                this.drop_tab_on_group(drag, id, window, cx)
            }))
            .child(measure_for_drag(&self.drag_bounds, DragPart::Content(id)))
            .when_some(zone, |d, zone| {
                let half = relative(0.5);
                d.child(
                    div()
                        .absolute()
                        .bg(wash)
                        .border_2()
                        .border_color(edge)
                        .rounded(px(6.))
                        .map(|d| match zone {
                            DropZone::Center => d.top_0().left_0().right_0().bottom_0(),
                            DropZone::Left => d.top_0().left_0().bottom_0().w(half),
                            DropZone::Right => d.top_0().right_0().bottom_0().w(half),
                            DropZone::Top => d.top_0().left_0().right_0().h(half),
                            DropZone::Bottom => d.bottom_0().left_0().right_0().h(half),
                        }),
                )
            })
    }

    /// The view header: back/forward, the note's folder and name, the graph toggle and
    /// the mode toggle. Its buttons act on their group, made active first.
    fn render_header(&self, view: &GroupView, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let id = view.id;
        // The leaf's name, the same string its tab carries: "New tab" with nothing open.
        let title = view
            .tabs
            .get(view.active_tab)
            .map(|t| t.title())
            .unwrap_or_else(|| leaf_title(view.current.as_deref(), view.mode));
        let crumbs: SharedString = view
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/').map(|(dir, _)| dir.replace('/', " / ")))
            .unwrap_or_default()
            .into();
        // Obsidian's view action shows the current mode: a book while reading, a pencil while editing.
        let mode_icon = match view.mode {
            ViewMode::Reading => "book-open",
            _ => "pencil",
        };
        let can_back = view.history_ix > 1;
        let can_forward = view.history_ix < view.history_len;
        let release_notes_leaf = view
            .tabs
            .get(view.active_tab)
            .and_then(release_notes::version_of)
            .is_some();
        // `.view-header-title` is `--text-normal`; a non-active group keeps its title muted.
        let title_color = if view.active {
            theme.tokens.text_normal
        } else {
            theme.tokens.text_muted
        };
        // Obsidian's view header: 40 px, 12 px side padding, history arrows at 40 %
        // when unusable, the title in the small UI size.
        crate::ui::workspace_view_header(theme)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(
                        crate::ui::icon_button("back", "arrow-left", theme)
                            .when(!can_back, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_back(&GoBack, window, cx)
                            })),
                    )
                    .child(
                        crate::ui::icon_button("forward", "arrow-right", theme)
                            .when(!can_forward, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_forward(&GoForward, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    // `.view-header-title-parent`: `--text-muted`.
                    .when(!crumbs.is_empty(), |d| {
                        d.child(
                            div()
                                .text_color(theme.tokens.text_muted)
                                .child(crumbs.clone()),
                        )
                        .child(div().text_color(theme.tokens.text_muted).child("/"))
                    })
                    .child(self.render_title(view, title.clone(), title_color, theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .child(if release_notes_leaf {
                        // The release notes are not a note: no graph and no mode switch.
                        div()
                    } else {
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(2.))
                            .child(
                                crate::ui::icon_button("graph", "git-fork", theme)
                                    .when(view.mode == ViewMode::Graph, |d| {
                                        d.bg(theme.tokens.background_modifier_hover)
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        this.open_graph_action(&OpenGraph, window, cx)
                                    })),
                            )
                            .when(header_shows_mode_switch(&view.current, view.mode), |d| {
                                d.child(crate::ui::icon_button("mode", mode_icon, theme).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        this.toggle_mode(&ToggleMode, window, cx)
                                    }),
                                ))
                            })
                    }),
            )
    }

    /// A right click on tab `ix`: the tab header's menu at the click. The click leaves
    /// the active tab where it was, so the rows name their tab.
    fn open_tab_menu(
        &mut self,
        id: u64,
        ix: usize,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.stash_active_group();
        let Some(gix) = self.group_ix(id) else {
            return;
        };
        let active = gix == self.active_group && ix == self.groups[gix].active_tab;
        let group = &self.groups[gix];
        let Some(tab) = group.tabs.get(ix) else {
            return;
        };
        self.file_menu = Some(file_menu::Menu {
            position,
            submenu: false,
            anchor: Anchor::TopLeft,
            target: file_menu::Target::Tab {
                group: id,
                ix,
                path: tab.path.clone().unwrap_or_default(),
                tabs: group.tabs.len(),
                pinned: tab.pinned,
                active,
            },
        });
        cx.notify();
    }

    /// A tab header row picked: its group made active; rows that close or move act on
    /// its index, the rest on the active tab.
    fn tab_menu_pick(
        &mut self,
        pick: Pick,
        group: u64,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_group(group, cx);
        if self.active_group_id() != group || ix >= self.tabs.len() {
            return;
        }
        match pick {
            Pick::CloseTab => self.close_tab(ix, cx),
            Pick::CloseOthers => self.close_tabs_but(|i, t| i == ix || t.pinned, Some(ix), cx),
            Pick::CloseAfter => self.close_tabs_but(|i, t| i <= ix || t.pinned, Some(ix), cx),
            Pick::CloseAll => self.close_all_tabs(cx),
            Pick::TogglePin => {
                self.stash_active_tab();
                self.tabs[ix].pinned = !self.tabs[ix].pinned;
                self.stash_active_group();
                cx.notify();
            }
            Pick::MoveToNewWindow => {
                self.activate_tab(ix, cx);
                self.move_to_new_window(window, cx);
            }
            _ => {}
        }
    }

    /// `workspace:toggle-pin`: the active tab pinned or unpinned.
    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        self.stash_active_tab();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.pinned = !tab.pinned;
            self.stash_active_group();
            cx.notify();
        }
    }

    fn active_tab_pinned(&self) -> bool {
        self.tabs.get(self.active_tab).is_some_and(|t| t.pinned)
    }

    /// The view header's title, editable where it stands, so a click starts an edit
    /// (see [[Specification#The view-header title is an inline rename field]]). Painted
    /// as a shaped line so a click lands the caret on a glyph; a leaf with no file is not renamed.
    fn render_title(
        &self,
        view: &GroupView,
        title: SharedString,
        color: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = view.id;
        let editing = self.title_edit.as_ref().is_some_and(|e| e.group == id);
        let read_entity = cx.entity();
        let paint_entity = cx.entity();
        let read_focus = self.title_focus.clone();
        let paint_focus = self.title_focus.clone();
        let mut selection_color = theme.tokens.interactive_accent;
        selection_color.a = if theme.dark { 0.35 } else { 0.25 };
        let caret_color = theme.tokens.text_normal;
        let body = canvas(
            move |bounds, window, cx| {
                let ws = read_entity.read(cx);
                let edit = ws.title_edit.as_ref().filter(|e| e.group == id);
                let text: SharedString = match edit {
                    Some(edit) => edit.editing.field.text().to_string().into(),
                    None => title.clone(),
                };
                let scroll_x = ws
                    .title_layout
                    .get(&id)
                    .map(|l| l.scroll_x)
                    .unwrap_or(px(0.));
                inline_field::shape(
                    text,
                    edit.map(|e| &e.editing),
                    color,
                    read_focus.is_focused(window),
                    bounds,
                    scroll_x,
                    window,
                )
            },
            move |bounds, prepaint: inline_field::Prepaint, window, cx| {
                // The input method is pointed at the field only while it is open: the
                // title is not a tab stop.
                if editing {
                    window.handle_input(
                        &paint_focus,
                        ElementInputHandler::new(bounds, paint_entity.clone()),
                        cx,
                    );
                }
                let layout =
                    inline_field::paint(prepaint, bounds, selection_color, caret_color, window, cx);
                paint_entity.update(cx, |ws, _| {
                    ws.title_layout.insert(id, layout);
                });
            },
        )
        .size_full();
        let mut host = div()
            .id(("view-title", id))
            .flex_1()
            .min_w_0()
            // The canvas has no size of its own, so the row's height is set here.
            .h(px(HEADER_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(FONT_UI_SMALL))
            .text_color(color)
            .child(body);
        if let Some(path) = view.current.clone() {
            host = host.cursor_text().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.title_clicked(id, path.clone(), event, window, cx)
                }),
            );
        }
        if !editing {
            return host.into_any_element();
        }
        // The keys here are only a field's, in the crate's own field key context.
        let host = host.track_focus(&self.title_focus).key_context("TextInput");
        inline_field::bind_keys(host, cx).into_any_element()
    }

    /// A click on a view-header title focuses the field and puts the caret where the
    /// pointer landed; a double click takes the word. A click past the text changes nothing.
    fn title_clicked(
        &mut self,
        group: u64,
        path: String,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.title_layout.get(&group).cloned() else {
            return;
        };
        // A click in another group's header makes that group active first.
        self.activate_group(group, cx);
        // The field holds the name without its extension, the same stem the rename prompt boxes.
        let name = leaf_title(Some(&path), ViewMode::Reading).to_string();
        let mut edit = match self.title_edit.take() {
            Some(edit) if edit.group == group && edit.path == path => edit,
            _ => TitleEdit {
                group,
                path,
                editing: inline_field::Editing::new(name),
            },
        };
        let Some(at) = layout.offset_for(event.position, edit.editing.field.text()) else {
            self.title_edit = None;
            return;
        };
        edit.editing
            .field
            .click(at, event.click_count, event.modifiers.shift);
        self.title_edit = Some(edit);
        // The field's focus handle joins the dispatch tree only with the frame that
        // paints it; a click landing before that frame takes the keys next frame.
        let focus = self.title_focus.clone();
        window.focus(&focus, cx);
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    /// Enter in the view-header title: the same validator and link rewrite as the
    /// rename prompt. A refused name leaves the field open and says why.
    fn commit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.title_edit.take() else {
            return;
        };
        let value = edit.editing.field.text().trim().to_string();
        let to = match self.renamed_path(&edit.path, &value) {
            Ok(to) => to,
            Err(err) => {
                self.title_edit = Some(edit);
                self.report(err, cx);
                return;
            }
        };
        // The keyboard goes back to the view before the rename, which may raise the
        // "Update links?" prompt and want it.
        self.focus_main(window, cx);
        self.rename_with_links(&edit.path, &to, window, cx);
        cx.notify();
    }

    /// Enter in the note's inline title: the same path, validator and link rewrite as
    /// the view-header title.
    fn rename_from_inline_title(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.current.clone() else {
            return;
        };
        match self.renamed_path(&from, value.trim()) {
            Ok(to) => self.rename_with_links(&from, &to, window, cx),
            Err(err) => self.report(err, cx),
        }
    }

    /// Escape in the view-header title: the edit is abandoned, as Obsidian's does.
    fn cancel_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_edit.take().is_none() {
            return;
        }
        self.focus_main(window, cx);
        cx.notify();
    }

    /// The file group `id` shows, wherever that group's fields live.
    fn group_current(&self, id: u64) -> Option<&str> {
        let ix = self.group_ix(id)?;
        if ix == self.active_group {
            return self.current.as_deref();
        }
        let group = &self.groups[ix];
        group.tabs.get(group.active_tab)?.path.as_deref()
    }

    /// [`renamed_path`] for a vault path, asked whether it is a folder.
    fn renamed_path(&self, from: &str, value: &str) -> Result<String, String> {
        let is_folder = self
            .state
            .as_ref()
            .map(|s| s.vault().absolute(from).is_dir())
            .unwrap_or(false);
        renamed_path(from, value, is_folder)
    }

    /// What a group's area paints, by [`Workspace::main_view`]'s rule. There is one
    /// graph view, so a second group says where it is instead.
    fn render_group_main(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_note = view.reading.read(cx).path.is_some();
        let image = view
            .current
            .as_deref()
            .is_some_and(|p| shown_as_embed(FileKind::from_path(p)));
        let has_buffer = !view.editor.read(cx).name.is_empty();
        let canvas =
            view.current.is_some() && view.canvas.read(cx).path() == view.current.as_deref();
        let pdf = view.current.is_some() && view.pdf.read(cx).path() == view.current.as_deref();
        if let Some(version) = view
            .tabs
            .get(view.active_tab)
            .and_then(release_notes::version_of)
        {
            return self.release_notes_view(&version, cx).into_any_element();
        }
        match Self::main_view_for(view.mode, has_note, image, has_buffer, canvas, pdf) {
            MainView::Canvas => view.canvas.clone().into_any_element(),
            MainView::Pdf => view.pdf.clone().into_any_element(),
            MainView::Graph if self.graph_group() == Some(view.id) => {
                self.graph.clone().into_any_element()
            }
            MainView::Graph => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.tokens.text_muted)
                .child("The graph is open in another pane")
                .into_any_element(),
            MainView::Reading | MainView::ReleaseNotes => view.reading.clone().into_any_element(),
            MainView::Editor => Self::render_editor(&view.editor, view.detached.clone(), theme),
            MainView::Empty => match view.current.as_deref() {
                Some(path) => Self::render_unsupported_file(path, theme),
                None => self.render_empty_tab(theme, cx),
            },
        }
    }

    /// The editor, under a line naming the lost file if its buffer is no longer backed
    /// by one; the alternative is unreported typing.
    fn render_editor(
        editor: &Entity<Editor>,
        detached: Option<SharedString>,
        theme: &Theme,
    ) -> gpui::AnyElement {
        let Some(name) = detached else {
            return editor.clone().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .bg(theme.tokens.background_modifier_hover)
                    .text_color(theme.tokens.text_muted)
                    .border_b_1()
                    .border_color(theme.tokens.background_modifier_border)
                    .child(format!(
                        "{name} — this buffer no longer belongs to a file in the vault, \
                         and nothing saves it"
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(editor.clone().into_any_element()),
            )
            .into_any_element()
    }

    /// A file whose kind no view can draw: an error message on its own.
    fn render_unsupported_file(path: &str, theme: &Theme) -> gpui::AnyElement {
        let message: SharedString = format!(
            "{:?} files are not supported yet",
            FileKind::from_path(path)
        )
        .into();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.tokens.text_muted)
            .child(message)
            .into_any_element()
    }

    /// The empty tab: the four pills Obsidian offers, each running its command, under
    /// TFT's mark. The body is the same however the tab emptied.
    fn render_empty_tab(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let registry = cx.global::<Commands>().0.clone();
        let pills: Vec<_> =
            empty_tab_actions(&registry)
                .into_iter()
                .map(|action| {
                    let command = action.command;
                    // Obsidian's `mod-pill`: fully rounded, the label in the UI size with
                    // the hotkey muted behind it.
                    div()
                        .id(action.id)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(14.))
                        .py(px(6.))
                        .rounded(px(14.))
                        .bg(theme.tokens.background_modifier_hover)
                        .border_1()
                        .border_color(theme.tokens.background_modifier_border)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                        .text_size(px(FONT_UI_SMALL))
                        .child(
                            div()
                                .text_color(theme.tokens.text_normal)
                                .child(action.label),
                        )
                        .children(
                            action
                                .hotkey
                                .map(|keys| div().text_color(theme.tokens.text_faint).child(keys)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_command(command, window, cx)
                        }))
                })
                .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.tokens.text_muted)
            .child(logo(104.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .children(pills),
            )
            .into_any_element()
    }

    // ----- tabs ------------------------------------------------------------------

    /// Copies the active tab's live fields into its slot.
    fn stash_active_tab(&mut self) {
        let slot = self.tabs.get(self.active_tab);
        let leaf = slot.and_then(|t| t.leaf.clone());
        // The view state belongs to the canvas; a tab that moved on to another file leaves it behind.
        let view_state = slot
            .filter(|t| t.path == self.current)
            .and_then(|t| t.view_state.clone());
        let pinned = slot.is_some_and(|t| t.pinned);
        let tab = Tab {
            node: None,
            path: self.current.clone(),
            mode: self.mode,
            history: self.history.clone(),
            history_ix: self.history_ix,
            leaf,
            view_state,
            pinned,
        };
        match self.tabs.get_mut(self.active_tab) {
            Some(slot) => *slot = tab,
            None => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
        }
    }

    /// Every tab as it stands, the active one from the live fields.
    fn tab_snapshots(&self) -> Vec<Tab> {
        let mut tabs = self.tabs.clone();
        if let Some(slot) = tabs.get_mut(self.active_tab) {
            slot.path = self.current.clone();
            slot.mode = self.mode;
            slot.history = self.history.clone();
            slot.history_ix = self.history_ix;
        }
        tabs
    }

    /// Puts `tabs[active_tab]` on screen: its note in its mode, or the empty state.
    fn load_tab(&mut self, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        let Some(tab) = self.tabs.get(self.active_tab).cloned() else {
            return;
        };
        self.history = tab.history;
        self.history_ix = tab.history_ix;
        if tab.mode == ViewMode::Graph && self.mode != ViewMode::Graph {
            self.mode_before_graph = self.mode;
        }
        self.mode = tab.mode;
        if tab.mode.is_editing() {
            self.editor.update(cx, |e, cx| {
                e.set_live_preview(tab.mode == ViewMode::LivePreview, cx)
            });
        }
        match (tab.path, self.state.clone()) {
            (Some(path), Some(state)) => {
                self.current = None;
                self.show_file(&path, &state, cx);
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (path, None) => self.current = path,
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// The empty tab: no note and nothing describing one. A buffer with unsaved edits
    /// nowhere else stays, marked as saved nowhere.
    fn show_empty(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        self.release_buffer(cx);
        self.release_canvas(cx);
        self.release_pdf(cx);
        self.reading.update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.current = None;
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
        self.status = SharedString::default();
    }

    /// `workspace:new-tab` and the strip's `+`: the new tab goes on the strip's right
    /// end and becomes active, whichever tab was active.
    pub fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_tab();
        let mode = self.default_mode();
        self.tabs.push(Tab::empty(mode));
        self.active_tab = self.tabs.len() - 1;
        self.load_tab(cx);
    }

    /// Opens `path` in a new tab right after the active one and makes it active: where
    /// Obsidian 1.15 puts a tab a link, a Files row or the quick switcher opens.
    pub fn open_in_new_tab(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_tab();
        let mode = self.default_mode();
        self.active_tab = place_new_tab(&mut self.tabs, self.active_tab, Tab::empty(mode));
        self.load_tab(cx);
        self.open_file(path, subpath, cx);
    }

    /// Whether a tab opened from a link goes to the reader; "Always focus new tabs" is fixed on.
    fn focus_new_tabs(&self) -> bool {
        crate::fixed::FOCUS_NEW_TABS
    }

    /// A link, row, bookmark or "Open in new tab": a tab after the active one, made
    /// active only while "Always focus new tabs" is on. Off, it opens behind and the
    /// active tab keeps the keyboard. The switcher's and `new-tab`'s tabs are always active.
    pub fn open_link_in_new_tab(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        if self.focus_new_tabs() {
            return self.open_in_new_tab(path, subpath, cx);
        }
        self.open_in_background_tab(path, cx);
    }

    /// `path` in a new tab after the active one, which stays active. The tab holds one
    /// history entry and loads when shown, as a restored tab does.
    fn open_in_background_tab(&mut self, path: String, cx: &mut Context<Self>) {
        self.stash_active_tab();
        let mode = self.default_mode();
        place_new_tab(&mut self.tabs, self.active_tab, Tab::unloaded(path, mode));
        cx.notify();
    }

    pub fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() || ix == self.active_tab {
            return;
        }
        self.flush_save(cx);
        self.stash_active_tab();
        self.active_tab = ix;
        self.load_tab(cx);
    }

    /// `workspace:next-tab` and `previous-tab`: `step` tabs along, wrapping.
    fn cycle_tab(&mut self, step: usize, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        if n > 1 {
            self.activate_tab((self.active_tab + step) % n, cx);
        }
    }

    /// `workspace:close`: the last close leaves the empty tab; the tab to the right
    /// takes over, or the one to the left at the end of the strip.
    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_tab();
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        self.remember_closed(ix, tab);
        if self.tabs.is_empty() {
            // A group whose last tab closes goes with it, unless it is the main area's
            // only one. A popout's goes regardless, and its window with it.
            if self.active_group_in_popout() || self.main_group_count() > 1 {
                self.tabs.push(Tab::empty(self.default_mode()));
                self.active_tab = 0;
                let id = self.active_group_id();
                self.close_group(id, false, cx);
                return;
            }
            self.tabs.push(Tab::empty(self.default_mode()));
            self.active_tab = 0;
            self.load_tab(cx);
        } else if ix == self.active_tab {
            self.active_tab = ix.min(self.tabs.len() - 1);
            self.load_tab(cx);
        } else {
            if ix < self.active_tab {
                self.active_tab -= 1;
            }
            cx.notify();
        }
    }

    fn remember_closed(&mut self, ix: usize, tab: Tab) {
        // An empty tab is not worth bringing back.
        if tab.path.is_some()
            || tab.mode == ViewMode::Graph
            || release_notes::version_of(&tab).is_some()
        {
            self.closed_tabs.push((ix, tab));
            if self.closed_tabs.len() > CLOSED_TABS_KEPT {
                self.closed_tabs.remove(0);
            }
        }
    }

    /// `workspace:close-others`: every tab but the active one and the pinned ones (observed).
    fn close_other_tabs(&mut self, cx: &mut Context<Self>) {
        let active = self.active_tab;
        self.close_tabs_but(|i, t| i == active || t.pinned, Some(active), cx);
    }

    /// The tab header's "Close all": every tab but the pinned ones, the first of which
    /// stays active (observed). With none pinned, tabs close from the right.
    fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        self.stash_active_tab();
        if self.tabs.iter().any(|t| t.pinned) {
            return self.close_tabs_but(|_, t| t.pinned, None, cx);
        }
        let group = self.active_group_id();
        for ix in (0..self.tabs.len()).rev() {
            if self.active_group_id() != group {
                break;
            }
            self.tabs[ix].pinned = false;
            self.close_tab(ix, cx);
        }
    }

    /// Closes every tab of the active group `keep` turns down, keeping one. `prefer` is
    /// the tab to leave active when it is kept.
    fn close_tabs_but(
        &mut self,
        keep: impl Fn(usize, &Tab) -> bool,
        prefer: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_tab();
        let old_active = self.active_tab;
        let mut kept = Vec::new();
        let (mut preferred, mut was_active) = (None, None);
        for (i, tab) in std::mem::take(&mut self.tabs).into_iter().enumerate() {
            if keep(i, &tab) {
                if Some(i) == prefer {
                    preferred = Some(kept.len());
                }
                if i == old_active {
                    was_active = Some(kept.len());
                }
                kept.push(tab);
            } else {
                self.remember_closed(i, tab);
            }
        }
        debug_assert!(!kept.is_empty());
        if kept.is_empty() {
            kept.push(Tab::empty(self.default_mode()));
        }
        self.tabs = kept;
        let active = preferred.or(was_active).unwrap_or(0);
        self.active_tab = active;
        if was_active == Some(active) {
            cx.notify();
        } else {
            self.load_tab(cx);
        }
    }

    /// `workspace:undo-close-pane`: the most recently closed tab, back where it was.
    fn undo_close_tab(&mut self, cx: &mut Context<Self>) {
        let Some((ix, tab)) = self.closed_tabs.pop() else {
            return;
        };
        self.flush_save(cx);
        self.stash_active_tab();
        let at = ix.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active_tab = at;
        self.load_tab(cx);
    }

    // ----- tab groups --------------------------------------------------------------

    fn active_group_id(&self) -> u64 {
        self.groups[self.active_group].id
    }

    fn group_ix(&self, id: u64) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    /// Copies the active group's live fields into its slot.
    fn stash_active_group(&mut self) {
        self.stash_active_tab();
        let ix = self.active_group;
        let group = &mut self.groups[ix];
        group.tabs = self.tabs.clone();
        group.active_tab = self.active_tab;
        group.mode_before_graph = self.mode_before_graph;
        group.detached = self.detached.clone();
    }

    /// Makes group `ix` the live one. Nothing reloads, but everything following the
    /// active note is pointed at it, and a vanished note is let go.
    fn load_group(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        self.active_group = ix;
        let group = &self.groups[ix];
        let id = group.id;
        self.tabs = group.tabs.clone();
        self.active_tab = group.active_tab.min(self.tabs.len().saturating_sub(1));
        self.mode_before_graph = group.mode_before_graph;
        self.detached = group.detached.clone();
        self.editor = group.editor.clone();
        self.reading = group.reading.clone();
        self.canvas = group.canvas.clone();
        self.pdf = group.pdf.clone();
        let tab = self
            .tabs
            .get(self.active_tab)
            .cloned()
            .unwrap_or_else(|| Tab::empty(self.default_mode()));
        self.current = tab.path;
        self.mode = tab.mode;
        self.history = tab.history;
        self.history_ix = tab.history_ix;
        self.group_mru.retain(|g| *g != id);
        self.group_mru.insert(0, id);
        self.save_task = None;
        match (self.current.clone(), self.state.clone()) {
            (Some(path), Some(state)) => {
                if state.vault().absolute(&path).is_file() {
                    self.explorer.update(cx, |e, cx| e.follow(&path, cx));
                    self.graph
                        .update(cx, |g, cx| g.set_current(Some(path.clone()), cx));
                    self.status = path.into();
                } else {
                    self.detach_current(true, &state, cx);
                }
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (_, None) => {}
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// Hands the keyboard and live fields to group `id`: a click, a focus command, or
    /// the group that takes over.
    pub fn activate_group(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        if ix == self.active_group {
            return;
        }
        self.flush_save(cx);
        self.stash_active_group();
        self.load_group(ix, cx);
    }

    /// A group of its own, with an editor and reading view like the others', not yet in the layout.
    fn new_group(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let editor = cx.new(|cx| crate::new_editor(window, cx));
        let reading = cx.new(ReadingView::new);
        let canvas = cx.new(CanvasView::new);
        let pdf = cx.new(PdfView::new);
        self._subscriptions
            .extend(view_subscriptions(&editor, &reading, &canvas, &pdf, cx));
        let id = self.next_group_id;
        self.next_group_id += 1;
        if let Some(state) = self.state.clone() {
            Self::give_editor_the_vault(&editor, &state, cx);
        }
        self.groups
            .push(TabGroup::new(id, editor, reading, canvas, pdf, mode));
        if let Some(state) = self.state.clone() {
            self.apply_app_config(&state, cx);
        }
        id
    }

    /// `workspace:split-vertical`/`split-horizontal`: a new group beside the active one
    /// showing the same note and mode, taking the focus (see [[Specification#Observed tab group behavior]]).
    fn split(&mut self, direction: SplitDirection, window: &mut Window, cx: &mut Context<Self>) {
        self.split_beside(None, direction, window, cx);
    }

    /// The explorer menu's "Open to the right": a new group beside the active one on
    /// `path`, unlike a split, which keeps the group's own note.
    fn open_to_right(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.split_beside(Some(path), SplitDirection::Vertical, window, cx);
    }

    /// The two share: `None` duplicates the active tab (a split), `Some(path)` opens a
    /// fresh tab in the default mode. Flush and stash happen before the tab is built,
    /// so `view_state` is the one just written.
    fn split_beside(
        &mut self,
        path: Option<String>,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let from = self.active_group_id();
        let (tab, mode_before_graph) = match path {
            None => (self.copy_of_active_tab(), self.mode_before_graph),
            Some(path) => {
                let tab = self.fresh_tab(path);
                let mode = tab.mode;
                (tab, mode)
            }
        };
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(from, id, direction, true);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.groups[ix].mode_before_graph = mode_before_graph;
        self.load_group(ix, cx);
        // The new group's views are empty: put the note in them.
        if let (Some(path), Some(state)) = (self.current.clone(), self.state.clone()) {
            if self.mode.is_editing() {
                let live = self.mode == ViewMode::LivePreview;
                self.editor.update(cx, |e, cx| e.set_live_preview(live, cx));
            }
            self.show_file(&path, &state, cx);
        }
    }

    /// A twin of the active tab (a split or "Open in new window"): same note, mode,
    /// history and view state; call with the active group stashed.
    fn copy_of_active_tab(&self) -> Tab {
        Tab {
            node: None,
            path: self.current.clone(),
            mode: self.mode,
            history: self.history.clone(),
            history_ix: self.history_ix,
            leaf: None,
            view_state: self
                .tabs
                .get(self.active_tab)
                .and_then(|t| t.view_state.clone()),
            pinned: false,
        }
    }

    /// A fresh tab on `path`, no history or view state, in the default mode: what
    /// "Open to the right" and a row's "Open in new window" show.
    fn fresh_tab(&self, path: String) -> Tab {
        Tab {
            node: None,
            path: Some(path),
            mode: self.default_mode(),
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
            pinned: false,
        }
    }

    /// Puts group `new` beside `from` along `direction`, halving the `dimension` when
    /// they become siblings or moving it onto the wrapping split.
    fn place_beside(&mut self, from: u64, new: u64, direction: SplitDirection, after: bool) {
        let Some(from_ix) = self.group_ix(from) else {
            return;
        };
        let dimension = self.groups[from_ix].dimension();
        // The tree `from` is in: a popout's groups split inside their own
        // window.
        let window = self.window_of_group(from);
        match self
            .tree_mut(window)
            .insert_beside(from, new, direction, after)
        {
            Some(false) => {
                if let Some(d) = dimension {
                    self.groups[from_ix].set_dimension(Some(d / 2.0));
                    if let Some(new_ix) = self.group_ix(new) {
                        self.groups[new_ix].set_dimension(Some(d / 2.0));
                    }
                }
            }
            Some(true) => {
                self.groups[from_ix].set_dimension(None);
                let tree = self.tree_mut(window);
                if let Some(mut path) = tree.path_to(from) {
                    path.pop();
                    if let Some(Layout::Split { node, .. }) = tree.node_at_mut(&path) {
                        set_dimension(node, dimension);
                    }
                }
            }
            None => {}
        }
    }

    /// The popout window group `id` is in, by its id; `None` for the main area.
    fn window_of_group(&self, id: u64) -> Option<u64> {
        self.windows
            .iter()
            .find(|w| w.tree.contains(id))
            .map(|w| w.id)
    }

    /// The layout tree of a popout, or the main area's for `None`. A gone window reads as the main area.
    fn tree(&self, window: Option<u64>) -> &Layout {
        window
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .map(|w| &w.tree)
            .unwrap_or(&self.layout_tree)
    }

    fn tree_mut(&mut self, window: Option<u64>) -> &mut Layout {
        match window.and_then(|id| self.windows.iter().position(|w| w.id == id)) {
            Some(wix) => &mut self.windows[wix].tree,
            None => &mut self.layout_tree,
        }
    }

    /// Whether the active group is a popout's.
    fn active_group_in_popout(&self) -> bool {
        self.window_of_group(self.active_group_id()).is_some()
    }

    /// The window the active group is in: its popout while open, else the main window.
    /// Where the keyboard belongs, and the window the control socket drives.
    pub fn active_window(&self, cx: &App) -> gpui::AnyWindowHandle {
        self.window_of_group(self.active_group_id())
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .filter(|w| w.is_open(cx))
            .and_then(|w| w.handle)
            .map(Into::into)
            .unwrap_or(self.main_window)
    }

    /// The same without checking the window is up, for code inside that window's own
    /// update, where it reads as closed.
    fn active_window_handle(&self) -> gpui::AnyWindowHandle {
        self.window_of_group(self.active_group_id())
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .and_then(|w| w.handle)
            .map(Into::into)
            .unwrap_or(self.main_window)
    }

    // ----- popout windows ----------------------------------------------------------

    /// The desktop gave window `win` the focus: unless its active group is already
    /// there, the most recently used group of it becomes active, as Obsidian's active
    /// leaf follows the front window.
    pub(crate) fn window_activated(&mut self, win: Option<u64>, cx: &mut Context<Self>) {
        if self.window_of_group(self.active_group_id()) == win {
            return;
        }
        let mut in_window = Vec::new();
        self.tree(win).groups(&mut in_window);
        let Some(group) = self
            .group_mru
            .iter()
            .copied()
            .find(|g| in_window.contains(g))
            .or_else(|| in_window.first().copied())
        else {
            return;
        };
        self.activate_group(group, cx);
        self.refocus_main = true;
        cx.notify();
    }

    /// Popout `id`'s window while it is open, for the control socket.
    pub fn popout_window(&self, id: u64, cx: &App) -> Option<gpui::AnyWindowHandle> {
        self.windows
            .iter()
            .find(|w| w.id == id)
            .filter(|w| w.is_open(cx))
            .and_then(|w| w.handle)
            .map(Into::into)
    }

    /// One popout's content: its groups through the main area's renderers, under the
    /// window buttons, with no ribbon, docks or status bar (see [[Specification#Observed popout windows]]).
    /// `None` once the node is gone, which tells the view to close. Nothing here may `notify`.
    pub(crate) fn render_popout(
        &mut self,
        id: u64,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let wix = self.windows.iter().position(|w| w.id == id)?;
        // The first frame draws while the window is still opening, before the opener
        // records its handle; the window itself is the handle.
        if self.windows[wix].handle.is_none() {
            self.windows[wix].handle = window.window_handle().downcast::<PopoutWindow>();
        }
        // Where the window is for `workspace.json`: the screen origin (always 0, 0 on
        // Wayland) and inner size. The inner size is the surface less `popout_margin`
        // on untiled edges, as the opener grew it.
        let outer = window.window_bounds().get_bounds();
        let viewport = window.viewport_size();
        let tiling = match window.window_decorations() {
            Decorations::Client { tiling } => tiling,
            Decorations::Server => Tiling::default(),
        };
        let margin = |tiled: bool| if tiled { px(0.) } else { popout_margin() };
        let whole = |p: Pixels| f32::from(p).round() as i32;
        let mut width = whole(viewport.width - margin(tiling.left) - margin(tiling.right));
        let mut height = whole(viewport.height - margin(tiling.top) - margin(tiling.bottom));
        // A compositor may briefly see the window with no inset and configure it grown
        // by the margin on every edge (sway does this). Exactly that grow, before
        // anything else changed the size, is put back once.
        let grow = whole(popout_margin() * 2.);
        match self.windows[wix].opened_for {
            Some((w, h)) if (w - width).abs() <= 1 && (h - height).abs() <= 1 => {
                (width, height) = (w, h);
            }
            Some((w, h))
                if grow > 0
                    && matches!(window.window_decorations(), Decorations::Client { .. })
                    && tiling == Tiling::default()
                    && (width - w - grow).abs() <= 1
                    && (height - h - grow).abs() <= 1 =>
            {
                let back = popout_outer_size(w as f32, h as f32);
                window.defer(cx, move |window, _| window.resize(back));
                (width, height) = (w, h);
            }
            Some(_) => self.windows[wix].opened_for = None,
            None => {}
        }
        self.windows[wix].set_bounds(
            WindowBounds {
                x: whole(outer.origin.x),
                y: whole(outer.origin.y),
                width,
                height,
            },
            window.is_maximized(),
        );
        // The title follows the window's own active tab, in the vault window's pattern.
        let title = self.popout_title(wix);
        if self.windows[wix].title != title {
            window.set_window_title(&title);
            self.windows[wix].title = title;
        }
        // The frame is this window's; the main window's render sets these back for itself.
        self.corners = WindowCorners::of(window);
        self.frameless = frameless(window);
        self.fullscreen = window.is_fullscreen();
        // The keyboard: only an active group of this window is this window's.
        if self.window_of_group(self.active_group_id()) == Some(id)
            && std::mem::take(&mut self.refocus_main)
            && self.modal == Modal::None
            && self.keyboard_is_parked(window, cx)
        {
            self.focus_main(window, cx);
        }
        self.tab_drag_frame(window, cx);
        // A modal or file menu raised from this window draws and holds the keyboard here.
        self.sync_menu_focus(Some(id), window, cx);
        let overlay = self.render_modal_overlay(Some(id));
        let file_menu = self.render_file_menu(Some(id), theme, window, cx);
        // So do notices that went up while it was active; it has no status bar to clear.
        let notices = self.render_notices(Some(id), px(16.), theme, cx);
        let preview = self.render_hover_preview(Some(id), theme, window, cx);
        let tree = self.windows[wix].tree.clone();
        self.rendering_window = Some(id);
        let body = self.render_layout(&tree, &mut Vec::new(), theme, cx);
        self.rendering_window = None;
        let corners = self.corners;
        // macOS has the desktop's traffic lights instead. The close button closes the
        // window's groups, then the window, from inside its own update.
        let controls = (self.frameless && !self.mac_frame()).then(|| {
            let this = cx.weak_entity();
            window_controls(theme, window, corners, move |window, cx| {
                this.update(cx, |ws, cx| ws.drop_popout(id, cx)).ok();
                window.remove_window();
            })
        });
        // The hotkeys stand down behind a modal, in every window.
        let hotkeys_live = self.modal == Modal::None;
        Some(
            div()
                .key_context("Workspace")
                .track_focus(&self.focus_handle)
                .on_modifiers_changed(cx.listener(
                    |this, e: &gpui::ModifiersChangedEvent, _, cx| {
                        this.preview_modifiers(e.modifiers, cx)
                    },
                ))
                // A tab dragged out of this window: the moves and release reach here
                // only, so this is where other windows' zones are found and dropped on.
                .on_drag_move(cx.listener(|this, e: &DragMoveEvent<TabDrag>, window, cx| {
                    this.cross_window_drag_moved(e.bounds, e.event.position, window, cx)
                }))
                .on_mouse_up_out(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.cross_window_drop(window, cx)),
                )
                .when(hotkeys_live, |d| {
                    d.on_action(cx.listener(Self::toggle_mode))
                        .on_action(cx.listener(Self::go_back))
                        .on_action(cx.listener(Self::go_forward))
                        .on_action(cx.listener(Self::open_search_action))
                        .on_action(cx.listener(Self::open_graph_action))
                        .on_action(cx.listener(Self::run_command_action))
                })
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .bg(theme.tokens.background_primary)
                .map(|d| corners.apply(d))
                .font_family(theme.ui_font.clone())
                .text_size(px(crate::theme::FONT_UI_MEDIUM))
                .text_color(theme.tokens.text_normal)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(body),
                )
                .children(controls)
                .children(overlay)
                .children(file_menu)
                .children(notices)
                .children(preview)
                .into_any_element(),
        )
    }

    /// A popout's title: `<leaf> - <vault> - TFT`.
    fn popout_title(&self, wix: usize) -> SharedString {
        let leaf = self.window_leaf_title(&self.windows[wix].tree);
        format!("{leaf} - {} - TFT", self.vault_name).into()
    }

    /// The leaf a window's title names: the active tab of its most recently active group.
    fn window_leaf_title(&self, tree: &Layout) -> SharedString {
        let mut order = Vec::new();
        tree.groups(&mut order);
        let group = self
            .group_mru
            .iter()
            .copied()
            .find(|g| order.contains(g))
            .or_else(|| order.first().copied());
        group
            .and_then(|g| self.group_ix(g))
            .and_then(|ix| {
                let view = self.group_view(ix);
                view.tabs.get(view.active_tab).map(|t| t.title())
            })
            .unwrap_or_else(|| leaf_title(None, ViewMode::Reading))
    }

    /// `workspace:move-to-new-window`: the active tab moves to a popout, which takes the
    /// focus. An emptied group closes, unless it is the main area's only one, which is
    /// left holding an empty tab (observed).
    fn move_to_new_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_group();
        let from_ix = self.active_group;
        let from_id = self.active_group_id();
        let at = self.groups[from_ix].active_tab;
        let Some(tab) = self.take_tab(from_ix, at, cx) else {
            return;
        };
        let emptied = self.groups[from_ix].tabs.is_empty();
        let main_only =
            emptied && self.window_of_group(from_id).is_none() && self.main_group_count() == 1;
        if main_only {
            // Its views still show the tab that left.
            let mode = self.default_mode();
            self.groups[from_ix].tabs.push(Tab::empty(mode));
            self.groups[from_ix].active_tab = 0;
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            self.stash_active_group();
        }
        if self
            .open_group_in_new_window(vec![tab.clone()], 0, window, cx)
            .is_none()
        {
            // No group, no move: the tab goes back where it was.
            if main_only {
                self.groups[from_ix].tabs.clear();
            }
            let group = &mut self.groups[from_ix];
            let at = at.min(group.tabs.len());
            group.tabs.insert(at, tab);
            group.active_tab = at;
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            return;
        }
        if emptied && !main_only {
            self.close_group(from_id, false, cx);
        }
    }

    /// `workspace:open-in-new-window`, and the header's "Open in new window": a copy of
    /// the active tab in a popout; the source tab stays (observed).
    fn open_in_new_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_group();
        let tab = self.copy_of_active_tab();
        self.open_group_in_new_window(vec![tab], 0, window, cx);
    }

    /// A row's "Open in new window": a fresh tab on `path` in a popout of its own.
    fn open_path_in_new_window(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let tab = self.fresh_tab(path);
        self.open_group_in_new_window(vec![tab], 0, window, cx);
    }

    /// `workspace:new-window`: a popout holding one empty tab (observed); returns its id.
    fn new_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        self.flush_save(cx);
        self.stash_active_group();
        let mode = self.default_mode();
        self.open_group_in_new_window(vec![Tab::empty(mode)], 0, window, cx)
    }

    /// `workspace:close-window`: closes the window the command ran in, not the active
    /// group's. A popout's groups close with it; the main window saves edits and
    /// layout first.
    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = window.window_handle();
        let popout = self
            .windows
            .iter()
            .find(|w| w.handle.map(gpui::AnyWindowHandle::from) == Some(here))
            .map(|w| w.id);
        match popout {
            Some(id) => self.drop_popout(id, cx),
            None => self.on_quit(cx),
        }
        window.remove_window();
    }

    /// `editor:open-link-in-new-window`: the link under the caret opens in a popout and
    /// the source stays. The link resolves against the source note before the window
    /// opens; a dangling link makes its note there.
    fn open_link_in_new_window(
        &mut self,
        target: LinkTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let LinkTarget::Internal(body) = target else {
            self.follow_link(target, cx);
            return;
        };
        let Some(state) = self.state.clone() else {
            return;
        };
        let source = self.current.clone().unwrap_or_default();
        let resolved = state.cache().resolve(&body, &source).map(str::to_string);
        if self.new_window(window, cx).is_none() {
            return;
        }
        let parsed = tft_markdown::parse_linktext(&body);
        match resolved {
            Some(path) => self.open_file(path, parsed.subpath, cx),
            None => {
                let name = parsed.path.to_string();
                self.create_note_for(&name, None, cx);
            }
        }
    }

    /// A new group holding `tabs`, in a popout of its own, made active. The window
    /// opens centered after this update, since its first frame reads the workspace
    /// being updated.
    fn open_group_in_new_window(
        &mut self,
        tabs: Vec<Tab>,
        active_tab: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let mode = tabs
            .get(active_tab)
            .map(|t| t.mode)
            .unwrap_or_else(|| self.default_mode());
        let (inner_w, inner_h) = crate::popout::INNER_SIZE;
        let window_id = self.next_group_id;
        self.next_group_id += 1;
        let id = self.new_group(mode, window, cx);
        let ix = self.group_ix(id)?;
        self.groups[ix].tabs = tabs;
        self.groups[ix].active_tab = active_tab;
        self.windows.push(WindowNode::new(
            window_id,
            Layout::split(SplitDirection::Vertical, vec![Layout::Group(id)]),
            WindowBounds {
                x: 0,
                y: 0,
                width: inner_w as i32,
                height: inner_h as i32,
            },
        ));
        self.load_group(ix, cx);
        self.load_tab(cx);
        self.refocus_main = true;
        self.show_popout(window_id, None, cx);
        cx.notify();
        Some(window_id)
    }

    /// Opens the gpui window for popout `window_id` after this update, at `recorded`
    /// or centered. A desktop that gives no window is reported and the groups close.
    fn show_popout(
        &mut self,
        window_id: u64,
        recorded: Option<(WindowBounds, bool)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == window_id) {
            let (width, height) = crate::popout::INNER_SIZE;
            w.opened_for = Some(
                recorded
                    .filter(|(_, maximized)| !maximized)
                    .map_or((width as i32, height as i32), |(b, _)| (b.width, b.height)),
            );
        }
        let this = cx.entity();
        cx.defer(move |cx| {
            let bounds = popout_placement(recorded, cx);
            let opened = PopoutWindow::open(this.clone(), window_id, bounds, cx);
            this.update(cx, |ws, cx| match opened {
                Ok(handle) => {
                    if let Some(w) = ws.windows.iter_mut().find(|w| w.id == window_id) {
                        w.handle = Some(handle);
                    }
                    cx.notify();
                }
                Err(err) => {
                    ws.report(format!("Could not open a window: {err}"), cx);
                    ws.drop_popout(window_id, cx);
                }
            });
        });
    }

    /// Reopens the popouts `workspace.json` recorded, the active group's last so it is
    /// in front, then raises the vault window if its own group is active.
    fn open_restored_popouts(&mut self, cx: &mut Context<Self>) {
        let active = self.window_of_group(self.active_group_id());
        let mut order: Vec<(u64, Option<(WindowBounds, bool)>)> = self
            .windows
            .iter()
            .filter(|w| w.handle.is_none())
            .map(|w| {
                let maximized = w
                    .node
                    .get("maximize")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                (w.id, w.bounds().map(|b| (b, maximized)))
            })
            .collect();
        if order.is_empty() {
            return;
        }
        order.sort_by_key(|(id, _)| Some(*id) == active);
        for (id, recorded) in order {
            self.show_popout(id, recorded, cx);
        }
        if active.is_none() {
            let main = self.main_window;
            cx.defer(move |cx| {
                main.update(cx, |_, window, _| window.activate_window())
                    .ok();
            });
        }
    }

    /// Closes popout `id`'s groups, taking its node with the last, and returns the
    /// keyboard. The caller closes the gpui window.
    pub(crate) fn drop_popout(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(w) = self.windows.iter().find(|w| w.id == id) else {
            return;
        };
        let mut groups = Vec::new();
        w.tree.groups(&mut groups);
        for g in groups {
            self.close_group(g, true, cx);
        }
        self.windows.retain(|w| w.id != id);
        self.refocus_main = true;
        cx.notify();
    }

    // ----- dragging tabs ---------------------------------------------------------

    /// A tab drag started: every group paints its drop zones from the next frame.
    /// Called through a weak handle, since the drag runs with the app.
    fn begin_tab_drag(&mut self, drag: TabDrag, cx: &mut Context<Self>) {
        self.dragging_tab = true;
        self.tab_drag = Some(drag);
        *self.drag_bounds.borrow_mut() = DragBounds::default();
        self.tab_drop = None;
        self.drop_bar = None;
        self.tab_bounds.clear();
        cx.notify();
    }

    /// The dragged tab moved over (or off) group `group`'s strip: over it,
    /// the drop bar goes to the tab boundary nearest the pointer, the left
    /// edge of a tab or the right edge of the last one, sliding there from
    /// wherever it was. Off it, the bar goes if it was this strip's.
    fn strip_drag_moved(
        &mut self,
        group: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !bounds.contains(&position) {
            if self.drop_bar.is_some_and(|b| b.group == group) {
                self.drop_bar = None;
                cx.notify();
            }
            return;
        }
        let mut tabs: Vec<(usize, gpui::Bounds<gpui::Pixels>)> = self
            .tab_bounds
            .iter()
            .filter(|((g, _), _)| *g == group)
            .map(|((_, ix), b)| (*ix, *b))
            .collect();
        tabs.sort_by_key(|(ix, _)| *ix);
        let Some((last_ix, last)) = tabs.last().copied() else {
            return;
        };
        let pointer = f32::from(position.x);
        let edges = tabs
            .iter()
            .map(|(ix, b)| (*ix, f32::from(b.origin.x)))
            .chain(std::iter::once((last_ix + 1, f32::from(last.right()))));
        let Some((at, x)) =
            edges.min_by(|a, b| (a.1 - pointer).abs().total_cmp(&(b.1 - pointer).abs()))
        else {
            return;
        };
        let now = Instant::now();
        let strip_x = f32::from(bounds.origin.x);
        let next = match self.drop_bar {
            Some(bar) if bar.group == group && bar.at == at => {
                if bar.strip_x == strip_x {
                    return;
                }
                DropBar { strip_x, ..bar }
            }
            // Within the strip the bar slides; arriving from elsewhere it appears where it is wanted.
            Some(bar) if bar.group == group => DropBar {
                group,
                at,
                x,
                from: bar.shown_x(now),
                since: now,
                strip_x,
            },
            _ => DropBar {
                group,
                at,
                x,
                from: x,
                since: now,
                strip_x,
            },
        };
        self.drop_bar = Some(next);
        cx.notify();
    }

    /// Each window's frame during a drag. A drag that ended away from a zone just
    /// stops; the zones go away in every window and the others redraw once. A sliding
    /// bar needs a frame per step until it lands.
    fn tab_drag_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dragging_tab && !cx.has_active_drag() {
            self.dragging_tab = false;
            self.tab_drag = None;
            self.tab_drop = None;
            self.drop_bar = None;
            self.tab_bounds.clear();
            let this = cx.entity();
            cx.defer(move |cx| this.update(cx, |_, cx| cx.notify()));
        }
        if self.drop_bar.is_some_and(|b| !b.settled(Instant::now())) {
            window.request_animation_frame();
        }
    }

    /// The window `window` is as the layout names it: a popout's id, or `None` for the vault.
    fn window_id_of(&self, window: &Window) -> Option<u64> {
        let here = window.window_handle();
        self.windows
            .iter()
            .find(|w| w.handle.map(gpui::AnyWindowHandle::from) == Some(here))
            .map(|w| w.id)
    }

    /// The dragged tab moved, heard by its source window. Off that window the pointer
    /// is mapped to the screen and each other window's groups light their zones and bar
    /// using their own measured bounds. Wayland never knows where its windows are, so
    /// the drag stays in its own window; overlapping windows go in layout order.
    fn cross_window_drag_moved(
        &mut self,
        root: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging_tab || cx.compositor_name() == "Wayland" {
            return;
        }
        let source = self.window_id_of(window);
        let screen = window.bounds().origin + position;
        let inside = root.contains(&position);
        let mut others: Vec<(Option<u64>, gpui::AnyWindowHandle)> = self
            .windows
            .iter()
            .filter(|w| Some(w.id) != source)
            .filter_map(|w| w.handle.map(|h| (Some(w.id), h.into())))
            .collect();
        if source.is_some() {
            others.insert(0, (None, self.main_window));
        }
        let placed: Vec<(Option<u64>, gpui::Bounds<Pixels>)> = others
            .into_iter()
            .filter_map(|(id, handle)| {
                handle
                    .update(cx, |_, w, _| w.bounds())
                    .ok()
                    .map(|b| (id, b))
            })
            .collect();
        let over = (!inside)
            .then(|| {
                placed
                    .iter()
                    .find(|(_, b)| b.contains(&screen))
                    .map(|(id, _)| *id)
            })
            .flatten();
        // Nowhere near any window, for the windows the pointer is not over.
        let away = gpui::point(px(-1e6), px(-1e6));
        let measured = self.drag_bounds.clone();
        let measured = measured.borrow();
        for (id, bounds) in &placed {
            let local = if over == Some(*id) {
                screen - bounds.origin
            } else {
                away
            };
            let mut groups = Vec::new();
            self.tree(*id).groups(&mut groups);
            for g in groups {
                if let Some(content) = measured.contents.get(&g) {
                    self.tab_drag_moved(g, *content, local, cx);
                }
                if let Some(strip) = measured.strips.get(&g) {
                    for (key, b) in measured.tabs.iter().filter(|((tg, _), _)| *tg == g) {
                        self.tab_bounds.insert(*key, *b);
                    }
                    self.strip_drag_moved(g, *strip, local, cx);
                }
            }
        }
    }

    /// The button came up outside the source window: a drop on another window's zone
    /// lands there as if dropped in it, and the active group follows the desktop's
    /// focus, as Obsidian's active leaf did. Over no window, the drag just ends.
    fn cross_window_drop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag.clone() else {
            return;
        };
        let source = self.window_id_of(window);
        let foreign = |g: u64| self.window_of_group(g) != source;
        if let Some(bar) = self.drop_bar.filter(|b| foreign(b.group)) {
            self.drop_tab_at_bar(&drag, bar.group, None, cx);
        } else if let Some((g, _)) = self.tab_drop.filter(|(g, _)| foreign(*g)) {
            self.drop_tab_on_group(&drag, g, window, cx);
        } else {
            return;
        }
        self.tab_drag = None;
        let source_open = source.is_none_or(|id| self.windows.iter().any(|w| w.id == id));
        if source_open {
            self.window_activated(source, cx);
        }
    }

    /// A tab dropped on group `to`'s strip lands where the bar says, or at `fallback`.
    fn drop_tab_at_bar(
        &mut self,
        drag: &TabDrag,
        to: u64,
        fallback: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let at = match self.drop_bar {
            Some(bar) if bar.group == to => Some(bar.at),
            _ => fallback,
        };
        self.drop_tab_on_strip(drag, to, at, cx);
    }

    /// A dragged tab over group `id`'s content lights the zone it would land in;
    /// leaving puts it out.
    fn tab_drag_moved(
        &mut self,
        id: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let next = if bounds.contains(&position) {
            Some((id, DropZone::at(bounds, position)))
        } else if self.tab_drop.is_some_and(|(g, _)| g == id) {
            None
        } else {
            return;
        };
        if self.tab_drop != next {
            self.tab_drop = next;
            cx.notify();
        }
    }

    /// A tab dropped on group `id`'s content: into the strip from the middle, a new
    /// group from a side.
    fn drop_tab_on_group(
        &mut self,
        drag: &TabDrag,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let zone = match self.tab_drop.take() {
            Some((g, zone)) if g == id => zone,
            _ => DropZone::Center,
        };
        self.dragging_tab = false;
        self.drop_bar = None;
        self.tab_bounds.clear();
        match zone.split() {
            None => self.move_tab(drag, id, None, cx),
            Some((direction, after)) => self.split_with_tab(drag, id, direction, after, window, cx),
        }
    }

    /// A tab dropped on group `to`'s strip at `at` joins it there and becomes active.
    fn drop_tab_on_strip(
        &mut self,
        drag: &TabDrag,
        to: u64,
        at: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.tab_drop = None;
        self.dragging_tab = false;
        self.drop_bar = None;
        self.tab_bounds.clear();
        self.move_tab(drag, to, at, cx);
    }

    /// Takes tab `ix` out of group `from_ix` with every group stashed; says whether
    /// the group has nothing left.
    fn take_tab(&mut self, from_ix: usize, ix: usize, cx: &mut Context<Self>) -> Option<Tab> {
        let group = &mut self.groups[from_ix];
        if ix >= group.tabs.len() {
            return None;
        }
        let tab = group.tabs.remove(ix);
        if group.tabs.is_empty() {
            return Some(tab);
        }
        if ix < group.active_tab {
            group.active_tab -= 1;
        } else if ix == group.active_tab {
            group.active_tab = ix.min(group.tabs.len() - 1);
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            self.stash_active_group();
        }
        Some(tab)
    }

    /// Group `ix`, emptied by a drag, when it is the main area's only group: it keeps
    /// an empty tab, as `move-to-new-window` leaves it (observed). Every group must be
    /// stashed; says whether it stays.
    fn keep_last_main_group(&mut self, ix: usize, cx: &mut Context<Self>) -> bool {
        let id = self.groups[ix].id;
        if self.window_of_group(id).is_some() || self.main_group_count() > 1 {
            return false;
        }
        let empty = Tab::empty(self.default_mode());
        self.groups[ix].tabs.push(empty);
        self.groups[ix].active_tab = 0;
        self.load_group(ix, cx);
        self.load_tab(cx);
        self.stash_active_group();
        true
    }

    /// Moves the dragged tab to group `to`, before its tab `at` or at the
    /// end, and makes it the active tab of the active group. In its own
    /// group that is a reorder. A group left with nothing closes, as one
    /// whose last tab closed does.
    fn move_tab(&mut self, drag: &TabDrag, to: u64, at: Option<usize>, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_group();
        let (Some(from_ix), Some(to_ix)) = (self.group_ix(drag.group), self.group_ix(to)) else {
            return;
        };
        let mut at = at.unwrap_or(usize::MAX);
        let mut empty = false;
        let tab = if from_ix == to_ix {
            // A reorder: the strip's views keep showing what they show
            // until the end, when the dragged tab takes over if it must.
            let group = &mut self.groups[to_ix];
            if drag.tab >= group.tabs.len() {
                return;
            }
            if drag.tab < at {
                at -= 1;
            }
            group.tabs.remove(drag.tab)
        } else {
            let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
                return;
            };
            empty = self.groups[from_ix].tabs.is_empty() && !self.keep_last_main_group(from_ix, cx);
            tab
        };
        // What the target's views show now: the tab that was active there, if any.
        let target = &mut self.groups[to_ix];
        let shown = (from_ix != to_ix || drag.tab != target.active_tab)
            .then(|| target.tabs.get(target.active_tab).cloned())
            .flatten();
        let at = at.min(target.tabs.len());
        target.tabs.insert(at, tab);
        target.active_tab = at;
        self.load_group(to_ix, cx);
        if shown.is_none() || self.tabs.get(self.active_tab) != shown.as_ref() {
            self.load_tab(cx);
        }
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// Puts the dragged tab in a new group beside `beside`, after or before along
    /// `direction`, and makes it active. A group's only tab dropped on its own edge
    /// does nothing.
    fn split_with_tab(
        &mut self,
        drag: &TabDrag,
        beside: u64,
        direction: SplitDirection,
        after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let Some(from_ix) = self.group_ix(drag.group) else {
            return;
        };
        if drag.group == beside && self.groups[from_ix].tabs.len() <= 1 {
            return;
        }
        if self.group_ix(beside).is_none() {
            return;
        }
        let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
            return;
        };
        let empty = self.groups[from_ix].tabs.is_empty() && !self.keep_last_main_group(from_ix, cx);
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(beside, id, direction, after);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.load_group(ix, cx);
        self.load_tab(cx);
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// `workspace:close-tab-group`, and closing a group's last tab: the group goes,
    /// its siblings take the room, and focus goes to the most recently used. The last
    /// group is left with an empty tab.
    fn close_group(&mut self, id: u64, remember: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        let in_window = self.windows.iter().position(|w| w.tree.contains(id));
        if in_window.is_none() && self.main_group_count() == 1 {
            self.flush_save(cx);
            self.stash_active_tab();
            let tabs = std::mem::take(&mut self.tabs);
            if remember {
                for (i, tab) in tabs.into_iter().enumerate() {
                    self.remember_closed(i, tab);
                }
            }
            self.tabs.push(Tab::empty(self.default_mode()));
            self.active_tab = 0;
            self.load_tab(cx);
            return;
        }
        let was_active = ix == self.active_group;
        let active_id = self.active_group_id();
        // Nothing paints this group's header again, so its title's shaped line goes with it.
        self.title_layout.remove(&id);
        if was_active {
            self.flush_save(cx);
            self.stash_active_group();
        }
        if remember {
            let tabs = self.groups[ix].tabs.clone();
            for (i, tab) in tabs.into_iter().enumerate() {
                self.remember_closed(i, tab);
            }
        }
        let window = in_window.map(|wix| self.windows[wix].id);
        let mut parent = self.tree(window).path_to(id).unwrap_or_default();
        parent.pop();
        self.tree_mut(window).remove(id);
        self.groups.remove(ix);
        self.group_mru.retain(|g| *g != id);
        let mut left = Vec::new();
        self.tree(window).groups(&mut left);
        if let (Some(wix), true) = (in_window, left.is_empty()) {
            // A popout's last group takes the window with it, as Obsidian's does. Its
            // gpui window is removed here unless this runs inside it.
            let node = self.windows.remove(wix);
            if let Some(handle) = node.handle {
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            }
        } else {
            // The split the group left, or the nearest one still standing.
            loop {
                if matches!(
                    self.tree(window).node_at(&parent),
                    Some(Layout::Split { .. })
                ) {
                    self.renormalise_dimensions(window, &parent);
                    break;
                }
                if parent.pop().is_none() {
                    break;
                }
            }
        }
        if was_active {
            let next = self
                .group_mru
                .first()
                .copied()
                .and_then(|g| self.group_ix(g))
                .unwrap_or(0);
            self.load_group(next, cx);
        } else {
            self.active_group = self.group_ix(active_id).unwrap_or(0);
            cx.notify();
        }
    }

    /// How many groups the main area holds (`groups` also counts popouts').
    fn main_group_count(&self) -> usize {
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.len()
    }

    /// The `dimension` of a layout node: a group's on the group, a split's on the split.
    fn dimension_of(&self, node: &Layout) -> Option<f64> {
        match node {
            Layout::Group(id) => self.groups[self.group_ix(*id)?].dimension(),
            Layout::Split { node, .. } => node.get("dimension").and_then(Value::as_f64),
        }
    }

    /// Sets the `dimension` of the node at `path` of a window's tree (main area for `None`).
    fn set_dimension_at(&mut self, window: Option<u64>, path: &[usize], dimension: Option<f64>) {
        match self.tree_mut(window).node_at_mut(path) {
            Some(Layout::Group(id)) => {
                let id = *id;
                if let Some(ix) = self.group_ix(id) {
                    self.groups[ix].set_dimension(dimension);
                }
            }
            Some(Layout::Split { node, .. }) => set_dimension(node, dimension),
            None => {}
        }
    }

    /// After a child left the split at `path`, the rest share its room: scaled to a
    /// hundred, or dropped when one is left, as Obsidian wrote.
    fn renormalise_dimensions(&mut self, window: Option<u64>, path: &[usize]) {
        let Some(Layout::Split { children, .. }) = self.tree(window).node_at(path) else {
            return;
        };
        let dims: Vec<Option<f64>> = children.iter().map(|c| self.dimension_of(c)).collect();
        let n = dims.len();
        let scaled: Vec<Option<f64>> = if n <= 1 {
            vec![None; n]
        } else if dims.iter().all(Option::is_some) {
            let sum: f64 = dims.iter().flatten().sum();
            if sum <= 0.0 {
                return;
            }
            dims.iter().map(|d| d.map(|d| d * 100.0 / sum)).collect()
        } else {
            return;
        };
        for (ix, d) in scaled.into_iter().enumerate() {
            let mut child = path.to_vec();
            child.push(ix);
            self.set_dimension_at(window, &child, d);
        }
    }

    /// A divider dragged to `position`: the pair's room is redistributed and every
    /// child's share written as a `dimension`, as Obsidian does.
    fn drag_divider(
        &mut self,
        drag: &DividerDrag,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(Layout::Split {
            direction,
            children,
            ..
        }) = self.tree(drag.window).node_at(&drag.path)
        else {
            return;
        };
        let n = children.len();
        if drag.before + 1 >= n {
            return;
        }
        let row = *direction == SplitDirection::Vertical;
        let (origin, size) = if row {
            (bounds.origin.x, bounds.size.width)
        } else {
            (bounds.origin.y, bounds.size.height)
        };
        let along = if row { position.x } else { position.y };
        let size = f32::from(size);
        if size <= 0.0 {
            return;
        }
        let at = (f32::from(along - origin) / size).clamp(0.0, 1.0) as f64 * 100.0;
        let mut dims: Vec<f64> = match children
            .iter()
            .map(|c| self.dimension_of(c))
            .collect::<Option<Vec<f64>>>()
        {
            Some(dims) => dims,
            None => vec![100.0 / n as f64; n],
        };
        let before: f64 = dims[..drag.before].iter().sum();
        let pair = dims[drag.before] + dims[drag.before + 1];
        const MIN: f64 = 5.0;
        if pair <= 2.0 * MIN {
            return;
        }
        let a = (at - before).clamp(MIN, pair - MIN);
        dims[drag.before] = a;
        dims[drag.before + 1] = pair - a;
        for (ix, d) in dims.into_iter().enumerate() {
            let mut child = drag.path.clone();
            child.push(ix);
            self.set_dimension_at(drag.window, &child, Some(d));
        }
        cx.notify();
    }

    /// `editor:focus-left` and siblings: the keyboard moves to the group on that side,
    /// the most recently used one.
    fn focus_neighbor(&mut self, direction: SplitDirection, forward: bool, cx: &mut Context<Self>) {
        // Within the active group's own window: a popout's groups are neighbours of
        // each other, not of the main area's.
        let active = self.active_group_id();
        let candidates = self
            .tree(self.window_of_group(active))
            .neighbors(active, direction, forward);
        let target = self
            .group_mru
            .iter()
            .find(|g| candidates.contains(g))
            .copied()
            .or_else(|| candidates.first().copied());
        if let Some(target) = target {
            self.activate_group(target, cx);
        }
    }

    /// Every tab but the live one: the active group's other tabs and all of
    /// the other groups'. What the live one needs is done on the workspace's
    /// own fields.
    fn stashed_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        let active_tab = self.active_tab;
        let active_group = self.active_group;
        let own = self
            .tabs
            .iter_mut()
            .enumerate()
            .filter(move |(ix, _)| *ix != active_tab)
            .map(|(_, t)| t);
        let others = self
            .groups
            .iter_mut()
            .enumerate()
            .filter(move |(gx, _)| *gx != active_group)
            .flat_map(|(_, g)| g.tabs.iter_mut());
        own.chain(others)
    }

    /// The group whose area the one graph view paints in: the active one while it shows
    /// the graph, else the first that does.
    fn graph_group(&self) -> Option<u64> {
        if self.mode == ViewMode::Graph {
            return Some(self.active_group_id());
        }
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.into_iter().find(|id| {
            self.group_ix(*id).is_some_and(|ix| {
                let g = &self.groups[ix];
                g.tabs
                    .get(g.active_tab)
                    .is_some_and(|t| t.mode == ViewMode::Graph)
            })
        })
    }

    /// A sidebar's collapse toggle; it stays on screen when the sidebar is collapsed.
    fn sidebar_toggle(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let command = "app:toggle-left-sidebar";
        crate::ui::icon_button("toggle-left", "panel-left", theme)
            .on_click(cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)))
    }

    pub(crate) fn show_view_header(&self) -> bool {
        true
    }

    /// Whether the window has macOS's hidden frame: system traffic lights, no title or
    /// TFT buttons. Obsidian keeps the sidebar toggles in the headers there.
    fn mac_frame(&self) -> bool {
        cfg!(target_os = "macos") && self.frameless
    }

    /// What the top-left header is padded by past its 8 px: `--frame-left-space`, room
    /// for the traffic lights on macOS (nothing elsewhere or in fullscreen).
    fn frame_left_space(&self) -> f32 {
        if !self.mac_frame() || self.fullscreen {
            return 0.;
        }
        MAC_FRAME_LEFT_SPACE
    }

    /// A sidebar's resize handle: the 3 px strip on its inner edge, accent-coloured on hover.
    fn dock_handle(&self, left: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(if left {
                "left-dock-handle"
            } else {
                "right-dock-handle"
            })
            .absolute()
            .top_0()
            .h_full()
            .w(px(3.))
            .when(left, |d| d.right(px(0.)).border_r_1())
            .when(!left, |d| d.left(px(0.)).border_l_1())
            .border_color(theme.tokens.background_modifier_border)
            .cursor_col_resize()
            .occlude()
            .hover(|s| {
                s.bg(theme.tokens.interactive_accent)
                    .border_color(theme.tokens.interactive_accent)
            })
            .on_drag(DockDrag { left }, |_, _, _, cx| cx.new(|_| NoDragPreview))
    }

    /// A sidebar handle dragged: the dock follows the pointer between a header row's
    /// width and what leaves the main area room.
    fn drag_dock(
        &mut self,
        left: bool,
        bounds: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let x = f64::from(f32::from(position.x));
        let row = f64::from(f32::from(bounds.size.width));
        let max = (row - 2. * MIN_SIDEBAR_WIDTH).max(MIN_SIDEBAR_WIDTH);
        if left {
            let origin = f64::from(f32::from(bounds.origin.x));
            self.left_width = (x - origin).clamp(MIN_SIDEBAR_WIDTH, max);
        }
        cx.notify();
    }

    /// The minimise, maximise and close buttons in the window's right corner, drawn
    /// only when the frame is TFT's. Windows has them named to the hit test instead of
    /// clicked, so the system performs them and snap layouts work; `occlude` keeps the
    /// press from dragging the window.
    fn render_window_controls(&self, theme: &Theme, window: &Window) -> gpui::Div {
        window_controls(theme, window, self.corners, |w, _| w.remove_window())
    }

    /// The left sidebar's footer, Obsidian's vault profile: the vault's name behind a
    /// switcher, and Settings, above a hairline. It collapses with the sidebar.
    fn render_vault_profile(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let menu = self.render_vault_menu(theme, window, cx);
        let button = |id: &'static str, name: &'static str, command: &'static str| {
            crate::ui::icon_button(id, name, theme).on_click(
                cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)),
            )
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .px(px(8.))
            .gap(px(2.))
            // `.workspace-sidedock-vault-profile`: secondary background over a divider
            // hairline; the name is `--vault-profile-color` at weight 500.
            .border_t_1()
            .border_color(theme.tokens.divider_color)
            .child(
                div()
                    .id("vault-switcher")
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w_0()
                    .flex_1()
                    .h(px(24.))
                    .pr(px(6.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                    .when(self.vault_menu_open, |d| {
                        d.bg(theme.tokens.background_modifier_hover)
                    })
                    .on_hover(cx.listener(|this, over: &bool, _, _| {
                        this.vault_switcher_hovered = *over;
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_vault_menu(cx)))
                    // The menu's anchor is this box: an absolute element left to the
                    // row's centring would hang from its middle.
                    .child(div().absolute().top_0().left_0().size_0().children(menu))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(28.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon("chevrons-up-down", ICON_SIZE, theme.tokens.icon_color)
                                    .opacity(theme.tokens.icon_opacity),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(FONT_UI_SMALL))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.tokens.vault_profile_color)
                            .child(self.vault_name.clone()),
                    ),
            )
            .child(button("vault-settings", "settings", "app:open-settings"))
    }

    fn render_vault_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.vault_menu_open {
            return None;
        }
        let current = self.state.as_ref().map(|s| s.root.clone());
        let registry = VaultRegistry::open().ok().flatten();
        let known: Vec<(String, PathBuf)> = registry
            .as_ref()
            .map(|r| {
                r.vaults()
                    .into_iter()
                    .filter(|(_, e)| e.path.is_dir())
                    .map(|(_, e)| (e.name(), e.path.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let item = |id: gpui::ElementId, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(5.))
                .rounded(px(4.))
                .mx_1()
                .cursor_pointer()
                .hover(|s| s.bg(theme.tokens.background_modifier_hover))
        };
        let check = |on: bool, theme: &Theme| {
            div()
                .w(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .children(on.then(|| icon("check", 14., theme.tokens.text_muted)))
        };
        // A machine with dozens of vaults has a menu taller than the window, so cap it
        // at the window and scroll, as Obsidian's `.menu-scroll` does.
        let tallest = (window.viewport_size().height - px(96.)).max(px(160.));
        let mut list = div()
            .id("vault-menu")
            .flex()
            .flex_col()
            .min_w(px(240.))
            .max_w(px(360.))
            .max_h(tallest)
            .overflow_y_scroll()
            .py_1()
            .bg(theme.tokens.background_secondary)
            .border_1()
            .border_color(theme.tokens.background_modifier_border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.tokens.text_normal)
            .occlude()
            // A press on the switcher row is a press outside the menu, and the click
            // that ends it toggles; the row's own handler closes when the press is its.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.vault_switcher_hovered {
                    return;
                }
                this.vault_menu_open = false;
                cx.notify();
            }));
        if known.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py(px(5.))
                    .text_color(theme.tokens.text_faint)
                    .child("No vaults opened yet"),
            );
        }
        for (ix, (name, path)) in known.into_iter().enumerate() {
            let showing = current.as_deref() == Some(path.as_path());
            let tooltip = crate::ui::tooltip(path.display().to_string());
            list = list.child(
                item(("vault-menu-item", ix).into(), theme)
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.vault_menu_open = false;
                        if showing {
                            cx.notify();
                        } else if path.is_dir() {
                            this.load_vault(path.clone(), window, cx);
                        } else {
                            this.report(format!("{} is not a folder any more", path.display()), cx);
                        }
                    }))
                    .child(check(showing, theme))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name),
                    ),
            );
        }
        // Obsidian's tail is a single row under the separator, "Manage
        // vaults...", which raises the vault manager (probe of
        // 2026-10-29). TFT's own "Open vault..." and "Open sandbox
        // vault" moved into that window, where Obsidian keeps the same
        // two; they are still commands, and still in the palette.
        list = list
            .child(
                div()
                    .my_1()
                    .h(px(1.))
                    .bg(theme.tokens.background_modifier_border),
            )
            .child(
                item("vault-menu-manage".into(), theme)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_command("app:open-vault", window, cx)
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .child(icon("folder-open", 14., theme.tokens.text_muted)),
                    )
                    .child("Manage vaults..."),
            );
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(point(px(0.), px(-6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// A group's tab strip. `first`/`last` mark the top-left/top-right corners, whose
    /// strips carry the collapsed-sidebar toggles and clear the window controls.
    fn render_tab_strip(
        &self,
        view: &GroupView,
        first: bool,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let tabs = &view.tabs;
        let active_ix = view.active_tab;
        let group = view.id;
        let group_active = view.active;
        let frameless = self.frameless;
        let mac = self.mac_frame();
        // A popout has no sidebars or ribbon: its corner strips draw no toggle (observed).
        let popout = self.rendering_window.is_some();
        let left_open = !popout && self.left_visible;
        // The window buttons overlay the right end of the last group's header; macOS has none.
        let under_controls = frameless && !mac && last;
        // The top-left strip keeps clear of the macOS traffic lights when the left sidebar is collapsed.
        let left_space = if first && !left_open {
            self.frame_left_space()
        } else {
            0.
        };
        let at_left = first && !left_open;
        let at_right = last;
        div()
            .flex()
            .flex_row()
            .items_end()
            .when(at_left && self.corners.tl, |d| d.rounded_tl(WINDOW_RADIUS))
            .when(at_right && self.corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
            .h(px(HEADER_HEIGHT))
            .pt(px(6.))
            .pl(px(8. + left_space))
            .pr(if under_controls {
                px(3. * RIBBON_WIDTH)
            } else {
                px(8.)
            })
            .bg(theme.tokens.tab_container_background)
            .relative()
            .overflow_hidden()
            // A dragged tab over the strip moves the drop bar to the nearest boundary;
            // dropped, it lands there.
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.strip_drag_moved(group, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.drop_tab_at_bar(drag, group, None, cx)
            }))
            .when(self.dragging_tab, |d| {
                d.child(measure_for_drag(&self.drag_bounds, DragPart::Strip(group)))
            })
            // The strip's outline is a bottom hairline, so the active tab paints over it
            // and joins the header without a seam; other tabs stop a pixel short.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(theme.tokens.tab_outline_color),
            )
            // Without the ribbon a collapsed left sidebar would have no way back;
            // macOS keeps the toggle here, after the traffic lights.
            .when(first && !popout && !left_open, |d| {
                d.child(self.sidebar_toggle(theme, cx).mb(px(8.)).mr(px(2.)))
            })
            // The tabs scroll sideways in their own box, so a crowded strip keeps every
            // tab reachable.
            .child(
                div()
                    .id(("tab-strip-scroll", group))
                    .flex()
                    .flex_row()
                    .items_end()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .overflow_x_scroll()
                    .children(tabs.iter().enumerate().map(|(ix, tab)| {
                        let active = ix == active_ix;
                        let pinned = tab.pinned;
                        let divider = !active && ix + 1 != active_ix;
                        let title = tab.title();
                        let drag = TabDrag {
                            group,
                            tab: ix,
                            title: title.clone(),
                        };
                        let this = cx.weak_entity();
                        // A tab showing a file previews it, anchored on the whole tab.
                        let preview = {
                            let this = this.clone();
                            let path = tab.path.clone();
                            let window = self.window_of_group(group);
                            move |over, anchor, modifiers, _: &mut Window, cx: &mut App| {
                                let Some(path) = path.clone() else {
                                    return;
                                };
                                this.update(cx, |this, cx| {
                                    this.row_hover(
                                        crate::hover_preview::Source::TabHeader,
                                        window,
                                        crate::hover_preview::exact(&path),
                                        path,
                                        over,
                                        anchor,
                                        modifiers,
                                        cx,
                                    )
                                })
                                .ok();
                            }
                        };
                        // Obsidian's tab: 34 px tall, 200 px wide, active in the content
                        // colour, its top corners as round as its feet, close button a
                        // 20 px disc. The inner box, not the tab, takes the hover wash.
                        crate::hover_preview::hover_row(
                            crate::ui::workspace_tab(("tab", ix), active, group_active, theme),
                            preview,
                        )
                        // The divider on an inactive tab's right edge, as Obsidian's
                        // `::after`: a 1 px line astride the edge, parting the strip from
                        // the new-tab button after the last tab. Deferred so the next tab's
                        // opaque background cannot clip its half.
                        .when(divider, |d| {
                            d.child(deferred(
                                div()
                                    .absolute()
                                    .right(px(-0.5))
                                    .top(px(8.))
                                    .w(px(1.))
                                    .h(px(15.))
                                    .bg(theme.tokens.tab_divider_color),
                            ))
                        })
                        // The active tab's feet: concave curves carrying its bottom
                        // corners into the header, painted late so a hover wash stays underneath.
                        .when(active, |d| {
                            d.child(deferred(tab_foot(true, theme)))
                                .child(deferred(tab_foot(false, theme)))
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.activate_group(group, cx);
                            this.activate_tab(ix, cx)
                        }))
                        // Picking the tab up: the ghost is its title, and the workspace
                        // shows every group's zones.
                        .on_drag(drag, move |drag, grab, window, cx| {
                            let picked = drag.clone();
                            this.update(cx, |w, cx| w.begin_tab_drag(picked, cx)).ok();
                            let title = drag.title.clone();
                            let source = window.window_handle();
                            cx.new(|_| TabDragPreview {
                                title,
                                grab,
                                source,
                            })
                        })
                        // Where the tab is, for the strip's drop bar; a dropped tab lands
                        // there, else in front of it.
                        .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, _| {
                            if this.dragging_tab {
                                this.tab_bounds.insert((group, ix), e.bounds);
                            }
                        }))
                        .when(self.dragging_tab, |d| {
                            d.child(measure_for_drag(
                                &self.drag_bounds,
                                DragPart::Tab(group, ix),
                            ))
                        })
                        .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                            this.drop_tab_at_bar(drag, group, Some(ix), cx)
                        }))
                        .on_mouse_down(
                            gpui::MouseButton::Middle,
                            cx.listener(move |this, _, _, cx| {
                                this.activate_group(group, cx);
                                this.close_tab(ix, cx)
                            }),
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_tab_menu(group, ix, e.position, cx)
                            }),
                        )
                        // The inner box: title and close button, padded `0 3px 0 6px`,
                        // washed on hover when the tab is not active.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(2.))
                                .pl(px(6.))
                                .pr(px(3.))
                                .rounded(px(4.))
                                .when(!active, |d| {
                                    d.group_hover("tab", |s| {
                                        s.bg(theme.tokens.tab_inner_background_hover)
                                    })
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(title),
                                )
                                // The close button: no wash of its own, its cross
                                // brightens from muted to normal under the pointer.
                                .child(
                                    div()
                                        .id(("tab-close", ix))
                                        .group("tab-close")
                                        .flex_shrink_0()
                                        .w(px(20.))
                                        .h(px(20.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(8.))
                                        // A pinned tab shows its pin in the close button's
                                        // place; a click unpins.
                                        .when(!active && !pinned, |d| {
                                            d.invisible().group_hover("tab", |s| s.visible())
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.activate_group(group, cx);
                                            if pinned {
                                                this.activate_tab(ix, cx);
                                                this.toggle_pin(cx);
                                            } else {
                                                this.close_tab(ix, cx);
                                            }
                                        }))
                                        .child(
                                            icon(
                                                if pinned { "pin" } else { "x" },
                                                14.,
                                                theme.tokens.tab_text_color_focused_active,
                                            )
                                            .group_hover("tab-close", |s| {
                                                s.text_color(theme.tokens.text_normal)
                                            }),
                                        ),
                                ),
                        )
                    }))
                    .child(
                        div()
                            .id("new-tab")
                            .flex_shrink_0()
                            .ml(px(8.))
                            .mr(px(4.))
                            .mb(px(7.))
                            .w(px(24.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .group("new-tab")
                            .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_group(group, cx);
                                this.new_tab(cx)
                            }))
                            // A `clickable-icon`: `--icon-color`, not the text colour.
                            .child(
                                icon("plus", ICON_SIZE, theme.tokens.icon_color)
                                    .opacity(theme.tokens.icon_opacity)
                                    .group_hover("new-tab", |s| {
                                        s.opacity(theme.tokens.icon_opacity_hover)
                                            .text_color(theme.tokens.icon_color_hover)
                                    }),
                            ),
                    )
                    .child(window_handle(
                        div().id(("tab-strip-space", group)).flex_1().h_full(),
                        frameless,
                    )),
            )
            // The drop bar: an accent bar astride the boundary the dragged tab would land
            // at, deferred past the late-painted dividers and feet.
            .when_some(self.drop_bar.filter(|b| b.group == group), |d, bar| {
                let left = bar.shown_x(Instant::now()) - bar.strip_x - DROP_BAR_WIDTH / 2.;
                d.child(deferred(
                    div()
                        .absolute()
                        .left(px(left))
                        .bottom(px(6.))
                        .w(px(DROP_BAR_WIDTH))
                        .h(px(25.))
                        .rounded(px(DROP_BAR_WIDTH / 2.))
                        .bg(theme.tokens.interactive_accent),
                ))
            })
    }

    // ----- workspace.json --------------------------------------------------------

    /// Whether a `workspace.json` was read at load; the startup flags then leave the
    /// first note alone, as Obsidian shows the empty state.
    pub fn layout_restored(&self) -> bool {
        self.layout_restored
    }

    /// Restores the layout `workspace.json` records: the main area's tab
    /// groups and how they are split, the popout windows and theirs, the
    /// active leaf (in either), each sidebar's collapse state and its tab.
    fn restore_layout(
        &mut self,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = match state.vault().workspace_json() {
            Ok(Some(layout)) => layout,
            Ok(None) => return,
            Err(err) => {
                self.report(format!("workspace.json could not be read: {err}"), cx);
                return;
            }
        };
        self.apply_layout(layout, state, window, cx);
    }

    /// Restores `layout`, a `workspace.json`, over a main area of one empty group.
    fn apply_layout(
        &mut self,
        layout: Value,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layout_restored = true;
        if let Some((sort, auto_reveal)) = explorer_leaf_state(&layout) {
            self.explorer
                .update(cx, |e, cx| e.set_leaf_state(sort, auto_reveal, cx));
        }
        self.restore_pane_states(&layout, cx);
        self.last_open_files = layout
            .get("lastOpenFiles")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let default_mode = self.default_mode();
        // A leaf naming a file the vault no longer has is dropped, as Obsidian drops one it cannot open.
        let known = |file: &str| state.vault().absolute(file).is_file();
        let spec = layout
            .get("main")
            .and_then(|main| read_main(main, default_mode, &known));
        let windows = layout
            .get("floating")
            .map(|floating| read_floating(floating, default_mode, &known))
            .unwrap_or_default();
        let main_restored = spec.is_some();
        let mut leaf_ids: Vec<(String, u64, usize)> = Vec::new();
        if let Some(spec) = spec {
            // The groups the file describes replace the window's placeholder, whose views were empty.
            let placeholder = self.active_group_id();
            let tree = self.materialize(spec, &mut leaf_ids, window, cx);
            self.layout_tree = tree;
            if let Some(ix) = self.group_ix(placeholder) {
                self.groups.remove(ix);
                self.group_mru.retain(|g| *g != placeholder);
            }
        }
        // The popouts' groups; each window's gpui window opens once this vault window is up.
        let main_leaves = leaf_ids.len();
        for spec in windows {
            let tree = self.materialize(spec.tree, &mut leaf_ids, window, cx);
            let id = self.next_group_id;
            self.next_group_id += 1;
            self.windows
                .push(WindowNode::from_node(id, tree, spec.node));
        }
        if main_restored || !self.windows.is_empty() {
            // `active` may name a popout's leaf; otherwise the main area's first leaf with a file.
            let active_id = layout.get("active").and_then(Value::as_str);
            let main_leaf_ids = &leaf_ids[..main_leaves];
            let (group, tab) = leaf_ids
                .iter()
                .find(|(id, _, _)| Some(id.as_str()) == active_id)
                .map(|(_, g, t)| (*g, Some(*t)))
                .or_else(|| {
                    main_leaf_ids
                        .iter()
                        .find(|(_, g, t)| {
                            self.group_ix(*g)
                                .is_some_and(|ix| self.groups[ix].tabs[*t].path.is_some())
                        })
                        .map(|(_, g, t)| (*g, Some(*t)))
                })
                .or_else(|| main_leaf_ids.first().map(|(_, g, _)| (*g, None)))
                .unwrap_or((self.groups[0].id, None));
            if let (Some(ix), Some(tab)) = (self.group_ix(group), tab) {
                self.groups[ix].active_tab = tab;
            }
            // Every group's views filled, the active one last so its note is what the
            // explorer and graph follow.
            let mut order = Vec::new();
            self.layout_tree.groups(&mut order);
            for w in &self.windows {
                w.tree.groups(&mut order);
            }
            for &id in order.iter().filter(|id| **id != group).chain([&group]) {
                if let Some(ix) = self.group_ix(id) {
                    self.active_group = ix;
                    self.load_group(ix, cx);
                    self.load_tab(cx);
                    self.stash_active_group();
                }
            }
            // The most recently used order Obsidian would have: the active group first.
            self.group_mru = std::iter::once(group)
                .chain(order.into_iter().filter(|g| *g != group))
                .collect();
            self.refocus_main = true;
            self.open_restored_popouts(cx);
        }
        let collapsed = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("collapsed"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        self.left_visible = !collapsed("left");
        // A dock's `width` is where its handle was last dragged; a fresh vault has 300,
        // and nothing below the minimum is believed.
        let width = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("width"))
                .and_then(Value::as_f64)
                .filter(|w| *w >= MIN_SIDEBAR_WIDTH)
        };
        if let Some(w) = width("left") {
            self.left_width = w;
        }
        match dock_current_view(&layout, "left").as_deref() {
            Some("search") => self.left_tab = LeftTab::Search,
            Some(_) => self.left_tab = LeftTab::Files,
            None => {}
        }
        self.layout = Some(layout);
    }

    /// Makes the groups a `main` node describes, tabs in place. `leaf_ids` collects
    /// each leaf's id with the group and tab it became.
    fn materialize(
        &mut self,
        spec: LayoutSpec,
        leaf_ids: &mut Vec<(String, u64, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Layout {
        match spec {
            LayoutSpec::Group(group) => {
                let mode = group
                    .tabs
                    .first()
                    .map(|(_, t)| t.mode)
                    .unwrap_or(ViewMode::Reading);
                let id = self.new_group(mode, window, cx);
                let ix = self.group_ix(id).expect("the group was just made");
                for (tab_ix, (leaf, _)) in group.tabs.iter().enumerate() {
                    leaf_ids.push((leaf.clone(), id, tab_ix));
                }
                let g = &mut self.groups[ix];
                g.tabs = group.tabs.into_iter().map(|(_, t)| t).collect();
                g.active_tab = group.active_tab.min(g.tabs.len().saturating_sub(1));
                g.node = group.node;
                Layout::Group(id)
            }
            LayoutSpec::Split {
                direction,
                children,
                node,
            } => Layout::Split {
                direction,
                children: children
                    .into_iter()
                    .map(|c| self.materialize(c, leaf_ids, window, cx))
                    .collect(),
                node,
            },
        }
    }

    /// The Search leaf's own state from `workspace.json`'s left dock.
    fn restore_pane_states(&mut self, layout: &Value, cx: &mut Context<Self>) {
        if let Some(state) = leaf_state(layout, "left", "search") {
            self.search.update(cx, |s, cx| s.restore(state, cx));
        }
    }

    /// `workspace.json` as this session would leave it: the file that was
    /// read with the tab groups, the active leaf, the sidebars' collapse
    /// state and `lastOpenFiles` brought up to date, and every other key as
    /// it was.
    fn layout_json(&self, cx: &App) -> Value {
        let mut layout = self.layout.clone().unwrap_or_else(|| json!({}));
        let Some(object) = layout.as_object_mut() else {
            return layout;
        };
        // A vault Obsidian never laid out has no docks in the file, or no
        // file at all. Obsidian writes its default ones; so does TFT, so
        // the panes' own state has a leaf to live in rather than being
        // lost, with `main` first the way Obsidian orders the keys.
        if !object.contains_key("main") {
            object.insert("main".into(), Value::Null);
        }
        let left_tab = self.left_tab as usize;
        if !object.get("left").is_some_and(Value::is_object) {
            let json = default_dock_json(self.current.as_deref(), left_tab);
            object.insert("left".into(), json);
        }
        // The Files leaf's own state, where the file has that leaf.
        {
            let explorer = self.explorer.read(cx);
            let (sort, auto_reveal) = (explorer.sort_order(), explorer.auto_reveal());
            if let Some(left) = object.get_mut("left") {
                set_explorer_leaf_state(left, sort, auto_reveal);
                set_leaf_state(left, "search", &self.search.read(cx).leaf_state(cx));
            }
        }
        let mut active_id = Value::Null;
        let main = self.layout_node_json(&self.layout_tree, &mut active_id);
        // `main` keeps its place in a file that had one.
        match object.get_mut("main") {
            Some(slot) => *slot = main,
            None => {
                object.insert("main".into(), main);
            }
        }
        // `floating` while a popout exists, in its place or else right
        // before `active` (after the docks and the ribbon, where Obsidian
        // puts it); no key at all once none does, as Obsidian's own
        // serialization has none then (observed, `s06` of the popout probe).
        let windows: Vec<Value> = self
            .windows
            .iter()
            .map(|w| self.window_json(w, &mut active_id))
            .collect();
        if windows.is_empty() {
            object.shift_remove("floating");
        } else {
            let existing = object.get("floating").and_then(Value::as_object).cloned();
            let floating = floating_json(existing.as_ref(), windows);
            match object.get_mut("floating") {
                Some(slot) => *slot = floating,
                None => {
                    let at = object
                        .keys()
                        .position(|k| k == "active")
                        .unwrap_or(object.len());
                    object.shift_insert(at, "floating".into(), floating);
                }
            }
        }
        object.insert("active".into(), active_id);
        {
            let (dock, visible, width) = ("left", self.left_visible, self.left_width);
            if let Some(d) = object.get_mut(dock).and_then(Value::as_object_mut) {
                if visible {
                    d.shift_remove("collapsed");
                } else {
                    d.insert("collapsed".into(), json!(true));
                }
                // The width is rewritten only once a drag moved it, so the
                // `300` Obsidian wrote stays the integer it was and a file
                // without the key gains it only for a sidebar that moved.
                let same = d
                    .get("width")
                    .and_then(Value::as_f64)
                    .is_some_and(|w| (w - width).abs() < 0.5);
                if !same && (d.contains_key("width") || width != SIDEBAR_WIDTH as f64) {
                    d.insert("width".into(), json!(width));
                }
            }
        }
        object.insert("lastOpenFiles".into(), json!(self.last_open_files));
        layout
    }

    /// The layout tree for the socket's state dump: group ids at the leaves.
    fn layout_tree_json(&self, node: &Layout) -> Value {
        match node {
            Layout::Group(id) => json!({"group": id, "dimension": self.dimension_of(node)}),
            Layout::Split {
                direction,
                children,
                ..
            } => json!({
                "direction": direction.as_str(),
                "dimension": self.dimension_of(node),
                "children": children.iter().map(|c| self.layout_tree_json(c)).collect::<Vec<Value>>(),
            }),
        }
    }

    /// One node of `main`: a group as a `tabs` node with its leaves and
    /// `currentTab`, a split with its children and `direction`, each over
    /// the keys it was read with. The active group's active leaf id lands in
    /// `active_id` on the way, since a new tab's id exists only in the leaf
    /// written here.
    fn layout_node_json(&self, node: &Layout, active_id: &mut Value) -> Value {
        match node {
            Layout::Group(id) => {
                let Some(ix) = self.group_ix(*id) else {
                    return Value::Null;
                };
                let active = ix == self.active_group;
                let (tabs, active_tab) = if active {
                    (self.tab_snapshots(), self.active_tab)
                } else {
                    (self.groups[ix].tabs.clone(), self.groups[ix].active_tab)
                };
                let leaves: Vec<Value> = tabs.iter().map(leaf_json).collect();
                if active {
                    *active_id = leaves
                        .get(active_tab)
                        .and_then(|l| l.get("id"))
                        .cloned()
                        .unwrap_or(Value::Null);
                }
                tabs_node_json(self.groups[ix].node.clone(), leaves, active_tab)
            }
            Layout::Split {
                direction,
                children,
                node,
            } => split_node_json(
                node.clone(),
                children
                    .iter()
                    .map(|c| self.layout_node_json(c, active_id))
                    .collect(),
                *direction,
            ),
        }
    }

    /// One `window` node of `floating`: the window's own keys around its
    /// groups, written as `main`'s are.
    fn window_json(&self, w: &WindowNode, active_id: &mut Value) -> Value {
        let (direction, children) = match &w.tree {
            Layout::Split {
                direction,
                children,
                ..
            } => (*direction, children.iter().collect::<Vec<_>>()),
            // The root is a split by construction; a bare group is wrapped
            // the way Obsidian's root always wraps one.
            group @ Layout::Group(_) => (SplitDirection::Vertical, vec![group]),
        };
        window_node_json(
            &w.node,
            children
                .into_iter()
                .map(|c| self.layout_node_json(c, active_id))
                .collect(),
            direction,
        )
    }

    /// Writes `workspace.json` back if the session changed the layout.
    /// Called on the way out, where Obsidian writes it.
    pub fn save_layout(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let fresh = self.layout_json(cx);
        if self.layout.as_ref() == Some(&fresh) {
            return;
        }
        // The whole file, not a merge into it: `fresh` already carries
        // every key the file had, and a merge would keep a `floating` the
        // last popout's closing took out.
        if let Err(err) = state.vault().replace_config("workspace.json", &fresh) {
            self.report(format!("workspace.json could not be written: {err}"), cx);
            return;
        }
        self.layout = Some(fresh);
    }

    /// Everything that has to reach the disk before the app goes away: the
    /// note's unsaved edits and the layout.
    pub fn on_quit(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        if let Some(path) = self.current.clone() {
            self.save_folds(&path, cx);
        }
        self.save_layout(cx);
    }

    // ----- the control socket ----------------------------------------------------

    /// The whole shell as JSON, for the control socket's `state` method: a
    /// test driving the app over the socket reads this instead of a
    /// screenshot. Every key is always there, `null` when there is nothing
    /// to say, so a caller can index into it without guessing. Each view
    /// describes itself; the rest is the shell's own.
    /// An `obsidian://` URI, from the command line or the control socket:
    /// see [`crate::uri`]. A URI naming another vault is refused.
    pub fn handle_uri(
        &mut self,
        uri: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        use crate::uri::{names_vault, Uri};
        let parsed = crate::uri::parse(uri)?;
        let Some(state) = self.state.clone() else {
            return Err("no vault is open".to_string());
        };
        let vault_name = state
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let vault = match &parsed {
            Uri::Open { vault, .. }
            | Uri::New { vault, .. }
            | Uri::Search { vault, .. }
            | Uri::Daily { vault } => vault.clone(),
        };
        if !names_vault(vault.as_deref(), &vault_name) {
            return Err(format!(
                "the URI names the vault {}; this window shows {vault_name}",
                vault.unwrap_or_default()
            ));
        }
        match parsed {
            Uri::Open { file, path, .. } => {
                if let Some(path) = path {
                    let exists = state.vault().absolute(&path).exists();
                    let path = if exists || path.contains('.') {
                        path
                    } else {
                        format!("{path}.md")
                    };
                    self.open_file(path, None, cx);
                } else if let Some(name) = file {
                    if !self.open_by_name(&name, cx) {
                        return Err(format!("no note named {name}"));
                    }
                }
                Ok(())
            }
            Uri::New {
                name,
                path,
                content,
                silent,
                append,
                prepend,
                overwrite,
                ..
            } => {
                let path = match (path, name) {
                    (Some(path), _) => path,
                    (None, Some(name)) => format!("{}.md", name.trim_end_matches(".md")),
                    (None, None) => "Untitled.md".to_string(),
                };
                let content = content.unwrap_or_default();
                let absolute = state.vault().absolute(&path);
                if absolute.exists() {
                    let on_disk = std::fs::read_to_string(&absolute).map_err(|e| e.to_string())?;
                    let text = if overwrite {
                        Some(content)
                    } else if append {
                        Some(format!("{on_disk}{content}"))
                    } else if prepend {
                        Some(format!("{content}{on_disk}"))
                    } else {
                        None
                    };
                    if let Some(text) = text {
                        if self.buffer_differs(&path, &on_disk, cx) {
                            return Err(format!("{path} has unsaved edits"));
                        }
                        tft_vault::write_no_follow(&absolute, text.as_bytes())
                            .map_err(|e| e.to_string())?;
                        self.files_changed(vec![absolute], cx);
                    }
                } else {
                    let changes = state.create_note(&path, &content)?;
                    self.after_changes(changes, cx);
                }
                if !silent {
                    self.open_file(path, None, cx);
                }
                Ok(())
            }
            Uri::Search { query, .. } => {
                self.show_search(&query.unwrap_or_default(), Some(window), cx);
                Ok(())
            }
            Uri::Daily { .. } => Ok(()),
        }
    }

    /// Builds the reading model a note opened in an editing mode is still
    /// waiting on, so a state dump describes the note rather than nothing.
    pub fn ensure_reading_built(&mut self, cx: &mut Context<Self>) {
        self.reading.update(cx, |r, cx| r.ensure_built(cx));
    }

    pub fn describe(&self, text: bool, rows: usize, window: &Window, cx: &App) -> Value {
        let mut vault = Map::new();
        vault.insert(
            "root".into(),
            json!(self.state.as_ref().map(|s| s.root.display().to_string())),
        );
        vault.insert("name".into(), json!(self.vault_name.to_string()));
        vault.insert("loaded".into(), json!(self.state.is_some()));
        vault.insert("menu_open".into(), json!(self.vault_menu_open));
        vault.insert(
            "loading".into(),
            json!(self.loading.as_ref().map(|l| l.to_string())),
        );

        let mut file = Map::new();
        file.insert("path".into(), json!(self.current));
        file.insert(
            "detached".into(),
            json!(self.detached.as_ref().map(|n| n.to_string())),
        );
        file.insert("mode".into(), json!(self.mode.as_str()));
        file.insert("dirty".into(), json!(self.editor.read(cx).is_dirty()));
        put_capped(
            &mut file,
            "history",
            self.history.iter().map(|e| json!(e.path)).collect(),
            rows,
        );
        file.insert("history_ix".into(), json!(self.history_ix));

        let tab_json = |t: &Tab| json!({"path": t.path, "mode": t.mode.as_str(), "title": t.title(), "pinned": t.pinned});
        let tabs = json!({
            "active": self.active_tab,
            "items": self.tab_snapshots().iter().map(tab_json).collect::<Vec<Value>>(),
        });
        // The tab groups and how they are split; `tabs` above is the active
        // group's, as it was before there could be more than one.
        let groups = json!({
            "active": self.active_group_id(),
            // The popout window the active group is in, `null` for the
            // main window: the window the socket's input goes to.
            "active_window": self.window_of_group(self.active_group_id()),
            "layout": self.layout_tree_json(&self.layout_tree),
            // The popout windows the layout holds; `open` says whether a
            // gpui window is showing one (a restored popout has none yet).
            "windows": self
                .windows
                .iter()
                .map(|w| {
                    json!({
                        "id": w.id,
                        // A window being described from inside its own
                        // update is out of the app's table for the moment.
                        "open": w.is_open(cx)
                            || w.handle.is_some_and(|h| {
                                gpui::AnyWindowHandle::from(h) == window.window_handle()
                            }),
                        "title": w.title,
                        "bounds": w.bounds().map(|b| {
                            json!({"x": b.x, "y": b.y, "width": b.width, "height": b.height})
                        }),
                        "layout": self.layout_tree_json(&w.tree),
                    })
                })
                .collect::<Vec<Value>>(),
            "items": self
                .groups
                .iter()
                .enumerate()
                .map(|(ix, g)| {
                    let view = self.group_view(ix);
                    json!({
                        "id": g.id,
                        "active_tab": view.active_tab,
                        "dimension": g.dimension(),
                        "tabs": view.tabs.iter().map(tab_json).collect::<Vec<Value>>(),
                    })
                })
                .collect::<Vec<Value>>(),
        });

        let mut left = Map::new();
        left.insert("visible".into(), json!(self.left_visible));
        left.insert("width".into(), json!(self.left_width));
        left.insert(
            "tab".into(),
            json!(match self.left_tab {
                LeftTab::Files => "files",
                LeftTab::Search => "search",
            }),
        );
        left.insert("explorer".into(), self.explorer.read(cx).describe(rows));
        left.insert("search".into(), self.search.read(cx).describe(rows, cx));

        // Whether the graph is on screen is the shell's to say; the view
        // itself only knows what it would draw.
        let mut graph = self.graph.read(cx).describe();
        if let Some(object) = graph.as_object_mut() {
            object.insert("open".into(), json!(matches!(self.mode, ViewMode::Graph)));
        }
        // Page preview's popovers, first to last: each one's window, the
        // note its links resolve against, the anchor's line box, the left
        // edge the box is drawn at, what it shows, and whether it edits.
        let viewport_width = window.viewport_size().width.as_f32();
        let popovers: Vec<Value> = self
            .hover_preview
            .shown
            .iter()
            .map(|s| {
                json!({
                    "window": s.window,
                    "from": s.from,
                    "anchor": [s.anchor.left().as_f32(), s.anchor.top().as_f32(), s.anchor.size.height.as_f32()],
                    "left": crate::hover_preview::left_edge(s.anchor.left().as_f32(), viewport_width),
                    "height": s.height.get(),
                    "content": match &s.content {
                        crate::hover_preview::Content::Missing(line) => json!({"missing": line}),
                        crate::hover_preview::Content::Blocks { blocks } => json!({"blocks": blocks.len()}),
                    },
                    "over": s.over,
                    "editing": s.editing.as_ref().map(|e| {
                        let e = e.read(cx);
                        json!({
                            "path": e.name.to_string(),
                            "dirty": e.is_dirty(),
                            "selection": [e.selection().start, e.selection().end],
                        })
                    }),
                })
            })
            .collect();

        json!({
            "vault": Value::Object(vault),
            "file": Value::Object(file),
            "tabs": tabs,
            "groups": groups,
            "docks": {"left": Value::Object(left)},
            // The one open file menu: the editor's text or a link in the
            // reading view.
            "file_menu": self.file_menu.as_ref().map(file_menu::describe),
            "modal": self.describe_modal(rows, cx),
            "editor": self.editor.read(cx).describe(text, cx),
            "reading": self.reading.read(cx).describe(cx),
            "graph": graph,
            "canvas": self.canvas.read(cx).describe(cx),
            "pdf": self.pdf.read(cx).state_json(cx),
            "status": self.status_text(),
            "status_bar": self.status_bar_text(cx),
            "status_bar_items": self.status_bar_items(cx),
            // The empty tab's pills, by the text they carry, while that is
            // what the main area shows.
            "empty_tab": (self.current.is_none() && self.main_view(cx) == MainView::Empty).then(|| {
                empty_tab_actions(&cx.global::<Commands>().0)
                    .iter()
                    .map(EmptyAction::text)
                    .collect::<Vec<String>>()
            }),
            // The view header's title while it is being renamed in place,
            // so a driver can see the typed name before Enter commits it.
            "view_title": self.title_edit.as_ref().map(|edit| json!({
                "group": edit.group,
                "file": edit.path,
                "text": edit.editing.field.text(),
                "cursor": edit.editing.field.cursor(),
            })),
            "notices": self.notices.texts(),
            // The window each of those draws in, the popout's id or null
            // for the vault window.
            "notices_window": self.notices.windows(&|id| self.popout_open(id)),
            // Page preview: the link under the pointer, and the popover up
            // (its window, the note its links resolve against, the anchor's
            // line box and the left edge the box is drawn at).
            "hover_preview": {
                "pending": self.hover_preview.pending.as_ref().map(|p| json!({
                    "target": p.target,
                    "from": p.from,
                    "window": p.window,
                    "depth": p.depth,
                })),
                "shown": popovers.first().cloned(),
                "popovers": popovers,
                "over_popover": self.hover_preview.shown.iter().any(|s| s.over),
            },
            "focus": {
                "name": self.focus_name(window, cx),
                "context_stack": context_names(window),
            },
            "window": {
                "width": window.viewport_size().width.as_f32(),
                "height": window.viewport_size().height.as_f32(),
                "scale": window.scale_factor(),
                "active": window.is_window_active(),
                "title": self.described_title(window),
                // The popout being described, `null` for the main window.
                "popout": self.popout_of_window(window.window_handle()),
            },
        })
    }

    /// The title of `window`: a popout's own, or the main window's.
    fn described_title(&self, window: &Window) -> SharedString {
        self.popout_of_window(window.window_handle())
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .map(|w| w.title.clone())
            .unwrap_or_else(|| self.window_title())
    }

    /// The id of the popout `handle` shows, if it is one.
    fn popout_of_window(&self, handle: gpui::AnyWindowHandle) -> Option<u64> {
        self.windows
            .iter()
            .find(|w| {
                w.handle
                    .is_some_and(|h| gpui::AnyWindowHandle::from(h) == handle)
            })
            .map(|w| w.id)
    }

    /// Runs `f` with the window the active group is in, which is the one
    /// a caller of the control socket is looking at: `window` itself when
    /// that is it, the popout otherwise (its own update, nested in this
    /// one). The popout gone, `window` stands in.
    pub fn in_active_window<R>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut App) -> R,
    ) -> R {
        let target = self.active_window_handle();
        if target == window.window_handle() {
            return f(self, window, cx);
        }
        let mut f = Some(f);
        match target.update(cx, |_, target, cx| (f.take().unwrap())(self, target, cx)) {
            Ok(result) => result,
            Err(_) => (f.take().unwrap())(self, window, cx),
        }
    }

    /// The overlay that is up, if any: its kind plus whatever the modal
    /// itself says about its query and rows.
    fn describe_modal(&self, rows: usize, cx: &App) -> Value {
        let mut modal = Map::new();
        // Settings is a window rather than a modal, but it is still the
        // thing in front of the caller, and the socket's contract has said
        // "settings" here since before it moved out of the workspace.
        let settings_up = self.settings_up(cx);
        modal.insert(
            "kind".into(),
            json!(match self.modal {
                _ if settings_up => "settings",
                Modal::None => "none",
                Modal::Switcher => "switcher",
                Modal::Prompt => "prompt",
                Modal::CopyResults => "copy-results",
                Modal::Chooser => "chooser",
                Modal::Downloads => "download-attachments",
                Modal::DebugInfo => "debug-info",
                Modal::MigrateFrontmatter => "migrate-frontmatter",
                Modal::PdfExport => "export-pdf",
                Modal::ImageExport => "export-image",
            }),
        );
        // The popout it draws in, by id, or null for the vault window (and
        // with nothing up). Settings is a window of its own.
        modal.insert(
            "window".into(),
            json!(self.overlay_window.flatten().filter(|_| !settings_up)),
        );
        // The four list modals fill these in; with no modal up they stay
        // empty rather than disappearing.
        modal.insert("query".into(), Value::Null);
        modal.insert("selected".into(), Value::Null);
        modal.insert("error".into(), Value::Null);
        modal.insert("rows".into(), Value::Array(Vec::new()));
        let view = match self.modal {
            _ if settings_up => Some(self.settings.read(cx).describe(rows, cx)),
            Modal::Switcher => Some(self.switcher.read(cx).describe(rows, cx)),
            Modal::Prompt => Some(self.prompt.read(cx).describe(rows, cx)),
            Modal::CopyResults => Some(self.copy_results.read(cx).describe()),
            Modal::Chooser => Some(self.chooser.read(cx).describe(rows, cx)),
            Modal::Downloads => self.describe_downloads(cx),
            Modal::DebugInfo => Some(self.debug_info.read(cx).describe()),
            Modal::MigrateFrontmatter => self.describe_migrate(cx),
            Modal::PdfExport => Some(self.pdf_export.read(cx).describe()),
            Modal::ImageExport => Some(self.image_export.read(cx).describe()),
            Modal::None => None,
        };
        if let Some(Value::Object(view)) = view {
            for (key, value) in view {
                modal.insert(key, value);
            }
        }
        // Settings has tabs rather than rows; the tab it is on is the part
        // a caller steers by.
        modal.insert(
            "settings_tab".into(),
            modal.get("tab").cloned().unwrap_or(Value::Null),
        );
        modal.insert(
            "pending_op".into(),
            match &self.pending {
                None => Value::Null,
                Some(PendingOp::Rename(path)) => json!({"op": "rename", "path": path}),
                Some(PendingOp::Move(path)) => json!({"op": "move", "path": path}),
                Some(PendingOp::Delete(path)) => json!({"op": "delete", "path": path}),
                Some(PendingOp::NewFolder { parent }) => {
                    json!({"op": "new-folder", "parent": parent})
                }
                Some(PendingOp::UpdateLinks { from, to }) => {
                    json!({"op": "update-links", "from": from, "to": to})
                }
                Some(PendingOp::RenameHeading { path, line }) => {
                    json!({"op": "rename-heading", "path": path, "line": line})
                }
            },
        );
        Value::Object(modal)
    }

    /// Where typed text would land: "editor", "input" or "none". Deciding
    /// is separate from doing because only one of the two routes may run
    /// with this workspace borrowed: delivering a keystroke to a focused
    /// input re-enters the focused view, and that view can be this one.
    pub fn type_target(&self, window: &Window, cx: &App) -> &'static str {
        Self::typed_text_target(self.focus_name(window, cx), self.main_view(cx))
    }

    /// Which of the two routes `text` takes, for the focus the shell
    /// reports and the view the main area is painting.
    ///
    /// The editor is answered only while it is on screen. Its handle can
    /// keep the focus after the main area has stopped painting it — the
    /// keyboard reaches nothing from there, but this route does not go
    /// through the dispatch tree, and would otherwise fill a buffer that
    /// belongs to no file with text nothing ever saves.
    fn typed_text_target(focus: &str, painted: MainView) -> &'static str {
        match focus {
            // A property row, a table cell or the inline title is the
            // editor's own field.
            "editor" | "property-field" | "cell-field" | "title-field" => match painted {
                MainView::Editor => "editor",
                _ => "none",
            },
            "canvas-editor" => match painted {
                MainView::Canvas => "canvas-editor",
                _ => "none",
            },
            // The pane is on screen whatever the main area paints, and its
            // field is the editor's, in reading view too.
            "file-properties-field" => "editor",
            "workspace" | "reading" | "graph" | "canvas" | "pdf" | "file-properties" | "none" => {
                "none"
            }
            _ => "input",
        }
    }

    /// Types `text` into the editor, for the socket's `type` method, and
    /// says so. Only the editor is reachable from here: the inputs belong
    /// to the views that own them and are typed into from outside this
    /// borrow, so a caller asks [`Workspace::type_target`] first and
    /// anything but "editor" is answered the other way.
    pub fn type_into_focused(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> &'static str {
        match self.type_target(window, cx) {
            "editor" => {
                // `type_text` already routes to an open property row or
                // table cell rather than the body.
                self.editor
                    .update(cx, |e, cx| e.type_text(text, window, cx));
                "editor"
            }
            "canvas-editor" => {
                if let Some(editor) = self.canvas.read(cx).card_editor().cloned() {
                    editor.update(cx, |e, cx| e.type_text(text, window, cx));
                }
                "canvas-editor"
            }
            // Nothing else is reachable from here, so nothing else is
            // typed into: a focused input is answered by the caller, which
            // asked where the text would go before handing it over.
            _ => "none",
        }
    }

    /// A name for whatever has focus, for the state dump and to decide
    /// where typed text goes. Only handles this shell can reach are named
    /// by handle; the find panel's two fields belong to the editor, which
    /// is asked. `explorer` never comes back: the file tree's rows are
    /// painted without a focus handle, so the keyboard is never in it.
    pub fn focus_name(&self, window: &Window, cx: &App) -> &'static str {
        // Settings has a window of its own, so its focus is not this
        // window's to report: that it is up is what the caller is after.
        if self.settings_up(cx) {
            return "settings";
        }
        if window.focused(cx).is_none() {
            return "none";
        }
        if self.editor.read(cx).focus_handle_ref().is_focused(window) {
            // An open property row or table cell takes the editor's keys
            // without taking the focus.
            return match self.editor.read(cx).field_kind() {
                Some("cell") => "cell-field",
                Some("title") => "title-field",
                Some(_) => "property-field",
                None => "editor",
            };
        }
        if self.reading.focus_handle(cx).is_focused(window) {
            return "reading";
        }
        if self.graph.focus_handle(cx).is_focused(window) {
            return "graph";
        }
        if self
            .canvas
            .read(cx)
            .card_editor()
            .is_some_and(|e| e.read(cx).focus_handle_ref().is_focused(window))
        {
            return "canvas-editor";
        }
        if self.canvas.focus_handle(cx).is_focused(window) {
            return "canvas";
        }
        if let Some(field) = self.pdf.read(cx).focused_field(window, cx) {
            return field;
        }
        if self.pdf.focus_handle(cx).is_focused(window) {
            return "pdf";
        }
        // The view header's inline rename field, which takes typed text
        // through the workspace's own input handler.
        if self.title_focus.is_focused(window) {
            return "view-title";
        }
        if self.switcher.focus_handle(cx).is_focused(window) {
            return "switcher-input";
        }
        if self.prompt.focus_handle(cx).is_focused(window) {
            return "prompt-input";
        }
        if self.chooser.focus_handle(cx).is_focused(window) {
            return "chooser-input";
        }
        if self.search.focus_handle(cx).is_focused(window) {
            return "search-input";
        }
        if self.explorer.focus_handle(cx).is_focused(window) {
            return "explorer";
        }
        if self.focus_handle.is_focused(window) {
            return "workspace";
        }
        // Both find fields sit in one key context, so the stack cannot tell
        // them apart; the editor holds the two handles and can.
        if let Some(name) = self.editor.read(cx).find_focus(window, cx) {
            return name;
        }
        "unknown"
    }

    /// Focuses one of the main views by name, for the socket's `focus`
    /// method. The mode is left alone: focus is all that is asked for.
    pub fn focus_view(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // The tree is in the left dock, not the main area: showing the
        // dock's Files tab is what puts its handle on screen.
        if name == "explorer" {
            self.leave_search_field(window, cx);
            self.left_visible = true;
            self.left_tab = LeftTab::Files;
            self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
            cx.notify();
            return Ok(());
        }
        match Self::focus_target(name, self.main_view(cx))? {
            // The root div paints in every frame, so its handle is always
            // somewhere the keyboard can sit.
            None => window.focus(&self.focus_handle, cx),
            Some(_) => self.focus_main(window, cx),
        }
        cx.notify();
        Ok(())
    }

    /// Which view the socket's `focus` names: `None` for the workspace
    /// root, or the main view asked for once it is the one being painted.
    ///
    /// A view the main area is not painting is refused rather than focused.
    /// Its handle is in no frame's dispatch tree, so the keyboard parked on
    /// it reaches nothing at all — not even the hotkey that would undo it,
    /// which is the dead state the mode switches were hardened against.
    fn focus_target(name: &str, painted: MainView) -> Result<Option<MainView>, String> {
        let wanted = match name {
            "workspace" => return Ok(None),
            "editor" => MainView::Editor,
            "reading" => MainView::Reading,
            "graph" => MainView::Graph,
            "canvas" => MainView::Canvas,
            "pdf" => MainView::Pdf,
            "release-notes" => MainView::ReleaseNotes,
            other => return Err(format!("unknown view: {other}")),
        };
        if wanted != painted {
            return Err(format!(
                "the main area is painting {}, not {name}: \
                 switch the mode first, or focus the view that is on screen",
                painted.as_str()
            ));
        }
        Ok(Some(wanted))
    }

    /// Empties the status bar and takes down every notice, so a caller
    /// can tell the next notice from the one already there.
    pub fn clear_status(&mut self, cx: &mut Context<Self>) {
        self.notices.clear();
        self.status = "".into();
        cx.notify();
    }

    /// The status bar's items, left to right, for what a leaf holds.
    ///
    /// An **empty** tab is not a blank status bar: with one active
    /// Obsidian's reads `0 properties` (see [[Specification#The empty tab]]),
    /// so the item stays and reads as zero. A canvas tab shows nothing. The
    /// caret position is TFT's own; the parse and shape numbers behind it
    /// are diagnostics and appear only with `TFT_DEBUG_STATUS` set.
    fn status_items(current: Option<&str>, caret: &str) -> Vec<String> {
        let Some(path) = current else {
            return EMPTY_TAB_STATUS
                .iter()
                .map(|item| item.to_string())
                .collect();
        };
        if FileKind::from_path(path) == FileKind::Canvas {
            return Vec::new();
        }
        if caret.is_empty() {
            Vec::new()
        } else {
            vec![caret.to_string()]
        }
    }

    /// The items [`Workspace::status_items`] gives the open leaf.
    pub fn status_bar_items(&self, cx: &App) -> Vec<String> {
        let caret = if self.mode.is_editing() {
            let editor = self.editor.read(cx);
            let cursor = editor.cursor();
            let cursors = match editor.cursor_count() {
                1 => String::new(),
                n => format!("{n} cursors  ·  "),
            };
            let timings = if debug_status() {
                format!(
                    "parse {:.1} ms, {} spans  ·  shape {:.2} ms  ·  ",
                    editor.stats.parse_ms,
                    editor.stats.spans,
                    editor.prepaint_ms()
                )
            } else {
                String::new()
            };
            format!(
                "Ln {}, Col {}  ·  {cursors}{timings}",
                cursor.line + 1,
                cursor.col + 1
            )
        } else {
            String::new()
        };
        Self::status_items(self.current.as_deref(), &caret)
    }

    /// The same items as one string, for the socket's state dump: the
    /// separator is the one the bar itself puts between them.
    pub fn status_bar_text(&self, cx: &App) -> Option<String> {
        let items = self.status_bar_items(cx);
        (!items.is_empty()).then(|| items.join("  ·  "))
    }

    /// The title the desktop shows, as the last paint set it: empty until
    /// the first frame has run.
    pub fn window_title(&self) -> SharedString {
        self.window_title.clone()
    }

    /// What the last command said, which is how every command reports:
    /// the newest notice while one is up, the status bar otherwise.
    pub fn status_text(&self) -> String {
        match self.notices.latest() {
            Some(text) => text.to_string(),
            None => self.status.to_string(),
        }
    }

    /// The open vault's folder, if one is open.
    pub fn vault_root(&self) -> Option<PathBuf> {
        self.state.as_ref().map(|s| s.root.clone())
    }

    /// An absolute path for a vault-relative one, refusing anything that
    /// points outside the vault. This is the only path the socket reads and
    /// writes through, and the vault folder is the only thing TFT may
    /// touch.
    pub fn vault_path(&self, path: &str) -> Result<PathBuf, String> {
        let root = self
            .vault_root()
            .ok_or_else(|| "No vault is open".to_string())?;
        vault_path_in(&root, path)
    }

    /// The vault's own spelling of a vault-relative path, if the index
    /// carries one: that spelling is the key every pane is looked up by, so
    /// the caller's casing is only a request for it.
    pub fn vault_file_path(&self, path: &str) -> Option<String> {
        let state = self.state.as_ref()?;
        state
            .vault()
            .files()
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(path))
            .map(|f| f.path.clone())
    }

    /// The open vault's files, for the socket's `list_files`: the same list
    /// the explorer is built from, under `folder` when one is given.
    pub fn list_vault_files(&self, folder: Option<&str>) -> Vec<Value> {
        let Some(state) = self.state.as_ref() else {
            return Vec::new();
        };
        // Vault paths keep their case and compare without it, so a folder
        // stored as `Sub` answers to `sub` as well.
        let prefix = folder
            .map(|f| f.trim_matches('/'))
            .filter(|f| !f.is_empty())
            .map(|f| format!("{}/", f.to_lowercase()));
        state
            .vault()
            .files()
            .iter()
            .filter(|f| {
                prefix
                    .as_deref()
                    .is_none_or(|p| f.path.to_lowercase().starts_with(p))
            })
            .map(|f| json!({"path": f.path, "kind": kind_name(f.kind), "size": f.size}))
            .collect()
    }

    /// Every command in the registry with the hotkeys `hotkeys.json` leaves
    /// it, and whether [`Workspace::run_command`] would run it *now*.
    ///
    /// `runnable` is the check callback's answer, so it moves with the
    /// window: an editing command reads false from a reading leaf, which
    /// is the same thing the palette shows.
    pub fn list_commands(&self, window: &Window, cx: &App) -> Vec<Value> {
        cx.global::<Commands>()
            .0
            .iter()
            .map(|c| {
                let hotkeys: Vec<String> = c.hotkeys.iter().filter_map(keystroke).collect();
                json!({
                    "id": c.id,
                    "name": c.name,
                    "hotkeys": hotkeys,
                    "runnable": self.can_run(&c.id, window, cx),
                })
            })
            .collect()
    }

    /// The mode a socket request names, if it names one at all.
    pub fn open_mode(&self, mode: Option<&str>) -> Result<Option<ViewMode>, String> {
        match mode {
            None => Ok(None),
            Some(name) => ViewMode::parse(name)
                .map(Some)
                .ok_or_else(|| format!("unknown mode: {name}")),
        }
    }

    // ----- rendering -------------------------------------------------------------

    /// A sidebar's tab: Obsidian shows the view's icon alone, 16 px in a
    /// 28 by 25 button with a 4 px radius, the active one on a wash.
    fn dock_tab(
        &self,
        label: &'static str,
        icon_name: &'static str,
        active: bool,
        theme: &Theme,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(label)
            .flex_shrink_0()
            .w(px(28.))
            .h(px(25.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .rounded(px(4.))
            .group(label)
            // The active dock tab: the hover wash under `--icon-color-focused`
            // at `--icon-opacity-active`; the others `--icon-color` at
            // `--icon-opacity`, brightening to `--icon-color-hover` under
            // the pointer (probe of 2026-11-02, `split.left.tab-header.*`).
            .when(active, |d| d.bg(theme.tokens.background_modifier_hover))
            .hover(|s| s.bg(theme.tokens.background_modifier_hover))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .child(
                icon(
                    icon_name,
                    ICON_SIZE,
                    if active {
                        theme.tokens.icon_color_focused
                    } else {
                        theme.tokens.icon_color
                    },
                )
                .opacity(if active {
                    theme.tokens.icon_opacity_active
                } else {
                    theme.tokens.icon_opacity
                })
                .when(!active, |s| {
                    s.group_hover(label, |s| {
                        s.opacity(theme.tokens.icon_opacity_hover)
                            .text_color(theme.tokens.icon_color_hover)
                    })
                }),
            )
    }

    /// A right click on something that stands for a file away from the
    /// Files view -- an Outgoing links row (a Links row, a
    /// mentions group, or a match, which belongs to its group's file), or a
    /// link in the reading view: the file menu, in the list Obsidian gives
    /// rows away from the Files view ([`file_menu::Target::Linked`]).
    fn open_file_menu(
        &mut self,
        path: String,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.file_menu = Some(file_menu::Menu {
            position,
            submenu: false,
            anchor: Anchor::TopLeft,
            target: file_menu::Target::Linked(path),
        });
        cx.notify();
    }

    fn close_file_menu(&mut self, cx: &mut Context<Self>) {
        if self.file_menu.take().is_some() {
            cx.notify();
        }
    }

    /// The vault file a wikilink's text resolves to, from the note it was
    /// written in; `None` for a dangling link.
    fn link_path(&self, body: &str) -> Option<String> {
        let state = self.state.as_ref()?;
        let source = self.current.clone().unwrap_or_default();
        state.cache().resolve(body, &source).map(str::to_string)
    }

    /// A right click in the editor, which has placed the caret already: the
    /// editor's context menu, at the click. What it landed in decides the
    /// rows — a wikilink that names a file wraps the file menu around them,
    /// a tag reads as a tag, and everything else (plain text, an external
    /// link, a dangling wikilink) is the plain menu.
    fn open_editor_menu(
        &mut self,
        position: gpui::Point<Pixels>,
        selection: String,
        selected: bool,
        link: Option<LinkTarget>,
        spelling: Option<crate::spellcheck::SpellHit>,
        cx: &mut Context<Self>,
    ) {
        let spot = match link {
            // A link to a note that is not there yet names no file for
            // Rename, Move or Copy path to act on, so it reads as text.
            Some(LinkTarget::Internal(body)) => match self.link_path(&body) {
                Some(path) => file_menu::Spot::Link(path),
                None => file_menu::Spot::Text,
            },
            Some(LinkTarget::Tag(tag)) => file_menu::Spot::Tag(tag),
            // An external link raised the plain menu in the probe, and a
            // footnote reference was not probed; both read as text.
            _ => file_menu::Spot::Text,
        };
        self.file_menu = Some(file_menu::Menu {
            position,
            submenu: false,
            anchor: Anchor::TopLeft,
            target: file_menu::Target::Editor(file_menu::EditorClick {
                spot,
                selection,
                selected,
                spelling,
            }),
        });
        cx.notify();
    }

    /// A right click on a link in the reading view: the file menu, the one
    /// menu that view raises. A link to a note that is not there yet names
    /// no file, so it raises nothing rather than a menu of rows that would
    /// act on nothing.
    fn open_reading_menu(
        &mut self,
        target: LinkTarget,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let LinkTarget::Internal(body) = target else {
            return;
        };
        let Some(path) = self.link_path(&body) else {
            return;
        };
        self.open_file_menu(path, position, cx);
    }

    /// What a pick on that menu does: the handlers the explorer's events
    /// already route to, plus the two rows the explorer's menu has no twin
    /// of, plus the editor menu's own rows.
    fn file_menu_pick(
        &mut self,
        pick: Pick,
        target: &file_menu::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = target.path().to_string();
        match pick {
            Pick::CloseTab
            | Pick::CloseOthers
            | Pick::CloseAfter
            | Pick::CloseAll
            | Pick::TogglePin
            | Pick::MoveToNewWindow => {
                if let file_menu::Target::Tab { group, ix, .. } = target {
                    self.tab_menu_pick(pick, *group, *ix, window, cx);
                }
            }
            Pick::AddFileProperty => self.run_command("markdown:add-metadata-property", window, cx),
            Pick::Find => self.run_command("editor:open-search", window, cx),
            Pick::Replace => self.run_command("editor:open-search-replace", window, cx),
            Pick::OpenInNewTab => self.open_link_in_new_tab(path, None, cx),
            Pick::OpenToRight => self.open_to_right(path, window, cx),
            // The view header's "Open in new window" copies the leaf, as
            // `workspace:open-in-new-window` does; a row's opens its file.
            Pick::OpenInNewWindow => match target {
                file_menu::Target::View(_) | file_menu::Target::Tab { .. } => {
                    self.open_in_new_window(window, cx)
                }
                _ => self.open_path_in_new_window(path, window, cx),
            },
            // The view header's splits act on the group the header belongs
            // to, which the button made active before the menu opened.
            Pick::SplitRight => self.split(SplitDirection::Vertical, window, cx),
            Pick::SplitDown => self.split(SplitDirection::Horizontal, window, cx),
            // "Delete file" on the view header's menu: the same prompt the
            // Files view's Delete raises. The explorer's own menu is not
            // drawn here, so this is the one path that reaches it.
            Pick::Delete => {
                if !path.is_empty() {
                    self.start_delete(&path, window, cx)
                }
            }
            Pick::Rename => self.start_rename(&path, window, cx),
            Pick::Move => self.start_move(&path, window, cx),
            Pick::CopyPath => self.copy_path(&path, cx),
            Pick::OpenInDefaultApp => self.open_in_default_app(&path, cx),
            Pick::ShowInSystemExplorer => self.show_in_system_explorer(&path, cx),
            Pick::RevealInNavigation => self.reveal_in_navigation(&path, window, cx),
            // The editor menu's rows, which act on the editor rather than
            // on a path. The menu only offers them where a right click in
            // the text raised it, so the editor they reach is the one that
            // was clicked.
            Pick::AddLink => self
                .editor
                .update(cx, |e, cx| e.insert_link(LinkSyntax::Wikilink, window, cx)),
            Pick::AddExternalLink => self
                .editor
                .update(cx, |e, cx| e.insert_link(LinkSyntax::Markdown, window, cx)),
            // The text the row was labelled with, which is the selection as
            // it stood at the click -- not whatever is selected now, which
            // the click may have taken away.
            Pick::SearchForSelection => {
                if let file_menu::Target::Editor(click) = target {
                    let query = click.selection.clone();
                    self.show_search(&query, Some(window), cx);
                }
            }
            Pick::Cut => self.editor.update(cx, |e, cx| e.cut_selection(window, cx)),
            Pick::Copy => self.editor.update(cx, |e, cx| e.copy_selection(cx)),
            Pick::Paste => self
                .editor
                .update(cx, |e, cx| e.paste_clipboard(window, cx)),
            // The clipboard's plain text only: no HTML conversion, no files.
            Pick::PastePlain => self.editor.update(cx, |e, cx| e.paste_plain(window, cx)),
            Pick::SelectAll => self.editor.update(cx, |e, cx| e.select_all_text(cx)),
            // A misspelled word's rows.
            Pick::Correction(i) => {
                if let file_menu::Target::Editor(click) = target {
                    if let Some(hit) = &click.spelling {
                        if let Some(with) = hit.suggestions.get(i as usize) {
                            let (hit, with) = (hit.clone(), with.clone());
                            self.editor
                                .update(cx, |e, cx| e.replace_misspelling(&hit, &with, window, cx));
                        }
                    }
                }
            }
            Pick::AddToDictionary => {
                if let file_menu::Target::Editor(click) = target {
                    if let Some(hit) = &click.spelling {
                        crate::spellcheck::add_word(&hit.word, cx);
                    }
                }
            }
            // The Files view's own rows, which no menu drawn here lists.
            Pick::Duplicate
            | Pick::NewNote
            | Pick::NewFolder
            | Pick::NewCanvas
            | Pick::SearchInFolder => {}
        }
    }

    /// The open file menu, drawn over everything else at the click. It is
    /// the editor's and the reading view's menu as well as the panes', so
    /// it is drawn at the workspace's root rather than inside a dock.
    fn render_file_menu(
        &mut self,
        here: Option<u64>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.overlay_home() != here {
            return None;
        }
        let menu = self.file_menu.as_ref()?;
        let target = menu.target.clone();
        let focus = self.menu_focus.handle.clone();
        Some(file_menu::render(
            menu,
            theme,
            &focus,
            window.viewport_size().width,
            cx,
            |this, cx| this.close_file_menu(cx),
            move |this, pick, window, cx| this.file_menu_pick(pick, &target, window, cx),
            |this, open, cx| {
                if let Some(menu) = this.file_menu.as_mut() {
                    if menu.submenu != open {
                        menu.submenu = open;
                        cx.notify();
                    }
                }
            },
        ))
    }

    /// The window the modal or file menu that is up draws in, latching it
    /// on the first ask: the active group's window, which is where the keys
    /// or the click that raised it were. A window closed since reads as the
    /// vault window, so an overlay never goes undrawn. A window counts as
    /// there while it has a node and a handle: reading the handle would
    /// fail inside that window's own render, where its root is leased.
    fn overlay_home(&mut self) -> Option<u64> {
        if self.modal == Modal::None && self.file_menu.is_none() {
            self.overlay_window = None;
            return None;
        }
        let home = match self.overlay_window {
            Some(home) => home,
            None => {
                let home = self.window_of_group(self.active_group_id());
                self.overlay_window = Some(home);
                home
            }
        };
        home.filter(|id| self.popout_open(*id))
    }

    /// Whether popout `id` is open: it has a node and a handle. Reading
    /// the handle would fail inside that window's own render, where its
    /// root is leased, so the handle stands for the window.
    fn popout_open(&self, id: u64) -> bool {
        self.windows
            .iter()
            .any(|w| w.id == id && w.handle.is_some())
    }

    /// The notices window `here` draws, stacked in its bottom-right corner
    /// (`bottom` above the edge: the vault window clears its status bar).
    /// Obsidian reports an outcome with a Notice in the corner rather
    /// than in the status bar, and the probe notes put it bottom right.
    fn render_notices(
        &self,
        here: Option<u64>,
        bottom: Pixels,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let open = |id| self.popout_open(id);
        let mut shown = self.notices.in_window(here, &open).peekable();
        shown.peek()?;
        Some(
            div()
                .absolute()
                .bottom(bottom)
                .right(px(16.))
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .children(shown.map(|notice| {
                    div()
                        .max_w(px(360.))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme.tokens.background_secondary)
                        .border_1()
                        .border_color(theme.tokens.background_modifier_border)
                        .text_size(px(12.))
                        .text_color(theme.tokens.text_normal)
                        .child(notice.text.clone())
                        .children(notice.action.map(|action| {
                            let id = notice.id;
                            div()
                                .id(("notice-action", id))
                                .mt_1()
                                .flex()
                                .justify_end()
                                .cursor_pointer()
                                .text_color(theme.tokens.text_accent)
                                .hover(|s| s.text_color(theme.tokens.text_accent_hover))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.notices.expire(id);
                                    this.run_notice_action(action, window, cx);
                                }))
                                .child(action.label())
                        }))
                }))
                .into_any_element(),
        )
    }

    /// Hands the keyboard to an open menu, or back from a closed one, in
    /// window `here`: the file menu in the window it draws in, the panes'
    /// sort menus in the vault window, which alone has the panes. A window
    /// the menu is not in leaves the focus alone, or its "closed" would
    /// throw away the other window's place to go back to.
    fn sync_menu_focus(&mut self, here: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let home = self.overlay_home();
        let open = self.file_menu.is_some() && home == here;
        if open {
            self.menu_window = here;
            self.menu_focus.sync(true, window, cx);
        } else if self.menu_window == here {
            self.menu_focus.sync(false, window, cx);
        }
    }

    /// The modal that is up, centred near the top of window `here` when it
    /// is the window the modal belongs to.
    ///
    /// Every overlay occludes: a modal in Obsidian owns the pointer as
    /// well as the keyboard, and without this the editor's raw window
    /// listeners take the mouse-down that landed on the modal and focus
    /// the note behind it, leaving the modal visible but keyboard-dead.
    /// Occluding costs nothing a modal wants — a click outside still
    /// reaches each list modal's `on_mouse_down_out`, which is bounds
    /// based rather than hitbox based, and Settings closes on its own
    /// backdrop click.
    fn render_modal_overlay(&mut self, here: Option<u64>) -> Option<gpui::AnyElement> {
        if self.overlay_home() != here {
            return None;
        }
        match self.modal {
            Modal::None => None,
            Modal::Switcher
            | Modal::Prompt
            | Modal::Chooser
            | Modal::Downloads
            | Modal::CopyResults
            | Modal::DebugInfo
            | Modal::MigrateFrontmatter
            | Modal::PdfExport
            | Modal::ImageExport => {
                let child: gpui::AnyElement = match self.modal {
                    Modal::Switcher => self.switcher.clone().into_any_element(),
                    Modal::Chooser => self.chooser.clone().into_any_element(),
                    Modal::Downloads => self.downloads_element()?,
                    Modal::CopyResults => self.copy_results.clone().into_any_element(),
                    Modal::DebugInfo => self.debug_info.clone().into_any_element(),
                    Modal::MigrateFrontmatter => self.migrate_element()?,
                    Modal::PdfExport => self.pdf_export.clone().into_any_element(),
                    Modal::ImageExport => self.image_export.clone().into_any_element(),
                    _ => self.prompt.clone().into_any_element(),
                };
                Some(
                    div()
                        .occlude()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .items_start()
                        .pt(px(90.))
                        .child(child)
                        .into_any_element(),
                )
            }
        }
    }

    /// A pane's sections, drawn the way Obsidian's are: a section row with
    /// its count (no fold glyph; the row itself folds), a collapsible row
    /// per file with a triangle and its count, and the file's matches in
    /// one card with the matched text highlighted. The Links section's rows
    /// stand alone with a glyph for what they point at; the Outgoing pane's
    /// unlinked mentions are one card for the note itself, each match
    /// naming the note it mentions. A match wraps, so this is a scrolling
    /// column rather than a uniform list.
    fn left_pane_enabled(&self, id: &str) -> bool {
        self.state
            .as_ref()
            .is_none_or(|state| state.core_plugin_enabled(id))
    }
}

fn debug_status() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TFT_DEBUG_STATUS").is_some())
}

/// The `s` a count needs, for the notices that carry one.
fn vault_path_in(root: &Path, path: &str) -> Result<PathBuf, String> {
    let mut absolute = root.to_path_buf();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => absolute.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(format!("{path} leaves the vault")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{path} is not a vault-relative path"))
            }
        }
    }
    if absolute.as_path() == root {
        return Err(format!("{path} names the vault folder, not a file"));
    }
    let real_root = root
        .canonicalize()
        .map_err(|error| format!("{}: {error}", root.display()))?;
    let mut existing: &Path = absolute.as_path();
    let inside = loop {
        if existing.symlink_metadata().is_ok() {
            break match existing.canonicalize() {
                Ok(real) => real.starts_with(&real_root),
                // A dangling link has nothing to resolve; so has a path the
                // process may not look at. Neither can be shown to stay in
                // the vault, so neither is written through.
                Err(_) => false,
            };
        }
        match existing.parent() {
            // The walk stops at `root` at the latest: it canonicalized
            // above, so it exists.
            Some(parent) if parent.starts_with(root) => existing = parent,
            _ => break false,
        }
    };
    if !inside {
        return Err(format!("{path} leaves the vault"));
    }
    Ok(absolute)
}

/// Inserts `list` under `key`, capped to `rows`, with a sibling
/// `{key}_total`. The total is always there, `null` when the cap dropped
/// nothing, so a caller has one rule for every list in the dump.
fn put_capped(into: &mut Map<String, Value>, key: &str, list: Vec<Value>, rows: usize) {
    let total = list.len();
    into.insert(
        key.to_string(),
        Value::Array(list.into_iter().take(rows).collect()),
    );
    into.insert(
        format!("{key}_total"),
        if total > rows {
            json!(total)
        } else {
            Value::Null
        },
    );
}

/// An Outline row as drawn, with the offset the pane scrolls the note to.
pub(crate) fn context_names(window: &Window) -> Vec<String> {
    window
        .context_stack()
        .iter()
        .filter_map(|c| c.primary().map(|e| e.key.to_string()))
        .collect()
}

/// Removes `path`, and anything beneath it when it is a folder, from a
/// navigation history, answering where the one-based cursor lands. Entries
/// before the cursor carry it back with them, so back and forward still
/// reach the same notes they did.
fn prune_history(history: &mut Vec<HistoryEntry>, ix: usize, path: &str) -> usize {
    let prefix = format!("{path}/");
    let mut cursor = ix;
    let mut seen = 0;
    history.retain(|entry| {
        let keep = entry.path != path && !entry.path.starts_with(&prefix);
        if !keep && seen < ix {
            cursor -= 1;
        }
        seen += 1;
        keep
    });
    cursor.min(history.len())
}

/// A `tabs` node of `workspace.json` read into tabs, before any group
/// exists for it: each leaf's id with its tab, the `currentTab`, and the
/// node's own keys less `children` and `currentTab`, which are rewritten.
#[derive(Debug, Clone, PartialEq)]
struct GroupSpec {
    tabs: Vec<(String, Tab)>,
    active_tab: usize,
    node: Map<String, Value>,
}

/// `main` as read: the tree of splits over the groups, without the groups
/// existing yet. Pure, so the reading can be tested without a window.
#[derive(Debug, Clone, PartialEq)]
enum LayoutSpec {
    Group(GroupSpec),
    Split {
        direction: SplitDirection,
        children: Vec<LayoutSpec>,
        node: Map<String, Value>,
    },
}

/// A `window` node of `floating` as read: its groups as a split whose root
/// `node` map is empty, and the window's own keys beside it. Pure, like
/// [`LayoutSpec`].
#[derive(Debug, Clone, PartialEq)]
struct WindowSpec {
    tree: LayoutSpec,
    node: Map<String, Value>,
}

/// Reads `floating`: every `window` child, read as `main` is read -- a
/// window whose files are all gone is dropped with them -- with its own
/// keys less `children` and `direction`, which the tree carries. A child of
/// any other type is left out; Obsidian writes no other kind.
fn read_floating(
    floating: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Vec<WindowSpec> {
    floating
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|w| w.get("type").and_then(Value::as_str) == Some("window"))
        .filter_map(|w| {
            let object = w.as_object()?;
            let children = read_children(w, default_mode, known);
            if children.is_empty() {
                return None;
            }
            let direction = SplitDirection::parse(w.get("direction").and_then(Value::as_str));
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("direction");
            Some(WindowSpec {
                tree: LayoutSpec::Split {
                    direction,
                    children,
                    node: Map::new(),
                },
                node: own,
            })
        })
        .collect()
}

/// A split's (or a window's) children read, the ones with nothing left
/// dropped.
fn read_children(
    node: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Vec<LayoutSpec> {
    node.get("children")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter_map(|child| read_main(child, default_mode, known))
                .collect()
        })
        .unwrap_or_default()
}

/// Reads `main`. A `tabs` node with no leaf left (every file gone) is
/// dropped, and a split with no child left goes with its children; a split
/// with one child stays nested as Obsidian keeps it. `known` says whether a
/// leaf's file is still in the vault.
fn read_main(
    node: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Option<LayoutSpec> {
    let object = node.as_object()?;
    match node.get("type").and_then(Value::as_str) {
        Some("tabs") => {
            let tabs: Vec<(String, Tab)> = node
                .get("children")
                .and_then(Value::as_array)
                .map(|leaves| {
                    leaves
                        .iter()
                        .filter_map(|leaf| tab_from_leaf(leaf, default_mode, known))
                        .collect()
                })
                .unwrap_or_default();
            if tabs.is_empty() {
                return None;
            }
            let active_tab = node
                .get("currentTab")
                .and_then(Value::as_u64)
                .map(|t| t as usize)
                .unwrap_or(0)
                .min(tabs.len() - 1);
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("currentTab");
            Some(LayoutSpec::Group(GroupSpec {
                tabs,
                active_tab,
                node: own,
            }))
        }
        Some("split") => {
            let children = read_children(node, default_mode, known);
            if children.is_empty() {
                return None;
            }
            let direction = SplitDirection::parse(node.get("direction").and_then(Value::as_str));
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("direction");
            Some(LayoutSpec::Split {
                direction,
                children,
                node: own,
            })
        }
        _ => None,
    }
}

/// The tab a `leaf` node of `workspace.json` stands for, with the leaf's
/// id, or `None` for a view TFT has no tab for or a file that is gone.
fn tab_from_leaf(
    leaf: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Option<(String, Tab)> {
    let id = leaf.get("id").and_then(Value::as_str)?;
    let view = leaf
        .get("state")
        .and_then(|s| s.get("type"))
        .and_then(Value::as_str);
    let inner = leaf.get("state").and_then(|s| s.get("state"));
    let file = inner.and_then(|s| s.get("file")).and_then(Value::as_str);
    let pinned = leaf
        .get("state")
        .and_then(|s| s.get("pinned"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut tab = match (view, file) {
        (_, Some(file)) => {
            if !known(file) {
                return None;
            }
            let mode = match inner.and_then(|s| s.get("mode")).and_then(Value::as_str) {
                Some("preview") => ViewMode::Reading,
                Some("source")
                    if inner
                        .and_then(|s| s.get("source"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false) =>
                {
                    ViewMode::Source
                }
                Some("source") => ViewMode::LivePreview,
                _ => default_mode,
            };
            Tab {
                node: None,
                path: Some(file.to_string()),
                mode,
                history: vec![HistoryEntry::new(file.to_string(), mode)],
                history_ix: 1,
                leaf: Some(leaf.clone()),
                view_state: inner.and_then(|s| s.get("viewState")).cloned(),
                pinned: false,
            }
        }
        (Some("empty"), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(default_mode)
        },
        (Some("graph"), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(ViewMode::Graph)
        },
        (Some(release_notes::VIEW_TYPE), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(default_mode)
        },
        _ => return None,
    };
    tab.pinned = pinned;
    Some((id.to_string(), tab))
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The view type of the sidebar's current tab: the `tabs` group's
/// `currentTab` child (the first when the key is absent).
fn dock_current_view(layout: &Value, dock: &str) -> Option<String> {
    let mut groups = Vec::new();
    fn find_groups<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        if node.get("type").and_then(Value::as_str) == Some("tabs") {
            out.push(node);
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                find_groups(child, out);
            }
        }
    }
    find_groups(layout.get(dock)?, &mut groups);
    let group = groups.first()?;
    let ix = group.get("currentTab").and_then(Value::as_u64).unwrap_or(0) as usize;
    let leaf = group.get("children")?.as_array()?.get(ix)?;
    leaf.get("state")?.get("type")?.as_str().map(str::to_string)
}

fn default_dock_json(current: Option<&str>, tab: usize) -> Value {
    let leaf = |kind: &str, state: Value, icon: &str, title: &str| {
        let mut state = state;
        if current.is_none() {
            if let Some(map) = state.as_object_mut() {
                map.shift_remove("file");
            }
        }
        json!({
            "id": leaf_id(),
            "type": "leaf",
            "state": {"type": kind, "state": state, "icon": icon, "title": title},
        })
    };
    let children = vec![
        leaf(
            "file-explorer",
            json!({"sortOrder": "alphabetical", "autoReveal": false, "showSearch": false, "searchQuery": ""}),
            "lucide-folder-closed",
            "Files",
        ),
        leaf(
            "search",
            json!({"query": "", "matchingCase": false, "explainSearch": false, "collapseAll": false, "extraContext": false, "sortOrder": "alphabetical"}),
            "lucide-search",
            "Search",
        ),
    ];
    let tab = tab.min(children.len().saturating_sub(1));
    json!({
        "id": leaf_id(),
        "type": "split",
        "children": [{"id": leaf_id(), "type": "tabs", "children": children, "currentTab": tab}],
        "direction": "horizontal",
        "width": SIDEBAR_WIDTH as i64,
    })
}

/// The first leaf of view type `kind` anywhere under a dock's tree, as
/// the leaf node.
fn leaf_of_kind_mut<'a>(node: &'a mut Value, kind: &str) -> Option<&'a mut Map<String, Value>> {
    let is_leaf = node.get("type").and_then(Value::as_str) == Some("leaf")
        && node
            .get("state")
            .and_then(|s| s.get("type"))
            .and_then(Value::as_str)
            == Some(kind);
    if is_leaf {
        return node.as_object_mut();
    }
    let children = node.get_mut("children")?.as_array_mut()?;
    children.iter_mut().find_map(|c| leaf_of_kind_mut(c, kind))
}

fn leaf_of_kind<'a>(node: &'a Value, kind: &str) -> Option<&'a Map<String, Value>> {
    let is_leaf = node.get("type").and_then(Value::as_str) == Some("leaf")
        && node
            .get("state")
            .and_then(|s| s.get("type"))
            .and_then(Value::as_str)
            == Some(kind);
    if is_leaf {
        return node.as_object();
    }
    let children = node.get("children")?.as_array()?;
    children.iter().find_map(|c| leaf_of_kind(c, kind))
}

/// The `state.state` object of the first `kind` leaf under `dock`.
fn leaf_state<'a>(layout: &'a Value, dock: &str, kind: &str) -> Option<&'a Value> {
    leaf_of_kind(layout.get(dock)?, kind)?
        .get("state")?
        .get("state")
}

/// Writes `keys` into the state of the first `kind` leaf under `dock`, in
/// the place they had; a leaf without a state object gains one holding
/// only them, and a `null` value removes its key. A dock without the leaf
/// is left alone.
fn set_leaf_state(dock: &mut Value, kind: &str, keys: &[(&str, Value)]) {
    let Some(leaf) = leaf_of_kind_mut(dock, kind) else {
        return;
    };
    let Some(outer) = leaf.get_mut("state").and_then(Value::as_object_mut) else {
        return;
    };
    if !outer.get("state").is_some_and(Value::is_object) {
        outer.insert("state".into(), json!({}));
    }
    if let Some(state) = outer.get_mut("state").and_then(Value::as_object_mut) {
        for (key, value) in keys {
            if value.is_null() {
                state.remove(*key);
            } else {
                state.insert((*key).into(), value.clone());
            }
        }
    }
}

/// The `file-explorer` leaf anywhere under a dock's tree, as the leaf node.
fn explorer_leaf(node: &Value) -> Option<&Map<String, Value>> {
    leaf_of_kind(node, "file-explorer")
}

/// The Files leaf's `sortOrder` and `autoReveal` from `workspace.json`'s
/// left dock, each at Obsidian's default (`alphabetical`, off) when the key
/// is missing or unreadable; `None` when the dock has no such leaf.
fn explorer_leaf_state(layout: &Value) -> Option<(SortOrder, bool)> {
    let leaf = explorer_leaf(layout.get("left")?)?;
    let state = leaf.get("state").and_then(|s| s.get("state"));
    let sort = state
        .and_then(|s| s.get("sortOrder"))
        .and_then(Value::as_str)
        .and_then(SortOrder::parse)
        .unwrap_or_default();
    let auto_reveal = state
        .and_then(|s| s.get("autoReveal"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((sort, auto_reveal))
}

/// Writes the two keys into the Files leaf's state under `dock`, in the
/// place they had; a leaf without a state object gains one holding only
/// them. A dock without the leaf is left alone.
fn set_explorer_leaf_state(dock: &mut Value, sort: SortOrder, auto_reveal: bool) {
    set_leaf_state(
        dock,
        "file-explorer",
        &[
            ("sortOrder", json!(sort.as_str())),
            ("autoReveal", json!(auto_reveal)),
        ],
    );
}

/// A fresh leaf id in Obsidian's shape: sixteen hex digits.
fn leaf_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ (COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    format!("{x:016x}")
}

/// A `tabs` node over its leaves: the keys it was read with (or a fresh
/// `id`), `children` and `currentTab` rewritten at the end, the latter only
/// when it is not the first (as Obsidian leaves it out then). A group read
/// stacked comes back horizontal: `"stacked"` is dropped rather than kept.
fn tabs_node_json(mut map: Map<String, Value>, leaves: Vec<Value>, active_tab: usize) -> Value {
    if !map.contains_key("id") {
        map.shift_insert(0, "id".into(), json!(leaf_id()));
    }
    map.insert("type".into(), json!("tabs"));
    map.shift_remove("children");
    map.shift_remove("currentTab");
    map.shift_remove("stacked");
    map.insert("children".into(), Value::Array(leaves));
    if active_tab > 0 {
        map.insert("currentTab".into(), json!(active_tab));
    }
    Value::Object(map)
}

/// A `split` node over its children, `direction` after them as Obsidian
/// writes it.
fn split_node_json(
    mut map: Map<String, Value>,
    children: Vec<Value>,
    direction: SplitDirection,
) -> Value {
    if !map.contains_key("id") {
        map.shift_insert(0, "id".into(), json!(leaf_id()));
    }
    map.insert("type".into(), json!("split"));
    map.shift_remove("children");
    map.shift_remove("direction");
    map.insert("children".into(), Value::Array(children));
    map.insert("direction".into(), json!(direction.as_str()));
    Value::Object(map)
}

/// A `window` node of `floating`, in Obsidian's key order: `id`, `type`,
/// `children`, `direction`, then the rest of the window's own keys (the
/// bounds, `maximize`, `zoom`, anything unknown) as they came.
fn window_node_json(
    own: &Map<String, Value>,
    children: Vec<Value>,
    direction: SplitDirection,
) -> Value {
    let mut map = Map::new();
    map.insert(
        "id".into(),
        own.get("id").cloned().unwrap_or_else(|| json!(leaf_id())),
    );
    map.insert("type".into(), json!("window"));
    map.insert("children".into(), Value::Array(children));
    map.insert("direction".into(), json!(direction.as_str()));
    for (key, value) in own {
        if !matches!(key.as_str(), "id" | "type" | "children" | "direction") {
            map.insert(key.clone(), value.clone());
        }
    }
    Value::Object(map)
}

/// The `floating` split over the windows: the node the file had with its
/// children replaced, or a fresh one.
fn floating_json(existing: Option<&Map<String, Value>>, windows: Vec<Value>) -> Value {
    let mut map = existing.cloned().unwrap_or_default();
    if !map.contains_key("id") {
        map.shift_insert(0, "id".into(), json!(leaf_id()));
    }
    map.insert("type".into(), json!("floating"));
    map.shift_remove("children");
    map.insert("children".into(), Value::Array(windows));
    Value::Object(map)
}

/// The `workspace.json` leaf for a tab: the one it was restored from with
/// its state brought up to date, or a new one.
fn leaf_json(tab: &Tab) -> Value {
    let (mode, source) = match tab.mode {
        ViewMode::Reading => ("preview", false),
        ViewMode::Source => ("source", true),
        ViewMode::LivePreview | ViewMode::Graph => ("source", false),
    };
    let title = tab.title();
    let mut leaf = tab
        .leaf
        .clone()
        .filter(|l| {
            let file = l
                .get("state")
                .and_then(|s| s.get("state"))
                .and_then(|s| s.get("file"))
                .and_then(Value::as_str);
            let view = l
                .get("state")
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str);
            match (&tab.path, tab.mode) {
                (Some(p), _) => file == Some(p.as_str()),
                (None, ViewMode::Graph) => view == Some("graph"),
                (None, _) if release_notes::version_of(tab).is_some() => {
                    view == Some(release_notes::VIEW_TYPE)
                }
                (None, _) => view == Some("empty"),
            }
        })
        .unwrap_or_else(|| match (&tab.path, tab.mode) {
            (Some(p), _) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {
                    "type": leaf_view_type(p),
                    "state": {"file": p},
                    "icon": leaf_icon(p),
                    "title": title,
                },
            }),
            (None, ViewMode::Graph) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "graph", "state": {}, "icon": "lucide-git-fork", "title": title},
            }),
            (None, _) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "empty", "state": {}, "icon": "lucide-file", "title": title},
            }),
        });
    if let Some(path) = &tab.path {
        if leaf_view_type(path) == "markdown" {
            if let Some(inner) = leaf
                .get_mut("state")
                .and_then(|s| s.get_mut("state"))
                .and_then(Value::as_object_mut)
            {
                inner.insert("mode".into(), json!(mode));
                inner.insert("source".into(), json!(source));
            }
        }
        // A canvas leaf records where the view looked, when it is known; a
        // tab restored with a `viewState` and never moved keeps the one it
        // was read with.
        if leaf_view_type(path) == "canvas" {
            if let Some(view_state) = &tab.view_state {
                if let Some(inner) = leaf
                    .get_mut("state")
                    .and_then(|s| s.get_mut("state"))
                    .and_then(Value::as_object_mut)
                {
                    inner.insert("viewState".into(), view_state.clone());
                }
            }
        }
        if let Some(state) = leaf.get_mut("state").and_then(Value::as_object_mut) {
            state.insert("title".into(), json!(title));
        }
    }
    set_pinned(&mut leaf, tab.pinned);
    leaf
}

/// Obsidian writes a pinned leaf's `"pinned": true` twice — on the leaf
/// after `type`, and in its `state` after the inner `state` — and an
/// unpinned one without either key, never `false` (observed,
/// `tools/obsidian-probe/results/2026-11-09-tab-header-menu.json`). A key
/// already in place is left where it is.
fn set_pinned(leaf: &mut Value, pinned: bool) {
    fn put_after(map: &mut serde_json::Map<String, Value>, after: &str, pinned: bool) {
        if !pinned {
            map.shift_remove("pinned");
            return;
        }
        if map.contains_key("pinned") {
            map.insert("pinned".into(), json!(true));
            return;
        }
        let at = map
            .keys()
            .position(|k| k == after)
            .map_or(map.len(), |i| i + 1);
        map.shift_insert(at, "pinned".into(), json!(true));
    }
    let Some(outer) = leaf.as_object_mut() else {
        return;
    };
    put_after(outer, "type", pinned);
    if let Some(state) = outer.get_mut("state").and_then(Value::as_object_mut) {
        put_after(state, "state", pinned);
    }
}

/// The Lucide icon Obsidian records on a file's leaf: the dashboard for a
/// canvas, the text file for a PDF (observed:
/// `tools/obsidian-probe/results/2026-10-26-pdf.json`), the plain file
/// otherwise.
///
/// An audio file's is `lucide-file-audio` and a video's the plain file
/// (observed: `tools/obsidian-probe/results/2026-11-15-embeds-media.json`,
/// `s09`).
///
/// OBSERVE: what it writes for images and the rest.
fn leaf_icon(path: &str) -> &'static str {
    match FileKind::from_path(path) {
        FileKind::Canvas => "lucide-layout-dashboard",
        FileKind::Pdf => "lucide-file-text",
        FileKind::Audio => "lucide-file-audio",
        _ => "lucide-file",
    }
}

/// A file the main view shows as the embed `![[file]]` would draw it, in
/// every mode: an image, the audio player, TFT's video card. None has a
/// source to edit.
fn shown_as_embed(kind: FileKind) -> bool {
    matches!(kind, FileKind::Image | FileKind::Audio | FileKind::Video)
}

/// One foot of the active tab: the region just outside its bottom corner
/// that lies between the tab's edge, the strip's bottom and a quarter-circle
/// centred on the strip side, painted in the tab's colour. Only that region
/// is painted, so a neighbouring tab's hover wash shows through the curve.
/// What shows is the tab's edge curving down into the view header; the
/// outline hairline stops where the curve begins.
fn tab_foot(left: bool, theme: &Theme) -> impl IntoElement {
    let r = px(TAB_CURVE);
    let color = theme.tokens.background_primary;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let mut path = PathBuilder::fill();
            if left {
                // The arc is centred on the top-left corner: from the
                // bottom-left point round to the top-right one.
                path.move_to(point(o.x + r, o.y));
                path.line_to(point(o.x + r, o.y + r));
                path.line_to(point(o.x, o.y + r));
                path.arc_to(point(r, r), px(0.), false, false, point(o.x + r, o.y));
            } else {
                // Mirrored: the arc is centred on the top-right corner.
                path.move_to(point(o.x, o.y));
                path.line_to(point(o.x, o.y + r));
                path.line_to(point(o.x + r, o.y + r));
                path.arc_to(point(r, r), px(0.), false, true, point(o.x, o.y));
            }
            path.close();
            if let Ok(built) = path.build() {
                window.paint_path(built, color);
            }
        },
    )
    .absolute()
    .bottom_0()
    .w(r)
    .h(r)
    .map(|d| {
        if left {
            d.left(px(-TAB_CURVE))
        } else {
            d.right(px(-TAB_CURVE))
        }
    })
}

/// The tab strip's name for a file: a note's name without `.md`, and a
/// canvas without `.canvas`, a PDF without `.pdf` or an audio or video file
/// without its extension too (observed: `tone.mp3` is "tone", `clip.mp4`
/// "clip"), which is how Obsidian titles its tabs -- no view of its own ever
/// shows an extension (the PDF's is observed: `doc.pdf` is the tab "doc" and
/// the leaf's `title` "doc").
fn tab_display_name(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if (ext.eq_ignore_ascii_case("canvas")
                || ext.eq_ignore_ascii_case("pdf")
                || matches!(
                    FileKind::from_extension(&ext.to_ascii_lowercase()),
                    FileKind::Audio | FileKind::Video
                ))
                && !stem.is_empty() =>
        {
            stem
        }
        _ => display_name(file_name),
    }
}

/// What a leaf showing `path` in `mode` is called, which is the one string
/// the tab strip, the view header and the window title all show: the file's
/// display name, Obsidian's "New tab" for a leaf of type `empty` and
/// "Graph view" for the graph (see [[Specification#The empty tab]] -- both
/// the empty leaf's display text and its view-header title read "New tab",
/// not "No file is open").
fn leaf_title(path: Option<&str>, mode: ViewMode) -> SharedString {
    match (path, mode) {
        (Some(p), _) => tab_display_name(p.rsplit('/').next().unwrap_or(p))
            .to_string()
            .into(),
        (None, ViewMode::Graph) => "Graph view".into(),
        (None, _) => "New tab".into(),
    }
}

/// The leaf view type Obsidian gives a file of this kind.
fn leaf_view_type(path: &str) -> &'static str {
    match FileKind::from_path(path) {
        FileKind::Markdown => "markdown",
        FileKind::Image => "image",
        FileKind::Pdf => "pdf",
        FileKind::Canvas => "canvas",
        FileKind::Audio => "audio",
        FileKind::Video => "video",
        _ => "markdown",
    }
}

/// Obsidian's "Update links" modal, to the drag pass that caught it
/// (`tools/obsidian-probe/results/2026-10-25-explorer-dnd.json`, step
/// `s05`): the title, two lines that name neither the file nor the new name,
/// and three buttons in that order with no Cancel. The probe read the
/// second line as "This will affect 1 link in 1 file.", so Obsidian
/// pluralises rather than writing "link(s)".
fn update_links_prompt(files: usize, links: usize) -> PromptSpec {
    PromptSpec {
        title: "Update links".into(),
        detail: vec![
            "Do you want to update internal links that link to this file?".into(),
            format!(
                "This will affect {links} link{} in {files} file{}.",
                if links == 1 { "" } else { "s" },
                if files == 1 { "" } else { "s" }
            )
            .into(),
        ],
        input: None,
        buttons: vec![
            PromptButton::new("Always update", PromptAction::Tertiary),
            PromptButton::new("Just once", PromptAction::Submit),
            PromptButton::new("Do not update", PromptAction::Secondary),
        ],
        ..PromptSpec::default()
    }
}

/// Enter and Escape in the view header's title field.
impl inline_field::Host for Workspace {
    fn editing(&mut self) -> Option<&mut inline_field::Editing> {
        self.title_edit.as_mut().map(|edit| &mut edit.editing)
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_title(window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_title(window, cx);
    }
}

/// Typed, composed and pasted text while the view header's title is being
/// renamed in place; [`inline_field`] is the half the file tree's row
/// rename shares. The handler is registered by the title's own element
/// against the title's own focus handle (see [`Workspace::render_title`]),
/// so it only ever sees text meant for that field; with no field open every
/// method answers as an empty one would.
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        inline_field::text_for_range(
            &self.title_edit.as_ref()?.editing,
            range_utf16,
            adjusted_range,
        )
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(inline_field::selected_text_range(
            &self.title_edit.as_ref()?.editing,
        ))
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        inline_field::marked_text_range(&self.title_edit.as_ref()?.editing)
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(edit) = self.title_edit.as_mut() {
            edit.editing.marked = None;
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.title_edit.as_mut() else {
            return;
        };
        inline_field::replace_text_in_range(&mut edit.editing, range_utf16, new_text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.title_edit.as_mut() else {
            return;
        };
        inline_field::replace_and_mark_text_in_range(
            &mut edit.editing,
            range_utf16,
            new_text,
            new_selected_range,
        );
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let edit = self.title_edit.as_ref()?;
        let layout = self.title_layout.get(&edit.group)?;
        Some(inline_field::bounds_for_range(
            &edit.editing,
            layout,
            range_utf16,
            element_bounds,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.corners = WindowCorners::of(window);
        if let Some(value) = self.pending_title_rename.take() {
            cx.defer_in(window, move |this, window, cx| {
                this.rename_from_inline_title(&value, window, cx)
            });
        }
        for (editor, run) in std::mem::take(&mut self.pending_slash) {
            cx.defer_in(window, move |this, window, cx| match run {
                None => {
                    let commands = this.slash_commands(window, cx);
                    editor.update(cx, |e, cx| e.open_slash(commands, cx));
                }
                Some(id) => this.run_slash_command(&editor, &id, window, cx),
            });
        }
        for (editor, incoming) in std::mem::take(&mut self.pending_attachments) {
            cx.defer_in(window, move |this, window, cx| {
                this.save_incoming(&editor, incoming, window, cx)
            });
        }
        self.dock_tree_keyboard = self.dock_tree_focus.is_focused(window);
        let corners = self.corners;
        // The desktop's scheme: a vault on `theme: "system"` follows it.
        let system_dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        if system_dark != self.system_dark {
            self.system_dark = system_dark;
            self.reload_theme(cx);
        }
        // An open menu holds the keyboard, so Escape reaches it first; a
        // closed one hands the keyboard back where it was.
        self.sync_menu_focus(None, window, cx);
        if let Some(kind) = self.pending_canvas_pick.take() {
            self.open_canvas_file_chooser(kind, window, cx);
        }
        // A view's focus handle only joins the dispatch tree when the frame
        // that paints it is prepainted, so this — the start of that frame —
        // is the first moment a focus asked for elsewhere can land. A modal
        // owns the keyboard while it is up, and closing one focuses the main
        // area anyway, so the request is dropped rather than queued.
        // A popout's render does the same for an active group of its own
        // (see `render_popout`), so this window only acts on its own.
        if !self.active_group_in_popout()
            && std::mem::take(&mut self.refocus_main)
            && self.modal == Modal::None
            && self.keyboard_is_parked(window, cx)
        {
            self.focus_main(window, cx);
        }
        // A title edit belongs to the file whose header it opened on: if
        // that leaf moved on -- another tab, another file, the rename
        // itself -- the field goes with it rather than committing a name to
        // whatever is on screen now.
        if let Some(edit) = &self.title_edit {
            if self.group_current(edit.group) != Some(edit.path.as_str()) {
                self.title_edit = None;
                if self.title_focus.is_focused(window) {
                    self.focus_main(window, cx);
                }
            }
        }
        self.tab_drag_frame(window, cx);
        let theme = cx.global::<Theme>().clone();
        // The active leaf's name, which is the window title's first part:
        // "New tab" with nothing open, as Obsidian's reads
        // (`New tab - <vault> - Obsidian 1.14.5`).
        let title = self.window_leaf_title(&self.layout_tree);

        let main_body: gpui::AnyElement = if let Some(loading) = &self.loading {
            div()
                .p_4()
                .text_color(theme.tokens.text_muted)
                .child(loading.clone())
                .into_any_element()
        } else {
            let tree = self.layout_tree.clone();
            self.render_layout(&tree, &mut Vec::new(), &theme, cx)
        };

        let wanted_title: SharedString = format!("{title} - {} - TFT", self.vault_name).into();
        if self.window_title != wanted_title {
            window.set_window_title(&wanted_title);
            self.window_title = wanted_title;
        }

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(main_body);

        let frameless = frameless(window);
        self.frameless = frameless;
        self.fullscreen = window.is_fullscreen();
        let mac = self.mac_frame();
        // A pane whose plugin is off has no tab, so it cannot be the one
        // showing.
        for (tab, id) in [
            (LeftTab::Files, "file-explorer"),
            (LeftTab::Search, "global-search"),
        ] {
            if self.left_tab == tab && !self.left_pane_enabled(id) {
                self.left_tab = [
                    (LeftTab::Files, "file-explorer"),
                    (LeftTab::Search, "global-search"),
                ]
                .into_iter()
                .find(|(_, id)| self.left_pane_enabled(id))
                .map(|(t, _)| t)
                .unwrap_or(tab);
            }
        }
        let left_tab = self.left_tab;
        let left_body: gpui::AnyElement = match left_tab {
            LeftTab::Files => self.explorer.clone().into_any_element(),
            LeftTab::Search => self.search.clone().into_any_element(),
        };
        let left = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.left_width as f32))
            .h_full()
            .bg(theme.tokens.tab_container_background)
            .map(|d| corners.left().apply(d))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    // Past the traffic lights on macOS, where this header
                    // is the window's top-left corner.
                    .pl(px(8. + self.frame_left_space()))
                    .pr(px(8.))
                    .bg(theme.tokens.sidebar_tab_container_background)
                    .gap(px(3.))
                    .border_b_1()
                    .border_color(theme.tokens.tab_outline_color)
                    .children(self.left_pane_enabled("file-explorer").then(|| {
                        self.dock_tab(
                            "Files",
                            "folder-closed",
                            left_tab == LeftTab::Files,
                            &theme,
                            |this, window, cx| {
                                this.leave_search_field(window, cx);
                                this.left_tab = LeftTab::Files;
                                cx.notify();
                            },
                            cx,
                        )
                    }))
                    .children(self.left_pane_enabled("global-search").then(|| {
                        self.dock_tab(
                            "Search",
                            "search",
                            left_tab == LeftTab::Search,
                            &theme,
                            |this, window, cx| {
                                let q = this.search.read(cx).query(cx);
                                this.show_search(&q, Some(window), cx);
                            },
                            cx,
                        )
                    }))
                    .child(window_handle(
                        div().id("left-dock-space").flex_1().h_full(),
                        frameless,
                    ))
                    // The toggle sits at the right end, against the dock's
                    // edge.
                    .child(self.sidebar_toggle(&theme, cx)),
            )
            .child(div().flex_1().overflow_hidden().child(left_body))
            .child(self.render_vault_profile(&theme, window, cx))
            .child(self.dock_handle(true, &theme));

        // The modal and the file menu draw in the window they came up in:
        // this one, unless that was a popout.
        let overlay = self.render_modal_overlay(None);
        // The notices that went up while this window was the active one.
        let notices = self.render_notices(None, px(30.), &theme, cx);
        let preview = self.render_hover_preview(None, &theme, window, cx);

        let left_visible = self.left_visible;
        // A modal owns the keyboard while it is up, the way an Obsidian
        // modal's own keymap scope keeps app hotkeys from firing behind it.
        // gpui's key contexts nest rather than replace — the overlay is a
        // child of this div — so the workspace stands its handlers down
        // instead: without them a hotkey can no longer focus a pane behind
        // the modal, or swap the modal out from under its own Escape.
        let hotkeys_live = self.modal == Modal::None;
        // macOS has the desktop's traffic lights instead.
        let controls = (frameless && !mac).then(|| self.render_window_controls(&theme, window));

        let content = div()
            .key_context("Workspace")
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, _, cx| {
                this.preview_modifiers(e.modifiers, cx)
            }))
            .track_focus(&self.focus_handle)
            // A tab dragged out of this window: the moves and the
            // release reach this window only, so this is where the
            // other windows' zones are found and dropped on.
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<TabDrag>, window, cx| {
                this.cross_window_drag_moved(e.bounds, e.event.position, window, cx)
            }))
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.cross_window_drop(window, cx)),
            )
            .when(hotkeys_live, |d| {
                d.on_action(cx.listener(Self::toggle_mode))
                    .on_action(cx.listener(Self::go_back))
                    .on_action(cx.listener(Self::go_forward))
                    .on_action(cx.listener(Self::open_search_action))
                    .on_action(cx.listener(Self::open_graph_action))
                    .on_action(cx.listener(Self::run_command_action))
            })
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.background_primary)
            // This box paints the whole content square, over the frame's
            // rounding and under its children's; a corner left square here
            // is a notch of `theme.tokens.background_primary` outside every rounded child.
            .map(|d| corners.apply(d))
            .font_family(theme.ui_font.clone())
            .text_size(px(crate::theme::FONT_UI_MEDIUM))
            .text_color(theme.tokens.text_normal)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DockDrag>, _, cx| {
                        let left = e.drag(cx).left;
                        this.drag_dock(left, e.bounds, e.event.position, cx);
                    }))
                    .when(left_visible, |d| d.child(left))
                    .child(main),
            )
            .children(controls)
            .children(overlay)
            // One file menu for every view that raises it -- the Backlinks
            // and Outgoing links rows, the editor's text, a link in the
            // reading view -- positioned at the click and drawn over
            // everything else, so no dock's clipping can cut it off.
            .children(self.render_file_menu(None, &theme, window, cx))
            .children(notices)
            .children(preview);
        client_frame(content.into_any_element(), &theme, window)
    }
}

/// What `workspace:copy-full-path` ("Copy current file path from system
/// root") puts on the clipboard: the file's path from the root of the file
/// system, in the platform's own form. The vault root is kept as it was
/// opened, so a relative one is made absolute against the working folder
/// here; symbolic links are not resolved, since the path the vault was
/// opened by is the one its owner knows it by.
fn full_path_text(absolute: PathBuf) -> String {
    std::path::absolute(&absolute)
        .unwrap_or(absolute)
        .to_string_lossy()
        .into_owned()
}

/// The vault's own trash folder, at the vault root, where
/// `trashOption: "local"` moves a deleted file.
const TRASH_FOLDER: &str = ".trash";

/// The vault's name as `workspace:copy-url` writes it: its folder's name,
/// read from the absolute root so a vault opened as `.` (or as the empty
/// path, which is the working folder too) is still named. The last plain
/// component is the name: `absolute` keeps a trailing `.`.
fn vault_folder_name(root: &Path) -> String {
    let absolute = match root.as_os_str().is_empty() {
        true => std::env::current_dir(),
        false => std::path::absolute(root),
    }
    .unwrap_or_else(|_| root.to_path_buf());
    absolute
        .components()
        .rev()
        .find_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Obsidian's notice for a path that is not there, as "Show trash" showed
/// it with no `.trash` folder: the absolute path in curly quotes.
fn missing_file_notice(absolute: &str) -> String {
    format!("The file or folder \u{201c}{absolute}\u{201d} does not exist.")
}

/// Whether any of `leaves` is a Markdown leaf, which is what
/// `editor:focus` ("Focus on last note") is offered on: each is a tab's file
/// and whether the tab shows the graph instead. A canvas, a base, a PDF,
/// an image or an empty tab is not one (Obsidian 1.15.0: with an empty tab
/// and a canvas only, the palette did not list it).
fn any_markdown_leaf<'a>(mut leaves: impl Iterator<Item = (Option<&'a str>, bool)>) -> bool {
    leaves.any(|(path, graph)| {
        !graph && path.is_some_and(|p| FileKind::from_path(p) == FileKind::Markdown)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tab a link opens goes right after the active one, and with
    /// "Always focus new tabs" off it is a tab holding the file that has
    /// not been loaded, the active one left where it was (2026-11-15,
    /// `settings-gaps.mjs` s02: Alpha, Links (active), Beta became Alpha,
    /// Links, Target, Beta).
    #[test]
    fn a_tab_opened_from_a_link_goes_after_the_active_one() {
        let note = |p: &str| Tab::unloaded(p.to_string(), ViewMode::LivePreview);
        let mut tabs = vec![note("Alpha.md"), note("Links.md"), note("Beta.md")];
        let at = place_new_tab(&mut tabs, 1, note("Target.md"));
        assert_eq!(at, 2);
        let order: Vec<_> = tabs.iter().map(|t| t.path.clone().unwrap()).collect();
        assert_eq!(order, ["Alpha.md", "Links.md", "Target.md", "Beta.md"]);
        // At the end of the strip it is the last tab.
        assert_eq!(place_new_tab(&mut tabs, 3, note("End.md")), 4);
        // The background tab: the file, one entry to go back from.
        let t = Tab::unloaded("Target.md".into(), ViewMode::Reading);
        assert_eq!(t.path.as_deref(), Some("Target.md"));
        assert_eq!(t.history_ix, 1);
        assert_eq!(t.history[0].path, "Target.md");
        assert_eq!(t.mode, ViewMode::Reading);
        assert!(!t.pinned);
    }

    use crate::title_field::TitleField;
    use std::fs;
    use std::path::Path;

    /// A symlink to a folder or a file on either platform. Windows refuses
    /// to make one without Developer Mode or elevation, so a test that
    /// needs one skips that part when creation is denied there.
    fn symlink(target: &Path, link: &Path) -> Option<()> {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match made {
            Ok(()) => Some(()),
            Err(e) if cfg!(windows) => {
                eprintln!("symlinks are not permitted here, skipping: {e}");
                None
            }
            Err(e) => panic!("symlink {}: {e}", link.display()),
        }
    }

    fn vault() -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Sub")).unwrap();
        fs::create_dir_all(r.join(".obsidian")).unwrap();
        fs::write(r.join(".obsidian/app.json"), r#"{"trashOption":"local"}"#).unwrap();
        fs::write(r.join("A.md"), "# A\n\nSee [[B]] and [S](Sub/S.md).\n").unwrap();
        fs::write(
            r.join("B.md"),
            "\u{feff}---\r\nrelated: \"[[A]]\"\r\n---\r\n[[A]] and [[A#A|the a]] and `[[A]]`\r\n",
        )
        .unwrap();
        fs::write(r.join("Sub/S.md"), "[A](../A.md) [[B]]\n").unwrap();
        let state = VaultState::load(&r.to_path_buf()).unwrap();
        (dir, state)
    }

    #[test]
    fn the_node_graph_tracks_the_path_vault() {
        let (_dir, state) = vault();
        assert!(state.verify_data().is_ok());
        let node = state.data().lookup("Sub/S.md").unwrap();
        assert_eq!(state.data().path(node), Some("Sub/S.md"));
        state.create_note("New/N.md", "x").unwrap();
        assert!(state.verify_data().is_ok());
        let a = state.data().lookup("A.md").unwrap();
        state.rename("A.md", "C.md", false).unwrap();
        assert_eq!(state.data().lookup("C.md"), Ok(a), "a rename keeps NodeId");
        assert!(state.verify_data().is_ok());
        state.trash("C.md").unwrap();
        assert!(state.verify_data().is_ok());
    }

    /// `workspace:copy-full-path` copies the file from the file system's
    /// root, its folders joined with the platform's separator, while
    /// `workspace:copy-path` copies the vault-relative `Sub/S.md` as is.
    #[test]
    fn copy_full_path_is_the_vault_root_joined_with_the_vault_path() {
        let (dir, state) = vault();
        let text = full_path_text(state.vault().absolute("Sub/S.md"));
        let expected = dir.path().join("Sub").join("S.md");
        assert_eq!(text, expected.to_string_lossy());
        assert!(Path::new(&text).is_absolute());
        assert!(Path::new(&text).is_file());
    }

    /// A vault opened by a relative path still copies an absolute one, made
    /// against the working folder, as the command's name promises.
    #[test]
    fn copy_full_path_of_a_relatively_opened_vault_is_absolute() {
        let text = full_path_text(PathBuf::from("vault").join("Note.md"));
        let expected = std::env::current_dir()
            .unwrap()
            .join("vault")
            .join("Note.md");
        assert_eq!(text, expected.to_string_lossy());
    }

    /// The window cluster's commands the 2026-11-09 probe settled are
    /// runnable and are Obsidian's ids; the ones gpui blocks, the tab
    /// switcher Obsidian does not register on desktop, and the two whose
    /// screens report on Obsidian itself (its version, its changelog) are
    /// not.
    #[test]
    fn the_observed_app_and_window_commands_are_runnable() {
        let known = tft_commands::known_ids();
        for id in [
            "workspace:copy-path",
            "workspace:copy-full-path",
            "workspace:copy-url",
            "workspace:show-trash",
            "open-with-default-app:open",
            "open-with-default-app:show",
            "theme:toggle-light-dark",
        ] {
            assert!(RUNNABLE.contains(&id), "{id} runnable");
            assert!(known.contains(&id), "{id} is Obsidian's");
        }
        for id in [
            "window:zoom-in",
            "window:zoom-out",
            "window:reset-zoom",
            "window:toggle-always-on-top",
            "app:reload",
        ] {
            assert!(RUNNABLE.contains(&id), "{id} runnable");
            assert!(known.contains(&id), "{id} is Obsidian's");
        }
        // Not registered on a desktop (2026-11-15 probe).
        assert!(!RUNNABLE.contains(&"app:show-tab-switcher"));
        for id in ACTIVE_FILE_ONLY {
            assert!(RUNNABLE.contains(id), "{id} runnable");
        }
    }

    /// The URL's vault is the folder's name, a relatively opened vault's
    /// too.
    #[test]
    fn the_copied_urls_vault_is_the_folders_name() {
        assert_eq!(
            vault_folder_name(Path::new("/tmp/x/Probe Vault")),
            "Probe Vault"
        );
        let here = std::env::current_dir().unwrap();
        let name = here.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(vault_folder_name(Path::new(".")), name);
        assert_eq!(vault_folder_name(Path::new("")), name);
    }

    /// Show trash's notice with no `.trash` folder, the path in curly
    /// quotes, as Obsidian 1.15.0 wrote it.
    #[test]
    fn show_trash_names_the_missing_folder_in_curly_quotes() {
        assert_eq!(
            missing_file_notice("/tmp/claude-1000/appwin/main/Probe Vault/.trash"),
            "The file or folder \u{201c}/tmp/claude-1000/appwin/main/Probe Vault/.trash\u{201d} does not exist."
        );
        assert_eq!(TRASH_FOLDER, ".trash");
    }

    /// Focus on last note is offered while any tab is a note: an empty tab
    /// and a canvas alone do not count, nor do an image, a PDF, a base or a
    /// tab showing the graph.
    #[test]
    fn focus_on_last_note_is_offered_only_with_a_markdown_leaf() {
        assert!(!any_markdown_leaf(
            [(None, false), (Some("Board.canvas"), false)].into_iter()
        ));
        assert!(!any_markdown_leaf(
            [
                (Some("img/pic.png"), false),
                (Some("Paper.pdf"), false),
                (Some("Note.md"), true),
            ]
            .into_iter()
        ));
        assert!(any_markdown_leaf(
            [(Some("Board.canvas"), false), (Some("Note A.md"), false)].into_iter()
        ));
        assert!(!any_markdown_leaf(std::iter::empty()));
    }

    fn history(entries: &[&str]) -> Vec<HistoryEntry> {
        entries
            .iter()
            .map(|e| HistoryEntry::new(e.to_string(), ViewMode::LivePreview))
            .collect()
    }

    /// `can_run` checks `RUNNABLE` first, so an id only in `EDITOR_ONLY`
    /// would be filtered twice over and never offered at all.
    #[test]
    fn every_editor_only_id_is_runnable() {
        for id in EDITOR_ONLY {
            assert!(
                RUNNABLE.contains(id),
                "{id} is editor-only but not runnable"
            );
        }
        // The table commands have their own rule in `can_run` and must not
        // be in the editor-only list as well, or the narrower check never
        // runs.
        for id in EDITOR_ONLY {
            assert!(
                TableCommand::from_id(id).is_none(),
                "{id} is both a table command and editor-only"
            );
        }
    }

    /// Neither list may name an id the registry does not have: a typo here
    /// is a command that silently never appears.
    #[test]
    fn both_lists_name_real_commands() {
        let registry = CommandRegistry::with_known_ids();
        for id in RUNNABLE.iter().chain(EDITOR_ONLY) {
            assert!(registry.get(id).is_some(), "{id} is not a known command");
        }
    }

    /// The converse of [`every_editor_only_id_is_runnable`], and the one
    /// that was missing: an id `run_command` answers but `RUNNABLE` does not
    /// name is a command that works and nothing can reach. `can_run` tests
    /// `RUNNABLE` first, so the palette leaves the id out, the socket
    /// reports it as not runnable, `bind_keys` never binds its hotkey and
    /// `run_command` does not even count it as recent. "Open help" sat that
    /// way with F1 in `commands` and no key bound to it.
    ///
    /// The ids are read off the match itself rather than listed again here,
    /// because a list kept by hand is what failed the first time.
    #[test]
    fn every_handled_id_is_runnable() {
        const SOURCE: &str = include_str!("workspace.rs");
        let body = SOURCE
            .split_once("pub fn run_command(")
            .expect("run_command is declared in this file")
            .1;
        let mut handled: Vec<String> = Vec::new();
        // An arm's pattern is the lines at the match's own indentation that
        // start with an id or carry on an `|` chain, up to the `=>`: a long
        // chain is wrapped over several lines, and a guard (`if ...`) sits
        // between the last id and the arrow. An arm's body is indented
        // past this, so a nested match and a string literal in a call are
        // both out of reach.
        let mut pattern = String::new();
        for line in body.lines() {
            if line.starts_with("            _ =>") {
                break;
            }
            if !(line.starts_with("            \"") || line.starts_with("            | \"")) {
                continue;
            }
            pattern.push_str(line);
            let Some((ids, _)) = pattern.split_once("=>") else {
                continue;
            };
            // Every id is quoted, so the odd pieces of a split on the quote
            // are the ids and the even ones are the `|`s and the guard.
            handled.extend(ids.split('"').skip(1).step_by(2).map(|id| id.to_string()));
            pattern.clear();
        }
        // A floor, so that a change to the match's shape shows up as a
        // failure here rather than as a test that quietly checks nothing.
        assert!(
            handled.len() >= 95,
            "only {} arms found; the reader above has stopped matching \
             run_command's match",
            handled.len()
        );
        for id in &handled {
            assert!(
                RUNNABLE.contains(&id.as_str()),
                "{id} is handled by run_command but not in RUNNABLE, so \
                 nothing can reach it"
            );
        }
    }

    /// A deleted note leaves the history, so back and forward cannot land
    /// on a file the vault no longer has. Entries before the cursor take it
    /// back with them; the notes it can still reach do not change.
    fn leaf(id: &str, file: &str, mode: &str, source: bool) -> Value {
        // A pinned leaf, in the shape Obsidian writes one (the
        // tab-header probe's `pinnedWorkspace`).
        json!({
            "id": id,
            "type": "leaf",
            "pinned": true,
            "state": {
                "type": "markdown",
                "state": {"file": file, "mode": mode, "source": source},
                "pinned": true,
                "icon": "lucide-file",
                "title": file.rsplit('/').next().unwrap().trim_end_matches(".md"),
            },
        })
    }

    #[test]
    fn a_restored_leaf_is_written_back_as_it_was_read() {
        let read = leaf("a1072f377b3d1c69", "Sub/Note.md", "source", false);
        let tab = Tab {
            node: None,
            path: Some("Sub/Note.md".into()),
            mode: ViewMode::LivePreview,
            history: vec![HistoryEntry::new(
                "Sub/Note.md".into(),
                ViewMode::LivePreview,
            )],
            history_ix: 1,
            leaf: Some(read.clone()),
            view_state: None,
            pinned: true,
        };
        assert_eq!(leaf_json(&tab), read);
    }

    #[test]
    fn a_mode_change_rewrites_only_the_mode_keys() {
        let read = leaf("a1072f377b3d1c69", "Note.md", "source", false);
        let tab = Tab {
            node: None,
            path: Some("Note.md".into()),
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            leaf: Some(read.clone()),
            view_state: None,
            pinned: true,
        };
        let written = leaf_json(&tab);
        assert_eq!(written["id"], read["id"]);
        assert_eq!(written["state"]["pinned"], json!(true));
        assert_eq!(written["state"]["state"]["mode"], json!("preview"));
        assert_eq!(written["state"]["state"]["source"], json!(false));
        let tab = Tab {
            mode: ViewMode::Source,
            ..tab
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["state"]["mode"], json!("source"));
        assert_eq!(written["state"]["state"]["source"], json!(true));
    }

    /// A canvas tab is written as Obsidian's canvas leaf: its view type and
    /// icon, the title without the extension, and the `viewState` the view
    /// reported; a leaf restored with one and never moved keeps it.
    #[test]
    fn a_canvas_tab_is_written_as_a_canvas_leaf() {
        let tab = Tab {
            node: None,
            path: Some("Plans/Board.canvas".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: Some(json!({"x": 100, "y": -20, "zoom": -1})),
            pinned: false,
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["type"], json!("canvas"));
        assert_eq!(written["state"]["icon"], json!("lucide-layout-dashboard"));
        assert_eq!(written["state"]["title"], json!("Board"));
        assert_eq!(
            written["state"]["state"],
            json!({"file": "Plans/Board.canvas", "viewState": {"x": 100, "y": -20, "zoom": -1}})
        );
        assert!(written["state"]["state"].get("mode").is_none());
        let known = |_: &str| true;
        let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
        assert_eq!(restored.view_state, tab.view_state);
        assert_eq!(leaf_json(&restored), written);
    }

    /// Audio and video tabs are Obsidian's `audio` and `video` leaves:
    /// observed, `{"type":"audio","state":{"file":"tone.mp3"},"icon":
    /// "lucide-file-audio","title":"tone"}` and the video's with the plain
    /// file icon (`2026-11-15-embeds-media.json`, `s09`).
    #[test]
    fn audio_and_video_tabs_are_written_as_obsidian_writes_them() {
        let tab = |path: &str| Tab {
            node: None,
            path: Some(path.into()),
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
            pinned: false,
        };
        let audio = leaf_json(&tab("tone.mp3"));
        assert_eq!(audio["state"]["type"], json!("audio"));
        assert_eq!(audio["state"]["icon"], json!("lucide-file-audio"));
        assert_eq!(audio["state"]["title"], json!("tone"));
        assert_eq!(audio["state"]["state"], json!({"file": "tone.mp3"}));
        let video = leaf_json(&tab("Clips/clip.MP4"));
        assert_eq!(video["state"]["type"], json!("video"));
        assert_eq!(video["state"]["icon"], json!("lucide-file"));
        assert_eq!(video["state"]["title"], json!("clip"));
        assert!(shown_as_embed(FileKind::Audio) && shown_as_embed(FileKind::Video));
        assert!(!shown_as_embed(FileKind::Pdf));
    }

    /// A PDF tab is written as Obsidian's `pdf` leaf, which is the file
    /// alone: observed, `state.state` is `{"file": ..}` before and after
    /// scrolling, zooming and a quit, the icon is `lucide-file-text` and
    /// the title drops the extension. Where the view is in the file never
    /// reaches the leaf, and a leaf read with extra keys is written back as
    /// read.
    #[test]
    fn a_pdf_tab_is_written_as_a_file_only_pdf_leaf() {
        let tab = Tab {
            node: None,
            path: Some("Papers/Field.pdf".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
            pinned: false,
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["type"], json!("pdf"));
        assert_eq!(written["state"]["icon"], json!("lucide-file-text"));
        assert_eq!(written["state"]["title"], json!("Field"));
        assert_eq!(
            written["state"]["state"],
            json!({"file": "Papers/Field.pdf"})
        );
        assert!(written["state"]["state"].get("mode").is_none());
        assert_eq!(tab.title(), "Field", "the strip shows no extension either");
        assert_eq!(tab_display_name("Field.PDF"), "Field");
        let known = |_: &str| true;
        let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
        assert_eq!(restored.view_state, None);
        assert_eq!(leaf_json(&restored), written);
        // A leaf somebody else wrote with keys of its own keeps them.
        let read = json!({
            "id": "0f6c3a9d2b1e4c5a",
            "type": "leaf",
            "state": {
                "type": "pdf",
                "state": {"file": "Papers/Field.pdf", "mystery": true},
                "icon": "lucide-file-text",
                "title": "Field"
            }
        });
        let (_, restored) = tab_from_leaf(&read, ViewMode::Reading, &known).unwrap();
        assert_eq!(leaf_json(&restored), read, "never moved: written as read");
    }

    /// Pinning writes `pinned` twice, in Obsidian's places, byte for byte
    /// against the probe's file; unpinning takes both keys out.
    #[test]
    fn a_pinned_leaf_is_written_in_obsidians_shape() {
        let probed = r#"{
  "id": "726a32fa984bda69",
  "type": "leaf",
  "pinned": true,
  "state": {
    "type": "markdown",
    "state": {
      "file": "Alpha.md",
      "mode": "source",
      "source": false
    },
    "pinned": true,
    "icon": "lucide-file",
    "title": "Alpha"
  }
}"#;
        let read: Value = serde_json::from_str(probed).unwrap();
        let known = |_: &str| true;
        let (_, mut tab) = tab_from_leaf(&read, ViewMode::LivePreview, &known).unwrap();
        assert!(tab.pinned);
        tab.pinned = false;
        let unpinned = leaf_json(&tab);
        assert!(unpinned.get("pinned").is_none());
        assert!(unpinned["state"].get("pinned").is_none());
        tab.leaf = Some(unpinned);
        tab.pinned = true;
        let pinned = serde_json::to_string_pretty(&leaf_json(&tab)).unwrap();
        assert_eq!(pinned, probed);
    }

    #[test]
    fn a_tab_that_moved_to_another_note_gets_a_fresh_leaf() {
        let read = leaf("a1072f377b3d1c69", "Old.md", "source", false);
        let tab = Tab {
            node: None,
            path: Some("New.md".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: Some(read),
            view_state: None,
            pinned: false,
        };
        let written = leaf_json(&tab);
        let id = written["id"].as_str().unwrap();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id, "a1072f377b3d1c69");
        assert_eq!(written["state"]["type"], json!("markdown"));
        assert_eq!(written["state"]["state"]["file"], json!("New.md"));
        assert_eq!(written["state"]["title"], json!("New"));
        assert!(written["state"].get("pinned").is_none());
    }

    #[test]
    fn an_empty_tab_and_a_graph_tab_write_their_own_leaf_types() {
        let empty = leaf_json(&Tab::empty(ViewMode::LivePreview));
        assert_eq!(empty["state"]["type"], json!("empty"));
        assert_eq!(empty["state"]["title"], json!("New tab"));
        let graph = leaf_json(&Tab::empty(ViewMode::Graph));
        assert_eq!(graph["state"]["type"], json!("graph"));
        assert_eq!(graph["state"]["icon"], json!("lucide-git-fork"));
    }

    /// An empty leaf is called "New tab" wherever its name is shown: the
    /// tab strip, the view header and the window title all ask for the one
    /// string, so none of them can read "No file is open" (see
    /// [[Specification#The empty tab]]).
    #[test]
    fn an_empty_leaf_is_called_new_tab_wherever_its_name_is_shown() {
        for mode in [
            ViewMode::Reading,
            ViewMode::LivePreview,
            ViewMode::Source,
            ViewMode::Reading,
        ] {
            assert_eq!(leaf_title(None, mode), "New tab");
            assert_eq!(Tab::empty(mode).title(), "New tab");
        }
        assert_eq!(leaf_title(None, ViewMode::Graph), "Graph view");
        assert_eq!(
            leaf_title(Some("Folder/Note.md"), ViewMode::LivePreview),
            "Note"
        );
    }

    /// The empty tab's four pills, in Obsidian's order and with the text it
    /// draws on them: the hotkeys come from the registry, so `New note` and
    /// `Go to file` carry the chords their commands are bound to and the
    /// other two carry none.
    #[test]
    fn the_empty_tab_offers_obsidians_four_pills() {
        let registry = CommandRegistry::with_known_ids();
        let actions = empty_tab_actions(&registry);
        let mod_key = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let texts: Vec<String> = actions.iter().map(EmptyAction::text).collect();
        assert_eq!(
            texts,
            [
                format!("New note {mod_key} + N"),
                "New canvas".to_string(),
                format!("Go to file {mod_key} + O"),
            ]
        );
        let commands: Vec<&str> = actions.iter().map(|a| a.command).collect();
        assert_eq!(
            commands,
            ["file-explorer:new-file", "canvas:new-file", "switcher:open"]
        );
        // Each pill runs its command, so each has to be one the workspace
        // will actually run.
        for action in &actions {
            assert!(
                RUNNABLE.contains(&action.command),
                "{} is not runnable",
                action.command
            );
        }
    }

    /// A pill whose command the registry does not have is not drawn: the
    /// registry holds the commands of the core plugins that are on, so
    /// turning Canvas off takes "New canvas" with it rather than leaving a
    /// pill that does nothing.
    #[test]
    fn a_pill_for_a_disabled_plugin_is_left_out() {
        let mut registry = CommandRegistry::with_known_ids();
        registry.retain_enabled(|plugin| plugin != "canvas");
        let labels: Vec<&str> = empty_tab_actions(&registry)
            .iter()
            .map(|a| a.label)
            .collect();
        assert_eq!(labels, ["New note", "Go to file"]);
    }

    /// The status bar is not hidden on an empty tab: with one active
    /// Obsidian's reads `0 properties`. A canvas and a base show nothing,
    /// and a note shows its caret.
    #[test]
    fn an_empty_tabs_status_bar_reads_zero_rather_than_nothing() {
        assert_eq!(Workspace::status_items(None, ""), ["0 properties"]);
        assert!(Workspace::status_items(Some("Board.canvas"), "").is_empty());
        assert_eq!(
            Workspace::status_items(Some("Note.md"), "Ln 1, Col 1  ·  "),
            ["Ln 1, Col 1  ·  "]
        );
    }

    /// The empty tab's header carries the two navigation buttons and no
    /// mode switch: there is nothing to read or edit. The graph keeps its
    /// own toggle instead, as it already did.
    #[test]
    fn the_empty_tabs_header_has_no_mode_switch() {
        for mode in [
            ViewMode::Reading,
            ViewMode::LivePreview,
            ViewMode::Source,
            ViewMode::Graph,
        ] {
            assert!(
                !header_shows_mode_switch(&None, mode),
                "an empty tab in {mode:?} must not offer the mode switch"
            );
        }
        assert!(header_shows_mode_switch(
            &Some("Note.md".to_string()),
            ViewMode::Reading
        ));
        assert!(!header_shows_mode_switch(
            &Some("Note.md".to_string()),
            ViewMode::Graph
        ));
    }

    /// Reaching the empty tab by closing every tab gives the same view as
    /// a fresh vault's: [`Workspace::close_tab`] pushes a [`Tab::empty`] in
    /// the default mode, and every one of those is the same leaf -- no
    /// file, the "New tab" name, the empty main view and the zeroed status
    /// bar -- however the window got there.
    #[test]
    fn closing_every_tab_leaves_the_tab_a_fresh_vault_opens_with() {
        let fresh = Tab::empty(ViewMode::Reading);
        for mode in [ViewMode::LivePreview, ViewMode::Source, ViewMode::Reading] {
            let closed = Tab::empty(mode);
            assert_eq!(closed.path, fresh.path);
            assert_eq!(closed.title(), fresh.title());
            assert_eq!(
                Workspace::main_view_for(closed.mode, false, false, false, false, false),
                MainView::Empty
            );
            assert_eq!(
                Workspace::status_items(closed.path.as_deref(), ""),
                Workspace::status_items(fresh.path.as_deref(), "")
            );
            assert!(!header_shows_mode_switch(&closed.path, closed.mode));
        }
    }

    /// Enter in the view-header title renames the file the field opened on,
    /// through the path the rename prompt's box commits through: the field
    /// holds the stem, so the extension goes back on and the name is
    /// checked before the filesystem is asked for it. The gesture the probe
    /// recorded: `Props` clicked at offset 3 and typed `X` reads `ProXps`
    /// while the file is still `Props.md`, and Enter asks for `ProXps.md`.
    #[test]
    fn the_header_titles_commit_asks_for_the_path_the_rename_prompt_would() {
        let mut field = TitleField::new("Props");
        field.click(3, 1, false);
        field.insert("X");
        assert_eq!(
            field.text(),
            "ProXps",
            "the title changed, the file has not"
        );
        assert_eq!(
            renamed_path("Props.md", field.text().trim(), false).as_deref(),
            Ok("ProXps.md")
        );
        assert_eq!(
            renamed_path("Folder/Props.md", "New name", false).as_deref(),
            Ok("Folder/New name.md")
        );
        assert_eq!(
            renamed_path("Folder/Board.canvas", "Board 2", false).as_deref(),
            Ok("Folder/Board 2.canvas")
        );
        // A folder has no extension to put back.
        assert_eq!(
            renamed_path("Folder", "Renamed", true).as_deref(),
            Ok("Renamed")
        );
        // And a name the filesystem would refuse never gets there: the
        // field stays open on the error instead.
        assert!(renamed_path("Props.md", "", false).is_err());
        assert!(renamed_path("Props.md", "a/b", false).is_err());
        assert!(renamed_path("Props.md", &"x".repeat(300), false).is_err());
    }

    /// Obsidian's `main` after "Split right" on A, then B opened in the new
    /// group and "Split down" there, as observed on 2026-10-20: a vertical
    /// root over a `tabs` group and a nested horizontal split.
    fn nested_main() -> Value {
        json!({
            "id": "449f821d63addc06",
            "type": "split",
            "children": [
                {"id": "3ab610226e243404", "type": "tabs", "dimension": 29.411764705882355,
                 "children": [leaf("3c245a8fc7dc4598", "A.md", "source", false)]},
                {"id": "0fa61223ab358f4f", "type": "split", "dimension": 70.58823529411765,
                 "children": [
                    {"id": "6c9c4444fe9200bb", "type": "tabs",
                     "children": [leaf("f0559f5cf8f3ac65", "B.md", "source", false)]},
                    {"id": "189916eb401608c6", "type": "tabs", "currentTab": 1,
                     "children": [
                        leaf("747a588e39ce11fa", "B.md", "source", false),
                        leaf("52ae6ba5c0105abd", "C.md", "preview", false),
                     ]},
                 ],
                 "direction": "horizontal"},
            ],
            "direction": "vertical",
        })
    }

    #[test]
    fn main_is_read_as_a_tree_of_groups_with_their_own_keys() {
        let spec = read_main(&nested_main(), ViewMode::LivePreview, &|_| true).unwrap();
        let LayoutSpec::Split {
            direction,
            children,
            node,
        } = spec
        else {
            panic!("the root is a split");
        };
        assert_eq!(direction, SplitDirection::Vertical);
        assert_eq!(node.get("id"), Some(&json!("449f821d63addc06")));
        assert!(!node.contains_key("children") && !node.contains_key("direction"));
        assert_eq!(children.len(), 2);
        let LayoutSpec::Group(left) = &children[0] else {
            panic!("the first child is a group");
        };
        assert_eq!(left.tabs.len(), 1);
        assert_eq!(left.tabs[0].0, "3c245a8fc7dc4598");
        assert_eq!(left.tabs[0].1.mode, ViewMode::LivePreview);
        assert_eq!(left.node.get("dimension"), Some(&json!(29.411764705882355)));
        let LayoutSpec::Split {
            direction,
            children,
            ..
        } = &children[1]
        else {
            panic!("the second child is a split");
        };
        assert_eq!(*direction, SplitDirection::Horizontal);
        let LayoutSpec::Group(bottom) = &children[1] else {
            panic!("a group");
        };
        assert_eq!(bottom.active_tab, 1);
        assert_eq!(bottom.tabs[1].1.mode, ViewMode::Reading);
    }

    #[test]
    fn a_group_whose_files_are_all_gone_is_dropped_with_its_empty_split() {
        let spec = read_main(&nested_main(), ViewMode::LivePreview, &|f| f == "A.md").unwrap();
        let LayoutSpec::Split { children, .. } = spec else {
            panic!("the root is a split");
        };
        assert_eq!(children.len(), 1, "the horizontal split had nothing left");
        assert!(read_main(&nested_main(), ViewMode::LivePreview, &|_| false).is_none());
    }

    /// Obsidian 1.15.0's `workspace.json` with four popouts open, as the
    /// probe recorded it (`tools/obsidian-probe/results/2026-11-09-popout-windows.json`,
    /// `s04-new-window`): the vault window holding one empty tab, a window
    /// each on Beta, Alpha and Gamma, and a fourth on an empty tab, which
    /// is the active leaf.
    const FOUR_POPOUTS: &str = include_str!("../tests/fixtures/workspace-four-popouts.json");

    /// A `LayoutSpec` written back the way the workspace writes its groups,
    /// so the reading and the writing can be checked without a window.
    fn spec_json(spec: &LayoutSpec) -> Value {
        match spec {
            LayoutSpec::Group(group) => tabs_node_json(
                group.node.clone(),
                group.tabs.iter().map(|(_, t)| leaf_json(t)).collect(),
                group.active_tab,
            ),
            LayoutSpec::Split {
                direction,
                children,
                node,
            } => split_node_json(
                node.clone(),
                children.iter().map(spec_json).collect(),
                *direction,
            ),
        }
    }

    fn window_spec_json(w: &WindowSpec) -> Value {
        let LayoutSpec::Split {
            direction,
            children,
            ..
        } = &w.tree
        else {
            panic!("a window's root is a split");
        };
        window_node_json(
            &w.node,
            children.iter().map(spec_json).collect(),
            *direction,
        )
    }

    #[test]
    fn floating_is_read_as_windows_over_groups_with_their_bounds() {
        let layout: Value = serde_json::from_str(FOUR_POPOUTS).unwrap();
        let windows = read_floating(&layout["floating"], ViewMode::LivePreview, &|_| true);
        assert_eq!(windows.len(), 4);
        let first = &windows[0];
        assert_eq!(first.node.get("id"), Some(&json!("fbe32d82ccdfaec7")));
        assert_eq!(first.node.get("type"), Some(&json!("window")));
        assert!(!first.node.contains_key("children") && !first.node.contains_key("direction"));
        assert_eq!(
            first.node.keys().collect::<Vec<_>>(),
            ["id", "type", "x", "y", "width", "height", "maximize", "zoom"]
        );
        let LayoutSpec::Split {
            direction,
            children,
            node,
        } = &first.tree
        else {
            panic!("a window's root is a split");
        };
        assert_eq!(*direction, SplitDirection::Vertical);
        assert!(node.is_empty(), "the window's keys live on the window");
        let LayoutSpec::Group(group) = &children[0] else {
            panic!("one group");
        };
        assert_eq!(group.tabs[0].1.path.as_deref(), Some("Beta.md"));
        assert_eq!(group.tabs[0].1.mode, ViewMode::LivePreview);
        // The recorded bounds come through the node the workspace keeps.
        let node = WindowNode::from_node(
            1,
            Layout::split(SplitDirection::Vertical, vec![Layout::Group(1)]),
            first.node.clone(),
        );
        assert_eq!(
            node.bounds(),
            Some(WindowBounds {
                x: 16,
                y: 10,
                width: 980,
                height: 759
            })
        );
        // The fourth window holds an empty tab, which is a tab here too.
        let LayoutSpec::Split { children, .. } = &windows[3].tree else {
            panic!("a split");
        };
        let LayoutSpec::Group(group) = &children[0] else {
            panic!("one group");
        };
        assert_eq!(group.tabs[0].1.path, None);
        // No `floating` key, no windows; and a `floating` with none is none.
        assert!(read_floating(
            &json!({"id": "x", "type": "floating", "children": []}),
            ViewMode::LivePreview,
            &|_| true
        )
        .is_empty());
    }

    #[test]
    fn a_window_whose_files_are_all_gone_is_dropped() {
        let layout: Value = serde_json::from_str(FOUR_POPOUTS).unwrap();
        let windows = read_floating(&layout["floating"], ViewMode::LivePreview, &|f| {
            f != "Gamma.md"
        });
        assert_eq!(
            windows.len(),
            3,
            "Gamma's window went; the empty tab's stays"
        );
        assert_eq!(windows[2].node.get("id"), Some(&json!("b3bc438a3f884b7d")));
    }

    /// The Writer gate's rule for the whole file, applied to `floating`:
    /// read Obsidian's bytes into the model and write them back, and
    /// nothing moves. `main` goes through the same writer, so the test
    /// covers the file as the workspace writes it, `active` and
    /// `lastOpenFiles` untouched here because the pure writers do not set
    /// them.
    #[test]
    fn obsidians_popout_layout_round_trips_byte_for_byte() {
        let layout: Value = serde_json::from_str(FOUR_POPOUTS).unwrap();
        let main = read_main(&layout["main"], ViewMode::LivePreview, &|_| true).unwrap();
        let windows = read_floating(&layout["floating"], ViewMode::LivePreview, &|_| true);
        let mut written = layout.clone();
        written["main"] = spec_json(&main);
        let existing = layout["floating"].as_object().cloned();
        written["floating"] = floating_json(
            existing.as_ref(),
            windows.iter().map(window_spec_json).collect(),
        );
        assert_eq!(
            serde_json::to_string_pretty(&written).unwrap(),
            FOUR_POPOUTS
        );
    }

    /// A window TFT opens is written with Obsidian's keys in Obsidian's
    /// order, the bounds as whole numbers, and a `floating` the file never
    /// had gets a fresh node.
    #[test]
    fn a_fresh_window_node_writes_obsidians_keys_in_obsidians_order() {
        let tree = Layout::split(SplitDirection::Vertical, vec![Layout::Group(3)]);
        let w = WindowNode::new(
            1,
            tree.clone(),
            WindowBounds {
                x: 16,
                y: 10,
                width: 1024,
                height: 800,
            },
        );
        let Layout::Split { direction, .. } = &tree else {
            panic!("a split");
        };
        let group = json!({"id": "g", "type": "tabs", "children": []});
        let written = window_node_json(&w.node, vec![group], *direction);
        assert_eq!(
            written.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "id",
                "type",
                "children",
                "direction",
                "x",
                "y",
                "width",
                "height",
                "maximize",
                "zoom"
            ]
        );
        assert_eq!(written["type"], json!("window"));
        assert_eq!(written["direction"], json!("vertical"));
        assert_eq!(written["x"], json!(16), "a whole number, not 16.0");
        assert_eq!(written["height"], json!(800));
        assert_eq!(written["maximize"], json!(false));
        assert_eq!(written["zoom"], json!(0));
        assert_eq!(written["id"].as_str().unwrap().len(), 16);
        let floating = floating_json(None, vec![written]);
        assert_eq!(
            floating.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["id", "type", "children"]
        );
        assert_eq!(floating["type"], json!("floating"));
        assert_eq!(floating["children"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_top_right_group_follows_the_splits_shape() {
        // Side by side: the last child. Stacked: the first.
        let row = Layout::split(
            SplitDirection::Vertical,
            vec![Layout::Group(1), Layout::Group(2)],
        );
        assert_eq!(row.top_right(), Some(2));
        let stack = Layout::split(
            SplitDirection::Horizontal,
            vec![Layout::Group(1), Layout::Group(2)],
        );
        assert_eq!(stack.top_right(), Some(1));
        // A stack on the right of a row: its top group holds the corner,
        // though the depth-first order ends on its bottom one.
        let nested = Layout::split(
            SplitDirection::Vertical,
            vec![
                Layout::Group(1),
                Layout::split(
                    SplitDirection::Horizontal,
                    vec![Layout::Group(2), Layout::Group(3)],
                ),
            ],
        );
        assert_eq!(nested.top_right(), Some(2));
        let mut order = Vec::new();
        nested.groups(&mut order);
        assert_eq!(order.last(), Some(&3));
    }

    #[test]
    fn splitting_puts_a_sibling_in_a_split_that_runs_that_way_and_wraps_otherwise() {
        let mut tree = Layout::split(SplitDirection::Vertical, vec![Layout::Group(1)]);
        assert_eq!(
            tree.insert_beside(1, 2, SplitDirection::Vertical, true),
            Some(false)
        );
        let mut order = Vec::new();
        tree.groups(&mut order);
        assert_eq!(order, [1, 2]);
        assert_eq!(
            tree.insert_beside(2, 3, SplitDirection::Horizontal, true),
            Some(true)
        );
        assert_eq!(tree.path_to(3), Some(vec![1, 1]));
        assert!(matches!(
            tree.node_at(&[1]),
            Some(Layout::Split { direction: SplitDirection::Horizontal, children, .. }) if children.len() == 2
        ));
        // Neighbours: from 1, right is the whole right column; from 3, up is 2
        // and left is 1; from 2, down is 3, up is nothing.
        assert_eq!(tree.neighbors(1, SplitDirection::Vertical, true), [2, 3]);
        assert_eq!(tree.neighbors(3, SplitDirection::Horizontal, false), [2]);
        assert_eq!(tree.neighbors(3, SplitDirection::Vertical, false), [1]);
        assert_eq!(tree.neighbors(2, SplitDirection::Horizontal, true), [3]);
        assert!(tree
            .neighbors(2, SplitDirection::Horizontal, false)
            .is_empty());
        assert!(tree
            .neighbors(1, SplitDirection::Vertical, false)
            .is_empty());
        // Removing 2 leaves the horizontal split nested around 3, as Obsidian
        // keeps it; removing 3 too takes the split with it.
        assert!(tree.remove(2));
        assert_eq!(tree.path_to(3), Some(vec![1, 0]));
        assert!(tree.remove(3));
        let mut order = Vec::new();
        tree.groups(&mut order);
        assert_eq!(order, [1]);
        assert!(matches!(&tree, Layout::Split { children, .. } if children.len() == 1));
    }

    /// A tab dropped on a group's left or top edge goes before it: a sibling
    /// in front when the split runs that way, the first child of the split
    /// that wraps the pair when it does not.
    #[test]
    fn dropping_on_a_near_edge_puts_the_new_group_in_front() {
        let mut tree = Layout::split(SplitDirection::Vertical, vec![Layout::Group(1)]);
        assert_eq!(
            tree.insert_beside(1, 2, SplitDirection::Vertical, false),
            Some(false)
        );
        let mut order = Vec::new();
        tree.groups(&mut order);
        assert_eq!(order, [2, 1]);
        assert_eq!(
            tree.insert_beside(1, 3, SplitDirection::Horizontal, false),
            Some(true)
        );
        assert_eq!(tree.path_to(3), Some(vec![1, 0]));
        assert_eq!(tree.path_to(1), Some(vec![1, 1]));
        assert_eq!(tree.neighbors(1, SplitDirection::Horizontal, false), [3]);
    }

    /// The zones over a group's content: the outer band of each side splits
    /// towards that side, the corner goes to the nearer edge, and the middle
    /// joins the strip. The band is measured on the shorter side.
    #[test]
    fn drop_zones_follow_the_nearest_edge_within_the_band() {
        let bounds = gpui::Bounds {
            origin: point(px(100.), px(50.)),
            size: gpui::size(px(400.), px(200.)),
        };
        let at = |x: f32, y: f32| DropZone::at(bounds, point(px(100. + x), px(50. + y)));
        // The band is 30 percent of 200: 60 px.
        assert_eq!(at(200., 100.), DropZone::Center);
        assert_eq!(at(10., 100.), DropZone::Left);
        assert_eq!(at(390., 100.), DropZone::Right);
        assert_eq!(at(200., 10.), DropZone::Top);
        assert_eq!(at(200., 190.), DropZone::Bottom);
        assert_eq!(at(70., 100.), DropZone::Center);
        assert_eq!(at(200., 65.), DropZone::Center);
        // In a corner the nearer edge wins.
        assert_eq!(at(20., 40.), DropZone::Left);
        assert_eq!(at(40., 20.), DropZone::Top);
        assert_eq!(
            DropZone::Left.split(),
            Some((SplitDirection::Vertical, false))
        );
        assert_eq!(
            DropZone::Bottom.split(),
            Some((SplitDirection::Horizontal, true))
        );
        assert_eq!(DropZone::Center.split(), None);
    }

    #[test]
    fn a_dimension_sits_after_type() {
        let mut node = Map::new();
        node.insert("id".into(), json!("x"));
        node.insert("type".into(), json!("tabs"));
        node.insert("extra".into(), json!(1));
        set_dimension(&mut node, Some(40.0));
        let keys: Vec<&String> = node.keys().collect();
        assert_eq!(keys, ["id", "type", "dimension", "extra"]);
        set_dimension(&mut node, None);
        assert!(!node.contains_key("dimension"));
    }

    /// The Files leaf's state round trips through the two helpers: the
    /// keys keep their place among the ones Obsidian also writes, and a
    /// leaf written without a state gains one.
    #[test]
    fn explorer_leaf_state_is_read_and_written_in_place() {
        let mut layout = json!({
            "left": {"type": "split", "children": [{
                "type": "tabs",
                "children": [
                    {"type": "leaf", "state": {"type": "file-explorer", "state": {
                        "sortOrder": "byModifiedTime", "autoReveal": true,
                        "showSearch": false, "searchQuery": ""
                    }}},
                    {"type": "leaf", "state": {"type": "search"}},
                ],
            }]},
        });
        assert_eq!(
            explorer_leaf_state(&layout),
            Some((SortOrder::ByModifiedTime, true))
        );
        set_explorer_leaf_state(&mut layout["left"], SortOrder::ByCreatedTimeReverse, false);
        let state = &layout["left"]["children"][0]["children"][0]["state"]["state"];
        let keys: Vec<&String> = state.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["sortOrder", "autoReveal", "showSearch", "searchQuery"]
        );
        assert_eq!(state["sortOrder"], "byCreatedTimeReverse");
        assert_eq!(state["autoReveal"], false);

        let mut bare = json!({"left": {"type": "split", "children": [{
            "type": "tabs",
            "children": [{"type": "leaf", "state": {"type": "file-explorer"}}],
        }]}});
        assert_eq!(
            explorer_leaf_state(&bare),
            Some((SortOrder::Alphabetical, false))
        );
        set_explorer_leaf_state(&mut bare["left"], SortOrder::AlphabeticalReverse, true);
        assert_eq!(
            bare["left"]["children"][0]["children"][0]["state"]["state"],
            json!({"sortOrder": "alphabeticalReverse", "autoReveal": true})
        );

        let no_leaf = json!({"left": {"type": "split", "children": []}});
        assert_eq!(explorer_leaf_state(&no_leaf), None);
        let mut untouched = no_leaf.clone();
        set_explorer_leaf_state(&mut untouched["left"], SortOrder::ByCreatedTime, true);
        assert_eq!(untouched, no_leaf);
    }

    #[test]
    fn a_docks_current_tab_is_the_leaf_current_tab_points_at() {
        let layout = json!({
            "left": {"type": "split", "children": [{
                "type": "tabs",
                "currentTab": 1,
                "children": [
                    {"type": "leaf", "state": {"type": "file-explorer"}},
                    {"type": "leaf", "state": {"type": "search"}},
                ],
            }]},
            "right": {"type": "split", "children": [{
                "type": "tabs",
                "children": [{"type": "leaf", "state": {"type": "outgoing-link"}}],
            }]},
        });
        assert_eq!(
            dock_current_view(&layout, "left").as_deref(),
            Some("search")
        );
        assert_eq!(
            dock_current_view(&layout, "right").as_deref(),
            Some("outgoing-link")
        );
        assert_eq!(dock_current_view(&layout, "main"), None);
    }

    #[test]
    fn prune_history_forgets_a_deleted_note() {
        let mut h = history(&["A.md", "B.md", "C.md"]);
        assert_eq!(prune_history(&mut h, 2, "B.md"), 1);
        assert_eq!(h, history(&["A.md", "C.md"]));

        let mut h = history(&["A.md", "B.md", "C.md"]);
        assert_eq!(prune_history(&mut h, 3, "A.md"), 2, "still at C.md");
        assert_eq!(h, history(&["B.md", "C.md"]));

        let mut h = history(&["A.md", "B.md"]);
        assert_eq!(prune_history(&mut h, 1, "B.md"), 1, "after the cursor");
        assert_eq!(h, history(&["A.md"]));
    }

    /// Deleting a folder deletes every note under it, and a repeat visit is
    /// a repeat entry, so more than one entry can go at once.
    #[test]
    fn prune_history_forgets_a_whole_folder() {
        let mut h = history(&["Sub/S.md", "A.md", "Sub/T.md", "A.md"]);
        assert_eq!(prune_history(&mut h, 4, "Sub"), 2);
        assert_eq!(h, history(&["A.md", "A.md"]));

        let mut h = history(&["A.md", "A.md", "B.md"]);
        assert_eq!(prune_history(&mut h, 3, "A.md"), 1);
        assert_eq!(h, history(&["B.md"]));
    }

    /// A path the history never held, and a prefix that is not a folder
    /// boundary, leave it alone.
    #[test]
    fn prune_history_keeps_what_it_does_not_name() {
        let mut h = history(&["A.md", "Subtotal/S.md"]);
        assert_eq!(prune_history(&mut h, 2, "Sub"), 2);
        assert_eq!(h, history(&["A.md", "Subtotal/S.md"]));

        let mut h = history(&["A.md"]);
        assert_eq!(prune_history(&mut h, 0, "A.md"), 0, "nothing open");
        assert!(h.is_empty());
    }

    /// The block id write: it reaches the target's bytes, keeps that
    /// note's own line endings, and refuses a file it did not plan from.
    #[test]
    fn writing_a_block_id_reaches_the_disk_and_refuses_a_stale_file() {
        use tft_suggest::blocks::{plan_id, Placement};
        let (dir, state) = vault();
        let path = "A.md";
        let before = fs::read_to_string(dir.path().join(path)).unwrap();
        let end = before.find("See [[B]]").unwrap() + "See [[B]] and [S](Sub/S.md).".len();
        let insertion = plan_id(&before, end, Placement::Inline, "abc123", "\n").expect("a plan");
        state.write_block_id(path, &before, &insertion).unwrap();
        let after = fs::read_to_string(dir.path().join(path)).unwrap();
        assert_eq!(after, insertion.applied(&before));
        assert!(
            after.contains("^abc123"),
            "the id is not in the file: {after}"
        );

        // The same insertion against bytes the file no longer holds is
        // refused rather than written.
        let err = state
            .write_block_id(path, &before, &insertion)
            .expect_err("the file changed since");
        assert!(err.contains("changed on disk"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.path().join(path)).unwrap(),
            after,
            "a refused write still changed the file"
        );
    }

    /// A CRLF note keeps its own endings when an id is written into it,
    /// which is where TFT deliberately differs from Obsidian.
    #[test]
    fn a_block_id_written_into_a_crlf_note_keeps_its_endings() {
        use tft_suggest::blocks::{plan_id, Placement};
        let (dir, state) = vault();
        let path = "Crlf.md";
        let before = "# Crlf\r\n\r\npara in a CRLF file.\r\n";
        fs::write(dir.path().join(path), before).unwrap();
        let end = before.find("para in a CRLF file.").unwrap() + "para in a CRLF file.".len();
        let insertion = plan_id(before, end, Placement::Inline, "abc123", "\r\n").expect("a plan");
        state.write_block_id(path, before, &insertion).unwrap();
        let after = fs::read_to_string(dir.path().join(path)).unwrap();
        assert_eq!(after, "# Crlf\r\n\r\npara in a CRLF file. ^abc123\r\n");
        assert!(
            !after.contains("\n\n"),
            "a bare newline reached a CRLF note"
        );
    }

    /// The removed features left no runnable or editor-only id behind: the
    /// command registry no longer knows them, and the workspace must not
    /// offer them either.
    #[test]
    fn removed_features_leave_no_runnable_or_editor_only_ids() {
        for id in [
            "bases:new-file",
            "bases:insert",
            "bases:copy-table",
            "bases:change-view",
            "bases:add-view",
            "bases:add-item",
            "command-palette:open",
            "note-composer:merge-file",
            "note-composer:split-file",
            "note-composer:extract-heading",
            "file-recovery:open",
            "insert-template",
            "insert-current-date",
            "insert-current-time",
            "daily-notes",
            "daily-notes:goto-prev",
            "daily-notes:goto-next",
            "sync:setup",
            "audio-recorder:start",
            "audio-recorder:stop",
            "footnotes:open",
            "publish:publish-file",
            "slides:start",
            "webviewer:open",
            "workspaces:save",
            "zk-prefixer",
            "insert-unique-link",
            "random-note",
            "bookmarks:open",
            "bookmarks:bookmark-all-tabs",
            "backlink:open",
            "backlink:toggle-backlinks-in-document",
        ] {
            assert!(!RUNNABLE.contains(&id), "{id} is still runnable");
            assert!(!EDITOR_ONLY.contains(&id), "{id} is still editor-only");
        }
    }

    /// The probe's heading-rename vault: `Note.md` and its `Linker.md`.
    fn heading_vault(app_json: &str) -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".obsidian")).unwrap();
        fs::write(r.join(".obsidian/app.json"), app_json).unwrap();
        fs::write(r.join("Note.md"), "# Parent\n\n## Old\n\nUnder old. Self [[#Old]] and [[#Old|self alias]] and [[Note#Old]].\n\nSetext Old\n----------\n\nUnder setext.\n").unwrap();
        fs::write(r.join("Linker.md"), "A [[Note#Old]]\nB [[Note#Old|alias]]\nC ![[Note#Old]]\nD [x](Note.md#Old)\nE [[Note#Parent#Old]]\nF [[Note#Old|Old]]\nG [[Note#old]]\nH [[Note#Old|Note > Old]]\nI [y](<Note.md#Old>)\nJ [[Note#Setext Old]] and [z](Note.md#Setext%20Old)\nK [[Note]]\n").unwrap();
        let state = VaultState::load(&r.to_path_buf()).unwrap();
        (dir, state)
    }

    fn plan_heading(state: &VaultState, own: &str, name: &str) -> tft_cache::HeadingRename {
        let texts = state.texts();
        state
            .cache()
            .plan_heading_rename("Note.md", own, 2, name, &|p| {
                texts.get(p).map(|t| t.to_string())
            })
            .unwrap()
    }

    /// `editor:rename-heading` writes every linking note on disk and in the
    /// index, with the probe's bytes -- and the same bytes with
    /// `alwaysUpdateLinks` off, which Obsidian does not consult here.
    #[test]
    fn heading_rename_rewrites_linking_notes_on_disk_whatever_always_update_links_says() {
        let expected = "A [[Note#New]]\nB [[Note#New|alias]]\nC ![[Note#New]]\nD [x](Note.md#New)\nE [[Note#Parent#Old]]\nF [[Note#New|Old]]\nG [[Note#New]]\nH [[Note#New|Note > Old]]\nI [y](<Note.md#New>)\nJ [[Note#Setext Old]] and [z](Note.md#Setext%20Old)\nK [[Note]]\n";
        for app in [
            r#"{"alwaysUpdateLinks":true}"#,
            r#"{"alwaysUpdateLinks":false}"#,
        ] {
            let (dir, state) = heading_vault(app);
            let own = fs::read_to_string(dir.path().join("Note.md")).unwrap();
            let plan = plan_heading(&state, &own, "New");
            assert_eq!(
                tft_cache::apply_edits(&own, &plan.own),
                "# Parent\n\n## New\n\nUnder old. Self [[#New]] and [[#New|self alias]] and [[Note#New]].\n\nSetext Old\n----------\n\nUnder setext.\n"
            );
            let (changes, stale) = state.write_link_rewrites(&plan.rewrites).unwrap();
            assert!(stale.is_empty());
            assert_eq!(changes.notes, vec!["Linker.md".to_string()]);
            assert_eq!(
                fs::read_to_string(dir.path().join("Linker.md")).unwrap(),
                expected,
                "{app}"
            );
            assert_eq!(state.texts()["Linker.md"].as_str(), expected);
            // The index follows: the rewritten links point at the new heading.
            assert!(state
                .cache()
                .file("Linker.md")
                .unwrap()
                .links
                .iter()
                .any(|l| l.link == "Note#New"));
        }
    }

    /// A linking note another app saved since it was indexed keeps its own
    /// bytes, as a file rename leaves it.
    #[test]
    fn heading_rename_leaves_a_linking_note_that_changed_underneath_alone() {
        let (dir, state) = heading_vault("{}");
        let own = fs::read_to_string(dir.path().join("Note.md")).unwrap();
        let plan = plan_heading(&state, &own, "New");
        let theirs = "A [[Note#Old]] and the other app's edit\n";
        fs::write(dir.path().join("Linker.md"), theirs).unwrap();
        let (changes, stale) = state.write_link_rewrites(&plan.rewrites).unwrap();
        assert_eq!(stale, vec!["Linker.md".to_string()]);
        assert!(changes.files.is_empty());
        assert_eq!(
            fs::read_to_string(dir.path().join("Linker.md")).unwrap(),
            theirs
        );
    }

    /// Both commands run, the way their siblings do: rename-heading and
    /// add-alias are offered only with an editor holding the keys.
    #[test]
    fn rename_heading_and_add_alias_are_runnable_editor_commands() {
        let known = tft_commands::known_ids();
        for id in ["editor:rename-heading", "markdown:add-alias"] {
            assert!(RUNNABLE.contains(&id), "{id} runnable");
            assert!(EDITOR_ONLY.contains(&id), "{id} needs the note");
            assert!(known.contains(&id), "{id} is Obsidian's");
        }
    }

    #[test]
    fn rename_leaves_a_note_that_changed_underneath_alone() {
        // The rewrites are planned from the text the index holds. If the
        // other app saved one of those notes since, writing the plan would
        // throw that save away, so the note keeps its own bytes.
        let (dir, state) = vault();
        let theirs = "# B\n\nRewritten by the other app, with [[A]] still in it.\n";
        fs::write(dir.path().join("B.md"), theirs).unwrap();
        let outcome = state.rename("A.md", "Renamed A.md", true).unwrap();
        assert_eq!(outcome.stale, vec!["B.md".to_string()]);
        assert_eq!(
            fs::read_to_string(dir.path().join("B.md")).unwrap(),
            theirs,
            "the other app's save was overwritten"
        );
        // The notes that did not change are still rewritten.
        assert!(outcome.rewritten_files >= 1);
        let sub = fs::read_to_string(dir.path().join("Sub/S.md")).unwrap();
        assert!(
            sub.contains("Renamed A"),
            "an untouched note was not rewritten: {sub}"
        );
    }

    #[test]
    fn rename_rewrites_links_on_disk_and_in_the_index() {
        let (dir, state) = vault();
        let outcome = state.rename("A.md", "Renamed A.md", true).unwrap();
        assert_eq!(
            outcome.renamed,
            vec![("A.md".to_string(), "Renamed A.md".to_string())]
        );
        assert_eq!((outcome.rewritten_files, outcome.rewritten_links), (2, 4));
        let b = fs::read_to_string(dir.path().join("B.md")).unwrap();
        assert_eq!(
            b,
            "\u{feff}---\r\nrelated: \"[[Renamed A]]\"\r\n---\r\n[[Renamed A]] and [[Renamed A#A|the a]] and `[[A]]`\r\n",
            "the BOM and CRLF endings survive"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("Sub/S.md")).unwrap(),
            "[Renamed A](Renamed%20A.md) [[B]]\n"
        );
        assert!(dir.path().join("Renamed A.md").is_file());
        assert!(!dir.path().join("A.md").exists());
        {
            let cache = state.cache();
            assert!(cache.file("A.md").is_none());
            assert!(cache.file("Renamed A.md").is_some());
            // B's `related` property, its two body links, and S's markdown link.
            assert_eq!(cache.backlinks("Renamed A.md").len(), 4);
            assert!(cache.unresolved_links("B.md").is_none());
            assert_eq!(state.texts()["B.md"].as_str(), b);
        }
        assert!(outcome.changes.structural());
        // The watcher's events for the same paths find nothing new.
        let changes = state.apply(&[
            dir.path().join("A.md"),
            dir.path().join("Renamed A.md"),
            dir.path().join("B.md"),
        ]);
        assert!(changes
            .files
            .iter()
            .all(|c| matches!(c, FileChange::Modified(_))));
        assert_eq!(state.cache().backlinks("Renamed A.md").len(), 4);
    }

    #[test]
    fn rename_without_updating_links_leaves_them_dangling() {
        let (dir, state) = vault();
        let outcome = state.rename("A.md", "A2.md", false).unwrap();
        assert_eq!(outcome.rewritten_links, 0);
        assert!(fs::read_to_string(dir.path().join("B.md"))
            .unwrap()
            .contains("[[A]] and"));
        assert_eq!(
            state.cache().unresolved_links("B.md").unwrap().get("A"),
            Some(&3),
            "two body links and the frontmatter link"
        );
        assert!(
            state.rename("A2.md", "B.md", true).is_err(),
            "destination exists"
        );
        assert_eq!(
            state.rename_preview("B.md", "Sub/B.md"),
            (0, 0),
            "a unique name moving folders changes no shortest link"
        );
        assert_eq!(state.rename_preview("B.md", "Sub/Bee.md"), (2, 2));
    }

    /// `alwaysUpdateLinks` absent means **ask**, not rewrite. Obsidian
    /// rendered the toggle off on a vault whose `app.json` was `{}`
    /// (`tools/obsidian-probe/results/2026-10-25-ui-rules.json`), and its own
    /// description is "Turn off to be prompted to update links after renaming
    /// a file". The gate's day 2 found this inverted, which made the
    /// "Update links?" prompt unreachable on every vault that has never
    /// written the key — the gate vault among them.
    #[test]
    fn a_vault_with_no_always_update_links_key_asks_before_rewriting() {
        let (_dir, state) = vault();
        assert!(
            state.app().always_update_links.is_none(),
            "the fixture must not write the key, or this asserts nothing"
        );
        assert!(
            !state.update_links_automatically(),
            "no key means ask, as Obsidian's unchecked toggle does"
        );
        // And there is something to ask about, so the prompt is reached
        // rather than skipped by the `links == 0` shortcut.
        let (files, links) = state.rename_preview("A.md", "A2.md");
        assert!(
            files > 0 && links > 0,
            "renaming A should affect links, got {links} in {files}"
        );
    }

    /// Obsidian's modal, string for string and button for button
    /// (`tools/obsidian-probe/results/2026-10-25-explorer-dnd.json`, `s05`).
    /// The counts there were both 1, which is why the singular is the case
    /// pinned to the probe and the plural is the inference.
    #[test]
    fn the_update_links_prompt_is_obsidians_three_buttons_and_two_lines() {
        let one = update_links_prompt(1, 1);
        assert_eq!(one.title, "Update links");
        assert_eq!(
            one.detail,
            vec![
                "Do you want to update internal links that link to this file?",
                "This will affect 1 link in 1 file."
            ],
            "the two lines the probe read, concatenated, were exactly these"
        );
        let row: Vec<(&str, PromptAction, bool)> = one
            .buttons
            .iter()
            .map(|b| (b.label.as_ref(), b.action, b.cta))
            .collect();
        assert_eq!(
            row,
            vec![
                ("Always update", PromptAction::Tertiary, false),
                ("Just once", PromptAction::Submit, false),
                ("Do not update", PromptAction::Secondary, false),
            ],
            "Obsidian's order, no accent on any, and no Cancel -- its modal has three buttons"
        );
        assert_eq!(
            update_links_prompt(6, 6).detail[1],
            "This will affect 6 links in 6 files.",
            "both nouns pluralise"
        );
    }

    /// A write through the control socket can only land inside the vault
    /// folder, and the path it is given is usually one that does not exist
    /// yet, so the folders it goes through are what the check has to look
    /// at.
    #[test]
    fn vault_paths_cannot_leave_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join("Sub")).unwrap();
        fs::write(root.join("A.md"), "a").unwrap();

        assert_eq!(vault_path_in(&root, "A.md").unwrap(), root.join("A.md"));
        assert_eq!(
            vault_path_in(&root, "Sub/New.md").unwrap(),
            root.join("Sub/New.md"),
            "a note that is not there yet, under a folder that is"
        );
        assert_eq!(vault_path_in(&root, "./A.md").unwrap(), root.join("A.md"));

        assert!(vault_path_in(&root, "../escape.md").is_err());
        assert!(vault_path_in(&root, "Sub/../../escape.md").is_err());
        assert!(vault_path_in(&root, "/etc/passwd").is_err());
        assert!(vault_path_in(&root, "").is_err(), "the folder itself");

        // A symlinked folder in the vault: the path is innocent, the folder
        // it goes through is not.
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("there.md"), "o").unwrap();
        if symlink(outside.path(), &root.join("linked")).is_none() {
            return;
        }
        assert!(vault_path_in(&root, "linked/new.md").is_err());
        assert!(vault_path_in(&root, "linked/there.md").is_err());

        // A dangling link resolves to nothing, and a write through it would
        // create the file wherever it points.
        if symlink(&outside.path().join("gone.md"), &root.join("dangling.md")).is_none() {
            return;
        }
        assert!(vault_path_in(&root, "dangling.md").is_err());
    }

    /// A file kind reaches the state dump in lower case.
    #[test]
    fn file_kinds_are_named_in_lower_case() {
        assert_eq!(kind_name(FileKind::Markdown), "markdown");
        assert_eq!(kind_name(FileKind::Other), "other");
    }

    #[test]
    fn create_duplicate_and_trash_keep_the_index_current() {
        let (dir, state) = vault();
        assert_eq!(
            state.new_note_path("Missing", Some("Sub/S.md")),
            "Missing.md"
        );
        assert_eq!(
            state.new_note_path("Deep/Also Missing", None),
            "Deep/Also Missing.md"
        );
        let changes = state.create_note("Deep/Also Missing.md", "").unwrap();
        assert_eq!(
            changes.files,
            vec![FileChange::Added("Deep/Also Missing.md".into())]
        );
        assert!(state.cache().file("Deep/Also Missing.md").is_some());
        assert!(state.vault().folders().contains(&"Deep".to_string()));
        let (copy, _) = state.duplicate("A.md").unwrap();
        assert_eq!(copy, "A 1.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("A 1.md")).unwrap(),
            fs::read_to_string(dir.path().join("A.md")).unwrap()
        );
        let changes = state.trash("Sub").unwrap();
        assert_eq!(changes.removed_notes, vec!["Sub/S.md"]);
        assert!(dir.path().join(".trash/Sub/S.md").is_file());
        assert!(state.cache().file("Sub/S.md").is_none());
        assert_eq!(state.cache().backlinks("B.md").len(), 2, "A and its copy");
        assert!(state.vault().folders().iter().all(|f| f != "Sub"));
    }

    /// The socket's `focus` may not park the keyboard on a view the main
    /// area is not painting: that handle is in no frame's dispatch tree, and
    /// from there the app answers no key at all, the one that would undo it
    /// included.
    #[test]
    fn focus_refuses_a_view_the_main_area_is_not_painting() {
        assert_eq!(
            Workspace::focus_target("editor", MainView::Editor),
            Ok(Some(MainView::Editor))
        );
        assert_eq!(
            Workspace::focus_target("workspace", MainView::Empty),
            Ok(None),
            "the root div paints in every frame"
        );
        let err = Workspace::focus_target("reading", MainView::Editor).unwrap_err();
        assert!(err.contains("painting editor, not reading"), "{err}");
        let err = Workspace::focus_target("graph", MainView::Empty).unwrap_err();
        assert!(err.contains("painting nothing, not graph"), "{err}");
        assert_eq!(
            Workspace::focus_target("pane", MainView::Editor),
            Err("unknown view: pane".to_string())
        );
    }

    /// The socket's name for the PDF view is `pdf`, the same three ways;
    /// its page box is a field of its own, so `type` reaches it the way it
    /// reaches any input.
    #[test]
    fn the_socket_calls_the_pdf_view_pdf() {
        assert_eq!(MainView::Pdf.as_str(), "pdf");
        assert_eq!(
            Workspace::focus_target("pdf", MainView::Pdf),
            Ok(Some(MainView::Pdf))
        );
        let err = Workspace::focus_target("pdf", MainView::Editor).unwrap_err();
        assert!(err.contains("painting editor, not pdf"), "{err}");
        assert_eq!(Workspace::typed_text_target("pdf", MainView::Pdf), "none");
    }

    /// An editor whose buffer names no file never gets the main area: on
    /// launch over a vault with nothing to open the buffer belongs to
    /// nothing, and anything typed into it would be saved nowhere.
    #[test]
    fn the_main_area_paints_no_editor_without_a_file_behind_it() {
        for mode in [ViewMode::LivePreview, ViewMode::Source] {
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, false, false),
                MainView::Empty,
                "{mode:?} with nothing open"
            );
            assert_eq!(
                Workspace::main_view_for(mode, true, false, true, false, false),
                MainView::Editor,
                "{mode:?} with a note open"
            );
            // The `--synthetic` spike hands the editor a note that is in no
            // vault: the buffer names a file, and that is what mounts it.
            assert_eq!(
                Workspace::main_view_for(mode, false, false, true, false, false),
                MainView::Editor,
                "{mode:?} with a buffer that belongs to no vault"
            );
            assert_eq!(
                Workspace::main_view_for(mode, true, true, true, false, false),
                MainView::Reading,
                "{mode:?} over an image"
            );
        }
        assert_eq!(
            Workspace::main_view_for(ViewMode::Reading, false, false, true, false, false),
            MainView::Empty,
            "a buffer is not a built note"
        );
        assert_eq!(
            Workspace::main_view_for(ViewMode::Graph, false, false, false, false, false),
            MainView::Graph,
            "the graph needs no file"
        );
        for mode in [ViewMode::Reading, ViewMode::LivePreview, ViewMode::Source] {
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, true, false),
                MainView::Canvas,
                "{mode:?} over a canvas"
            );
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, false, true),
                MainView::Pdf,
                "{mode:?} over a PDF"
            );
        }
        assert_eq!(
            Workspace::main_view_for(ViewMode::Graph, false, false, false, false, true),
            MainView::Graph,
            "the graph wins over a PDF"
        );
        // The graph is the one view that outranks a file: a canvas open
        // behind it stays behind it.
        assert_eq!(
            Workspace::main_view_for(ViewMode::Graph, false, false, false, true, false),
            MainView::Graph,
            "the graph wins over a canvas"
        );
    }

    /// The socket's `type` reaches the editor only while the main area is
    /// painting it. The editor's handle can still hold the focus under the
    /// empty state, and this route does not go through the dispatch tree
    /// that would otherwise refuse it.
    #[test]
    fn typing_reaches_no_editor_the_main_area_is_not_painting() {
        for focus in ["editor", "property-field", "cell-field", "title-field"] {
            assert_eq!(
                Workspace::typed_text_target(focus, MainView::Editor),
                "editor"
            );
            assert_eq!(Workspace::typed_text_target(focus, MainView::Empty), "none");
            assert_eq!(
                Workspace::typed_text_target(focus, MainView::Reading),
                "none"
            );
        }
        assert_eq!(
            Workspace::typed_text_target("prompt-input", MainView::Empty),
            "input",
            "a modal's field is on screen whatever the main area paints"
        );
        assert_eq!(
            Workspace::typed_text_target("workspace", MainView::Editor),
            "none"
        );
    }

    /// A field of the File properties pane is the editor's, and the pane is
    /// on screen whatever the main area paints, so typed text reaches it in
    /// reading view too; the pane with no field takes none.
    #[test]
    fn typing_reaches_a_file_properties_field_in_every_view() {
        for painted in [MainView::Editor, MainView::Reading, MainView::Empty] {
            assert_eq!(
                Workspace::typed_text_target("file-properties-field", painted),
                "editor"
            );
            assert_eq!(
                Workspace::typed_text_target("file-properties", painted),
                "none"
            );
        }
    }

    /// A notice leaves on its own timer, and only the one that timer named:
    /// a message put up later stays until its own turn comes.
    #[test]
    fn a_notice_expires_by_id_and_leaves_the_newer_one() {
        let mut n = Notices::default();
        let first = n.push("The cursor is not in a table".into(), None);
        let second = n.push("A.md saved".into(), None);
        assert_eq!(n.latest().map(|t| t.as_ref()), Some("A.md saved"));

        n.expire(first);
        assert_eq!(n.texts(), vec!["A.md saved".to_string()]);
        n.expire(second);
        assert!(n.texts().is_empty());
        assert!(n.latest().is_none(), "nothing is left to misread");
    }

    /// The autosave retries on a timer, so the same refusal arrives again
    /// and again: it renews the message on screen instead of stacking
    /// copies of it, and the renewed one outlives the first one's timer.
    #[test]
    fn a_repeated_notice_is_renewed_rather_than_stacked() {
        let mut n = Notices::default();
        let first = n.push("A.md changed on disk".into(), None);
        let again = n.push("A.md changed on disk".into(), None);
        assert_ne!(first, again);
        assert_eq!(n.texts(), vec!["A.md changed on disk".to_string()]);

        n.expire(first);
        assert_eq!(
            n.texts(),
            vec!["A.md changed on disk".to_string()],
            "the first timer must not take the renewed message down"
        );
        n.expire(again);
        assert!(n.texts().is_empty());
    }

    /// Clearing takes down everything, which is what the control socket's
    /// `notice_clear` is for.
    #[test]
    fn clearing_takes_every_notice_down() {
        let mut n = Notices::default();
        n.push("one".into(), None);
        n.push("two".into(), None);
        n.clear();
        assert!(n.latest().is_none());
        assert_eq!(n.push("three".into(), None), 3, "ids keep counting up");
    }

    /// A notice stays in the window it went up in, whichever window has
    /// the focus later, and falls back to the vault window once its popout
    /// closes; a renewal moves it to the window of the renewal.
    #[test]
    fn a_notice_draws_in_its_own_window_until_that_closes() {
        let mut n = Notices::default();
        n.push("in the vault window".into(), None);
        n.push("in the popout".into(), Some(7));
        let texts = |it: &mut dyn Iterator<Item = &Notice>| -> Vec<String> {
            it.map(|n| n.text.to_string()).collect()
        };

        let open = |id| id == 7;
        assert_eq!(
            texts(&mut n.in_window(None, &open)),
            vec!["in the vault window"]
        );
        assert_eq!(
            texts(&mut n.in_window(Some(7), &open)),
            vec!["in the popout"]
        );
        assert_eq!(n.windows(&open), vec![None, Some(7)]);

        let closed = |_| false;
        assert_eq!(
            texts(&mut n.in_window(None, &closed)),
            vec!["in the vault window", "in the popout"],
            "a closed popout hands its notices to the vault window"
        );
        assert_eq!(n.windows(&closed), vec![None, None]);

        n.push("in the vault window".into(), Some(7));
        assert_eq!(n.windows(&open), vec![Some(7), Some(7)]);
    }
}
