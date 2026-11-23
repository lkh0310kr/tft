//! Editor autocompletion without a window: triggers, candidates and the text
//! an accepted row writes. The GPUI popup only renders [`Suggestion`]s.
//!
//! One line of text, one query, three answers:
//!
//! ```
//! # use tft_suggest::{trigger, Kind};
//! let t = trigger("see [[Alpha#first", 17).expect("a link query");
//! assert_eq!(t.kind, Kind::Link);
//! assert_eq!(t.query, "Alpha#first");
//! ```

pub mod blocks;
mod candidates;
mod insert;
pub mod slash;
mod trigger;

pub use candidates::{suggestions, BlockId, Item, Suggestion, Vault, LIMIT};
pub use insert::{file_link, insert, Accept, Insertion, Options};
pub use slash::{
    rank_commands, slash_accept_edit, slash_rows, slash_trigger, CommandEntry, SlashTriggered,
};
/// The Excluded files rules a [`Vault`] carries, from `search`.
pub use tft_search::ExcludeFilters;
pub use trigger::{trigger, Kind, Triggered};
