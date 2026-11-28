//! A Web-hosted GPUI composition over TFT's shared vault and workspace models.
//! Storage and authentication are supplied by the host; editing stays in the
//! production Editor and document bytes live in `VaultData`.

use gpui::{
    canvas, div, fill, img, point, prelude::*, px, size, Action, App, Bounds, ClipboardItem,
    Context, DispatchPhase, DragMoveEvent, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, HitboxBehavior, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    PathBuilder, Pixels, Point, Render, ScrollWheelEvent, SharedString, SharedUri, StyledText,
    Subscription, UTF16Selection, Window,
};
use radial_tree_explorer::{RadialTree, RadialTreeEvent, TreeViewOptions};
use std::cell::RefCell;
#[cfg(feature = "browser-test")]
use std::collections::BTreeSet;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tft_app::command_palette::{CommandPalette, PaletteEvent};
use tft_app::editor::Editor;
use tft_app::explorer::{FileTree, SortOrder};
use tft_app::pdf::view::PdfView;
use tft_app::reading::{
    self, EmbedSource, PropertyItemRemoved, ReadingEvent, ReadingView, TaskToggled,
};
use tft_app::text_input::{InputEvent, TextInput};
use tft_app::theme::Theme;
use tft_app::{
    BackendObjectId, FileMeta, FileOperationRequest, Layout, LeftTab, NodeId, NodeKind,
    SplitDirection, Tab, TabGroup, VaultData, ViewMode, WorkspaceModel,
};
use tft_cache::LinkResolver;
#[cfg(feature = "browser-test")]
use tft_formats::files::VaultFile;

#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = web_workspace, no_json)]
struct RunWebCommand {
    id: String,
    binding: Option<String>,
}

const WEB_COMMANDS: &[&str] = &[
    "app:go-back",
    "app:go-forward",
    "app:toggle-left-sidebar",
    "editor:delete-paragraph",
    "editor:follow-link",
    "editor:insert-link",
    "editor:open-link-in-new-leaf",
    "editor:open-link-in-new-split",
    "editor:open-search",
    "editor:save-file",
    "editor:toggle-bold",
    "editor:toggle-checklist-status",
    "editor:toggle-comments",
    "editor:toggle-italics",
    "file-explorer:new-file",
    "global-search:open",
    "graph:open",
    "markdown:add-metadata-property",
    "markdown:toggle-preview",
    "switcher:open",
    "workspace:close",
    "workspace:edit-file-title",
    "workspace:goto-last-tab",
    "workspace:goto-tab-1",
    "workspace:goto-tab-2",
    "workspace:goto-tab-3",
    "workspace:goto-tab-4",
    "workspace:goto-tab-5",
    "workspace:goto-tab-6",
    "workspace:goto-tab-7",
    "workspace:goto-tab-8",
    "workspace:new-tab",
    "workspace:next-tab",
    "workspace:previous-tab",
];

fn browser_keystrokes(hotkey: &tft_app::formats::config::Hotkey) -> Vec<String> {
    let has_mod = hotkey.modifiers.iter().any(|modifier| modifier == "Mod");
    let variants = if has_mod {
        ["Meta", "Ctrl"]
            .into_iter()
            .map(|modifier| {
                let mut variant = hotkey.clone();
                for item in &mut variant.modifiers {
                    if item == "Mod" {
                        *item = modifier.into();
                    }
                }
                variant
            })
            .collect::<Vec<_>>()
    } else {
        vec![hotkey.clone()]
    };
    variants
        .iter()
        .filter_map(tft_app::commands::keystroke)
        .collect()
}

pub fn bind_keys(cx: &mut App) {
    let mut bindings = Vec::new();
    for (id, hotkeys) in tft_app::commands::default_hotkeys() {
        if !WEB_COMMANDS.contains(&id) {
            continue;
        }
        for hotkey in hotkeys {
            for keys in browser_keystrokes(&hotkey) {
                bindings.push(KeyBinding::new(
                    &keys,
                    RunWebCommand {
                        id: id.into(),
                        binding: Some(keys.clone()),
                    },
                    None,
                ));
            }
        }
    }
    // The palette is a workspace accelerator rather than a registered command.
    for keys in ["cmd-shift-p", "ctrl-shift-p"] {
        bindings.push(KeyBinding::new(
            keys,
            RunWebCommand {
                id: "command-palette:open".into(),
                binding: None,
            },
            None,
        ));
    }
    cx.bind_keys(bindings);
}

#[derive(Clone)]
struct SearchHit {
    node: NodeId,
    path: String,
    name: String,
    name_ranges: Vec<Range<usize>>,
    snippets: Vec<tft_search::MatchLine>,
    hits: Vec<Range<usize>>,
    contents: Arc<String>,
    match_count: usize,
    mtime: u64,
    ctime: u64,
}

struct SearchDocument {
    node: NodeId,
    path: String,
    contents: Arc<String>,
    metadata: tft_cache::CachedMetadata,
    mtime: u64,
    ctime: u64,
}

#[derive(Clone)]
struct QuickSwitcherHit {
    node: NodeId,
    path: String,
    label: String,
    alias: bool,
    ranges: Vec<Range<usize>>,
    downranked: bool,
}

#[derive(Clone)]
struct FileTreeContextMenu {
    position: Point<Pixels>,
    node: Option<NodeId>,
    key: String,
    path: String,
    is_folder: bool,
}

#[derive(Clone)]
struct TabContextMenu {
    position: Point<Pixels>,
    group: u64,
    index: usize,
}

#[derive(Clone)]
struct EditorContextMenu {
    group_id: u64,
    editor: bool,
    position: Point<Pixels>,
    selection: String,
    selected: bool,
    link: Option<reading::LinkTarget>,
    link_path: Option<String>,
    drive_id: Option<String>,
}

#[derive(Clone, Copy)]
enum EditorContextAction {
    OpenInNewTab,
    OpenToRight,
    AddLink,
    AddExternalLink,
    SearchSelection,
    Cut,
    Copy,
    Paste,
    PastePlain,
    SelectAll,
    CopyPath,
    RenameFile,
    MoveFile,
    OpenInDrive,
    RevealInNavigation,
}

#[derive(Clone)]
struct DockResizeDrag;

#[derive(Clone)]
struct SplitResizeDrag {
    path: Vec<usize>,
    before: usize,
}

#[derive(Clone)]
struct WebCanvasDrag {
    file: NodeId,
    node_id: String,
    last: Point<Pixels>,
    scale: f64,
    resize_handle: Option<tft_app::canvas_core::Handle>,
}

#[derive(Clone, Copy)]
struct WebCanvasViewport {
    zoom: f32,
    pan: (f32, f32),
}

impl Default for WebCanvasViewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
        }
    }
}

struct WebCanvasCardEditor {
    file: NodeId,
    id: String,
    editor: Entity<Editor>,
    _subscription: Subscription,
}

#[derive(Clone)]
struct WebFilePreview {
    node: NodeId,
    path: String,
    anchor: Bounds<Pixels>,
    offset: Option<usize>,
    depth: usize,
}

struct WebPreviewEditor {
    editor: Entity<Editor>,
    _subscription: Subscription,
}

const WEB_PREVIEW_DEPTH_LIMIT: usize = 8;

fn place_web_preview_caret(
    editor: gpui::WeakEntity<Editor>,
    position: Point<Pixels>,
    tries: usize,
    window: &mut Window,
) {
    window.on_next_frame(move |window, cx| {
        let placed = editor
            .update(cx, |editor, cx| editor.caret_at_point(position, window, cx))
            .unwrap_or(true);
        if !placed && tries > 0 {
            place_web_preview_caret(editor, position, tries - 1, window);
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebGraphSlider {
    Depth,
    TextFade,
    NodeSize,
    LinkSize,
    Center,
    Repel,
    Link,
    LinkDistance,
}

impl WebGraphSlider {
    fn label(self) -> &'static str {
        match self {
            Self::Depth => "Depth",
            Self::TextFade => "Text fade threshold",
            Self::NodeSize => "Node size",
            Self::LinkSize => "Link thickness",
            Self::Center => "Center force",
            Self::Repel => "Repel force",
            Self::Link => "Link force",
            Self::LinkDistance => "Link distance",
        }
    }

    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::Depth => (1.0, 5.0, 1.0),
            Self::TextFade => (-3.0, 3.0, 0.1),
            Self::NodeSize | Self::LinkSize => (0.1, 5.0, 0.1),
            Self::Center => (0.0, 1.0, 0.01),
            Self::Repel => (0.0, 20.0, 0.1),
            Self::Link => (0.0, 1.0, 0.01),
            Self::LinkDistance => (30.0, 500.0, 1.0),
        }
    }

    fn value(self, config: &tft_formats::config::GraphConfig, local: WebLocalGraphOptions) -> f32 {
        let defaults = tft_app::graph_layout::Forces::default();
        match self {
            Self::Depth => local.jumps as f32,
            Self::TextFade => config.text_fade_multiplier.unwrap_or(0.0),
            Self::NodeSize => config.node_size_multiplier.unwrap_or(1.0),
            Self::LinkSize => config.line_size_multiplier.unwrap_or(1.0),
            Self::Center => config.center_strength.unwrap_or(defaults.center),
            Self::Repel => config.repel_strength.unwrap_or(defaults.repel),
            Self::Link => config.link_strength.unwrap_or(defaults.link),
            Self::LinkDistance => config.link_distance.unwrap_or(defaults.link_distance),
        }
    }

    fn set(
        self,
        config: &mut tft_formats::config::GraphConfig,
        local: &mut WebLocalGraphOptions,
        value: f32,
    ) {
        if self == Self::Depth {
            local.jumps = value.round().clamp(1.0, 5.0) as u32;
            return;
        }
        let slot = match self {
            Self::Depth => unreachable!(),
            Self::TextFade => &mut config.text_fade_multiplier,
            Self::NodeSize => &mut config.node_size_multiplier,
            Self::LinkSize => &mut config.line_size_multiplier,
            Self::Center => &mut config.center_strength,
            Self::Repel => &mut config.repel_strength,
            Self::Link => &mut config.link_strength,
            Self::LinkDistance => &mut config.link_distance,
        };
        *slot = Some(value);
    }

    fn value_at(self, fraction: f32) -> f32 {
        let (min, max, step) = self.range();
        let steps = ((min + fraction.clamp(0.0, 1.0) * (max - min) - min) / step).round();
        ((min + steps * step).clamp(min, max) * 100.0).round() / 100.0
    }
}

#[derive(Clone, Copy)]
struct WebLocalGraphOptions {
    jumps: u32,
    backlinks: bool,
    forelinks: bool,
    interlinks: bool,
}

impl Default for WebLocalGraphOptions {
    fn default() -> Self {
        Self {
            jumps: 1,
            backlinks: true,
            forelinks: true,
            interlinks: false,
        }
    }
}

const WEB_GRAPH_GROUP_COLORS: &[u32] = &[
    0xe93147, 0xec7500, 0xe0ac00, 0x08b94e, 0x00bfbc, 0x086dff, 0x7852ee, 0xd53984,
];

struct EmptyDragPreview;

#[derive(Clone)]
struct TabReorderDrag {
    source: tft_app::TabDragSource,
    title: SharedString,
}

struct TabDragPreview {
    title: SharedString,
}

impl Render for EmptyDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        div()
            .px_3()
            .py_2()
            .rounded(px(4.))
            .bg(theme.tokens.tab_background_active)
            .text_color(theme.tokens.text_normal)
            .text_size(px(13.))
            .child(self.title.clone())
    }
}

#[cfg(feature = "browser-test")]
#[derive(Clone)]
pub struct NoteDocument {
    pub path: String,
    pub contents: String,
}

/// Shared Files / tabs / Editor composition. The host supplies the initial
/// snapshot and owns persistence; this view updates the shared in-memory model.
struct GroupViews {
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    pdf: Entity<PdfView>,
}

struct WebTitleEdit {
    group: u64,
    path: String,
    editing: tft_app::inline_field::Editing,
}

struct WebTitleEditing {
    focus: FocusHandle,
    editing: Option<WebTitleEdit>,
    layouts: HashMap<u64, tft_app::inline_field::Layout>,
}

struct WebRadialTree {
    view: Entity<RadialTree>,
    _subscription: Subscription,
}

static RADIAL_TREE_FONTS_REGISTERED: OnceLock<()> = OnceLock::new();

pub struct WebWorkspace {
    graph_data: Arc<tft_app::graph_data::GraphData>,
    graph_layout: Arc<Mutex<tft_app::graph_layout::Layout>>,
    graph_config: tft_formats::config::GraphConfig,
    graph_previous_modes: HashMap<u64, (Option<NodeId>, Option<String>, ViewMode)>,
    radial_trees: HashMap<u64, WebRadialTree>,
    radial_tree_active: std::collections::HashSet<u64>,
    pending_radial_open: Option<(u64, String)>,
    graph_panel_open: bool,
    graph_search_input: Entity<TextInput>,
    graph_group_input: Entity<TextInput>,
    graph_editing_group: Option<usize>,
    graph_local_center: Option<String>,
    graph_local_options: WebLocalGraphOptions,
    graph_simulation_started: std::cell::Cell<bool>,
    graph_refresh_generation: u64,
    graph_scale: std::cell::Cell<f32>,
    graph_offset: std::cell::Cell<(f32, f32)>,
    graph_drag_start: std::cell::Cell<Option<Point<Pixels>>>,
    graph_drag_moved: std::cell::Cell<bool>,
    graph_hovered: std::cell::Cell<Option<usize>>,
    graph_slider_drag: std::cell::Cell<Option<WebGraphSlider>>,
    group_views: std::collections::HashMap<u64, GroupViews>,
    title_editing: WebTitleEditing,
    command_palette: Entity<CommandPalette>,
    command_palette_open: bool,
    recent_commands: Vec<String>,
    file_preview_views: Vec<Entity<ReadingView>>,
    file_preview_pending: Option<WebFilePreview>,
    file_previews: Vec<Option<WebFilePreview>>,
    preview_editors: HashMap<usize, WebPreviewEditor>,
    file_preview_generation: u64,
    file_preview_hide_generation: u64,
    file_preview_pointer_over: Vec<bool>,
    hovered_preview_link: Option<(
        reading::LinkHover,
        String,
        tft_app::page_preview::Source,
        usize,
    )>,
    hovered_preview_file: Option<(WebFilePreview, bool)>,
    search_input: Entity<TextInput>,
    quick_switcher_input: Entity<TextInput>,
    quick_switcher_open: bool,
    quick_switcher_selected: usize,
    pending_quick_switcher_focus_restore: bool,
    pending_quick_switcher_open: Option<(NodeId, String)>,
    search_hits: Vec<SearchHit>,
    selected_search_hit: Option<usize>,
    selected_search_match: Option<(usize, usize)>,
    search_scroll_handles: RefCell<HashMap<Option<u64>, gpui::ScrollHandle>>,
    search_matching_case: bool,
    search_extra_context: bool,
    search_explain: bool,
    search_settings_open: bool,
    search_sort: SortOrder,
    search_sort_menu_open: bool,
    search_collapse_all: bool,
    search_toggled_paths: std::collections::HashSet<String>,
    pending_search_open: Option<(usize, Option<usize>)>,
    search_error: Option<String>,
    search_generation: u64,
    search_documents: Vec<SearchDocument>,
    recent_files: Vec<String>,
    excluded: tft_search::ExcludeFilters,
    switcher_config: tft_formats::config::SwitcherConfig,
    app_config: tft_formats::config::AppConfig,
    page_preview_config: tft_formats::config::PagePreviewConfig,
    hotkeys: tft_formats::config::Hotkeys,
    media_previews: HashMap<NodeId, String>,
    media_preview_loading: std::collections::HashSet<NodeId>,
    media_preview_errors: HashMap<NodeId, String>,
    canvas_scenes: HashMap<NodeId, tft_app::canvas_core::Scene>,
    canvas_origin: std::rc::Rc<std::cell::Cell<(f32, f32)>>,
    canvas_viewports: HashMap<NodeId, WebCanvasViewport>,
    canvas_background_drag: Option<(NodeId, Point<Pixels>)>,
    canvas_loading: std::collections::HashSet<NodeId>,
    canvas_errors: HashMap<NodeId, String>,
    canvas_saving: std::collections::HashSet<NodeId>,
    canvas_drag: Option<WebCanvasDrag>,
    canvas_card_editor: Option<WebCanvasCardEditor>,
    audio_ticking: std::collections::HashSet<NodeId>,
    pdf_loading: std::collections::HashSet<NodeId>,
    pdf_loaded: std::collections::HashSet<NodeId>,
    pdf_errors: HashMap<NodeId, String>,
    vault: VaultData,
    tree: FileTree,
    tree_selection: tft_app::explorer::TreeSelection,
    tree_focus: FocusHandle,
    tree_focused_key: Option<String>,
    file_tree_context_menu: Option<FileTreeContextMenu>,
    tab_context_menu: Option<TabContextMenu>,
    editor_context_menu: Option<EditorContextMenu>,
    dock_width: f32,
    dock_dragging: bool,
    /// A narrow viewport shows the dock as an overlay instead of letting it
    /// squeeze the document.
    narrow_dock_open: bool,
    tab_drop_boundary: Option<usize>,
    split_dragging: Option<(Vec<usize>, usize)>,
    model: WorkspaceModel,
    closed_tab_history: tft_app::workspace::ClosedTabHistory,
    vault_name: String,
    drive_controls: bool,
    pending_link: Option<(reading::LinkTarget, bool)>,
    pending_link_source: Option<String>,
    last_link_event: String,
    recent_group_order: Vec<u64>,
    group_subscriptions: std::collections::HashMap<u64, Vec<Subscription>>,
    _subscriptions: Vec<Subscription>,
}

pub enum WebWorkspaceEvent {
    Edited {
        node: NodeId,
        contents: String,
    },
    WorkspaceStateChanged(String),
    FileOperationRequested(FileOperationRequest),
    CreateCanvasRequested {
        parent: String,
    },
    CreateFileFromLink {
        parent: String,
        name: String,
        kind: tft_formats::FileKind,
    },
    CanvasEdgeLabelRequested {
        file: NodeId,
        edge_id: String,
        label: String,
    },
    SaveRequested,
    SwitchVaultRequested,
    RenameFileRequested {
        from: String,
        to: String,
    },
    RenameFilePromptRequested {
        path: String,
    },
    MoveFilePromptRequested {
        path: String,
    },
    RenameHeadingRequested {
        path: String,
        line: u32,
        initial: String,
    },
    DuplicateFileRequested {
        path: String,
    },
    DeleteFileRequested {
        path: String,
    },
}

impl EventEmitter<WebWorkspaceEvent> for WebWorkspace {}

impl WebWorkspace {
    fn hotkey_is_active(&self, id: &str, binding: &str) -> bool {
        let configured = self
            .hotkeys
            .get(id)
            .cloned()
            .or_else(|| tft_app::commands::default_hotkeys().remove(id));
        configured.is_some_and(|hotkeys| {
            hotkeys
                .iter()
                .any(|hotkey| browser_keystrokes(hotkey).iter().any(|key| key == binding))
        })
    }

    #[cfg(feature = "browser-test")]
    pub fn from_vault(
        vault: VaultData,
        initial_node: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::from_vault_with_state(
            vault,
            initial_node,
            None,
            tft_formats::config::AppConfig::default(),
            tft_formats::config::SwitcherConfig::default(),
            tft_formats::config::PagePreviewConfig::default(),
            tft_formats::config::GraphConfig::default(),
            tft_formats::config::Hotkeys::new(),
            window,
            cx,
        )
    }

    pub fn from_vault_with_state(
        vault: VaultData,
        initial_node: Option<NodeId>,
        saved_state: Option<String>,
        app_config: tft_formats::config::AppConfig,
        switcher_config: tft_formats::config::SwitcherConfig,
        page_preview_config: tft_formats::config::PagePreviewConfig,
        graph_config: tft_formats::config::GraphConfig,
        hotkeys: tft_formats::config::Hotkeys,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let restored_state = saved_state
            .as_deref()
            .and_then(|state| serde_json::from_str::<serde_json::Value>(state).ok());
        let canvas_viewports = restored_state
            .as_ref()
            .and_then(|state| state.get("canvasViewports"))
            .and_then(serde_json::Value::as_object)
            .map(|viewports| {
                viewports
                    .iter()
                    .filter_map(|(path, value)| {
                        let node = vault.lookup(path).ok()?;
                        let zoom = value
                            .get("zoom")
                            .and_then(serde_json::Value::as_f64)
                            .filter(|zoom| zoom.is_finite())
                            .unwrap_or(1.0)
                            .clamp(0.25, 4.0) as f32;
                        let pan = value.get("pan").and_then(serde_json::Value::as_array);
                        let pan_x = pan
                            .and_then(|pan| pan.first())
                            .and_then(serde_json::Value::as_f64)
                            .filter(|value| value.is_finite())
                            .unwrap_or(0.0)
                            .clamp(-10_000.0, 10_000.0) as f32;
                        let pan_y = pan
                            .and_then(|pan| pan.get(1))
                            .and_then(serde_json::Value::as_f64)
                            .filter(|value| value.is_finite())
                            .unwrap_or(0.0)
                            .clamp(-10_000.0, 10_000.0) as f32;
                        Some((
                            node,
                            WebCanvasViewport {
                                zoom,
                                pan: (pan_x, pan_y),
                            },
                        ))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let graph_panel_open = restored_state
            .as_ref()
            .and_then(|state| state.get("graphPanelOpen"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let graph_local_center = restored_state
            .as_ref()
            .and_then(|state| state.get("graphLocalCenter"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .filter(|path| vault.lookup(path).is_ok());
        let graph_local_options = restored_state
            .as_ref()
            .and_then(|state| state.get("graphLocalOptions"));
        let graph_local_options = WebLocalGraphOptions {
            jumps: graph_local_options
                .and_then(|state| state.get("jumps"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 5) as u32,
            backlinks: graph_local_options
                .and_then(|state| state.get("backlinks"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            forelinks: graph_local_options
                .and_then(|state| state.get("forelinks"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            interlinks: graph_local_options
                .and_then(|state| state.get("interlinks"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        };
        let mut tree = FileTree::from_vault(&vault);
        tree.expand_all();
        let mut model = WorkspaceModel::new(ViewMode::default_editing());
        let mut recent_files = Vec::new();
        let mut restored_active = None;
        let mut restored_tabs = false;
        let mut dock_width = tft_app::theme::SIDEBAR_WIDTH;
        if let Some(state) =
            saved_state.and_then(|state| serde_json::from_str::<serde_json::Value>(&state).ok())
        {
            if let Some(tabs) = state.get("tabs").and_then(serde_json::Value::as_array) {
                let restored = restore_web_tabs(&vault, tabs);
                if !restored.is_empty() {
                    restored_tabs = true;
                    let active = state
                        .get("activeTab")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as usize;
                    let group = model
                        .active_mut()
                        .expect("new workspace has an active group");
                    group.tabs = restored;
                    group.active_tab = active.min(group.tabs.len() - 1);
                    restored_active = group.active().and_then(|tab| tab.node);
                }
            }
            if let Some(saved_groups) = state.get("groups").and_then(serde_json::Value::as_array) {
                let mut restored_groups = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for saved in saved_groups {
                    let Some(id) = saved.get("id").and_then(serde_json::Value::as_u64) else {
                        continue;
                    };
                    if !seen.insert(id) {
                        continue;
                    }
                    let Some(saved_tabs) = saved.get("tabs").and_then(serde_json::Value::as_array)
                    else {
                        continue;
                    };
                    let mut tabs = restore_web_tabs(&vault, saved_tabs);
                    if tabs.is_empty() {
                        tabs.push(Tab::empty(ViewMode::default_editing()));
                    }
                    let mut group = TabGroup::new(id, ViewMode::default_editing());
                    group.active_tab = saved
                        .get("activeTab")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as usize;
                    group.active_tab = group.active_tab.min(tabs.len() - 1);
                    group.tabs = tabs;
                    restored_groups.push(group);
                }
                if !restored_groups.is_empty() {
                    let ids = restored_groups
                        .iter()
                        .map(|group| group.id)
                        .collect::<Vec<_>>();
                    let restored_layout = state
                        .get("layout")
                        .and_then(parse_web_layout)
                        .filter(|layout| layout_matches_group_ids(layout, &ids))
                        .map(|layout| match layout {
                            Layout::Group(id) => {
                                Layout::split(SplitDirection::Vertical, vec![Layout::Group(id)])
                            }
                            layout => layout,
                        });
                    model.layout = restored_layout.unwrap_or_else(|| {
                        Layout::split(
                            SplitDirection::Vertical,
                            ids.iter().copied().map(Layout::Group).collect(),
                        )
                    });
                    model.groups = restored_groups;
                    let restored_active_group = state
                        .get("activeGroup")
                        .and_then(serde_json::Value::as_u64)
                        .filter(|id| ids.contains(id))
                        .unwrap_or(ids[0]);
                    model.activate_group(restored_active_group);
                    restored_tabs = true;
                    restored_active = model
                        .active()
                        .and_then(|group| group.active())
                        .and_then(|tab| tab.node);
                }
            }
            model.dock.visible = state
                .get("leftVisible")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            dock_width = state
                .get("dockWidth")
                .and_then(serde_json::Value::as_f64)
                .map(|width| width.clamp(150.0, 600.0) as f32)
                .unwrap_or(dock_width);
            model.dock.tab = match state.get("leftTab").and_then(serde_json::Value::as_str) {
                Some("search") => LeftTab::Search,
                _ => LeftTab::Files,
            };
            if let Some(order) = state
                .get("sortOrder")
                .and_then(serde_json::Value::as_str)
                .and_then(SortOrder::parse)
            {
                tree.set_sort(order);
            }
            if let Some(folders) = state
                .get("expandedFolders")
                .and_then(serde_json::Value::as_array)
            {
                let folders = folders
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect();
                tree.expand_folders(&folders);
            }
            recent_files = state
                .get("recentFiles")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .filter(|path| vault.lookup(path).is_ok())
                .take(50)
                .map(str::to_owned)
                .collect();
        }
        let active_node = if restored_tabs {
            restored_active
        } else {
            initial_node
        };
        let editor = cx.new(|cx| Editor::new(window, cx));
        let reading = cx.new(ReadingView::new);
        let pdf = cx.new(PdfView::new);
        let group_id = model.active_group;
        let mut group_views = std::collections::HashMap::new();
        for group in model.groups.iter().filter(|group| group.id != group_id) {
            let editor = cx.new(|cx| Editor::new(window, cx));
            let reading = cx.new(ReadingView::new);
            let pdf = cx.new(PdfView::new);
            if let Some(tab) = group.active() {
                let path = tab.path.clone().unwrap_or_default();
                let text = tab
                    .node
                    .map_or_else(String::new, |node| document_text(&vault, node));
                let offset = tab
                    .history
                    .get(tab.history_ix.saturating_sub(1))
                    .map(|entry| entry.offset)
                    .unwrap_or(0);
                editor.update(cx, |editor, cx| {
                    editor.set_workspace_document(path.clone(), text.clone(), tab.mode, cx);
                    editor.reveal_offset(offset, cx);
                });
                if tab.mode == ViewMode::Reading {
                    if let Some(node) = tab.node {
                        let source = VaultEmbedSource::new(&vault);
                        let model = reading::build_with(
                            &path,
                            &text,
                            &source,
                            reading::BuildOptions::default(),
                        );
                        reading.update(cx, |reader, cx| reader.set_note(Some(path), model, cx));
                        let _ = node;
                    }
                }
            }
            group_views.insert(
                group.id,
                GroupViews {
                    editor,
                    reading,
                    pdf,
                },
            );
        }
        let search_input = cx.new(|cx| TextInput::new("Search vault…", cx));
        let quick_switcher_input = cx.new(|cx| TextInput::new("Search files by name…", cx));
        let command_palette = cx.new(CommandPalette::new);
        let file_preview_views = (0..WEB_PREVIEW_DEPTH_LIMIT)
            .map(|_| cx.new(ReadingView::new))
            .collect::<Vec<_>>();
        let graph_search_input = cx.new(|cx| TextInput::new("Search files…", cx));
        graph_search_input.update(cx, |input, cx| {
            input.set_text(graph_config.search.clone().unwrap_or_default(), cx)
        });
        let graph_group_input = cx.new(|cx| TextInput::new("Enter query…", cx));
        let tree_focus = cx.focus_handle();
        let title_focus = cx.focus_handle();
        let search_documents = build_search_index(&vault);
        let excluded = tft_search::ExcludeFilters::new(app_config.ignore_filters());
        let (graph_data, graph_layout) = build_web_graph(
            &vault,
            &search_documents,
            &excluded,
            &graph_config,
            graph_local_center.as_deref(),
            graph_local_options,
        );
        let subscriptions = vec![
            cx.subscribe_in(
                &command_palette,
                window,
                |this, _, event: &PaletteEvent, window, cx| match event {
                    PaletteEvent::Run(id) => {
                        this.command_palette_open = false;
                        this.recent_commands.retain(|recent| recent != id);
                        this.recent_commands.insert(0, id.clone());
                        this.recent_commands.truncate(20);
                        match id.as_str() {
                            "switcher:open" => this.open_quick_switcher(window, cx),
                            "workspace:new-tab" => this.new_tab(cx),
                            "workspace:close" => {
                                let index = this.model.active().map(|group| group.active_tab);
                                if let Some(index) = index {
                                    let pinned = this
                                        .model
                                        .active()
                                        .and_then(|group| group.tabs.get(index))
                                        .is_some_and(|tab| tab.pinned);
                                    if pinned {
                                        this.toggle_tab_pin(index, cx);
                                    } else {
                                        this.close_tab(index, window, cx);
                                    }
                                }
                            }
                            "workspace:close-others" | "workspace:close-others-tab-group" => {
                                let active_index =
                                    this.model.active().map(|group| group.active_tab);
                                this.close_tabs_except(
                                    |index, tab| Some(index) == active_index || tab.pinned,
                                    window,
                                    cx,
                                );
                            }
                            "workspace:copy-path" => {
                                if let Some(path) = this.current_path().map(str::to_owned) {
                                    cx.write_to_clipboard(ClipboardItem::new_string(path));
                                    cx.notify();
                                }
                            }
                            "canvas:new-file" => {
                                cx.emit(WebWorkspaceEvent::CreateCanvasRequested {
                                    parent: String::new(),
                                });
                            }
                            "app:switch-vault" => {
                                cx.emit(WebWorkspaceEvent::SwitchVaultRequested);
                            }
                            "file-explorer:move-file" => {
                                if let Some(path) = this.current_path().map(str::to_owned) {
                                    cx.emit(WebWorkspaceEvent::MoveFilePromptRequested { path });
                                }
                            }
                            "editor:context-menu" => {
                                if this.can_edit_active_markdown() {
                                    this.active_editor()
                                        .update(cx, |editor, cx| editor.context_menu_at_caret(cx));
                                }
                            }
                            "editor:focus" => this.focus_editor(window, cx),
                            "open-with-default-app:open" | "open-with-default-app:show" => {
                                if let Some(id) = this.active_drive_id() {
                                    tft_drive_web::open_drive_file(&id);
                                }
                            }
                            "workspace:undo-close-pane" => this.undo_close_tab(window, cx),
                            "workspace:toggle-pin" => {
                                let index = this.model.active().map(|group| group.active_tab);
                                if let Some(index) = index {
                                    this.toggle_tab_pin(index, cx);
                                }
                            }
                            "workspace:split-vertical" => {
                                this.split_active_group(SplitDirection::Vertical, window, cx)
                            }
                            "workspace:split-horizontal" => {
                                this.split_active_group(SplitDirection::Horizontal, window, cx)
                            }
                            "workspace:close-tab-group" => this.close_active_group(window, cx),
                            "editor:focus-left" => {
                                this.focus_neighbor(SplitDirection::Vertical, false, window, cx)
                            }
                            "editor:focus-right" => {
                                this.focus_neighbor(SplitDirection::Vertical, true, window, cx)
                            }
                            "editor:focus-top" => {
                                this.focus_neighbor(SplitDirection::Horizontal, false, window, cx)
                            }
                            "editor:focus-bottom" => {
                                this.focus_neighbor(SplitDirection::Horizontal, true, window, cx)
                            }
                            "global-search:open" => {
                                let query = this.search_input.read(cx).text().to_owned();
                                this.show_search(&query, window, cx);
                            }
                            "app:go-back" => this.navigate_history(false, window, cx),
                            "app:go-forward" => this.navigate_history(true, window, cx),
                            "markdown:toggle-preview" => this.toggle_view(window, cx),
                            "editor:toggle-source" => this.toggle_source_view(window, cx),
                            "workspace:edit-file-title" => this.edit_active_title(window, cx),
                            "editor:rename-heading" => this.request_rename_heading(cx),
                            "markdown:add-metadata-property"
                            | "markdown:add-alias"
                            | "markdown:clear-metadata-properties"
                            | "editor:toggle-fold-properties" => {
                                this.run_metadata_command(id, window, cx)
                            }
                            id if tft_app::text::table::TableCommand::from_id(id).is_some() => {
                                this.run_table_command(id, window, cx)
                            }
                            "graph:open" => {
                                this.toggle_graph_view(this.model.active_group, window, cx)
                            }
                            "graph:open-local" => {
                                this.open_local_graph(this.model.active_group, window, cx)
                            }
                            "editor:save-file" => cx.emit(WebWorkspaceEvent::SaveRequested),
                            "editor:open-search" => this
                                .active_editor()
                                .update(cx, |editor, cx| editor.open_find("", false, window, cx)),
                            "editor:open-search-replace" => {
                                this.active_editor().update(cx, |editor, cx| {
                                    editor.open_find("", true, window, cx)
                                });
                            }
                            id if Self::is_markdown_editor_command(id) => {
                                this.run_markdown_editor_command(id, window, cx)
                            }
                            "editor:follow-link" => {
                                this.follow_editor_link(false, false, window, cx)
                            }
                            "editor:open-link-in-new-leaf" => {
                                this.follow_editor_link(true, false, window, cx)
                            }
                            "editor:open-link-in-new-split" => {
                                this.follow_editor_link(false, true, window, cx)
                            }
                            "file-explorer:open" => this.open_file_explorer(false, window, cx),
                            "file-explorer:reveal-active-file" => {
                                this.open_file_explorer(true, window, cx)
                            }
                            "file-explorer:new-file" => this.request_new_note(cx),
                            "file-explorer:new-folder" => {
                                cx.emit(WebWorkspaceEvent::FileOperationRequested(
                                    FileOperationRequest::CreateFolder {
                                        parent: String::new(),
                                    },
                                ));
                            }
                            "file-explorer:duplicate-file" => {
                                if let Some(path) = this.current_path().map(str::to_owned) {
                                    cx.emit(WebWorkspaceEvent::DuplicateFileRequested { path });
                                }
                            }
                            "app:delete-file" => {
                                if let Some(path) = this.current_path().map(str::to_owned) {
                                    cx.emit(WebWorkspaceEvent::DeleteFileRequested { path });
                                }
                            }
                            "app:toggle-left-sidebar" => this.toggle_left_dock(cx),
                            _ => {}
                        }
                        cx.notify();
                    }
                    PaletteEvent::Close => {
                        this.command_palette_open = false;
                        this.focus_editor(window, cx);
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &quick_switcher_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => {
                        this.quick_switcher_selected = 0;
                        cx.notify();
                    }
                    InputEvent::Down => {
                        let count = this.quick_switcher_results(cx).len();
                        if count > 0 {
                            this.quick_switcher_selected = tft_app::selection_step(
                                Some(this.quick_switcher_selected),
                                count,
                                1,
                                true,
                            )
                            .unwrap_or(0);
                            cx.notify();
                        }
                    }
                    InputEvent::Up => {
                        let count = this.quick_switcher_results(cx).len();
                        if count > 0 {
                            this.quick_switcher_selected = tft_app::selection_step(
                                Some(this.quick_switcher_selected),
                                count,
                                -1,
                                true,
                            )
                            .unwrap_or(0);
                            cx.notify();
                        }
                    }
                    InputEvent::Submit => this.open_quick_switcher_selection(cx),
                    InputEvent::Cancel => {
                        this.quick_switcher_open = false;
                        this.pending_quick_switcher_focus_restore = true;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &search_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.rerun_search(cx),
                    InputEvent::Down if !this.search_hits.is_empty() => {
                        this.move_search_focus(1, cx);
                    }
                    InputEvent::Up if !this.search_hits.is_empty() => {
                        this.move_search_focus(-1, cx);
                    }
                    InputEvent::Submit if !this.search_hits.is_empty() => {
                        if let Some(selected) = this.selected_search_hit {
                            let offset = this.selected_search_match.and_then(|(hit, snippet)| {
                                (hit == selected)
                                    .then(|| {
                                        this.search_hits
                                            .get(hit)?
                                            .snippets
                                            .get(snippet)
                                            .map(|line| line.offset)
                                    })
                                    .flatten()
                            });
                            this.pending_search_open = Some((selected, offset));
                        }
                        cx.notify();
                    }
                    InputEvent::Cancel => {
                        if this.selected_search_hit.take().is_some() {
                            this.selected_search_match = None;
                            cx.notify();
                        } else if this.model.dock.tab == LeftTab::Search {
                            this.model.dock.select(LeftTab::Files);
                            cx.notify();
                            this.emit_workspace_state(cx);
                        }
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &graph_group_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.commit_graph_group(cx),
                    InputEvent::Cancel => {
                        this.graph_editing_group = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &graph_search_input,
                |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Changed) {
                        let query = input.read(cx).text().trim().to_owned();
                        this.graph_config.search = (!query.is_empty()).then_some(query);
                        this.schedule_graph_rebuild(cx, true);
                    }
                },
            ),
        ];
        let mut subscriptions = subscriptions;
        for (depth, preview_view) in file_preview_views.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                preview_view,
                window,
                move |this, _, event: &ReadingEvent, window, cx| {
                    let Some(source_path) = this
                        .file_previews
                        .get(depth)
                        .and_then(Option::as_ref)
                        .map(|item| item.path.clone())
                    else {
                        return;
                    };
                    match event {
                        ReadingEvent::OpenLink(target) => {
                            this.hide_file_preview(cx);
                            this.follow_link_target(
                                target.clone(),
                                false,
                                Some(source_path),
                                window,
                                cx,
                            );
                        }
                        ReadingEvent::OpenLinkInNewTab(target) => {
                            this.hide_file_preview(cx);
                            this.follow_link_target(
                                target.clone(),
                                true,
                                Some(source_path),
                                window,
                                cx,
                            );
                        }
                        ReadingEvent::Hover(hover) => this.link_preview_hover(
                            hover.clone(),
                            source_path,
                            tft_app::page_preview::Source::Reading,
                            depth + 1,
                            cx,
                        ),
                        _ => {}
                    }
                },
            ));
        }

        if let Some(node) = active_node {
            let path = vault.path(node).unwrap_or_default().to_owned();
            if let Some(tab) = model.active_mut().and_then(|group| group.active_mut()) {
                if tab.node != Some(node) {
                    tab.open_node(node, path.clone(), ViewMode::default_editing(), 0);
                }
            }
            if !recent_files.iter().any(|recent| recent == &path) {
                recent_files.insert(0, path.clone());
            }
            let text = document_text(&vault, node);
            let offset = model
                .active()
                .and_then(|group| group.active())
                .and_then(|tab| tab.history.get(tab.history_ix.saturating_sub(1)))
                .map(|entry| entry.offset)
                .unwrap_or(0);
            let mode = model
                .active()
                .and_then(|group| group.active())
                .map(|tab| tab.mode)
                .unwrap_or(ViewMode::default_editing());
            editor.update(cx, |editor, cx| {
                editor.set_workspace_document(path, text, mode, cx);
                editor.reveal_offset(offset, cx);
            });
        }

        let focus = editor.read(cx).focus_handle_ref().clone();
        window.focus(&focus, cx);
        let mut recent_group_order = vec![model.active_group];
        recent_group_order.extend(
            model
                .groups
                .iter()
                .map(|group| group.id)
                .filter(|group| *group != model.active_group),
        );
        let mut view = Self {
            graph_data: Arc::new(graph_data),
            graph_layout: Arc::new(Mutex::new(graph_layout)),
            graph_config,
            graph_previous_modes: HashMap::new(),
            radial_trees: HashMap::new(),
            radial_tree_active: std::collections::HashSet::new(),
            pending_radial_open: None,
            graph_panel_open,
            graph_search_input,
            graph_group_input,
            graph_editing_group: None,
            graph_local_center,
            graph_local_options,
            graph_simulation_started: std::cell::Cell::new(false),
            graph_refresh_generation: 0,
            graph_scale: std::cell::Cell::new(1.0),
            graph_offset: std::cell::Cell::new((0.0, 0.0)),
            graph_drag_start: std::cell::Cell::new(None),
            graph_drag_moved: std::cell::Cell::new(false),
            graph_hovered: std::cell::Cell::new(None),
            graph_slider_drag: std::cell::Cell::new(None),
            group_views,
            title_editing: WebTitleEditing {
                focus: title_focus,
                editing: None,
                layouts: HashMap::new(),
            },
            command_palette,
            command_palette_open: false,
            recent_commands: Vec::new(),
            file_preview_views,
            file_preview_pending: None,
            file_previews: vec![None; WEB_PREVIEW_DEPTH_LIMIT],
            preview_editors: HashMap::new(),
            file_preview_generation: 0,
            file_preview_hide_generation: 0,
            file_preview_pointer_over: vec![false; WEB_PREVIEW_DEPTH_LIMIT],
            hovered_preview_link: None,
            hovered_preview_file: None,
            search_input,
            quick_switcher_input,
            quick_switcher_open: false,
            quick_switcher_selected: 0,
            pending_quick_switcher_focus_restore: false,
            pending_quick_switcher_open: None,
            search_hits: Vec::new(),
            selected_search_hit: None,
            selected_search_match: None,
            search_scroll_handles: RefCell::default(),
            search_matching_case: false,
            search_extra_context: false,
            search_explain: false,
            search_settings_open: false,
            search_sort: SortOrder::default(),
            search_sort_menu_open: false,
            search_collapse_all: false,
            search_toggled_paths: std::collections::HashSet::new(),
            pending_search_open: None,
            search_error: None,
            search_generation: 0,
            search_documents,
            recent_files,
            excluded,
            switcher_config,
            app_config,
            page_preview_config,
            hotkeys,
            media_previews: HashMap::new(),
            media_preview_loading: std::collections::HashSet::new(),
            media_preview_errors: HashMap::new(),
            canvas_scenes: HashMap::new(),
            canvas_origin: std::rc::Rc::new(std::cell::Cell::new((0.0, 0.0))),
            canvas_viewports,
            canvas_background_drag: None,
            canvas_loading: std::collections::HashSet::new(),
            canvas_errors: HashMap::new(),
            canvas_saving: std::collections::HashSet::new(),
            canvas_drag: None,
            canvas_card_editor: None,
            audio_ticking: std::collections::HashSet::new(),
            pdf_loading: std::collections::HashSet::new(),
            pdf_loaded: std::collections::HashSet::new(),
            pdf_errors: HashMap::new(),
            vault,
            tree,
            tree_selection: Default::default(),
            tree_focus,
            tree_focused_key: None,
            file_tree_context_menu: None,
            tab_context_menu: None,
            editor_context_menu: None,
            dock_width,
            dock_dragging: false,
            narrow_dock_open: false,
            tab_drop_boundary: None,
            split_dragging: None,
            model,
            closed_tab_history: tft_app::workspace::ClosedTabHistory::default(),
            pending_link: None,
            pending_link_source: None,
            last_link_event: String::new(),
            recent_group_order,
            group_subscriptions: std::collections::HashMap::new(),
            vault_name: "Vault".into(),
            drive_controls: false,
            _subscriptions: subscriptions,
        };
        let mut custom_bindings = Vec::new();
        for (id, hotkeys) in &view.hotkeys {
            if !WEB_COMMANDS.contains(&id.as_str()) {
                continue;
            }
            for hotkey in hotkeys {
                for keys in browser_keystrokes(hotkey) {
                    custom_bindings.push(KeyBinding::new(
                        &keys,
                        RunWebCommand {
                            id: id.clone(),
                            binding: Some(keys.clone()),
                        },
                        None,
                    ));
                }
            }
        }
        cx.bind_keys(custom_bindings);
        view.register_group_views(group_id, editor, reading, pdf, cx);
        let other_groups = view
            .group_views
            .keys()
            .copied()
            .filter(|id| *id != group_id)
            .collect::<Vec<_>>();
        for id in other_groups {
            view.attach_group_view_subscriptions(id, cx);
        }
        let active_image_nodes = view
            .model
            .groups
            .iter()
            .filter_map(|group| group.active().and_then(|tab| tab.node))
            .filter(|node| {
                matches!(
                    view.vault.node(*node).map(|entry| &entry.kind),
                    Some(NodeKind::File(meta))
                        if matches!(meta.kind, tft_formats::FileKind::Image | tft_formats::FileKind::Audio | tft_formats::FileKind::Video)
                )
            })
            .collect::<Vec<_>>();
        for node in active_image_nodes {
            view.load_image_preview(node, cx);
        }
        let active_pdf_nodes = view
            .model
            .groups
            .iter()
            .filter_map(|group| group.active().and_then(|tab| tab.node))
            .filter(|node| {
                matches!(
                    view.vault.node(*node).map(|entry| &entry.kind),
                    Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Pdf
                )
            })
            .collect::<Vec<_>>();
        for node in active_pdf_nodes {
            view.load_pdf(node, cx);
        }
        let active_canvas_nodes = view
            .model
            .groups
            .iter()
            .filter_map(|group| group.active().and_then(|tab| tab.node))
            .filter(|node| {
                matches!(
                    view.vault.node(*node).map(|entry| &entry.kind),
                    Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Canvas
                )
            })
            .collect::<Vec<_>>();
        for node in active_canvas_nodes {
            view.load_canvas(node, cx);
        }
        view.publish_test_state();
        view
    }

    fn active_views(&self) -> &GroupViews {
        self.group_views
            .get(&self.model.active_group)
            .expect("every workspace group has its own views")
    }

    fn graph_node_hover_preview(
        &mut self,
        path: String,
        over: bool,
        position: Point<Pixels>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let node = self.vault.lookup(&path).ok();
        let anchor = Bounds::new(
            point(position.x - px(8.), position.y - px(8.)),
            size(px(16.), px(16.)),
        );
        self.file_tree_row_hover(
            node,
            path,
            over,
            anchor,
            modifiers,
            tft_app::page_preview::Source::Graph,
            cx,
        );
    }

    fn render_graph_view(
        &self,
        group_id: u64,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if !self.graph_simulation_started.replace(true) {
            let layout = self.graph_layout.clone();
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let layout_for_step = layout.clone();
                let settled = cx
                    .background_executor()
                    .spawn(async move {
                        let mut layout = layout_for_step
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        layout.step();
                        layout.is_settled()
                    })
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() || settled {
                    break;
                }
            })
            .detach();
        }
        let data = self.graph_data.clone();
        let layout = self.graph_layout.clone();
        let node_count = data.nodes.len();
        let edge_count = data.edges.len();
        let is_local_graph = self.graph_local_center.is_some();
        let hovered_label = self
            .graph_hovered
            .get()
            .and_then(|index| data.nodes.get(index))
            .map(|node| node.label.clone());
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let canvas_theme = theme.clone();
        let show_arrows = self.graph_config.show_arrow.unwrap_or(false);
        let line_size = self.graph_config.line_size_multiplier.unwrap_or(1.0);
        let node_size = self.graph_config.node_size_multiplier.unwrap_or(1.0);
        let text_fade = self.graph_config.text_fade_multiplier.unwrap_or(0.0);
        let group_rows = self
            .graph_config
            .color_groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let mut color: gpui::Hsla = gpui::rgb(group.color.rgb).into();
                color.a = group.color.a.clamp(0.0, 1.0);
                let query = if self.graph_editing_group == Some(index) {
                    self.graph_group_input.clone().into_any_element()
                } else {
                    div()
                        .id(SharedString::from(format!("web-graph-group-query-{index}")))
                        .flex_1()
                        .min_w_0()
                        .px_1()
                        .cursor_pointer()
                        .text_color(if group.query.is_empty() {
                            theme.tokens.text_faint
                        } else {
                            theme.tokens.text_normal
                        })
                        .child(if group.query.is_empty() {
                            "Enter query…".to_string()
                        } else {
                            group.query.clone()
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_graph_group(index, window, cx)
                        }))
                        .into_any_element()
                };
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(2.))
                    .child(
                        div()
                            .id(SharedString::from(format!("web-graph-group-color-{index}")))
                            .w(px(16.))
                            .h(px(16.))
                            .rounded(px(3.))
                            .bg(color)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.cycle_graph_group_color(index, cx)
                            })),
                    )
                    .child(query)
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "web-graph-group-remove-{index}"
                            )))
                            .px_1()
                            .cursor_pointer()
                            .text_color(theme.tokens.text_muted)
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_graph_group(index, cx)
                            })),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let scale = self.graph_scale.get();
        let offset = self.graph_offset.get();
        let hovered = self.graph_hovered.get();
        div()
            .relative()
            .size_full()
            .bg(theme.tokens.background_primary)
            .child(canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |bounds, hitbox, window, paint_cx| {
                    let layout_for_events = layout.clone();
                    let layout = layout.lock().unwrap_or_else(|error| error.into_inner());
                    let positions = graph_screen_positions(&layout.bodies, bounds, scale, offset);
                    let mut links = PathBuilder::stroke(px(line_size));
                    let mut highlighted_links = PathBuilder::stroke(px(line_size * 1.5));
                    let mut arrows = PathBuilder::fill();
                    for &(from, to) in &data.edges {
                        if let (Some(a), Some(b)) =
                            (positions.get(from as usize), positions.get(to as usize))
                        {
                            let path =
                                if hovered == Some(from as usize) || hovered == Some(to as usize) {
                                    &mut highlighted_links
                                } else {
                                    &mut links
                                };
                            path.move_to(*a);
                            path.line_to(*b);
                            if show_arrows {
                                let dx = (b.x - a.x).as_f32();
                                let dy = (b.y - a.y).as_f32();
                                let length = (dx * dx + dy * dy).sqrt().max(1.0);
                                let ux = dx / length;
                                let uy = dy / length;
                                let tip = point(b.x - px(3.0 * ux), b.y - px(3.0 * uy));
                                let base = point(tip.x - px(7.0 * ux), tip.y - px(7.0 * uy));
                                arrows.move_to(tip);
                                arrows.line_to(point(base.x - px(3.0 * uy), base.y + px(3.0 * ux)));
                                arrows.line_to(point(base.x + px(3.0 * uy), base.y - px(3.0 * ux)));
                                arrows.close();
                            }
                        }
                    }
                    if let Ok(path) = links.build() {
                        window.paint_path(path, canvas_theme.tokens.graph_line);
                    }
                    if let Ok(path) = highlighted_links.build() {
                        window.paint_path(path, canvas_theme.tokens.interactive_accent);
                    }
                    if show_arrows {
                        if let Ok(path) = arrows.build() {
                            window.paint_path(path, canvas_theme.tokens.text_muted);
                        }
                    }
                    let style = window.text_style();
                    let line_height = window.line_height();
                    let label_threshold = 0.4
                        + (node_count.max(1) as f32 / 300.0).ln().max(0.0) * 0.5
                        + text_fade * 0.5;
                    let mut label_count = 0;
                    for (index, position) in positions.iter().enumerate() {
                        let node = data.nodes.get(index);
                        let degree = node.map(|node| node.degree).unwrap_or(0);
                        let radius = px((2.5 + (degree as f32).sqrt() * 1.2) * node_size);
                        let color = if hovered == Some(index) {
                            canvas_theme.tokens.interactive_accent
                        } else {
                            node.and_then(|node| node.color.as_ref())
                                .map(|color| {
                                    let mut color_value: gpui::Hsla = gpui::rgb(color.rgb).into();
                                    color_value.a = color.a.clamp(0.0, 1.0);
                                    color_value
                                })
                                .unwrap_or(canvas_theme.tokens.graph_node)
                        };
                        let mut quad = fill(
                            Bounds::new(
                                point(position.x - radius, position.y - radius),
                                size(radius * 2.0, radius * 2.0),
                            ),
                            color,
                        );
                        quad.corner_radii = gpui::Corners::all(radius);
                        window.paint_quad(quad);
                        if (scale >= label_threshold && label_count < 400) || hovered == Some(index)
                        {
                            if let Some(node) = node {
                                if scale >= label_threshold && label_count < 400 {
                                    label_count += 1;
                                }
                                let mut run = style.to_run(node.label.len());
                                run.color = if hovered == Some(index) {
                                    canvas_theme.tokens.text_normal
                                } else {
                                    canvas_theme.tokens.graph_text
                                };
                                let line = window.text_system().shape_line(
                                    SharedString::from(node.label.clone()),
                                    px(11.0),
                                    &[run],
                                    None,
                                );
                                let _ = line.paint(
                                    point(
                                        position.x - line.width / 2.0,
                                        position.y + radius + px(3.0),
                                    ),
                                    line_height,
                                    gpui::TextAlign::Left,
                                    None,
                                    window,
                                    paint_cx,
                                );
                            }
                        }
                    }
                    drop(layout);
                    let hover_positions = positions.clone();
                    let hover_nodes = data.nodes.clone();
                    let e = paint_entity.clone();
                    let hb = hitbox.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && event.button == MouseButton::Left
                            && hb.is_hovered(window)
                        {
                            e.update(cx, |this, _| {
                                this.graph_drag_start.set(Some(event.position));
                                this.graph_drag_moved.set(false);
                            });
                        }
                    });
                    let e = paint_entity.clone();
                    let hb = hitbox.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        if hb.is_hovered(window) {
                            e.update(cx, |this, cx| {
                                if let Some(start) = this.graph_drag_start.get() {
                                    if (event.position.x - start.x).abs() > px(3.)
                                        || (event.position.y - start.y).abs() > px(3.)
                                    {
                                        this.graph_drag_moved.set(true);
                                    }
                                    let origin = this.graph_offset.get();
                                    this.graph_offset.set((
                                        origin.0 + (event.position.x - start.x).as_f32(),
                                        origin.1 + (event.position.y - start.y).as_f32(),
                                    ));
                                    this.graph_drag_start.set(Some(event.position));
                                    cx.notify();
                                } else {
                                    let nearest = hover_positions
                                        .iter()
                                        .enumerate()
                                        .filter_map(|(index, position)| {
                                            let dx = (event.position.x - position.x).as_f32();
                                            let dy = (event.position.y - position.y).as_f32();
                                            let distance = dx * dx + dy * dy;
                                            (distance <= 144.0).then_some((index, distance))
                                        })
                                        .min_by(|left, right| left.1.total_cmp(&right.1))
                                        .map(|(index, _)| index);
                                    let previous = this.graph_hovered.get();
                                    if previous != nearest {
                                        if let Some(index) = previous {
                                            if let (Some(position), Some(node)) =
                                                (hover_positions.get(index), hover_nodes.get(index))
                                            {
                                                this.graph_node_hover_preview(
                                                    node.id.clone(),
                                                    false,
                                                    *position,
                                                    event.modifiers,
                                                    cx,
                                                );
                                            }
                                        }
                                        this.graph_hovered.set(nearest);
                                        if let Some(index) = nearest {
                                            if let (Some(position), Some(node)) =
                                                (hover_positions.get(index), hover_nodes.get(index))
                                            {
                                                this.graph_node_hover_preview(
                                                    node.id.clone(),
                                                    true,
                                                    *position,
                                                    event.modifiers,
                                                    cx,
                                                );
                                            }
                                        }
                                        cx.notify();
                                    }
                                }
                            });
                        } else {
                            e.update(cx, |this, cx| {
                                if this.graph_drag_start.get().is_none() {
                                    let previous = this.graph_hovered.replace(None);
                                    if let Some(index) = previous {
                                        if let (Some(position), Some(node)) =
                                            (hover_positions.get(index), hover_nodes.get(index))
                                        {
                                            this.graph_node_hover_preview(
                                                node.id.clone(),
                                                false,
                                                *position,
                                                event.modifiers,
                                                cx,
                                            );
                                        }
                                        cx.notify();
                                    }
                                }
                            });
                        }
                    });
                    let e = paint_entity.clone();
                    let data = data.clone();
                    let layout = layout_for_events.clone();
                    let hb = hitbox.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let was_click = e.update(cx, |this, _| {
                            this.graph_drag_start.take().is_some()
                                && !this.graph_drag_moved.replace(false)
                        });
                        if was_click && event.button == MouseButton::Left && hb.is_hovered(window) {
                            let layout = layout.lock().unwrap_or_else(|error| error.into_inner());
                            let current_scale = e.update(cx, |this, _| this.graph_scale.get());
                            let current_offset = e.update(cx, |this, _| this.graph_offset.get());
                            let positions = graph_screen_positions(
                                &layout.bodies,
                                bounds,
                                current_scale,
                                current_offset,
                            );
                            let nearest = positions
                                .iter()
                                .enumerate()
                                .filter_map(|(index, position)| {
                                    let dx = (event.position.x - position.x).as_f32();
                                    let dy = (event.position.y - position.y).as_f32();
                                    let distance = dx * dx + dy * dy;
                                    (distance <= 100.0).then_some((index, distance))
                                })
                                .min_by(|a, b| a.1.total_cmp(&b.1))
                                .map(|(index, _)| index);
                            if let Some(path) = nearest
                                .and_then(|index| data.nodes.get(index))
                                .map(|node| node.id.clone())
                            {
                                e.update(cx, |this, cx| {
                                    this.activate_group(group_id, cx);
                                    if let Ok(node) = this.vault.lookup(&path) {
                                        this.open(node, path, false, window, cx);
                                    }
                                });
                            }
                        }
                    });
                    let e = paint_entity.clone();
                    let hb = hitbox.clone();
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hb.should_handle_scroll(window) {
                            let delta = event.delta.pixel_delta(window.line_height()).y.as_f32();
                            e.update(cx, |this, cx| {
                                this.graph_scale.set(
                                    (this.graph_scale.get() * (delta * 0.002).exp())
                                        .clamp(0.1, 8.0),
                                );
                                cx.notify();
                            });
                        }
                    });
                },
            ))
            .child(
                div()
                    .absolute()
                    .top_2()
                    .right_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(hovered_label, |element, label| {
                        element.child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.tokens.text_normal)
                                .child(label),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.tokens.text_muted)
                            .child(format!("{node_count} nodes, {edge_count} links")),
                    )
                    .child(
                        div()
                            .id("web-graph-toggle-local")
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(if is_local_graph {
                                theme.tokens.interactive_accent
                            } else {
                                theme.tokens.background_secondary
                            })
                            .text_color(theme.tokens.text_normal)
                            .cursor_pointer()
                            .child(if is_local_graph {
                                "Local"
                            } else {
                                "Local graph"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_graph_local(group_id, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("web-graph-zoom-out")
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(theme.tokens.background_secondary)
                            .text_color(theme.tokens.text_normal)
                            .cursor_pointer()
                            .child("−")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.graph_scale
                                    .set((this.graph_scale.get() / 1.25).max(0.1));
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("web-graph-zoom-in")
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(theme.tokens.background_secondary)
                            .text_color(theme.tokens.text_normal)
                            .cursor_pointer()
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.graph_scale
                                    .set((this.graph_scale.get() * 1.25).min(8.0));
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("web-graph-fit")
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(theme.tokens.background_secondary)
                            .text_color(theme.tokens.text_normal)
                            .cursor_pointer()
                            .child("Fit")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.graph_scale.set(1.0);
                                this.graph_offset.set((0.0, 0.0));
                                cx.notify();
                            })),
                    ),
            )
            .child(if self.graph_panel_open {
                div()
                    .id("web-graph-controls")
                    .absolute()
                    .top_2()
                    .left_2()
                    .w(px(240.))
                    .max_h_full()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.tokens.menu_border_color)
                    .bg(theme.tokens.menu_background)
                    .text_size(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.tokens.background_modifier_border)
                            .text_color(theme.tokens.text_normal)
                            .child("Graph settings")
                            .child(
                                div()
                                    .id("web-graph-controls-close")
                                    .cursor_pointer()
                                    .child("×")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.graph_panel_open = false;
                                        this.emit_workspace_state(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().px_2().py_2().child(self.graph_search_input.clone()))
                    .when(self.graph_local_center.is_some(), |panel| {
                        panel
                            .child(
                                div()
                                    .px_2()
                                    .pt_1()
                                    .pb_1()
                                    .text_color(theme.tokens.text_muted)
                                    .child("Local graph"),
                            )
                            .child(self.graph_slider_row(WebGraphSlider::Depth, &theme, cx))
                            .child(self.graph_local_option_row(
                                "Incoming links",
                                "localBacklinks",
                                self.graph_local_options.backlinks,
                                &theme,
                                cx,
                            ))
                            .child(self.graph_local_option_row(
                                "Outgoing links",
                                "localForelinks",
                                self.graph_local_options.forelinks,
                                &theme,
                                cx,
                            ))
                            .child(self.graph_local_option_row(
                                "Neighbor links",
                                "localInterlinks",
                                self.graph_local_options.interlinks,
                                &theme,
                                cx,
                            ))
                    })
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_color(theme.tokens.text_muted)
                            .child("Filters"),
                    )
                    .child(self.graph_setting_row(
                        "Tags",
                        "showTags",
                        self.graph_config.show_tags.unwrap_or(false),
                        &theme,
                        cx,
                    ))
                    .child(self.graph_setting_row(
                        "Attachments",
                        "showAttachments",
                        self.graph_config.show_attachments.unwrap_or(false),
                        &theme,
                        cx,
                    ))
                    .child(self.graph_setting_row(
                        "Existing files only",
                        "hideUnresolved",
                        self.graph_config.hide_unresolved.unwrap_or(false),
                        &theme,
                        cx,
                    ))
                    .when(self.graph_local_center.is_none(), |panel| {
                        panel.child(self.graph_setting_row(
                            "Orphans",
                            "showOrphans",
                            self.graph_config.show_orphans.unwrap_or(true),
                            &theme,
                            cx,
                        ))
                    })
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_color(theme.tokens.text_muted)
                            .child("Groups"),
                    )
                    .children(group_rows)
                    .child(
                        div().px_2().py_1().child(
                            div()
                                .id("web-graph-group-add")
                                .px_2()
                                .py_1()
                                .rounded(px(4.))
                                .bg(theme.tokens.interactive_normal)
                                .text_color(theme.tokens.text_normal)
                                .cursor_pointer()
                                .child("New group")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_graph_group(window, cx)
                                })),
                        ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_color(theme.tokens.text_muted)
                            .child("Display"),
                    )
                    .child(self.graph_setting_row(
                        "Arrows",
                        "showArrow",
                        self.graph_config.show_arrow.unwrap_or(false),
                        &theme,
                        cx,
                    ))
                    .child(self.graph_slider_row(WebGraphSlider::TextFade, &theme, cx))
                    .child(self.graph_slider_row(WebGraphSlider::NodeSize, &theme, cx))
                    .child(self.graph_slider_row(WebGraphSlider::LinkSize, &theme, cx))
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_color(theme.tokens.text_muted)
                            .child("Forces"),
                    )
                    .child(self.graph_slider_row(WebGraphSlider::Center, &theme, cx))
                    .child(self.graph_slider_row(WebGraphSlider::Repel, &theme, cx))
                    .child(self.graph_slider_row(WebGraphSlider::Link, &theme, cx))
                    .child(self.graph_slider_row(WebGraphSlider::LinkDistance, &theme, cx))
                    .into_any_element()
            } else {
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .id("web-graph-controls-open")
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(theme.tokens.background_secondary)
                    .text_color(theme.tokens.text_normal)
                    .cursor_pointer()
                    .child("⚙")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.graph_panel_open = true;
                        this.emit_workspace_state(cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .into_any_element()
    }

    fn active_editor(&self) -> &Entity<Editor> {
        &self.active_views().editor
    }

    fn active_reading(&self) -> &Entity<ReadingView> {
        &self.active_views().reading
    }

    fn sync_document_editors(
        &mut self,
        source: &Entity<Editor>,
        node: NodeId,
        path: &str,
        contents: &str,
        cx: &mut Context<Self>,
    ) {
        let source_id = source.entity_id();
        let mut peers = self
            .model
            .groups
            .iter()
            .filter_map(|group| {
                let tab = group.active()?;
                (tab.node == Some(node)).then(|| {
                    self.group_views
                        .get(&group.id)
                        .map(|views| (views.editor.clone(), tab.mode))
                })?
            })
            .collect::<Vec<_>>();
        peers.extend(self.preview_editors.iter().filter_map(|(depth, editing)| {
            self.file_previews
                .get(*depth)
                .and_then(Option::as_ref)
                .filter(|preview| preview.node == node)
                .map(|_| (editing.editor.clone(), ViewMode::LivePreview))
        }));
        for (editor, mode) in peers {
            if editor.entity_id() == source_id {
                continue;
            }
            editor.update(cx, |editor, cx| {
                editor.set_workspace_document_preserving_selection(
                    path.to_owned(),
                    contents.to_owned(),
                    mode,
                    cx,
                );
            });
        }
    }

    fn register_group_views(
        &mut self,
        group_id: u64,
        editor: Entity<Editor>,
        reading: Entity<ReadingView>,
        pdf: Entity<PdfView>,
        cx: &mut Context<Self>,
    ) {
        self.group_views.insert(
            group_id,
            GroupViews {
                editor,
                reading,
                pdf,
            },
        );
        self.attach_group_view_subscriptions(group_id, cx);
    }

    fn attach_group_view_subscriptions(&mut self, group_id: u64, cx: &mut Context<Self>) {
        let Some(views) = self.group_views.get(&group_id) else {
            return;
        };
        let editor = views.editor.clone();
        let reading_view = views.reading.clone();
        let editor_subscription = cx.subscribe(
            &editor,
            move |this, _, event: &tft_app::editor::EditorEvent, cx| {
                if let tft_app::editor::EditorEvent::Hover(hover) = event {
                    let path = this
                        .model
                        .group(group_id)
                        .and_then(|group| group.active())
                        .and_then(|tab| tab.path.clone())
                        .unwrap_or_default();
                    this.link_preview_hover(
                        hover.clone(),
                        path,
                        tft_app::page_preview::Source::Editor,
                        0,
                        cx,
                    );
                    return;
                }
                if let tft_app::editor::EditorEvent::ContextMenu {
                    position,
                    selection,
                    selected,
                    link,
                } = event
                {
                    if this.model.group(group_id).is_some() {
                        let source_path = this
                            .model
                            .group(group_id)
                            .and_then(|group| group.active())
                            .and_then(|tab| tab.path.as_deref());
                        let link_path = link.as_ref().and_then(|target| match target {
                            reading::LinkTarget::Internal(body) => {
                                let source_path = source_path?;
                                let resolver = vault_resolver(&this.vault);
                                tft_app::resolve_internal_link(&resolver, body, source_path)
                                    .path
                                    .filter(|path| this.vault.lookup(path).is_ok())
                            }
                            _ => None,
                        });
                        let drive_id = link_path.as_deref().and_then(|path| {
                            this.vault.lookup(path).ok().and_then(|node| {
                                this.vault.backend(node).and_then(|backend| match backend {
                                    BackendObjectId::Drive(id) => Some(id.clone()),
                                    BackendObjectId::Local(_) => None,
                                })
                            })
                        });
                        this.activate_group(group_id, cx);
                        this.file_tree_context_menu = None;
                        this.tab_context_menu = None;
                        this.editor_context_menu = Some(EditorContextMenu {
                            group_id,
                            editor: true,
                            position: *position,
                            selection: selection.clone(),
                            selected: *selected,
                            link: link.clone(),
                            link_path,
                            drive_id,
                        });
                        cx.notify();
                    }
                    return;
                }
                if matches!(event, tft_app::editor::EditorEvent::Edited) {
                    let Some(node) = this
                        .model
                        .group(group_id)
                        .and_then(|group| group.active())
                        .and_then(|tab| tab.node)
                    else {
                        return;
                    };
                    let Some(editor) = this
                        .group_views
                        .get(&group_id)
                        .map(|views| views.editor.clone())
                    else {
                        return;
                    };
                    let contents = editor.read(cx).file_contents();
                    this.vault.set_text(node, contents.clone());
                    if let Some(path) = this.vault.path(node).map(str::to_owned) {
                        this.sync_document_editors(&editor, node, &path, &contents, cx);
                        this.upsert_search_document(node, path, contents.clone());
                        this.schedule_graph_rebuild(cx, false);
                    }
                    cx.emit(WebWorkspaceEvent::Edited { node, contents });
                    this.publish_test_state();
                    return;
                }
                let (target, new_tab) = match event {
                    tft_app::editor::EditorEvent::OpenLink(target) => (target, false),
                    tft_app::editor::EditorEvent::OpenLinkInNewTab(target) => (target, true),
                    _ => return,
                };
                match target {
                    reading::LinkTarget::Internal(_) | reading::LinkTarget::Tag(_) => {
                        this.activate_group(group_id, cx);
                        this.pending_link_source = None;
                        this.pending_link = Some((target.clone(), new_tab));
                        cx.notify();
                    }
                    reading::LinkTarget::External(url) => cx.open_url(url),
                    reading::LinkTarget::Footnote(_) => {
                        this.activate_group(group_id, cx);
                        this.pending_link_source = None;
                        this.pending_link = Some((target.clone(), new_tab));
                        cx.notify();
                    }
                }
            },
        );
        let reading_subscription = cx.subscribe(
            &reading_view,
            move |this, _, event: &ReadingEvent, cx| match event {
                ReadingEvent::ContextMenu { target, position } => {
                    let source_path = this
                        .model
                        .group(group_id)
                        .and_then(|group| group.active())
                        .and_then(|tab| tab.path.as_deref());
                    let link_path = match target {
                        reading::LinkTarget::Internal(body) => source_path.and_then(|source| {
                            let resolver = vault_resolver(&this.vault);
                            tft_app::resolve_internal_link(&resolver, body, source)
                                .path
                                .filter(|path| this.vault.lookup(path).is_ok())
                        }),
                        _ => None,
                    };
                    if let Some(link_path) = link_path {
                        let drive_id = this
                            .vault
                            .lookup(&link_path)
                            .ok()
                            .and_then(|node| this.vault.backend(node))
                            .and_then(|backend| match backend {
                                BackendObjectId::Drive(id) => Some(id.clone()),
                                BackendObjectId::Local(_) => None,
                            });
                        this.activate_group(group_id, cx);
                        this.file_tree_context_menu = None;
                        this.tab_context_menu = None;
                        this.editor_context_menu = Some(EditorContextMenu {
                            group_id,
                            editor: false,
                            position: *position,
                            selection: String::new(),
                            selected: false,
                            link: Some(target.clone()),
                            link_path: Some(link_path),
                            drive_id,
                        });
                        cx.notify();
                    }
                }
                ReadingEvent::Hover(hover) => {
                    let path = this
                        .model
                        .group(group_id)
                        .and_then(|group| group.active())
                        .and_then(|tab| tab.path.clone())
                        .unwrap_or_default();
                    this.link_preview_hover(
                        hover.clone(),
                        path,
                        tft_app::page_preview::Source::Reading,
                        0,
                        cx,
                    );
                }
                ReadingEvent::OpenLink(reading::LinkTarget::Internal(target)) => {
                    this.activate_group(group_id, cx);
                    this.last_link_event = target.clone();
                    this.pending_link =
                        Some((reading::LinkTarget::Internal(target.clone()), false));
                    cx.notify();
                }
                ReadingEvent::OpenLinkInNewTab(reading::LinkTarget::Internal(target)) => {
                    this.activate_group(group_id, cx);
                    this.last_link_event = target.clone();
                    this.pending_link_source = None;
                    this.pending_link = Some((reading::LinkTarget::Internal(target.clone()), true));
                    cx.notify();
                }
                ReadingEvent::OpenLink(reading::LinkTarget::External(url))
                | ReadingEvent::OpenLinkInNewTab(reading::LinkTarget::External(url)) => {
                    cx.open_url(url);
                }
                ReadingEvent::OpenLink(reading::LinkTarget::Footnote(number)) => {
                    if let Some(reader) =
                        this.group_views.get(&group_id).map(|views| &views.reading)
                    {
                        if let Some(offset) = reader
                            .read(cx)
                            .model()
                            .footnotes
                            .get(number.saturating_sub(1))
                            .copied()
                        {
                            reader.update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
                        }
                    }
                }
                ReadingEvent::OpenLink(reading::LinkTarget::Tag(tag))
                | ReadingEvent::OpenLinkInNewTab(reading::LinkTarget::Tag(tag)) => {
                    this.activate_group(group_id, cx);
                    this.pending_link_source = None;
                    this.pending_link = Some((reading::LinkTarget::Tag(tag.clone()), false));
                    cx.notify();
                }
                _ => {}
            },
        );
        let task_subscription =
            cx.subscribe(&reading_view, move |this, _, event: &TaskToggled, cx| {
                this.activate_group(group_id, cx);
                this.toggle_task(&event.path, event.marker, cx)
            });
        let property_subscription = cx.subscribe(
            &reading_view,
            move |this, _, event: &PropertyItemRemoved, cx| {
                this.activate_group(group_id, cx);
                this.remove_property_item(&event.path, &event.key, event.index, cx)
            },
        );
        self.group_subscriptions.insert(
            group_id,
            vec![
                editor_subscription,
                reading_subscription,
                task_subscription,
                property_subscription,
            ],
        );
    }

    fn rerun_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).text().trim().to_owned();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        self.search_hits.clear();
        self.search_toggled_paths.clear();
        self.selected_search_hit = None;
        self.selected_search_match = None;
        self.search_error = None;
        if query.is_empty() {
            cx.notify();
            self.publish_test_state();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            this.update(cx, |this, cx| {
                if this.search_generation == generation {
                    this.run_search(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn fold_selected_search_hit(&mut self, collapse: bool, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.selected_search_hit else {
            return false;
        };
        let Some(hit) = self.search_hits.get(index) else {
            return false;
        };
        let path = hit.path.clone();
        let is_collapsed = self.search_collapse_all ^ self.search_toggled_paths.contains(&path);
        if self.selected_search_match.is_some() {
            self.selected_search_match = None;
            self.scroll_search_selection(self.search_row_index(index, None).unwrap_or(0), cx);
        }
        if is_collapsed != collapse {
            if !self.search_toggled_paths.remove(&path) {
                self.search_toggled_paths.insert(path);
            }
            cx.notify();
        }
        true
    }

    fn search_keyboard_rows(&self) -> Vec<(usize, Option<usize>)> {
        let mut rows = Vec::new();
        for (hit_index, hit) in self.search_hits.iter().enumerate() {
            rows.push((hit_index, None));
            let collapsed =
                self.search_collapse_all ^ self.search_toggled_paths.contains(&hit.path);
            if !collapsed {
                rows.extend((0..hit.snippets.len()).map(|snippet| (hit_index, Some(snippet))));
            }
        }
        rows
    }

    fn search_row_index(&self, hit: usize, snippet: Option<usize>) -> Option<usize> {
        self.search_keyboard_rows()
            .iter()
            .position(|row| *row == (hit, snippet))
    }

    fn move_search_focus(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.search_keyboard_rows();
        let current = self.selected_search_hit.and_then(|hit| {
            let snippet = self
                .selected_search_match
                .filter(|(selected_hit, _)| *selected_hit == hit)
                .map(|(_, snippet)| snippet);
            self.search_row_index(hit, snippet)
        });
        let next = tft_app::selection_step(current, rows.len(), delta, false);
        if let Some(row) = next.and_then(|index| rows.get(index).copied()) {
            self.selected_search_hit = Some(row.0);
            self.selected_search_match = row.1.map(|snippet| (row.0, snippet));
            if let Some(index) = self.search_row_index(row.0, row.1) {
                self.scroll_search_selection(index, cx);
            }
        } else {
            self.selected_search_hit = None;
            self.selected_search_match = None;
        }
        cx.notify();
        self.publish_test_state();
    }

    fn search_scroll_handle(&self, group_id: Option<u64>) -> gpui::ScrollHandle {
        self.search_scroll_handles
            .borrow_mut()
            .entry(group_id)
            .or_default()
            .clone()
    }

    fn scroll_search_selection(&self, index: usize, cx: &App) {
        let query_is_empty = self.search_input.read(cx).text().trim().is_empty();
        let show_no_results =
            !query_is_empty && self.search_hits.is_empty() && self.search_error.is_none();
        let show_explanation = self.search_explain && !query_is_empty;
        let show_result_header = !query_is_empty && !self.search_hits.is_empty();
        let preceding_items = usize::from(self.search_error.is_some())
            + usize::from(query_is_empty)
            + usize::from(show_no_results)
            + usize::from(show_explanation)
            + usize::from(show_result_header);
        for handle in self.search_scroll_handles.borrow().values() {
            handle.scroll_to_item(preceding_items + index);
        }
    }

    fn capture_search_left(
        &mut self,
        _: &tft_app::text_input::Left,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold_selected_search_hit(true, cx) {
            cx.stop_propagation();
        }
    }

    fn capture_search_right(
        &mut self,
        _: &tft_app::text_input::Right,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold_selected_search_hit(false, cx) {
            cx.stop_propagation();
        }
    }

    fn can_edit_active_markdown(&self) -> bool {
        self.model
            .active()
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode.is_editing() && self.is_markdown_node(tab.node))
    }

    fn is_markdown_node(&self, node: Option<NodeId>) -> bool {
        node.and_then(|node| self.vault.node(node))
            .is_some_and(|node| {
                matches!(
                    &node.kind,
                    NodeKind::File(meta) if meta.kind == tft_formats::FileKind::Markdown
                )
            })
    }

    fn is_markdown_editor_command(id: &str) -> bool {
        matches!(
            id,
            "editor:toggle-bold"
                | "editor:toggle-italics"
                | "editor:toggle-strikethrough"
                | "editor:toggle-highlight"
                | "editor:toggle-code"
                | "editor:toggle-inline-math"
                | "editor:toggle-comments"
                | "editor:toggle-checklist-status"
                | "editor:delete-paragraph"
                | "editor:insert-link"
                | "editor:insert-wikilink"
                | "editor:insert-embed"
                | "editor:toggle-bullet-list"
                | "editor:toggle-numbered-list"
                | "editor:toggle-blockquote"
                | "editor:add-cursor-above"
                | "editor:add-cursor-below"
                | "editor:clear-formatting"
                | "editor:insert-table"
                | "editor:insert-callout"
                | "editor:insert-codeblock"
                | "editor:insert-mathblock"
                | "editor:insert-horizontal-rule"
                | "editor:insert-tag"
                | "editor:insert-footnote"
                | "editor:set-heading-1"
                | "editor:set-heading-2"
                | "editor:set-heading-3"
                | "editor:set-heading-4"
                | "editor:set-heading-5"
                | "editor:set-heading-6"
                | "editor:indent-list"
                | "editor:unindent-list"
                | "editor:swap-line-up"
                | "editor:swap-line-down"
                | "editor:toggle-fold"
                | "editor:fold-all"
                | "editor:unfold-all"
                | "editor:fold-more"
                | "editor:fold-less"
        )
    }

    fn follow_editor_link(
        &mut self,
        new_tab: bool,
        new_split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_active_markdown() {
            return;
        }
        let Some(target) = self.active_editor().read(cx).link_target_at_cursor() else {
            return;
        };
        match target {
            reading::LinkTarget::Internal(target) => {
                if new_split {
                    self.split_active_group(SplitDirection::Vertical, window, cx);
                }
                self.pending_link_source = None;
                self.pending_link = Some((reading::LinkTarget::Internal(target), new_tab));
                cx.notify();
            }
            reading::LinkTarget::External(url) => cx.open_url(&url),
            reading::LinkTarget::Tag(tag) => self.show_search(&format!("tag:#{tag}"), window, cx),
            reading::LinkTarget::Footnote(number) => {
                let offset = self
                    .active_reading()
                    .read(cx)
                    .model()
                    .footnotes
                    .get(number.saturating_sub(1))
                    .copied();
                if let Some(offset) = offset {
                    self.active_editor().update(cx, |editor, cx| {
                        editor.reveal_offset(offset, cx);
                    });
                }
            }
        }
    }

    fn request_rename_heading(&mut self, cx: &mut Context<Self>) {
        if !self.can_edit_active_markdown() {
            return;
        }
        let Some(path) = self.current_path().map(str::to_owned) else {
            return;
        };
        let editor = self.active_editor();
        let editor = editor.read(cx);
        let line = editor.cursor().line as u32;
        let heading = editor.heading_to_rename();
        let Some(heading) = heading else {
            return;
        };
        cx.emit(WebWorkspaceEvent::RenameHeadingRequested {
            path,
            line,
            initial: heading.text,
        });
    }

    fn focus_neighbor(
        &mut self,
        direction: SplitDirection,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let candidates = self
            .model
            .layout
            .neighbors(self.model.active_group, direction, forward);
        let group_id = self
            .recent_group_order
            .iter()
            .find(|group_id| candidates.contains(group_id))
            .copied()
            .or_else(|| candidates.first().copied());
        let Some(group_id) = group_id else {
            return;
        };
        self.stash_group_view_state(self.model.active_group, cx);
        self.activate_group(group_id, cx);
        let mode = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .map_or(ViewMode::default_editing(), |tab| tab.mode);
        if mode == ViewMode::Reading {
            let focus = self.active_reading().focus_handle(cx);
            window.focus(&focus, cx);
        } else {
            self.focus_editor(window, cx);
        }
    }

    fn run_markdown_editor_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_active_markdown() {
            return;
        }
        self.active_editor().update(cx, |editor, cx| match id {
            "editor:toggle-bold" => editor.toggle_wrap("**", window, cx),
            "editor:toggle-italics" => editor.toggle_wrap("*", window, cx),
            "editor:toggle-strikethrough" => editor.toggle_wrap("~~", window, cx),
            "editor:toggle-highlight" => editor.toggle_highlight(None, window, cx),
            "editor:toggle-code" => editor.toggle_wrap("`", window, cx),
            "editor:toggle-inline-math" => editor.toggle_wrap("$", window, cx),
            "editor:toggle-comments" => editor.toggle_wrap("%%", window, cx),
            "editor:toggle-checklist-status" => editor.toggle_checklist(window, cx),
            "editor:delete-paragraph" => editor.delete_paragraph(window, cx),
            "editor:insert-link" => {
                editor.insert_link(tft_app::text::LinkSyntax::Markdown, window, cx)
            }
            "editor:insert-wikilink" => {
                editor.insert_link(tft_app::text::LinkSyntax::Wikilink, window, cx)
            }
            "editor:insert-embed" => {
                editor.insert_link(tft_app::text::LinkSyntax::Embed, window, cx)
            }
            "editor:toggle-bullet-list" => {
                editor.toggle_line_prefix(tft_app::text::LinePrefix::Bullet, window, cx)
            }
            "editor:toggle-numbered-list" => {
                editor.toggle_line_prefix(tft_app::text::LinePrefix::Ordered, window, cx)
            }
            "editor:toggle-blockquote" => {
                editor.toggle_line_prefix(tft_app::text::LinePrefix::Quote, window, cx)
            }
            "editor:add-cursor-above" => editor.add_cursor(true, window, cx),
            "editor:add-cursor-below" => editor.add_cursor(false, window, cx),
            "editor:clear-formatting" => editor.clear_formatting(window, cx),
            "editor:insert-table" => editor.insert_table(window, cx),
            "editor:insert-callout"
            | "editor:insert-codeblock"
            | "editor:insert-mathblock"
            | "editor:insert-horizontal-rule"
            | "editor:insert-tag"
            | "editor:insert-footnote" => {
                if let Some(kind) = tft_app::text::BlockInsert::from_id(id) {
                    editor.insert_block(kind, window, cx);
                }
            }
            "editor:set-heading-1"
            | "editor:set-heading-2"
            | "editor:set-heading-3"
            | "editor:set-heading-4"
            | "editor:set-heading-5"
            | "editor:set-heading-6" => {
                if let Some(level) = id.rsplit('-').next().and_then(|value| value.parse().ok()) {
                    editor.set_heading(level, window, cx);
                }
            }
            "editor:indent-list" => editor.indent_list(true, window, cx),
            "editor:unindent-list" => editor.indent_list(false, window, cx),
            "editor:swap-line-up" => editor.swap_lines(true, window, cx),
            "editor:swap-line-down" => editor.swap_lines(false, window, cx),
            "editor:toggle-fold" => editor.toggle_fold(cx),
            "editor:fold-all" => editor.fold_all(cx),
            "editor:unfold-all" => editor.unfold_all(cx),
            "editor:fold-more" => editor.fold_step(true, cx),
            "editor:fold-less" => editor.fold_step(false, cx),
            _ => {}
        });
    }

    fn run_metadata_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((node, mode)) = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node.map(|node| (node, tab.mode)))
        else {
            return;
        };
        let is_markdown = self
            .vault
            .node(node)
            .is_some_and(|node| matches!(&node.kind, NodeKind::File(meta) if meta.kind == tft_formats::FileKind::Markdown));
        if !is_markdown {
            return;
        }

        if id == "editor:toggle-fold-properties" {
            if mode.is_editing() {
                self.active_editor()
                    .update(cx, |editor, cx| editor.toggle_properties_fold(cx));
            } else if mode == ViewMode::Reading {
                self.active_reading()
                    .update(cx, |reading, cx| reading.toggle_properties_fold(cx));
            }
            return;
        }
        if !mode.is_editing() {
            return;
        }

        self.store_editor_text(Some(node), cx);
        match id {
            "markdown:add-metadata-property" => self
                .active_editor()
                .update(cx, |editor, cx| editor.add_property_row(window, cx)),
            "markdown:add-alias" => {
                let changed = self
                    .active_editor()
                    .update(cx, |editor, cx| editor.add_alias(true, window, cx));
                if changed {
                    self.store_editor_text(Some(node), cx);
                    cx.emit(WebWorkspaceEvent::SaveRequested);
                }
            }
            "markdown:clear-metadata-properties" => {
                let before = document_text(&self.vault, node);
                self.active_editor()
                    .update(cx, |editor, cx| editor.clear_properties(window, cx));
                self.store_editor_text(Some(node), cx);
                if document_text(&self.vault, node) != before {
                    cx.emit(WebWorkspaceEvent::SaveRequested);
                }
            }
            _ => {}
        }
    }

    fn run_table_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_active_markdown() {
            return;
        }
        let Some(command) = tft_app::text::table::TableCommand::from_id(id) else {
            return;
        };
        self.active_editor()
            .update(cx, |editor, cx| editor.table_command(command, window, cx));
    }

    fn open_file_explorer(
        &mut self,
        reveal_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.dock.show(LeftTab::Files);
        self.set_left_dock_visible(true, cx);
        if reveal_active {
            if let Some(path) = self.current_path().map(str::to_owned) {
                self.tree.reveal(&path);
            }
        }
        window.focus(&self.tree_focus, cx);
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn request_new_note(&mut self, cx: &mut Context<Self>) {
        if self.current_path().is_some() {
            self.new_tab(cx);
        }
        cx.emit(WebWorkspaceEvent::FileOperationRequested(
            FileOperationRequest::CreateNote {
                parent: String::new(),
            },
        ));
    }

    fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const AVAILABLE: &[&str] = &[
            "switcher:open",
            "workspace:new-tab",
            "workspace:close",
            "workspace:edit-file-title",
            "editor:rename-heading",
            "editor:focus-left",
            "editor:focus-right",
            "editor:focus-top",
            "editor:focus-bottom",
            "workspace:close-others",
            "workspace:close-others-tab-group",
            "workspace:undo-close-pane",
            "workspace:toggle-pin",
            "workspace:split-vertical",
            "workspace:split-horizontal",
            "workspace:close-tab-group",
            "workspace:copy-path",
            "canvas:new-file",
            "app:switch-vault",
            "graph:open-local",
            "global-search:open",
            "app:go-back",
            "app:go-forward",
            "markdown:toggle-preview",
            "editor:toggle-source",
            "markdown:add-metadata-property",
            "markdown:add-alias",
            "markdown:clear-metadata-properties",
            "editor:toggle-fold-properties",
            "graph:open",
            "editor:save-file",
            "editor:open-search",
            "editor:open-search-replace",
            "editor:context-menu",
            "editor:focus",
            "file-explorer:move-file",
            "open-with-default-app:open",
            "open-with-default-app:show",
            "editor:toggle-bold",
            "editor:toggle-italics",
            "editor:toggle-comments",
            "editor:toggle-checklist-status",
            "editor:insert-link",
            "editor:insert-wikilink",
            "editor:insert-embed",
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
            "editor:delete-paragraph",
            "editor:follow-link",
            "editor:open-link-in-new-leaf",
            "editor:open-link-in-new-split",
            "editor:toggle-strikethrough",
            "editor:toggle-highlight",
            "editor:toggle-code",
            "editor:toggle-inline-math",
            "editor:toggle-bullet-list",
            "editor:toggle-numbered-list",
            "editor:toggle-blockquote",
            "editor:clear-formatting",
            "editor:insert-table",
            "editor:insert-callout",
            "editor:insert-codeblock",
            "editor:insert-mathblock",
            "editor:insert-horizontal-rule",
            "editor:insert-tag",
            "editor:insert-footnote",
            "editor:set-heading-1",
            "editor:set-heading-2",
            "editor:set-heading-3",
            "editor:set-heading-4",
            "editor:set-heading-5",
            "editor:set-heading-6",
            "editor:indent-list",
            "editor:unindent-list",
            "editor:swap-line-up",
            "editor:swap-line-down",
            "editor:toggle-fold",
            "editor:fold-all",
            "editor:unfold-all",
            "editor:fold-more",
            "editor:fold-less",
            "editor:add-cursor-above",
            "editor:add-cursor-below",
            "file-explorer:open",
            "file-explorer:reveal-active-file",
            "file-explorer:new-file",
            "file-explorer:new-folder",
            "file-explorer:duplicate-file",
            "app:delete-file",
            "app:toggle-left-sidebar",
        ];
        let mut registry = tft_app::commands::CommandRegistry::with_known_ids();
        registry.apply_overrides(&self.hotkeys);
        let active_tab = self.model.active().and_then(|group| group.active());
        let has_history_back = active_tab.is_some_and(|tab| tab.history_ix > 1);
        let has_history_forward = active_tab.is_some_and(|tab| tab.history_ix < tab.history.len());
        let has_document = active_tab.is_some_and(|tab| tab.node.is_some());
        let has_markdown_document = active_tab
            .and_then(|tab| tab.node)
            .and_then(|node| self.vault.node(node))
            .is_some_and(|node| matches!(&node.kind, NodeKind::File(meta) if meta.kind == tft_formats::FileKind::Markdown));
        let can_edit_title = active_tab.is_some_and(|tab| tab.path.is_some())
            && active_tab
                .and_then(|tab| tab.node)
                .and_then(|node| self.vault.node(node))
                .is_some_and(|node| matches!(&node.kind, NodeKind::File(_)));
        let can_edit_markdown = self.can_edit_active_markdown();
        let modifier_label = tft_drive_web::modifier_key_label();
        let commands = AVAILABLE
            .iter()
            .filter(|id| match **id {
                "workspace:close-others" | "workspace:close-others-tab-group" => self
                    .model
                    .active()
                    .is_some_and(|group| group.tabs.len() > 1),
                "workspace:copy-path" => has_document,
                "canvas:new-file" => true,
                "app:switch-vault" => true,
                "file-explorer:move-file" => has_document,
                "editor:context-menu" => can_edit_markdown,
                "editor:focus" => has_document,
                "open-with-default-app:open" | "open-with-default-app:show" => active_tab
                    .and_then(|tab| tab.node)
                    .and_then(|node| self.vault.backend(node))
                    .is_some_and(|backend| matches!(backend, BackendObjectId::Drive(_))),
                "workspace:undo-close-pane" => !self.closed_tab_history.is_empty(),
                "workspace:split-vertical" | "workspace:split-horizontal" => {
                    tft_app::workspace::workspace_can_split(self.model.groups.len())
                }
                "workspace:close-tab-group" => self.model.groups.len() > 1,
                "app:go-back" => has_history_back,
                "app:go-forward" => has_history_forward,
                "markdown:toggle-preview" => active_tab.is_some_and(|tab| {
                    tft_app::workspace::workspace_header_shows_mode_switch(
                        tab.path.as_deref(),
                        tab.mode,
                    )
                }),
                "editor:toggle-source" => can_edit_markdown,
                "markdown:add-metadata-property"
                | "markdown:add-alias"
                | "markdown:clear-metadata-properties" => can_edit_markdown,
                "editor:toggle-fold-properties" => has_markdown_document,
                "editor:save-file" => has_document,
                "editor:add-cursor-above" | "editor:add-cursor-below" => can_edit_markdown,
                "workspace:edit-file-title" => can_edit_title,
                "editor:rename-heading" => {
                    can_edit_markdown && self.active_editor().read(cx).heading_to_rename().is_some()
                }
                "editor:focus-left" => !self
                    .model
                    .layout
                    .neighbors(self.model.active_group, SplitDirection::Vertical, false)
                    .is_empty(),
                "editor:focus-right" => !self
                    .model
                    .layout
                    .neighbors(self.model.active_group, SplitDirection::Vertical, true)
                    .is_empty(),
                "editor:focus-top" => !self
                    .model
                    .layout
                    .neighbors(self.model.active_group, SplitDirection::Horizontal, false)
                    .is_empty(),
                "editor:focus-bottom" => !self
                    .model
                    .layout
                    .neighbors(self.model.active_group, SplitDirection::Horizontal, true)
                    .is_empty(),
                id if tft_app::text::table::TableCommand::from_id(id).is_some() => {
                    can_edit_markdown
                        && tft_app::text::table::TableCommand::from_id(id).is_some_and(|command| {
                            self.active_editor().read(cx).table_command_applies(command)
                        })
                }
                "graph:open-local" => active_tab
                    .and_then(|tab| tab.path.as_deref())
                    .is_some_and(|path| self.vault.lookup(path).is_ok()),
                "file-explorer:reveal-active-file" => {
                    active_tab.and_then(|tab| tab.path.as_deref()).is_some()
                }
                "file-explorer:duplicate-file" | "app:delete-file" => has_document,
                "editor:open-search"
                | "editor:open-search-replace"
                | "editor:follow-link"
                | "editor:open-link-in-new-leaf"
                | "editor:open-link-in-new-split" => can_edit_markdown,
                id if Self::is_markdown_editor_command(id) => can_edit_markdown,
                _ => true,
            })
            .filter_map(|id| registry.get(id))
            .map(|command| tft_app::suggest_core::CommandEntry {
                id: command.id.clone(),
                name: command.name.clone(),
                hotkeys: command
                    .hotkeys
                    .iter()
                    .filter(|_| command.id != "workspace:undo-close-pane")
                    .map(|hotkey| {
                        tft_app::commands::hotkey_label_with_modifier(hotkey, &modifier_label)
                    })
                    .collect(),
            })
            .collect();
        self.quick_switcher_open = false;
        self.command_palette_open = true;
        self.command_palette.update(cx, |palette, cx| {
            palette.open(commands, self.recent_commands.clone(), "", window, cx)
        });
        cx.notify();
    }

    fn file_tree_row_hover(
        &mut self,
        node: Option<NodeId>,
        path: String,
        over: bool,
        anchor: Bounds<Pixels>,
        modifiers: gpui::Modifiers,
        source: tft_app::page_preview::Source,
        cx: &mut Context<Self>,
    ) {
        let requires_mod = source.requires_mod(&self.page_preview_config);
        if over {
            let Some(node) = node.filter(|node| {
                matches!(
                    self.vault.node(*node).map(|node| &node.kind),
                    Some(NodeKind::File(meta))
                        if meta.kind == tft_formats::FileKind::Markdown
                )
            }) else {
                return;
            };
            let preview = WebFilePreview {
                node,
                path,
                anchor,
                offset: None,
                depth: 0,
            };
            self.hovered_preview_file = Some((preview.clone(), requires_mod));
            self.hovered_preview_link = None;
            if !requires_mod || modifiers.secondary() {
                self.schedule_web_file_preview(preview, cx);
            } else {
                self.arm_file_preview_hide(cx);
            }
            return;
        }
        self.hovered_preview_file = None;

        let matches_current = self
            .file_preview_pending
            .as_ref()
            .is_some_and(|item| item.path == path)
            || self
                .file_previews
                .iter()
                .flatten()
                .any(|item| item.path == path);
        if matches_current {
            self.arm_file_preview_hide(cx);
        }
    }

    fn link_preview_hover(
        &mut self,
        hover: Option<reading::LinkHover>,
        source_path: String,
        source: tft_app::page_preview::Source,
        depth: usize,
        cx: &mut Context<Self>,
    ) {
        let requires_mod = source.requires_mod(&self.page_preview_config);
        let Some(hover) = hover else {
            self.hovered_preview_link = None;
            self.arm_file_preview_hide(cx);
            return;
        };
        self.hovered_preview_file = None;
        self.hovered_preview_link = Some((hover.clone(), source_path.clone(), source, depth));
        if depth >= WEB_PREVIEW_DEPTH_LIMIT {
            self.arm_file_preview_hide(cx);
            return;
        }
        let reading::LinkTarget::Internal(body) = hover.target else {
            self.arm_file_preview_hide(cx);
            return;
        };
        if requires_mod && !hover.modifiers.secondary() {
            self.arm_file_preview_hide(cx);
            return;
        }
        let resolver = vault_resolver(&self.vault);
        let resolved = tft_app::resolve_internal_link(&resolver, &body, &source_path);
        let Some(path) = resolved.path else {
            self.arm_file_preview_hide(cx);
            return;
        };
        let Ok(node) = self.vault.lookup(&path) else {
            self.arm_file_preview_hide(cx);
            return;
        };
        if !matches!(
            self.vault.node(node).map(|entry| &entry.kind),
            Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Markdown
        ) {
            self.arm_file_preview_hide(cx);
            return;
        }
        let offset = self.link_subpath_offset(&path, resolved.link.subpath.as_ref());
        self.schedule_web_file_preview(
            WebFilePreview {
                node,
                path,
                anchor: hover.anchor,
                offset,
                depth,
            },
            cx,
        );
    }

    fn file_preview_modifiers_changed(
        &mut self,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        if modifiers.secondary() {
            if let Some((mut hover, source_path, source, depth)) = self.hovered_preview_link.clone()
            {
                hover.modifiers = modifiers;
                self.link_preview_hover(Some(hover), source_path, source, depth, cx);
            } else if let Some((preview, requires_mod)) = self.hovered_preview_file.clone() {
                if requires_mod {
                    self.schedule_web_file_preview(preview, cx);
                }
            }
        }
    }

    fn edit_file_preview(
        &mut self,
        depth: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_editors.contains_key(&depth) {
            return;
        }
        let Some(preview) = self
            .file_previews
            .get(depth)
            .and_then(Option::as_ref)
            .cloned()
        else {
            return;
        };
        let text = document_text(&self.vault, preview.node);
        let editor = cx.new(|cx| Editor::new(window, cx));
        editor.update(cx, |editor, cx| {
            editor.set_readable_line_length(false);
            editor.set_show_inline_title(false);
            editor.set_live_preview(true, cx);
            editor.set_text(preview.path.clone(), text, cx);
        });
        let subscription = cx.subscribe(
            &editor,
            move |this, editor, event: &tft_app::editor::EditorEvent, cx| {
                this.preview_editor_event(depth, &editor, event, cx)
            },
        );
        self.preview_editors.insert(
            depth,
            WebPreviewEditor {
                editor: editor.clone(),
                _subscription: subscription,
            },
        );
        window.focus(&editor.focus_handle(cx), cx);
        place_web_preview_caret(editor.downgrade(), position, 4, window);
        cx.notify();
    }

    fn preview_editor_event(
        &mut self,
        depth: usize,
        editor: &Entity<Editor>,
        event: &tft_app::editor::EditorEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = self.file_previews.get(depth).and_then(Option::as_ref) else {
            return;
        };
        let path = preview.path.clone();
        let node = preview.node;
        match event {
            tft_app::editor::EditorEvent::Edited => {
                let contents = editor.read(cx).file_contents();
                self.vault.set_text(node, contents.clone());
                self.sync_document_editors(editor, node, &path, &contents, cx);
                self.upsert_search_document(node, path, contents.clone());
                self.schedule_graph_rebuild(cx, false);
                cx.emit(WebWorkspaceEvent::Edited { node, contents });
                self.publish_test_state();
                cx.notify();
            }
            tft_app::editor::EditorEvent::Hover(hover) => self.link_preview_hover(
                hover.clone(),
                path,
                tft_app::page_preview::Source::Editor,
                depth + 1,
                cx,
            ),
            tft_app::editor::EditorEvent::OpenLink(target)
            | tft_app::editor::EditorEvent::OpenLinkInNewTab(target) => {
                let new_tab = matches!(event, tft_app::editor::EditorEvent::OpenLinkInNewTab(_));
                match target {
                    reading::LinkTarget::External(url) => cx.open_url(url),
                    _ => {
                        self.hide_file_preview(cx);
                        self.pending_link_source = Some(path);
                        self.pending_link = Some((target.clone(), new_tab));
                        cx.notify();
                    }
                }
            }
            _ => {}
        }
    }

    fn flush_preview_editors_from(&mut self, depth: usize, cx: &mut Context<Self>) {
        let dirty = self
            .preview_editors
            .iter()
            .filter(|(preview_depth, _)| **preview_depth >= depth)
            .filter_map(|(preview_depth, editing)| {
                let preview = self.file_previews.get(*preview_depth)?.as_ref()?;
                let editor = editing.editor.clone();
                editor.read(cx).is_dirty().then(|| {
                    (
                        preview.node,
                        preview.path.clone(),
                        editor.read(cx).file_contents(),
                        editor,
                    )
                })
            })
            .collect::<Vec<_>>();
        for (node, path, contents, editor) in dirty {
            self.vault.set_text(node, contents.clone());
            self.sync_document_editors(&editor, node, &path, &contents, cx);
            self.upsert_search_document(node, path, contents.clone());
            self.schedule_graph_rebuild(cx, false);
            cx.emit(WebWorkspaceEvent::Edited { node, contents });
        }
        self.preview_editors
            .retain(|preview_depth, _| *preview_depth < depth);
    }

    fn schedule_web_file_preview(&mut self, pending: WebFilePreview, cx: &mut Context<Self>) {
        self.file_preview_generation = self.file_preview_generation.wrapping_add(1);
        self.file_preview_hide_generation = self.file_preview_hide_generation.wrapping_add(1);
        self.file_preview_pointer_over.truncate(pending.depth + 1);
        while self.file_preview_pointer_over.len() < WEB_PREVIEW_DEPTH_LIMIT {
            self.file_preview_pointer_over.push(false);
        }
        for pointer_over in self
            .file_preview_pointer_over
            .iter_mut()
            .skip(pending.depth)
        {
            *pointer_over = false;
        }
        self.flush_preview_editors_from(pending.depth, cx);
        for preview in self.file_previews.iter_mut().skip(pending.depth) {
            *preview = None;
        }
        let generation = self.file_preview_generation;
        self.file_preview_pending = Some(pending.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            this.update(cx, |this, cx| {
                if this.file_preview_generation != generation
                    || this.file_preview_pending.as_ref().is_none_or(|item| {
                        item.node != pending.node
                            || item.path != pending.path
                            || item.depth != pending.depth
                    })
                {
                    return;
                }
                let text = document_text(&this.vault, pending.node);
                let source = VaultEmbedSource::new(&this.vault);
                let model = reading::build_with(
                    &pending.path,
                    &text,
                    &source,
                    reading::BuildOptions::default(),
                );
                let Some(reader) = this.file_preview_views.get(pending.depth).cloned() else {
                    return;
                };
                reader.update(cx, |reader, cx| {
                    reader.set_note(Some(pending.path.clone()), model, cx);
                    if let Some(offset) = pending.offset {
                        reader.scroll_to_offset(offset, cx);
                    }
                });
                if let Some(preview) = this.file_previews.get_mut(pending.depth) {
                    *preview = Some(pending.clone());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn file_preview_popup_hover(&mut self, depth: usize, over: bool, cx: &mut Context<Self>) {
        if let Some(pointer_over) = self.file_preview_pointer_over.get_mut(depth) {
            *pointer_over = over;
        }
        if over {
            self.file_preview_hide_generation = self.file_preview_hide_generation.wrapping_add(1);
        } else {
            self.arm_file_preview_hide(cx);
        }
    }

    fn hide_file_preview(&mut self, cx: &mut Context<Self>) {
        self.file_preview_generation = self.file_preview_generation.wrapping_add(1);
        self.file_preview_hide_generation = self.file_preview_hide_generation.wrapping_add(1);
        self.flush_preview_editors_from(0, cx);
        self.file_preview_pending = None;
        self.file_previews.fill(None);
        self.hovered_preview_link = None;
        self.hovered_preview_file = None;
        self.file_preview_pointer_over.fill(false);
        cx.notify();
    }

    fn close_file_preview_from(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.flush_preview_editors_from(depth, cx);
        for preview in self.file_previews.iter_mut().skip(depth) {
            *preview = None;
        }
        for pointer_over in self.file_preview_pointer_over.iter_mut().skip(depth) {
            *pointer_over = false;
        }
        if self
            .file_preview_pending
            .as_ref()
            .is_some_and(|pending| pending.depth >= depth)
        {
            self.file_preview_generation = self.file_preview_generation.wrapping_add(1);
            self.file_preview_pending = None;
        }
        cx.notify();
    }

    fn arm_file_preview_hide(&mut self, cx: &mut Context<Self>) {
        self.file_preview_hide_generation = self.file_preview_hide_generation.wrapping_add(1);
        let generation = self.file_preview_hide_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            this.update(cx, |this, cx| {
                if this.file_preview_hide_generation == generation
                    && !this.file_preview_pointer_over.iter().any(|over| *over)
                {
                    this.file_preview_generation = this.file_preview_generation.wrapping_add(1);
                    this.file_preview_pending = None;
                    this.file_previews.fill(None);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_switcher_open = true;
        self.quick_switcher_selected = 0;
        self.quick_switcher_input
            .update(cx, |input, cx| input.set_text("", cx));
        let focus = self.quick_switcher_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn quick_switcher_results(&self, cx: &App) -> Vec<QuickSwitcherHit> {
        let query = self.quick_switcher_input.read(cx).text().trim().to_owned();
        let mut rows = Vec::new();
        let documents = self
            .search_documents
            .iter()
            .map(|document| (document.node, document))
            .collect::<HashMap<_, _>>();
        for (node, file) in self.vault.file_ids() {
            if !tft_app::file_switcher::offers(
                file.kind,
                tft_app::fixed::SHOW_UNSUPPORTED_FILES,
                &self.switcher_config,
            ) {
                continue;
            }
            let downranked = self.excluded.is_excluded(&file.path);
            let name =
                tft_app::explorer::display_name(file.path.rsplit('/').next().unwrap_or(&file.path))
                    .to_owned();
            rows.push((node, file.path.clone(), name, false, downranked));
            if !query.is_empty() {
                if let Some(document) = documents.get(&node) {
                    for alias in tft_cache::frontmatter_aliases(&document.metadata) {
                        rows.push((node, file.path.clone(), alias, true, downranked));
                    }
                }
            }
        }
        let candidates = rows
            .iter()
            .map(
                |(_, path, label, alias, downranked)| tft_app::file_switcher::SwitchCandidate {
                    path,
                    label,
                    alias: *alias,
                    downranked: *downranked,
                },
            )
            .collect::<Vec<_>>();
        tft_app::file_switcher::rank_candidates(&candidates, &self.recent_files, &query)
            .into_iter()
            .take(50)
            .map(|ranked| {
                let (node, path, label, alias, downranked) = &rows[ranked.candidate_index];
                QuickSwitcherHit {
                    node: *node,
                    path: path.clone(),
                    label: label.clone(),
                    alias: *alias,
                    ranges: ranked.ranges,
                    downranked: *downranked,
                }
            })
            .collect()
    }

    fn open_quick_switcher_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(hit) = self
            .quick_switcher_results(cx)
            .get(self.quick_switcher_selected)
            .cloned()
        {
            self.pending_quick_switcher_open = Some((hit.node, hit.path));
            self.quick_switcher_open = false;
            cx.notify();
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        self.store_editor_text(active_node, cx);
        let query = self.search_input.read(cx).text().trim().to_owned();
        let query_tree = tft_search::parse(&query);
        let query_tree = if self.search_matching_case {
            tft_search::Query::CaseSensitive(Box::new(query_tree))
        } else {
            query_tree
        };
        let search = match tft_search::Search::from_query(query_tree) {
            Ok(search) => search,
            Err(error) => {
                self.search_error = Some(error.to_string());
                cx.notify();
                self.publish_test_state();
                return;
            }
        };
        if search.is_empty() {
            cx.notify();
            self.publish_test_state();
            return;
        }
        for file in &self.search_documents {
            if self.excluded.is_excluded(&file.path) {
                continue;
            }
            let contents = file.contents.as_str();
            let inputs = tft_search::inputs(&file.metadata, contents);
            let search_file = tft_search::SearchFile {
                path: &file.path,
                content: contents,
                tags: &inputs.tags,
                frontmatter: file.metadata.frontmatter.as_ref(),
                blocks: &inputs.blocks,
                sections: &inputs.sections,
                tasks: &inputs.tasks,
            };
            let Some(matches) = search.matches(&search_file) else {
                continue;
            };
            let name = file
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&file.path)
                .trim_end_matches(".md")
                .to_owned();
            let name_ranges = matches
                .name
                .into_iter()
                .filter(|range| range.start <= range.end && range.end <= name.len())
                .collect::<Vec<_>>();
            let snippets = tft_search::match_lines(contents, &matches.content);
            let match_count = snippets.len() + usize::from(!name_ranges.is_empty());
            self.search_hits.push(SearchHit {
                node: file.node,
                path: file.path.clone(),
                name,
                name_ranges,
                snippets,
                hits: matches.content,
                contents: file.contents.clone(),
                match_count,
                mtime: file.mtime,
                ctime: file.ctime,
            });
        }
        sort_web_search_hits(&mut self.search_hits, self.search_sort);
        cx.notify();
        self.publish_test_state();
    }

    fn show_search(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.model.active().and_then(|group| {
            group
                .tabs
                .iter()
                .position(|tab| tab.mode == ViewMode::Search)
        });
        if let Some(index) = existing {
            self.activate_tab(index, window, cx);
        } else {
            let active_node = self
                .model
                .active()
                .and_then(|group| group.active())
                .and_then(|tab| tab.node);
            self.store_editor_text(active_node, cx);
            if self.model.new_tab(ViewMode::Search).is_none() {
                return;
            }
            self.active_editor().update(cx, |editor, cx| {
                editor.set_text(String::new(), String::new(), cx)
            });
        }
        self.search_input
            .update(cx, |input, cx| input.set_text(query, cx));
        let focus = self.search_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn run_editor_context_action(
        &mut self,
        action: EditorContextAction,
        menu: EditorContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor_context_menu = None;
        let Some(editor) = self
            .group_views
            .get(&menu.group_id)
            .map(|views| views.editor.clone())
        else {
            cx.notify();
            return;
        };
        match action {
            EditorContextAction::OpenInNewTab | EditorContextAction::OpenToRight => {
                if let Some(target) = menu.link {
                    match &target {
                        reading::LinkTarget::External(url) => cx.open_url(url),
                        _ => {
                            let source = self
                                .model
                                .group(menu.group_id)
                                .and_then(|group| group.active())
                                .and_then(|tab| tab.path.clone());
                            self.activate_group(menu.group_id, cx);
                            if matches!(action, EditorContextAction::OpenToRight) {
                                self.split_active_group(SplitDirection::Vertical, window, cx);
                            }
                            self.pending_link_source = source;
                            self.pending_link =
                                Some((target, matches!(action, EditorContextAction::OpenInNewTab)));
                            cx.notify();
                        }
                    }
                }
            }
            EditorContextAction::AddLink => editor.update(cx, |editor, cx| {
                editor.insert_link(tft_app::text::LinkSyntax::Wikilink, window, cx)
            }),
            EditorContextAction::AddExternalLink => editor.update(cx, |editor, cx| {
                editor.insert_link(tft_app::text::LinkSyntax::Markdown, window, cx)
            }),
            EditorContextAction::SearchSelection => {
                self.activate_group(menu.group_id, cx);
                self.show_search(&menu.selection, window, cx);
            }
            EditorContextAction::Cut if menu.selected => {
                editor.update(cx, |editor, cx| editor.cut_selection(window, cx))
            }
            EditorContextAction::Copy if menu.selected => {
                editor.update(cx, |editor, cx| editor.copy_selection(cx))
            }
            EditorContextAction::Paste => {
                editor.update(cx, |editor, cx| editor.paste_plain(window, cx))
            }
            EditorContextAction::PastePlain => {
                editor.update(cx, |editor, cx| editor.paste_plain(window, cx))
            }
            EditorContextAction::SelectAll => {
                editor.update(cx, |editor, cx| editor.select_all_text(cx))
            }
            EditorContextAction::CopyPath => {
                if let Some(path) = menu.link_path.as_ref() {
                    cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                }
            }
            EditorContextAction::RenameFile => {
                if let Some(path) = menu.link_path.as_ref() {
                    cx.emit(WebWorkspaceEvent::RenameFilePromptRequested { path: path.clone() });
                }
            }
            EditorContextAction::MoveFile => {
                if let Some(path) = menu.link_path.as_ref() {
                    cx.emit(WebWorkspaceEvent::MoveFilePromptRequested { path: path.clone() });
                }
            }
            EditorContextAction::OpenInDrive => {
                if let Some(id) = menu.drive_id.as_deref() {
                    tft_drive_web::open_drive_file(id);
                }
            }
            EditorContextAction::RevealInNavigation => {
                if let Some(path) = menu.link_path.as_deref() {
                    self.tree.reveal(path);
                    self.tree_focused_key = self
                        .tree
                        .visible_index(path)
                        .and_then(|index| self.tree.row(index).map(|row| row.key.clone()));
                    self.model.dock.show(LeftTab::Files);
                    self.set_left_dock_visible(true, cx);
                    self.emit_workspace_state(cx);
                }
            }
            EditorContextAction::Cut | EditorContextAction::Copy => {}
        }
        cx.notify();
    }

    fn render_editor_context_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let menu = self.editor_context_menu.clone()?;
        let viewport = window.viewport_size();
        let mut rows: Vec<(usize, SharedString, &'static str, EditorContextAction, bool)> =
            Vec::new();
        if menu.link_path.is_some() {
            rows.push((
                0,
                "Open in new tab".into(),
                "file-plus",
                EditorContextAction::OpenInNewTab,
                true,
            ));
            rows.push((
                0,
                "Open to the right".into(),
                "separator-vertical",
                EditorContextAction::OpenToRight,
                tft_app::workspace::workspace_can_split(self.model.groups.len()),
            ));
        }
        if menu.editor {
            rows.push((
                1,
                "Add link".into(),
                "link",
                EditorContextAction::AddLink,
                true,
            ));
            rows.push((
                1,
                "Add external link".into(),
                "external-link",
                EditorContextAction::AddExternalLink,
                true,
            ));
            if !menu.selection.is_empty() {
                rows.push((
                    2,
                    format!("Search for “{}”", menu.selection).into(),
                    "search",
                    EditorContextAction::SearchSelection,
                    true,
                ));
            }
            rows.extend([
                (
                    3,
                    "Cut".into(),
                    "scissors",
                    EditorContextAction::Cut,
                    menu.selected,
                ),
                (
                    3,
                    "Copy".into(),
                    "copy",
                    EditorContextAction::Copy,
                    menu.selected,
                ),
                (
                    3,
                    "Paste".into(),
                    "clipboard-check",
                    EditorContextAction::Paste,
                    true,
                ),
                (
                    3,
                    "Paste as plain text".into(),
                    "clipboard-type",
                    EditorContextAction::PastePlain,
                    true,
                ),
                (
                    3,
                    "Select all".into(),
                    "square-dashed",
                    EditorContextAction::SelectAll,
                    true,
                ),
            ]);
        }
        if menu.link_path.is_some() {
            rows.push((
                4,
                "Copy path".into(),
                "clipboard",
                EditorContextAction::CopyPath,
                true,
            ));
            rows.push((
                5,
                "Rename...".into(),
                "pen-line",
                EditorContextAction::RenameFile,
                true,
            ));
            rows.push((
                5,
                "Move file to...".into(),
                "folder-tree",
                EditorContextAction::MoveFile,
                true,
            ));
            if menu.drive_id.is_some() {
                rows.push((
                    6,
                    "Open in Google Drive".into(),
                    "external-link",
                    EditorContextAction::OpenInDrive,
                    true,
                ));
            }
            rows.push((
                7,
                "Reveal in navigation".into(),
                "folder-open",
                EditorContextAction::RevealInNavigation,
                true,
            ));
        }
        let mut children: Vec<gpui::AnyElement> = Vec::new();
        let mut previous_section = None;
        for (index, (section, label, icon, action, enabled)) in rows.into_iter().enumerate() {
            if previous_section.is_some_and(|previous| previous != section) {
                children.push(tft_app::ui::menu_separator(theme).into_any_element());
            }
            previous_section = Some(section);
            let mut row = tft_app::ui::menu_item_with_enabled(
                "web-editor-context-menu-row",
                index,
                Some(icon),
                label,
                false,
                enabled,
                theme,
            );
            if enabled {
                let menu = menu.clone();
                row = row.on_click(cx.listener(move |this, _, window, cx| {
                    this.run_editor_context_action(action, menu.clone(), window, cx);
                }));
            }
            children.push(row.into_any_element());
        }
        let weak = cx.entity().downgrade();
        let mut menu_view = tft_app::ui::menu("web-editor-context-menu", theme)
            .absolute()
            .left(menu.position.x.min((viewport.width - px(230.)).max(px(0.))))
            .top(
                menu.position
                    .y
                    .min((viewport.height - px(300.)).max(px(0.))),
            )
            .children(children)
            .on_mouse_down_out(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.editor_context_menu = None;
                    cx.notify();
                })
                .ok();
            });
        let weak = cx.entity().downgrade();
        menu_view = menu_view.capture_key_down(move |event, _, app| {
            if event.keystroke.key == "escape" {
                app.stop_propagation();
                weak.update(app, |this, cx| {
                    this.editor_context_menu = None;
                    cx.notify();
                })
                .ok();
            }
        });
        Some(menu_view.into_any_element())
    }

    fn render_search_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.search_matching_case;
        let settings_open = self.search_settings_open;
        let mut active_bg = theme.tokens.interactive_accent;
        active_bg.a = 0.1;
        let mut controls = div()
            .relative()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .id("web-search-match-case")
                    .w(px(28.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(active, |element| element.bg(active_bg))
                    .hover(|style| style.bg(theme.tokens.background_modifier_hover))
                    .text_size(px(12.))
                    .text_color(if active {
                        theme.tokens.interactive_accent
                    } else {
                        theme.tokens.icon_color
                    })
                    .child("Aa")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.search_matching_case = !this.search_matching_case;
                        this.rerun_search(cx);
                    })),
            )
            .child(
                div()
                    .id("web-search-settings")
                    .w(px(28.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(settings_open, |element| {
                        element.bg(theme.tokens.background_modifier_hover)
                    })
                    .hover(|style| style.bg(theme.tokens.background_modifier_hover))
                    .text_color(theme.tokens.icon_color)
                    .child(tft_app::assets::icon(
                        "sliders-horizontal",
                        15.,
                        theme.tokens.icon_color,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.search_settings_open = !this.search_settings_open;
                        cx.notify();
                    })),
            );
        if settings_open {
            let rows = [
                ("Collapse results", self.search_collapse_all, 0usize),
                ("Show more context", self.search_extra_context, 1),
                ("Explain search terms", self.search_explain, 2),
            ];
            controls = controls.child(
                tft_app::ui::menu("web-search-settings-menu", theme)
                    .absolute()
                    .top(px(32.))
                    .right(px(0.))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.search_settings_open = false;
                        cx.notify();
                    }))
                    .children(rows.into_iter().enumerate().map(
                        |(index, (label, checked, setting))| {
                            tft_app::ui::menu_item(
                                "web-search-setting",
                                index,
                                None,
                                label,
                                checked,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    match setting {
                                        0 => {
                                            this.search_collapse_all = !this.search_collapse_all;
                                            this.search_toggled_paths.clear();
                                            if this.search_collapse_all {
                                                this.selected_search_match = None;
                                            }
                                        }
                                        1 => this.search_extra_context = !this.search_extra_context,
                                        _ => this.search_explain = !this.search_explain,
                                    }
                                    this.search_settings_open = false;
                                    cx.notify();
                                },
                            ))
                        },
                    )),
            );
        }
        controls.into_any_element()
    }

    fn render_search_results(
        &self,
        id: impl Into<gpui::ElementId>,
        group_id: Option<u64>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tokens = &theme.tokens;
        let scroll_handle = self.search_scroll_handle(group_id);
        let query = self.search_input.read(cx).text().trim().to_owned();
        let query_is_empty = query.is_empty();
        let explanation = if self.search_explain && !query_is_empty {
            let parsed = tft_search::parse(&query);
            let parsed = if self.search_matching_case {
                tft_search::Query::CaseSensitive(Box::new(parsed))
            } else {
                parsed
            };
            Some(parsed.explain())
        } else {
            None
        };
        let error = self.search_error.clone();
        let selected = self.selected_search_hit;
        let selected_match = self.selected_search_match;
        let hits = self.search_hits.clone();
        let result_count = hits.iter().map(|hit| hit.match_count).sum::<usize>();
        let sort = self.search_sort;
        let sort_menu_open = self.search_sort_menu_open;
        let collapse_all = self.search_collapse_all;
        let extra_context = self.search_extra_context;
        let show_no_results = !query_is_empty && hits.is_empty() && error.is_none();
        div()
            .id(id)
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&scroll_handle)
            .when_some(error, |container, error| {
                container.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(tokens.text_muted)
                        .child(error),
                )
            })
            .when(query_is_empty, |container| {
                container.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(tokens.text_muted)
                        .child("Type to search this Vault."),
                )
            })
            .when(show_no_results, |container| {
                container.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(tokens.text_muted)
                        .child("No matching notes."),
                )
            })
            .when_some(explanation, |container, explanation| {
                container.child(
                    div()
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .text_size(px(11.))
                        .text_color(tokens.text_muted)
                        .child(explanation),
                )
            })
            .when(!query_is_empty && !hits.is_empty(), |container| {
                let menu = sort_menu_open.then(|| {
                    let mut menu = tft_app::ui::menu("web-search-sort-menu", theme)
                        .absolute()
                        .top(px(32.))
                        .right(px(8.))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.search_sort_menu_open = false;
                            cx.notify();
                        }));
                    for (index, order) in SortOrder::ALL.into_iter().enumerate() {
                        if index > 0 && index % 2 == 0 {
                            menu = menu.child(tft_app::ui::file_tree_sort_separator(theme));
                        }
                        menu = menu.child(
                            tft_app::ui::file_tree_sort_item(
                                "web-search-sort-option",
                                index,
                                order,
                                sort,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let focused_path = this
                                        .selected_search_hit
                                        .and_then(|index| this.search_hits.get(index))
                                        .map(|hit| hit.path.clone());
                                    let focused_offset =
                                        this.selected_search_match.and_then(|(hit, snippet)| {
                                            this.search_hits
                                                .get(hit)?
                                                .snippets
                                                .get(snippet)
                                                .map(|line| line.offset)
                                        });
                                    this.search_sort = order;
                                    sort_web_search_hits(&mut this.search_hits, order);
                                    if let Some(path) = focused_path {
                                        this.selected_search_hit = this
                                            .search_hits
                                            .iter()
                                            .position(|hit| hit.path == path);
                                        this.selected_search_match = this
                                            .selected_search_hit
                                            .zip(focused_offset)
                                            .and_then(|(hit, offset)| {
                                                this.search_hits[hit]
                                                    .snippets
                                                    .iter()
                                                    .position(|line| line.offset == offset)
                                                    .map(|snippet| (hit, snippet))
                                            });
                                    }
                                    this.search_sort_menu_open = false;
                                    cx.notify();
                                },
                            )),
                        );
                    }
                    menu.into_any_element()
                });
                container.child(
                    div()
                        .relative()
                        .h(px(32.))
                        .flex_shrink_0()
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(tokens.background_modifier_border)
                        .text_size(px(11.))
                        .text_color(tokens.text_muted)
                        .child(format!("{result_count} matches in {} files", hits.len()))
                        .child(
                            div().flex().items_center().gap(px(4.)).children([
                                div()
                                    .id("web-search-collapse-all")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(tokens.background_modifier_hover))
                                    .child(if collapse_all {
                                        "Expand all"
                                    } else {
                                        "Collapse all"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.search_collapse_all = !this.search_collapse_all;
                                        this.search_toggled_paths.clear();
                                        if this.search_collapse_all {
                                            this.selected_search_match = None;
                                        }
                                        cx.notify();
                                    })),
                                div()
                                    .id("web-search-sort")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(tokens.background_modifier_hover))
                                    .text_color(tokens.text_normal)
                                    .child(sort.label())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.search_sort_menu_open = !this.search_sort_menu_open;
                                        cx.notify();
                                    })),
                            ]),
                        )
                        .children(menu),
                )
            })
            .children(hits.into_iter().enumerate().map(|(index, hit)| {
                let node = hit.node;
                let path = hit.path.clone();
                let toggled = self.search_toggled_paths.contains(&path);
                let collapsed = self.search_collapse_all ^ toggled;
                let name = StyledText::new(SharedString::from(hit.name.clone())).with_highlights(
                    hit.name_ranges
                        .iter()
                        .map(|range| {
                            (
                                range.clone(),
                                HighlightStyle {
                                    background_color: Some(tokens.highlight_background),
                                    ..HighlightStyle::default()
                                },
                            )
                        })
                        .collect::<Vec<_>>(),
                );
                let title_path = path.clone();
                let context_path = path.clone();
                tft_app::ui::file_tree_row(
                    SharedString::from(format!("search-{}", hit.path)),
                    0,
                    Some(index) == selected
                        && selected_match.is_none_or(|(hit_index, _)| hit_index != index),
                    false,
                    false,
                    theme,
                )
                .h_auto()
                .items_start()
                .py_1()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_search_hit = Some(index);
                    this.selected_search_match = None;
                    if !this.search_toggled_paths.remove(&title_path) {
                        this.search_toggled_paths.insert(title_path.clone());
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.file_tree_context_menu = Some(FileTreeContextMenu {
                            position: event.position,
                            node: Some(node),
                            key: context_path.clone(),
                            path: context_path.clone(),
                            is_folder: false,
                        });
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(name)
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(tokens.text_faint)
                                        .child(hit.match_count.to_string()),
                                ),
                        )
                        .when(!collapsed, |content| {
                            content.children(hit.snippets.into_iter().enumerate().map(
                                |(snippet_index, line)| {
                                    let line = if extra_context {
                                        tft_search::context_window(
                                            &hit.contents,
                                            line.offset,
                                            1,
                                            1,
                                            &hit.hits,
                                        )
                                    } else {
                                        line
                                    };
                                    let offset = line.offset;
                                    let snippet_node = node;
                                    let snippet_path = path.clone();
                                    let text = StyledText::new(SharedString::from(line.snippet))
                                        .with_highlights(
                                            line.ranges
                                                .iter()
                                                .map(|range| {
                                                    (
                                                        range.clone(),
                                                        HighlightStyle {
                                                            background_color: Some(
                                                                tokens.highlight_background,
                                                            ),
                                                            color: Some(tokens.text_normal),
                                                            ..HighlightStyle::default()
                                                        },
                                                    )
                                                })
                                                .collect::<Vec<_>>(),
                                        );
                                    div()
                                        .id(SharedString::from(format!(
                                            "web-search-match-{index}-{offset}"
                                        )))
                                        .mt_1()
                                        .text_size(px(11.))
                                        .text_color(tokens.text_muted)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .cursor_pointer()
                                        .when(
                                            selected_match == Some((index, snippet_index)),
                                            |row| row.bg(tokens.nav_item_background_active),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.selected_search_hit = Some(index);
                                            this.selected_search_match =
                                                Some((index, snippet_index));
                                            this.pending_search_open = Some((index, Some(offset)));
                                            if let Some(group_id) = group_id {
                                                this.activate_group(group_id, cx);
                                            }
                                            cx.notify();
                                        }))
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            cx.listener(
                                                move |this, event: &MouseDownEvent, _, cx| {
                                                    cx.stop_propagation();
                                                    this.file_tree_context_menu =
                                                        Some(FileTreeContextMenu {
                                                            position: event.position,
                                                            node: Some(snippet_node),
                                                            key: snippet_path.clone(),
                                                            path: snippet_path.clone(),
                                                            is_folder: false,
                                                        });
                                                    cx.notify();
                                                },
                                            ),
                                        )
                                        .child(text)
                                },
                            ))
                        }),
                )
            }))
            .into_any_element()
    }

    #[cfg(feature = "browser-test")]
    pub fn from_documents(
        documents: Vec<NoteDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_path = documents.first().map(|doc| doc.path.clone());
        let files = documents
            .iter()
            .map(|doc| VaultFile {
                path: doc.path.clone(),
                kind: tft_formats::FileKind::from_path(&doc.path),
                size: doc.contents.len() as u64,
                mtime: 0,
                ctime: 0,
            })
            .collect::<Vec<_>>();
        let mut folders = BTreeSet::new();
        let mut texts = std::collections::HashMap::new();
        for doc in &documents {
            if let Some((directory, _)) = doc.path.rsplit_once('/') {
                let mut parent = String::new();
                for segment in directory.split('/') {
                    if !parent.is_empty() {
                        parent.push('/');
                    }
                    parent.push_str(segment);
                    folders.insert(parent.clone());
                }
            }
            texts.insert(doc.path.clone(), Arc::new(doc.contents.clone()));
        }

        let vault = VaultData::from_flat(files, folders.into_iter().collect(), texts);
        let initial_node = initial_path.and_then(|path| vault.lookup(&path).ok());
        Self::from_vault(vault, initial_node, window, cx)
    }

    fn open(
        &mut self,
        node: NodeId,
        path: String,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.radial_tree_active.remove(&self.model.active_group);
        self.recent_files.retain(|recent| recent != &path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(50);
        let is_markdown = matches!(
            self.vault.node(node).map(|node| &node.kind),
            Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Markdown
        );
        if !is_markdown
            && matches!(
                self.vault.node(node).map(|node| &node.kind),
                Some(NodeKind::File(meta))
                    if matches!(meta.kind, tft_formats::FileKind::Image | tft_formats::FileKind::Audio | tft_formats::FileKind::Video)
            )
        {
            self.load_image_preview(node, cx);
        }
        if !is_markdown
            && matches!(
                self.vault.node(node).map(|node| &node.kind),
                Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Pdf
            )
        {
            self.load_pdf(node, cx);
        }
        if !is_markdown
            && matches!(
                self.vault.node(node).map(|node| &node.kind),
                Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Canvas
            )
        {
            self.load_canvas(node, cx);
        }
        if !new_tab {
            let existing = self
                .model
                .active()
                .and_then(|group| group.tabs.iter().position(|tab| tab.node == Some(node)));
            if let Some(index) = existing {
                let active_index = self.model.active().map(|group| group.active_tab);
                if active_index != Some(index) {
                    self.activate_tab(index, window, cx);
                } else if self
                    .model
                    .active()
                    .and_then(|group| group.active())
                    .is_some_and(|tab| tab.mode == ViewMode::Graph)
                {
                    self.toggle_graph_view(self.model.active_group, window, cx);
                } else {
                    // Re-opening the already active note still changes the
                    // persisted recent-files ordering even though no tab moves.
                    self.emit_workspace_state(cx);
                }
                self.follow_local_graph_center(&path);
                self.emit_workspace_state(cx);
                return;
            }
        }
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        let active_tab = self.model.active().and_then(|group| group.active());
        let graph_previous_mode = active_tab.and_then(|tab| {
            (tab.mode == ViewMode::Graph)
                .then(|| self.graph_previous_modes.get(&self.model.active_group))
                .flatten()
                .filter(|(node, path, _)| *node == tab.node && path == &tab.path)
                .map(|(_, _, mode)| *mode)
        });
        let mode = if is_markdown {
            if new_tab {
                ViewMode::markdown_document_mode(None)
            } else {
                ViewMode::markdown_document_mode(
                    graph_previous_mode.or_else(|| active_tab.map(|tab| tab.mode)),
                )
            }
        } else {
            ViewMode::Source
        };
        if new_tab || active_node != Some(node) {
            self.stash_group_view_state(self.model.active_group, cx);
            if new_tab {
                self.model
                    .open_node_in_new_tab(self.model.active_group, node, path.clone(), mode);
            } else {
                self.model
                    .open_node(self.model.active_group, node, path.clone(), mode);
            }
            self.follow_local_graph_center(&path);
            if is_markdown {
                let text = document_text(&self.vault, node);
                self.active_editor().update(cx, |editor, cx| {
                    editor.set_workspace_document(path, text, mode, cx)
                });
                if mode.is_editing() {
                    self.focus_editor(window, cx);
                } else {
                    self.refresh_reading(cx);
                }
            }
            cx.notify();
            self.publish_test_state();
            self.emit_workspace_state(cx);
        }
    }

    fn render_drive_preview_notice(
        &self,
        group_id: u64,
        node: NodeId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let kind = self
            .vault
            .node(node)
            .and_then(|node| match &node.kind {
                NodeKind::File(meta) => Some(meta.kind),
                NodeKind::Folder => None,
            })
            .unwrap_or(tft_formats::FileKind::Other);
        if kind == tft_formats::FileKind::Image {
            if let Some(url) = self.media_previews.get(&node) {
                return div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .child(
                        img(SharedUri::from(url.clone()))
                            .max_w_full()
                            .max_h_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .into_any_element();
            }
        }
        if kind == tft_formats::FileKind::Audio {
            if let Some(url) = self.media_previews.get(&node) {
                let player_id = format!("tft-web-audio-{}", node.0);
                let state = serde_json::from_str::<serde_json::Value>(&tft_drive_web::audio_state(
                    &player_id,
                ))
                .unwrap_or_default();
                let playing = state
                    .get("playing")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let muted = state
                    .get("muted")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let position = state
                    .get("position")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                let duration = state
                    .get("duration")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                let error = state
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .filter(|error| !error.is_empty())
                    .map(str::to_owned);
                let fraction = if duration > 0.0 {
                    (position / duration).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let audio_url = url.clone();
                let play_player_id = player_id.clone();
                let control = div()
                    .id(format!("web-audio-toggle-{}", node.0))
                    .w(px(40.))
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(20.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.tokens.background_modifier_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let _ = tft_drive_web::toggle_audio(&play_player_id, &audio_url);
                        this.start_audio_tick(node, cx);
                        cx.notify();
                    }))
                    .child(tft_app::assets::icon(
                        if playing { "pause" } else { "play" },
                        18.,
                        theme.tokens.text_normal,
                    ));
                let audio_url = url.clone();
                let mute_player_id = player_id.clone();
                let mute = div()
                    .id(format!("web-audio-mute-{}", node.0))
                    .w(px(32.))
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(16.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.tokens.background_modifier_hover))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = tft_drive_web::toggle_audio_mute(&mute_player_id, &audio_url);
                        cx.notify();
                    }))
                    .child(tft_app::assets::icon(
                        if muted { "volume-x" } else { "volume-2" },
                        16.,
                        theme.tokens.text_normal,
                    ));
                let track_bounds: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>> =
                    std::rc::Rc::new(std::cell::Cell::new(None));
                let seek_to = {
                    let player_id = player_id.clone();
                    let bounds = track_bounds.clone();
                    move |x: Pixels| {
                        if duration <= 0.0 {
                            return;
                        }
                        if let Some(bounds) = bounds.get() {
                            let width = f32::from(bounds.size.width);
                            if width > 0.0 {
                                let fraction =
                                    (f32::from(x - bounds.origin.x) / width).clamp(0.0, 1.0);
                                tft_drive_web::seek_audio(
                                    &player_id,
                                    duration * f64::from(fraction),
                                );
                            }
                        }
                    }
                };
                let seek_down = seek_to.clone();
                let seek_bar = div()
                    .id(format!("web-audio-seek-{}", node.0))
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        seek_down(event.position.x);
                    })
                    .on_mouse_move(move |event: &MouseMoveEvent, _, _| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            seek_to(event.position.x);
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(theme.tokens.text_muted.opacity(0.45))
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(gpui::relative(fraction as f32))
                                    .rounded(px(2.))
                                    .bg(theme.tokens.text_normal),
                            )
                            .child({
                                let stored_bounds = track_bounds.clone();
                                canvas(
                                    move |bounds, _, _| stored_bounds.set(Some(bounds)),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full()
                            }),
                    );
                return div()
                    .size_full()
                    .flex()
                    .items_start()
                    .justify_center()
                    .pt(px(24.))
                    .child(
                        div()
                            .w(px(420.))
                            .max_w_full()
                            .p(px(16.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme.tokens.background_modifier_border)
                            .bg(theme.tokens.background_secondary)
                            .child(control)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(theme.base_font_size))
                                            .text_color(theme.tokens.text_normal)
                                            .child("Audio"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(theme.base_font_size * 0.85))
                                            .text_color(theme.tokens.text_muted)
                                            .child(format_audio_time(position)),
                                    )
                                    .child(seek_bar)
                                    .children(error.map(|error| {
                                        div()
                                            .text_size(px(theme.base_font_size * 0.85))
                                            .text_color(theme.tokens.text_error)
                                            .child(error)
                                    })),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(78.))
                                    .text_right()
                                    .text_size(px(theme.base_font_size * 0.85))
                                    .text_color(theme.tokens.text_muted)
                                    .child(format_audio_time(duration)),
                            )
                            .child(mute),
                    )
                    .into_any_element();
            }
        }
        if kind == tft_formats::FileKind::Canvas {
            if let Some(scene) = self.canvas_scenes.get(&node) {
                let Some(minimap) = tft_app::canvas_core::minimap::Minimap::of(&scene.canvas)
                else {
                    return tft_app::ui::workspace_notice(
                        "Empty canvas",
                        "Add a text card to start building this board.",
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-empty-{group_id}"),
                            "Add text card",
                            None,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_canvas_text(node, cx);
                        })),
                        theme,
                    )
                    .into_any_element();
                };
                let scale_dims = (1000.0, 700.0);
                let (fit_scale, mut offset) = minimap.fit(scale_dims.0, scale_dims.1);
                let viewport = self
                    .canvas_viewports
                    .get(&node)
                    .copied()
                    .unwrap_or_default();
                let zoom = f64::from(viewport.zoom.clamp(0.25, 4.0));
                let (pan_x, pan_y) = viewport.pan;
                let center_x = scale_dims.0 * 0.5;
                let center_y = scale_dims.1 * 0.5;
                let scale = fit_scale * zoom;
                offset.x = center_x + (offset.x - center_x) * zoom + f64::from(pan_x);
                offset.y = center_y + (offset.y - center_y) * zoom + f64::from(pan_y);
                let mut canvas_layer = div()
                    .id(format!("web-canvas-board-{group_id}"))
                    .relative()
                    .w(px(scale_dims.0 as f32))
                    .h(px(scale_dims.1 as f32))
                    .bg(theme.tokens.background_primary)
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            this.move_canvas_pointer(event.position, cx);
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.finish_canvas_drag(cx)),
                    )
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                        let zoom_in = match event.delta {
                            gpui::ScrollDelta::Pixels(delta) => delta.y > px(0.),
                            gpui::ScrollDelta::Lines(delta) => delta.y > 0.0,
                        };
                        this.zoom_canvas(node, zoom_in, cx);
                        cx.stop_propagation();
                    }));
                let canvas_edges = minimap.edges.clone();
                let canvas_view = minimap.view;
                let canvas_edge_width = (minimap.edge_width * scale) as f32;
                let edge_color = theme.tokens.graph_line;
                let canvas_origin_prepaint = self.canvas_origin.clone();
                let canvas_origin = self.canvas_origin.clone();
                let edge_hit_scale = scale;
                canvas_layer = canvas_layer.child(
                    div()
                        .absolute()
                        .size_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                let (origin_x, origin_y) = canvas_origin.get();
                                let local_x = event.position.x.as_f32() - origin_x;
                                let local_y = event.position.y.as_f32() - origin_y;
                                let at = tft_app::canvas_core::Point::new(
                                    canvas_view.x + (local_x as f64 - offset.x) / edge_hit_scale,
                                    canvas_view.y + (local_y as f64 - offset.y) / edge_hit_scale,
                                );
                                if this.canvas_card_editor.is_some() {
                                    this.close_canvas_card_editor(window, cx);
                                    if this.canvas_saving.contains(&node) {
                                        return;
                                    }
                                }
                                if let Some(scene) = this.canvas_scenes.get_mut(&node) {
                                    if let Some(edge_ix) = tft_app::canvas_core::hit_edge(
                                        &scene.canvas,
                                        at,
                                        8.0 / edge_hit_scale,
                                    ) {
                                        let edge_id = scene.canvas.edges[edge_ix].id.clone();
                                        scene.selection =
                                            tft_app::canvas_core::Selection::only_edge(&edge_id);
                                        this.canvas_background_drag = None;
                                    } else {
                                        scene.selection.clear();
                                        this.canvas_background_drag = Some((node, event.position));
                                    }
                                }
                                cx.notify();
                            }),
                        )
                        .child(
                            canvas(
                                move |bounds, window, _| {
                                    canvas_origin_prepaint
                                        .set((bounds.origin.x.as_f32(), bounds.origin.y.as_f32()));
                                    window.insert_hitbox(bounds, HitboxBehavior::Normal)
                                },
                                move |bounds, _, window, _| {
                                    let to_screen =
                                        |x: f64, y: f64| {
                                            point(
                                                bounds.origin.x
                                                    + px((offset.x + (x - canvas_view.x) * scale)
                                                        as f32),
                                                bounds.origin.y
                                                    + px((offset.y + (y - canvas_view.y) * scale)
                                                        as f32),
                                            )
                                        };
                                    let mut path =
                                        PathBuilder::stroke(px(canvas_edge_width.max(1.0)));
                                    for edge in &canvas_edges {
                                        path.move_to(to_screen(edge.from.x, edge.from.y));
                                        path.cubic_bezier_to(
                                            to_screen(edge.to.x, edge.to.y),
                                            to_screen(edge.c1.x, edge.c1.y),
                                            to_screen(edge.c2.x, edge.c2.y),
                                        );
                                    }
                                    if let Ok(path) = path.build() {
                                        window.paint_path(path, edge_color);
                                    }
                                },
                            )
                            .absolute()
                            .size_full(),
                        ),
                );
                for canvas_node in &scene.canvas.nodes {
                    let rect = tft_app::canvas_core::Rect::of(canvas_node);
                    let left = offset.x + (rect.x - minimap.view.x) * scale;
                    let top = offset.y + (rect.y - minimap.view.y) * scale;
                    let width = rect.width * scale;
                    let height = rect.height * scale;
                    let id = canvas_node.id.clone();
                    let selected = scene.selection.nodes.contains(&id);
                    let resize_handles = if selected {
                        tft_app::canvas_core::Handle::ALL
                            .into_iter()
                            .map(|handle| {
                                let handle_position = handle.position(
                                    &tft_app::canvas_core::Rect::new(0.0, 0.0, width, height),
                                );
                                let resize_id = id.clone();
                                div()
                                    .id(format!(
                                        "web-canvas-resize-{group_id}-{resize_id}-{handle:?}"
                                    ))
                                    .absolute()
                                    .left(px((handle_position.x as f32 - 6.0).max(0.0)))
                                    .top(px((handle_position.y as f32 - 6.0).max(0.0)))
                                    .size(px(12.))
                                    .rounded(px(2.))
                                    .border_1()
                                    .border_color(theme.tokens.background_primary)
                                    .bg(theme.tokens.text_accent)
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, window, cx| {
                                                cx.stop_propagation();
                                                this.begin_canvas_resize(
                                                    node,
                                                    resize_id.clone(),
                                                    handle,
                                                    event.position,
                                                    scale,
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    let card_is_text = canvas_node.kind == tft_formats::canvas::NodeType::Text;
                    let card_id_for_click = id.clone();
                    let text = match canvas_node.kind {
                        tft_formats::canvas::NodeType::Text => {
                            canvas_node.text.clone().unwrap_or_default()
                        }
                        tft_formats::canvas::NodeType::File => canvas_node
                            .file
                            .as_deref()
                            .and_then(|file| file.rsplit('/').next())
                            .unwrap_or("File")
                            .to_owned(),
                        tft_formats::canvas::NodeType::Link => {
                            canvas_node.url.clone().unwrap_or_default()
                        }
                        tft_formats::canvas::NodeType::Group => canvas_node
                            .label
                            .clone()
                            .unwrap_or_else(|| "Group".to_owned()),
                    };
                    canvas_layer = canvas_layer.child(
                        div()
                            .id(format!("web-canvas-card-{group_id}-{id}"))
                            .absolute()
                            .left(px(left as f32))
                            .top(px(top as f32))
                            .w(px(width as f32))
                            .h(px(height as f32))
                            .px(px(10.))
                            .py(px(8.))
                            .overflow_hidden()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(if selected {
                                theme.tokens.text_accent
                            } else {
                                theme.tokens.background_modifier_border
                            })
                            .bg(theme.tokens.background_secondary.opacity(
                                if matches!(canvas_node.kind, tft_formats::canvas::NodeType::Group)
                                {
                                    0.45
                                } else {
                                    1.0
                                },
                            ))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    if event.click_count == 2 && card_is_text {
                                        this.edit_canvas_text(
                                            node,
                                            card_id_for_click.clone(),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.begin_canvas_drag(
                                            node,
                                            card_id_for_click.clone(),
                                            event.position,
                                            scale,
                                            event.modifiers.shift,
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                            )
                            .children(resize_handles)
                            .child(
                                div()
                                    .max_w_full()
                                    .max_h_full()
                                    .overflow_hidden()
                                    .text_size(px(14.))
                                    .text_color(theme.tokens.text_normal)
                                    .when_some(
                                        self.canvas_card_editor
                                            .as_ref()
                                            .filter(|editor| editor.file == node && editor.id == id)
                                            .map(|editor| editor.editor.clone()),
                                        |content, editor| content.child(editor),
                                    )
                                    .when(
                                        !self.canvas_card_editor.as_ref().is_some_and(|editor| {
                                            editor.file == node && editor.id == id
                                        }),
                                        |content| content.child(text),
                                    ),
                            ),
                    );
                }
                for edge in scene
                    .canvas
                    .edges
                    .iter()
                    .filter(|edge| edge.label.as_deref().is_some_and(|label| !label.is_empty()))
                {
                    let Some(path) = tft_app::canvas_core::resolved_edge_path(&scene.canvas, edge)
                    else {
                        continue;
                    };
                    let Some(label) = edge.label.as_deref() else {
                        continue;
                    };
                    let midpoint = path.midpoint();
                    let width = (label.chars().count() as f64 * 8.0 + 16.0).clamp(40.0, 260.0);
                    let left = offset.x + (midpoint.x - minimap.view.x) * scale - width * 0.5;
                    let top = offset.y + (midpoint.y - minimap.view.y) * scale - 12.0;
                    let edge_id = edge.id.clone();
                    canvas_layer = canvas_layer.child(
                        div()
                            .id(format!("web-canvas-edge-label-{group_id}-{edge_id}"))
                            .absolute()
                            .left(px(left.max(0.0) as f32))
                            .top(px(top.max(0.0) as f32))
                            .w(px(width as f32))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme.tokens.background_modifier_border)
                            .bg(theme.tokens.background_primary)
                            .text_size(px(12.))
                            .text_color(theme.tokens.text_normal)
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(scene) = this.canvas_scenes.get_mut(&node) {
                                        scene.selection =
                                            tft_app::canvas_core::Selection::only_edge(&edge_id);
                                    }
                                    cx.notify();
                                }),
                            )
                            .child(label.to_owned()),
                    );
                }
                let canvas_status = self.canvas_errors.get(&node).cloned().unwrap_or_else(|| {
                    if self.canvas_saving.contains(&node) {
                        "Saving canvas…".to_owned()
                    } else if scene.is_dirty() {
                        "Unsaved canvas changes".to_owned()
                    } else {
                        "Drag cards to move · resize from the selected card corner · changes save automatically".to_owned()
                    }
                });
                let selected_text_card = match scene.selection.nodes.as_slice() {
                    [id] if scene
                        .node(id)
                        .is_some_and(|node| node.kind == tft_formats::canvas::NodeType::Text) =>
                    {
                        Some(id.clone())
                    }
                    _ => None,
                };
                let selected_text_is_editing = selected_text_card.as_ref().is_some_and(|id| {
                    self.canvas_card_editor
                        .as_ref()
                        .is_some_and(|editor| editor.file == node && editor.id == *id)
                });
                let can_group_canvas_selection = scene.selection.nodes.len() > 1;
                let selected_canvas_edge =
                    scene.selection.nodes.is_empty() && scene.selection.edges.len() == 1;
                let selected_canvas_edge_has_label = selected_canvas_edge
                    && scene
                        .selection
                        .edges
                        .first()
                        .and_then(|id| scene.edge(id))
                        .and_then(|edge| edge.label.as_deref())
                        .is_some_and(|label| !label.is_empty());
                let canvas_color_swatches = std::iter::once((
                    tft_app::canvas_core::Color::Default,
                    theme.canvas.card_border,
                ))
                .chain((1..=6).filter_map(|preset| {
                    theme
                        .canvas
                        .preset(preset)
                        .map(|color| (tft_app::canvas_core::Color::Preset(preset), color))
                }))
                .collect::<Vec<_>>();
                let toolbar = div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-zoom-out-{group_id}"),
                            "Zoom out",
                            None,
                            theme,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.zoom_canvas(node, false, cx)),
                        ),
                    )
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-zoom-fit-{group_id}"),
                            "Fit",
                            None,
                            theme,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.reset_canvas_zoom(node, cx)),
                        ),
                    )
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-zoom-in-{group_id}"),
                            "Zoom in",
                            None,
                            theme,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.zoom_canvas(node, true, cx)),
                        ),
                    )
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-add-{group_id}"),
                            "Add text card",
                            None,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_canvas_text(node, cx);
                        })),
                    )
                    .when(selected_text_card.is_some(), |toolbar| {
                        let card_id = selected_text_card.clone().unwrap_or_default();
                        toolbar.child(
                            tft_app::ui::workspace_empty_action(
                                format!("web-canvas-edit-{group_id}"),
                                if selected_text_is_editing {
                                    "Finish editing"
                                } else {
                                    "Edit text"
                                },
                                None,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    if this.canvas_card_editor.as_ref().is_some_and(|editor| {
                                        editor.file == node && editor.id == card_id
                                    }) {
                                        this.close_canvas_card_editor(window, cx);
                                    } else {
                                        this.edit_canvas_text(node, card_id.clone(), window, cx);
                                    }
                                },
                            )),
                        )
                    })
                    .when(!scene.selection.is_empty(), |toolbar| {
                        let swatches = div().flex().items_center().gap(px(5.)).pl_2().children(
                            canvas_color_swatches.iter().enumerate().map(
                                |(index, (color, fill))| {
                                    let color = *color;
                                    let fill = *fill;
                                    div()
                                        .id(format!("web-canvas-color-{group_id}-{index}"))
                                        .size(px(18.))
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(fill)
                                        .border_2()
                                        .border_color(theme.tokens.background_primary)
                                        .hover(move |style| style.border_color(fill))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_canvas_selection_color(node, color, cx);
                                        }))
                                },
                            ),
                        );
                        toolbar.child(swatches)
                    })
                    .when(can_group_canvas_selection, |toolbar| {
                        toolbar.child(
                            tft_app::ui::workspace_empty_action(
                                format!("web-canvas-group-{group_id}"),
                                "Create group",
                                None,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.group_canvas_selection(node, cx);
                                },
                            )),
                        )
                    })
                    .when(selected_canvas_edge, |toolbar| {
                        let edge_id = scene.selection.edges.first().cloned().unwrap_or_default();
                        let label = scene
                            .edge(&edge_id)
                            .and_then(|edge| edge.label.clone())
                            .unwrap_or_default();
                        let label_action = if label.is_empty() {
                            "Add label"
                        } else {
                            "Edit label"
                        };
                        toolbar.child(
                            tft_app::ui::workspace_empty_action(
                                format!("web-canvas-edge-label-{group_id}"),
                                label_action,
                                None,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.emit(WebWorkspaceEvent::CanvasEdgeLabelRequested {
                                        file: node,
                                        edge_id: edge_id.clone(),
                                        label: label.clone(),
                                    });
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .when(selected_canvas_edge, |toolbar| {
                        toolbar.child(
                            tft_app::ui::workspace_empty_action(
                                format!("web-canvas-edge-direction-{group_id}"),
                                "Cycle line direction",
                                None,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.cycle_canvas_edge_direction(node, cx);
                                },
                            )),
                        )
                    })
                    .when(selected_canvas_edge_has_label, |toolbar| {
                        toolbar.child(
                            tft_app::ui::workspace_empty_action(
                                format!("web-canvas-edge-remove-label-{group_id}"),
                                "Remove label",
                                None,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.remove_canvas_edge_label(node, cx);
                                },
                            )),
                        )
                    })
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-undo-{group_id}"),
                            "Undo",
                            None,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.undo_canvas(node, cx);
                        })),
                    )
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-redo-{group_id}"),
                            "Redo",
                            None,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.redo_canvas(node, cx);
                        })),
                    )
                    .child(
                        tft_app::ui::workspace_empty_action(
                            format!("web-canvas-delete-{group_id}"),
                            "Delete selected",
                            None,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_canvas_selection(node, cx);
                        })),
                    );
                return div()
                    .id(format!("web-canvas-preview-{group_id}"))
                    .size_full()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .child(toolbar)
                    .child(
                        div()
                            .px_3()
                            .pb_2()
                            .text_size(px(theme.base_font_size * 0.85))
                            .text_color(if self.canvas_errors.contains_key(&node) {
                                theme.tokens.text_error
                            } else {
                                theme.tokens.text_muted
                            })
                            .child(canvas_status),
                    )
                    .child(canvas_layer)
                    .into_any_element();
            }
        }
        if kind == tft_formats::FileKind::Video {
            if let Some(url) = self.media_previews.get(&node) {
                let video_url = url.clone();
                let title = self
                    .vault
                    .path(node)
                    .and_then(|path| path.rsplit('/').next())
                    .unwrap_or("Video")
                    .to_owned();
                let player_title = title.clone();
                return div()
                    .size_full()
                    .flex()
                    .items_start()
                    .justify_center()
                    .pt(px(24.))
                    .child(
                        div()
                            .w(px(480.))
                            .max_w_full()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme.tokens.background_modifier_border)
                            .bg(theme.tokens.background_secondary)
                            .child(tft_app::assets::icon("film", 32., theme.tokens.text_muted))
                            .child(
                                div()
                                    .text_size(px(theme.base_font_size))
                                    .text_color(theme.tokens.text_normal)
                                    .child(title),
                            )
                            .child(
                                tft_app::ui::workspace_empty_action(
                                    format!("web-video-play-{group_id}"),
                                    "Play video",
                                    None,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, _| {
                                        tft_drive_web::open_video_player(&video_url, &player_title);
                                    },
                                )),
                            ),
                    )
                    .into_any_element();
            }
        }
        let label = match kind {
            tft_formats::FileKind::Canvas => "Canvas",
            tft_formats::FileKind::Pdf => "PDF",
            tft_formats::FileKind::Image => "Image",
            tft_formats::FileKind::Audio => "Audio",
            tft_formats::FileKind::Video => "Video",
            _ => "File",
        };
        tft_app::ui::workspace_notice(
            format!("{label} preview is available in Google Drive"),
            self.media_preview_errors
                .get(&node)
                .or_else(|| self.pdf_errors.get(&node))
                .or_else(|| self.canvas_errors.get(&node))
                .map(|error| format!("{label} preview could not load: {error}"))
                .unwrap_or_else(|| {
                    if matches!(
                        kind,
                        tft_formats::FileKind::Image
                            | tft_formats::FileKind::Audio
                            | tft_formats::FileKind::Video
                    ) && self.media_preview_loading.contains(&node)
                    {
                        format!("Loading {} preview…", label.to_lowercase())
                    } else if kind == tft_formats::FileKind::Pdf && self.pdf_loading.contains(&node)
                    {
                        "Loading PDF…".to_owned()
                    } else if kind == tft_formats::FileKind::Canvas
                        && self.canvas_loading.contains(&node)
                    {
                        "Loading canvas preview…".to_owned()
                    } else {
                        "This file stays in your workspace tab. Open its Drive preview to view it."
                            .to_owned()
                    }
                }),
            tft_app::ui::workspace_empty_action(
                format!("web-drive-preview-{group_id}"),
                "Open in Google Drive",
                None,
                theme,
            )
            .on_click(cx.listener(move |this, _, _, _| {
                if let Some(BackendObjectId::Drive(id)) = this.vault.backend(node) {
                    tft_drive_web::open_drive_file(id);
                }
            })),
            theme,
        )
        .into_any_element()
    }

    fn load_image_preview(&mut self, node: NodeId, cx: &mut Context<Self>) {
        if self.media_previews.contains_key(&node) || !self.media_preview_loading.insert(node) {
            return;
        }
        let Some(BackendObjectId::Drive(id)) = self.vault.backend(node) else {
            self.media_preview_loading.remove(&node);
            self.media_preview_errors
                .insert(node, "The file is not stored in Google Drive.".into());
            return;
        };
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = tft_drive_web::blob_url(&id).await;
            this.update(cx, |this, cx| {
                this.media_preview_loading.remove(&node);
                match result {
                    Ok(url) => {
                        this.media_previews.insert(node, url);
                        this.media_preview_errors.remove(&node);
                    }
                    Err(error) => {
                        this.media_preview_errors.insert(node, error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_audio_tick(&mut self, node: NodeId, cx: &mut Context<Self>) {
        if !self.audio_ticking.insert(node) {
            return;
        }
        let player_id = format!("tft-web-audio-{}", node.0);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let keep_ticking = this
                .update(cx, |this, cx| {
                    let state = serde_json::from_str::<serde_json::Value>(
                        &tft_drive_web::audio_state(&player_id),
                    )
                    .unwrap_or_default();
                    let playing = state
                        .get("playing")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    cx.notify();
                    if !playing {
                        this.audio_ticking.remove(&node);
                    }
                    playing
                })
                .unwrap_or(false);
            if !keep_ticking {
                break;
            }
        })
        .detach();
    }

    fn load_canvas(&mut self, node: NodeId, cx: &mut Context<Self>) {
        if self.canvas_scenes.contains_key(&node) || !self.canvas_loading.insert(node) {
            return;
        }
        let Some(BackendObjectId::Drive(id)) = self.vault.backend(node) else {
            self.canvas_loading.remove(&node);
            self.canvas_errors
                .insert(node, "The file is not stored in Google Drive.".into());
            return;
        };
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = async {
                let text = tft_drive_web::read_file(&id).await?;
                let canvas = tft_formats::canvas::Canvas::from_json(&text)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(tft_app::canvas_core::Scene::new(canvas))
            }
            .await;
            this.update(cx, |this, cx| {
                this.canvas_loading.remove(&node);
                match result {
                    Ok(scene) => {
                        this.canvas_scenes.insert(node, scene);
                        this.canvas_errors.remove(&node);
                    }
                    Err(error) => {
                        this.canvas_errors.insert(node, error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn begin_canvas_drag(
        &mut self,
        file: NodeId,
        node_id: String,
        position: Point<Pixels>,
        scale: f64,
        additive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if !additive
            && self
                .canvas_card_editor
                .as_ref()
                .is_some_and(|editor| editor.file == file && editor.id == node_id)
        {
            return;
        }
        if self
            .canvas_card_editor
            .as_ref()
            .is_some_and(|editor| additive || editor.file != file || editor.id != node_id)
        {
            self.close_canvas_card_editor(window, cx);
            if self.canvas_saving.contains(&file) {
                return;
            }
        }
        let Some(scene) = self.canvas_scenes.get_mut(&file) else {
            return;
        };
        if additive {
            scene.selection.toggle_node(&node_id);
            cx.notify();
            return;
        }
        if !scene.selection.has_node(&node_id) {
            scene.selection = tft_app::canvas_core::Selection::only_node(&node_id);
        }
        scene.begin();
        self.canvas_drag = Some(WebCanvasDrag {
            file,
            node_id,
            last: position,
            scale,
            resize_handle: None,
        });
        cx.notify();
    }

    fn zoom_canvas(&mut self, file: NodeId, zoom_in: bool, cx: &mut Context<Self>) {
        let viewport = self.canvas_viewports.entry(file).or_default();
        let next = if zoom_in {
            viewport.zoom * 1.2
        } else {
            viewport.zoom / 1.2
        };
        viewport.zoom = next.clamp(0.25, 4.0);
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn reset_canvas_zoom(&mut self, file: NodeId, cx: &mut Context<Self>) {
        self.canvas_viewports
            .insert(file, WebCanvasViewport::default());
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn begin_canvas_resize(
        &mut self,
        file: NodeId,
        node_id: String,
        handle: tft_app::canvas_core::Handle,
        position: Point<Pixels>,
        scale: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if self
            .canvas_card_editor
            .as_ref()
            .is_some_and(|editor| editor.file != file || editor.id != node_id)
        {
            self.close_canvas_card_editor(window, cx);
            if self.canvas_saving.contains(&file) {
                return;
            }
        }
        let Some(scene) = self.canvas_scenes.get_mut(&file) else {
            return;
        };
        scene.begin();
        scene.selection = tft_app::canvas_core::Selection::only_node(&node_id);
        self.canvas_drag = Some(WebCanvasDrag {
            file,
            node_id,
            last: position,
            scale,
            resize_handle: Some(handle),
        });
        cx.notify();
    }

    fn add_canvas_text(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            let center = tft_app::canvas_core::bounds(&scene.canvas.nodes)
                .map(|bounds| bounds.center())
                .unwrap_or_else(|| tft_app::canvas_core::Point::new(125.0, 30.0));
            scene.begin();
            scene.add_text_card(
                tft_app::canvas_core::Point::new(center.x - 125.0, center.y - 30.0),
                "New card",
            );
            scene.commit();
        }
        self.save_canvas(file, cx);
        cx.notify();
    }

    fn group_canvas_selection(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        let dirty = if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            if scene.selection.nodes.len() < 2 {
                return;
            }
            scene.begin();
            let _ = scene.group_selection(40.0);
            scene.commit();
            scene.is_dirty()
        } else {
            false
        };
        if dirty {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    fn cycle_canvas_edge_direction(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        let dirty = if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            if scene.selection.nodes.len() != 0 || scene.selection.edges.len() != 1 {
                return;
            }
            scene.begin();
            for id in scene.selection.edges.clone() {
                let Some(edge) = scene.edge(&id) else {
                    continue;
                };
                let ends = tft_app::canvas_core::edge_ends(edge);
                let (from_arrow, to_arrow) = match ends {
                    (false, true) => (true, true),
                    (true, true) => (false, false),
                    _ => (false, true),
                };
                scene.set_edge_ends(&id, from_arrow, to_arrow);
            }
            scene.commit();
            scene.is_dirty()
        } else {
            false
        };
        if dirty {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    fn remove_canvas_edge_label(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        let dirty = if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            if scene.selection.nodes.len() != 0 || scene.selection.edges.is_empty() {
                return;
            }
            scene.begin();
            for id in scene.selection.edges.clone() {
                scene.set_edge_label(&id, "");
            }
            scene.commit();
            scene.is_dirty()
        } else {
            false
        };
        if dirty {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    pub fn set_canvas_edge_label(
        &mut self,
        file: NodeId,
        edge_id: String,
        label: String,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        let dirty = if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            if scene.edge(&edge_id).is_none() {
                return;
            }
            scene.begin();
            scene.set_edge_label(&edge_id, label.trim());
            scene.commit();
            scene.is_dirty()
        } else {
            false
        };
        if dirty {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    fn set_canvas_selection_color(
        &mut self,
        file: NodeId,
        color: tft_app::canvas_core::Color,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        let editing = self
            .canvas_card_editor
            .as_ref()
            .is_some_and(|editor| editor.file == file);
        let dirty = if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            if !editing {
                scene.begin();
            }
            scene.set_selection_color(color);
            if !editing {
                scene.commit();
            }
            scene.is_dirty()
        } else {
            false
        };
        if dirty && !editing {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    fn edit_canvas_text(
        &mut self,
        file: NodeId,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if self
            .canvas_card_editor
            .as_ref()
            .is_some_and(|editor| editor.file == file && editor.id == id)
        {
            if let Some(editor) = &self.canvas_card_editor {
                let focus = editor.editor.read(cx).focus_handle_ref().clone();
                window.focus(&focus, cx);
            }
            return;
        }
        if self.canvas_card_editor.is_some() {
            self.close_canvas_card_editor(window, cx);
            if self.canvas_saving.contains(&file) {
                return;
            }
        }
        let Some(scene) = self.canvas_scenes.get(&file) else {
            return;
        };
        let Some(node) = scene
            .node(&id)
            .filter(|node| node.kind == tft_formats::canvas::NodeType::Text)
        else {
            return;
        };
        let text = node.text.clone().unwrap_or_default();
        if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            scene.selection = tft_app::canvas_core::Selection::only_node(&id);
            scene.begin();
        }
        let path: SharedString = self.vault.path(file).unwrap_or_default().into();
        let editor = cx.new(|cx| Editor::new(window, cx));
        editor.update(cx, |editor, cx| {
            editor.set_readable_line_length(false);
            editor.set_live_preview(true, cx);
            editor.set_text(path, text, cx);
        });
        let card_id = id.clone();
        let subscription = cx.subscribe(
            &editor,
            move |this, editor, event: &tft_app::editor::EditorEvent, cx| match event {
                tft_app::editor::EditorEvent::Edited => {
                    if let Some(scene) = this.canvas_scenes.get_mut(&file) {
                        scene.set_text(&card_id, &editor.read(cx).text());
                    }
                    cx.notify();
                }
                tft_app::editor::EditorEvent::OpenLink(target)
                | tft_app::editor::EditorEvent::OpenLinkInNewTab(target) => match target {
                    reading::LinkTarget::External(url) => cx.open_url(url),
                    _ => {
                        let new_tab =
                            matches!(event, tft_app::editor::EditorEvent::OpenLinkInNewTab(_));
                        this.pending_link_source = None;
                        this.pending_link = Some((target.clone(), new_tab));
                        cx.notify();
                    }
                },
                _ => {}
            },
        );
        self.canvas_card_editor = Some(WebCanvasCardEditor {
            file,
            id,
            editor,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_canvas_card_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.canvas_card_editor.take() else {
            return;
        };
        if let Some(scene) = self.canvas_scenes.get_mut(&card.file) {
            let text = card.editor.read(cx).text().to_string();
            if text.trim().is_empty() {
                let selection = std::mem::take(&mut scene.selection);
                scene.selection = tft_app::canvas_core::Selection::only_node(&card.id);
                scene.delete_selection();
                scene.selection = selection;
                scene.selection.retain_present(&scene.canvas);
            } else {
                scene.set_text(&card.id, &text);
            }
            scene.commit();
            if scene.is_dirty() {
                self.save_canvas(card.file, cx);
            }
        }
        if card.editor.read(cx).focus_handle_ref().is_focused(window) {
            window.focus(&self.tree_focus, cx);
        }
        cx.notify();
    }

    fn undo_canvas(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if self
            .canvas_scenes
            .get_mut(&file)
            .is_some_and(tft_app::canvas_core::Scene::undo)
        {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    fn redo_canvas(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if self
            .canvas_scenes
            .get_mut(&file)
            .is_some_and(tft_app::canvas_core::Scene::redo)
        {
            self.save_canvas(file, cx);
        }
        cx.notify();
    }

    fn delete_canvas_selection(&mut self, file: NodeId, cx: &mut Context<Self>) {
        if self.canvas_saving.contains(&file) {
            return;
        }
        if let Some(scene) = self.canvas_scenes.get_mut(&file) {
            if !scene.selection.nodes.is_empty() || !scene.selection.edges.is_empty() {
                scene.begin();
                scene.delete_selection();
                scene.commit();
                self.save_canvas(file, cx);
            }
        }
        cx.notify();
    }

    fn move_canvas_pointer(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some((file, last)) = self.canvas_background_drag {
            let dx = (position.x - last.x).as_f32();
            let dy = (position.y - last.y).as_f32();
            if dx.abs() > 0.01 || dy.abs() > 0.01 {
                let viewport = self.canvas_viewports.entry(file).or_default();
                viewport.pan.0 += dx;
                viewport.pan.1 += dy;
                self.canvas_background_drag = Some((file, position));
                cx.notify();
            }
            return;
        }
        self.move_canvas_drag(position, cx);
    }

    fn move_canvas_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.canvas_drag.as_mut() else {
            return;
        };
        let dx = f64::from(position.x - drag.last.x) / drag.scale.max(0.001);
        let dy = f64::from(position.y - drag.last.y) / drag.scale.max(0.001);
        if dx.abs() < 0.01 && dy.abs() < 0.01 {
            return;
        }
        if let Some(scene) = self.canvas_scenes.get_mut(&drag.file) {
            if let Some(handle) = drag.resize_handle {
                scene.resize_node(&drag.node_id, handle, dx, dy);
                drag.last = position;
                cx.notify();
            } else {
                scene.move_selection(dx, dy);
                drag.last = position;
                cx.notify();
            }
        }
    }

    fn finish_canvas_drag(&mut self, cx: &mut Context<Self>) {
        let was_panning = self.canvas_background_drag.take().is_some();
        let Some(drag) = self.canvas_drag.take() else {
            if was_panning {
                self.emit_workspace_state(cx);
                cx.notify();
            }
            return;
        };
        if let Some(scene) = self.canvas_scenes.get_mut(&drag.file) {
            scene.commit();
            if scene.is_dirty() {
                self.save_canvas(drag.file, cx);
            }
        }
        cx.notify();
    }

    fn save_canvas(&mut self, node: NodeId, cx: &mut Context<Self>) {
        if !self.canvas_saving.insert(node) {
            return;
        }
        let Some(BackendObjectId::Drive(id)) = self.vault.backend(node) else {
            self.canvas_saving.remove(&node);
            self.canvas_errors
                .insert(node, "The canvas is not stored in Google Drive.".into());
            return;
        };
        let id = id.to_owned();
        let Some(scene) = self.canvas_scenes.get(&node) else {
            self.canvas_saving.remove(&node);
            return;
        };
        let contents = scene.to_json();
        let revision = self
            .vault
            .node(node)
            .and_then(|entry| entry.revision.clone());
        cx.spawn(async move |this, cx| {
            let result = tft_drive_web::write_file(&id, &contents, revision.as_deref()).await;
            this.update(cx, |this, cx| {
                this.canvas_saving.remove(&node);
                match result {
                    Ok(revision) => {
                        this.vault.set_revision(node, revision);
                        if let Some(scene) = this.canvas_scenes.get_mut(&node) {
                            scene.mark_saved();
                        }
                        this.canvas_errors.remove(&node);
                    }
                    Err(error) => {
                        this.canvas_errors.insert(node, error);
                    }
                }
                this.emit_workspace_state(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_pdf(&mut self, node: NodeId, cx: &mut Context<Self>) {
        let path = self.vault.path(node).unwrap_or_default().to_owned();
        let cache_key = PathBuf::from(format!("/tft-drive/{}.pdf", node.0));
        if self.pdf_loaded.contains(&node) {
            for (group_id, views) in &self.group_views {
                if self
                    .model
                    .group(*group_id)
                    .and_then(|group| group.active())
                    .is_some_and(|tab| tab.node == Some(node))
                {
                    views.pdf.update(cx, |pdf, cx| {
                        pdf.load(&path, cache_key.clone(), cx);
                    });
                }
            }
            return;
        }
        if !self.pdf_loading.insert(node) {
            return;
        }
        let Some(BackendObjectId::Drive(id)) = self.vault.backend(node) else {
            self.pdf_loading.remove(&node);
            self.pdf_errors
                .insert(node, "The file is not stored in Google Drive.".into());
            return;
        };
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = tft_drive_web::file_bytes(&id).await;
            this.update(cx, |this, cx| {
                this.pdf_loading.remove(&node);
                match result {
                    Ok(bytes) => {
                        this.pdf_loaded.insert(node);
                        this.pdf_errors.remove(&node);
                        let path = this.vault.path(node).unwrap_or_default().to_owned();
                        let cache_key = PathBuf::from(format!("/tft-drive/{}.pdf", node.0));
                        let views = this
                            .model
                            .groups
                            .iter()
                            .filter(|group| {
                                group.active().is_some_and(|tab| tab.node == Some(node))
                            })
                            .filter_map(|group| {
                                this.group_views
                                    .get(&group.id)
                                    .map(|views| views.pdf.clone())
                            })
                            .collect::<Vec<_>>();
                        for pdf in views {
                            let bytes = bytes.clone();
                            let path = path.clone();
                            let cache_key = cache_key.clone();
                            pdf.update(cx, |pdf, cx| {
                                pdf.load_bytes(&path, cache_key, bytes, cx);
                            });
                        }
                    }
                    Err(error) => {
                        this.pdf_errors.insert(node, error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn follow_link(
        &mut self,
        node: NodeId,
        path: String,
        offset: Option<usize>,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group_id = self.model.active_group;
        let active_tab = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .map(|tab| (tab.mode, tab.pinned, tab.path.clone(), tab.node));
        let open_in_new_tab = tft_app::workspace::linked_note_opens_new_tab(
            active_tab
                .as_ref()
                .map_or(ViewMode::default_editing(), |tab| tab.0),
            active_tab.as_ref().is_some_and(|tab| tab.1),
            active_tab.as_ref().and_then(|tab| tab.2.as_deref()),
            &path,
            new_tab,
        );
        if open_in_new_tab {
            self.open(node, path, true, window, cx);
        } else {
            let graph_previous_mode = active_tab.as_ref().and_then(|tab| {
                (tab.0 == ViewMode::Graph)
                    .then(|| self.graph_previous_modes.get(&group_id))
                    .flatten()
                    .filter(|(node, previous_path, _)| *node == tab.3 && previous_path == &tab.2)
                    .map(|(_, _, mode)| *mode)
            });
            let mode = ViewMode::markdown_document_mode(
                graph_previous_mode.or_else(|| active_tab.as_ref().map(|tab| tab.0)),
            );
            self.stash_group_view_state(group_id, cx);
            self.model.navigate_node(group_id, node, path.clone(), mode);
            self.graph_previous_modes.remove(&group_id);
            self.radial_tree_active.remove(&group_id);
            let text = document_text(&self.vault, node);
            self.active_editor().update(cx, |editor, cx| {
                editor.set_workspace_document(path, text, mode, cx)
            });
            if mode.is_editing() {
                self.focus_editor(window, cx);
            } else {
                self.refresh_reading(cx);
            }
        }
        if let Some(offset) = offset {
            self.active_editor().update(cx, |editor, cx| {
                editor.reveal_offset(offset, cx);
            });
            self.active_reading()
                .update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn follow_link_target(
        &mut self,
        target: reading::LinkTarget,
        new_tab: bool,
        source_override: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_path =
            source_override.unwrap_or_else(|| self.current_path().unwrap_or_default().to_owned());
        let resolver = vault_resolver(&self.vault);
        match tft_app::resolve_link_navigation(&resolver, &target, &source_path) {
            tft_app::LinkNavigation::Internal(resolved) => {
                if let Some(path) = resolved.path {
                    let offset = self.link_subpath_offset(&path, resolved.link.subpath.as_ref());
                    if let Ok(node) = self.vault.lookup(&path) {
                        self.follow_link(node, path, offset, new_tab, window, cx);
                    }
                } else if !resolved.link.path.is_empty()
                    && !matches!(
                        tft_formats::FileKind::from_path(resolved.link.path),
                        tft_formats::FileKind::Image
                            | tft_formats::FileKind::Audio
                            | tft_formats::FileKind::Video
                            | tft_formats::FileKind::Pdf
                    )
                {
                    let kind = tft_formats::FileKind::from_path(resolved.link.path);
                    let kind = if kind == tft_formats::FileKind::Canvas {
                        kind
                    } else {
                        tft_formats::FileKind::Markdown
                    };
                    let mut parent = self.new_note_parent_for(&source_path);
                    let mut name = resolved.link.path.to_owned();
                    let mut can_create = true;
                    if let Some((folder, file_name)) = name.rsplit_once('/') {
                        let is_folder = self.vault.lookup(folder).ok().is_some_and(|node| {
                            matches!(
                                self.vault.node(node).map(|entry| &entry.kind),
                                Some(NodeKind::Folder)
                            )
                        });
                        if !is_folder {
                            can_create = false;
                        } else {
                            parent = folder.to_owned();
                            name = file_name.to_owned();
                        }
                    }
                    if can_create {
                        if new_tab {
                            self.new_tab(cx);
                        }
                        cx.emit(WebWorkspaceEvent::CreateFileFromLink { parent, name, kind });
                    }
                }
            }
            tft_app::LinkNavigation::External(url) => cx.open_url(url),
            tft_app::LinkNavigation::Tag(tag) => {
                self.show_search(&format!("tag:#{tag}"), window, cx)
            }
            tft_app::LinkNavigation::Footnote(number) => {
                let offset = self
                    .active_reading()
                    .read(cx)
                    .model()
                    .footnotes
                    .get(number.saturating_sub(1))
                    .copied();
                if let Some(offset) = offset {
                    let reading_mode = self
                        .model
                        .active()
                        .and_then(|group| group.active())
                        .is_some_and(|tab| tab.mode == ViewMode::Reading);
                    if reading_mode {
                        self.active_reading()
                            .update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
                    } else {
                        self.active_editor()
                            .update(cx, |editor, cx| editor.reveal_offset(offset, cx));
                    }
                }
            }
        }
    }

    fn link_subpath_offset(
        &self,
        path: &str,
        subpath: Option<&tft_markdown::Subpath<'_>>,
    ) -> Option<usize> {
        let document = self
            .search_documents
            .iter()
            .find(|document| document.path == path)?;
        tft_app::subpath_offset(&document.metadata, subpath?)
    }

    fn new_note_parent_for(&self, source_path: &str) -> String {
        match self.app_config.new_file_location.as_deref() {
            Some("current") => source_path
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_owned())
                .unwrap_or_default(),
            Some("folder") => {
                let path = self
                    .app_config
                    .new_file_folder_path
                    .as_deref()
                    .unwrap_or_default()
                    .trim_matches('/')
                    .replace('\\', "/");
                if self.vault.lookup(&path).ok().is_some_and(|node| {
                    matches!(
                        self.vault.node(node).map(|entry| &entry.kind),
                        Some(NodeKind::Folder)
                    )
                }) {
                    path
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    }

    fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.model.active() else {
            return;
        };
        if index >= group.tabs.len() || index == group.active_tab {
            return;
        }
        let group_id = self.model.active_group;
        self.stash_group_view_state(group_id, cx);
        self.model.activate_tab(group_id, index);
        let selected = self
            .model
            .active()
            .and_then(|group| group.active())
            .map(|tab| (tab.node, tab.path.clone(), tab.mode));
        if let Some((node, selected_path, mode)) = selected {
            if let Some(node) = node {
                match self.vault.node(node).map(|node| &node.kind) {
                    Some(NodeKind::File(meta))
                        if matches!(
                            meta.kind,
                            tft_formats::FileKind::Image
                                | tft_formats::FileKind::Audio
                                | tft_formats::FileKind::Video
                        ) =>
                    {
                        self.load_image_preview(node, cx);
                    }
                    Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Pdf => {
                        self.load_pdf(node, cx);
                    }
                    Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Canvas => {
                        self.load_canvas(node, cx);
                    }
                    _ => {}
                }
            }
            let path = selected_path.unwrap_or_default();
            let text = node.map_or_else(String::new, |node| document_text(&self.vault, node));
            self.active_editor().update(cx, |editor, cx| {
                editor.set_workspace_document(path, text, mode, cx)
            });
        }
        if self
            .model
            .active()
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode == ViewMode::Reading)
        {
            self.refresh_reading(cx);
            let offset = self
                .model
                .active()
                .and_then(|group| group.active())
                .and_then(|tab| tab.history.get(tab.history_ix.saturating_sub(1)))
                .map_or(0, |entry| entry.offset);
            self.active_reading()
                .update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
        }
        if self
            .model
            .active()
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode != ViewMode::Reading)
        {
            let offset = self
                .model
                .active()
                .and_then(|group| group.active())
                .and_then(|tab| tab.history.get(tab.history_ix.saturating_sub(1)))
                .map_or(0, |entry| entry.offset);
            self.active_editor()
                .update(cx, |editor, cx| editor.reveal_offset(offset, cx));
            self.focus_editor(window, cx);
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn reorder_tab(
        &mut self,
        group_id: u64,
        source: usize,
        boundary: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_drop_boundary = None;
        self.activate_group(group_id, cx);
        self.stash_group_view_state(group_id, cx);
        let Some(group) = self.model.active_mut() else {
            return;
        };
        let Some(active) = tft_app::workspace::move_tab_to_boundary(
            &mut group.tabs,
            &mut group.active_tab,
            source,
            boundary,
        ) else {
            return;
        };
        if let Some((path, contents, mode, offset)) = self
            .model
            .active()
            .and_then(|group| group.tabs.get(active))
            .map(|tab| {
                (
                    tab.path.clone().unwrap_or_default(),
                    tab.node
                        .map_or_else(String::new, |node| document_text(&self.vault, node)),
                    tab.mode,
                    tab.history
                        .get(tab.history_ix.saturating_sub(1))
                        .map_or(0, |entry| entry.offset),
                )
            })
        {
            self.active_editor().update(cx, |editor, cx| {
                editor.set_workspace_document(path, contents, mode, cx)
            });
            if mode == ViewMode::Reading {
                self.refresh_reading(cx);
                self.active_reading()
                    .update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
            } else {
                self.active_editor()
                    .update(cx, |editor, cx| editor.reveal_offset(offset, cx));
                self.focus_editor(window, cx);
            }
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn store_group_editor_text(&mut self, group_id: u64, cx: &App) {
        let Some((node, contents)) = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .and_then(|tab| {
                let node = tab.node?;
                let editor = &self.group_views.get(&group_id)?.editor;
                Some((node, editor.read(cx).file_contents()))
            })
        else {
            return;
        };
        self.vault.set_text(node, contents.clone());
        if let Some(path) = self.vault.path(node).map(str::to_owned) {
            self.upsert_search_document(node, path, contents);
        }
    }

    fn stash_group_view_state(&mut self, group_id: u64, cx: &App) {
        let Some((node, path, mode)) = self.model.group(group_id).and_then(|group| {
            group
                .active()
                .map(|tab| (tab.node, tab.path.clone(), tab.mode))
        }) else {
            return;
        };
        self.store_group_editor_text(group_id, cx);
        let offset = self.document_view_offset(group_id, mode, cx);
        if let Some(tab) = self
            .model
            .group_mut(group_id)
            .and_then(|group| group.active_mut())
        {
            if let Some(path) = path.as_deref() {
                tft_app::workspace::update_history_entry_state(
                    &mut tab.history,
                    tab.history_ix,
                    path,
                    mode,
                    offset,
                );
            } else if let (Some(node), Some(path)) = (node, path) {
                tab.open_node(node, path, mode, offset);
            }
        }
    }

    fn sync_group_folds(&mut self, group_id: u64, to_editor: bool, cx: &mut Context<Self>) {
        let Some(views) = self.group_views.get(&group_id) else {
            return;
        };
        if to_editor {
            let (keys, properties) = {
                let reader = views.reading.read(cx);
                (reader.fold_keys(), reader.properties_folded())
            };
            views.editor.update(cx, |editor, cx| {
                editor.set_fold_keys(keys, cx);
                editor.set_properties_folded(properties, cx);
            });
        } else {
            let (keys, properties) = {
                let editor = views.editor.read(cx);
                (editor.fold_keys(), editor.properties_folded())
            };
            views.reading.update(cx, |reader, cx| {
                reader.set_fold_keys(keys, cx);
                reader.set_properties_folded(properties, cx);
            });
        }
    }

    fn sync_group_document(&mut self, group_id: u64, cx: &mut Context<Self>) {
        let Some((node, path, mode)) = self.model.group(group_id).and_then(|group| {
            group
                .active()
                .map(|tab| (tab.node, tab.path.clone(), tab.mode))
        }) else {
            return;
        };
        let path = path.unwrap_or_default();
        let contents = node.map_or_else(String::new, |node| document_text(&self.vault, node));
        let Some(views) = self.group_views.get(&group_id) else {
            return;
        };
        views.editor.update(cx, |editor, cx| {
            editor.set_workspace_document(path.clone(), contents.clone(), mode, cx)
        });
        if mode == ViewMode::Reading {
            if let Some(node) = node {
                let source = VaultEmbedSource::new(&self.vault);
                let model = reading::build_with(
                    &path,
                    &contents,
                    &source,
                    reading::BuildOptions::default(),
                );
                views
                    .reading
                    .update(cx, |reader, cx| reader.set_note(Some(path), model, cx));
                let _ = node;
            }
        }
    }

    fn transfer_tab_between_groups(
        &mut self,
        source_group: u64,
        source_index: usize,
        target_group: u64,
        boundary: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_drop_boundary = None;
        if source_group == target_group {
            return;
        }
        self.stash_group_view_state(source_group, cx);
        self.stash_group_view_state(target_group, cx);
        let Some(outcome) =
            self.model
                .transfer_tab(source_group, source_index, target_group, boundary)
        else {
            return;
        };
        if outcome.source_empty {
            self.group_views.remove(&source_group);
            self.group_subscriptions.remove(&source_group);
        } else {
            self.sync_group_document(source_group, cx);
        }
        self.sync_group_document(target_group, cx);
        if let Some(views) = self.group_views.get(&target_group) {
            let focus = views.editor.read(cx).focus_handle_ref().clone();
            window.focus(&focus, cx);
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn set_left_dock_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.model.dock.set_visible(visible);
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn toggle_left_dock(&mut self, cx: &mut Context<Self>) {
        self.model.dock.toggle();
        self.narrow_dock_open = self.model.dock.visible;
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let group_id = self.model.active_group;
        self.stash_group_view_state(group_id, cx);
        let closed = self
            .model
            .group(group_id)
            .and_then(|group| group.tabs.get(index))
            .cloned();
        if !self.model.close(self.model.active_group, index) {
            return;
        }
        if let Some(tab) = closed {
            let restorable = tab.path.is_some()
                || matches!(
                    tab.mode,
                    ViewMode::Graph | ViewMode::Search | ViewMode::Terminal
                );
            self.closed_tab_history.remember(index, tab, restorable);
        }
        if let Some(tab) = self.model.active().and_then(|group| group.active()) {
            let path = tab.path.clone().unwrap_or_default();
            let text = tab
                .node
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            self.active_editor().update(cx, |editor, cx| {
                editor.set_workspace_document(path, text, tab.mode, cx)
            });
            if tab.mode == ViewMode::Reading {
                self.refresh_reading(cx);
            } else {
                self.focus_editor(window, cx);
            }
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn duplicate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let group_id = self.model.active_group;
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        self.store_editor_text(active_node, cx);
        let Some(group) = self.model.group_mut(group_id) else {
            return;
        };
        let Some(tab) = group.tabs.get(index).cloned() else {
            return;
        };
        let duplicate_index = index + 1;
        group.tabs.insert(duplicate_index, tab);
        group.active_tab = duplicate_index;
        self.sync_group_document(group_id, cx);
        if self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode.is_editing())
        {
            if let Some(views) = self.group_views.get(&group_id) {
                let focus = views.editor.read(cx).focus_handle_ref().clone();
                window.focus(&focus, cx);
            }
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn toggle_tab_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .model
            .toggle_tab_pin(self.model.active_group, index)
            .is_some()
        {
            cx.notify();
            self.publish_test_state();
            self.emit_workspace_state(cx);
        }
    }

    fn close_tabs_except(
        &mut self,
        keep: impl Fn(usize, &tft_app::Tab) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group_id = self.model.active_group;
        let remove = self
            .model
            .active()
            .map(|group| {
                group
                    .tabs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, tab)| (!keep(index, tab)).then_some(index))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if remove.is_empty() {
            return;
        }
        self.stash_group_view_state(group_id, cx);
        let closed = self
            .model
            .group(group_id)
            .map(|group| {
                remove
                    .iter()
                    .filter_map(|&index| group.tabs.get(index).cloned().map(|tab| (index, tab)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (index, tab) in closed.into_iter().rev() {
            let restorable = tab.path.is_some()
                || matches!(
                    tab.mode,
                    ViewMode::Graph | ViewMode::Search | ViewMode::Terminal
                );
            self.closed_tab_history.remember(index, tab, restorable);
        }
        for index in remove.into_iter().rev() {
            self.model.close(group_id, index);
        }
        if let Some(tab) = self.model.active().and_then(|group| group.active()) {
            let path = tab.path.clone().unwrap_or_default();
            let text = tab
                .node
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            self.active_editor().update(cx, |editor, cx| {
                editor.set_workspace_document(path, text, tab.mode, cx)
            });
            if tab.mode == ViewMode::Reading {
                self.refresh_reading(cx);
            } else {
                self.focus_editor(window, cx);
            }
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn undo_close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, tab)) = self.closed_tab_history.take_last() else {
            return;
        };
        let group_id = self.model.active_group;
        self.stash_group_view_state(group_id, cx);
        let Some(group) = self.model.group_mut(group_id) else {
            return;
        };
        let index = index.min(group.tabs.len());
        group.tabs.insert(index, tab);
        group.active_tab = index;
        self.sync_group_document(group_id, cx);
        let (mode, offset) = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .map(|tab| {
                let offset = tab
                    .history
                    .get(tab.history_ix.saturating_sub(1))
                    .map_or(0, |entry| entry.offset);
                (tab.mode, offset)
            })
            .unwrap_or((ViewMode::default_editing(), 0));
        if mode.is_editing() {
            self.active_editor()
                .update(cx, |editor, cx| editor.reveal_offset(offset, cx));
            self.focus_editor(window, cx);
        } else if mode == ViewMode::Reading {
            self.active_reading()
                .update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.stash_group_view_state(self.model.active_group, cx);
        if self.model.new_tab(ViewMode::default_editing()).is_none() {
            return;
        }
        self.active_editor()
            .update(cx, |editor, cx| editor.set_text("", String::new(), cx));
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn split_active_group(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.model.active_group;
        let Some(_) = self.model.active().and_then(|group| group.active()) else {
            return;
        };
        self.stash_group_view_state(source, cx);
        let Some(group_id) = self.model.split_with_active_tab(source, direction, true) else {
            return;
        };
        let Some(tab) = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .cloned()
        else {
            return;
        };
        self.create_split_group_views(group_id, tab, window, cx);
    }

    fn create_split_group_views(
        &mut self,
        group_id: u64,
        tab: Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.new(|cx| Editor::new(window, cx));
        let reading = cx.new(ReadingView::new);
        let pdf = cx.new(PdfView::new);
        let path = tab.path.clone().unwrap_or_default();
        let text = tab
            .node
            .map_or_else(String::new, |node| document_text(&self.vault, node));
        editor.update(cx, |editor, cx| {
            editor.set_workspace_document(path.clone(), text.clone(), tab.mode, cx)
        });
        if tab.mode == ViewMode::Reading {
            if tab.node.is_some() {
                let source = VaultEmbedSource::new(&self.vault);
                let model =
                    reading::build_with(&path, &text, &source, reading::BuildOptions::default());
                reading.update(cx, |reader, cx| reader.set_note(Some(path), model, cx));
            }
        }
        self.register_group_views(group_id, editor.clone(), reading, pdf, cx);
        if let Some(node) = tab.node {
            if matches!(
                self.vault.node(node).map(|node| &node.kind),
                Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Pdf
            ) {
                self.load_pdf(node, cx);
            }
        }
        if tab.mode.is_editing() {
            let focus = editor.read(cx).focus_handle_ref().clone();
            window.focus(&focus, cx);
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn activate_group(&mut self, group_id: u64, cx: &mut Context<Self>) {
        if self.model.active_group == group_id {
            return;
        }
        self.stash_group_view_state(self.model.active_group, cx);
        if !self.model.activate_group(group_id) {
            return;
        }
        self.recent_group_order.retain(|group| *group != group_id);
        self.recent_group_order.insert(0, group_id);
        self.emit_workspace_state(cx);
        self.publish_test_state();
        cx.notify();
    }

    fn close_active_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let group_id = self.model.active_group;
        if !self.model.close_group(group_id) {
            return;
        }
        self.group_views.remove(&group_id);
        self.group_subscriptions.remove(&group_id);
        let focus = self.active_editor().read(cx).focus_handle_ref().clone();
        window.focus(&focus, cx);
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn render_title(
        &self,
        group_id: u64,
        path: Option<String>,
        title: SharedString,
        active: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editing = self
            .title_editing
            .editing
            .as_ref()
            .is_some_and(|edit| edit.group == group_id);
        let color = if active {
            theme.tokens.text_normal
        } else {
            theme.tokens.text_muted
        };
        let read_entity = cx.entity();
        let paint_entity = cx.entity();
        let read_focus = self.title_editing.focus.clone();
        let paint_focus = self.title_editing.focus.clone();
        let mut selection_color = theme.tokens.interactive_accent;
        selection_color.a = if theme.dark { 0.35 } else { 0.25 };
        let caret_color = theme.tokens.text_normal;
        let body = canvas(
            move |bounds, window, cx| {
                let workspace = read_entity.read(cx);
                let edit = workspace
                    .title_editing
                    .editing
                    .as_ref()
                    .filter(|edit| edit.group == group_id);
                let text: SharedString = edit
                    .map(|edit| edit.editing.field.text().to_owned())
                    .unwrap_or_else(|| title.clone().to_string())
                    .into();
                let scroll_x = workspace
                    .title_editing
                    .layouts
                    .get(&group_id)
                    .map(|layout| layout.scroll_x)
                    .unwrap_or(px(0.));
                tft_app::inline_field::shape(
                    text,
                    edit.map(|edit| &edit.editing),
                    color,
                    read_focus.is_focused(window),
                    bounds,
                    scroll_x,
                    window,
                )
            },
            move |bounds, prepaint, window, cx| {
                if editing {
                    window.handle_input(
                        &paint_focus,
                        ElementInputHandler::new(bounds, paint_entity.clone()),
                        cx,
                    );
                }
                let layout = tft_app::inline_field::paint(
                    prepaint,
                    bounds,
                    selection_color,
                    caret_color,
                    window,
                    cx,
                );
                paint_entity.update(cx, |workspace, _| {
                    workspace.title_editing.layouts.insert(group_id, layout);
                });
            },
        )
        .size_full();
        let mut host = div()
            .id(SharedString::from(format!("web-pane-title-{group_id}")))
            .flex_1()
            .min_w_0()
            .h(px(tft_app::theme::HEADER_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(theme.base_font_size))
            .text_color(color)
            .child(body);
        if let Some(path) = path {
            host = host.cursor_text().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |workspace, event: &MouseDownEvent, window, cx| {
                    workspace.title_clicked(group_id, path.clone(), event, window, cx)
                }),
            );
        }
        if !editing {
            return host.into_any_element();
        }
        let host = host
            .track_focus(&self.title_editing.focus)
            .key_context("TextInput");
        tft_app::inline_field::bind_keys(host, cx).into_any_element()
    }

    fn title_clicked(
        &mut self,
        group_id: u64,
        path: String,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.title_editing.layouts.get(&group_id).cloned() else {
            return;
        };
        let Some(node) = self.vault.lookup(&path).ok() else {
            return;
        };
        if !matches!(
            self.vault.node(node).map(|node| &node.kind),
            Some(NodeKind::File(_))
        ) {
            return;
        }
        self.activate_group(group_id, cx);
        let name = tft_app::workspace::workspace_leaf_title(Some(&path), ViewMode::Reading);
        let mut edit = match self.title_editing.editing.take() {
            Some(edit) if edit.group == group_id && edit.path == path => edit,
            _ => WebTitleEdit {
                group: group_id,
                path,
                editing: tft_app::inline_field::Editing::new(name),
            },
        };
        let Some(offset) = layout.offset_for(event.position, edit.editing.field.text()) else {
            self.title_editing.editing = None;
            return;
        };
        edit.editing
            .field
            .click(offset, event.click_count, event.modifiers.shift);
        self.title_editing.editing = Some(edit);
        let focus = self.title_editing.focus.clone();
        window.focus(&focus, cx);
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    fn commit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.title_editing.editing.take() else {
            return;
        };
        let value = edit.editing.field.text().trim().to_owned();
        if value.is_empty() || value.contains('/') || value.contains('\\') {
            self.title_editing.editing = Some(edit);
            cx.notify();
            return;
        }
        let (parent, file_name) = edit.path.rsplit_once('/').unwrap_or(("", &edit.path));
        let extension = file_name.rsplit_once('.').map(|(_, extension)| extension);
        let renamed = match extension {
            Some(extension) => format!("{value}.{extension}"),
            None => value,
        };
        let to = if parent.is_empty() {
            renamed
        } else {
            format!("{parent}/{renamed}")
        };
        if tft_formats::files::validate_relative_path(&to).is_err() {
            self.title_editing.editing = Some(edit);
            cx.notify();
            return;
        }
        if edit.path != to {
            if self.plan_drive_rename(&edit.path, &to).is_err() {
                self.title_editing.editing = Some(edit);
                cx.notify();
                return;
            }
            cx.emit(WebWorkspaceEvent::RenameFileRequested {
                from: edit.path,
                to,
            });
        }
        self.focus_editor(window, cx);
        cx.notify();
    }

    fn cancel_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_editing.editing.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    fn edit_active_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((group, path, node)) = self.model.active().and_then(|group| {
            group
                .active()
                .and_then(|tab| Some((group.id, tab.path.clone()?, tab.node?)))
        }) else {
            return;
        };
        if !matches!(
            self.vault.node(node).map(|node| &node.kind),
            Some(NodeKind::File(_))
        ) {
            return;
        }
        let name = tft_app::workspace::workspace_leaf_title(Some(&path), ViewMode::Reading);
        self.title_editing.editing = Some(WebTitleEdit {
            group,
            path,
            editing: tft_app::inline_field::Editing::selected(name),
        });
        let focus = self.title_editing.focus.clone();
        window.focus(&focus, cx);
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    fn render_group_chrome(
        &self,
        group_id: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(group) = self.model.group(group_id) else {
            return div().into_any_element();
        };
        let active_group = self.model.active_group == group_id;
        let active_index = group.active_tab;
        let active_tab = group.active();
        let mode = active_tab.map_or(ViewMode::default_editing(), |tab| tab.mode);
        let active_document_is_markdown =
            self.is_markdown_node(active_tab.and_then(|tab| tab.node));
        let title = tft_app::workspace::workspace_leaf_title(
            active_tab.and_then(|tab| tab.path.as_deref()),
            mode,
        );
        let can_back = active_tab.is_some_and(|tab| tab.history_ix > 1);
        let can_forward = active_tab.is_some_and(|tab| tab.history_ix < tab.history.len());
        let tab_count = group.tabs.len();
        let tab_drop_boundary = self.tab_drop_boundary;
        let breadcrumbs = tft_app::workspace::workspace_breadcrumbs(
            active_tab.and_then(|tab| tab.path.as_deref()),
        );
        let breadcrumb = tft_app::ui::workspace_breadcrumb_trail(
            breadcrumbs
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, (label, path))| {
                    tft_app::ui::workspace_breadcrumb_item(
                        SharedString::from(format!("web-pane-crumb-{group_id}-{index}")),
                        label,
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_group(group_id, cx);
                        this.tree.reveal(&path);
                        this.model.dock.show(LeftTab::Files);
                        this.set_left_dock_visible(true, cx);
                    }))
                    .into_any_element()
                })
                .collect::<Vec<_>>(),
            theme,
        );
        let tab_rows = group
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = index == active_index;
                let pinned = tab.pinned;
                let tab_title =
                    tft_app::workspace::workspace_leaf_title(tab.path.as_deref(), tab.mode);
                let divider = !active && index + 1 != active_index && index + 1 != tab_count;
                (index, tab.path.clone(), tab_title, active, pinned, divider)
            })
            .collect::<Vec<_>>();
        let mode_icon = tft_app::ui::workspace_document_mode_icon(mode);
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(
                tft_app::ui::workspace_tab_strip_shell(theme)
                    .when(!self.model.dock.visible && active_group, |strip| {
                        strip.child(
                            tft_app::ui::icon_button_with_id(
                                SharedString::from(format!("pane-sidebar-restore-{group_id}")),
                                SharedString::from(format!("pane-sidebar-restore-{group_id}")),
                                "panel-left",
                                theme,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_left_dock_visible(true, cx);
                            })),
                        )
                    })
                    .child(tft_app::ui::workspace_tab_strip_navigation(
                        tft_app::ui::icon_button_with_id(
                            SharedString::from(format!("pane-back-{group_id}")),
                            SharedString::from(format!("pane-back-{group_id}")),
                            "arrow-left",
                            theme,
                        )
                        .when(!can_back, |button| button.opacity(0.4))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.activate_group(group_id, cx);
                                this.navigate_history(false, window, cx);
                            },
                        )),
                        tft_app::ui::icon_button_with_id(
                            SharedString::from(format!("pane-forward-{group_id}")),
                            SharedString::from(format!("pane-forward-{group_id}")),
                            "arrow-right",
                            theme,
                        )
                        .when(!can_forward, |button| button.opacity(0.4))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.activate_group(group_id, cx);
                                this.navigate_history(true, window, cx);
                            },
                        )),
                        theme,
                    ))
                    .child(tft_app::ui::workspace_tab_strip_tabs(
                        ("pane-tabs", group_id),
                        tab_rows.into_iter().map(
                            |(index, tab_path, tab_title, active, pinned, divider)| {
                                let preview_workspace = cx.weak_entity();
                                let preview_path = tab_path.clone();
                                let tab = tft_app::ui::workspace_tab(
                                    SharedString::from(format!("pane-tab-{group_id}-{index}")),
                                    active,
                                    active_group,
                                    theme,
                                );
                                tft_app::ui::hover_row(
                                    tab,
                                    move |over, anchor, modifiers, _, app| {
                                        let Some(workspace) = preview_workspace.upgrade() else {
                                            return;
                                        };
                                        let Some(path) = preview_path.clone() else {
                                            return;
                                        };
                                        workspace.update(app, |this, cx| {
                                            let node = this.vault.lookup(&path).ok();
                                            this.file_tree_row_hover(
                                                node,
                                                path,
                                                over,
                                                anchor,
                                                modifiers,
                                                tft_app::page_preview::Source::TabHeader,
                                                cx,
                                            );
                                        });
                                    },
                                )
                                .when(divider, |tab| {
                                    tab.child(gpui::deferred(tft_app::ui::workspace_tab_divider(
                                        theme,
                                    )))
                                })
                                .when(tab_drop_boundary == Some(index), |tab| {
                                    tab.child(gpui::deferred(
                                        tft_app::ui::workspace_tab_drop_indicator(false, theme),
                                    ))
                                })
                                .cursor_pointer()
                                .on_drag(
                                    TabReorderDrag {
                                        source: tft_app::TabDragSource {
                                            group_id,
                                            tab_index: index,
                                        },
                                        title: tab_title.clone().into(),
                                    },
                                    |drag, _, _, cx| {
                                        let title = drag.title.clone();
                                        cx.new(|_| TabDragPreview { title })
                                    },
                                )
                                .on_drag_move(cx.listener(
                                    move |this, event: &DragMoveEvent<TabReorderDrag>, _, cx| {
                                        let bounds = [(
                                            index,
                                            f32::from(event.bounds.left()),
                                            f32::from(event.bounds.right()),
                                        )];
                                        let boundary =
                                            tft_app::workspace::nearest_tab_drop_boundary(
                                                f32::from(event.event.position.x),
                                                &bounds,
                                            )
                                            .map(|(boundary, _)| boundary)
                                            .unwrap_or(index);
                                        if this.tab_drop_boundary != Some(boundary) {
                                            this.tab_drop_boundary = Some(boundary);
                                            cx.notify();
                                        }
                                    },
                                ))
                                .on_drop(cx.listener(
                                    move |this, drag: &TabReorderDrag, window, cx| {
                                        let boundary =
                                            this.tab_drop_boundary.take().unwrap_or(index);
                                        if drag.source.group_id == group_id {
                                            this.reorder_tab(
                                                group_id,
                                                drag.source.tab_index,
                                                boundary,
                                                window,
                                                cx,
                                            );
                                        } else {
                                            this.transfer_tab_between_groups(
                                                drag.source.group_id,
                                                drag.source.tab_index,
                                                group_id,
                                                boundary,
                                                window,
                                                cx,
                                            );
                                        }
                                    },
                                ))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.activate_group(group_id, cx);
                                    this.activate_tab(index, window, cx);
                                }))
                                .on_mouse_down(
                                    MouseButton::Middle,
                                    cx.listener(move |this, _, window, cx| {
                                        this.activate_group(group_id, cx);
                                        this.close_tab(index, window, cx);
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.activate_group(group_id, cx);
                                        this.file_tree_context_menu = None;
                                        this.tab_context_menu = Some(TabContextMenu {
                                            position: event.position,
                                            group: group_id,
                                            index,
                                        });
                                        cx.notify();
                                    }),
                                )
                                .child(tft_app::ui::workspace_tab_content(
                                    tab_title,
                                    tft_app::ui::workspace_tab_close_button(
                                        SharedString::from(format!(
                                            "pane-tab-close-{group_id}-{index}"
                                        )),
                                        active,
                                        pinned,
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.activate_group(group_id, cx);
                                            if pinned {
                                                this.activate_tab(index, window, cx);
                                                this.toggle_tab_pin(index, cx);
                                            } else {
                                                this.close_tab(index, window, cx);
                                            }
                                        },
                                    )),
                                    active,
                                    theme,
                                ))
                                .into_any_element()
                            },
                        ),
                        tft_app::ui::workspace_new_tab_button(("pane-new-tab", group_id), theme)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_group(group_id, cx);
                                this.new_tab(cx);
                            })),
                        None,
                    )),
            )
            .child(tft_app::ui::workspace_document_header(
                tft_app::ui::workspace_view_header_title()
                    .text_ellipsis()
                    .when(!breadcrumbs.is_empty(), |header| {
                        header
                            .child(breadcrumb)
                            .child(tft_app::ui::workspace_breadcrumb_separator(theme))
                    })
                    .child(self.render_title(
                        group_id,
                        active_tab.and_then(|tab| tab.path.clone()),
                        title.into(),
                        active_group,
                        theme,
                        cx,
                    )),
                tft_app::ui::workspace_view_header_tools(("pane-tools", group_id))
                    .when(mode != ViewMode::Terminal, |tools| {
                        tools.child(
                            tft_app::ui::workspace_graph_engine_button(
                                SharedString::from(format!("pane-graph-{group_id}")),
                                SharedString::from(format!("pane-graph-{group_id}")),
                                false,
                                mode == ViewMode::Graph
                                    && !self.radial_tree_active.contains(&group_id),
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.toggle_graph_view(group_id, window, cx);
                                },
                            )),
                        )
                    })
                    .when(mode != ViewMode::Terminal, |tools| {
                        tools.child(
                            tft_app::ui::workspace_graph_engine_button(
                                SharedString::from(format!("pane-radial-tree-{group_id}")),
                                SharedString::from(format!("pane-radial-tree-{group_id}")),
                                true,
                                mode == ViewMode::Graph
                                    && self.radial_tree_active.contains(&group_id),
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.show_radial_tree(group_id, window, cx);
                                },
                            )),
                        )
                    })
                    .when(
                        tft_app::workspace::workspace_can_split(self.model.groups.len()),
                        |tools| {
                            tools
                                .child(
                                    tft_app::ui::icon_button_with_id(
                                        SharedString::from(format!("pane-split-right-{group_id}")),
                                        SharedString::from(format!("pane-split-right-{group_id}")),
                                        "separator-vertical",
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.activate_group(group_id, cx);
                                            this.split_active_group(
                                                SplitDirection::Vertical,
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                                .child(
                                    tft_app::ui::icon_button_with_id(
                                        SharedString::from(format!("pane-split-down-{group_id}")),
                                        SharedString::from(format!("pane-split-down-{group_id}")),
                                        "separator-horizontal",
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.activate_group(group_id, cx);
                                            this.split_active_group(
                                                SplitDirection::Horizontal,
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                        },
                    )
                    .when(self.model.groups.len() > 1, |tools| {
                        tools.child(
                            tft_app::ui::icon_button_with_id(
                                SharedString::from(format!("pane-close-{group_id}")),
                                SharedString::from(format!("pane-close-{group_id}")),
                                "x",
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.activate_group(group_id, cx);
                                    this.close_active_group(window, cx);
                                },
                            )),
                        )
                    })
                    .when(
                        tft_app::workspace::workspace_header_shows_mode_switch(
                            active_tab.and_then(|tab| tab.path.as_deref()),
                            mode,
                        ),
                        |tools| {
                            tools.child(
                                tft_app::ui::icon_button_with_id(
                                    SharedString::from(format!("pane-mode-{group_id}")),
                                    SharedString::from(format!("pane-mode-{group_id}")),
                                    mode_icon,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.activate_group(group_id, cx);
                                        this.toggle_view(window, cx);
                                    },
                                )),
                            )
                        },
                    )
                    .when(
                        self.drive_controls && active_document_is_markdown,
                        |tools| {
                            tools.child(
                                tft_app::ui::header_button_with_id(
                                    SharedString::from(format!("pane-save-{group_id}")),
                                    SharedString::from(format!("pane-save-{group_id}")),
                                    "check",
                                    "Save to Drive",
                                    false,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.activate_group(group_id, cx);
                                        cx.emit(WebWorkspaceEvent::SaveRequested);
                                    },
                                )),
                            )
                        },
                    ),
                theme,
            ))
            .into_any_element()
    }

    fn render_layout_content(
        &self,
        layout: &Layout,
        path: &mut Vec<usize>,
        mod_key: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match layout {
            Layout::Group(group_id) => {
                let id = *group_id;
                let Some(group) = self.model.group(*group_id) else {
                    return div().into_any_element();
                };
                let active = *group_id == self.model.active_group;
                let content = if let Some(tab) = group.active() {
                    if let Some(node) = tab.node.filter(|node| {
                        !matches!(
                            self.vault.node(*node).map(|node| &node.kind),
                            Some(NodeKind::File(meta))
                                if meta.kind == tft_formats::FileKind::Markdown
                        )
                    }) {
                        let is_pdf = matches!(
                            self.vault.node(node).map(|entry| &entry.kind),
                            Some(NodeKind::File(meta))
                                if meta.kind == tft_formats::FileKind::Pdf
                        );
                        if is_pdf && self.pdf_loaded.contains(&node) {
                            self.group_views
                                .get(group_id)
                                .map(|views| views.pdf.clone().into_any_element())
                                .unwrap_or_else(|| div().into_any_element())
                        } else {
                            self.render_drive_preview_notice(*group_id, node, theme, cx)
                        }
                    } else {
                        match tab.mode {
                            ViewMode::Graph if self.radial_tree_active.contains(group_id) => self
                                .radial_trees
                                .get(group_id)
                                .map(|tree| tree.view.clone().into_any_element())
                                .unwrap_or_else(|| {
                                    self.render_graph_view(*group_id, theme.clone(), cx)
                                }),
                            ViewMode::Graph => self.render_graph_view(*group_id, theme.clone(), cx),
                            ViewMode::Search => tft_app::ui::workspace_search_view(
                                format!("web-search-pane-{group_id}"),
                                self.search_input.clone(),
                                self.render_search_controls(theme, cx),
                                self.render_search_results(
                                    format!("web-search-pane-results-{group_id}"),
                                    Some(*group_id),
                                    theme,
                                    cx,
                                ),
                                theme,
                            )
                            .capture_action(cx.listener(Self::capture_search_left))
                            .capture_action(cx.listener(Self::capture_search_right))
                            .into_any_element(),
                            ViewMode::Terminal => tft_app::ui::workspace_notice(
                                "Terminal is available in Desktop",
                                "Open a note in this pane to continue working on Web.",
                                tft_app::ui::workspace_empty_action(
                                    format!("web-terminal-open-note-{group_id}"),
                                    "Go to file",
                                    Some(SharedString::from(format!("{mod_key} + P"))),
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        if let Some(tab) =
                                            this.model.group_mut(id).and_then(|g| g.active_mut())
                                        {
                                            tab.mode = ViewMode::default_editing();
                                        }
                                        this.open_quick_switcher(window, cx);
                                    },
                                )),
                                theme,
                            )
                            .into_any_element(),
                            _ if tab.path.is_none()
                                && !matches!(
                                    tab.mode,
                                    ViewMode::Graph | ViewMode::Search | ViewMode::Terminal
                                ) =>
                            {
                                let actions = vec![
                                    tft_app::ui::workspace_empty_action(
                                        format!("web-empty-new-note-{group_id}"),
                                        "New note",
                                        Some(SharedString::from(format!("{mod_key} + N"))),
                                        theme,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.activate_group(id, cx);
                                        cx.emit(WebWorkspaceEvent::FileOperationRequested(
                                            FileOperationRequest::CreateNote {
                                                parent: String::new(),
                                            },
                                        ));
                                    }))
                                    .into_any_element(),
                                    tft_app::ui::workspace_empty_action(
                                        format!("web-empty-new-canvas-{group_id}"),
                                        "New canvas",
                                        None,
                                        theme,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.activate_group(id, cx);
                                        cx.emit(WebWorkspaceEvent::CreateCanvasRequested {
                                            parent: String::new(),
                                        });
                                    }))
                                    .into_any_element(),
                                    tft_app::ui::workspace_empty_action(
                                        format!("web-empty-go-to-file-{group_id}"),
                                        "Go to file",
                                        Some(SharedString::from(format!("{mod_key} + P"))),
                                        theme,
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        this.open_quick_switcher(window, cx);
                                    }))
                                    .into_any_element(),
                                ];
                                tft_app::ui::workspace_empty_tab(theme)
                                    .child(tft_app::ui::workspace_empty_action_list(actions))
                                    .into_any_element()
                            }
                            ViewMode::Reading => self
                                .group_views
                                .get(group_id)
                                .map(|views| views.reading.clone().into_any_element())
                                .unwrap_or_else(|| div().into_any_element()),
                            _ => self
                                .group_views
                                .get(group_id)
                                .map(|views| views.editor.clone().into_any_element())
                                .unwrap_or_else(|| div().into_any_element()),
                        }
                    }
                } else {
                    div().into_any_element()
                };
                let content = div()
                    .id(format!("web-group-content-{group_id}"))
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .when(!active, |pane| {
                        pane.capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
                            this.activate_group(id, cx);
                        }))
                    })
                    .child(content)
                    .into_any_element();
                if self.model.groups.len() > 1 {
                    div()
                        .id(format!("web-group-pane-{group_id}"))
                        .flex()
                        .flex_col()
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.render_group_chrome(*group_id, theme, cx))
                        .child(content)
                        .into_any_element()
                } else {
                    content
                }
            }
            Layout::Split {
                direction,
                children,
                node,
            } => {
                let id = SharedString::from(format!("web-layout-split-{path:?}"));
                let split_path = path.clone();
                let saved_dimensions = node
                    .get("dimensions")
                    .and_then(serde_json::Value::as_array)
                    .filter(|values| values.len() == children.len())
                    .map(|values| {
                        values
                            .iter()
                            .map(|v| v.as_f64().unwrap_or(1.0).max(0.01))
                            .collect::<Vec<_>>()
                    });
                let drag_path = split_path.clone();
                let child_count = children.len();
                let split_direction = *direction;
                let mut container = tft_app::ui::workspace_split_container(id, *direction)
                    .on_drag_move(cx.listener(
                        move |this, event: &DragMoveEvent<SplitResizeDrag>, _, cx| {
                            let drag = event.drag(cx).clone();
                            if drag.path != drag_path || drag.before + 1 >= child_count {
                                return;
                            }
                            let horizontal = split_direction == SplitDirection::Vertical;
                            let (origin, size, position) = if horizontal {
                                (
                                    event.bounds.origin.x,
                                    event.bounds.size.width,
                                    event.event.position.x,
                                )
                            } else {
                                (
                                    event.bounds.origin.y,
                                    event.bounds.size.height,
                                    event.event.position.y,
                                )
                            };
                            let Some(relative) = tft_app::normalized_split_position(
                                f32::from(origin) as f64,
                                f32::from(size) as f64,
                                f32::from(position) as f64,
                            ) else {
                                return;
                            };
                            // The model owns the normalized child sizes; the
                            // drag payload supplies only pointer geometry.
                            if !this
                                .model
                                .resize_split_at(&drag.path, drag.before, relative)
                            {
                                return;
                            }
                            this.split_dragging = Some((drag.path, drag.before));
                            cx.notify();
                        },
                    ))
                    .on_drop(cx.listener(|this, _: &SplitResizeDrag, _, cx| {
                        this.split_dragging = None;
                        this.emit_workspace_state(cx);
                        cx.notify();
                    }));
                for (index, child) in children.iter().enumerate() {
                    path.push(index);
                    let content = self.render_layout_content(child, path, mod_key, theme, cx);
                    path.pop();
                    let grow = saved_dimensions
                        .as_ref()
                        .map(|values| values[index] as f32)
                        .unwrap_or(1.0);
                    let mut cell = tft_app::ui::workspace_split_cell(*direction, index > 0, theme)
                        .flex_basis(px(0.))
                        .child(content);
                    cell.style().flex_grow = Some(grow.max(0.01));
                    if index > 0 {
                        let before = index - 1;
                        let drag = SplitResizeDrag {
                            path: split_path.clone(),
                            before,
                        };
                        let dragging = self
                            .split_dragging
                            .as_ref()
                            .is_some_and(|(p, i)| p == &split_path && *i == before);
                        let accent = theme.tokens.interactive_accent;
                        let handle = div()
                            .id(("web-split-divider", index as u64))
                            .absolute()
                            .when(*direction == SplitDirection::Vertical, |d| {
                                d.left(px(-2.))
                                    .top_0()
                                    .h_full()
                                    .w(px(5.))
                                    .cursor_col_resize()
                            })
                            .when(*direction == SplitDirection::Horizontal, |d| {
                                d.top(px(-2.))
                                    .left_0()
                                    .w_full()
                                    .h(px(5.))
                                    .cursor_row_resize()
                            })
                            .when(dragging, |d| d.bg(accent))
                            .hover(|d| d.bg(accent))
                            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragPreview));
                        cell = cell.child(handle);
                    }
                    container = container.child(cell);
                }
                container.into_any_element()
            }
        }
    }

    fn store_editor_text(&mut self, node: Option<NodeId>, cx: &App) {
        if let Some(node) = node {
            let contents = self.active_editor().read(cx).file_contents();
            self.vault.set_text(node, contents.clone());
            if let Some(path) = self.vault.path(node).map(str::to_owned) {
                self.upsert_search_document(node, path, contents);
            }
        }
    }

    /// Captures the position owned by the visible document view. Reading mode
    /// has no active caret, so its history position comes from the reader's
    /// top block; editing modes use the start of the shared Editor selection,
    /// matching Desktop's history convention.
    fn document_view_offset(&self, group_id: u64, mode: ViewMode, cx: &App) -> usize {
        let Some(views) = self.group_views.get(&group_id) else {
            return 0;
        };
        let mode_before_graph = self
            .graph_previous_modes
            .get(&group_id)
            .map(|(_, _, mode)| *mode);
        tft_app::document_history_state(
            mode,
            mode_before_graph,
            views.reading.read(cx).top_offset(),
            views.editor.read(cx).selection().start,
        )
        .1
    }

    fn upsert_search_document(&mut self, node: NodeId, path: String, contents: String) {
        let contents = Arc::new(contents);
        let metadata = tft_cache::index_markdown(&contents);
        let (mtime, ctime) = self
            .search_documents
            .iter()
            .find(|existing| existing.node == node)
            .map(|existing| (existing.mtime, existing.ctime))
            .unwrap_or_default();
        let document = SearchDocument {
            node,
            mtime,
            ctime,
            path,
            contents,
            metadata,
        };
        if let Some(existing) = self
            .search_documents
            .iter_mut()
            .find(|existing| existing.node == node)
        {
            *existing = document;
        } else {
            self.search_documents.push(document);
            self.search_documents
                .sort_by(|a, b| tft_search::natural_cmp(&a.path, &b.path));
        }
    }

    fn schedule_graph_rebuild(&mut self, cx: &mut Context<Self>, persist: bool) {
        self.graph_refresh_generation = self.graph_refresh_generation.wrapping_add(1);
        let generation = self.graph_refresh_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            this.update(cx, |this, cx| {
                if this.graph_refresh_generation != generation {
                    return;
                }
                let (data, layout) = build_web_graph(
                    &this.vault,
                    &this.search_documents,
                    &this.excluded,
                    &this.graph_config,
                    this.graph_local_center.as_deref(),
                    this.graph_local_options,
                );
                this.graph_data = Arc::new(data);
                this.graph_layout = Arc::new(Mutex::new(layout));
                this.graph_simulation_started.set(false);
                this.graph_hovered.set(None);
                this.graph_scale.set(1.0);
                this.graph_offset.set((0.0, 0.0));
                if persist {
                    this.emit_workspace_state(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_graph_flag(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let (slot, default) = match key {
            "showTags" => (&mut self.graph_config.show_tags, false),
            "showAttachments" => (&mut self.graph_config.show_attachments, false),
            "hideUnresolved" => (&mut self.graph_config.hide_unresolved, false),
            "showOrphans" => (&mut self.graph_config.show_orphans, true),
            "showArrow" => (&mut self.graph_config.show_arrow, false),
            _ => return,
        };
        *slot = Some(!slot.unwrap_or(default));
        if key != "showArrow" {
            self.rebuild_graph_data();
        }
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn rebuild_graph_data(&mut self) {
        let (data, layout) = build_web_graph(
            &self.vault,
            &self.search_documents,
            &self.excluded,
            &self.graph_config,
            self.graph_local_center.as_deref(),
            self.graph_local_options,
        );
        self.graph_data = Arc::new(data);
        self.graph_layout = Arc::new(Mutex::new(layout));
        self.graph_simulation_started.set(false);
        self.graph_hovered.set(None);
    }

    fn open_local_graph(&mut self, group_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .and_then(|tab| tab.path.clone())
            .filter(|path| self.vault.lookup(path).is_ok())
        else {
            return;
        };
        let graph_is_open = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode == ViewMode::Graph);
        self.graph_local_center = Some(path);
        self.rebuild_graph_data();
        if !graph_is_open {
            self.toggle_graph_view(group_id, window, cx);
        } else {
            self.radial_tree_active.remove(&group_id);
            self.activate_group(group_id, cx);
            self.publish_test_state();
            self.emit_workspace_state(cx);
            cx.notify();
        }
    }

    fn toggle_graph_local(&mut self, group_id: u64, cx: &mut Context<Self>) {
        if self.graph_local_center.is_some() {
            self.graph_local_center = None;
        } else {
            let Some(center) = self
                .model
                .group(group_id)
                .and_then(|group| group.active())
                .and_then(|tab| tab.path.clone())
                .filter(|path| self.vault.lookup(path).is_ok())
            else {
                return;
            };
            self.graph_local_center = Some(center);
        }
        self.rebuild_graph_data();
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn follow_local_graph_center(&mut self, path: &str) {
        if self.graph_local_center.is_some() && self.graph_local_center.as_deref() != Some(path) {
            self.graph_local_center = Some(path.to_owned());
            self.rebuild_graph_data();
        }
    }

    fn toggle_local_graph_option(&mut self, key: &'static str, cx: &mut Context<Self>) {
        match key {
            "localBacklinks" => {
                self.graph_local_options.backlinks = !self.graph_local_options.backlinks
            }
            "localForelinks" => {
                self.graph_local_options.forelinks = !self.graph_local_options.forelinks
            }
            "localInterlinks" => {
                self.graph_local_options.interlinks = !self.graph_local_options.interlinks
            }
            _ => return,
        }
        if self.graph_local_center.is_some() {
            self.rebuild_graph_data();
        }
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn add_graph_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.graph_config.color_groups.len();
        let rgb = WEB_GRAPH_GROUP_COLORS[index % WEB_GRAPH_GROUP_COLORS.len()];
        self.graph_config
            .color_groups
            .push(tft_formats::config::GraphColorGroup {
                query: String::new(),
                color: tft_formats::config::GraphColor { a: 1.0, rgb },
                extra: Default::default(),
            });
        self.edit_graph_group(index, window, cx);
        self.emit_workspace_state(cx);
    }

    fn edit_graph_group(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self
            .graph_config
            .color_groups
            .get(index)
            .map(|group| group.query.clone())
        else {
            return;
        };
        self.graph_editing_group = Some(index);
        self.graph_group_input.update(cx, |input, cx| {
            input.set_text(query, cx);
            input.select_all(cx);
        });
        window.focus(&self.graph_group_input.focus_handle(cx), cx);
        cx.notify();
    }

    fn commit_graph_group(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.graph_editing_group.take() else {
            return;
        };
        let query = self.graph_group_input.read(cx).text().to_string();
        let Some(group) = self.graph_config.color_groups.get_mut(index) else {
            return;
        };
        if group.query == query {
            self.emit_workspace_state(cx);
            cx.notify();
            return;
        }
        group.query = query;
        self.rebuild_graph_data();
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn cycle_graph_group_color(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(group) = self.graph_config.color_groups.get_mut(index) else {
            return;
        };
        let next = WEB_GRAPH_GROUP_COLORS
            .iter()
            .position(|color| *color == group.color.rgb)
            .map(|position| (position + 1) % WEB_GRAPH_GROUP_COLORS.len())
            .unwrap_or(0);
        group.color.rgb = WEB_GRAPH_GROUP_COLORS[next];
        self.rebuild_graph_data();
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn remove_graph_group(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.graph_config.color_groups.len() {
            return;
        }
        self.graph_config.color_groups.remove(index);
        self.graph_editing_group = None;
        self.rebuild_graph_data();
        self.emit_workspace_state(cx);
        cx.notify();
    }

    fn graph_setting_row(
        &self,
        label: &'static str,
        key: &'static str,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("web-graph-setting-{key}")))
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .cursor_pointer()
            .hover(|style| style.bg(theme.tokens.background_modifier_hover))
            .text_color(theme.tokens.text_normal)
            .child(label)
            .child(if enabled { "On" } else { "Off" })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_graph_flag(key, cx)))
    }

    fn graph_local_option_row(
        &self,
        label: &'static str,
        key: &'static str,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("web-graph-setting-{key}")))
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .cursor_pointer()
            .hover(|style| style.bg(theme.tokens.background_modifier_hover))
            .text_color(theme.tokens.text_normal)
            .child(label)
            .child(if enabled { "On" } else { "Off" })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_local_graph_option(key, cx)))
    }

    fn graph_slider_to(
        &mut self,
        slider: WebGraphSlider,
        bounds: Bounds<Pixels>,
        x: Pixels,
        cx: &mut Context<Self>,
    ) {
        let width = bounds.size.width.as_f32().max(1.0);
        let fraction = (x.as_f32() - bounds.left().as_f32()) / width;
        let value = slider.value_at(fraction);
        slider.set(&mut self.graph_config, &mut self.graph_local_options, value);
        if slider == WebGraphSlider::Depth {
            self.rebuild_graph_data();
        }
        if matches!(
            slider,
            WebGraphSlider::Center
                | WebGraphSlider::Repel
                | WebGraphSlider::Link
                | WebGraphSlider::LinkDistance
        ) {
            let mut layout = self
                .graph_layout
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let restart_simulation = layout.is_settled();
            let defaults = tft_app::graph_layout::Forces::default();
            layout.forces = tft_app::graph_layout::Forces {
                center: self.graph_config.center_strength.unwrap_or(defaults.center),
                repel: self.graph_config.repel_strength.unwrap_or(defaults.repel),
                link: self.graph_config.link_strength.unwrap_or(defaults.link),
                link_distance: self
                    .graph_config
                    .link_distance
                    .unwrap_or(defaults.link_distance),
            };
            layout.reheat();
            if restart_simulation {
                self.graph_simulation_started.set(false);
            }
        }
        cx.notify();
    }

    fn graph_slider_row(
        &self,
        slider: WebGraphSlider,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let value = slider.value(&self.graph_config, self.graph_local_options);
        let (min, max, _) = slider.range();
        let fraction = ((value - min) / (max - min)).clamp(0.0, 1.0);
        let entity = cx.entity();
        let track_color = theme.tokens.slider_track_background;
        let fill_color = theme.tokens.slider_fill_background;
        let knob_color = theme.tokens.slider_thumb_background;
        let track = canvas(
            move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, _cx| {
                let y = bounds.center().y;
                let line = Bounds::new(
                    point(bounds.left(), y - px(2.0)),
                    size(bounds.size.width, px(4.0)),
                );
                let filled = bounds.size.width * fraction;
                let mut background = fill(line, track_color);
                background.corner_radii = gpui::Corners::all(px(2.0));
                window.paint_quad(background);
                let mut progress =
                    fill(Bounds::new(line.origin, size(filled, px(4.0))), fill_color);
                progress.corner_radii = gpui::Corners::all(px(2.0));
                window.paint_quad(progress);
                let mut knob = fill(
                    Bounds::new(
                        point(bounds.left() + filled - px(6.0), y - px(6.0)),
                        size(px(12.0), px(12.0)),
                    ),
                    knob_color,
                );
                knob.corner_radii = gpui::Corners::all(px(6.0));
                window.paint_quad(knob);

                let down_entity = entity.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && hitbox.is_hovered(window)
                    {
                        cx.stop_propagation();
                        down_entity.update(cx, |this, cx| {
                            this.graph_slider_drag.set(Some(slider));
                            this.graph_slider_to(slider, bounds, event.position.x, cx);
                        });
                    }
                });
                let move_entity = entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && move_entity.read(cx).graph_slider_drag.get() == Some(slider)
                    {
                        move_entity.update(cx, |this, cx| {
                            this.graph_slider_to(slider, bounds, event.position.x, cx)
                        });
                    }
                });
                let up_entity = entity.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && up_entity.read(cx).graph_slider_drag.get() == Some(slider)
                    {
                        up_entity.update(cx, |this, cx| {
                            this.graph_slider_drag.set(None);
                            this.emit_workspace_state(cx);
                        });
                    }
                });
            },
        )
        .w(px(96.))
        .h(px(18.));
        let shown = if slider.range().2 >= 1.0 {
            format!("{value:.0}")
        } else {
            format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py(px(3.))
            .child(
                div()
                    .text_color(theme.tokens.text_normal)
                    .child(slider.label()),
            )
            .child(
                div().flex().items_center().gap_1().child(track).child(
                    div()
                        .w(px(30.))
                        .text_right()
                        .text_color(theme.tokens.text_muted)
                        .child(shown),
                ),
            )
    }

    pub fn current_document(&self, cx: &App) -> Option<(NodeId, String)> {
        let tab = self.model.active()?.active()?;
        Some((tab.node?, self.active_editor().read(cx).file_contents()))
    }

    pub fn can_create_note(&self, parent_path: &str, name: &str) -> Result<(), String> {
        let parent = self
            .vault
            .lookup(parent_path)
            .map_err(|_| format!("Folder not found: {parent_path}"))?;
        if !matches!(
            self.vault.node(parent).map(|node| &node.kind),
            Some(NodeKind::Folder)
        ) {
            return Err(format!("Not a folder: {parent_path}"));
        }
        self.vault
            .validate_new_child_name(parent, name)
            .map_err(|error| match error {
                tft_app::RenameError::InvalidName => format!("Invalid file name: {name}"),
                tft_app::RenameError::DestinationExists => {
                    format!("A file or folder named {name} already exists.")
                }
                _ => "The Vault root is unavailable.".into(),
            })
    }

    pub fn drive_folder_id(&self, path: &str) -> Option<String> {
        let node = self.vault.lookup(path).ok()?;
        if !matches!(
            self.vault.node(node).map(|node| &node.kind),
            Some(NodeKind::Folder)
        ) {
            return None;
        }
        match self.vault.backend(node)? {
            BackendObjectId::Drive(id) => Some(id.clone()),
            _ => None,
        }
    }

    pub fn move_destination_suggestions(&self, path: &str) -> Option<Vec<String>> {
        let node = self.vault.lookup(path).ok()?;
        if !matches!(
            self.vault.node(node).map(|node| &node.kind),
            Some(NodeKind::File(_))
        ) {
            return None;
        }
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        let own_prefix = format!("{path}/");
        let mut suggestions = vec!["/".to_owned()];
        suggestions.extend(self.vault.folders().into_iter().filter(|folder| {
            folder != path && !folder.starts_with(&own_prefix) && folder != parent
        }));
        if parent.is_empty() {
            suggestions.remove(0);
        }
        Some(suggestions)
    }

    pub fn drive_file_id(&self, path: &str) -> Option<String> {
        let node = self.vault.lookup(path).ok()?;
        match self.vault.backend(node)? {
            BackendObjectId::Drive(id) => Some(id.clone()),
            _ => None,
        }
    }

    pub fn always_update_links(&self) -> bool {
        self.app_config.always_update_links.unwrap_or(false)
    }

    pub fn plan_drive_rename(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<tft_cache::Rewrite>, String> {
        let snapshot = tft_app::vault::VaultSnapshot::new(self.vault.files(), self.vault.folders());
        snapshot
            .check_rename(from, to)
            .map_err(|error| format!("Cannot rename {from}: {error:?}"))?;
        let renames = vec![(from.to_owned(), to.to_owned())];
        let paths = self
            .vault
            .files()
            .into_iter()
            .map(|file| file.path)
            .collect::<Vec<_>>();
        let resolver = LinkResolver::new(paths);
        let cache = tft_cache::MetadataCache::new(
            resolver,
            self.search_documents
                .iter()
                .map(|document| (document.path.clone(), document.metadata.clone())),
        );
        let rewrites = cache.plan_rename_rewrites(
            &renames,
            tft_cache::LinkFormat::from_config(self.app_config.new_link_format.as_deref()),
            &|path| {
                self.vault
                    .lookup(path)
                    .ok()
                    .map(|node| document_text(&self.vault, node))
            },
        );
        Ok(rewrites)
    }

    pub fn plan_heading_rename(
        &self,
        path: &str,
        line: u32,
        name: &str,
        own_text: &str,
    ) -> Option<tft_cache::HeadingRename> {
        let cache = tft_cache::MetadataCache::new(
            vault_resolver(&self.vault),
            self.search_documents
                .iter()
                .map(|document| (document.path.clone(), document.metadata.clone())),
        );
        cache.plan_heading_rename(path, own_text, line, name, &|source| {
            self.vault
                .lookup(source)
                .ok()
                .map(|node| document_text(&self.vault, node))
        })
    }

    pub fn apply_heading_rename(
        &mut self,
        path: &str,
        plan: &tft_cache::HeadingRename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some((node, active_path)) = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| Some((tab.node?, tab.path.as_deref()?)))
        else {
            return Err("There is no active note to rename a heading in.".into());
        };
        if active_path != path {
            return Err("The active note changed before the heading rename was applied.".into());
        }
        let current_heading = self.active_editor().read(cx).heading_to_rename();
        if !current_heading.is_some_and(|heading| {
            heading.line_start == plan.heading.line_start && heading.span == plan.heading.span
        }) {
            return Err("The heading changed before the rename was applied.".into());
        }

        let own_contents = self.active_editor().update(cx, |editor, cx| {
            editor.apply_heading_rename(plan.own.clone(), &plan.heading, window, cx);
            editor.file_contents()
        });
        self.vault.set_text(node, own_contents.clone());
        let mut changed = vec![(node, path.to_owned(), own_contents)];
        for rewrite in &plan.rewrites {
            let Some(rewritten_node) = self.vault.lookup(&rewrite.old_path).ok() else {
                continue;
            };
            self.vault.set_text(rewritten_node, rewrite.text.clone());
            changed.push((
                rewritten_node,
                rewrite.old_path.clone(),
                rewrite.text.clone(),
            ));
        }
        for (changed_node, changed_path, contents) in &changed {
            self.upsert_search_document(*changed_node, changed_path.clone(), contents.clone());
        }

        let active_group = self.model.active_group;
        let changed_nodes = changed
            .iter()
            .map(|(node, _, _)| *node)
            .collect::<HashSet<_>>();
        let groups = self
            .model
            .groups
            .iter()
            .filter(|group| {
                group.id != active_group
                    && group
                        .active()
                        .and_then(|tab| tab.node)
                        .is_some_and(|node| changed_nodes.contains(&node))
            })
            .map(|group| group.id)
            .collect::<Vec<_>>();
        for group in groups {
            self.sync_group_document(group, cx);
        }
        for (changed_node, contents) in changed
            .into_iter()
            .map(|(node, _, contents)| (node, contents))
        {
            cx.emit(WebWorkspaceEvent::Edited {
                node: changed_node,
                contents,
            });
        }
        self.schedule_graph_rebuild(cx, false);
        self.rerun_search(cx);
        self.emit_workspace_state(cx);
        self.publish_test_state();
        cx.notify();
        Ok(())
    }

    pub fn apply_drive_rename(
        &mut self,
        from: &str,
        to: &str,
        rewrites: &[tft_cache::Rewrite],
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.apply_drive_path_change(from, to, None, rewrites, cx)
    }

    pub fn apply_drive_move(
        &mut self,
        from: &str,
        to: &str,
        destination: &str,
        rewrites: &[tft_cache::Rewrite],
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.apply_drive_path_change(from, to, Some(destination), rewrites, cx)
    }

    fn apply_drive_path_change(
        &mut self,
        from: &str,
        to: &str,
        destination: Option<&str>,
        rewrites: &[tft_cache::Rewrite],
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let node = self
            .vault
            .lookup(from)
            .map_err(|_| format!("The file is no longer in the Vault: {from}"))?;
        let parent = if let Some(destination) = destination {
            let parent = self.vault.lookup(destination).map_err(|_| {
                format!("The destination folder is no longer in the Vault: {destination}")
            })?;
            if !matches!(
                self.vault.node(parent).map(|node| &node.kind),
                Some(NodeKind::Folder)
            ) {
                return Err(format!("Not a destination folder: {destination}"));
            }
            parent
        } else {
            self.vault
                .node(node)
                .and_then(|node| node.parent)
                .ok_or_else(|| "The Vault root cannot be renamed as a file.".to_owned())?
        };
        let name = to.rsplit('/').next().unwrap_or(to).to_owned();
        self.vault
            .rename(node, parent, name)
            .map_err(|error| format!("Could not update the Vault index: {error:?}"))?;
        for rewrite in rewrites {
            let path = if rewrite.old_path == from {
                to
            } else {
                rewrite.old_path.as_str()
            };
            if let Ok(node) = self.vault.lookup(path) {
                self.vault.set_text(node, rewrite.text.clone());
            }
        }
        let renames = HashMap::from([(from.to_owned(), to.to_owned())]);
        for group in &mut self.model.groups {
            for tab in &mut group.tabs {
                if let Some(path) = tab.path.as_mut() {
                    if let Some(next) = renames.get(path) {
                        *path = next.clone();
                    }
                }
                for entry in &mut tab.history {
                    if let Some(next) = renames.get(&entry.path) {
                        entry.path = next.clone();
                    }
                }
            }
        }
        let rewritten_texts = rewrites
            .iter()
            .map(|rewrite| {
                (
                    if rewrite.old_path == from {
                        to.to_owned()
                    } else {
                        rewrite.old_path.clone()
                    },
                    rewrite.text.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        for (group_id, views) in &self.group_views {
            let Some(tab) = self.model.group(*group_id).and_then(|group| group.active()) else {
                continue;
            };
            if let Some(path) = tab.path.as_deref() {
                let Some(node) = tab.node else {
                    continue;
                };
                let mode = tab.mode;
                let text = rewritten_texts.get(path).cloned();
                if text.is_none() && path != to {
                    continue;
                }
                views.editor.update(cx, |editor, cx| {
                    if let Some(text) = text {
                        let offset = editor.selection().end;
                        editor.set_workspace_document(to.to_owned(), text, mode, cx);
                        editor.reveal_offset(offset, cx);
                    } else {
                        editor.set_name(path.to_owned());
                    }
                });
                let contents = rewritten_texts
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| document_text(&self.vault, node));
                let source = VaultEmbedSource::new(&self.vault);
                let model =
                    reading::build_with(path, &contents, &source, reading::BuildOptions::default());
                views.reading.update(cx, |reading, cx| {
                    reading.set_note(Some(path.to_owned()), model, cx);
                });
            }
        }
        let mut tree = FileTree::from_vault_sorted(&self.vault, self.tree.sort());
        tree.preserve_view_from(&self.tree);
        self.tree_selection.retain_in(&tree);
        self.tree = tree;
        self.tree.reveal(to);
        self.recent_files = self
            .recent_files
            .iter()
            .map(|path| renames.get(path).cloned().unwrap_or_else(|| path.clone()))
            .collect();
        self.search_documents = build_search_index(&self.vault);
        for rewrite in rewrites {
            if let Ok(node) = self.vault.lookup(if rewrite.old_path == from {
                to
            } else {
                &rewrite.old_path
            }) {
                cx.emit(WebWorkspaceEvent::Edited {
                    node,
                    contents: rewrite.text.clone(),
                });
            }
        }
        self.schedule_graph_rebuild(cx, true);
        self.emit_workspace_state(cx);
        self.publish_test_state();
        cx.notify();
        Ok(())
    }

    pub fn insert_drive_note(
        &mut self,
        id: String,
        name: String,
        path: String,
        contents: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<NodeId, String> {
        self.insert_drive_file(
            id,
            name,
            path,
            contents,
            tft_formats::FileKind::Markdown,
            window,
            cx,
        )
    }

    pub fn insert_drive_canvas(
        &mut self,
        id: String,
        name: String,
        path: String,
        contents: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<NodeId, String> {
        self.insert_drive_file(
            id,
            name,
            path,
            contents,
            tft_formats::FileKind::Canvas,
            window,
            cx,
        )
    }

    fn insert_drive_file(
        &mut self,
        id: String,
        name: String,
        path: String,
        contents: String,
        kind: tft_formats::FileKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<NodeId, String> {
        let (parent_path, _) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        self.can_create_note(parent_path, &name)?;
        let parent_id = self
            .drive_folder_id(parent_path)
            .ok_or_else(|| format!("The Drive folder has no identity: {parent_path}"))?;
        let parent = BackendObjectId::Drive(parent_id);
        let node = self.vault.upsert_object(
            BackendObjectId::Drive(id),
            Some(parent),
            name,
            NodeKind::File(FileMeta {
                kind,
                size: contents.len() as u64,
                mtime: 0,
                ctime: 0,
            }),
            None,
        );
        self.vault.rebuild_indexes();
        self.vault.set_text(node, contents);
        let mut tree = FileTree::from_vault(&self.vault);
        tree.preserve_view_from(&self.tree);
        self.tree_selection.retain_in(&tree);
        self.tree = tree;
        let path = self
            .vault
            .path(node)
            .ok_or_else(|| "The new note has no Vault path.".to_string())?
            .to_owned();
        if kind == tft_formats::FileKind::Markdown {
            self.upsert_search_document(node, path.clone(), document_text(&self.vault, node));
        }
        self.schedule_graph_rebuild(cx, false);
        self.open(node, path, false, window, cx);
        Ok(node)
    }

    pub fn insert_drive_copy(
        &mut self,
        id: String,
        name: String,
        path: String,
        kind: tft_formats::FileKind,
        size: u64,
        mtime: u64,
        ctime: u64,
        contents: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<NodeId, String> {
        let (parent_path, _) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        let parent_id = {
            let parent = self
                .vault
                .lookup(parent_path)
                .map_err(|_| format!("Folder not found: {parent_path}"))?;
            if !matches!(
                self.vault.node(parent).map(|node| &node.kind),
                Some(NodeKind::Folder)
            ) {
                return Err(format!("Not a folder: {parent_path}"));
            }
            let id = self
                .drive_folder_id(parent_path)
                .ok_or_else(|| format!("The Drive folder has no identity: {parent_path}"))?;
            id
        };
        self.can_create_note(parent_path, &name)?;
        let node = self.vault.upsert_object(
            BackendObjectId::Drive(id),
            Some(BackendObjectId::Drive(parent_id)),
            name,
            NodeKind::File(FileMeta {
                kind,
                size,
                mtime,
                ctime,
            }),
            None,
        );
        self.vault.rebuild_indexes();
        if let Some(contents) = contents {
            self.vault.set_text(node, contents.clone());
            if kind == tft_formats::FileKind::Markdown {
                let path = self.vault.path(node).unwrap_or_default().to_owned();
                self.upsert_search_document(node, path, contents);
                if let Some(document) = self
                    .search_documents
                    .iter_mut()
                    .find(|document| document.node == node)
                {
                    document.mtime = mtime;
                    document.ctime = ctime;
                }
            }
        }
        let mut tree = FileTree::from_vault(&self.vault);
        tree.preserve_view_from(&self.tree);
        self.tree_selection.retain_in(&tree);
        self.tree = tree;
        self.schedule_graph_rebuild(cx, false);
        self.emit_workspace_state(cx);
        self.publish_test_state();
        Ok(node)
    }

    pub fn remove_drive_file(
        &mut self,
        node: NodeId,
        path: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.vault.lookup(path).ok() != Some(node) {
            return Err(format!("The file is no longer in the Vault: {path}"));
        }
        self.store_group_editor_text(self.model.active_group, cx);
        let groups = self
            .model
            .groups
            .iter()
            .map(|group| group.id)
            .collect::<Vec<_>>();
        for group_id in &groups {
            let remove = self
                .model
                .group(*group_id)
                .map(|group| {
                    group
                        .tabs
                        .iter()
                        .enumerate()
                        .filter_map(|(index, tab)| {
                            (tab.node == Some(node) || tab.path.as_deref() == Some(path))
                                .then_some(index)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for index in remove.into_iter().rev() {
                self.model.close(*group_id, index);
            }
            if let Some(group) = self.model.group_mut(*group_id) {
                for tab in &mut group.tabs {
                    let current = tab.history.get(tab.history_ix.saturating_sub(1)).cloned();
                    tab.history.retain(|entry| entry.path != path);
                    if let Some(current) = current.filter(|entry| entry.path != path) {
                        tab.history_ix = tab
                            .history
                            .iter()
                            .rposition(|entry| entry.path == current.path)
                            .map_or(0, |index| index + 1);
                    }
                    if tab.history.is_empty() {
                        if let Some(current_path) = tab.path.clone() {
                            let mut entry = tft_app::HistoryEntry::new(current_path, tab.mode);
                            entry.node = tab.node;
                            tab.history.push(entry);
                            tab.history_ix = 1;
                        }
                    }
                }
            }
        }

        self.vault.remove(node);
        self.search_documents
            .retain(|document| document.node != node);
        self.recent_files.retain(|recent| recent != path);
        self.canvas_scenes.remove(&node);
        self.canvas_viewports.remove(&node);
        self.canvas_loading.remove(&node);
        self.canvas_errors.remove(&node);
        self.canvas_saving.remove(&node);
        self.media_previews.remove(&node);
        self.media_preview_loading.remove(&node);
        self.media_preview_errors.remove(&node);
        self.pdf_loading.remove(&node);
        self.pdf_loaded.remove(&node);
        self.pdf_errors.remove(&node);
        let mut tree = FileTree::from_vault(&self.vault);
        tree.preserve_view_from(&self.tree);
        self.tree_selection.retain_in(&tree);
        self.tree = tree;
        for group_id in &groups {
            self.sync_group_document(*group_id, cx);
        }
        self.schedule_graph_rebuild(cx, false);
        self.emit_workspace_state(cx);
        self.publish_test_state();
        cx.notify();
        Ok(())
    }

    pub fn insert_drive_folder(
        &mut self,
        id: String,
        name: String,
        path: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (parent_path, _) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        self.can_create_note(parent_path, &name)?;
        let parent_id = self
            .drive_folder_id(parent_path)
            .ok_or_else(|| format!("The Drive folder has no identity: {parent_path}"))?;
        self.vault.upsert_object(
            BackendObjectId::Drive(id),
            Some(BackendObjectId::Drive(parent_id)),
            name,
            NodeKind::Folder,
            None,
        );
        self.vault.rebuild_indexes();
        let mut tree = FileTree::from_vault(&self.vault);
        tree.preserve_view_from(&self.tree);
        tree.reveal(&path);
        self.tree_selection.retain_in(&tree);
        self.tree = tree;
        self.tree_focused_key = self
            .tree
            .visible_index(&path)
            .and_then(|index| self.tree.row(index).map(|row| row.key.clone()));
        self.emit_workspace_state(cx);
        self.publish_test_state();
        cx.notify();
        Ok(())
    }

    pub fn current_path(&self) -> Option<&str> {
        self.model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.path.as_deref())
    }

    /// The Drive object id behind the active tab, for the actions that hand
    /// the document to Google Drive (open in Drive, open with the default app).
    fn active_drive_id(&self) -> Option<String> {
        let node = self.model.active()?.active()?.node?;
        match self.vault.backend(node)? {
            BackendObjectId::Drive(id) => Some(id.clone()),
            BackendObjectId::Local(_) => None,
        }
    }

    fn emit_workspace_state(&self, cx: &mut Context<Self>) {
        let Some(group) = self.model.active() else {
            return;
        };
        let editor_offsets = self
            .model
            .groups
            .iter()
            .filter_map(|group| {
                let tab = group.active()?;
                Some((group.id, self.document_view_offset(group.id, tab.mode, cx)))
            })
            .collect::<HashMap<_, _>>();
        let tabs_value = |group: &TabGroup| {
            group
                .tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| {
                    let offset = (index == group.active_tab)
                        .then(|| editor_offsets.get(&group.id).copied())
                        .flatten();
                    web_tab_value(tab, offset)
                })
                .collect::<Vec<_>>()
        };
        let canvas_viewports = self
            .canvas_viewports
            .iter()
            .filter_map(|(node, viewport)| {
                let path = self.vault.path(*node)?;
                Some((
                    path.to_owned(),
                    serde_json::json!({ "zoom": viewport.zoom, "pan": viewport.pan }),
                ))
            })
            .collect::<serde_json::Map<_, _>>();
        let state = serde_json::json!({
            "version": 2,
            "tabs": tabs_value(group),
            "activeTab": group.active_tab,
            "activeGroup": self.model.active_group,
            "groups": self.model.groups.iter().map(|group| serde_json::json!({
                "id": group.id,
                "activeTab": group.active_tab,
                "tabs": tabs_value(group),
            })).collect::<Vec<_>>(),
            "layout": web_layout_value(&self.model.layout),
            "leftVisible": self.model.dock.visible,
            "dockWidth": self.dock_width,
            "leftTab": match self.model.dock.tab { LeftTab::Files => "files", LeftTab::Search => "search" },
            "sortOrder": self.tree.sort().as_str(),
            "recentFiles": self.recent_files,
            "expandedFolders": self.tree.expanded_folders(),
            "graphConfig": &self.graph_config,
            "graphPanelOpen": self.graph_panel_open,
            "graphLocalCenter": self.graph_local_center,
            "graphLocalOptions": {
                "jumps": self.graph_local_options.jumps,
                "backlinks": self.graph_local_options.backlinks,
                "forelinks": self.graph_local_options.forelinks,
                "interlinks": self.graph_local_options.interlinks,
            },
            "canvasViewports": canvas_viewports,
        });
        cx.emit(WebWorkspaceEvent::WorkspaceStateChanged(state.to_string()));
    }

    pub fn set_vault_name(&mut self, name: String, cx: &mut Context<Self>) {
        if self.vault_name != name {
            self.vault_name = name;
            cx.notify();
        }
    }

    pub fn set_drive_controls(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.drive_controls != enabled {
            self.drive_controls = enabled;
            cx.notify();
        }
    }

    pub fn mark_saved(&mut self, node: NodeId, contents: String, cx: &mut Context<Self>) {
        if !self
            .vault
            .text(node)
            .is_some_and(|latest| latest.as_str() == contents)
        {
            return;
        }
        self.vault.set_text(node, contents.clone());
        let editors = self
            .model
            .groups
            .iter()
            .filter(|group| group.active().and_then(|tab| tab.node) == Some(node))
            .filter_map(|group| self.group_views.get(&group.id))
            .map(|views| views.editor.clone())
            .collect::<Vec<_>>();
        for editor in editors {
            editor.update(cx, |editor, cx| {
                if editor.matches_file(&contents) {
                    editor.mark_saved(&contents);
                }
                cx.notify();
            });
        }
        for editing in self.preview_editors.values() {
            editing.editor.update(cx, |editor, cx| {
                if editor.matches_file(&contents) {
                    editor.mark_saved(&contents);
                }
                cx.notify();
            });
        }
    }

    fn toggle_graph_view(&mut self, group_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.group(group_id).is_none() {
            return;
        }
        self.activate_group(group_id, cx);
        let Some((node, path, mode)) = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .map(|tab| (tab.node, tab.path.clone(), tab.mode))
        else {
            return;
        };
        if mode == ViewMode::Terminal {
            return;
        }
        if mode.is_editing() {
            self.store_editor_text(node, cx);
        }
        if mode == ViewMode::Graph && self.radial_tree_active.remove(&group_id) {
            cx.notify();
            return;
        }
        let offset = self.document_view_offset(group_id, mode, cx);
        let next = if mode == ViewMode::Graph {
            self.graph_previous_modes
                .remove(&group_id)
                .filter(|(previous_node, previous_path, _)| {
                    *previous_node == node && previous_path == &path
                })
                .map(|(_, _, previous_mode)| previous_mode)
                .unwrap_or(ViewMode::Reading)
        } else {
            self.radial_tree_active.remove(&group_id);
            self.graph_previous_modes
                .insert(group_id, (node, path.clone(), mode));
            ViewMode::Graph
        };
        if let Some(tab) = self
            .model
            .group_mut(group_id)
            .and_then(|group| group.active_mut())
        {
            if let (Some(node), Some(path)) = (node, path) {
                tab.open_node(node, path, next, offset);
            } else {
                tab.mode = next;
            }
        }
        if next == ViewMode::Reading {
            self.refresh_reading(cx);
        } else if next.is_editing() {
            self.focus_editor(window, cx);
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn show_radial_tree(&mut self, group_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mode) = self
            .model
            .group(group_id)
            .and_then(|group| group.active())
            .map(|tab| tab.mode)
        else {
            return;
        };
        if mode == ViewMode::Terminal {
            return;
        }
        if mode == ViewMode::Graph && self.radial_tree_active.remove(&group_id) {
            cx.notify();
            return;
        }
        if mode != ViewMode::Graph {
            self.toggle_graph_view(group_id, window, cx);
        } else {
            self.activate_group(group_id, cx);
        }
        if !self.radial_trees.contains_key(&group_id) {
            RADIAL_TREE_FONTS_REGISTERED.get_or_init(|| {
                radial_tree_explorer::register_fonts(&window.text_system());
            });
            let folders = self.vault.folders();
            let files = self.vault.files();
            let (data, paths) =
                tft_app::explorer::radial_tree_data(&self.vault_name, &folders, &files);
            let focus = cx.focus_handle();
            let tree = cx
                .new(|_| RadialTree::new(data, window, focus.clone(), TreeViewOptions::default()));
            let event_paths = paths.clone();
            let subscription = cx.subscribe(&tree, move |this, _, event: &RadialTreeEvent, cx| {
                if let Some(Some(path)) = event_paths.get(event.0).cloned() {
                    this.radial_tree_active.remove(&group_id);
                    this.pending_radial_open = Some((group_id, path));
                    cx.notify();
                }
            });
            self.radial_trees.insert(
                group_id,
                WebRadialTree {
                    view: tree,
                    _subscription: subscription,
                },
            );
        }
        self.radial_tree_active.insert(group_id);
        if let Some(tree) = self.radial_trees.get(&group_id) {
            let focus = tree.view.read(cx).focus_handle();
            window.focus(&focus, cx);
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn toggle_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((node, mode)) = self
            .model
            .active()
            .and_then(|group| group.active())
            .map(|tab| (tab.node, tab.mode))
        else {
            return;
        };
        // Terminal is a pane type, not a document view mode; match Desktop's
        // guard so Cmd/Ctrl+E cannot replace a running terminal with Reading.
        if mode == ViewMode::Terminal {
            return;
        }
        let offset = self.document_view_offset(self.model.active_group, mode, cx);
        self.store_editor_text(node, cx);
        let next = mode.toggle_reading_editing(ViewMode::LivePreview);
        if mode.is_editing() != next.is_editing() {
            self.sync_group_folds(self.model.active_group, next.is_editing(), cx);
        }
        self.radial_tree_active.remove(&self.model.active_group);
        if let Some(tab) = self.model.active_mut().and_then(|group| group.active_mut()) {
            if let (Some(node), Some(path)) = (tab.node, tab.path.clone()) {
                tab.open_node(node, path, next, offset);
            } else {
                tab.mode = next;
            }
        }
        self.active_editor().update(cx, |editor, cx| {
            editor.set_live_preview(next == ViewMode::LivePreview, cx)
        });
        if next == ViewMode::Reading {
            self.refresh_reading(cx);
            if mode.is_editing() && node.is_some() {
                cx.emit(WebWorkspaceEvent::SaveRequested);
            }
        } else {
            self.focus_editor(window, cx);
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn toggle_source_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((node, path, mode)) = self
            .model
            .active()
            .and_then(|group| group.active())
            .map(|tab| (tab.node, tab.path.clone(), tab.mode))
        else {
            return;
        };
        let Some(next) = mode.toggle_source_preview() else {
            return;
        };
        if !self.can_edit_active_markdown() {
            return;
        }

        let offset = self.document_view_offset(self.model.active_group, mode, cx);
        self.store_editor_text(node, cx);
        if let Some(tab) = self.model.active_mut().and_then(|group| group.active_mut()) {
            tab.mode = next;
            if tft_app::workspace::current_history_entry_mut(&mut tab.history, tab.history_ix)
                .is_some()
            {
                tft_app::workspace::update_history_entry_state(
                    &mut tab.history,
                    tab.history_ix,
                    path.as_deref().unwrap_or_default(),
                    next,
                    offset,
                );
            } else if let (Some(node), Some(path)) = (node, path) {
                tab.open_node(node, path, next, offset);
            }
        }
        self.active_editor().update(cx, |editor, cx| {
            editor.set_live_preview(next == ViewMode::LivePreview, cx)
        });
        self.focus_editor(window, cx);
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn navigate_history(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_group_view_state(self.model.active_group, cx);
        let entry = if forward {
            self.model.forward(self.model.active_group)
        } else {
            self.model.back(self.model.active_group)
        };
        let Some(entry) = entry else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };
        let mode = entry.mode;
        let text = document_text(&self.vault, node);
        self.active_editor().update(cx, |editor, cx| {
            editor.set_workspace_document(entry.path, text, mode, cx);
            editor.reveal_offset(entry.offset, cx);
        });
        if mode.is_editing() {
            self.focus_editor(window, cx);
        } else {
            self.refresh_reading(cx);
            self.active_reading()
                .update(cx, |reader, cx| reader.scroll_to_offset(entry.offset, cx));
        }
        cx.notify();
        self.publish_test_state();
        self.emit_workspace_state(cx);
    }

    fn refresh_reading(&mut self, cx: &mut Context<Self>) {
        let Some((node, path)) = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| Some((tab.node?, tab.path.clone()?)))
        else {
            return;
        };
        let contents = self.active_editor().read(cx).file_contents();
        self.vault.set_text(node, contents.clone());
        let source = VaultEmbedSource::new(&self.vault);
        let model =
            reading::build_with(&path, &contents, &source, reading::BuildOptions::default());
        self.active_reading()
            .update(cx, |reader, cx| reader.set_note(Some(path), model, cx));
        self.publish_test_state();
    }

    fn toggle_task(&mut self, path: &str, marker: reading::TaskMarker, cx: &mut Context<Self>) {
        let active_path = self.current_path().map(str::to_owned);
        if active_path.as_deref() != Some(path) {
            return;
        }
        let changed = self.active_editor().update(cx, |editor, cx| {
            editor.toggle_task_at(marker.line, marker.col, None, cx)
        });
        if changed {
            self.refresh_reading(cx);
            cx.notify();
        }
    }

    fn remove_property_item(
        &mut self,
        path: &str,
        key: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let active_path = self.current_path().map(str::to_owned);
        if active_path.as_deref() != Some(path) {
            return;
        }
        let changed = self.active_editor().update(cx, |editor, cx| {
            editor.remove_property_item(key, index, None, cx)
        });
        if changed {
            self.refresh_reading(cx);
            cx.notify();
        }
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut App) {
        let focus = self.active_editor().read(cx).focus_handle_ref().clone();
        window.focus(&focus, cx);
    }

    fn tree_focus_index(&self) -> Option<usize> {
        let key = self.tree_focused_key.as_deref()?;
        self.tree.rows().position(|row| row.key == key)
    }

    fn handle_tree_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.file_tree_context_menu.is_some() {
            self.file_tree_context_menu = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key == "escape" && !self.tree_selection.is_empty() {
            self.tree_selection.clear();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.tree.is_empty() {
            if matches!(event.keystroke.key.as_str(), "escape" | "tab") {
                self.focus_editor(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let current_key = self
            .tree_focus_index()
            .and_then(|index| self.tree.row(index).map(|row| row.key.clone()))
            .or_else(|| {
                self.current_path()
                    .and_then(|path| self.tree.visible_index(path))
                    .and_then(|index| self.tree.row(index).map(|row| row.key.clone()))
            })
            .or_else(|| self.tree.row(0).map(|row| row.key.clone()));
        let key = event.keystroke.key.as_str();
        let mut handled = true;
        match key {
            "down" | "up" | "shift-down" | "shift-up" => {
                let down = key.ends_with("down");
                let extend = key.starts_with("shift-");
                if let Some(next) = self.tree.focus_vertical_key(
                    self.tree_focused_key.as_deref(),
                    self.current_path(),
                    down,
                ) {
                    self.tree_focused_key = Some(next.clone());
                    if extend {
                        if let Some(index) = self.tree.visible_index(&next) {
                            self.tree_selection.click(&self.tree, index, false, true);
                        }
                    }
                    cx.notify();
                }
            }
            "right" | "left" => {
                if let Some(current) = current_key.as_deref() {
                    if let Some(next) = self.tree.focus_horizontal_key(current, key == "right") {
                        self.tree_focused_key = Some(next);
                        cx.notify();
                    }
                }
            }
            "enter" | "space" => {
                if let Some(row) = current_key
                    .as_deref()
                    .and_then(|key| self.tree.rows().find(|row| row.key == key).cloned())
                {
                    if row.is_folder {
                        self.tree.toggle_key(&row.key);
                        self.tree_focused_key = Some(row.key);
                        cx.notify();
                        self.emit_workspace_state(cx);
                    } else if key == "enter" {
                        if let Some(node) = row.node {
                            self.open(node, row.path, false, window, cx);
                        }
                    }
                }
            }
            "escape" | "tab" => self.focus_editor(window, cx),
            _ => handled = false,
        }
        if handled {
            cx.stop_propagation();
        }
    }

    fn publish_test_state(&self) {
        #[cfg(all(feature = "browser-test", target_arch = "wasm32"))]
        {
            use wasm_bindgen::JsValue;
            let state = js_sys::Object::new();
            let tabs = js_sys::Array::new();
            let search_results = js_sys::Array::new();
            let visible_paths = js_sys::Array::new();
            for row in self.tree.rows() {
                visible_paths.push(&JsValue::from_str(&row.path));
            }
            for hit in &self.search_hits {
                search_results.push(&JsValue::from_str(&hit.path));
            }
            if let Some(group) = self.model.active() {
                for tab in &group.tabs {
                    tabs.push(&JsValue::from_str(tab.path.as_deref().unwrap_or("")));
                }
            }
            let active = self.model.active().and_then(|group| group.active());
            let path = active.and_then(|tab| tab.path.as_deref()).unwrap_or("");
            let node = active
                .and_then(|tab| tab.node)
                .map_or(-1, |node| node.0 as i32);
            let mode = active.map_or("source", |tab| match tab.mode {
                ViewMode::Reading => "reading",
                ViewMode::LivePreview => "live-preview",
                ViewMode::Source => "source",
                ViewMode::Graph => "graph",
                ViewMode::Search => "search",
                ViewMode::Terminal => "terminal",
            });
            let left_tab = match self.model.dock.tab {
                LeftTab::Files => "files",
                LeftTab::Search => "search",
            };
            let last_link = self.last_link_event.clone();
            let vault_text = active
                .and_then(|tab| tab.node)
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("activePath"),
                &JsValue::from_str(path),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("activeNode"),
                &JsValue::from_f64(node as f64),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("vaultText"),
                &JsValue::from_str(&vault_text),
            );
            let _ = js_sys::Reflect::set(&state, &JsValue::from_str("tabs"), &tabs);
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("leftTab"),
                &JsValue::from_str(left_tab),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("leftVisible"),
                &JsValue::from_bool(self.model.dock.visible),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("sortOrder"),
                &JsValue::from_str(self.tree.sort().as_str()),
            );
            let _ =
                js_sys::Reflect::set(&state, &JsValue::from_str("visiblePaths"), &visible_paths);
            let _ =
                js_sys::Reflect::set(&state, &JsValue::from_str("searchResults"), &search_results);
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("selectedSearchHit"),
                &self
                    .selected_search_hit
                    .map(|index| JsValue::from_f64(index as f64))
                    .unwrap_or(JsValue::NULL),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("activeMode"),
                &JsValue::from_str(mode),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("lastLinkEvent"),
                &JsValue::from_str(&last_link),
            );
            let _ = js_sys::Reflect::set(
                &js_sys::global(),
                &JsValue::from_str("__tftWorkspaceState"),
                &state,
            );
        }
    }
}

fn document_text(vault: &VaultData, node: NodeId) -> String {
    vault
        .text(node)
        .map(|text| text.as_ref().clone())
        .unwrap_or_default()
}

fn format_audio_time(seconds: f64) -> String {
    let seconds = if seconds.is_finite() && seconds > 0.0 {
        seconds.floor() as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn build_search_index(vault: &VaultData) -> Vec<SearchDocument> {
    let mut documents = vault
        .files()
        .iter()
        .filter(|file| file.kind == tft_formats::FileKind::Markdown)
        .filter_map(|file| {
            let node = vault.lookup(&file.path).ok()?;
            let contents = vault.text(node)?.clone();
            let metadata = tft_cache::index_markdown(&contents);
            Some(SearchDocument {
                node,
                path: file.path.clone(),
                contents,
                metadata,
                mtime: file.mtime,
                ctime: file.ctime,
            })
        })
        .collect::<Vec<_>>();
    documents.sort_by(|a, b| tft_search::natural_cmp(&a.path, &b.path));
    documents
}

fn sort_web_search_hits(hits: &mut [SearchHit], order: SortOrder) {
    hits.sort_by(|a, b| {
        tft_app::search_results::compare_search_results(
            tft_app::search_results::SearchSortKey {
                name: &a.name,
                path: &a.path,
                modified: a.mtime,
                created: a.ctime,
            },
            tft_app::search_results::SearchSortKey {
                name: &b.name,
                path: &b.path,
                modified: b.mtime,
                created: b.ctime,
            },
            order,
        )
    });
}

fn build_web_graph(
    vault: &VaultData,
    documents: &[SearchDocument],
    excluded: &tft_search::ExcludeFilters,
    config: &tft_formats::config::GraphConfig,
    local: Option<&str>,
    local_options: WebLocalGraphOptions,
) -> (
    tft_app::graph_data::GraphData,
    tft_app::graph_layout::Layout,
) {
    let cache = tft_cache::MetadataCache::new(
        vault_resolver(vault),
        documents
            .iter()
            .map(|document| (document.path.clone(), document.metadata.clone())),
    );
    let data = tft_app::graph_data::build_graph(
        vault,
        &cache,
        excluded,
        config,
        local,
        tft_app::graph_data::LocalGraphOptions {
            jumps: local_options.jumps,
            backlinks: local_options.backlinks,
            forelinks: local_options.forelinks,
            interlinks: local_options.interlinks,
        },
    );
    let defaults = tft_app::graph_layout::Forces::default();
    let mut layout = tft_app::graph_layout::Layout::new(
        data.nodes.len(),
        data.edges.clone(),
        tft_app::graph_layout::Forces {
            center: config.center_strength.unwrap_or(defaults.center),
            repel: config.repel_strength.unwrap_or(defaults.repel),
            link: config.link_strength.unwrap_or(defaults.link),
            link_distance: config.link_distance.unwrap_or(defaults.link_distance),
        },
    );
    for (body, node) in layout.bodies.iter_mut().zip(&data.nodes) {
        body.mass = 1.0 + (node.degree as f32).sqrt() * 0.5;
    }
    (data, layout)
}

impl tft_app::inline_field::Host for WebWorkspace {
    fn editing(&mut self) -> Option<&mut tft_app::inline_field::Editing> {
        self.title_editing
            .editing
            .as_mut()
            .map(|edit| &mut edit.editing)
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_title(window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_title(window, cx);
    }
}

impl EntityInputHandler for WebWorkspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        tft_app::inline_field::text_for_range(
            &self.title_editing.editing.as_ref()?.editing,
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
        Some(tft_app::inline_field::selected_text_range(
            &self.title_editing.editing.as_ref()?.editing,
        ))
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        tft_app::inline_field::marked_text_range(&self.title_editing.editing.as_ref()?.editing)
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(edit) = self.title_editing.editing.as_mut() {
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
        let Some(edit) = self.title_editing.editing.as_mut() else {
            return;
        };
        tft_app::inline_field::replace_text_in_range(&mut edit.editing, range_utf16, new_text);
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
        let Some(edit) = self.title_editing.editing.as_mut() else {
            return;
        };
        tft_app::inline_field::replace_and_mark_text_in_range(
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
        let edit = self.title_editing.editing.as_ref()?;
        let layout = self.title_editing.layouts.get(&edit.group)?;
        Some(tft_app::inline_field::bounds_for_range(
            &edit.editing,
            layout,
            range_utf16,
            element_bounds,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for WebWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((group_id, path)) = self.pending_radial_open.take() {
            if self.model.group(group_id).is_some() {
                self.activate_group(group_id, cx);
                if let Ok(node) = self.vault.lookup(&path) {
                    self.open(node, path, false, window, cx);
                }
            }
        }
        if self.pending_quick_switcher_focus_restore {
            self.pending_quick_switcher_focus_restore = false;
            self.focus_editor(window, cx);
        }
        if let Some((node, path)) = self.pending_quick_switcher_open.take() {
            self.open(node, path, false, window, cx);
        }
        if let Some((index, offset)) = self.pending_search_open.take() {
            if let Some(hit) = self.search_hits.get(index) {
                self.open(hit.node, hit.path.clone(), false, window, cx);
                if let Some(offset) = offset {
                    self.active_editor().update(cx, |editor, cx| {
                        editor.reveal_offset(offset, cx);
                    });
                }
            }
        }
        if let Some((target, new_tab)) = self.pending_link.take() {
            let source = self.pending_link_source.take();
            self.follow_link_target(target, new_tab, source, window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let tokens = &theme.tokens;
        let quick_switcher_results = self.quick_switcher_results(cx);
        let quick_switcher_input = self.quick_switcher_input.clone();
        let mod_key = tft_drive_web::modifier_key_label();
        let (active_node_before_close, active_is_canvas) = self
            .model
            .active()
            .and_then(|group| group.active())
            .map(|tab| (tab.node, tab.path.as_deref()))
            .map(|(node, path)| {
                (
                    node,
                    path.is_some_and(|path| {
                        tft_formats::FileKind::from_path(path) == tft_formats::FileKind::Canvas
                    }),
                )
            })
            .unwrap_or((None, false));
        if self.canvas_card_editor.as_ref().is_some_and(|editor| {
            Some(editor.file) != active_node_before_close || !active_is_canvas
        }) {
            self.close_canvas_card_editor(window, cx);
        }
        // Desktop has one Files pane in the left dock; Search is a main-area
        // tab, not a dock tab. Keeping the dock on Files is the shell parity.
        if self.model.dock.tab != LeftTab::Files {
            self.model.dock.select(LeftTab::Files);
        }
        let active_tab = self.model.active().and_then(|group| group.active());
        let active_node = active_tab.and_then(|tab| tab.node);
        let active_index = self.model.active().map_or(0, |group| group.active_tab);
        let tab_count = self.model.active().map_or(0, |group| group.tabs.len());
        let active_group_id = self.model.active_group;
        let tab_drop_boundary = self.tab_drop_boundary;
        let can_back = active_tab.is_some_and(|tab| tab.history_ix > 1);
        let can_forward = active_tab.is_some_and(|tab| tab.history_ix < tab.history.len());
        let rows = self
            .tree
            .rows()
            .map(|row| {
                (
                    row.node,
                    row.key.clone(),
                    row.path.clone(),
                    row.name.clone(),
                    row.tag.clone(),
                    row.depth,
                    row.is_folder,
                    row.node.is_some() && row.node == active_node,
                    self.tree_selection.contains(&row.path),
                    self.tree_focused_key.as_deref() == Some(row.key.as_str()),
                    self.tree.is_collapsed_key(&row.key),
                )
            })
            .collect::<Vec<_>>();
        let tabs = self
            .model
            .active()
            .into_iter()
            .flat_map(|group| group.tabs.iter())
            .enumerate()
            .map(|(index, tab)| {
                let title = tft_app::workspace::workspace_leaf_title(tab.path.as_deref(), tab.mode);
                let active = index == active_index;
                let divider = !active && index + 1 != active_index && index + 1 != tab_count;
                (
                    index,
                    tab.path.clone().unwrap_or_default(),
                    title,
                    tab.pinned,
                    active,
                    divider,
                )
            })
            .collect::<Vec<_>>();
        let active_mode = active_tab.map_or(ViewMode::default_editing(), |tab| tab.mode);
        let active_document_is_markdown =
            self.is_markdown_node(active_tab.and_then(|tab| tab.node));
        let radial_tree_active = self.radial_tree_active.contains(&active_group_id);
        let main_content = self.render_layout_content(
            &self.model.layout.clone(),
            &mut Vec::new(),
            &mod_key,
            &theme,
            cx,
        );
        let mode_icon = tft_app::ui::workspace_document_mode_icon(active_mode);
        let active_title = tft_app::workspace::workspace_leaf_title(
            active_tab.and_then(|tab| tab.path.as_deref()),
            active_tab.map_or(ViewMode::default_editing(), |tab| tab.mode),
        );
        let breadcrumbs = tft_app::workspace::workspace_breadcrumbs(
            active_tab.and_then(|tab| tab.path.as_deref()),
        );
        let breadcrumb = tft_app::ui::workspace_breadcrumb_trail(
            breadcrumbs
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, (label, path))| {
                    tft_app::ui::workspace_breadcrumb_item(
                        ("web-document-folder-crumb", index),
                        label,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tree.reveal(&path);
                        this.model.dock.show(LeftTab::Files);
                        this.set_left_dock_visible(true, cx);
                    }))
                    .into_any_element()
                })
                .collect::<Vec<_>>(),
            &theme,
        );
        let preview_workspace = cx.entity().downgrade();
        let files_content = {
            div()
                .id("web-files-scroll")
                .key_context("WebExplorer")
                .track_focus(&self.tree_focus)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.handle_tree_key(event, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        window.focus(&this.tree_focus, cx);
                        this.file_tree_context_menu = Some(FileTreeContextMenu {
                            position: event.position,
                            node: None,
                            key: String::new(),
                            path: String::new(),
                            is_folder: false,
                        });
                        cx.notify();
                    }),
                )
                .children(rows.into_iter().map(
                    |(
                        node,
                        key,
                        path,
                        name,
                        tag,
                        depth,
                        folder,
                        active,
                        selected,
                        focused,
                        collapsed,
                    )| {
                        let row_path = path.clone();
                        let menu_key = key.clone();
                        let menu_path = path.clone();
                        let menu_node = node;
                        let hover_path = path.clone();
                        let hover_workspace = preview_workspace.clone();
                        tft_app::ui::hover_row(
                            tft_app::ui::file_tree_row(
                                SharedString::from(format!("file-{key}")),
                                depth,
                                active,
                                selected,
                                focused,
                                &theme,
                            )
                            .on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, window, cx| {
                                    this.file_tree_context_menu = None;
                                    let modifiers = event.modifiers();
                                    if let Some(ix) = this.tree.visible_index(&row_path) {
                                        let range = modifiers.shift;
                                        let additive = modifiers.control || modifiers.platform;
                                        this.tree_selection.click(&this.tree, ix, additive, range);
                                        if additive || range {
                                            this.tree_focused_key = Some(key.clone());
                                            window.focus(&this.tree_focus, cx);
                                            cx.notify();
                                            return;
                                        }
                                    }
                                    this.tree_focused_key = Some(key.clone());
                                    if folder {
                                        window.focus(&this.tree_focus, cx);
                                        this.tree.toggle_key(&key);
                                        cx.notify();
                                        this.emit_workspace_state(cx);
                                    } else if let Some(node) = node {
                                        this.open(
                                            node,
                                            row_path.clone(),
                                            modifiers.control || modifiers.platform,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    window.focus(&this.tree_focus, cx);
                                    if !this.tree_selection.contains(&menu_path) {
                                        if let Some(index) = this.tree.visible_index(&menu_path) {
                                            this.tree_selection
                                                .click(&this.tree, index, false, false);
                                        }
                                    }
                                    this.file_tree_context_menu = Some(FileTreeContextMenu {
                                        position: event.position,
                                        node: menu_node,
                                        key: menu_key.clone(),
                                        path: menu_path.clone(),
                                        is_folder: folder,
                                    });
                                    cx.notify();
                                }),
                            )
                            .children(tft_app::ui::file_tree_indent_guides(depth, &theme))
                            .child(tft_app::ui::file_tree_row_marker(folder, collapsed, &theme))
                            .child(tft_app::ui::file_tree_row_label().child(name))
                            .children(tag.map(|tag| tft_app::ui::file_tree_row_tag(tag, &theme))),
                            move |over, anchor, modifiers, _window, app| {
                                hover_workspace
                                    .update(app, |this, cx| {
                                        this.file_tree_row_hover(
                                            node,
                                            hover_path.clone(),
                                            over,
                                            anchor,
                                            modifiers,
                                            tft_app::page_preview::Source::Files,
                                            cx,
                                        )
                                    })
                                    .ok();
                            },
                        )
                    },
                ))
                .child(div().flex_1().w_full().min_h(px(0.)).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.tree_selection.clear();
                        cx.notify();
                    }),
                ))
                .into_any_element()
        };

        let drive_controls = self.drive_controls;
        let file_tree_context_menu = self.file_tree_context_menu.clone().map(|target| {
            let viewport = window.bounds().size;
            let parent = if target.is_folder {
                target.path.clone()
            } else {
                target
                    .path
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_owned())
                    .unwrap_or_default()
            };
            let mut menu = tft_app::ui::menu("web-file-tree-context-menu", &theme)
                .absolute()
                .left(
                    target
                        .position
                        .x
                        .min((viewport.width - px(210.)).max(px(0.))),
                )
                .top(
                    target
                        .position
                        .y
                        .min((viewport.height - px(320.)).max(px(0.))),
                )
                .max_h((viewport.height - px(16.)).max(px(0.)))
                .overflow_y_scroll();
            let is_file = !target.is_folder && target.node.is_some();
            let menu_rows = tft_app::file_tree_menu::file_tree_menu_items(
                is_file,
                target.is_folder,
                "Reveal in navigation",
            );
            let drive_id = target
                .node
                .and_then(|node| self.vault.backend(node))
                .and_then(|backend| match backend {
                    BackendObjectId::Drive(id) => Some(id.clone()),
                    BackendObjectId::Local(_) => None,
                });
            let mut children: Vec<gpui::AnyElement> = Vec::new();
            let mut previous_section = None;
            for (index, row) in menu_rows.into_iter().enumerate() {
                for _ in 0..tft_app::file_tree_menu::separators_above(previous_section, row.section)
                {
                    children.push(tft_app::ui::menu_separator(&theme).into_any_element());
                }
                previous_section = Some(row.section);
                let action = row.action;
                let available = match action {
                    tft_app::file_tree_menu::FileTreeMenuAction::NewFile
                    | tft_app::file_tree_menu::FileTreeMenuAction::NewFolder
                    | tft_app::file_tree_menu::FileTreeMenuAction::NewCanvas
                    | tft_app::file_tree_menu::FileTreeMenuAction::SearchInFolder
                    | tft_app::file_tree_menu::FileTreeMenuAction::CopyRelativePath
                    | tft_app::file_tree_menu::FileTreeMenuAction::RevealInSystemExplorer => true,
                    tft_app::file_tree_menu::FileTreeMenuAction::OpenInNewTab => is_file,
                    tft_app::file_tree_menu::FileTreeMenuAction::Duplicate => is_file,
                    tft_app::file_tree_menu::FileTreeMenuAction::Move => is_file,
                    tft_app::file_tree_menu::FileTreeMenuAction::Delete => is_file,
                    tft_app::file_tree_menu::FileTreeMenuAction::Rename => is_file,
                    tft_app::file_tree_menu::FileTreeMenuAction::OpenToRight => {
                        is_file && tft_app::workspace::workspace_can_split(self.model.groups.len())
                    }
                    tft_app::file_tree_menu::FileTreeMenuAction::OpenInDefaultApp => {
                        is_file && drive_id.is_some()
                    }
                    _ => false,
                };
                let enabled = row.enabled && available;
                let icon_name = match action {
                    tft_app::file_tree_menu::FileTreeMenuAction::NewFile
                    | tft_app::file_tree_menu::FileTreeMenuAction::OpenInNewTab => "file-plus",
                    tft_app::file_tree_menu::FileTreeMenuAction::NewFolder => "folder-plus",
                    tft_app::file_tree_menu::FileTreeMenuAction::NewCanvas => "layout-dashboard",
                    tft_app::file_tree_menu::FileTreeMenuAction::OpenToRight => {
                        "separator-vertical"
                    }
                    tft_app::file_tree_menu::FileTreeMenuAction::OpenInNewWindow => {
                        "picture-in-picture-2"
                    }
                    tft_app::file_tree_menu::FileTreeMenuAction::Duplicate => "copy",
                    tft_app::file_tree_menu::FileTreeMenuAction::Move => "folder-tree",
                    tft_app::file_tree_menu::FileTreeMenuAction::SearchInFolder => "search",
                    tft_app::file_tree_menu::FileTreeMenuAction::CopyAbsolutePath
                    | tft_app::file_tree_menu::FileTreeMenuAction::CopyRelativePath => "clipboard",
                    tft_app::file_tree_menu::FileTreeMenuAction::OpenInDefaultApp => {
                        "external-link"
                    }
                    tft_app::file_tree_menu::FileTreeMenuAction::RevealInSystemExplorer => {
                        "folder-open"
                    }
                    tft_app::file_tree_menu::FileTreeMenuAction::Rename => "pen-line",
                    tft_app::file_tree_menu::FileTreeMenuAction::Delete => "trash-2",
                };
                let label =
                    if action == tft_app::file_tree_menu::FileTreeMenuAction::OpenInDefaultApp {
                        "Open in Google Drive"
                    } else {
                        row.label
                    };
                let mut item = tft_app::ui::menu_item_with_enabled(
                    "web-file-tree-menu-row",
                    index,
                    Some(icon_name),
                    label,
                    false,
                    enabled,
                    &theme,
                );
                if enabled {
                    item = match action {
                        tft_app::file_tree_menu::FileTreeMenuAction::NewFile => {
                            let parent = parent.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::FileOperationRequested(
                                    FileOperationRequest::CreateNote {
                                        parent: parent.clone(),
                                    },
                                ));
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::NewFolder => {
                            let parent = parent.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::FileOperationRequested(
                                    FileOperationRequest::CreateFolder {
                                        parent: parent.clone(),
                                    },
                                ));
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::NewCanvas => {
                            let parent = parent.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::CreateCanvasRequested {
                                    parent: parent.clone(),
                                });
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::OpenInNewTab => {
                            let node = target.node.expect("enabled file menu row has a node");
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, window, cx| {
                                this.file_tree_context_menu = None;
                                this.open(node, path.clone(), true, window, cx);
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::OpenToRight => {
                            let node = target.node.expect("enabled file menu row has a node");
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, window, cx| {
                                this.file_tree_context_menu = None;
                                this.split_active_group(SplitDirection::Vertical, window, cx);
                                this.open(node, path.clone(), false, window, cx);
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::SearchInFolder => {
                            let search_parent = parent.clone();
                            item.on_click(cx.listener(move |this, _, window, cx| {
                                this.file_tree_context_menu = None;
                                let query = if search_parent.is_empty() {
                                    String::new()
                                } else {
                                    format!("path:\"{search_parent}/\"")
                                };
                                this.show_search(&query, window, cx);
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::CopyRelativePath => {
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                                this.file_tree_context_menu = None;
                                cx.notify();
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::RevealInSystemExplorer => {
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                this.tree.reveal(&path);
                                this.tree_focused_key =
                                    this.tree.visible_index(&path).and_then(|index| {
                                        this.tree.row(index).map(|row| row.key.clone())
                                    });
                                this.model.dock.show(LeftTab::Files);
                                this.set_left_dock_visible(true, cx);
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::OpenInDefaultApp => {
                            let drive_id = drive_id.clone().expect("enabled Drive file row");
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                tft_drive_web::open_drive_file(&drive_id);
                                this.file_tree_context_menu = None;
                                cx.notify();
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::Rename => {
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::RenameFilePromptRequested {
                                    path: path.clone(),
                                });
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::Duplicate => {
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::DuplicateFileRequested {
                                    path: path.clone(),
                                });
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::Move => {
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::MoveFilePromptRequested {
                                    path: path.clone(),
                                });
                            }))
                        }
                        tft_app::file_tree_menu::FileTreeMenuAction::Delete => {
                            let path = target.path.clone();
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_context_menu = None;
                                cx.emit(WebWorkspaceEvent::DeleteFileRequested {
                                    path: path.clone(),
                                });
                            }))
                        }
                        _ => item,
                    };
                }
                children.push(item.into_any_element());
            }
            if target.is_folder {
                let key = target.key.clone();
                let collapsed = self.tree.is_collapsed_key(&key);
                let title = if collapsed {
                    "Expand folder"
                } else {
                    "Collapse folder"
                };
                children.push(tft_app::ui::menu_separator(&theme).into_any_element());
                children.push(
                    tft_app::ui::menu_item(
                        "web-file-tree-menu-toggle-folder",
                        0,
                        Some(if collapsed { "folder-open" } else { "folder" }),
                        title,
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tree.toggle_key(&key);
                        this.file_tree_context_menu = None;
                        cx.notify();
                        this.publish_test_state();
                        this.emit_workspace_state(cx);
                    }))
                    .into_any_element(),
                );
            }
            menu = menu.children(children);
            menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        });

        let quick_switcher_overlay = self.quick_switcher_open.then(|| {
            let count = quick_switcher_results.len();
            let panel = tft_app::ui::palette_panel("web-quick-switcher", 620., &theme)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.quick_switcher_open = false;
                    this.pending_quick_switcher_focus_restore = true;
                    cx.notify();
                }))
                .child(tft_app::ui::palette_search_slot(&theme).child(quick_switcher_input))
                .when(count > 0, |panel| {
                    panel.child(
                        tft_app::ui::palette_results_slot()
                            .id("web-quick-switcher-results")
                            .max_h(px(468.))
                            .overflow_y_scroll()
                            .children(quick_switcher_results.iter().enumerate().map(
                                |(index, hit)| {
                                    let node = hit.node;
                                    let path = hit.path.clone();
                                    let icon_name = match tft_formats::FileKind::from_path(&path) {
                                        tft_formats::FileKind::Canvas => "layout-dashboard",
                                        tft_formats::FileKind::Pdf => "book-open",
                                        _ => "files",
                                    };
                                    let highlights: Vec<_> = hit
                                        .ranges
                                        .iter()
                                        .map(|range| {
                                            (
                                                range.clone(),
                                                HighlightStyle {
                                                    font_weight: Some(FontWeight::BOLD),
                                                    ..HighlightStyle::default()
                                                },
                                            )
                                        })
                                        .collect();
                                    let name =
                                        StyledText::new(SharedString::from(hit.label.clone()))
                                            .with_highlights(highlights);
                                    let detail = if hit.alias {
                                        Some(format!(
                                            "alias of {}",
                                            path.rsplit('/')
                                                .next()
                                                .unwrap_or(&path)
                                                .trim_end_matches(".md")
                                        ))
                                    } else {
                                        path.rsplit_once('/').map(|(parent, _)| parent.to_owned())
                                    };
                                    tft_app::ui::palette_result_row(
                                        SharedString::from(format!("web-quick-switcher-{index}")),
                                        index == self.quick_switcher_selected,
                                        &theme,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.quick_switcher_selected = index;
                                        this.pending_quick_switcher_open =
                                            Some((node, path.clone()));
                                        this.quick_switcher_open = false;
                                        cx.notify();
                                    }))
                                    .child(tft_app::assets::icon(
                                        icon_name,
                                        16.,
                                        theme.tokens.text_muted,
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(14.))
                                            .text_color(if hit.downranked {
                                                theme.tokens.text_muted
                                            } else {
                                                theme.tokens.text_normal
                                            })
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(name),
                                    )
                                    .children(detail.map(
                                        |detail| {
                                            div()
                                                .max_w(px(220.))
                                                .flex_shrink_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .text_size(px(11.))
                                                .text_color(theme.tokens.text_muted)
                                                .child(detail)
                                        },
                                    ))
                                },
                            )),
                    )
                })
                .when(count == 0, |panel| {
                    panel.child(
                        tft_app::ui::palette_results_slot().child(
                            div()
                                .h(px(38.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .text_size(px(12.))
                                .text_color(theme.tokens.text_muted)
                                .child("No matching files."),
                        ),
                    )
                });
            div()
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(90.))
                .child(panel)
        });

        let tab_context_menu = self.tab_context_menu.clone().and_then(|target| {
            let group = self.model.group(target.group)?;
            let tab = group.tabs.get(target.index)?;
            let group_id = target.group;
            let index = target.index;
            let total = group.tabs.len();
            let pinned = tab.pinned;
            let path = tab.path.clone();
            let node = tab.node;
            let mode = tab.mode;
            let is_file = node
                .and_then(|node| self.vault.node(node))
                .is_some_and(|node| matches!(&node.kind, NodeKind::File(_)));
            let is_markdown = node
                .and_then(|node| self.vault.node(node))
                .is_some_and(|node| matches!(&node.kind, NodeKind::File(meta) if meta.kind == tft_formats::FileKind::Markdown));
            let is_active = group_id == self.model.active_group && group.active_tab == index;
            let viewport = window.bounds().size;
            let mut menu = tft_app::ui::menu("web-tab-context-menu", &theme)
                .absolute()
                .left(
                    target
                        .position
                        .x
                        .min((viewport.width - px(210.)).max(px(0.))),
                )
                .top(
                    target
                        .position
                        .y
                        .min((viewport.height - px(330.)).max(px(0.))),
                )
                .child(
                    tft_app::ui::menu_item("web-tab-close", 0, Some("x"), "Close", false, &theme)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.tab_context_menu = None;
                            this.activate_group(group_id, cx);
                            this.close_tab(index, window, cx);
                        })),
                );
            if tft_app::tab_close_action_available(
                tft_app::TabCloseAction::Others,
                total,
                index,
            ) {
                menu = menu.child(tft_app::ui::menu_separator(&theme)).child(
                    tft_app::ui::menu_item(
                        "web-tab-close-others",
                        1,
                        Some("x"),
                        "Close others",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.close_tabs_except(
                            |ix, tab| {
                                tft_app::tab_close_action_keeps(
                                    tft_app::TabCloseAction::Others,
                                    ix,
                                    index,
                                    tab,
                                )
                            },
                            window,
                            cx,
                        );
                    })),
                );
                if tft_app::tab_close_action_available(
                    tft_app::TabCloseAction::After,
                    total,
                    index,
                ) {
                    menu = menu.child(
                        tft_app::ui::menu_item(
                            "web-tab-close-after",
                            2,
                            Some("x"),
                            "Close tabs after",
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.tab_context_menu = None;
                                this.activate_group(group_id, cx);
                                this.close_tabs_except(
                                    |ix, tab| {
                                        tft_app::tab_close_action_keeps(
                                            tft_app::TabCloseAction::After,
                                            ix,
                                            index,
                                            tab,
                                        )
                                    },
                                    window,
                                    cx,
                                );
                            },
                        )),
                    );
                }
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-tab-close-all",
                        3,
                        Some("x"),
                        "Close all",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.close_tabs_except(
                            |ix, tab| {
                                tft_app::tab_close_action_keeps(
                                    tft_app::TabCloseAction::All,
                                    ix,
                                    index,
                                    tab,
                                )
                            },
                            window,
                            cx,
                        );
                    })),
                );
            }
            menu = menu
                .child(tft_app::ui::menu_separator(&theme))
                .child(
                    tft_app::ui::menu_item(
                        "web-tab-toggle-pin",
                        4,
                        Some("pin"),
                        if pinned { "Unpin" } else { "Pin" },
                        pinned,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.toggle_tab_pin(index, cx);
                    })),
                )
                .child(tft_app::ui::menu_separator(&theme))
                .child(
                    tft_app::ui::menu_item(
                        "web-tab-duplicate",
                        5,
                        Some("copy"),
                        "Duplicate",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.duplicate_tab(index, window, cx);
                    })),
                );
            if is_active
                && tft_app::workspace::workspace_can_split(self.model.groups.len())
            {
                menu = menu
                    .child(tft_app::ui::menu_separator(&theme))
                    .child(
                        tft_app::ui::menu_item(
                            "web-tab-split-right",
                            6,
                            Some("separator-vertical"),
                            "Split right",
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.tab_context_menu = None;
                            this.activate_group(group_id, cx);
                            this.activate_tab(index, window, cx);
                            this.split_active_group(SplitDirection::Vertical, window, cx);
                        })),
                    )
                    .child(
                        tft_app::ui::menu_item(
                            "web-tab-split-down",
                            7,
                            Some("separator-horizontal"),
                            "Split down",
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.tab_context_menu = None;
                            this.activate_group(group_id, cx);
                            this.activate_tab(index, window, cx);
                            this.split_active_group(SplitDirection::Horizontal, window, cx);
                        })),
                    );
            }
            if is_active && is_file {
                menu = menu.child(tft_app::ui::menu_separator(&theme)).child(
                    tft_app::ui::menu_item(
                        "web-tab-rename",
                        11,
                        Some("pen-line"),
                        "Rename...",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.activate_tab(index, window, cx);
                        this.edit_active_title(window, cx);
                    })),
                );
                let move_path = path.clone().unwrap_or_default();
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-tab-move-file",
                        15,
                        Some("folder-tree"),
                        "Move file to...",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.activate_tab(index, window, cx);
                        cx.emit(WebWorkspaceEvent::MoveFilePromptRequested {
                            path: move_path.clone(),
                        });
                    })),
                );
                if is_markdown && mode.is_editing() {
                    menu = menu
                        .child(
                            tft_app::ui::menu_item(
                                "web-tab-find",
                                12,
                                Some("search"),
                                "Find...",
                                false,
                                &theme,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.tab_context_menu = None;
                                this.activate_group(group_id, cx);
                                this.activate_tab(index, window, cx);
                                this.active_editor().update(cx, |editor, cx| {
                                    editor.open_find("", false, window, cx)
                                });
                            })),
                        )
                        .child(
                            tft_app::ui::menu_item(
                                "web-tab-replace",
                                13,
                                Some("search"),
                                "Replace...",
                                false,
                                &theme,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.tab_context_menu = None;
                                this.activate_group(group_id, cx);
                                this.activate_tab(index, window, cx);
                                this.active_editor().update(cx, |editor, cx| {
                                    editor.open_find("", true, window, cx)
                                });
                            })),
                        )
                        .child(
                            tft_app::ui::menu_item(
                                "web-tab-add-property",
                                14,
                                Some("circle-plus"),
                                "Add file property",
                                false,
                                &theme,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.tab_context_menu = None;
                                this.activate_group(group_id, cx);
                                this.activate_tab(index, window, cx);
                                this.run_metadata_command(
                                    "markdown:add-metadata-property",
                                    window,
                                    cx,
                                );
                            })),
                        );
                }
            }
            if let Some(path) = path {
                let reveal_path = path.clone();
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-tab-reveal",
                        9,
                        Some("folder-open"),
                        "Reveal in navigation",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.tree.reveal(&reveal_path);
                        this.tree_focused_key = this
                            .tree
                            .visible_index(&reveal_path)
                            .and_then(|index| this.tree.row(index).map(|row| row.key.clone()));
                        this.model.dock.show(LeftTab::Files);
                        this.set_left_dock_visible(true, cx);
                    })),
                );
                if let Some(node) = node {
                    menu = menu.child(
                        tft_app::ui::menu_item(
                            "web-tab-open-drive",
                            8,
                            Some("arrow-up-right"),
                            "Open in Google Drive",
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(BackendObjectId::Drive(id)) = this.vault.backend(node) {
                                tft_drive_web::open_drive_file(id);
                            }
                            this.tab_context_menu = None;
                            cx.notify();
                        })),
                    );
                }
                let delete_path = path.clone();
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-tab-copy-path",
                        10,
                        Some("clipboard"),
                        "Copy path",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_group(group_id, cx);
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        this.tab_context_menu = None;
                        cx.notify();
                    })),
                );
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-tab-delete-file",
                        16,
                        Some("trash-2"),
                        "Delete file",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.activate_group(group_id, cx);
                        this.activate_tab(index, window, cx);
                        cx.emit(WebWorkspaceEvent::DeleteFileRequested {
                            path: delete_path.clone(),
                        });
                    })),
                );
            }
            Some(menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()))
        });

        let editor_context_menu = self.render_editor_context_menu(&theme, window, cx);

        let preview_overlays =
            self.file_previews
                .iter()
                .enumerate()
                .filter_map(|(depth, preview)| {
                    preview.as_ref().map(|preview| {
                        let preview = preview.clone();
                        let reader = self.file_preview_views[depth].clone();
                        let viewport = window.bounds().size;
                        let anchor_right =
                            preview.anchor.origin.x.as_f32() + preview.anchor.size.width.as_f32();
                        let left = if anchor_right + 462.0 <= viewport.width.as_f32() {
                            anchor_right + 10.0
                        } else {
                            (preview.anchor.origin.x.as_f32() - 462.0).max(0.0)
                        };
                        let top = (preview.anchor.origin.y.as_f32()
                            + preview.anchor.size.height.as_f32()
                            + 10.0)
                            .min((viewport.height.as_f32() - 402.0).max(0.0));
                        let workspace = cx.entity().downgrade();
                        let editing = self
                            .preview_editors
                            .get(&depth)
                            .map(|preview| preview.editor.clone());
                        let mut overlay = div()
                            .id(SharedString::from(format!(
                                "web-file-hover-preview-{depth}"
                            )))
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .w(px(450.))
                            .h(px(400.))
                            .overflow_hidden()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.tokens.background_modifier_border)
                            .bg(theme.tokens.background_primary)
                            .shadow_lg()
                            .on_hover(move |over, _, app| {
                                workspace
                                    .update(app, |this, cx| {
                                        this.file_preview_popup_hover(depth, *over, cx)
                                    })
                                    .ok();
                            });
                        if let Some(editor) = editing {
                            let escape_workspace = cx.entity().downgrade();
                            overlay = overlay.child(editor).capture_key_down(
                                move |event, window, app| {
                                    if event.keystroke.key == "escape" {
                                        app.stop_propagation();
                                        escape_workspace
                                            .update(app, |this, cx| {
                                                this.close_file_preview_from(depth, cx);
                                                let focus = this
                                                    .active_editor()
                                                    .read(cx)
                                                    .focus_handle_ref()
                                                    .clone();
                                                window.focus(&focus, cx);
                                            })
                                            .ok();
                                    }
                                },
                            );
                            let outside_workspace = cx.entity().downgrade();
                            overlay = overlay.on_mouse_down_out(move |_, _, app| {
                                outside_workspace
                                    .update(app, |this, cx| this.close_file_preview_from(depth, cx))
                                    .ok();
                            });
                        } else {
                            let workspace = cx.entity().downgrade();
                            overlay = overlay
                                .on_click(move |event, window, app| {
                                    workspace
                                        .update(app, |this, cx| {
                                            let on_link = this
                                                .hovered_preview_link
                                                .as_ref()
                                                .is_some_and(|(_, _, _, target_depth)| {
                                                    *target_depth == depth + 1
                                                });
                                            if !on_link {
                                                this.edit_file_preview(
                                                    depth,
                                                    event.position(),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        })
                                        .ok();
                                })
                                .child(reader);
                        }
                        overlay
                    })
                })
                .collect::<Vec<_>>();

        let narrow_viewport = f32::from(window.bounds().size.width) < 510.;
        let dock_header = tft_app::ui::workspace_dock_header(&theme)
            .gap(px(3.))
            .px_2()
            .child(div().flex_1())
            .child(
                tft_app::ui::icon_button("web-sidebar-toggle", "panel-left", &theme)
                    .opacity(0.72)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if narrow_viewport {
                            this.narrow_dock_open = false;
                        }
                        this.set_left_dock_visible(false, cx);
                    })),
            );
        let dock_panel = div().flex().flex_col().size_full().child(files_content);
        let dock_footer = tft_app::ui::workspace_dock_footer(&theme).child(
            tft_app::ui::workspace_dock_profile_button(
                "web-vault-switcher",
                self.vault_name.clone(),
                &theme,
            )
            .on_click(cx.listener(|_, _, _, cx| cx.emit(WebWorkspaceEvent::SwitchVaultRequested))),
        );
        let dock_resize_handle = tft_app::ui::workspace_dock_resize_handle(
            "web-left-dock-resize",
            true,
            self.dock_dragging,
            &theme,
        )
        .on_drag(DockResizeDrag, |_, _, _, cx| cx.new(|_| EmptyDragPreview));

        let viewport_width = f32::from(window.bounds().size.width);
        let dock_width =
            tft_app::ui::workspace_dock_width_for_viewport(self.dock_width, viewport_width);
        let dock_visible = self.model.dock.visible
            && (dock_width.is_some() || (narrow_viewport && self.narrow_dock_open));
        let dock_width = if narrow_viewport {
            if self.narrow_dock_open {
                (viewport_width - 140.).clamp(150., 300.)
            } else {
                0.
            }
        } else {
            dock_width.unwrap_or(0.)
        };
        div()
            .size_full()
            .relative()
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                    this.file_preview_modifiers_changed(event.modifiers, cx)
                },
            ))
            .on_action(cx.listener(|this, action: &RunWebCommand, window, cx| {
                if action.binding.as_deref().is_some_and(|binding| {
                    !this.hotkey_is_active(&action.id, binding)
                }) {
                    return;
                }
                match action.id.as_str() {
                    "switcher:open" => this.open_quick_switcher(window, cx),
                    "command-palette:open" => this.open_command_palette(window, cx),
                    "workspace:new-tab" => this.new_tab(cx),
                    "workspace:close" => {
                        let index = this.model.active().map(|group| group.active_tab);
                        if let Some(index) = index {
                            let pinned = this.model.active()
                                .and_then(|group| group.tabs.get(index))
                                .is_some_and(|tab| tab.pinned);
                            if pinned {
                                this.toggle_tab_pin(index, cx);
                            } else {
                                this.close_tab(index, window, cx);
                            }
                        }
                    }
                    "workspace:close-others" | "workspace:close-others-tab-group" => {
                        let active_index = this.model.active().map(|group| group.active_tab);
                        this.close_tabs_except(
                            |index, tab| Some(index) == active_index || tab.pinned,
                            window,
                            cx,
                        );
                    }
                    "workspace:copy-path" => {
                        if let Some(path) = this.current_path().map(str::to_owned) {
                            cx.write_to_clipboard(ClipboardItem::new_string(path));
                            cx.notify();
                        }
                    }
                    "canvas:new-file" => {
                        cx.emit(WebWorkspaceEvent::CreateCanvasRequested {
                            parent: String::new(),
                        });
                    }
                    "app:switch-vault" => {
                        cx.emit(WebWorkspaceEvent::SwitchVaultRequested);
                    }
                    "file-explorer:move-file" => {
                        if let Some(path) = this.current_path().map(str::to_owned) {
                            cx.emit(WebWorkspaceEvent::MoveFilePromptRequested { path });
                        }
                    }
                    "editor:context-menu" => {
                        if this.can_edit_active_markdown() {
                            this.active_editor()
                                .update(cx, |editor, cx| editor.context_menu_at_caret(cx));
                        }
                    }
                    "editor:focus" => this.focus_editor(window, cx),
                    "open-with-default-app:open" | "open-with-default-app:show" => {
                        if let Some(id) = this.active_drive_id() {
                            tft_drive_web::open_drive_file(&id);
                        }
                    }
                    "workspace:toggle-pin" => {
                        let index = this.model.active().map(|group| group.active_tab);
                        if let Some(index) = index {
                            this.toggle_tab_pin(index, cx);
                        }
                    }
                    "workspace:split-vertical" => this.split_active_group(
                        SplitDirection::Vertical,
                        window,
                        cx,
                    ),
                    "workspace:split-horizontal" => this.split_active_group(
                        SplitDirection::Horizontal,
                        window,
                        cx,
                    ),
                    "workspace:close-tab-group" => this.close_active_group(window, cx),
                    "workspace:next-tab" | "workspace:previous-tab" => {
                        let next = this.model.active().and_then(|group| {
                            tft_app::cycle_tab_index(
                                group.active_tab,
                                group.tabs.len(),
                                action.id == "workspace:next-tab",
                            )
                        });
                        if let Some(index) = next {
                            this.activate_tab(index, window, cx);
                        }
                    }
                    id if id.starts_with("workspace:goto-tab-")
                        || id == "workspace:goto-last-tab" =>
                    {
                        let count = this.model.active().map_or(0, |group| group.tabs.len());
                        let index = if id == "workspace:goto-last-tab" {
                            count.checked_sub(1)
                        } else {
                            id.rsplit('-')
                                .next()
                                .and_then(|number| number.parse::<usize>().ok())
                                .and_then(|number| number.checked_sub(1))
                                .filter(|index| *index < count)
                        };
                        if let Some(index) = index {
                            this.activate_tab(index, window, cx);
                        }
                    }
                    "global-search:open" => {
                        let query = this.search_input.read(cx).text().to_owned();
                        this.show_search(&query, window, cx);
                    }
                    "app:go-back" => this.navigate_history(false, window, cx),
                    "app:go-forward" => this.navigate_history(true, window, cx),
                    "markdown:toggle-preview" => this.toggle_view(window, cx),
                    "editor:toggle-source" => this.toggle_source_view(window, cx),
                    "workspace:edit-file-title" => this.edit_active_title(window, cx),
                    "editor:rename-heading" => this.request_rename_heading(cx),
                    "editor:focus-left" => {
                        this.focus_neighbor(SplitDirection::Vertical, false, window, cx)
                    }
                    "editor:focus-right" => {
                        this.focus_neighbor(SplitDirection::Vertical, true, window, cx)
                    }
                    "editor:focus-top" => {
                        this.focus_neighbor(SplitDirection::Horizontal, false, window, cx)
                    }
                    "editor:focus-bottom" => {
                        this.focus_neighbor(SplitDirection::Horizontal, true, window, cx)
                    }
                    "markdown:add-metadata-property"
                    | "markdown:add-alias"
                    | "markdown:clear-metadata-properties"
                    | "editor:toggle-fold-properties" => {
                        this.run_metadata_command(&action.id, window, cx)
                    }
                    id if tft_app::text::table::TableCommand::from_id(id).is_some() => {
                        this.run_table_command(id, window, cx)
                    }
                    "graph:open" => {
                        this.toggle_graph_view(this.model.active_group, window, cx)
                    }
                    "graph:open-local" => {
                        this.open_local_graph(this.model.active_group, window, cx)
                    }
                    "editor:save-file" => cx.emit(WebWorkspaceEvent::SaveRequested),
                    "editor:open-search" => this
                        .active_editor()
                        .update(cx, |editor, cx| editor.open_find("", false, window, cx)),
                    "editor:open-search-replace" => this
                        .active_editor()
                        .update(cx, |editor, cx| editor.open_find("", true, window, cx)),
                    id if Self::is_markdown_editor_command(id) => {
                        this.run_markdown_editor_command(id, window, cx)
                    }
                    "editor:follow-link" => this.follow_editor_link(false, false, window, cx),
                    "editor:open-link-in-new-leaf" => {
                        this.follow_editor_link(true, false, window, cx)
                    }
                    "editor:open-link-in-new-split" => {
                        this.follow_editor_link(false, true, window, cx)
                    }
                    "file-explorer:open" => this.open_file_explorer(false, window, cx),
                    "file-explorer:reveal-active-file" => {
                        this.open_file_explorer(true, window, cx)
                    }
                    "file-explorer:new-file" => this.request_new_note(cx),
                    "file-explorer:new-folder" => {
                        cx.emit(WebWorkspaceEvent::FileOperationRequested(
                            FileOperationRequest::CreateFolder {
                                parent: String::new(),
                            },
                        ));
                    }
                    "file-explorer:duplicate-file" => {
                        if let Some(path) = this.current_path().map(str::to_owned) {
                            cx.emit(WebWorkspaceEvent::DuplicateFileRequested { path });
                        }
                    }
                    "app:delete-file" => {
                        if let Some(path) = this.current_path().map(str::to_owned) {
                            cx.emit(WebWorkspaceEvent::DeleteFileRequested { path });
                        }
                    }
                    "app:toggle-left-sidebar" => this.toggle_left_dock(cx),
                    _ => {}
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.file_tree_context_menu.take().is_some()
                        || this.tab_context_menu.take().is_some()
                    {
                        cx.notify();
                    }
                }),
                                )
            .flex()
            .flex_col()
            .bg(tokens.background_primary)
            .text_color(tokens.text_normal)
            .font_family(theme.ui_font.clone())
                                .child(
                                    div()
                    .flex_1()
                    .min_h_0()
                                        .flex()
                    .on_drag_move(cx.listener(|this, event: &DragMoveEvent<DockResizeDrag>, _, cx| {
                        this.dock_width = tft_app::ui::workspace_dock_width_at(
                            f32::from(event.event.position.x),
                            f32::from(event.bounds.origin.x),
                            f32::from(event.bounds.size.width),
                        );
                        this.dock_dragging = true;
                        cx.notify();
                    }))
                    .on_drop(cx.listener(|this, _: &DockResizeDrag, _, cx| {
                        this.dock_dragging = false;
                        this.emit_workspace_state(cx);
                        cx.notify();
                    }))
                    .when(dock_visible, |layout| {
                        layout.child(
                            tft_app::ui::workspace_dock_shell(
                                px(dock_width),
                                dock_header,
                                dock_panel,
                                dock_footer,
                                dock_resize_handle,
                                                &theme,
                                            )
                            .border_r_1()
                            .border_color(tokens.tab_outline_color),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(self.model.groups.len() == 1, |area| area.child(
                                tft_app::ui::workspace_tab_strip_shell(&theme)
                                    .flex_shrink_0()
                                    .when(!dock_visible, |strip| {
                                        strip.child(
                                            tft_app::ui::icon_button(
                                                "web-sidebar-restore",
                                                "panel-left",
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    if narrow_viewport {
                                                        this.narrow_dock_open = true;
                                                    }
                                                    this.set_left_dock_visible(true, cx);
                                                }),
                                            ),
                                        )
                                    })
                                    .child(tft_app::ui::workspace_tab_strip_navigation(
                                        tft_app::ui::icon_button(
                                            "web-tab-back",
                                            "arrow-left",
                                            &theme,
                                        )
                                        .when(!can_back, |button| button.opacity(0.4))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate_history(false, window, cx)
                                            }),
                                        ),
                                        tft_app::ui::icon_button(
                                            "web-tab-forward",
                                            "arrow-right",
                                            &theme,
                                        )
                                        .when(!can_forward, |button| button.opacity(0.4))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate_history(true, window, cx)
                                            }),
                                        ),
                                        &theme,
                                    ))
                                    .child(tft_app::ui::workspace_tab_strip_tabs(
                                            "web-tab-strip-scroll",
                                            tabs.into_iter().map(
                                            |(index, path, title, pinned, active, divider)| {
                                                let preview_workspace = cx.weak_entity();
                                                let preview_path = path.clone();
                                                let tab = tft_app::ui::workspace_tab(
                                                    SharedString::from(format!(
                                                        "tab-{index}-{path}"
                                                    )),
                                                    active,
                                                    true,
                                                    &theme,
                                                );
                                                tft_app::ui::hover_row(tab, move |over, anchor, modifiers, _, app| {
                                                    let Some(workspace) = preview_workspace.upgrade() else {
                                                        return;
                                                    };
                                                    let path = preview_path.clone();
                                                    workspace
                                                        .update(app, |this, cx| {
                                                            let node = this.vault.lookup(&path).ok();
                                                            this.file_tree_row_hover(
                                                                node,
                                                                path,
                                                                over,
                                                                anchor,
                                                                modifiers,
                                                                tft_app::page_preview::Source::TabHeader,
                                                                cx,
                                                            );
                                                        });
                                                })
                                                .flex()
                                                .when(divider, |tab| {
                                                    tab.child(gpui::deferred(
                                                        tft_app::ui::workspace_tab_divider(&theme),
                                                    ))
                                                })
                                                .when(tab_drop_boundary == Some(index), |tab| {
                                                    tab.child(gpui::deferred(
                                                        tft_app::ui::workspace_tab_drop_indicator(
                                                            false, &theme,
                                                        ),
                                                    ))
                                                })
                                                .when(
                                                    tab_drop_boundary == Some(index + 1)
                                                        && index + 1 == tab_count,
                                                    |tab| {
                                                        tab.child(gpui::deferred(
                                                            tft_app::ui::workspace_tab_drop_indicator(
                                                                true, &theme,
                                                            ),
                                                        ))
                                                    },
                                                )
                                                .items_center()
                                                .cursor_pointer()
                                                .on_drag(
                                                    TabReorderDrag {
                                                        source: tft_app::TabDragSource {
                                                            group_id: active_group_id,
                                                            tab_index: index,
                                                        },
                                                        title: title.clone().into(),
                                                    },
                                                    |drag, _, _, cx| {
                                                        let title = drag.title.clone();
                                                        cx.new(|_| TabDragPreview { title })
                                                    },
                                                )
                                                .on_drag_move(cx.listener(
                                                    move |this,
                                                          event: &DragMoveEvent<TabReorderDrag>,
                                                          _,
                                                          cx| {
                                                        let bounds = [(
                                                            index,
                                                            f32::from(event.bounds.left()),
                                                            f32::from(event.bounds.right()),
                                                        )];
                                                        let boundary = tft_app::workspace::nearest_tab_drop_boundary(
                                                            f32::from(event.event.position.x),
                                                            &bounds,
                                                        )
                                                        .map(|(boundary, _)| boundary)
                                                        .unwrap_or(index);
                                                        if this.tab_drop_boundary != Some(boundary) {
                                                            this.tab_drop_boundary = Some(boundary);
                                                            cx.notify();
                                                        }
                                                    },
                                                ))
                                                .on_drop(cx.listener(
                                                    move |this,
                                                          drag: &TabReorderDrag,
                                                          window,
                                                          cx| {
                                                        let boundary = this
                                                            .tab_drop_boundary
                                                            .take()
                                                            .unwrap_or(index);
                                                        if drag.source.group_id == active_group_id {
                                                            this.reorder_tab(
                                                                active_group_id,
                                                                drag.source.tab_index,
                                                                boundary,
                                                                window,
                                                                cx,
                                                            );
                                                        } else {
                                                            this.transfer_tab_between_groups(
                                                                drag.source.group_id,
                                                                drag.source.tab_index,
                                                                active_group_id,
                                                                boundary,
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    },
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.activate_tab(index, window, cx)
                                                    },
                                                ))
                                                .on_mouse_down(
                                                    gpui::MouseButton::Middle,
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.close_tab(index, window, cx)
                                                    }),
                                                )
                                                .on_mouse_down(
                                                    MouseButton::Right,
                                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                                        cx.stop_propagation();
                                                        this.activate_group(active_group_id, cx);
                                                        this.file_tree_context_menu = None;
                                                        this.tab_context_menu =
                                                            Some(TabContextMenu {
                                                            position: event.position,
                                                            group: active_group_id,
                                                            index,
                                                        });
                                                        cx.notify();
                                                    }),
                                                )
                                                .child(tft_app::ui::workspace_tab_content(
                                                    title,
                                                        tft_app::ui::workspace_tab_close_button(
                                                            format!("close-tab-{index}"),
                                                            active,
                                                            pinned,
                                                            &theme,
                                                        )
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                cx.stop_propagation();
                                                            if pinned {
                                                                this.activate_tab(index, window, cx);
                                                                this.toggle_tab_pin(index, cx);
                                                            } else {
                                                                this.close_tab(index, window, cx);
                                                            }
                                                            },
                                                        )),
                                                    active,
                                                    &theme,
                                        ))
                                                .into_any_element()
                                            },
                                            ),
                                            tft_app::ui::workspace_new_tab_button("web-new-tab", &theme)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.new_tab(cx);
                                                })),
                                            None,
                                    )),
                            ))
                            .when(self.model.groups.len() == 1, |area| area.child(
                                tft_app::ui::workspace_document_header(
                                    tft_app::ui::workspace_view_header_title()
                                            .text_ellipsis()
                                            .when(!breadcrumbs.is_empty(), |header| {
                                                header.child(breadcrumb).child(
                                                    tft_app::ui::workspace_breadcrumb_separator(
                                                        &theme,
                                                    ),
                                                )
                                            })
                                            .child(active_title),
                                    tft_app::ui::workspace_view_header_tools(
                                            "web-document-header-tools",
                                        )
                                        .when(active_mode != ViewMode::Terminal, |tools| {
                                            tools
                                                .child(
                                                    tft_app::ui::workspace_graph_engine_button(
                                                        "web-graph",
                                                        "web-graph",
                                                        false,
                                                        active_mode == ViewMode::Graph
                                                            && !radial_tree_active,
                                                        &theme,
                                                    )
                                                    .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.toggle_graph_view(
                                                                        active_group_id,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                    )),
                                                )
                                                .child(
                                                    tft_app::ui::workspace_graph_engine_button(
                                                        "web-radial-tree",
                                                        "web-radial-tree",
                                                        true,
                                                        active_mode == ViewMode::Graph
                                                            && radial_tree_active,
                                                        &theme,
                                                    )
                                                    .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.show_radial_tree(
                                                                        active_group_id,
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                    )),
                                                )
                                        })
                                        .when(
                                            tft_app::workspace::workspace_header_shows_mode_switch(
                                                active_tab.and_then(|tab| tab.path.as_deref()),
                                                active_tab.map_or(ViewMode::default_editing(), |tab| tab.mode),
                                            ),
                                            |tools| {
                                                tools.child(
                                                    tft_app::ui::icon_button(
                                                        "web-mode", mode_icon, &theme,
                                                    )
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.toggle_view(window, cx)
                                                    })),
                                                )
                                            },
                                        )
                                        .when(drive_controls && active_document_is_markdown, |tools| {
                                            tools.child(
                                            tft_app::ui::header_button(
                                                "web-save-drive",
                                                "check",
                                                "Save to Drive",
                                                false,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|_, _, _, cx| {
                                                    cx.emit(WebWorkspaceEvent::SaveRequested)
                                                }),
                                            ),
                                        )
                                    }),
                                    &theme,
                            )
                            ))
                            .child(div().flex_1().min_h_0().child(main_content)),
                    ),
            )
            .children(file_tree_context_menu)
            .children(tab_context_menu)
            .children(editor_context_menu)
            .children(quick_switcher_overlay)
            .children(self.command_palette_open.then(|| self.command_palette.clone()))
            .children(preview_overlays)
    }
}

fn restore_web_tabs(vault: &VaultData, items: &[serde_json::Value]) -> Vec<Tab> {
    items
        .iter()
        .filter_map(|item| {
            let mode = item
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .and_then(ViewMode::parse)
                .unwrap_or(ViewMode::default_editing());
            let mut history = item
                .get("history")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| {
                    let path = entry.get("file")?.as_str()?.to_owned();
                    let node = vault.lookup(&path).ok()?;
                    if !vault.node(node).is_some_and(|entry| {
                        matches!(
                            &entry.kind,
                            NodeKind::File(meta) if is_web_openable_file(meta.kind)
                        )
                    }) {
                        return None;
                    }
                    let entry_mode = entry
                        .get("mode")
                        .and_then(serde_json::Value::as_str)
                        .and_then(ViewMode::parse)
                        .unwrap_or(mode);
                    let offset = entry
                        .get("offset")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as usize;
                    Some(tft_app::workspace::HistoryEntry::with_node(
                        node, path, entry_mode, offset,
                    ))
                })
                .collect::<Vec<_>>();
            let mut tab = if history.is_empty() {
                if let Some(path) = item.get("file").and_then(serde_json::Value::as_str) {
                    let node = vault.lookup(path).ok()?;
                    if !vault.node(node).is_some_and(|entry| {
                        matches!(
                            &entry.kind,
                            NodeKind::File(meta) if is_web_openable_file(meta.kind)
                        )
                    }) {
                        return None;
                    }
                    let mut tab = Tab::unloaded(path.to_owned(), mode);
                    tab.node = Some(node);
                    if let Some(entry) = tab.history.last_mut() {
                        entry.node = Some(node);
                    }
                    tab
                } else {
                    Tab::empty(mode)
                }
            } else {
                let history_ix = item
                    .get("historyIx")
                    .and_then(serde_json::Value::as_u64)
                    .map(|ix| ix as usize)
                    .unwrap_or(history.len())
                    .clamp(1, history.len());
                let current = history[history_ix - 1].clone();
                let mut tab = Tab::unloaded(current.path.clone(), current.mode);
                tab.history = std::mem::take(&mut history);
                tab.history_ix = history_ix;
                tab.node = current.node;
                tab.path = Some(current.path);
                tab.mode = current.mode;
                tab
            };
            tab.pinned = item
                .get("pinned")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            Some(tab)
        })
        .collect()
}

fn is_web_openable_file(kind: tft_formats::FileKind) -> bool {
    matches!(
        kind,
        tft_formats::FileKind::Markdown
            | tft_formats::FileKind::Canvas
            | tft_formats::FileKind::Pdf
            | tft_formats::FileKind::Image
            | tft_formats::FileKind::Audio
            | tft_formats::FileKind::Video
    )
}

fn web_tab_value(tab: &Tab, active_offset: Option<usize>) -> serde_json::Value {
    let active_history = tab.history_ix.checked_sub(1);
    serde_json::json!({
        "file": tab.path,
        "mode": tab.mode.as_str(),
        "pinned": tab.pinned,
        "historyIx": tab.history_ix,
        "history": tab.history.iter().enumerate().map(|(index, entry)| serde_json::json!({
            "file": entry.path,
            "mode": entry.mode.as_str(),
            "offset": if Some(index) == active_history { active_offset.unwrap_or(entry.offset) } else { entry.offset },
        })).collect::<Vec<_>>(),
    })
}

fn parse_web_layout(value: &serde_json::Value) -> Option<Layout> {
    if let Some(id) = value.get("group").and_then(serde_json::Value::as_u64) {
        return Some(Layout::Group(id));
    }
    let direction = SplitDirection::parse(value.get("direction").and_then(|v| v.as_str()));
    let children = value
        .get("children")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .map(parse_web_layout)
        .collect::<Option<Vec<_>>>()?;
    match children.len() {
        0 => None,
        1 => children.into_iter().next(),
        _ => {
            let mut node = serde_json::Map::from_iter([
                ("id".to_owned(), serde_json::json!(0)),
                ("type".to_owned(), serde_json::json!("split")),
            ]);
            if let Some(dimensions) = value.get("dimensions") {
                node.insert("dimensions".to_owned(), dimensions.clone());
            }
            Some(Layout::split_with_node(direction, children, node))
        }
    }
}

fn layout_group_ids(layout: &Layout) -> Vec<u64> {
    let mut ids = Vec::new();
    layout.groups(&mut ids);
    ids
}

fn layout_matches_group_ids(layout: &Layout, expected: &[u64]) -> bool {
    let mut actual = layout_group_ids(layout);
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    actual == expected && actual.windows(2).all(|pair| pair[0] != pair[1])
}

fn web_layout_value(layout: &Layout) -> serde_json::Value {
    match layout {
        Layout::Group(id) => serde_json::json!({ "group": id }),
        Layout::Split {
            direction,
            children,
            node,
        } => serde_json::json!({
            "direction": direction.as_str(),
            "dimensions": node.get("dimensions"),
            "children": children.iter().map(web_layout_value).collect::<Vec<_>>(),
        }),
    }
}

fn vault_resolver(vault: &VaultData) -> LinkResolver {
    LinkResolver::new(vault.files().iter().map(|file| file.path.clone()))
}

fn graph_screen_positions(
    bodies: &[tft_app::graph_layout::Body],
    bounds: Bounds<Pixels>,
    zoom: f32,
    offset: (f32, f32),
) -> Vec<Point<Pixels>> {
    if bodies.is_empty() {
        return Vec::new();
    }
    let (min_x, max_x, min_y, max_y) = bodies.iter().fold(
        (
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ),
        |(min_x, max_x, min_y, max_y), body| {
            (
                min_x.min(body.x),
                max_x.max(body.x),
                min_y.min(body.y),
                max_y.max(body.y),
            )
        },
    );
    let span = (max_x - min_x).max(max_y - min_y).max(1.0);
    let fit = (bounds.size.width.as_f32().min(bounds.size.height.as_f32()) * 0.82 / span)
        .clamp(0.01, 8.0)
        * zoom;
    let center_x = (min_x + max_x) * 0.5;
    let center_y = (min_y + max_y) * 0.5;
    let center = bounds.center();
    bodies
        .iter()
        .map(|body| {
            point(
                center.x + px((body.x - center_x) * fit + offset.0),
                center.y + px((body.y - center_y) * fit + offset.1),
            )
        })
        .collect()
}

struct VaultEmbedSource<'a> {
    vault: &'a VaultData,
    resolver: LinkResolver,
}

impl<'a> VaultEmbedSource<'a> {
    fn new(vault: &'a VaultData) -> Self {
        Self {
            vault,
            resolver: vault_resolver(vault),
        }
    }
}

impl EmbedSource for VaultEmbedSource<'_> {
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String> {
        self.resolver
            .resolve(linktext, source_path)
            .map(str::to_owned)
    }

    fn read(&self, path: &str) -> Option<String> {
        let node = self.vault.lookup(path).ok()?;
        self.vault.text(node).map(|text| text.as_ref().clone())
    }

    fn absolute(&self, path: &str) -> PathBuf {
        PathBuf::from(path)
    }
}
