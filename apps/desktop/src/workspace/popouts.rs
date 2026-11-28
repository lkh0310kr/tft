//! Desktop popout window lifecycle and rendering against the shared Workspace state.

use super::tab_drag::TabDrag;
use super::tab_strip::TabTitle;
use super::window_frame::{popout_margin, window_controls, WindowFrame};
use super::window_node::{WindowBounds, WindowNode};
use super::{
    desktop_split, leaf_title, popout_outer_size, popout_placement, Layout, Modal, SplitDirection,
    Tab, ViewMode, Workspace,
};
use crate::popout::PopoutWindow;
use crate::reading::LinkTarget;
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, App, Context, Decorations, DragMoveEvent, Pixels, SharedString, Tiling,
    Window,
};
use serde_json::Value;

impl Workspace {
    pub(super) fn described_title(&self, window: &Window) -> SharedString {
        self.popout_of_window(window.window_handle())
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .map(|w| w.title.clone())
            .unwrap_or_else(|| self.window_title())
    }

    /// The id of the popout `handle` shows, if it is one.
    pub(super) fn popout_of_window(&self, handle: gpui::AnyWindowHandle) -> Option<u64> {
        self.windows
            .iter()
            .find(|w| {
                w.handle
                    .is_some_and(|h| gpui::AnyWindowHandle::from(h) == handle)
            })
            .map(|w| w.id)
    }

    pub(super) fn window_of_group(&self, id: u64) -> Option<u64> {
        self.windows
            .iter()
            .find(|w| w.tree.contains(id))
            .map(|w| w.id)
    }

    /// The layout tree of a popout, or the main area's for `None`. A gone window reads as the main area.
    pub(super) fn tree(&self, window: Option<u64>) -> &Layout {
        window
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .map(|w| &w.tree)
            .unwrap_or(&self.layout_tree)
    }

    pub(super) fn tree_mut(&mut self, window: Option<u64>) -> &mut Layout {
        match window.and_then(|id| self.windows.iter().position(|w| w.id == id)) {
            Some(wix) => &mut self.windows[wix].tree,
            None => &mut self.layout_tree,
        }
    }

    /// Whether the active group is a popout's.
    pub(super) fn active_group_in_popout(&self) -> bool {
        self.window_of_group(self.active_group_id()).is_some()
    }

    /// The window the active group is in: its popout while open, else the main window.
    /// Where the keyboard belongs, and the window the control socket drives.
    pub fn active_window(&self, cx: &App) -> gpui::AnyWindowHandle {
        self.window_of_group(self.active_group_id())
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .filter(|w| w.is_open(cx))
            .and_then(|w| w.handle)
            .map(Into::into)
            .unwrap_or_else(|| self.main_window.handle())
    }

    /// The same without checking the window is up, for code inside that window's own
    /// update, where it reads as closed.
    pub(super) fn active_window_handle(&self) -> gpui::AnyWindowHandle {
        self.window_of_group(self.active_group_id())
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .and_then(|w| w.handle)
            .map(Into::into)
            .unwrap_or_else(|| self.main_window.handle())
    }

