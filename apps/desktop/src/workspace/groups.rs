//! Tab and group lifecycle: loading live views, opening, splitting, closing, and restoring tabs.

use super::layout_write::desktop_split_node;
use super::subscriptions::view_subscriptions;
use super::{
    leaf_id, release_notes, set_dimension, Layout, SplitDirection, VaultState, ViewMode, Workspace,
};
use crate::canvas::CanvasView;
use crate::editor::Editor;
use crate::pdf::view::PdfView;
use crate::reading::{self, ReadingView};
use gpui::{AppContext, Context, Entity, SharedString, Window};
use serde_json::{json, Value};
use std::sync::Arc;
use tft_app::{place_new_tab, HistoryEntry, Tab};
use tft_markdown::Subpath;

/// How many closed tabs `workspace:undo-close-pane` can bring back.
const CLOSED_TABS_KEPT: usize = 20;

/// Session-local undo history for closed tabs, oldest first.
#[derive(Default)]
pub(super) struct ClosedTabHistory {
    tabs: Vec<(usize, Tab)>,
}

impl ClosedTabHistory {
    fn remember(&mut self, ix: usize, tab: Tab) {
        // Empty tabs are not worth bringing back.
        if tab.path.is_none()
            && tab.mode != ViewMode::Graph
            && tab.mode != ViewMode::Search
            && release_notes::version_of(&tab).is_none()
        {
            return;
        }
        self.tabs.push((ix, tab));
        if self.tabs.len() > CLOSED_TABS_KEPT {
            self.tabs.remove(0);
        }
    }

    fn take_last(&mut self) -> Option<(usize, Tab)> {
        self.tabs.pop()
    }

    pub(super) fn clear(&mut self) {
        self.tabs.clear();
    }
}

/// Shared id allocation for groups/windows and MRU ordering for live groups.
pub(super) struct GroupRegistryState {
    next_id: u64,
    recent: Vec<u64>,
}

impl GroupRegistryState {
    pub(super) fn new() -> Self {
        Self {
            next_id: 1,
            recent: vec![0],
        }
    }

    pub(super) fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(super) fn activate(&mut self, id: u64) {
        self.recent.retain(|recent| *recent != id);
        self.recent.insert(0, id);
    }

    pub(super) fn remove(&mut self, id: u64) {
        self.recent.retain(|recent| *recent != id);
    }

    pub(super) fn replace_recent(&mut self, recent: impl IntoIterator<Item = u64>) {
        self.recent = recent.into_iter().collect();
    }

    pub(super) fn most_recent(&self) -> Option<u64> {
        self.recent.first().copied()
    }

    pub(super) fn most_recent_matching(&self, mut matches: impl FnMut(u64) -> bool) -> Option<u64> {
        self.recent.iter().copied().find(|id| matches(*id))
    }
}

/// Live state for the active group. `groups[active_group].state` is its stashed
/// model snapshot; this value remains authoritative while that group is active.
pub(super) struct ActiveGroupState {
    pub(super) mode_before_graph: ViewMode,
    pub(super) mode: ViewMode,
    pub(super) current: Option<String>,
    pub(super) detached: Option<SharedString>,
    pub(super) history: Vec<HistoryEntry>,
    pub(super) history_ix: usize,
    pub(super) tabs: Vec<Tab>,
    pub(super) active_tab: usize,
}

impl ActiveGroupState {
    pub(super) fn new(mode: ViewMode) -> Self {
        Self {
            mode_before_graph: mode,
            mode,
            current: None,
            detached: None,
            history: Vec::new(),
            history_ix: 0,
            tabs: vec![Tab::empty(mode)],
            active_tab: 0,
        }
    }

    pub(super) fn stash_active_tab(&mut self) {
        let slot = self.tabs.get(self.active_tab);
        let leaf = slot.and_then(|t| t.leaf.clone());
        // The view state belongs to the canvas; a tab that moved on to another file leaves it behind.
        let view_state = slot
            .filter(|t| t.path == self.current)
            .and_then(|t| t.view_state.clone());
        let pinned = slot.is_some_and(|t| t.pinned);
        let tab = Tab {
            node: None,
            path: self.current.clone(),
            mode: self.mode,
            history: self.history.clone(),
            history_ix: self.history_ix,
            leaf,
            view_state,
            pinned,
        };
        match self.tabs.get_mut(self.active_tab) {
            Some(slot) => *slot = tab,
            None => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
        }
    }

