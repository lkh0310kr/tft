//! Desktop Workspace actions initiated from a canvas.

use super::{MainView, Modal, Workspace};
use crate::canvas::FilePick;
use crate::chooser::Row as ChooserRow;
use gpui::{Context, Window};
use tft_cache::basename;
use tft_formats::FileKind;

/// What the chooser is open for: a vault file for a canvas card, or a group to jump to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChooserPurpose {
    CanvasFile,
    CanvasGroup,
}

impl Workspace {
    /// "Add note from vault" and "Add media from vault": the chooser over
    /// the vault's notes, or its images, PDFs, audio and video; the pick
    /// becomes a file card at the middle of the canvas.
    pub(super) fn open_canvas_file_chooser(
        &mut self,
        kind: FilePick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let wanted = |k: FileKind| match kind {
            FilePick::Note => k == FileKind::Markdown,
            crate::canvas::FilePick::Media => matches!(
                k,
                FileKind::Image | FileKind::Pdf | FileKind::Audio | FileKind::Video
            ),
        };
        let mut rows: Vec<ChooserRow> = state
            .vault()
            .files()
            .iter()
            .filter(|f| wanted(f.kind))
            .map(|f| ChooserRow {
                label: match kind {
                    FilePick::Note => basename(&f.path).to_string(),
                    FilePick::Media => f.path.clone(),
                },
                value: f.path.clone(),
                flair: None,
            })
            .collect();
        rows.sort_by(|a, b| a.label.cmp(&b.label));
        let (placeholder, empty) = match kind {
            FilePick::Note => ("Type name of a note...", "No notes in the vault"),
            FilePick::Media => ("Type name of a file...", "No media in the vault"),
        };
        self.overlay.modal = Modal::Chooser(ChooserPurpose::CanvasFile);
        self.chooser
            .update(cx, |c, cx| c.open(rows, placeholder, empty, window, cx));
        cx.notify();
    }

    /// `canvas:jump-to-group`: a chooser over the open canvas's labelled
    /// groups; the pick is framed and selected. Obsidian hides the command
    /// when no canvas is active; here it says so.
    pub(super) fn jump_to_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Canvas {
            self.report("Canvas: Jump to group needs an open canvas", cx);
            return;
        }
        let rows: Vec<ChooserRow> = self
            .active_canvas()
            .read(cx)
            .group_targets()
            .into_iter()
            .map(|(label, id)| ChooserRow {
                label,
                value: id,
                flair: None,
            })
            .collect();
        self.overlay.modal = Modal::Chooser(ChooserPurpose::CanvasGroup);
        self.chooser.update(cx, |c, cx| {
            c.open(
                rows,
                "Type name of a group...",
                "No groups in this canvas",
                window,
                cx,
            )
        });
        cx.notify();
    }

    /// `canvas:convert-to-file`: the selected text card's text becomes a
    /// note named after its first line (`Untitled` when blank) beside the
    /// canvas, and the card embeds it. Obsidian asks for the name in a
    /// dialog first; here the first line names it.
    pub(super) fn convert_card_to_file(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        if self.main_view(cx) != MainView::Canvas {
            self.report("Canvas: Convert to file needs a selected text card", cx);
            return;
        }
        let Some((id, text)) = self.active_canvas().read(cx).selected_text_card() else {
            self.report("Canvas: Convert to file needs a selected text card", cx);
            return;
        };
        let folder = self
            .active_group_state
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/'))
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let first = text
            .lines()
            .map(|l| l.trim().trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .unwrap_or("Untitled");
        let name: String = first
            .chars()
            .filter(|c| {
                !matches!(
                    c,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#' | '^'
                )
            })
            .take(60)
            .collect::<String>()
            .trim()
            .to_string();
        let name = if name.is_empty() {
            "Untitled".to_string()
        } else {
            name
        };
        let path = state.vault().available_path(folder, &name, "md");
        match state.create_note(&path, &text) {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.active_canvas()
                    .update(cx, |c, cx| c.convert_to_file(&id, &path, cx));
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }
}
