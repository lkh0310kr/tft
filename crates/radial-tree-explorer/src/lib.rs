mod shot;
mod text;

pub mod engine;
pub mod scenario;
pub mod tree_view;

pub use tree_view::RadialTreeEvent;
pub use tree_view::{RadialTree, ThemeMode, TreeViewOptions};

/// Register the font used by `RadialTree` with the host application's text system.
pub fn register_fonts(text_system: &gpui::TextSystem) {
    text::register_fonts(text_system);
}