    /// Every tab as it stands, the active one from the live fields.
    pub(super) fn tab_snapshots(&self) -> Vec<Tab> {
        let mut tabs = self.tabs.clone();
        if let Some(slot) = tabs.get_mut(self.active_tab) {
            slot.path = self.current.clone();
            slot.mode = self.mode;
            slot.history = self.history.clone();
            slot.history_ix = self.history_ix;
        }
        tabs
    }
    pub(super) fn copy_of_active_tab(&self) -> Tab {
        Tab {
            node: None,
            path: self.current.clone(),
            mode: self.mode,
            history: self.history.clone(),
            history_ix: self.history_ix,
            leaf: None,
            view_state: self
                .tabs
                .get(self.active_tab)
                .and_then(|t| t.view_state.clone()),
            pinned: false,
        }
    }

    pub(super) fn active_tab_pinned(&self) -> bool {
        self.tabs.get(self.active_tab).is_some_and(|tab| tab.pinned)
    }
}

/// One tab group owns its GPUI views. The active tab/navigation fields are live on
/// Workspace while this group's `tft_app` model holds the stashed snapshot.
pub(super) struct TabGroup {
    pub(super) state: tft_app::TabGroup,
    pub(super) editor: Entity<Editor>,
    pub(super) reading: Entity<ReadingView>,
    pub(super) canvas: Entity<CanvasView>,
    pub(super) pdf: Entity<PdfView>,
}

impl TabGroup {
    pub(super) fn new(
        id: u64,
        editor: Entity<Editor>,
        reading: Entity<ReadingView>,
        canvas: Entity<CanvasView>,
        pdf: Entity<PdfView>,
        mode: ViewMode,
    ) -> TabGroup {
        let mut state = tft_app::TabGroup::new(id, mode);
        state.node.insert("id".into(), json!(leaf_id()));
        state.node.insert("type".into(), json!("tabs"));
        TabGroup {
            state,
            editor,
            reading,
            canvas,
            pdf,
        }
    }

    /// The group's share of its split, Obsidian's `dimension` (a percentage).
    pub(super) fn dimension(&self) -> Option<f64> {
        self.state.node.get("dimension").and_then(Value::as_f64)
    }

    pub(super) fn set_dimension(&mut self, dimension: Option<f64>) {
        set_dimension(&mut self.state.node, dimension);
    }
}

impl Workspace {
    /// The active group's GPUI views live in its `TabGroup`; don't mirror their
    /// entity handles on Workspace, where group switches could stale the aliases.
    pub(super) fn active_editor(&self) -> &Entity<Editor> {
        &self.groups[self.active_group].editor
    }

    pub(super) fn active_reading(&self) -> &Entity<ReadingView> {
        &self.groups[self.active_group].reading
    }

    pub(super) fn active_canvas(&self) -> &Entity<CanvasView> {
        &self.groups[self.active_group].canvas
    }

    pub(super) fn active_pdf(&self) -> &Entity<PdfView> {
        &self.groups[self.active_group].pdf
    }

    /// The active group's editor, exposed for the Desktop app's editor integrations.
    pub fn editor(&self) -> &Entity<Editor> {
        self.active_editor()
    }

    /// The view mode of the active group.
    pub fn mode(&self) -> ViewMode {
        self.active_group_state.mode
    }

    /// The active group's current vault-relative document path, if any.
    pub fn current(&self) -> Option<&str> {
        self.active_group_state.current.as_deref()
    }

