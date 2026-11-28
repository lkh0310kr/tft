//! Desktop Workspace focus and typed-text routing.

use super::{FocusExplorer, MainView, Modal, Workspace};
use gpui::{App, Context, FocusHandle, Focusable, Window};

/// The Workspace root focus handle and a pending request to refocus its main view.
pub(super) struct WorkspaceFocusState {
    pub(super) handle: FocusHandle,
    pub(super) refocus_main: bool,
}

impl WorkspaceFocusState {
    pub(super) fn new(handle: FocusHandle) -> Self {
        Self {
            handle,
            refocus_main: false,
        }
    }
}

impl Workspace {
    /// Tab from the main area: the keyboard goes into the Files tree, the
    /// way Zed's sidebar takes a tab stop. The tree's own Tab binding hands
    /// it back. Nothing happens with the sidebar hidden or a modal up.
    pub(super) fn focus_explorer_action(
        &mut self,
        _: &FocusExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay.modal != Modal::None
            || self.file_menu_state.menu.is_some()
            || !self.left_dock.visible
        {
            return;
        }
        self.leave_search_field(window, cx);
        self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
        cx.notify();
    }

    /// Asks for the keyboard to go back to the main area at the next paint. Most callers
    /// have no `Window`, so they record the request and `render` carries it out.
    pub(super) fn request_focus_main(&mut self, cx: &mut Context<Self>) {
        self.focus.refocus_main = true;
        cx.notify();
    }

    /// Whether the keyboard is somewhere nothing can use it: the workspace root (where a
    /// dock click leaves it), no element, or a main view the area is not painting. Anything
    /// else holds it on purpose and must keep it.
    pub(super) fn keyboard_is_parked(&self, window: &Window, cx: &App) -> bool {
        if window.focused(cx).is_none() || self.focus.handle.is_focused(window) {
            return true;
        }
        // The dock's tree handle is out of the dispatch tree once its pane is hidden or
        // another tab took the dock.
        let painted = self.main_view(cx);
        let graph_focus = self
            .radial_tree
            .active
            .then(|| self.radial_tree.focus.clone())
            .flatten()
            .unwrap_or_else(|| self.graph.focus_handle(cx));
        let unpainted =
            |view: MainView, handle: &FocusHandle| painted != view && handle.is_focused(window);
        if unpainted(
            MainView::Editor,
            self.active_editor().read(cx).focus_handle_ref(),
        ) || unpainted(MainView::Reading, &self.active_reading().focus_handle(cx))
            || unpainted(MainView::Graph, &graph_focus)
            || unpainted(MainView::Search, &self.search.focus_handle(cx))
            || self
                .terminal_focus(cx)
                .is_some_and(|handle| unpainted(MainView::Terminal, &handle))
            || unpainted(MainView::Canvas, &self.active_canvas().focus_handle(cx))
            || unpainted(MainView::Pdf, &self.active_pdf().focus_handle(cx))
            || unpainted(
                MainView::ReleaseNotes,
                &self.release_notes.view.focus_handle(cx),
            )
        {
            return true;
        }
        // A view of a group that is no longer active is painted, but the keyboard
        // belongs with the active group's note.
        self.groups.iter().enumerate().any(|(ix, g)| {
            ix != self.active_group
                && (g.editor.read(cx).focus_handle_ref().is_focused(window)
                    || g.reading.focus_handle(cx).is_focused(window)
                    || g.canvas.focus_handle(cx).is_focused(window)
                    || g.pdf.focus_handle(cx).is_focused(window))
        })
    }

