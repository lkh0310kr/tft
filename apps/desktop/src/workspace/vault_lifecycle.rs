use super::{Changes, Commands, VaultState, Workspace};
use crate::editor::Editor;
use crate::explorer::FileTree;
use crate::reading;
use crate::settings::{self, SettingChange};
use crate::theme::Theme;
use crate::watch;
use crate::zoom;
use gpui::{App, Context, Entity, SharedString, Window};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tft_commands::CommandRegistry;
use tft_formats::properties::PropertiesInDocument;
use tft_formats::FileKind;
use tft_vault::FileChange;

/// System color scheme plus the Desktop-only session override for `system` themes.
pub(super) struct ThemeAppearanceState {
    system_dark: bool,
    session_flipped: bool,
}

impl ThemeAppearanceState {
    pub(super) fn new(system_dark: bool) -> Self {
        Self {
            system_dark,
            session_flipped: false,
        }
    }

    pub(super) fn set_system_dark(&mut self, dark: bool) -> bool {
        let changed = self.system_dark != dark;
        self.system_dark = dark;
        changed
    }

    pub(super) fn clear_session_override(&mut self) {
        self.session_flipped = false;
    }

    pub(super) fn flip_session(&mut self) {
        self.session_flipped = !self.session_flipped;
    }

    pub(super) fn effective_dark(&self) -> bool {
        self.system_dark != self.session_flipped
    }
}

/// The current vault and resources that begin and end with its loading lifecycle.
pub(super) struct VaultSessionState {
    pub(super) state: Option<Arc<VaultState>>,
    pub(super) name: SharedString,
    /// Whether this vault appears in the switcher registry; sandbox vaults do not.
    pub(super) record: bool,
    pub(super) loading: Option<SharedString>,
    pub(super) watcher: Option<watch::Watcher>,
}

impl VaultSessionState {
    pub(super) fn new() -> Self {
        Self {
            state: None,
            name: "No vault".into(),
            record: true,
            loading: None,
            watcher: None,
        }
    }
}

impl Workspace {
    pub(crate) fn load_vault(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_vault_with(root, true, window, cx);
    }

