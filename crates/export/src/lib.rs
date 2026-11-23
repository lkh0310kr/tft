//! Export to PDF and canvas to image, without gpui: shared layout over
//! krilla and tiny-skia, with the app's Inter and KaTeX fonts.

pub mod canvas;
pub mod doc;
pub mod fonts;
pub mod layout;
pub mod pdf;
mod raster;

pub use canvas::{
    estimate_text, estimated_size, export_image, render_image, Area, CanvasImage, CanvasPalette,
    CardContent, ImageOptions,
};
pub use doc::{Align, Block, Color, Inline, ListItem, Marker, Palette, SpanStyle};
pub use fonts::{Face, FontFiles, Fonts};
pub use pdf::{export_pdf, paginate, Margin, PageSize, PdfSettings};

#[cfg(test)]
mod canvas_tests;
#[cfg(test)]
mod tests;
