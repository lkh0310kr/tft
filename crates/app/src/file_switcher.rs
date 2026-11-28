//! Filename and path matching shared by file switchers in every host.

use std::cmp::Ordering;
use std::ops::Range;

use crate::search::{fuzzy_match, simple_search};

/// One file or alias offered by a host's switcher. `alias` rows match only
/// their label; file rows may also match a Vault-relative path.
pub struct SwitchCandidate<'a> {
    pub path: &'a str,
    pub label: &'a str,
    pub alias: bool,
    pub downranked: bool,
}

/// A ranked candidate refers to its input index so each host can retain its
/// own row data while sharing all matching and ordering rules.
pub struct RankedMatch {
    pub candidate_index: usize,
    pub score: f64,
    pub ranges: Vec<Range<usize>>,
}

/// Ranks file switcher entries for a query. Empty queries show recent files
/// in their supplied order and omit excluded files; non-empty queries match
/// file names, aliases and file paths, with excluded files sorted last.
pub fn rank_candidates(
    candidates: &[SwitchCandidate<'_>],
    recent: &[String],
    query: &str,
) -> Vec<RankedMatch> {
    if query.is_empty() {
        return recent
            .iter()
            .enumerate()
            .filter_map(|(recent_index, path)| {
                candidates
                    .iter()
                    .position(|candidate| {
                        !candidate.alias && !candidate.downranked && candidate.path == path
                    })
                    .map(|candidate_index| RankedMatch {
                        candidate_index,
                        score: -(recent_index as f64),
                        ranges: Vec::new(),
                    })
            })
            .collect();
    }

    let mut results = candidates
        .iter()
        .enumerate()
        .filter_map(|(candidate_index, candidate)| {
            let by_label = label_match(query, candidate.label);
            let by_path = (!candidate.alias)
                .then(|| label_match(query, candidate.path))
                .flatten()
                .map(|(score, _)| (10_000.0 + 9_000.0 * score / (score + 50_000.0), Vec::new()));
            let best = match (by_label, by_path) {
                (Some((label_score, ranges)), Some((path_score, _)))
                    if label_score >= path_score =>
                {
                    Some((label_score, ranges))
                }
                (_, Some((path_score, ranges))) => Some((path_score, ranges)),
                (Some((label_score, ranges)), None) => Some((label_score, ranges)),
                (None, None) => None,
            }?;
            let score = best.0 - if candidate.alias { 500.0 } else { 0.0 };
            Some((
                candidate_index,
                score,
                best.1,
                candidate.label,
                candidate.path,
                candidate.downranked,
            ))
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        compare_results(
            ResultKey {
                score: a.1,
                label: a.3,
                path: a.4,
                downranked: a.5,
            },
            ResultKey {
                score: b.1,
                label: b.3,
                path: b.4,
                downranked: b.5,
            },
        )
    });
    results
        .into_iter()
        .map(|(candidate_index, score, ranges, ..)| RankedMatch {
            candidate_index,
            score,
            ranges,
        })
        .collect()
}

/// Whether a file kind is available to the file switcher under the vault's
/// global unsupported-file setting and the switcher's attachment options.
pub fn offers(
    kind: tft_formats::FileKind,
    show_unsupported: bool,
    switcher: &tft_formats::config::SwitcherConfig,
) -> bool {
    match kind {
        tft_formats::FileKind::Other => {
            show_unsupported && switcher.show_all_file_types.unwrap_or(false)
        }
        tft_formats::FileKind::Image
        | tft_formats::FileKind::Audio
        | tft_formats::FileKind::Video
        | tft_formats::FileKind::Pdf => switcher.show_attachments.unwrap_or(true),
        tft_formats::FileKind::Markdown | tft_formats::FileKind::Canvas => true,
    }
}

/// Sort key for filename and alias rows. Excluded files sort last; within
/// either group, stronger matches and shorter labels rank first.
pub struct ResultKey<'a> {
    pub score: f64,
    pub label: &'a str,
    pub path: &'a str,
    pub downranked: bool,
}

pub fn compare_results(a: ResultKey<'_>, b: ResultKey<'_>) -> Ordering {
    a.downranked
        .cmp(&b.downranked)
        .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal))
        .then_with(|| a.label.len().cmp(&b.label.len()))
        .then_with(|| a.path.cmp(b.path))
}

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