    /// [`Self::load_vault`], saying whether the vault goes on the switcher's list.
    pub(super) fn load_vault_with(
        &mut self,
        root: PathBuf,
        record: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_vault(cx);
        self.vault_session.record = record;
        self.vault_session.name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into();
        self.vault_session.loading = Some(format!("Indexing {}…", root.display()).into());
        self.feedback.set_status("Indexing vault");
        // Restoring may make tab groups, and a group's editor needs the window, so the
        // vault comes back in it.
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { VaultState::load(&root) })
                .await;
            this.update_in(cx, |ws, window, cx| match result {
                Ok(state) => ws.set_state(state, record, window, cx),
                Err(err) => {
                    ws.vault_session.loading = None;
                    ws.feedback
                        .set_status(format!("Vault failed to open: {err}"));
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
        if self.vault_session.state.is_none() {
            return;
        }
        self.on_quit(cx);
        self.forget_vault(cx);
    }

    /// [`Self::leave_vault`] less the writes: every group and popout closed, the vault's state dropped.
    pub(super) fn forget_vault(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.groups.iter().map(|g| g.state.id).collect();
        for id in ids {
            self.close_group(id, false, cx);
        }
        self.windows.clear();
        self.closed_tab_history.clear();
        self.active_group_state.detached = None;
        self.layout_persistence.clear_vault_layout();
        self.vault_session.watcher = None;
        self.vault_session.state = None;
    }

    /// Records a property type in `types.json` and redraws the views from the new table.
    pub(super) fn assign_property_type(
        &mut self,
        key: String,
        ty: tft_formats::properties::PropertyType,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        match state.assign_property_type(&key, ty) {
            Ok(types) => {
                self.active_reading()
                    .update(cx, |r, _| r.set_property_types(types.clone()));
                self.active_editor()
                    .update(cx, |e, _| e.set_property_types(types));
                cx.notify();
            }
            Err(err) => {
                self.report(format!("types.json could not be written: {err}"), cx);
            }
        }
    }

    /// Pushes what `app.json` says into the open views; called when the vault loads
    /// and whenever Settings writes.
    pub(super) fn apply_app_config(&mut self, state: &VaultState, cx: &mut Context<Self>) {
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
    pub(super) fn configure_editor(editor: &Entity<Editor>, state: &VaultState, cx: &mut App) {
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

    pub(super) fn change_setting(&mut self, change: SettingChange, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
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

    fn set_state(
        &mut self,
        state: VaultState,
        record: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vault_session.loading = None;
        self.feedback.set_status(format!(
            "{} files, {} notes indexed in {:.0} ms",
            state.vault().files().len(),
            state.cache().len(),
            state.index_ms
        ));
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
        self.vault_session.state = Some(state.clone());
        // The vault window comes back at the zoom it was left at.
        let level = zoom::load_level(&state.root);
        if self.main_window.set_zoom_level(level) {
            self.apply_main_zoom(cx);
        }
        if record {
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
        if let Some(layout) = self.layout_persistence.take_pending_reload() {
            // A reload: the commands and keys from the files as they are
            // now, and the layout as it was left.
            self.rebuild_commands(&state, cx);
            self.apply_layout(layout, &state, window, cx);
            self.focus_main(window, cx);
        } else if let Some(current) = self.active_group_state.current.clone() {
            self.open_file(current, None, cx);
        } else {
            self.restore_layout(&state, window, cx);
        }
        cx.notify();
    }

    /// Points an editor at the vault: what its embeds render from and what
    /// its autocompletion offers. Every group's editor gets this, the first
    /// when the vault opens and the rest as they are made.
    pub(super) fn give_editor_the_vault(
        editor: &Entity<Editor>,
        state: &Arc<VaultState>,
        cx: &mut App,
    ) {
        let embed_source: Arc<dyn reading::EmbedSource + Send + Sync> = state.clone();
        let build_options = state.build_options();
        editor.update(cx, |e, cx| {
            e.set_vault(Some(embed_source), build_options, cx);
            e.set_suggestion_source(Some(Arc::new(
                crate::suggestion_source::DesktopSuggestionSource::new(state.clone()),
            )));
        });
    }

    /// Rebuilds everything that depends on the file list or the whole cache.
    pub(super) fn refresh_vault_views(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
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
        self.settings_state.set_vault_state(state.clone(), cx);
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
        self.vault_session.watcher = Some(watcher);
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

    pub(super) fn files_changed(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
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
        self.theme_appearance.set_system_dark(dark);
        self.reload_theme(cx);
    }

    /// `theme:toggle-light-dark`: with a named base theme write the other; under
    /// `system` write nothing and flip only the session's scheme.
    pub(super) fn toggle_light_dark(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        // Its own statement: `appearance()` is a read guard and `change_setting` takes the write lock.
        let swapped = settings::toggle_light_dark(&state.appearance());
        match swapped {
            Some(next) => {
                self.theme_appearance.clear_session_override();
                self.change_setting(SettingChange::Appearance(next), cx);
            }
            None => {
                self.theme_appearance.flip_session();
                self.reload_theme(cx);
            }
        }
    }

    /// Rebuilds the theme global from `appearance.json`'s base color scheme and repaints.
    pub fn reload_theme(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let config = state.reload_appearance();
        let theme = Theme::from_appearance(&config, self.theme_appearance.effective_dark());
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
        let editor = self.active_editor().read(cx);
        if editor.name.as_ref() != path {
            return false;
        }
        // Typing can resume before the echo arrives: the buffer has moved on, but
        // the file is what it last wrote.
        state.texts().get(path).is_some_and(|text| {
            editor.text() == text.as_str() || (editor.is_dirty() && editor.disk_unchanged(text))
        })
    }

    pub(super) fn after_changes(&mut self, changes: Changes, cx: &mut Context<Self>) {
        if changes.files.is_empty() {
            return;
        }
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        if changes.structural() {
            self.refresh_vault_views(cx);
        } else {
            self.search.update(cx, |s, cx| s.rerun(cx));
        }
        let n = changes.files.len();
        self.feedback.set_status(if n == 1 {
            format!("{} changed", changes.files[0].path())
        } else {
            format!("{n} files changed")
        });
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
        if let Some(current) = self.active_group_state.current.clone() {
            let removed = changes
                .files
                .iter()
                .any(|c| matches!(c, FileChange::Removed(p) if *p == current));
            if removed && !state.vault().absolute(&current).exists() {
                // The open note is gone: the buffer stays (it may hold unsaved edits)
                // but is no longer a file, and the panes stop describing it.
                self.detach_current(true, &state, cx);
                self.feedback.set_status(format!(
                    "{current} was deleted; this buffer is no longer saved anywhere"
                ));
            } else if changes.files.iter().any(|c| c.path() == current) {
                // TFT's own autosave echoes back a moment later; rebuilding would
                // re-enter `show_file` and re-expand the Files list every few seconds.
                if !self.is_own_write(&current, &changes, &state, cx) {
                    self.show_file(&current, &state, cx);
                }
            }
        } else if let Some(detached) = self.active_group_state.detached.clone() {
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

    pub fn is_loaded(&self) -> bool {
        self.vault_session.state.is_some()
    }

    /// "Reload app without saving", in the vault window: the vault, config, theme,
    /// hotkeys and plugins are read again; unsaved edits are dropped and the layout
    /// kept ([[Specification#Observed app reload]]).
    pub(super) fn reload_app(&mut self, cx: &mut Context<Self>) {
        if self.vault_session.state.is_none() {
            return;
        }
        let this = cx.entity();
        let main = self.main_window.handle();
        cx.defer(move |cx| {
            main.update(cx, |_, window, cx| {
                this.update(cx, |ws, cx| ws.reload_now(window, cx))
            })
            .ok();
        });
    }

    fn reload_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let layout = self.layout_json();
        // Without saving: the pending autosave is dropped and the closing groups write nothing.
        self.document_save.cancel();
        self.active_group_state.current = None;
        self.forget_vault(cx);
        self.active_group_state.history.clear();
        self.active_group_state.history_ix = 0;
        self.layout_persistence.set_pending_reload(layout);
        let record = self.vault_session.record;
        self.load_vault_with(state.root.clone(), record, window, cx);
    }
}
