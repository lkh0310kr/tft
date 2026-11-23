//! Attachment names and clipboard bytes for a paste or drop; pure, no gpui.

pub mod attachment;
pub mod clipboard;

pub use attachment::{joined_embeds, saved_name, split_name, Civil, Incoming};
