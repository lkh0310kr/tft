//! The canvas view: draws a `.canvas` file and edits it.
//!
//! `view` owns the element — viewport, painting of edges and card frames,
//! selection, dragging, resizing, drawing edges, saving through
//! `tft_formats::canvas` — over the model in the `canvas` crate.
//! `cards` builds what is drawn inside a card: a text card's Markdown, a file
//! card's note or image, a link card, a group's label. `minimap` is the
//! picture a canvas embedded in a note is drawn as.

#[cfg(feature = "desktop-native")]
pub mod cards;
#[cfg(feature = "desktop-native")]
pub mod menu;
pub use tft_app::canvas::minimap;
#[cfg(feature = "desktop-native")]
pub mod view;

#[cfg(feature = "desktop-native")]
pub use view::{CanvasEvent, CanvasView, FilePick};
