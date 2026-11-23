//! The editor's text model without gpui: buffer, line index, selections,
//! undo and edit plans. [`TextBuffer`] remembers the file's line endings and
//! BOM, so a save changes only edited bytes.

pub mod autopair;
pub mod bidi;
mod buffer;
pub mod find;
pub mod format;
pub mod gutter;
pub mod lists;
mod plan;
pub mod table;
pub mod wrap;

pub use buffer::{
    char_class, next_grapheme_boundary, prev_grapheme_boundary, Cursor, Cursors, EditKind,
    FileText, Sel, TextBuffer,
};
pub use format::BlockInsert;
pub use format::{LinePrefix, LinkSyntax};
pub use plan::{indent_plan, newline_plan, shifted};
