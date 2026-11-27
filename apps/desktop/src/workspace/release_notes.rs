//! "Show release notes" (`app:show-release-notes`): a tab of view type
//! `release-notes` at the end of the strip, as Obsidian 1.15.0 opens one
//! ([[Specification#Show release notes]]): state `{"currentVersion": …}`,
//! icon `lucide-book-up`, titled `Release Notes <version>`.
//!
//! The version is TFT's and so are the notes. Obsidian fetches its
//! changelog from obsidian.md each time the view opens; TFT fetches
//! nothing. Its notes are the release page shipped beside this module
//! (`releases.html`), compiled in, so the view works offline. The page's
//! section for the version is turned into Markdown here — headings and
//! paragraphs, the inline `code`, links and emphasis — and drawn by the
//! reading view through the one parser.
//!
//! A tab is a release-notes tab while it holds the leaf and nothing else:
//! no file and no history. Opening a note in it makes it a note's tab, as
//! it does in Obsidian, and the leaf written for it is then the note's.

use super::{leaf_id, Workspace};
use crate::reading::{self, ReadingView};
use gpui::{Context, Entity};
use serde_json::{json, Value};
use std::cell::RefCell;
use tft_app::{Tab, ViewMode};

/// Dedicated reader entity and its currently loaded release version.
pub(super) struct ReleaseNotesState {
    pub(super) view: Entity<ReadingView>,
    built: RefCell<Option<String>>,
}

impl ReleaseNotesState {
    pub(super) fn new(view: Entity<ReadingView>) -> Self {
        Self {
            view,
            built: RefCell::new(None),
        }
    }
}

/// The leaf's view type.
pub(super) const VIEW_TYPE: &str = "release-notes";

/// The leaf's icon (observed). Only the tests build a leaf now; the app
/// restores release-note leaves from `workspace.json`.
#[cfg(test)]
const ICON: &str = "lucide-book-up";

/// The version this build reports, the workspace manifest's.
pub(super) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The release page shipped with the app, which the notes are cut from.
const RELEASES_PAGE: &str = include_str!("releases.html");

/// Where the published release page lives.
const RELEASES_URL: &str = "https://tft.cc/releases/";

/// The tab's title and the leaf's: `Release Notes 1.15.0` in Obsidian.
pub(super) fn title(version: &str) -> String {
    format!("Release Notes {version}")
}

/// A new release-notes leaf in Obsidian's shape.
#[cfg(test)]
pub(super) fn leaf(version: &str) -> Value {
    json!({
        "id": leaf_id(),
        "type": "leaf",
        "state": {
            "type": VIEW_TYPE,
            "state": {"currentVersion": version},
            "icon": ICON,
            "title": title(version),
        },
    })
}