    /// Runs `f` with the window the active group is in, which is the one
    /// a caller of the control socket is looking at: `window` itself when
    /// that is it, the popout otherwise (its own update, nested in this
    /// one). The popout gone, `window` stands in.
    pub fn in_active_window<R>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut App) -> R,
    ) -> R {
        let target = self.active_window_handle();
        if target == window.window_handle() {
            return f(self, window, cx);
        }
        let mut f = Some(f);
        match target.update(cx, |_, target, cx| (f.take().unwrap())(self, target, cx)) {
            Ok(result) => result,
            Err(_) => (f.take().unwrap())(self, window, cx),
        }
    }

    // ----- popout windows ----------------------------------------------------------

    /// The desktop gave window `win` the focus: unless its active group is already
    /// there, the most recently used group of it becomes active, as Obsidian's active
    /// leaf follows the front window.
    pub(crate) fn window_activated(&mut self, win: Option<u64>, cx: &mut Context<Self>) {
        if self.window_of_group(self.active_group_id()) == win {
            return;
        }
        let mut in_window = Vec::new();
        self.tree(win).groups(&mut in_window);
        let Some(group) = self
            .group_registry
            .most_recent_matching(|g| in_window.contains(&g))
            .or_else(|| in_window.first().copied())
        else {
            return;
        };
        self.activate_group(group, cx);
        self.focus.refocus_main = true;
        cx.notify();
    }

    /// Popout `id`'s window while it is open, for the control socket.
    pub fn popout_window(&self, id: u64, cx: &App) -> Option<gpui::AnyWindowHandle> {
        self.windows
            .iter()
            .find(|w| w.id == id)
            .filter(|w| w.is_open(cx))
            .and_then(|w| w.handle)
            .map(Into::into)
    }

    /// One popout's content: its groups through the main area's renderers, under the
    /// window buttons, with no ribbon, docks or status bar (see [[Specification#Observed popout windows]]).
    /// `None` once the node is gone, which tells the view to close. Nothing here may `notify`.
    pub(crate) fn render_popout(
        &mut self,
        id: u64,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let wix = self.windows.iter().position(|w| w.id == id)?;
        // The first frame draws while the window is still opening, before the opener
        // records its handle; the window itself is the handle.
        if self.windows[wix].handle.is_none() {
            self.windows[wix].handle = window.window_handle().downcast::<PopoutWindow>();
        }
        // Where the window is for `workspace.json`: the screen origin (always 0, 0 on
        // Wayland) and inner size. The inner size is the surface less `popout_margin`
        // on untiled edges, as the opener grew it.
        let outer = window.window_bounds().get_bounds();
        let viewport = window.viewport_size();
        let tiling = match window.window_decorations() {
            Decorations::Client { tiling } => tiling,
            Decorations::Server => Tiling::default(),
        };
        let margin = |tiled: bool| if tiled { px(0.) } else { popout_margin() };
        let whole = |p: Pixels| f32::from(p).round() as i32;
        let mut width = whole(viewport.width - margin(tiling.left) - margin(tiling.right));
        let mut height = whole(viewport.height - margin(tiling.top) - margin(tiling.bottom));
        // A compositor may briefly see the window with no inset and configure it grown
        // by the margin on every edge (sway does this). Exactly that grow, before
        // anything else changed the size, is put back once.
        let grow = whole(popout_margin() * 2.);
        match self.windows[wix].opened_for {
            Some((w, h)) if (w - width).abs() <= 1 && (h - height).abs() <= 1 => {
                (width, height) = (w, h);
            }
            Some((w, h))
                if grow > 0
                    && matches!(window.window_decorations(), Decorations::Client { .. })
                    && tiling == Tiling::default()
                    && (width - w - grow).abs() <= 1
                    && (height - h - grow).abs() <= 1 =>
            {
                let back = popout_outer_size(w as f32, h as f32);
                window.defer(cx, move |window, _| window.resize(back));
                (width, height) = (w, h);
            }
            Some(_) => self.windows[wix].opened_for = None,
            None => {}
        }
        self.windows[wix].set_bounds(
            WindowBounds {
                x: whole(outer.origin.x),
                y: whole(outer.origin.y),
                width,
                height,
            },
            window.is_maximized(),
        );
        // The title follows the window's own active tab, in the vault window's pattern.
        let title = self.popout_title(wix);
        if self.windows[wix].title != title {
            window.set_window_title(&title);
            self.windows[wix].title = title;
        }
        // The frame is this window's; the main window's render sets these back for itself.
        let frame = WindowFrame::of(window);
        // The keyboard: only an active group of this window is this window's.
        if self.window_of_group(self.active_group_id()) == Some(id)
            && std::mem::take(&mut self.focus.refocus_main)
            && self.overlay.modal == Modal::None
            && self.keyboard_is_parked(window, cx)
        {
            self.focus_main(window, cx);
        }
        self.tab_drag_frame(window, cx);
        // A modal or file menu raised from this window draws and holds the keyboard here.
        self.sync_menu_focus(Some(id), window, cx);
        let overlay = self.render_modal_overlay(Some(id));
        let file_menu = self.render_file_menu(Some(id), theme, window, cx);
        // So do notices that went up while it was active; it has no status bar to clear.
        let notices = self.render_notices(Some(id), px(16.), theme, cx);
        let preview = self.render_hover_preview(Some(id), theme, window, cx);
        let tree = self.windows[wix].tree.clone();
        let body = self.render_layout(&tree, &mut Vec::new(), Some(id), frame, theme, cx);
        let corners = frame.corners;
        // macOS has the desktop's traffic lights instead. The close button closes the
        // window's groups, then the window, from inside its own update.
        let controls = (frame.frameless && !frame.mac_frame()).then(|| {
            let this = cx.weak_entity();
            window_controls(theme, window, corners, move |window, cx| {
                this.update(cx, |ws, cx| ws.drop_popout(id, cx)).ok();
                window.remove_window();
            })
        });
        // The hotkeys stand down behind a modal, in every window.
        let hotkeys_live = self.overlay.modal == Modal::None;
        Some(
            div()
                .key_context("Workspace")
                .track_focus(&self.focus.handle)
                .on_modifiers_changed(cx.listener(
                    |this, e: &gpui::ModifiersChangedEvent, _, cx| {
                        this.preview_modifiers(e.modifiers, cx)
                    },
                ))
                // A tab dragged out of this window: the moves and release reach here
                // only, so this is where other windows' zones are found and dropped on.
                .on_drag_move(cx.listener(|this, e: &DragMoveEvent<TabDrag>, window, cx| {
                    this.cross_window_drag_moved(e.bounds, e.event.position, window, cx)
                }))
                .on_mouse_up_out(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.cross_window_drop(window, cx)),
                )
                .on_drop(
                    cx.listener(|this, _: &super::group_layout::DividerDrag, _, cx| {
                        this.divider_drag_state.active = None;
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.divider_drag_state.active.is_some() {
                            this.divider_drag_state.active = None;
                            cx.notify();
                        }
                    }),
                )
                .when(hotkeys_live, |d| {
                    d.on_action(cx.listener(Self::toggle_mode))
                        .on_action(cx.listener(Self::go_back))
                        .on_action(cx.listener(Self::go_forward))
                        .on_action(cx.listener(Self::open_search_action))
                        .on_action(cx.listener(Self::open_graph_action))
                        .on_action(cx.listener(Self::open_command_palette_action))
                        .on_action(cx.listener(Self::run_command_action))
                })
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .bg(theme.tokens.background_primary)
                .map(|d| corners.apply(d))
                .font_family(theme.ui_font.clone())
                .text_size(px(crate::theme::FONT_UI_MEDIUM))
                .text_color(theme.tokens.text_normal)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(body),
                )
                .children(controls)
                .children(overlay)
                .children(file_menu)
                .children(notices)
                .children(preview)
                .into_any_element(),
        )
    }

    /// A popout's title: `<leaf> - <vault> - TFT`.
    fn popout_title(&self, wix: usize) -> SharedString {
        let leaf = self.window_leaf_title(&self.windows[wix].tree);
        format!("{leaf} - {} - TFT", self.vault_session.name).into()
    }

    /// The leaf a window's title names: the active tab of its most recently active group.
    pub(super) fn window_leaf_title(&self, tree: &Layout) -> SharedString {
        let mut order = Vec::new();
        tree.groups(&mut order);
        let group = self
            .group_registry
            .most_recent_matching(|g| order.contains(&g))
            .or_else(|| order.first().copied());
        group
            .and_then(|g| self.group_ix(g))
            .and_then(|ix| {
                let view = self.group_view(ix);
                view.tabs.get(view.active_tab).map(|t| t.title())
            })
            .unwrap_or_else(|| leaf_title(None, ViewMode::Reading))
    }

    /// `workspace:move-to-new-window`: the active tab moves to a popout, which takes the
    /// focus. An emptied group closes, unless it is the main area's only one, which is
    /// left holding an empty tab (observed).
    pub(super) fn move_to_new_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_group();
        let from_ix = self.active_group;
        let from_id = self.active_group_id();
        let at = self.groups[from_ix].state.active_tab;
        let Some(tab) = self.take_tab(from_ix, at, cx) else {
            return;
        };
        let emptied = self.groups[from_ix].state.tabs.is_empty();
        let main_only =
            emptied && self.window_of_group(from_id).is_none() && self.main_group_count() == 1;
        if main_only {
            // Its views still show the tab that left.
            let mode = self.default_mode();
            self.groups[from_ix].state.tabs.push(Tab::empty(mode));
            self.groups[from_ix].state.active_tab = 0;
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            self.stash_active_group();
        }
        if self
            .open_group_in_new_window(vec![tab.clone()], 0, window, cx)
            .is_none()
        {
            // No group, no move: the tab goes back where it was.
            if main_only {
                self.groups[from_ix].state.tabs.clear();
            }
            let group = &mut self.groups[from_ix];
            let at = at.min(group.state.tabs.len());
            group.state.tabs.insert(at, tab);
            group.state.active_tab = at;
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            return;
        }
        if emptied && !main_only {
            self.close_group(from_id, false, cx);
        }
    }

    /// `workspace:open-in-new-window`, and the header's "Open in new window": a copy of
    /// the active tab in a popout; the source tab stays (observed).
    pub(super) fn open_in_new_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_group();
        let mut tab = self.active_group_state.copy_of_active_tab();
        if tab.mode == ViewMode::Terminal {
            tab.view_state = Some(self.terminal.new_view_state());
        }
        self.open_group_in_new_window(vec![tab], 0, window, cx);
    }

    /// A row's "Open in new window": a fresh tab on `path` in a popout of its own.
    pub(super) fn open_path_in_new_window(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let tab = self.fresh_tab(path);
        self.open_group_in_new_window(vec![tab], 0, window, cx);
    }

    /// `workspace:new-window`: a popout holding one empty tab (observed); returns its id.
    pub(super) fn new_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        self.flush_save(cx);
        self.stash_active_group();
        let mode = self.default_mode();
        self.open_group_in_new_window(vec![Tab::empty(mode)], 0, window, cx)
    }

    /// `workspace:close-window`: closes the window the command ran in, not the active
    /// group's. A popout's groups close with it; the main window saves edits and
    /// layout first.
    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = window.window_handle();
        let popout = self
            .windows
            .iter()
            .find(|w| w.handle.map(gpui::AnyWindowHandle::from) == Some(here))
            .map(|w| w.id);
        match popout {
            Some(id) => self.drop_popout(id, cx),
            None => self.on_quit(cx),
        }
        window.remove_window();
    }

    /// `editor:open-link-in-new-window`: the link under the caret opens in a popout and
    /// the source stays. The link resolves against the source note before the window
    /// opens; a dangling link makes its note there.
    pub(super) fn open_link_in_new_window(
        &mut self,
        target: LinkTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let LinkTarget::Internal(body) = target else {
            self.follow_link(target, false, cx);
            return;
        };
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let source = self.active_group_state.current.clone().unwrap_or_default();
        let resolved = state.cache().resolve(&body, &source).map(str::to_string);
        if self.new_window(window, cx).is_none() {
            return;
        }
        let parsed = tft_markdown::parse_linktext(&body);
        match resolved {
            Some(path) => self.open_file(path, parsed.subpath, cx),
            None => {
                let name = parsed.path.to_string();
                self.create_note_for(&name, None, cx);
            }
        }
    }

    /// A new group holding `tabs`, in a popout of its own, made active. The window
    /// opens centered after this update, since its first frame reads the workspace
    /// being updated.
    fn open_group_in_new_window(
        &mut self,
        tabs: Vec<Tab>,
        active_tab: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let mode = tabs
            .get(active_tab)
            .map(|t| t.mode)
            .unwrap_or_else(|| self.default_mode());
        let (inner_w, inner_h) = crate::popout::INNER_SIZE;
        let window_id = self.group_registry.allocate_id();
        let id = self.new_group(mode, window, cx);
        let ix = self.group_ix(id)?;
        self.groups[ix].state.tabs = tabs;
        self.groups[ix].state.active_tab = active_tab;
        self.windows.push(WindowNode::new(
            window_id,
            desktop_split(SplitDirection::Vertical, vec![Layout::Group(id)]),
            WindowBounds {
                x: 0,
                y: 0,
                width: inner_w as i32,
                height: inner_h as i32,
            },
        ));
        self.load_group(ix, cx);
        self.load_tab(cx);
        self.focus.refocus_main = true;
        self.show_popout(window_id, None, cx);
        cx.notify();
        Some(window_id)
    }

    /// Opens the gpui window for popout `window_id` after this update, at `recorded`
    /// or centered. A desktop that gives no window is reported and the groups close.
    fn show_popout(
        &mut self,
        window_id: u64,
        recorded: Option<(WindowBounds, bool)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == window_id) {
            let (width, height) = crate::popout::INNER_SIZE;
            w.opened_for = Some(
                recorded
                    .filter(|(_, maximized)| !maximized)
                    .map_or((width as i32, height as i32), |(b, _)| (b.width, b.height)),
            );
        }
        let this = cx.entity();
        cx.defer(move |cx| {
            let bounds = popout_placement(recorded, cx);
            let opened = PopoutWindow::open(this.clone(), window_id, bounds, cx);
            this.update(cx, |ws, cx| match opened {
                Ok(handle) => {
                    if let Some(w) = ws.windows.iter_mut().find(|w| w.id == window_id) {
                        w.handle = Some(handle);
                    }
                    cx.notify();
                }
                Err(err) => {
                    ws.report(format!("Could not open a window: {err}"), cx);
                    ws.drop_popout(window_id, cx);
                }
            });
        });
    }

    /// Reopens the popouts `workspace.json` recorded, the active group's last so it is
    /// in front, then raises the vault window if its own group is active.
    pub(super) fn open_restored_popouts(&mut self, cx: &mut Context<Self>) {
        let active = self.window_of_group(self.active_group_id());
        let mut order: Vec<(u64, Option<(WindowBounds, bool)>)> = self
            .windows
            .iter()
            .filter(|w| w.handle.is_none())
            .map(|w| {
                let maximized = w
                    .node
                    .get("maximize")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                (w.id, w.bounds().map(|b| (b, maximized)))
            })
            .collect();
        if order.is_empty() {
            return;
        }
        order.sort_by_key(|(id, _)| Some(*id) == active);
        for (id, recorded) in order {
            self.show_popout(id, recorded, cx);
        }
        if active.is_none() {
            let main = self.main_window.handle();
            cx.defer(move |cx| {
                main.update(cx, |_, window, _| window.activate_window())
                    .ok();
            });
        }
    }

    /// Closes popout `id`'s groups, taking its node with the last, and returns the
    /// keyboard. The caller closes the gpui window.
    pub(crate) fn drop_popout(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(w) = self.windows.iter().find(|w| w.id == id) else {
            return;
        };
        let mut groups = Vec::new();
        w.tree.groups(&mut groups);
        for g in groups {
            self.close_group(g, true, cx);
        }
        self.windows.retain(|w| w.id != id);
        self.focus.refocus_main = true;
        cx.notify();
    }
}
