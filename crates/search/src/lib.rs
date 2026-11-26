//! Obsidian's search grammar, matcher, fuzzy search and collation, shared by
//! the Search view, graph and pickers. [`Query::explain`] is the grammar's
//! acceptance test.

pub mod collate;
pub mod exclude;
pub mod fuzzy;
pub mod matcher;
pub mod query;
pub mod snippet;

pub use collate::natural_cmp;
pub use exclude::{ExcludeFilters, Offer};
pub use fuzzy::{fuzzy_match, simple_search, FuzzyMatch};
pub use matcher::{inputs, MatchSet, Search, SearchError, SearchFile, SearchInputs, TaskLine};
pub use query::{parse, Compare, Field, Query};
pub use snippet::{context_window, match_lines, MatchLine};
