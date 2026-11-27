//! Desktop notices, modal overlays, menu focus handoff, and pane visibility.

use super::notices::{NOTICE_LIFETIME, NoticeAction};
use super::{ChooserPurpose, PendingOp, Workspace};
use crate::prompt::PromptSpec;
use crate::search::CopyEntry;
use crate::theme::Theme;
use gpui::{Context, Focusable, Pixels, SharedString, Window, div, prelude::*, px};

/// The modal overlay currently shown by the Desktop workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Modal {
    None,
    Switcher,
    CommandPalette,
    Prompt,
    Chooser(ChooserPurpose),
    CopyResults,
    Downloads,
    DebugInfo,
    MigrateFrontmatter,
    PdfExport,
    ImageExport,
}

/// Modal selection and the window it is anchored to share one overlay lifecycle.
pub(super) struct OverlayState {
    pub(super) modal: Modal,
    /// `None` while no modal/menu is up; `Some(None)` anchors to the vault window.
    pub(super) window: Option<Option<u64>>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            modal: Modal::None,
            window: None,
        }
    }
}

impl Workspace {
    pub(super) fn open_prompt(
        &mut self,
        op: PendingOp,
        spec: PromptSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt.pending = Some(op);
        self.close_editor_suggest(cx);
        self.overlay.modal = Modal::Prompt;
        self.prompt
            .view
            .update(cx, |p, cx| p.open(spec, window, cx));
        cx.notify();
    }

    /// Shows a message that expires on its own, the way Obsidian's Notice does; the
    /// status bar keeps only what stays true.
    pub(super) fn report(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.report_with(message, None, cx);
    }

