//! TFT application state and framework-independent shared services.

#[cfg(feature = "gpui-ui")]
pub mod assets;
#[cfg(feature = "gpui-ui")]
pub mod canvas;
#[cfg(feature = "gpui-ui")]
pub mod command_palette;
#[cfg(feature = "gpui-ui")]
pub mod editor;
pub mod effect;
pub mod file_switcher;
pub mod file_tree_menu;
#[cfg(feature = "gpui-ui")]
pub mod fixed;
#[cfg(feature = "gpui-ui")]
pub mod fold;
pub mod graph_data;
pub mod graph_layout;
#[cfg(feature = "gpui-ui")]
pub mod gutter;
#[cfg(feature = "gpui-ui")]
pub mod inline_field;
#[cfg(feature = "gpui-ui")]
pub mod keymap;
pub mod link;
#[cfg(feature = "gpui-ui")]
pub mod math;
#[cfg(feature = "gpui-ui")]
pub mod media;
#[cfg(feature = "gpui-ui")]
pub mod mermaid;
pub mod page_preview;
#[cfg(feature = "gpui-ui")]
pub mod pdf;
#[cfg(feature = "gpui-ui")]
pub mod preview;
#[cfg(feature = "gpui-ui")]
pub mod prompt;
#[cfg(feature = "gpui-ui")]
pub mod properties;
#[cfg(feature = "gpui-ui")]
pub mod reading;
#[cfg(feature = "gpui-ui")]
pub mod rtl;
#[cfg(feature = "gpui-ui")]
#[cfg(feature = "gpui-ui")]
pub mod suggest;
#[cfg(feature = "gpui-ui")]
pub mod text_input;
#[cfg(feature = "gpui-ui")]
pub mod theme;
#[cfg(feature = "gpui-ui")]
pub mod title_field;
#[cfg(feature = "gpui-ui")]
pub mod ui;
pub mod workspace;

pub use effect::VaultEffect;
pub use link::{
    resolve_internal_link, resolve_link_navigation, InternalLinkResolution, LinkNavigation,
};
pub use tft_domain::{
    BackendInstanceId, BackendObjectId, FileMeta, InvariantError, LookupError, NameConflict,
    NodeId, NodeKind, RenameError, ScannedNode, VaultData, VaultEvent, VaultNode,
};
pub use workspace::{
    current_history_entry_mut, cycle_tab_index, document_history_state, history_step,
    insert_tab_at_boundary, normalized_split_position, place_new_tab, push_history_entry,
    selection_step, split_dimensions_at, subpath_offset, tab_close_action_available,
    tab_close_action_keeps, transfer_tab_between_strips, update_history_entry_state,
    ClosedTabHistory, FileOperationRequest, HistoryDirection, HistoryEntry, Layout, LeftTab,
    SplitDirection, Tab, TabCloseAction, TabDragSource, TabGroup, TabTransferOutcome, ViewMode,
    WorkspaceDockState, WorkspaceModel,
};

pub use tft_cache as cache;
pub use tft_canvas as canvas_core;
pub use tft_commands as commands;
pub use tft_formats as formats;
pub use tft_highlight as highlight;
pub use tft_markdown as markdown;
#[cfg(feature = "math")]
pub use tft_math as math_core;
#[cfg(feature = "mermaid")]
pub use tft_mermaid as mermaid_core;
pub use tft_paste as paste;
#[cfg(feature = "pdf")]
pub use tft_pdf as pdf_core;
pub use tft_search as search;
pub use tft_suggest as suggest_core;
pub use tft_text as text;

pub mod design;
pub mod explorer;
pub mod remote;
pub mod search_results;
pub mod vault;