    /// Puts `tabs[active_tab]` on screen: its note in its mode, or the empty state.
    pub(super) fn load_tab(&mut self, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        let Some(tab) = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)
            .cloned()
        else {
            return;
        };
        self.active_group_state.history = tab.history;
        self.active_group_state.history_ix = tab.history_ix;
        if tab.mode == ViewMode::Graph && self.active_group_state.mode != ViewMode::Graph {
            self.active_group_state.mode_before_graph = self.active_group_state.mode;
        }
        self.active_group_state.mode = tab.mode;
        if tab.mode.is_editing() {
            self.active_editor().update(cx, |e, cx| {
                e.set_live_preview(tab.mode == ViewMode::LivePreview, cx)
            });
        }
        match (tab.path, self.vault_session.state.clone()) {
            (Some(path), Some(state)) => {
                self.active_group_state.current = None;
                self.show_file(&path, &state, cx);
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (path, None) => self.active_group_state.current = path,
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// The empty tab: no note and nothing describing one. A buffer with unsaved edits
    /// nowhere else stays, marked as saved nowhere.
    pub(super) fn show_empty(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        self.release_buffer(cx);
        self.release_canvas(cx);
        self.release_pdf(cx);
        self.active_reading().update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.active_group_state.current = None;
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
        self.feedback.clear_status_text();
    }

    /// `workspace:new-tab` and the strip's `+`: the new tab goes on the strip's right
    /// end and becomes active, whichever tab was active.
    pub fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        let mode = self.default_mode();
        self.active_group_state.tabs.push(Tab::empty(mode));
        self.active_group_state.active_tab = self.active_group_state.tabs.len() - 1;
        self.load_tab(cx);
    }

    /// Opens `path` in a new tab right after the active one and makes it active: where
    /// Obsidian 1.15 puts a tab a link, a Files row or the quick switcher opens.
    pub fn open_in_new_tab(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        let mode = self.default_mode();
        self.active_group_state.active_tab = place_new_tab(
            &mut self.active_group_state.tabs,
            self.active_group_state.active_tab,
            Tab::empty(mode),
        );
        self.load_tab(cx);
        self.open_file(path, subpath, cx);
    }

    /// Whether a tab opened from a link goes to the reader; "Always focus new tabs" is fixed on.
    pub(super) fn focus_new_tabs(&self) -> bool {
        crate::fixed::FOCUS_NEW_TABS
    }

    /// A link, row, bookmark or "Open in new tab": a tab after the active one, made
    /// active only while "Always focus new tabs" is on. Off, it opens behind and the
    /// active tab keeps the keyboard. The switcher's and `new-tab`'s tabs are always active.
    pub fn open_link_in_new_tab(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        if self.focus_new_tabs() {
            return self.open_in_new_tab(path, subpath, cx);
        }
        self.open_in_background_tab(path, cx);
    }

    /// `path` in a new tab after the active one, which stays active. The tab holds one
    /// history entry and loads when shown, as a restored tab does.
    pub(super) fn open_in_background_tab(&mut self, path: String, cx: &mut Context<Self>) {
        self.active_group_state.stash_active_tab();
        let mode = self.default_mode();
        place_new_tab(
            &mut self.active_group_state.tabs,
            self.active_group_state.active_tab,
            Tab::unloaded(path, mode),
        );
        cx.notify();
    }

    pub fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.active_group_state.tabs.len() || ix == self.active_group_state.active_tab {
            return;
        }
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        self.active_group_state.active_tab = ix;
        self.load_tab(cx);
    }

    /// `workspace:next-tab` and `previous-tab`: `step` tabs along, wrapping.
    pub(super) fn cycle_tab(&mut self, step: usize, cx: &mut Context<Self>) {
        let n = self.active_group_state.tabs.len();
        if n > 1 {
            self.activate_tab((self.active_group_state.active_tab + step) % n, cx);
        }
    }