    /// A notice's button pressed.
    pub(super) fn run_notice_action(
        &mut self,
        action: NoticeAction,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            NoticeAction::UndoMigration => self.undo_migration(cx),
        }
    }

    /// [`Workspace::report`] with a button after the message.
    pub(super) fn report_with(
        &mut self,
        message: impl Into<SharedString>,
        action: Option<NoticeAction>,
        cx: &mut Context<Self>,
    ) {
        let home = self.window_of_group(self.active_group_id());
        let id = self
            .feedback
            .notices
            .push_with(message.into(), home, action);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_LIFETIME).await;
            this.update(cx, |ws, cx| {
                ws.feedback.notices.expire(id);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The window the modal or file menu that is up draws in, latching it
    /// on the first ask: the active group's window, which is where the keys
    /// or the click that raised it were. A window closed since reads as the
    /// vault window, so an overlay never goes undrawn. A window counts as
    /// there while it has a node and a handle: reading the handle would
    /// fail inside that window's own render, where its root is leased.
    pub(super) fn overlay_home(&mut self) -> Option<u64> {
        if self.overlay.modal == Modal::None && self.file_menu_state.menu.is_none() {
            self.overlay.window = None;
            return None;
        }
        let home = match self.overlay.window {
            Some(home) => home,
            None => {
                let home = self.window_of_group(self.active_group_id());
                self.overlay.window = Some(home);
                home
            }
        };
        home.filter(|id| self.popout_open(*id))
    }

    /// Whether popout `id` is open: it has a node and a handle. Reading
    /// the handle would fail inside that window's own render, where its
    /// root is leased, so the handle stands for the window.
    pub(super) fn popout_open(&self, id: u64) -> bool {
        self.windows
            .iter()
            .any(|w| w.id == id && w.handle.is_some())
    }

    /// The notices window `here` draws, stacked in its bottom-right corner
    /// (`bottom` above the edge: the vault window clears its status bar).
    /// Obsidian reports an outcome with a Notice in the corner rather
    /// than in the status bar, and the probe notes put it bottom right.
    pub(super) fn render_notices(
        &self,
        here: Option<u64>,
        bottom: Pixels,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let open = |id| self.popout_open(id);
        let mut shown = self.feedback.notices.in_window(here, &open).peekable();
        shown.peek()?;
        Some(
            div()
                .absolute()
                .bottom(bottom)
                .right(px(16.))
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .children(shown.map(|notice| {
                    div()
                        .max_w(px(360.))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme.tokens.background_secondary)
                        .border_1()
                        .border_color(theme.tokens.background_modifier_border)
                        .text_size(px(12.))
                        .text_color(theme.tokens.text_normal)
                        .child(notice.text.clone())
                        .children(notice.action.map(|action| {
                            let id = notice.id;
                            div()
                                .id(("notice-action", id))
                                .mt_1()
                                .flex()
                                .justify_end()
                                .cursor_pointer()
                                .text_color(theme.tokens.text_accent)
                                .hover(|s| s.text_color(theme.tokens.text_accent_hover))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.feedback.notices.expire(id);
                                    this.run_notice_action(action, window, cx);
                                }))
                                .child(action.label())
                        }))
                }))
                .into_any_element(),
        )
    }

    /// Hands the keyboard to an open menu, or back from a closed one, in
    /// window `here`: the file menu in the window it draws in, the panes'
    /// sort menus in the vault window, which alone has the panes. A window
    /// the menu is not in leaves the focus alone, or its "closed" would
    /// throw away the other window's place to go back to.
    pub(super) fn sync_menu_focus(
        &mut self,
        here: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = self.overlay_home();
        let open = self.file_menu_state.menu.is_some() && home == here;
        if open {
            self.file_menu_state.window = here;
            self.file_menu_state.focus.sync(true, window, cx);
        } else if self.file_menu_state.window == here {
            self.file_menu_state.focus.sync(false, window, cx);
        }
    }

    /// The modal that is up, centred near the top of window `here` when it
    /// is the window the modal belongs to.
    ///
    /// Every overlay occludes: a modal in Obsidian owns the pointer as
    /// well as the keyboard, and without this the editor's raw window
    /// listeners take the mouse-down that landed on the modal and focus
    /// the note behind it, leaving the modal visible but keyboard-dead.
    /// Occluding costs nothing a modal wants — a click outside still
    /// reaches each list modal's `on_mouse_down_out`, which is bounds
    /// based rather than hitbox based, and Settings closes on its own
    /// backdrop click.
    pub(super) fn render_modal_overlay(&mut self, here: Option<u64>) -> Option<gpui::AnyElement> {
        if self.overlay_home() != here {
            return None;
        }
        match self.overlay.modal {
            Modal::None => None,
            Modal::Switcher
            | Modal::CommandPalette
            | Modal::Prompt
            | Modal::Chooser(_)
            | Modal::Downloads
            | Modal::CopyResults
            | Modal::DebugInfo
            | Modal::MigrateFrontmatter
            | Modal::PdfExport
            | Modal::ImageExport => {
                let child: gpui::AnyElement = match self.overlay.modal {
                    Modal::Switcher => self.switcher.clone().into_any_element(),
                    Modal::CommandPalette => self.command_palette.clone().into_any_element(),
                    Modal::Chooser(_) => self.chooser.clone().into_any_element(),
                    Modal::Downloads => self.downloads_element()?,
                    Modal::CopyResults => self.copy_results.clone().into_any_element(),
                    Modal::DebugInfo => self.debug_info.clone().into_any_element(),
                    Modal::MigrateFrontmatter => self.migrate_element()?,
                    Modal::PdfExport => self.pdf_export.clone().into_any_element(),
                    Modal::ImageExport => self.image_export.clone().into_any_element(),
                    _ => self.prompt.view.clone().into_any_element(),
                };
                Some(
                    div()
                        .occlude()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .items_start()
                        .pt(px(90.))
                        .child(child)
                        .into_any_element(),
                )
            }
        }
    }

    /// A modal going up takes the autocompletion popup down with it: the
    /// popup would otherwise stay painted under the modal, still holding
    /// the keys it binds.
    pub(super) fn close_editor_suggest(&mut self, cx: &mut Context<Self>) {
        self.active_editor().update(cx, |e, cx| e.close_suggest(cx));
    }

    pub(super) fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.modal != Modal::None {
            self.overlay.modal = Modal::None;
            let mode = self.active_group_state.mode;
            self.set_mode(mode, window, cx);
        }
    }

    /// "Copy search results" from the Search pane's count menu.
    pub(super) fn open_copy_results(
        &mut self,
        entries: Vec<CopyEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_editor_suggest(cx);
        self.copy_results.update(cx, |m, cx| m.open(entries, cx));
        self.overlay.modal = Modal::CopyResults;
        window.focus(&self.copy_results.focus_handle(cx), cx);
        cx.notify();
    }
}
