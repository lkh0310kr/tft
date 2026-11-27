//! The workspace shell: docks, tab groups, the reading/editor area, overlays and popouts.
//! Commands run by id; hotkeys come from `hotkeys.json`; the vault is indexed in the background.

use crate::assets::icon;
use crate::canvas::CanvasView;
use crate::chooser::Chooser;
use crate::copy_results::CopyResults;
use crate::explorer::{FileExplorer, display_name};
use crate::export_modals::{ImageExport, PdfExport};
use crate::file_menu::{self, Pick};
use crate::graph::GraphView;
use crate::inline_field;
use crate::palette::CommandPalette;
use crate::pdf::view::PdfView;
use crate::prompt::Prompt;
use crate::reading::{LinkTarget, ReadingView};
use crate::search::SearchPane;
use crate::settings::SettingsView;
use crate::switcher::QuickSwitcher;
use crate::theme::{HEADER_HEIGHT, ICON_SIZE, RIBBON_WIDTH, SIDEBAR_WIDTH};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Window, actions,
    prelude::*,
};
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::path::PathBuf;
use tft_cache::basename;
use tft_formats::FileKind;

// Files from outside the vault: insert attachment, download attachments, open file.
mod attachments;
mod file_menus;
use file_menus::FileMenuState;
mod file_operations;
#[cfg(test)]
use crate::prompt::PromptAction;
#[cfg(test)]
use file_operations::update_links_prompt;
use file_operations::{PendingOp, PromptState};
mod focus;
use focus::WorkspaceFocusState;
mod group_layout;
use group_layout::DividerDragState;
mod groups;
use groups::{ActiveGroupState, GroupRegistryState, TabGroup};
mod layout_docks;
mod layout_persistence;
use layout_persistence::WorkspacePersistenceState;
mod layout_read;
mod layout_write;
mod tab_dnd;
mod tab_drag;
use layout_docks::default_dock_json;
#[cfg(test)]
use layout_docks::dock_current_view;
use layout_read::{LayoutSpec, read_floating, read_main};
#[cfg(test)]
pub(crate) use layout_read::{tab_from_leaf, WindowSpec};
use layout_write::{
    desktop_split, floating_json, set_dimension, split_node_json, tabs_node_json, window_node_json,
};
pub(crate) use layout_write::{leaf_id, leaf_json};
use subscriptions::workspace_subscriptions;
use tab_drag::{DragBounds, DropBar, DropZone, TabDrag, TabDragState};
mod canvas_actions;
use canvas_actions::ChooserPurpose;
mod command_palette;
mod commands;
mod control_api;
use commands::CommandState;
pub use commands::{Commands, bind_keys};
#[allow(unused_imports)]
pub(super) use commands::{ACTIVE_FILE_ONLY, EDITOR_ONLY, RUNNABLE};
mod dock_controls;
use dock_controls::DockState;
mod document_folds;
use document_folds::FoldStoreState;
mod document_session;
use document_session::DocumentSaveState;
mod document_views;
#[cfg(test)]
use document_views::prune_history;
mod empty_tab;
mod inline_title;
use inline_title::TitleEditingState;
mod notices;
mod overlays;
use overlays::{Modal, OverlayState};
mod popouts;
mod previews;
mod radial_tree_explorer;
use previews::PreviewState;
use radial_tree_explorer::RadialTreeState;
mod quick_switcher;
mod render;
mod render_requests;
pub(crate) use render::context_names;
use render_requests::RenderRequests;
pub(super) use tab_strip::leaf_title;
#[cfg(test)]
use tab_strip::tab_display_name;
mod settings_window;
use settings_window::SettingsWindowState;
mod subscriptions;
mod tab_strip;
mod uri_handler;
mod vault_switcher;
use vault_switcher::VaultSwitcherState;
mod main_window;
mod view_header;
mod window_node;
#[cfg(test)]
use commands::{
    any_markdown_leaf, full_path_text, missing_file_notice, vault_folder_name, TRASH_FOLDER,
};
#[cfg(test)]
use control_api::vault_path_in;
use main_window::MainWindowState;
mod vault_lifecycle;
use vault_lifecycle::{ThemeAppearanceState, VaultSessionState};
mod vault_state;
mod window_frame;
use notices::FeedbackState;
pub(crate) use vault_state::{Changes, VaultState};
use window_frame::WINDOW_RADIUS;
use window_frame::WindowFrame;
pub(crate) use window_frame::{
    WindowCorners, client_frame, frame_window_options, popout_margin, window_controls,
    window_handle, window_title_row,
};
#[cfg(test)]
use window_node::WindowBounds;
use window_node::{WindowNode, popout_outer_size, popout_placement};

