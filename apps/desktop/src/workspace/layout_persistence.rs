//! Restoring and saving the Desktop Workspace layout and pane state.

use super::{
    default_dock_json, floating_json, leaf_json, read_floating, read_main, split_node_json,
    tabs_node_json, window_node_json, LayoutSpec, SplitDirection, VaultState, ViewMode, WindowNode,
    Workspace,
};
use crate::theme::SIDEBAR_WIDTH;
use gpui::{Context, Window};
use serde_json::{json, Value};
use std::sync::Arc;
use tft_app::explorer::SortOrder;
use tft_app::Layout;

/// How many paths `workspace.json`'s `lastOpenFiles` keeps after a new open.
const LAST_OPEN_FILES_KEPT: usize = 10;

#[derive(Default)]
pub(super) struct WorkspacePersistenceState {
    /// `workspace.json` as read at load, preserved when writing unknown keys back.
    snapshot: Option<Value>,
    /// The loaded layout suppresses the startup flag that would open the first note.
    restored: bool,
    /// Recently opened vault-relative paths stored in `workspace.json`.
    last_open_files: Vec<String>,
    file_tree_sort: SortOrder,
    file_tree_expanded: std::collections::HashSet<String>,
}

impl WorkspacePersistenceState {
    pub(super) fn clear_vault_layout(&mut self) {
        self.snapshot = None;
        self.restored = false;
        self.last_open_files.clear();
        self.file_tree_sort = SortOrder::default();
        self.file_tree_expanded.clear();
    }

    pub(super) fn load_last_open_files(&mut self, layout: &Value) {
        self.last_open_files = layout
            .get("lastOpenFiles")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    }

