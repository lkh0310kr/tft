//! The workspace state machine shared by GPUI and web. No UI and no I/O.

use serde_json::{json, Map, Value};

pub mod effect;
pub mod vault;

pub use effect::{VaultEffect, VaultEvent};
pub use vault::{
    BackendInstanceId, BackendObjectId, FileMeta, LookupError, NameConflict, NodeId, NodeKind,
    RenameError, ScannedNode, VaultData, VaultNode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Reading,
    LivePreview,
    Source,
    Graph,
}

impl ViewMode {
    pub fn is_editing(self) -> bool {
        matches!(self, ViewMode::LivePreview | ViewMode::Source)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::Reading => "reading",
            ViewMode::LivePreview => "live",
            ViewMode::Source => "source",
            ViewMode::Graph => "graph",
        }
    }

    pub fn parse(name: &str) -> Option<ViewMode> {
        match name {
            "reading" => Some(ViewMode::Reading),
            "live" => Some(ViewMode::LivePreview),
            "source" => Some(ViewMode::Source),
            "graph" => Some(ViewMode::Graph),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Files,
    Search,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub node: Option<NodeId>,
    pub path: String,
    pub mode: ViewMode,
    pub offset: usize,
}

impl HistoryEntry {
    pub fn new(path: String, mode: ViewMode) -> Self {
        Self {
            node: None,
            path,
            mode,
            offset: 0,
        }
    }

    pub fn with_node(node: NodeId, path: String, mode: ViewMode, offset: usize) -> Self {
        Self {
            node: Some(node),
            path,
            mode,
            offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub node: Option<NodeId>,
    pub path: Option<String>,
    pub mode: ViewMode,
    pub history: Vec<HistoryEntry>,
    pub history_ix: usize,
    pub leaf: Option<Value>,
    pub view_state: Option<Value>,
    pub pinned: bool,
}

impl Tab {
    pub fn empty(mode: ViewMode) -> Self {
        Self {
            node: None,
            path: None,
            mode,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
            pinned: false,
        }
    }

    pub fn unloaded(path: String, mode: ViewMode) -> Self {
        Self {
            history: vec![HistoryEntry::new(path.clone(), mode)],
            history_ix: 1,
            path: Some(path),
            ..Self::empty(mode)
        }
    }

    pub fn default_title(&self) -> String {
        self.path
            .as_deref()
            .map(|path| {
                let name = path.rsplit('/').next().unwrap_or(path);
                name.strip_suffix(".md").unwrap_or(name).to_string()
            })
            .unwrap_or_else(|| "New tab".to_string())
    }

    pub fn open(&mut self, path: String, mode: ViewMode, offset: usize) {
        self.open_target(None, path, mode, offset);
    }

    pub fn open_node(&mut self, node: NodeId, path: String, mode: ViewMode, offset: usize) {
        self.open_target(Some(node), path, mode, offset);
    }

    fn open_target(&mut self, node: Option<NodeId>, path: String, mode: ViewMode, offset: usize) {
        if self.node == node && self.path.as_deref() == Some(path.as_str()) {
            self.mode = mode;
            if let Some(entry) = self.history.get_mut(self.history_ix.saturating_sub(1)) {
                entry.offset = offset;
                entry.mode = mode;
            }
            return;
        }
        self.history.truncate(self.history_ix);
        self.history.push(HistoryEntry {
            node,
            path: path.clone(),
            mode,
            offset,
        });
        self.history_ix = self.history.len();
        self.node = node;
        self.path = Some(path);
        self.mode = mode;
    }

    pub fn back(&mut self) -> Option<&HistoryEntry> {
        if self.history_ix > 1 {
            self.history_ix -= 1;
            let entry = self.history.get(self.history_ix - 1)?;
            self.node = entry.node;
            self.path = Some(entry.path.clone());
            self.mode = entry.mode;
            Some(entry)
        } else {
            None
        }
    }

    pub fn forward(&mut self) -> Option<&HistoryEntry> {
        if self.history_ix < self.history.len() {
            self.history_ix += 1;
            let entry = self.history.get(self.history_ix - 1)?;
            self.node = entry.node;
            self.path = Some(entry.path.clone());
            self.mode = entry.mode;
            Some(entry)
        } else {
            None
        }
    }
}

/// Puts `tab` after the active one and returns its index.
pub fn place_new_tab(tabs: &mut Vec<Tab>, active: usize, tab: Tab) -> usize {
    let at = (active + 1).min(tabs.len());
    tabs.insert(at, tab);
    at
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

impl SplitDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            SplitDirection::Vertical => "vertical",
            SplitDirection::Horizontal => "horizontal",
        }
    }

    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("horizontal") => SplitDirection::Horizontal,
            _ => SplitDirection::Vertical,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    Group(u64),
    Split {
        direction: SplitDirection,
        children: Vec<Layout>,
        node: Map<String, Value>,
    },
}

impl Layout {
    pub fn split(direction: SplitDirection, children: Vec<Layout>) -> Layout {
        Layout::Split {
            direction,
            children,
            node: Map::from_iter([
                ("id".to_string(), json!(0)),
                ("type".to_string(), json!("split")),
            ]),
        }
    }

    pub fn groups(&self, out: &mut Vec<u64>) {
        match self {
            Layout::Group(id) => out.push(*id),
            Layout::Split { children, .. } => children.iter().for_each(|c| c.groups(out)),
        }
    }

    pub fn top_right(&self) -> Option<u64> {
        match self {
            Layout::Group(id) => Some(*id),
            Layout::Split {
                direction,
                children,
                ..
            } => match direction {
                SplitDirection::Vertical => children.last()?.top_right(),
                SplitDirection::Horizontal => children.first()?.top_right(),
            },
        }
    }

    pub fn contains(&self, id: u64) -> bool {
        match self {
            Layout::Group(g) => *g == id,
            Layout::Split { children, .. } => children.iter().any(|c| c.contains(id)),
        }
    }

    pub fn path_to(&self, id: u64) -> Option<Vec<usize>> {
        match self {
            Layout::Group(g) => (*g == id).then(Vec::new),
            Layout::Split { children, .. } => children.iter().enumerate().find_map(|(ix, c)| {
                let mut path = c.path_to(id)?;
                path.insert(0, ix);
                Some(path)
            }),
        }
    }

    pub fn node_at(&self, path: &[usize]) -> Option<&Layout> {
        match path.split_first() {
            None => Some(self),
            Some((ix, rest)) => match self {
                Layout::Split { children, .. } => children.get(*ix)?.node_at(rest),
                Layout::Group(_) => None,
            },
        }
    }

    pub fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut Layout> {
        match path.split_first() {
            None => Some(self),
            Some((ix, rest)) => match self {
                Layout::Split { children, .. } => children.get_mut(*ix)?.node_at_mut(rest),
                Layout::Group(_) => None,
            },
        }
    }

    pub fn insert_beside(
        &mut self,
        id: u64,
        new: u64,
        direction: SplitDirection,
        after: bool,
    ) -> Option<bool> {
        let Layout::Split {
            direction: own,
            children,
            ..
        } = self
        else {
            return None;
        };
        let ix = children.iter().position(|c| c.contains(id))?;
        if matches!(children[ix], Layout::Group(g) if g == id) {
            if *own == direction {
                children.insert(if after { ix + 1 } else { ix }, Layout::Group(new));
                return Some(false);
            }
            let pair = if after {
                vec![Layout::Group(id), Layout::Group(new)]
            } else {
                vec![Layout::Group(new), Layout::Group(id)]
            };
            children[ix] = Layout::split(direction, pair);
            return Some(true);
        }
        children[ix].insert_beside(id, new, direction, after)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let Layout::Split { children, .. } = self else {
            return false;
        };
        let Some(ix) = children.iter().position(|c| c.contains(id)) else {
            return false;
        };
        if matches!(children[ix], Layout::Group(g) if g == id) {
            children.remove(ix);
            return true;
        }
        children[ix].remove(id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabGroup {
    pub id: u64,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub mode_before_graph: ViewMode,
    pub detached: Option<String>,
    pub node: Map<String, Value>,
}

impl TabGroup {
    pub fn new(id: u64, mode: ViewMode) -> Self {
        Self {
            id,
            tabs: vec![Tab::empty(mode)],
            active_tab: 0,
            mode_before_graph: ViewMode::Reading,
            detached: None,
            node: Map::new(),
        }
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    pub fn open(&mut self, path: String, mode: ViewMode) -> usize {
        self.open_target(None, path, mode)
    }

    pub fn open_node(&mut self, node: NodeId, path: String, mode: ViewMode) -> usize {
        self.open_target(Some(node), path, mode)
    }

    fn open_target(&mut self, node: Option<NodeId>, path: String, mode: ViewMode) -> usize {
        let existing = match node {
            Some(node) => self.tabs.iter().position(|tab| tab.node == Some(node)),
            None => self
                .tabs
                .iter()
                .position(|tab| tab.node.is_none() && tab.path.as_deref() == Some(path.as_str())),
        };
        if let Some(ix) = existing {
            self.active_tab = ix;
            if let Some(tab) = self.tabs.get_mut(ix) {
                tab.mode = mode;
                tab.path = Some(path.clone());
                if let Some(entry) = tab.history.last_mut() {
                    entry.path = path;
                }
            }
            return ix;
        }
        let mut tab = Tab::unloaded(path, mode);
        tab.node = node;
        if let Some(entry) = tab.history.last_mut() {
            entry.node = node;
        }
        let at = place_new_tab(&mut self.tabs, self.active_tab, tab);
        self.active_tab = at;
        at
    }

    pub fn close(&mut self, ix: usize) -> bool {
        if ix >= self.tabs.len() {
            return false;
        }
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::empty(ViewMode::Reading));
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if ix < self.active_tab {
            self.active_tab -= 1;
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceModel {
    pub groups: Vec<TabGroup>,
    pub layout: Layout,
    pub active_group: u64,
    pub left_tab: LeftTab,
    pub left_visible: bool,
}

impl WorkspaceModel {
    pub fn new(mode: ViewMode) -> Self {
        let id = 1;
        Self {
            groups: vec![TabGroup::new(id, mode)],
            layout: Layout::split(SplitDirection::Vertical, vec![Layout::Group(id)]),
            active_group: id,
            left_tab: LeftTab::Files,
            left_visible: true,
        }
    }

    pub fn group(&self, id: u64) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: u64) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    pub fn active(&self) -> Option<&TabGroup> {
        self.group(self.active_group)
    }

    pub fn active_mut(&mut self) -> Option<&mut TabGroup> {
        self.group_mut(self.active_group)
    }

    pub fn open(&mut self, group: u64, path: String, mode: ViewMode) -> Option<usize> {
        self.group_mut(group).map(|g| g.open(path, mode))
    }

    pub fn open_node(
        &mut self,
        group: u64,
        node: NodeId,
        path: String,
        mode: ViewMode,
    ) -> Option<usize> {
        self.group_mut(group).map(|g| g.open_node(node, path, mode))
    }

    pub fn close(&mut self, group: u64, ix: usize) -> bool {
        self.group_mut(group).is_some_and(|g| g.close(ix))
    }

    pub fn split(&mut self, group: u64, direction: SplitDirection, after: bool) -> Option<u64> {
        let new = self.groups.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        let before = self.group(group)?.active()?.mode;
        self.layout.insert_beside(group, new, direction, after)?;
        self.groups.push(TabGroup::new(new, before));
        self.active_group = new;
        Some(new)
    }

    pub fn close_group(&mut self, group: u64) -> bool {
        if self.groups.len() <= 1 || !self.layout.remove(group) {
            return false;
        }
        self.groups.retain(|g| g.id != group);
        if self.active_group == group {
            let mut ids = Vec::new();
            self.layout.groups(&mut ids);
            self.active_group = ids.first().copied().unwrap_or(1);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_open_after_active_and_close_without_losing_one() {
        let mut group = TabGroup::new(1, ViewMode::Reading);
        group.open("A.md".into(), ViewMode::Reading);
        group.open("B.md".into(), ViewMode::Reading);
        assert_eq!(group.tabs.len(), 3);
        assert_eq!(group.active().unwrap().path.as_deref(), Some("B.md"));
        assert!(group.close(2));
        assert_eq!(group.tabs.len(), 2);
        assert!(group.close(1));
        assert!(group.close(0));
        assert_eq!(group.tabs.len(), 1);
        assert_eq!(group.active().unwrap().path, None);
    }

    #[test]
    fn history_back_and_forward_follow_the_model() {
        let mut tab = Tab::empty(ViewMode::Reading);
        tab.open("A.md".into(), ViewMode::Reading, 4);
        tab.open("B.md".into(), ViewMode::Source, 9);
        assert_eq!(tab.back().unwrap().path, "A.md");
        assert_eq!(tab.path.as_deref(), Some("A.md"));
        assert_eq!(tab.forward().unwrap().path, "B.md");
        assert_eq!(tab.path.as_deref(), Some("B.md"));
    }

    #[test]
    fn a_renamed_node_keeps_its_tab() {
        let mut group = TabGroup::new(1, ViewMode::Reading);
        group.open_node(NodeId(7), "A.md".into(), ViewMode::Reading);
        group.open_node(NodeId(7), "B.md".into(), ViewMode::Reading);
        assert_eq!(group.tabs.len(), 2);
        assert_eq!(group.active().unwrap().path.as_deref(), Some("B.md"));
        assert_eq!(group.active().unwrap().node, Some(NodeId(7)));
    }

    #[test]
    fn splitting_moves_the_new_group_into_the_layout() {
        let mut workspace = WorkspaceModel::new(ViewMode::Reading);
        let new = workspace
            .split(1, SplitDirection::Vertical, true)
            .expect("split");
        assert_eq!(workspace.active_group, new);
        let mut groups = Vec::new();
        workspace.layout.groups(&mut groups);
        assert_eq!(groups, vec![1, new]);
    }
}