    /// Puts the keyboard on whatever the main area paints; a handle on an absent view reaches nothing.
    pub(super) fn focus_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.refocus_main = false;
        let handle = match self.main_view(cx) {
            MainView::Reading => self.active_reading().focus_handle(cx),
            MainView::Editor => self.active_editor().focus_handle(cx),
            MainView::Graph => self
                .radial_tree
                .active
                .then(|| self.radial_tree.focus.clone())
                .flatten()
                .unwrap_or_else(|| self.graph.focus_handle(cx)),
            MainView::Search => self.search.focus_handle(cx),
            MainView::Terminal => self
                .terminal_focus(cx)
                .unwrap_or_else(|| self.focus.handle.clone()),
            MainView::Canvas => self.active_canvas().focus_handle(cx),
            MainView::Pdf => self.active_pdf().focus_handle(cx),
            MainView::ReleaseNotes => self.release_notes.view.focus_handle(cx),
            MainView::Empty => self.focus.handle.clone(),
        };
        // gpui's focus is per window, and the active group's views paint in one window:
        // the keyboard goes there. A window cannot be updated from within its own update.
        let target = self.active_window_handle();
        if target == window.window_handle() {
            window.focus(&handle, cx);
        } else {
            target
                .update(cx, |_, window, cx| window.focus(&handle, cx))
                .ok();
        }
    }

    /// Hands the keyboard back to the main area before the left dock stops painting the
    /// search field; focus on an unmounted input would take every key.
    pub(super) fn leave_search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.focus_handle(cx).is_focused(window) {
            self.focus_main(window, cx);
        }
    }

    /// Where typed text would land: "editor", "input" or "none". Deciding
    /// is separate from doing because only one of the two routes may run
    /// with this workspace borrowed: delivering a keystroke to a focused
    /// input re-enters the focused view, and that view can be this one.
    pub fn type_target(&self, window: &Window, cx: &App) -> &'static str {
        Self::typed_text_target(self.focus_name(window, cx), self.main_view(cx))
    }

    /// Which of the two routes `text` takes, for the focus the shell
    /// reports and the view the main area is painting.
    ///
    /// The editor is answered only while it is on screen. Its handle can
    /// keep the focus after the main area has stopped painting it — the
    /// keyboard reaches nothing from there, but this route does not go
    /// through the dispatch tree, and would otherwise fill a buffer that
    /// belongs to no file with text nothing ever saves.
    pub(super) fn typed_text_target(focus: &str, painted: MainView) -> &'static str {
        match focus {
            // A property row, a table cell or the inline title is the
            // editor's own field.
            "editor" | "property-field" | "cell-field" | "title-field" => match painted {
                MainView::Editor => "editor",
                _ => "none",
            },
            "canvas-editor" => match painted {
                MainView::Canvas => "canvas-editor",
                _ => "none",
            },
            // The pane is on screen whatever the main area paints, and its
            // field is the editor's, in reading view too.
            "file-properties-field" => "editor",
            "workspace" | "reading" | "graph" | "canvas" | "pdf" | "file-properties" | "none" => {
                "none"
            }
            "terminal" => "terminal",
            _ => "input",
        }
    }

    /// Types `text` into the editor, for the socket's `type` method, and
    /// says so. Only the editor is reachable from here: the inputs belong
    /// to the views that own them and are typed into from outside this
    /// borrow, so a caller asks [`Workspace::type_target`] first and
    /// anything but "editor" is answered the other way.
    pub fn type_into_focused(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> &'static str {
        match self.type_target(window, cx) {
            "editor" => {
                // `type_text` already routes to an open property row or
                // table cell rather than the body.
                self.active_editor()
                    .update(cx, |e, cx| e.type_text(text, window, cx));
                "editor"
            }
            "canvas-editor" => {
                if let Some(editor) = self.active_canvas().read(cx).card_editor().cloned() {
                    editor.update(cx, |e, cx| e.type_text(text, window, cx));
                }
                "canvas-editor"
            }
            "terminal" => {
                if let Some(session) = self.terminal.session.as_ref() {
                    session.write(text, window, cx);
                }
                "terminal"
            }
            // Nothing else is reachable from here, so nothing else is
            // typed into: a focused input is answered by the caller, which
            // asked where the text would go before handing it over.
            _ => "none",
        }
    }

    /// A name for whatever has focus, for the state dump and to decide
    /// where typed text goes. Only handles this shell can reach are named
    /// by handle; the find panel's two fields belong to the editor, which
    /// is asked. `explorer` never comes back: the file tree's rows are
    /// painted without a focus handle, so the keyboard is never in it.
    pub fn focus_name(&self, window: &Window, cx: &App) -> &'static str {
        // Settings has a window of its own, so its focus is not this
        // window's to report: that it is up is what the caller is after.
        if self.settings_up(cx) {
            return "settings";
        }
        if window.focused(cx).is_none() {
            return "none";
        }
        if self
            .active_editor()
            .read(cx)
            .focus_handle_ref()
            .is_focused(window)
        {
            // An open property row or table cell takes the editor's keys
            // without taking the focus.
            return match self.active_editor().read(cx).field_kind() {
                Some("cell") => "cell-field",
                Some("title") => "title-field",
                Some(_) => "property-field",
                None => "editor",
            };
        }
        if self.active_reading().focus_handle(cx).is_focused(window) {
            return "reading";
        }
        if self.graph.focus_handle(cx).is_focused(window) {
            return "graph";
        }
        if self
            .active_canvas()
            .read(cx)
            .card_editor()
            .is_some_and(|e| e.read(cx).focus_handle_ref().is_focused(window))
        {
            return "canvas-editor";
        }
        if self.active_canvas().focus_handle(cx).is_focused(window) {
            return "canvas";
        }
        if let Some(field) = self.active_pdf().read(cx).focused_field(window, cx) {
            return field;
        }
        if self.active_pdf().focus_handle(cx).is_focused(window) {
            return "pdf";
        }
        // The view header's inline rename field, which takes typed text
        // through the workspace's own input handler.
        if self.title_editing.focus.is_focused(window) {
            return "view-title";
        }
        if self.switcher.focus_handle(cx).is_focused(window) {
            return "switcher-input";
        }
        if self.prompt.view.focus_handle(cx).is_focused(window) {
            return "prompt-input";
        }
        if self.chooser.focus_handle(cx).is_focused(window) {
            return "chooser-input";
        }
        if self.search.focus_handle(cx).is_focused(window) {
            return "search-input";
        }
        if self
            .terminal_focus(cx)
            .is_some_and(|handle| handle.is_focused(window))
        {
            return "terminal";
        }
        if self.explorer.focus_handle(cx).is_focused(window) {
            return "explorer";
        }
        if self.focus.handle.is_focused(window) {
            return "workspace";
        }
        // Both find fields sit in one key context, so the stack cannot tell
        // them apart; the editor holds the two handles and can.
        if let Some(name) = self.active_editor().read(cx).find_focus(window, cx) {
            return name;
        }
        "unknown"
    }

    /// Focuses one of the main views by name, for the socket's `focus`
    /// method. The mode is left alone: focus is all that is asked for.
    pub fn focus_view(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // The tree is in the left dock, not the main area: showing the
        // dock's Files tab is what puts its handle on screen.
        if name == "explorer" {
            self.leave_search_field(window, cx);
            self.left_dock.set_visible(true);
            self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
            cx.notify();
            return Ok(());
        }
        match Self::focus_target(name, self.main_view(cx))? {
            // The root div paints in every frame, so its handle is always
            // somewhere the keyboard can sit.
            None => window.focus(&self.focus.handle, cx),
            Some(_) => self.focus_main(window, cx),
        }
        cx.notify();
        Ok(())
    }

    /// Which view the socket's `focus` names: `None` for the workspace
    /// root, or the main view asked for once it is the one being painted.
    ///
    /// A view the main area is not painting is refused rather than focused.
    /// Its handle is in no frame's dispatch tree, so the keyboard parked on
    /// it reaches nothing at all — not even the hotkey that would undo it,
    /// which is the dead state the mode switches were hardened against.
    pub(super) fn focus_target(name: &str, painted: MainView) -> Result<Option<MainView>, String> {
        let wanted = match name {
            "workspace" => return Ok(None),
            "editor" => MainView::Editor,
            "reading" => MainView::Reading,
            "graph" => MainView::Graph,
            "search" => MainView::Search,
            "terminal" => MainView::Terminal,
            "canvas" => MainView::Canvas,
            "pdf" => MainView::Pdf,
            "release-notes" => MainView::ReleaseNotes,
            other => return Err(format!("unknown view: {other}")),
        };
        if wanted != painted {
            return Err(format!(
                "the main area is painting {}, not {name}: \
                 switch the mode first, or focus the view that is on screen",
                painted.as_str()
            ));
        }
        Ok(Some(wanted))
    }
}