    pub(super) fn load_file_tree_settings(&mut self, layout: &Value) {
        let tft = layout.get("tft");
        self.file_tree_sort = tft
            .and_then(|value| value.get("fileTreeSort"))
            .and_then(Value::as_str)
            .and_then(SortOrder::parse)
            .unwrap_or_default();
        self.file_tree_expanded = tft
            .and_then(|value| value.get("fileTreeExpanded"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }

    pub(super) fn record_open_file(&mut self, path: &str) {
        self.last_open_files.retain(|recent| recent != path);
        self.last_open_files.insert(0, path.to_string());
        self.last_open_files.truncate(LAST_OPEN_FILES_KEPT);
    }

    pub(super) fn set_file_tree_sort(&mut self, sort: SortOrder) {
        self.file_tree_sort = sort;
    }

    pub(super) fn set_file_tree_expanded(&mut self, folders: &[String]) {
        self.file_tree_expanded = folders.iter().cloned().collect();
    }
}

impl Workspace {
    // ----- workspace.json --------------------------------------------------------

    /// Whether a `workspace.json` was read at load; the startup flags then leave the
    /// first note alone, as Obsidian shows the empty state.
    pub fn layout_restored(&self) -> bool {
        self.layout_persistence.restored
    }

    /// Restores the layout `workspace.json` records: the main area's tab
    /// groups and how they are split, the popout windows and theirs, the
    /// active leaf (in either), each sidebar's collapse state and its tab.
    pub(super) fn restore_layout(
        &mut self,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = match state.vault().workspace_json() {
            Ok(Some(layout)) => layout,
            Ok(None) => return,
            Err(err) => {
                self.report(format!("workspace.json could not be read: {err}"), cx);
                return;
            }
        };
        self.apply_layout(layout, state, window, cx);
    }

    /// Restores `layout`, a `workspace.json`, over a main area of one empty group.
    pub(super) fn apply_layout(
        &mut self,
        layout: Value,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layout_persistence.restored = true;
        self.layout_persistence.load_last_open_files(&layout);
        self.layout_persistence.load_file_tree_settings(&layout);
        let sort = self.layout_persistence.file_tree_sort;
        let expanded = self.layout_persistence.file_tree_expanded.clone();
        self.explorer.update(cx, |explorer, cx| {
            explorer.set_sort(sort, cx);
            explorer.set_expanded_folders(&expanded, cx);
        });
        let default_mode = self.default_mode();
        // A leaf naming a file the vault no longer has is dropped, as Obsidian drops one it cannot open.
        let known = |file: &str| state.vault().absolute(file).is_file();
        let spec = layout
            .get("main")
            .and_then(|main| read_main(main, default_mode, &known));
        let windows = layout
            .get("floating")
            .map(|floating| read_floating(floating, default_mode, &known))
            .unwrap_or_default();
        let main_restored = spec.is_some();
        let mut leaf_ids: Vec<(String, u64, usize)> = Vec::new();
        if let Some(spec) = spec {
            // The groups the file describes replace the window's placeholder, whose views were empty.
            let placeholder = self.active_group_id();
            let tree = self.materialize(spec, &mut leaf_ids, window, cx);
            self.layout_tree = tree;
            if let Some(ix) = self.group_ix(placeholder) {
                self.groups.remove(ix);
                self.group_registry.remove(placeholder);
            }
        }
        // The popouts' groups; each window's gpui window opens once this vault window is up.
        let main_leaves = leaf_ids.len();
        for spec in windows {
            let tree = self.materialize(spec.tree, &mut leaf_ids, window, cx);
            let id = self.group_registry.allocate_id();
            self.windows
                .push(WindowNode::from_node(id, tree, spec.node));
        }
        if main_restored || !self.windows.is_empty() {
            // `active` may name a popout's leaf; otherwise the main area's first leaf with a file.
            let active_id = layout.get("active").and_then(Value::as_str);
            let main_leaf_ids = &leaf_ids[..main_leaves];
            let (group, tab) = leaf_ids
                .iter()
                .find(|(id, _, _)| Some(id.as_str()) == active_id)
                .map(|(_, g, t)| (*g, Some(*t)))
                .or_else(|| {
                    main_leaf_ids
                        .iter()
                        .find(|(_, g, t)| {
                            self.group_ix(*g)
                                .is_some_and(|ix| self.groups[ix].state.tabs[*t].path.is_some())
                        })
                        .map(|(_, g, t)| (*g, Some(*t)))
                })
                .or_else(|| main_leaf_ids.first().map(|(_, g, _)| (*g, None)))
                .unwrap_or((self.groups[0].state.id, None));
            if let (Some(ix), Some(tab)) = (self.group_ix(group), tab) {
                self.groups[ix].state.active_tab = tab;
            }
            // Every group's views filled, the active one last so its note is what the
            // explorer and graph follow.
            let mut order = Vec::new();
            self.layout_tree.groups(&mut order);
            for w in &self.windows {
                w.tree.groups(&mut order);
            }
            for &id in order.iter().filter(|id| **id != group).chain([&group]) {
                if let Some(ix) = self.group_ix(id) {
                    self.active_group = ix;
                    self.load_group(ix, cx);
                    self.load_tab(cx);
                    self.stash_active_group();
                }
            }
            // The most recently used order Obsidian would have: the active group first.
            self.group_registry.replace_recent(
                std::iter::once(group).chain(order.into_iter().filter(|g| *g != group)),
            );
            self.focus.refocus_main = true;
            self.open_restored_popouts(cx);
        }
        let collapsed = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("collapsed"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        self.left_dock.set_visible(!collapsed("left"));
        // A dock's `width` is where its handle was last dragged; a fresh vault has 300,
        // and nothing below the minimum is believed.
        let width = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("width"))
                .and_then(Value::as_f64)
                .filter(|w| *w >= super::dock_controls::MIN_SIDEBAR_WIDTH)
        };
        if let Some(w) = width("left") {
            self.left_dock.width = w;
        }
        self.layout_persistence.snapshot = Some(layout);
    }

    /// Makes the groups a `main` node describes, tabs in place. `leaf_ids` collects
    /// each leaf's id with the group and tab it became.
    pub(super) fn materialize(
        &mut self,
        spec: LayoutSpec,
        leaf_ids: &mut Vec<(String, u64, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Layout {
        match spec {
            LayoutSpec::Group(group) => {
                let mode = group
                    .tabs
                    .first()
                    .map(|(_, t)| t.mode)
                    .unwrap_or(ViewMode::Reading);
                let id = self.new_group(mode, window, cx);
                let ix = self.group_ix(id).expect("the group was just made");
                for (tab_ix, (leaf, _)) in group.tabs.iter().enumerate() {
                    leaf_ids.push((leaf.clone(), id, tab_ix));
                }
                let g = &mut self.groups[ix];
                g.state.tabs = group.tabs.into_iter().map(|(_, t)| t).collect();
                g.state.active_tab = group.active_tab.min(g.state.tabs.len().saturating_sub(1));
                g.state.node = group.node;
                Layout::Group(id)
            }
            LayoutSpec::Split {
                direction,
                children,
                node,
            } => Layout::Split {
                direction,
                children: children
                    .into_iter()
                    .map(|c| self.materialize(c, leaf_ids, window, cx))
                    .collect(),
                node,
            },
        }
    }

    /// `workspace.json` as this session would leave it: the file that was
    /// read with the tab groups, the active leaf, the sidebars' collapse
    /// state and `lastOpenFiles` brought up to date, and every other key as
    /// it was.
    pub(super) fn layout_json(&self) -> Value {
        let mut layout = self
            .layout_persistence
            .snapshot
            .clone()
            .unwrap_or_else(|| json!({}));
        let Some(object) = layout.as_object_mut() else {
            return layout;
        };
        // A vault Obsidian never laid out has no docks in the file, or no
        // file at all. Obsidian writes its default ones; so does TFT, so
        // the panes' own state has a leaf to live in rather than being
        // lost, with `main` first the way Obsidian orders the keys.
        if !object.contains_key("main") {
            object.insert("main".into(), Value::Null);
        }
        if !object.get("left").is_some_and(Value::is_object) {
            let json = default_dock_json(self.active_group_state.current.as_deref(), 0);
            object.insert("left".into(), json);
        }
        let mut active_id = Value::Null;
        let main = self.layout_node_json(&self.layout_tree, &mut active_id);
        // `main` keeps its place in a file that had one.
        match object.get_mut("main") {
            Some(slot) => *slot = main,
            None => {
                object.insert("main".into(), main);
            }
        }
        // `floating` while a popout exists, in its place or else right
        // before `active` (after the docks and the ribbon, where Obsidian
        // puts it); no key at all once none does, as Obsidian's own
        // serialization has none then (observed, `s06` of the popout probe).
        let windows: Vec<Value> = self
            .windows
            .iter()
            .map(|w| self.window_json(w, &mut active_id))
            .collect();
        if windows.is_empty() {
            object.shift_remove("floating");
        } else {
            let existing = object.get("floating").and_then(Value::as_object).cloned();
            let floating = floating_json(existing.as_ref(), windows);
            match object.get_mut("floating") {
                Some(slot) => *slot = floating,
                None => {
                    let at = object
                        .keys()
                        .position(|k| k == "active")
                        .unwrap_or(object.len());
                    object.shift_insert(at, "floating".into(), floating);
                }
            }
        }
        object.insert("active".into(), active_id);
        {
            let (dock, visible, width) = ("left", self.left_dock.visible, self.left_dock.width);
            if let Some(d) = object.get_mut(dock).and_then(Value::as_object_mut) {
                if visible {
                    d.shift_remove("collapsed");
                } else {
                    d.insert("collapsed".into(), json!(true));
                }
                // The width is rewritten only once a drag moved it, so the
                // `300` Obsidian wrote stays the integer it was and a file
                // without the key gains it only for a sidebar that moved.
                let same = d
                    .get("width")
                    .and_then(Value::as_f64)
                    .is_some_and(|w| (w - width).abs() < 0.5);
                if !same && (d.contains_key("width") || width != SIDEBAR_WIDTH as f64) {
                    d.insert("width".into(), json!(width));
                }
            }
        }
        object.insert(
            "lastOpenFiles".into(),
            json!(self.layout_persistence.last_open_files),
        );
        let sort = self.layout_persistence.file_tree_sort;
        let expanded = &self.layout_persistence.file_tree_expanded;
        if sort != SortOrder::default() || !expanded.is_empty() {
            let tft = object.entry("tft").or_insert_with(|| json!({}));
            if let Some(tft) = tft.as_object_mut() {
                if sort != SortOrder::default() {
                    tft.insert("fileTreeSort".into(), json!(sort.as_str()));
                }
                if !expanded.is_empty() {
                    let mut expanded: Vec<_> = expanded.iter().collect();
                    expanded.sort();
                    tft.insert("fileTreeExpanded".into(), json!(expanded));
                }
            }
        }
        if let Some(tft) = object.get_mut("tft").and_then(Value::as_object_mut) {
            if sort == SortOrder::default() {
                tft.shift_remove("fileTreeSort");
            }
            if expanded.is_empty() {
                tft.shift_remove("fileTreeExpanded");
            }
        }
        layout
    }

    /// The layout tree for the socket's state dump: group ids at the leaves.
    pub(super) fn layout_tree_json(&self, node: &Layout) -> Value {
        match node {
            Layout::Group(id) => json!({"group": id, "dimension": self.dimension_of(node)}),
            Layout::Split {
                direction,
                children,
                ..
            } => json!({
                "direction": direction.as_str(),
                "dimension": self.dimension_of(node),
                "children": children.iter().map(|c| self.layout_tree_json(c)).collect::<Vec<Value>>(),
            }),
        }
    }

    /// One node of `main`: a group as a `tabs` node with its leaves and
    /// `currentTab`, a split with its children and `direction`, each over
    /// the keys it was read with. The active group's active leaf id lands in
    /// `active_id` on the way, since a new tab's id exists only in the leaf
    /// written here.
    pub(super) fn layout_node_json(&self, node: &Layout, active_id: &mut Value) -> Value {
        match node {
            Layout::Group(id) => {
                let Some(ix) = self.group_ix(*id) else {
                    return Value::Null;
                };
                let active = ix == self.active_group;
                let (tabs, active_tab) = if active {
                    (
                        self.active_group_state.tab_snapshots(),
                        self.active_group_state.active_tab,
                    )
                } else {
                    (
                        self.groups[ix].state.tabs.clone(),
                        self.groups[ix].state.active_tab,
                    )
                };
                let leaves: Vec<Value> = tabs.iter().map(leaf_json).collect();
                if active {
                    *active_id = leaves
                        .get(active_tab)
                        .and_then(|l| l.get("id"))
                        .cloned()
                        .unwrap_or(Value::Null);
                }
                tabs_node_json(self.groups[ix].state.node.clone(), leaves, active_tab)
            }
            Layout::Split {
                direction,
                children,
                node,
            } => split_node_json(
                node.clone(),
                children
                    .iter()
                    .map(|c| self.layout_node_json(c, active_id))
                    .collect(),
                *direction,
            ),
        }
    }

    /// One `window` node of `floating`: the window's own keys around its
    /// groups, written as `main`'s are.
    pub(super) fn window_json(&self, w: &WindowNode, active_id: &mut Value) -> Value {
        let (direction, children) = match &w.tree {
            Layout::Split {
                direction,
                children,
                ..
            } => (*direction, children.iter().collect::<Vec<_>>()),
            // The root is a split by construction; a bare group is wrapped
            // the way Obsidian's root always wraps one.
            group @ Layout::Group(_) => (SplitDirection::Vertical, vec![group]),
        };
        window_node_json(
            &w.node,
            children
                .into_iter()
                .map(|c| self.layout_node_json(c, active_id))
                .collect(),
            direction,
        )
    }

    /// Writes `workspace.json` back if the session changed the layout.
    /// Called on the way out, where Obsidian writes it.
    pub fn save_layout(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let fresh = self.layout_json();
        if self.layout_persistence.snapshot.as_ref() == Some(&fresh) {
            return;
        }
        // The whole file, not a merge into it: `fresh` already carries
        // every key the file had, and a merge would keep a `floating` the
        // last popout's closing took out.
        if let Err(err) = state.vault().replace_config("workspace.json", &fresh) {
            self.report(format!("workspace.json could not be written: {err}"), cx);
            return;
        }
        self.layout_persistence.snapshot = Some(fresh);
    }

    /// Everything that has to reach the disk before the app goes away: the
    /// note's unsaved edits and the layout.
    pub fn on_quit(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        if let Some(path) = self.active_group_state.current.clone() {
            self.save_folds(&path, cx);
        }
        self.save_layout(cx);
    }
}