// The app's own commands: debug info and release notes.
mod release_notes;
use release_notes::ReleaseNotesState;

// Frontmatter migration.

// Export to PDF and Export as image.
mod exports;

#[cfg(test)]
use notices::Notice;

actions!(
    workspace,
    [
        ToggleMode,
        GoBack,
        GoForward,
        OpenSearch,
        OpenGraph,
        OpenCommandPalette,
        FocusExplorer
    ]
);

actions!(
    dock_tree,
    [TreeUp, TreeDown, TreeLeft, TreeRight, TreeActivate]
);

pub use tft_app::ViewMode;

/// What the main area paints: the mode asks for a view, the open file decides whether
/// it can show one, and `Empty` stands in when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainView {
    Reading,
    Editor,
    Graph,
    Search,
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
            MainView::Search => "search",
            MainView::Canvas => "canvas",
            MainView::Pdf => "pdf",
            MainView::ReleaseNotes => "release-notes",
            MainView::Empty => "nothing",
        }
    }
}

#[cfg(test)]
use tft_app::{HistoryEntry, place_new_tab};
/// One place back and forward can land: the note and the state the view was left
/// in, so back to a note read in Reading returns it in Reading.
use tft_app::{Layout, SplitDirection, Tab};

pub struct Workspace {
    /// Workspace-root keyboard focus and deferred main-view focus requests.
    focus: WorkspaceFocusState,
    /// System appearance and the Desktop session override for system-following themes.
    theme_appearance: ThemeAppearanceState,
    /// The active vault and its load/watch lifecycle.
    vault_session: VaultSessionState,
    /// Vault menu and manager-window state owned by the switcher feature.
    vault_switcher_state: VaultSwitcherState,
    /// Settings view and its separate window lifecycle.
    settings_state: SettingsWindowState,
    explorer: Entity<FileExplorer>,
    search: Entity<SearchPane>,
    switcher: Entity<QuickSwitcher>,
    /// The command palette (`Mod+Shift+P`).
    command_palette: Entity<CommandPalette>,
    chooser: Entity<Chooser>,
    /// Attachment import and download modal lifecycle.
    attachments: attachments::AttachmentState,
    /// Frontmatter migration dialog and its last run's Undo data.
    /// Requests delivered by event callbacks that need a Window on the next render.
    render_requests: RenderRequests,
    prompt: PromptState,
    copy_results: Entity<CopyResults>,
    /// "Debug info", filled each time it opens.
    /// "Export to PDF", filled each time it opens.
    pdf_export: Entity<PdfExport>,
    /// "Export as image", at its defaults each time it opens.
    image_export: Entity<ImageExport>,
    /// Dedicated Reading view and loaded-version state for release notes.
    release_notes: ReleaseNotesState,
    /// Current overlay and the window that owns its focus/presentation lifecycle.
    overlay: OverlayState,
    /// Session command history and transient command-invocation context.
    command_state: CommandState,
    /// The notes' fold records, kept outside the vault and opened on first use.
    fold_store: FoldStoreState,
    /// Left sidebar state and focus, persisted as the left dock in workspace.json.
    left_dock: DockState,
    /// The workspace-wide graph view. Only one graph leaf is painted at a time;
    /// other groups show a placeholder, so this is intentionally not group-owned.
    graph: Entity<GraphView>,
    /// Optional folder hierarchy view; the force GraphView remains independent.
    radial_tree: RadialTreeState,
    /// Live tabs and navigation state for the active group, separate from its stashed snapshot.
    active_group_state: ActiveGroupState,
    /// Tabs eligible for session-local close/restore, with the history limit enforced.
    closed_tab_history: groups::ClosedTabHistory,
    /// Every tab group, the active one's slot holding what it last stashed.
    groups: Vec<TabGroup>,
    active_group: usize,
    /// How the groups are split, as `workspace.json`'s `main` records it.
    layout_tree: Layout,
    /// The popout windows `floating` records; their groups are in `groups` but not `layout_tree`.
    windows: Vec<WindowNode>,
    /// The main window: where `layout_tree`'s groups are, and where the keyboard goes for them.
    /// Also owns its native handle, persisted zoom level and displayed title.
    main_window: MainWindowState,
    /// Shared group/window id allocation and most-recently-active group order.
    group_registry: GroupRegistryState,
    /// Cross-window tab drag state, read by Workspace painting and drag handlers.
    tab_drag_state: TabDragState,
    /// The split divider being dragged, for its accent while held.
    divider_drag_state: DividerDragState,
    /// Inline title editing and its focus/layout state.
    title_editing: TitleEditingState,
    /// Persisted `workspace.json` state and its load/reload lifecycle.
    layout_persistence: WorkspacePersistenceState,
    /// Hover-preview state and its edit-save debounce.
    preview_state: PreviewState,
    /// File menu target and its keyboard/window handoff state.
    file_menu_state: FileMenuState,
    /// Stable status and temporary notices shown in the Desktop feedback surface.
    feedback: FeedbackState,
    /// Delayed writes for the active document session.
    document_save: DocumentSaveState,
    /// Startup readiness barrier reported to control clients.
    control_api_state: control_api::ControlApiState,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.handle.clone()
    }
}

