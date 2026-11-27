//! Tab drag/drop event coordination and group-to-group tab transitions.

use super::{DragBounds, DropBar, DropZone, TabDrag, Workspace};
use gpui::{px, Context, Pixels, Window};
use std::time::Instant;
use tft_app::{SplitDirection, Tab};

impl Workspace {
    // ----- dragging tabs ---------------------------------------------------------

    pub(super) fn begin_tab_drag(&mut self, drag: TabDrag, cx: &mut Context<Self>) {
        self.tab_drag_state.active = true;
        self.tab_drag_state.payload = Some(drag);
        *self.tab_drag_state.bounds.borrow_mut() = DragBounds::default();
        self.tab_drag_state.drop = None;
        self.tab_drag_state.bar = None;
        self.tab_drag_state.tab_bounds.clear();
        cx.notify();
    }

    /// The dragged tab moved over (or off) group `group`'s strip: over it,
    /// the drop bar goes to the tab boundary nearest the pointer, the left
    /// edge of a tab or the right edge of the last one, sliding there from
    /// wherever it was. Off it, the bar goes if it was this strip's.
    pub(super) fn strip_drag_moved(
        &mut self,
        group: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !bounds.contains(&position) {
            if self.tab_drag_state.bar.is_some_and(|b| b.group == group) {
                self.tab_drag_state.bar = None;
                cx.notify();
            }
            return;
        }
        let mut tabs: Vec<(usize, gpui::Bounds<gpui::Pixels>)> = self
            .tab_drag_state
            .tab_bounds
            .iter()
            .filter(|((g, _), _)| *g == group)
            .map(|((_, ix), b)| (*ix, *b))
            .collect();
        tabs.sort_by_key(|(ix, _)| *ix);
        let Some((last_ix, last)) = tabs.last().copied() else {
            return;
        };
        let pointer = f32::from(position.x);
        let edges = tabs
            .iter()
            .map(|(ix, b)| (*ix, f32::from(b.origin.x)))
            .chain(std::iter::once((last_ix + 1, f32::from(last.right()))));
        let Some((at, x)) =
            edges.min_by(|a, b| (a.1 - pointer).abs().total_cmp(&(b.1 - pointer).abs()))
        else {
            return;
        };
        let now = Instant::now();
        let strip_x = f32::from(bounds.origin.x);
        let next = match self.tab_drag_state.bar {
            Some(bar) if bar.group == group && bar.at == at => {
                if bar.strip_x == strip_x {
                    return;
                }
                DropBar { strip_x, ..bar }
            }
            // Within the strip the bar slides; arriving from elsewhere it appears where it is wanted.
            Some(bar) if bar.group == group => DropBar {
                group,
                at,
                x,
                from: bar.shown_x(now),
                since: now,
                strip_x,
            },
            _ => DropBar {
                group,
                at,
                x,
                from: x,
                since: now,
                strip_x,
            },
        };
        self.tab_drag_state.bar = Some(next);
        cx.notify();
    }

