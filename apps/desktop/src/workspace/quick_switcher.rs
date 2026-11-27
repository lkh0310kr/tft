//! Workspace transition that opens the note quick switcher.

use super::{Modal, Workspace};
use gpui::{Context, Window};

impl Workspace {
    /// Opens the quick switcher pre-filled with `query`.
    pub fn open_switcher(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut recent: Vec<String> = Vec::new();
        for entry in self.active_group_state.history.iter().rev() {
            if !recent.contains(&entry.path) {
                recent.push(entry.path.clone());
            }
        }
        self.close_editor_suggest(cx);
        self.overlay.modal = Modal::Switcher;
        self.switcher
            .update(cx, |s, cx| s.open(recent, query, window, cx));
        cx.notify();
    }
}
