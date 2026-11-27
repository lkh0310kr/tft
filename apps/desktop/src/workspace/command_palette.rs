//! Workspace transition that opens the command palette.

use super::{Modal, OpenCommandPalette, Workspace};
use gpui::{Context, Window};

impl Workspace {
    /// The palette's key binding, `Mod+Shift+P` (`Cmd` on macOS).
    pub(super) fn open_command_palette_action(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_command_palette(window, cx);
    }

    /// Opens the command palette over the commands that can run now, with this
    /// session's recently run ones first while the query is empty. The list is
    /// the slash popup's, so a command hidden by its check callback is hidden
    /// here too.
    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self.slash_commands(window, cx);
        self.close_editor_suggest(cx);
        self.overlay.modal = Modal::CommandPalette;
        self.command_palette.update(cx, |p, cx| {
            p.open(commands.entries, commands.recent, "", window, cx)
        });
        cx.notify();
    }
}
