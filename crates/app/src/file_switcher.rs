//! Filename and path matching shared by file switchers in every host.

use std::ops::Range;

use crate::search::{fuzzy_match, simple_search};

/// Scores a multi-token query against a file label. Exact, prefix, word-start
/// and substring matches outrank fuzzy subsequences. Every whitespace token
/// must match independently so separated words do not become one subsequence.
pub fn label_match(query: &str, label: &str) -> Option<(f64, Vec<Range<usize>>)> {
    let mut score = 0.0;
    let mut ranges = Vec::new();
    for token in query.split_whitespace() {
        let (token_score, mut token_ranges) = token_match(token, label)?;
        score += token_score;
        ranges.append(&mut token_ranges);
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    Some((score, merged))
}

fn token_match(token: &str, label: &str) -> Option<(f64, Vec<Range<usize>>)> {
    if let Some(ranges) = simple_search(token, label) {
        let range = ranges.first()?;
        if label.eq_ignore_ascii_case(token) {
            return Some((100_000.0, ranges));
        }
        if range.start == 0 {
            return Some((80_000.0 - label.chars().count() as f64, ranges));
        }
        let word_start = label[..range.start]
            .chars()
            .next_back()
            .is_some_and(|previous| !previous.is_alphanumeric());
        if word_start {
            return Some((60_000.0 - range.start as f64, ranges));
        }
        return Some((50_000.0 - range.start as f64, ranges));
    }
    fuzzy_match(token, label).map(|matched| (20_000.0 + matched.score, matched.ranges))
}
