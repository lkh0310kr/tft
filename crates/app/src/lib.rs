//! TFT application state and framework-independent shared services.

#[cfg(feature = "gpui-ui")]
pub mod assets;
#[cfg(feature = "gpui-ui")]
pub mod canvas;
#[cfg(feature = "gpui-ui")]
pub mod editor;
pub mod effect;
pub mod file_switcher;
#[cfg(feature = "gpui-ui")]
pub mod fixed;
#[cfg(feature = "gpui-ui")]
pub mod fold;
#[cfg(feature = "gpui-ui")]
pub mod gutter;
#[cfg(feature = "gpui-ui")]
pub mod inline_field;
#[cfg(feature = "gpui-ui")]
pub mod keymap;
#[cfg(feature = "gpui-ui")]
pub mod math;
#[cfg(feature = "gpui-ui")]
pub mod media;
#[cfg(feature = "gpui-ui")]
pub mod mermaid;
#[cfg(feature = "gpui-ui")]
pub mod pdf;
#[cfg(feature = "gpui-ui")]
pub mod preview;
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
pub use tft_domain::{
    BackendInstanceId, BackendObjectId, FileMeta, InvariantError, LookupError, NameConflict,
    NodeId, NodeKind, RenameError, ScannedNode, VaultData, VaultEvent, VaultNode,
};
pub use workspace::{
    current_history_entry_mut, history_step, place_new_tab, push_history_entry,
    FileOperationRequest, HistoryDirection, HistoryEntry, Layout, LeftTab, SplitDirection, Tab,
    TabGroup, ViewMode, WorkspaceModel,
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
pub mod vault;
