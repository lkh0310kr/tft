//! Desktop Workspace actions initiated from a canvas.

use super::{MainView, Modal, Workspace};
use crate::canvas::FilePick;
use crate::chooser::Row as ChooserRow;
use gpui::{Context, Window};
use tft_cache::basename;
use tft_formats::FileKind;

/// What the chooser is open for: a vault file for a canvas card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChooserPurpose {
    CanvasFile,
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
}
