//! Shared result ordering for Desktop and Web search views.

use crate::explorer::SortOrder;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy)]
pub struct SearchSortKey<'a> {
    pub name: &'a str,
    pub path: &'a str,
    pub modified: u64,
    pub created: u64,
}

/// Matches the native Search ordering: natural basename order with a
/// descending full-path tie break, and stable time ordering without a
/// secondary key.
pub fn compare_search_results(
    a: SearchSortKey<'_>,
    b: SearchSortKey<'_>,
    order: SortOrder,
) -> Ordering {
    match order {
        SortOrder::Alphabetical => {
            tft_search::natural_cmp(a.name, b.name).then_with(|| b.path.cmp(a.path))
        }
        SortOrder::AlphabeticalReverse => {
            tft_search::natural_cmp(b.name, a.name).then_with(|| b.path.cmp(a.path))
        }
        SortOrder::ByModifiedTime => b.modified.cmp(&a.modified),
        SortOrder::ByModifiedTimeReverse => a.modified.cmp(&b.modified),
        SortOrder::ByCreatedTime => b.created.cmp(&a.created),
        SortOrder::ByCreatedTimeReverse => a.created.cmp(&b.created),
    }
}