impl Workspace {
    pub fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Workspace {
        let explorer = cx.new(FileExplorer::new);
        let search = cx.new(SearchPane::new);
        let switcher = cx.new(QuickSwitcher::new);
        let command_palette = cx.new(CommandPalette::new);
        let chooser = cx.new(Chooser::new);
        let settings = cx.new(SettingsView::new);
        let prompt = cx.new(Prompt::new);
        let copy_results = cx.new(CopyResults::new);
        let pdf_export = cx.new(PdfExport::new);
        let image_export = cx.new(ImageExport::new);
        let release_notes = cx.new(ReadingView::new);
        let reading = cx.new(ReadingView::new);
        let editor = cx.new(|cx| crate::new_editor(window, cx));
        let canvas = cx.new(CanvasView::new);
        let pdf = cx.new(PdfView::new);
        let graph = cx.new(GraphView::new);
        let subscriptions = workspace_subscriptions(
            &explorer,
            &search,
            &switcher,
            &command_palette,
            &chooser,
            &settings,
            &prompt,
            &copy_results,
            &pdf_export,
            &image_export,
            &release_notes,
            &editor,
            &reading,
            &canvas,
            &pdf,
            &graph,
            window,
            cx,
        );
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
            focus: WorkspaceFocusState::new(cx.focus_handle()),
            theme_appearance: ThemeAppearanceState::new(true),
            vault_session: VaultSessionState::new(),
            vault_switcher_state: VaultSwitcherState::default(),
            settings_state: SettingsWindowState::new(settings.clone()),
            explorer,
            search,
            switcher,
            command_palette,
            chooser,
            attachments: attachments::AttachmentState::default(),
            render_requests: RenderRequests::default(),
            prompt: PromptState::new(prompt.clone()),
            copy_results,
            pdf_export,
            image_export,
            release_notes: ReleaseNotesState::new(release_notes),
            overlay: OverlayState::default(),
            command_state: CommandState::default(),
            fold_store: FoldStoreState::default(),
            left_dock: DockState::new(cx.focus_handle(), SIDEBAR_WIDTH as f64),
            graph,
            radial_tree: RadialTreeState::default(),
            active_group_state: ActiveGroupState::new(ViewMode::Reading),
            closed_tab_history: groups::ClosedTabHistory::default(),
            groups,
            active_group: 0,
            layout_tree: desktop_split(SplitDirection::Vertical, vec![Layout::Group(0)]),
            windows: Vec::new(),
            main_window: MainWindowState::new(window.window_handle()),
            group_registry: GroupRegistryState::new(),
            tab_drag_state: TabDragState::default(),
            divider_drag_state: DividerDragState::default(),
            title_editing: TitleEditingState::new(cx.focus_handle()),
            layout_persistence: WorkspacePersistenceState::default(),
            preview_state: PreviewState::default(),
            file_menu_state: FileMenuState::new(cx),
            feedback: FeedbackState::new("Pass a vault folder as the first argument"),
            document_save: DocumentSaveState::default(),
            control_api_state: control_api::ControlApiState::default(),
            _subscriptions: subscriptions,
        };
        if let Some(root) = root {
            ws.load_vault(root, window, cx);
        }
        ws
    }

    /// The title the desktop shows, as the last paint set it: empty until
    /// the first frame has run.
    pub fn window_title(&self) -> SharedString {
        self.main_window.title()
    }

    /// What the last command said, which is how every command reports:
    /// the newest notice while one is up, the status bar otherwise.
    pub fn status_text(&self) -> String {
        self.feedback.status_text()
    }

    /// The open vault's folder, if one is open.
    pub fn vault_root(&self) -> Option<PathBuf> {
        self.vault_session.state.as_ref().map(|s| s.root.clone())
    }

    // ----- rendering -------------------------------------------------------------
}

fn debug_status() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TFT_DEBUG_STATUS").is_some())
}

/// The `s` a count needs, for the notices that carry one.
fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Enter and Escape in the view header's title field.
#[cfg(test)]
mod tests;
