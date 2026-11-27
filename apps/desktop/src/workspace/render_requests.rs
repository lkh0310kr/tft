//! UI requests received without a `Window` and consumed by the next Workspace render.

use crate::canvas::FilePick;
use crate::editor::{Editor, IncomingAttachments};
use gpui::Entity;

#[derive(Default)]
pub(super) struct RenderRequests {
    canvas_pick: Option<FilePick>,
    title_rename: Option<String>,
    slash: Vec<(Entity<Editor>, Option<String>)>,
    attachments: Vec<(Entity<Editor>, IncomingAttachments)>,
}

impl RenderRequests {
    pub(super) fn request_canvas_pick(&mut self, pick: FilePick) {
        self.canvas_pick = Some(pick);
    }

    pub(super) fn take_canvas_pick(&mut self) -> Option<FilePick> {
        self.canvas_pick.take()
    }

    pub(super) fn request_title_rename(&mut self, title: String) {
        self.title_rename = Some(title);
    }

    pub(super) fn take_title_rename(&mut self) -> Option<String> {
        self.title_rename.take()
    }

    pub(super) fn request_slash(&mut self, editor: Entity<Editor>, command: Option<String>) {
        self.slash.push((editor, command));
    }

    pub(super) fn take_slash(&mut self) -> Vec<(Entity<Editor>, Option<String>)> {
        std::mem::take(&mut self.slash)
    }

    pub(super) fn request_attachments(
        &mut self,
        editor: Entity<Editor>,
        attachments: IncomingAttachments,
    ) {
        self.attachments.push((editor, attachments));
    }

    pub(super) fn take_attachments(&mut self) -> Vec<(Entity<Editor>, IncomingAttachments)> {
        std::mem::take(&mut self.attachments)
    }
}