impl Workspace {
    /// The version the active tab shows release notes for, if it is a
    /// release-notes tab. Its slot holds the leaf; the live fields hold
    /// the rest.
    pub(super) fn active_release_notes(&self) -> Option<String> {
        let mut tab = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)?
            .clone();
        tab.path = self.active_group_state.current.clone();
        tab.mode = self.active_group_state.mode;
        tab.history = self.active_group_state.history.clone();
        version_of(&tab)
    }

    /// The reading view that draws the notes, built for `version` if it
    /// holds another's. One view serves every group: the notes are the
    /// same text wherever they are shown.
    pub(super) fn release_notes_view(
        &self,
        version: &str,
        cx: &mut Context<Self>,
    ) -> Entity<ReadingView> {
        let view = self.release_notes.view.clone();
        if self.release_notes.built.borrow().as_deref() != Some(version) {
            if let Some(state) = self.vault_session.state.clone() {
                let text = notes(version);
                let model = reading::build_with("", &text, state.as_ref(), state.build_options());
                view.update(cx, |r, cx| r.set_note(None, model, cx));
                *self.release_notes.built.borrow_mut() = Some(version.to_string());
            }
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::super::{leaf_json, tab_from_leaf};
    use super::*;
    use tft_app::HistoryEntry;

    #[test]
    fn a_new_leaf_is_references_shape_with_tfts_version() {
        let leaf = leaf("0.5.0");
        assert_eq!(leaf["type"], json!("leaf"));
        assert!(leaf["id"].as_str().is_some_and(|id| id.len() == 16));
        assert_eq!(
            leaf["state"],
            json!({
                "type": "release-notes",
                "state": {"currentVersion": "0.5.0"},
                "icon": "lucide-book-up",
                "title": "Release Notes 0.5.0",
            })
        );
        let keys: Vec<&String> = leaf["state"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["type", "state", "icon", "title"]);
    }

    #[test]
    fn a_tab_is_release_notes_only_while_it_holds_nothing_else() {
        let mut tab = Tab::empty(ViewMode::LivePreview);
        assert_eq!(version_of(&tab), None);
        tab.leaf = Some(leaf("0.5.0"));
        assert_eq!(version_of(&tab).as_deref(), Some("0.5.0"));
        assert_eq!(tab.title(), "Release Notes 0.5.0");
        // A note opened in it makes it the note's tab.
        let mut opened = tab.clone();
        opened.path = Some("Note.md".into());
        opened.history = vec![HistoryEntry::new("Note.md".into(), ViewMode::LivePreview)];
        assert_eq!(version_of(&opened), None);
        assert_eq!(opened.title(), "Note");
        // A leaf Obsidian wrote names Obsidian's version, and the tab says so.
        let mut theirs = tab.clone();
        theirs.leaf = Some(json!({
            "id": "0123456789abcdef",
            "type": "leaf",
            "state": {"type": "release-notes", "state": {"currentVersion": "1.15.0"}},
        }));
        assert_eq!(theirs.title(), "Release Notes 1.15.0");
    }

    /// A restored release-notes leaf is a tab, and is written back as it
    /// was read.
    #[test]
    fn a_release_notes_leaf_round_trips_through_workspace_json() {
        let leaf = json!({
            "id": "4d2f1c0b9a8e7d6c",
            "type": "leaf",
            "state": {
                "type": "release-notes",
                "state": {"currentVersion": "0.5.0"},
                "icon": "lucide-book-up",
                "title": "Release Notes 0.5.0",
            },
        });
        let (id, tab) = tab_from_leaf(&leaf, ViewMode::LivePreview, &|_| true).unwrap();
        assert_eq!(id, "4d2f1c0b9a8e7d6c");
        assert_eq!(version_of(&tab).as_deref(), Some("0.5.0"));
        assert_eq!(leaf_json(&tab), leaf);
        // And a new one, written and read again.
        let mut fresh = Tab::empty(ViewMode::Reading);
        fresh.leaf = Some(super::leaf(VERSION));
        let written = leaf_json(&fresh);
        assert_eq!(written["state"]["type"], json!("release-notes"));
        let (_, again) = tab_from_leaf(&written, ViewMode::Reading, &|_| true).unwrap();
        assert_eq!(leaf_json(&again), written);
    }

    /// The compiled-in page has a section for this very build, so the
    /// view never falls back on a tagged release.
    #[test]
    fn the_releases_page_has_notes_for_this_build() {
        let notes = notes(VERSION);
        let first = notes.lines().next().unwrap();
        assert!(
            first.starts_with(&format!("# {VERSION} ")),
            "the releases page has a section for {VERSION}: {first}"
        );
        assert!(!notes.contains('<'), "no tag left: {notes}");
        assert!(notes.ends_with("[the releases page](https://tft.cc/releases/).\n"));
    }

    #[test]
    fn a_section_becomes_headings_and_paragraphs() {
        let page = r#"<section class="hero"><h1>Releases</h1><p>Intro</p></section>
<section class="band">
  <h2>1.2.0 — a test</h2>
  <p class="lede">
    Released 1 May. Uses <code>a_b</code> and
    <a href="/docs/">the manual</a>, <strong>bold</strong>.
  </p>
  <div class="grid"><div class="card"><span class="k">K</span>
    <h3>Card <code>x</code></h3>
    <p>Body with [[brackets]] and $5 and ==mark==.</p>
  </div></div>
  <pre>not a paragraph</pre>
</section>"#;
        assert_eq!(
            notes_from(page, "1.2.0"),
            "# 1.2.0 — a test\n\
             \n\
             Released 1 May. Uses `a_b` and [the manual](https://tft.cc/docs/), **bold**.\n\
             \n\
             ## Card `x`\n\
             \n\
             Body with \\[\\[brackets\\]\\] and \\$5 and \\=\\=mark\\=\\=.\n\
             \n\
             ---\n\
             \n\
             Every release is listed on [the releases page](https://tft.cc/releases/).\n"
        );
        // `1.2` does not name `1.2.0`'s section.
        assert!(notes_from(page, "1.2").starts_with("# Release Notes 1.2\n"));
    }

    #[test]
    fn a_version_in_the_table_only_is_its_row() {
        let page = r#"<table><tbody>
<tr><td><strong>0.4.0</strong></td><td>5 Nov 2026</td><td>Bases and
<code>PDFs</code>.</td><td><a href="https://example.com/v0.4.0">v0.4.0</a></td></tr>
</tbody></table>"#;
        assert_eq!(
            notes_from(page, "0.4.0"),
            "# 0.4.0\n\nReleased 5 Nov 2026. Bases and `PDFs`.\n\n\
             [v0.4.0](https://example.com/v0.4.0)\n\
             \n---\n\nEvery release is listed on [the releases page](https://tft.cc/releases/).\n"
        );
    }

    #[test]
    fn an_unknown_version_says_the_page_has_no_notes() {
        let notes = notes_from("<section><h2>0.1.0</h2></section>", "9.9.9");
        assert!(notes
            .starts_with("# Release Notes 9.9.9\n\nThe releases page has no notes for 9.9.9.\n"));
    }

    #[test]
    fn code_keeps_its_characters_and_decodes_entities() {
        assert_eq!(
            inline_markdown("<code>ragtux &lt;r@x&gt;</code>"),
            "`ragtux <r@x>`"
        );
        assert_eq!(inline_markdown("<code>a`b</code>"), "`` a`b ``");
    }
}