    /// Each window's frame during a drag. A drag that ended away from a zone just
    /// stops; the zones go away in every window and the others redraw once. A sliding
    /// bar needs a frame per step until it lands.
    pub(super) fn tab_drag_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_drag_state.active && !cx.has_active_drag() {
            self.tab_drag_state.active = false;
            self.tab_drag_state.payload = None;
            self.tab_drag_state.drop = None;
            self.tab_drag_state.bar = None;
            self.tab_drag_state.tab_bounds.clear();
            let this = cx.entity();
            cx.defer(move |cx| this.update(cx, |_, cx| cx.notify()));
        }
        if self
            .tab_drag_state
            .bar
            .is_some_and(|b| !b.settled(Instant::now()))
        {
            window.request_animation_frame();
        }
    }

    /// The window `window` is as the layout names it: a popout's id, or `None` for the vault.
    pub(super) fn window_id_of(&self, window: &Window) -> Option<u64> {
        let here = window.window_handle();
        self.windows
            .iter()
            .find(|w| w.handle.map(gpui::AnyWindowHandle::from) == Some(here))
            .map(|w| w.id)
    }

    /// The dragged tab moved, heard by its source window. Off that window the pointer
    /// is mapped to the screen and each other window's groups light their zones and bar
    /// using their own measured bounds. Wayland never knows where its windows are, so
    /// the drag stays in its own window; overlapping windows go in layout order.
    pub(super) fn cross_window_drag_moved(
        &mut self,
        root: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tab_drag_state.active || cx.compositor_name() == "Wayland" {
            return;
        }
        let source = self.window_id_of(window);
        let screen = window.bounds().origin + position;
        let inside = root.contains(&position);
        let mut others: Vec<(Option<u64>, gpui::AnyWindowHandle)> = self
            .windows
            .iter()
            .filter(|w| Some(w.id) != source)
            .filter_map(|w| w.handle.map(|h| (Some(w.id), h.into())))
            .collect();
        if source.is_some() {
            others.insert(0, (None, self.main_window.handle()));
        }
        let placed: Vec<(Option<u64>, gpui::Bounds<Pixels>)> = others
            .into_iter()
            .filter_map(|(id, handle)| {
                handle
                    .update(cx, |_, w, _| w.bounds())
                    .ok()
                    .map(|b| (id, b))
            })
            .collect();
        let over = (!inside)
            .then(|| {
                placed
                    .iter()
                    .find(|(_, b)| b.contains(&screen))
                    .map(|(id, _)| *id)
            })
            .flatten();
        // Nowhere near any window, for the windows the pointer is not over.
        let away = gpui::point(px(-1e6), px(-1e6));
        let measured = self.tab_drag_state.bounds.clone();
        let measured = measured.borrow();
        for (id, bounds) in &placed {
            let local = if over == Some(*id) {
                screen - bounds.origin
            } else {
                away
            };
            let mut groups = Vec::new();
            self.tree(*id).groups(&mut groups);
            for g in groups {
                if let Some(content) = measured.contents.get(&g) {
                    self.tab_drag_moved(g, *content, local, cx);
                }
                if let Some(strip) = measured.strips.get(&g) {
                    for (key, b) in measured.tabs.iter().filter(|((tg, _), _)| *tg == g) {
                        self.tab_drag_state.tab_bounds.insert(*key, *b);
                    }
                    self.strip_drag_moved(g, *strip, local, cx);
                }
            }
        }
    }

    /// The button came up outside the source window: a drop on another window's zone
    /// lands there as if dropped in it, and the active group follows the desktop's
    /// focus, as Obsidian's active leaf did. Over no window, the drag just ends.
    pub(super) fn cross_window_drop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag_state.payload.clone() else {
            return;
        };
        let source = self.window_id_of(window);
        let foreign = |g: u64| self.window_of_group(g) != source;
        if let Some(bar) = self.tab_drag_state.bar.filter(|b| foreign(b.group)) {
            self.drop_tab_at_bar(&drag, bar.group, None, cx);
        } else if let Some((g, _)) = self.tab_drag_state.drop.filter(|(g, _)| foreign(*g)) {
            self.drop_tab_on_group(&drag, g, window, cx);
        } else {
            return;
        }
        self.tab_drag_state.payload = None;
        let source_open = source.is_none_or(|id| self.windows.iter().any(|w| w.id == id));
        if source_open {
            self.window_activated(source, cx);
        }
    }

    /// A tab dropped on group `to`'s strip lands where the bar says, or at `fallback`.
    pub(super) fn drop_tab_at_bar(
        &mut self,
        drag: &TabDrag,
        to: u64,
        fallback: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let at = match self.tab_drag_state.bar {
            Some(bar) if bar.group == to => Some(bar.at),
            _ => fallback,
        };
        self.drop_tab_on_strip(drag, to, at, cx);
    }

    /// A dragged tab over group `id`'s content lights the zone it would land in;
    /// leaving puts it out.
    pub(super) fn tab_drag_moved(
        &mut self,
        id: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let next = if bounds.contains(&position) {
            Some((id, DropZone::at(bounds, position)))
        } else if self.tab_drag_state.drop.is_some_and(|(g, _)| g == id) {
            None
        } else {
            return;
        };
        if self.tab_drag_state.drop != next {
            self.tab_drag_state.drop = next;
            cx.notify();
        }
    }

    /// A tab dropped on group `id`'s content: into the strip from the middle, a new
    /// group from a side.
    pub(super) fn drop_tab_on_group(
        &mut self,
        drag: &TabDrag,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let zone = match self.tab_drag_state.drop.take() {
            Some((g, zone)) if g == id => zone,
            _ => DropZone::Center,
        };
        self.tab_drag_state.active = false;
        self.tab_drag_state.bar = None;
        self.tab_drag_state.tab_bounds.clear();
        match zone.split() {
            None => self.move_tab(drag, id, None, cx),
            Some((direction, after)) => self.split_with_tab(drag, id, direction, after, window, cx),
        }
    }

    /// A tab dropped on group `to`'s strip at `at` joins it there and becomes active.
    pub(super) fn drop_tab_on_strip(
        &mut self,
        drag: &TabDrag,
        to: u64,
        at: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.tab_drag_state.drop = None;
        self.tab_drag_state.active = false;
        self.tab_drag_state.bar = None;
        self.tab_drag_state.tab_bounds.clear();
        self.move_tab(drag, to, at, cx);
    }

    /// Takes tab `ix` out of group `from_ix` with every group stashed; says whether
    /// the group has nothing left.
    pub(super) fn take_tab(
        &mut self,
        from_ix: usize,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Option<Tab> {
        let group = &mut self.groups[from_ix];
        if ix >= group.state.tabs.len() {
            return None;
        }
        let tab = group.state.tabs.remove(ix);
        if group.state.tabs.is_empty() {
            return Some(tab);
        }
        if ix < group.state.active_tab {
            group.state.active_tab -= 1;
        } else if ix == group.state.active_tab {
            group.state.active_tab = ix.min(group.state.tabs.len() - 1);
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            self.stash_active_group();
        }
        Some(tab)
    }

    /// Group `ix`, emptied by a drag, when it is the main area's only group: it keeps
    /// an empty tab, as `move-to-new-window` leaves it (observed). Every group must be
    /// stashed; says whether it stays.
    pub(super) fn keep_last_main_group(&mut self, ix: usize, cx: &mut Context<Self>) -> bool {
        let id = self.groups[ix].state.id;
        if self.window_of_group(id).is_some() || self.main_group_count() > 1 {
            return false;
        }
        let empty = Tab::empty(self.default_mode());
        self.groups[ix].state.tabs.push(empty);
        self.groups[ix].state.active_tab = 0;
        self.load_group(ix, cx);
        self.load_tab(cx);
        self.stash_active_group();
        true
    }

    /// Moves the dragged tab to group `to`, before its tab `at` or at the
    /// end, and makes it the active tab of the active group. In its own
    /// group that is a reorder. A group left with nothing closes, as one
    /// whose last tab closed does.
    pub(super) fn move_tab(
        &mut self,
        drag: &TabDrag,
        to: u64,
        at: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let (Some(from_ix), Some(to_ix)) = (self.group_ix(drag.group), self.group_ix(to)) else {
            return;
        };
        let mut at = at.unwrap_or(usize::MAX);
        let mut empty = false;
        let tab = if from_ix == to_ix {
            // A reorder: the strip's views keep showing what they show
            // until the end, when the dragged tab takes over if it must.
            let group = &mut self.groups[to_ix];
            if drag.tab >= group.state.tabs.len() {
                return;
            }
            if drag.tab < at {
                at -= 1;
            }
            group.state.tabs.remove(drag.tab)
        } else {
            let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
                return;
            };
            empty = self.groups[from_ix].state.tabs.is_empty()
                && !self.keep_last_main_group(from_ix, cx);
            tab
        };
        // What the target's views show now: the tab that was active there, if any.
        let target = &mut self.groups[to_ix];
        let shown = (from_ix != to_ix || drag.tab != target.state.active_tab)
            .then(|| target.state.tabs.get(target.state.active_tab).cloned())
            .flatten();
        let at = at.min(target.state.tabs.len());
        target.state.tabs.insert(at, tab);
        target.state.active_tab = at;
        self.load_group(to_ix, cx);
        if shown.is_none()
            || self
                .active_group_state
                .tabs
                .get(self.active_group_state.active_tab)
                != shown.as_ref()
        {
            self.load_tab(cx);
        }
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// Puts the dragged tab in a new group beside `beside`, after or before along
    /// `direction`, and makes it active. A group's only tab dropped on its own edge
    /// does nothing.
    pub(super) fn split_with_tab(
        &mut self,
        drag: &TabDrag,
        beside: u64,
        direction: SplitDirection,
        after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let Some(from_ix) = self.group_ix(drag.group) else {
            return;
        };
        if drag.group == beside && self.groups[from_ix].state.tabs.len() <= 1 {
            return;
        }
        if self.group_ix(beside).is_none() {
            return;
        }
        let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
            return;
        };
        let empty =
            self.groups[from_ix].state.tabs.is_empty() && !self.keep_last_main_group(from_ix, cx);
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(beside, id, direction, after);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].state.tabs = vec![tab];
        self.groups[ix].state.active_tab = 0;
        self.load_group(ix, cx);
        self.load_tab(cx);
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }
}