    /// `workspace:close`: the last close leaves the empty tab; the tab to the right
    /// takes over, or the one to the left at the end of the strip.
    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        let was_active = ix == self.active_group_state.active_tab;
        let Some(tab) = tft_app::workspace::remove_tab_at(
            &mut self.active_group_state.tabs,
            &mut self.active_group_state.active_tab,
            ix,
        ) else {
            return;
        };
        self.remember_closed(ix, tab);
        if self.active_group_state.tabs.is_empty() {
            // A group whose last tab closes goes with it, unless it is the main area's
            // only one. A popout's goes regardless, and its window with it.
            if self.active_group_in_popout() || self.main_group_count() > 1 {
                self.active_group_state
                    .tabs
                    .push(Tab::empty(self.default_mode()));
                self.active_group_state.active_tab = 0;
                let id = self.active_group_id();
                self.close_group(id, false, cx);
                return;
            }
            self.active_group_state
                .tabs
                .push(Tab::empty(self.default_mode()));
            self.active_group_state.active_tab = 0;
            self.load_tab(cx);
        } else if was_active {
            self.load_tab(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn remember_closed(&mut self, ix: usize, tab: Tab) {
        self.closed_tab_history.remember(ix, tab);
    }

    /// `workspace:close-others`: every tab but the active one and the pinned ones (observed).
    pub(super) fn close_other_tabs(&mut self, cx: &mut Context<Self>) {
        let active = self.active_group_state.active_tab;
        self.close_tabs_but(|i, t| i == active || t.pinned, Some(active), cx);
    }

    /// The tab header's "Close all": every tab but the pinned ones, the first of which
    /// stays active (observed). With none pinned, tabs close from the right.
    pub(super) fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        self.active_group_state.stash_active_tab();
        if self.active_group_state.tabs.iter().any(|t| t.pinned) {
            return self.close_tabs_but(|_, t| t.pinned, None, cx);
        }
        let group = self.active_group_id();
        for ix in (0..self.active_group_state.tabs.len()).rev() {
            if self.active_group_id() != group {
                break;
            }
            self.active_group_state.tabs[ix].pinned = false;
            self.close_tab(ix, cx);
        }
    }

    /// Closes every tab of the active group `keep` turns down, keeping one. `prefer` is
    /// the tab to leave active when it is kept.
    pub(super) fn close_tabs_but(
        &mut self,
        keep: impl Fn(usize, &Tab) -> bool,
        prefer: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        let old_active = self.active_group_state.active_tab;
        let mut kept = Vec::new();
        let (mut preferred, mut was_active) = (None, None);
        for (i, tab) in std::mem::take(&mut self.active_group_state.tabs)
            .into_iter()
            .enumerate()
        {
            if keep(i, &tab) {
                if Some(i) == prefer {
                    preferred = Some(kept.len());
                }
                if i == old_active {
                    was_active = Some(kept.len());
                }
                kept.push(tab);
            } else {
                self.remember_closed(i, tab);
            }
        }
        debug_assert!(!kept.is_empty());
        if kept.is_empty() {
            kept.push(Tab::empty(self.default_mode()));
        }
        self.active_group_state.tabs = kept;
        let active = preferred.or(was_active).unwrap_or(0);
        self.active_group_state.active_tab = active;
        if was_active == Some(active) {
            cx.notify();
        } else {
            self.load_tab(cx);
        }
    }

    /// `workspace:undo-close-pane`: the most recently closed tab, back where it was.
    pub(super) fn undo_close_tab(&mut self, cx: &mut Context<Self>) {
        let Some((ix, tab)) = self.closed_tab_history.take_last() else {
            return;
        };
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        let at = ix.min(self.active_group_state.tabs.len());
        self.active_group_state.tabs.insert(at, tab);
        self.active_group_state.active_tab = at;
        self.load_tab(cx);
    }

    // ----- tab groups --------------------------------------------------------------

    pub(super) fn active_group_id(&self) -> u64 {
        self.groups[self.active_group].state.id
    }

    pub(super) fn group_ix(&self, id: u64) -> Option<usize> {
        self.groups.iter().position(|g| g.state.id == id)
    }

    /// Copies the active group's live tab/session fields into its model slot. That
    /// model is a snapshot while active; Workspace's tab/session fields remain
    /// authoritative until handoff. GPUI view entities remain owned by each TabGroup.
    pub(super) fn stash_active_group(&mut self) {
        self.active_group_state.stash_active_tab();
        let ix = self.active_group;
        let group = &mut self.groups[ix];
        group.state.tabs = self.active_group_state.tabs.clone();
        group.state.active_tab = self.active_group_state.active_tab;
        group.state.mode_before_graph = self.active_group_state.mode_before_graph;
        group.state.detached = self
            .active_group_state
            .detached
            .clone()
            .map(|name| name.to_string());
    }

    /// Makes group `ix` the live one. Nothing reloads, but everything following the
    /// active note is pointed at it, and a vanished note is let go.
    pub(super) fn load_group(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        self.active_group = ix;
        let group = &self.groups[ix];
        let id = group.state.id;
        self.active_group_state.tabs = group.state.tabs.clone();
        self.active_group_state.active_tab = group
            .state
            .active_tab
            .min(self.active_group_state.tabs.len().saturating_sub(1));
        self.active_group_state.mode_before_graph = group.state.mode_before_graph;
        self.active_group_state.detached = group.state.detached.clone().map(Into::into);
        let tab = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)
            .cloned()
            .unwrap_or_else(|| Tab::empty(self.default_mode()));
        self.active_group_state.current = tab.path;
        self.active_group_state.mode = tab.mode;
        self.active_group_state.history = tab.history;
        self.active_group_state.history_ix = tab.history_ix;
        self.group_registry.activate(id);
        self.document_save.cancel();
        match (
            self.active_group_state.current.clone(),
            self.vault_session.state.clone(),
        ) {
            (Some(path), Some(state)) => {
                if state.vault().absolute(&path).is_file() {
                    self.explorer.update(cx, |e, cx| e.follow(&path, cx));
                    self.graph
                        .update(cx, |g, cx| g.set_current(Some(path.clone()), cx));
                    self.feedback.set_status(path);
                } else {
                    self.detach_current(true, &state, cx);
                }
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (_, None) => {}
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// Hands the keyboard and live fields to group `id`: a click, a focus command, or
    /// the group that takes over.
    pub fn activate_group(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        if ix == self.active_group {
            return;
        }
        self.flush_save(cx);
        self.stash_active_group();
        self.load_group(ix, cx);
    }

    /// A group of its own, with an editor and reading view like the others', not yet in the layout.
    pub(super) fn new_group(
        &mut self,
        mode: ViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let editor = cx.new(|cx| crate::new_editor(window, cx));
        let reading = cx.new(ReadingView::new);
        let canvas = cx.new(CanvasView::new);
        let pdf = cx.new(PdfView::new);
        self._subscriptions
            .extend(view_subscriptions(&editor, &reading, &canvas, &pdf, cx));
        let id = self.group_registry.allocate_id();
        if let Some(state) = self.vault_session.state.clone() {
            Self::give_editor_the_vault(&editor, &state, cx);
        }
        self.groups
            .push(TabGroup::new(id, editor, reading, canvas, pdf, mode));
        if let Some(state) = self.vault_session.state.clone() {
            self.apply_app_config(&state, cx);
        }
        id
    }

    /// `workspace:split-vertical`/`split-horizontal`: a new group beside the active one
    /// showing the same note and mode, taking the focus (see [[Specification#Observed tab group behavior]]).
    pub(super) fn split(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_beside(None, direction, window, cx);
    }

    /// The explorer menu's "Open to the right": a new group beside the active one on
    /// `path`, unlike a split, which keeps the group's own note.
    pub(super) fn open_to_right(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_beside(Some(path), SplitDirection::Vertical, window, cx);
    }

    /// The two share: `None` duplicates the active tab (a split), `Some(path)` opens a
    /// fresh tab in the default mode. Flush and stash happen before the tab is built,
    /// so `view_state` is the one just written.
    pub(super) fn split_beside(
        &mut self,
        path: Option<String>,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let from = self.active_group_id();
        let (tab, mode_before_graph) = match path {
            None => (
                self.active_group_state.copy_of_active_tab(),
                self.active_group_state.mode_before_graph,
            ),
            Some(path) => {
                let tab = self.fresh_tab(path);
                let mode = tab.mode;
                (tab, mode)
            }
        };
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(from, id, direction, true);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].state.tabs = vec![tab];
        self.groups[ix].state.active_tab = 0;
        self.groups[ix].state.mode_before_graph = mode_before_graph;
        self.load_group(ix, cx);
        // The new group's views are empty: put the note in them.
        if let (Some(path), Some(state)) = (
            self.active_group_state.current.clone(),
            self.vault_session.state.clone(),
        ) {
            if self.active_group_state.mode.is_editing() {
                let live = self.active_group_state.mode == ViewMode::LivePreview;
                self.active_editor()
                    .update(cx, |e, cx| e.set_live_preview(live, cx));
            }
            self.show_file(&path, &state, cx);
        }
    }

    /// A twin of the active tab (a split or "Open in new window"): same note, mode,
    /// history and view state; call with the active group stashed.
    /// A fresh tab on `path`, no history or view state, in the default mode: what
    /// "Open to the right" and a row's "Open in new window" show.
    pub(super) fn fresh_tab(&self, path: String) -> Tab {
        Tab {
            node: None,
            path: Some(path),
            mode: self.default_mode(),
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
            pinned: false,
        }
    }

    /// Puts group `new` beside `from` along `direction`, halving the `dimension` when
    /// they become siblings or moving it onto the wrapping split.
    pub(super) fn place_beside(
        &mut self,
        from: u64,
        new: u64,
        direction: SplitDirection,
        after: bool,
    ) {
        let Some(from_ix) = self.group_ix(from) else {
            return;
        };
        let dimension = self.groups[from_ix].dimension();
        // The tree `from` is in: a popout's groups split inside their own
        // window.
        let window = self.window_of_group(from);
        match self.tree_mut(window).insert_beside_with_node(
            from,
            new,
            direction,
            after,
            desktop_split_node(),
        ) {
            Some(false) => {
                if let Some(d) = dimension {
                    self.groups[from_ix].set_dimension(Some(d / 2.0));
                    if let Some(new_ix) = self.group_ix(new) {
                        self.groups[new_ix].set_dimension(Some(d / 2.0));
                    }
                }
            }
            Some(true) => {
                self.groups[from_ix].set_dimension(None);
                let tree = self.tree_mut(window);
                if let Some(mut path) = tree.path_to(from) {
                    path.pop();
                    if let Some(Layout::Split { node, .. }) = tree.node_at_mut(&path) {
                        set_dimension(node, dimension);
                    }
                }
            }
            None => {}
        }
    }

    /// The popout window group `id` is in, by its id; `None` for the main area.
    /// `workspace:close-tab-group`, and closing a group's last tab: the group goes,
    /// its siblings take the room, and focus goes to the most recently used. The last
    /// group is left with an empty tab.
    pub(super) fn close_group(&mut self, id: u64, remember: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        let in_window = self.windows.iter().position(|w| w.tree.contains(id));
        if in_window.is_none() && self.main_group_count() == 1 {
            self.flush_save(cx);
            self.active_group_state.stash_active_tab();
            let tabs = std::mem::take(&mut self.active_group_state.tabs);
            if remember {
                for (i, tab) in tabs.into_iter().enumerate() {
                    self.remember_closed(i, tab);
                }
            }
            self.active_group_state
                .tabs
                .push(Tab::empty(self.default_mode()));
            self.active_group_state.active_tab = 0;
            self.load_tab(cx);
            return;
        }
        let was_active = ix == self.active_group;
        let active_id = self.active_group_id();
        // Nothing paints this group's header again, so its title's shaped line goes with it.
        self.title_editing.layouts.remove(&id);
        if was_active {
            self.flush_save(cx);
            self.stash_active_group();
        }
        if remember {
            let tabs = self.groups[ix].state.tabs.clone();
            for (i, tab) in tabs.into_iter().enumerate() {
                self.remember_closed(i, tab);
            }
        }
        let window = in_window.map(|wix| self.windows[wix].id);
        let mut parent = self.tree(window).path_to(id).unwrap_or_default();
        parent.pop();
        self.tree_mut(window).remove(id);
        self.groups.remove(ix);
        self.group_registry.remove(id);
        let mut left = Vec::new();
        self.tree(window).groups(&mut left);
        if let (Some(wix), true) = (in_window, left.is_empty()) {
            // A popout's last group takes the window with it, as Obsidian's does. Its
            // gpui window is removed here unless this runs inside it.
            let node = self.windows.remove(wix);
            if let Some(handle) = node.handle {
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            }
        } else {
            // The split the group left, or the nearest one still standing.
            loop {
                if matches!(
                    self.tree(window).node_at(&parent),
                    Some(Layout::Split { .. })
                ) {
                    self.renormalise_dimensions(window, &parent);
                    break;
                }
                if parent.pop().is_none() {
                    break;
                }
            }
        }
        if was_active {
            let next = self
                .group_registry
                .most_recent()
                .and_then(|g| self.group_ix(g))
                .unwrap_or(0);
            self.load_group(next, cx);
        } else {
            self.active_group = self.group_ix(active_id).unwrap_or(0);
            cx.notify();
        }
    }

    /// How many groups the main area holds (`groups` also counts popouts').
    /// Every tab but the live one: the active group's other tabs and all of
    /// the other groups'. What the live one needs is done on the workspace's
    /// own fields.
    pub(super) fn stashed_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        let active_tab = self.active_group_state.active_tab;
        let active_group = self.active_group;
        let own = self
            .active_group_state
            .tabs
            .iter_mut()
            .enumerate()
            .filter(move |(ix, _)| *ix != active_tab)
            .map(|(_, t)| t);
        let others = self
            .groups
            .iter_mut()
            .enumerate()
            .filter(move |(gx, _)| *gx != active_group)
            .flat_map(|(_, g)| g.state.tabs.iter_mut());
        own.chain(others)
    }
}
