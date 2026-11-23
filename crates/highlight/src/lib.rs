//! Prism's tokenizer and grammars, so token names match Obsidian's. A
//! language with no ported grammar renders plain.

pub mod compiled;
pub mod engine;
pub mod grammar;
pub mod hooks;
pub mod languages;
pub mod palette;

pub use engine::{spans, to_html, tokenize, Node, Span, Token};
pub use hooks::tokenize_language;
pub use languages::{grammar as grammar_for, LANGUAGES};
pub use palette::{style_of, Paint, Style};

/// A run of code to paint: a byte range and how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

/// The runs to paint `code` with, for the fence's info string.
///
/// The info string is resolved the way Obsidian resolves it: the first
/// word, lowercased ([[Specification#Observed code block copy behaviour]]),
/// and a name no grammar is ported for gives no runs.
///
/// Runs are returned **flat and in painting order**: a nested token comes
/// after the one that holds it, so a painter that applies them in sequence
/// ends on the innermost, which is how the cascade resolves a nested span.
/// Runs whose style is [`Paint::Normal`] with no emphasis are dropped — they
/// would paint what the block already paints.
pub fn highlight(code: &str, info: Option<&str>) -> Vec<Run> {
    let Some(nodes) = language_of(info).and_then(|n| hooks::tokenize_language(code, &n)) else {
        return Vec::new();
    };
    engine::spans(&nodes)
        .into_iter()
        .map(|s| Run {
            start: s.start,
            end: s.end,
            style: palette::style_of(s.kind, s.alias),
        })
        .filter(|r| r.style.paint != palette::Paint::Normal || r.style.bold || r.style.italic)
        .collect()
}

/// The language a fence's info string names: its first word, lowercased.
///
/// Obsidian takes the first word and lowercases it, and a word holding
/// anything but `[A-Za-z0-9_-]` does not resolve at all — `c#` and `C++`
/// are `language-none` (probed 2026-10-29, [[Specification#Observed code
/// block copy behaviour, Obsidian 1.14.5]]).
pub fn language_of(info: Option<&str>) -> Option<String> {
    let word = info?.split_whitespace().next()?;
    if word.is_empty()
        || !word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(word.to_ascii_lowercase())
}
