//! Workspace application state shared by Desktop and Web. No UI or I/O.

use crate::NodeId;
use serde_json::{json, Map, Value};

/// A platform-neutral request from the Workspace UI to its host. The parent
/// is a Vault-relative path (`""` means the Vault root); filesystem paths and
/// remote object IDs stay inside the platform adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperationRequest {
    CreateNote { parent: String },
    CreateFolder { parent: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Reading,
    LivePreview,
    Source,
    Graph,
    Search,
    /// A terminal pane (`terminal:open`). The shell runs in the Desktop
    /// app; this shared mode names the leaf so tabs close and focus like
    /// the search pane's.
    Terminal,
}

/// Pane limit used by workspace split controls and command availability.
pub const MAX_WORKSPACE_GROUPS: usize = 8;

pub fn workspace_can_split(group_count: usize) -> bool {
    group_count < MAX_WORKSPACE_GROUPS
}

impl ViewMode {
    /// Default mode for a newly opened editable Markdown document.
    pub const fn default_editing() -> Self {
        Self::LivePreview
    }

    /// Returns a saved Markdown view mode, falling back to the shared editing
    /// default for empty tabs and non-document modes such as Graph or Search.
    pub fn markdown_document_mode(mode: Option<Self>) -> Self {
        match mode {
            Some(mode @ (Self::Reading | Self::LivePreview | Self::Source)) => mode,
            _ => Self::default_editing(),
        }
    }

    pub fn is_editing(self) -> bool {
        matches!(self, ViewMode::LivePreview | ViewMode::Source)
    }

    /// Toggles between Reading and the caller's preferred editing mode.
    /// Any non-Reading mode, including Graph, returns to Reading.
    pub fn toggle_reading_editing(self, editing_mode: Self) -> Self {
        if self == Self::Reading {
            editing_mode
        } else {
            Self::Reading
        }
    }

    /// Switches between the two Markdown editing presentations. Reading,
    /// Graph, Search and Terminal are not source/live-preview states.
    pub fn toggle_source_preview(self) -> Option<Self> {
        match self {
            Self::LivePreview => Some(Self::Source),
            Self::Source => Some(Self::LivePreview),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::Reading => "reading",
            ViewMode::LivePreview => "live",
            ViewMode::Source => "source",
            ViewMode::Graph => "graph",
            ViewMode::Search => "search",
            ViewMode::Terminal => "terminal",
        }
    }

    pub fn parse(name: &str) -> Option<ViewMode> {
        match name {
            "reading" => Some(ViewMode::Reading),
            "live" => Some(ViewMode::LivePreview),
            "source" => Some(ViewMode::Source),
            "graph" => Some(ViewMode::Graph),
            "search" => Some(ViewMode::Search),
            "terminal" => Some(ViewMode::Terminal),
            _ => None,
        }
    }
}

/// The filename shown in a workspace tab. Markdown names use the Explorer's
/// normal display rule; canvas, PDF, audio, and video names omit their known
/// format extension as the native workspace does.
pub fn tab_display_name(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if (ext.eq_ignore_ascii_case("canvas")
                || ext.eq_ignore_ascii_case("pdf")
                || matches!(
                    crate::formats::FileKind::from_extension(&ext.to_ascii_lowercase()),
                    crate::formats::FileKind::Audio | crate::formats::FileKind::Video
                ))
                && !stem.is_empty() =>
        {
            stem
        }
        _ => crate::explorer::display_name(file_name),
    }
}

/// The title shared by a workspace tab and its view header.
pub fn workspace_leaf_title(path: Option<&str>, mode: ViewMode) -> String {
    match (path, mode) {
        (Some(path), _) => tab_display_name(path.rsplit('/').next().unwrap_or(path)).to_owned(),
        (None, ViewMode::Graph) => "Graph view".to_owned(),
        (None, ViewMode::Search) => "Search".to_owned(),
        (None, ViewMode::Terminal) => "Terminal".to_owned(),
        (None, _) => "New tab".to_owned(),
    }
}

/// Folder breadcrumbs for a vault-relative file path. Each pair contains the
/// displayed folder name and its vault-relative path, ready for a host to bind
/// to its own navigation action.
pub fn workspace_breadcrumbs(path: Option<&str>) -> Vec<(String, String)> {
    let Some(directory) =
        path.and_then(|path| path.rsplit_once('/').map(|(directory, _)| directory))
    else {
        return Vec::new();
    };

    let mut accumulated = String::new();
    directory
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            if !accumulated.is_empty() {
                accumulated.push('/');
            }
            accumulated.push_str(part);
            (part.to_owned(), accumulated.clone())
        })
        .collect()
}

/// A document header can switch between reading and editing only when it
/// represents an opened file view that supports those modes.
pub fn workspace_header_shows_mode_switch(path: Option<&str>, mode: ViewMode) -> bool {
    path.is_some()
        && !matches!(
            mode,
            ViewMode::Graph | ViewMode::Search | ViewMode::Terminal
        )
}

/// Opening a linked note from a special pane or a pinned tab uses a fresh tab.
/// An explicit new-tab request always takes precedence; reopening the current
/// pinned note stays in place, matching the Desktop workspace rule.
pub fn linked_note_opens_new_tab(
    mode: ViewMode,
    active_tab_pinned: bool,
    active_path: Option<&str>,
    target_path: &str,
    explicitly_new_tab: bool,
) -> bool {
    explicitly_new_tab
        || mode == ViewMode::Terminal
        || (active_tab_pinned && active_path != Some(target_path))
}

/// Chooses the position a document view stores in workspace history. Reading
/// views store their top visible block; editing views store the caret/selection
/// start. Graph tabs use the document mode they had before opening Graph.
pub fn document_history_state(
    mode: ViewMode,
    mode_before_graph: Option<ViewMode>,
    reading_top_offset: Option<usize>,
    selection_start: usize,
) -> (ViewMode, usize) {
    let document_mode = if mode == ViewMode::Graph {
        mode_before_graph.unwrap_or(ViewMode::Reading)
    } else {
        mode
    };
    let offset = if document_mode == ViewMode::Reading {
        reading_top_offset.unwrap_or(0)
    } else {
        selection_start
    };
    (document_mode, offset)
}

/// Resolves a wikilink heading or block subpath to the start offset shared by
/// Desktop and Web. Heading comparison uses the same Markdown parser as the
/// editor and strips inline markup before matching.
pub fn subpath_offset(
    metadata: &tft_cache::CachedMetadata,
    subpath: &tft_markdown::Subpath<'_>,
) -> Option<usize> {
    match subpath {
        tft_markdown::Subpath::Headings(names) => {
            let last = names.last()?;
            metadata
                .headings
                .iter()
                .find(|heading| {
                    let parsed = tft_markdown::parse(&heading.heading);
                    let text = match parsed.blocks.first().map(|block| &block.kind) {
                        Some(tft_markdown::BlockKind::Paragraph { content }) => {
                            tft_markdown::plain_text(content)
                        }
                        _ => heading.heading.clone(),
                    };
                    text.trim().eq_ignore_ascii_case(last.trim())
                })
                .map(|heading| heading.position.start.offset)
        }
        tft_markdown::Subpath::Block(id) => metadata
            .blocks
            .get(&id.to_lowercase())
            .or_else(|| metadata.blocks.get(*id))
            .map(|block| block.position.start.offset),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Files,
    Search,
}

/// Platform-neutral state and transitions for the workspace's left dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceDockState {
    pub visible: bool,
    pub tab: LeftTab,
}

impl Default for WorkspaceDockState {
    fn default() -> Self {
        Self {
            visible: true,
            tab: LeftTab::Files,
        }
    }
}

impl WorkspaceDockState {
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn toggle(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }

    pub fn show(&mut self, tab: LeftTab) {
        self.tab = tab;
        self.visible = true;
    }

    pub fn select(&mut self, tab: LeftTab) {
        self.tab = tab;
    }
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

/// Moves a one-based history cursor in the requested direction.
/// Returns the new cursor, or `None` when the cursor cannot move.
pub fn history_step(
    history_ix: usize,
    history_len: usize,
    direction: HistoryDirection,
) -> Option<usize> {
    match direction {
        HistoryDirection::Forward => history_ix
            .checked_add(1)
            .filter(|next| *next <= history_len),
        HistoryDirection::Back => history_ix.checked_sub(1).filter(|previous| *previous >= 1),
    }
}

/// Drops the forward branch and appends a new current history entry.
pub fn push_history_entry(
    history: &mut Vec<HistoryEntry>,
    history_ix: &mut usize,
    entry: HistoryEntry,
) {
    history.truncate(*history_ix);
    history.push(entry);
    *history_ix = history.len();
}

/// Returns the current history entry for the one-based cursor used by tabs.
pub fn current_history_entry_mut(
    history: &mut [HistoryEntry],
    history_ix: usize,
) -> Option<&mut HistoryEntry> {
    history.get_mut(history_ix.checked_sub(1)?)
}

/// Updates the visible view state only when the history cursor still points at
/// the expected note. Hosts use this when saving a caret or reading position
/// before switching modes or navigating elsewhere.
pub fn update_history_entry_state(
    history: &mut [HistoryEntry],
    history_ix: usize,
    expected_path: &str,
    mode: ViewMode,
    offset: usize,
) -> bool {
    let Some(entry) = current_history_entry_mut(history, history_ix) else {
        return false;
    };
    if entry.path != expected_path {
        return false;
    }
    entry.mode = mode;
    entry.offset = offset;
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDirection {
    Back,
    Forward,
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

/// Session-local history for recently closed tabs. Hosts decide whether a
/// special tab is restorable; the shared history owns ordering and retention.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClosedTabHistory {
    tabs: Vec<(usize, Tab)>,
}

impl ClosedTabHistory {
    pub const LIMIT: usize = 20;

    pub fn remember(&mut self, index: usize, tab: Tab, restorable: bool) {
        if !restorable {
            return;
        }
        self.tabs.push((index, tab));
        if self.tabs.len() > Self::LIMIT {
            self.tabs.remove(0);
        }
    }

    pub fn take_last(&mut self) -> Option<(usize, Tab)> {
        self.tabs.pop()
    }

    pub fn clear(&mut self) {
        self.tabs.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

/// Platform-neutral identity of a tab being dragged. Each host adds its own
/// visual preview and computes drop geometry in its native coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabDragSource {
    pub group_id: u64,
    pub tab_index: usize,
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
        if self.mode == ViewMode::Search {
            return "Search".to_string();
        }
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
            if let Some(entry) = current_history_entry_mut(&mut self.history, self.history_ix) {
                entry.offset = offset;
                entry.mode = mode;
            }
            return;
        }
        push_history_entry(
            &mut self.history,
            &mut self.history_ix,
            HistoryEntry {
                node,
                path: path.clone(),
                mode,
                offset,
            },
        );
        self.node = node;
        self.path = Some(path);
        self.mode = mode;
    }

    pub fn back(&mut self) -> Option<&HistoryEntry> {
        self.history_ix =
            history_step(self.history_ix, self.history.len(), HistoryDirection::Back)?;
        let entry = self.history.get(self.history_ix - 1)?;
        self.node = entry.node;
        self.path = Some(entry.path.clone());
        self.mode = entry.mode;
        Some(entry)
    }

    pub fn forward(&mut self) -> Option<&HistoryEntry> {
        self.history_ix = history_step(
            self.history_ix,
            self.history.len(),
            HistoryDirection::Forward,
        )?;
        let entry = self.history.get(self.history_ix - 1)?;
        self.node = entry.node;
        self.path = Some(entry.path.clone());
        self.mode = entry.mode;
        Some(entry)
    }
}

/// Puts `tab` after the active one and returns its index.
pub fn place_new_tab(tabs: &mut Vec<Tab>, active: usize, tab: Tab) -> usize {
    let at = (active + 1).min(tabs.len());
    tabs.insert(at, tab);
    at
}

/// Appends a fresh empty tab and returns its active index. Strip `+` actions
/// use append order; navigation-created tabs continue to use `place_new_tab`.
pub fn append_empty_tab(tabs: &mut Vec<Tab>, mode: ViewMode) -> usize {
    tabs.push(Tab::empty(mode));
    tabs.len() - 1
}

/// Removes a tab and keeps the active index on the same tab when possible.
/// The caller decides what an empty tab strip means for its platform.
pub fn remove_tab_at(tabs: &mut Vec<Tab>, active: &mut usize, ix: usize) -> Option<Tab> {
    if ix >= tabs.len() {
        return None;
    }
    let removed = tabs.remove(ix);
    if tabs.is_empty() {
        *active = 0;
    } else if *active >= tabs.len() {
        *active = tabs.len() - 1;
    } else if ix < *active {
        *active -= 1;
    }
    Some(removed)
}

/// Inserts a transferred tab at a strip boundary and makes it active. The
/// boundary is clamped so a stale drop index safely means append.
pub fn insert_tab_at_boundary(
    tabs: &mut Vec<Tab>,
    active: &mut usize,
    boundary: usize,
    tab: Tab,
) -> usize {
    let index = boundary.min(tabs.len());
    tabs.insert(index, tab);
    *active = index;
    index
}

/// Moves a tab between two distinct tab strips, activates it in the target,
/// and reports whether the source became empty. Both workspace hosts use this
/// transition while retaining ownership of their live view and layout updates.
pub fn transfer_tab_between_strips(
    source_tabs: &mut Vec<Tab>,
    source_active: &mut usize,
    source_index: usize,
    target_tabs: &mut Vec<Tab>,
    target_active: &mut usize,
    boundary: usize,
) -> Option<TabTransferOutcome> {
    let tab = remove_tab_at(source_tabs, source_active, source_index)?;
    let source_empty = source_tabs.is_empty();
    let target_index = insert_tab_at_boundary(target_tabs, target_active, boundary, tab);
    Some(TabTransferOutcome {
        source_empty,
        target_index,
    })
}

/// Moves a tab to the insertion boundary in the same tab strip and activates it.
/// `boundary` is measured in the original list and may equal `tabs.len()` to
/// place the tab last.
pub fn move_tab_to_boundary(
    tabs: &mut Vec<Tab>,
    active: &mut usize,
    source: usize,
    boundary: usize,
) -> Option<usize> {
    if source >= tabs.len() || boundary > tabs.len() {
        return None;
    }
    let tab = tabs.remove(source);
    let destination = if source < boundary {
        boundary - 1
    } else {
        boundary
    };
    tabs.insert(destination, tab);
    *active = destination;
    Some(destination)
}

/// Finds the nearest insertion boundary from measured tab boxes. Each box is
/// `(tab_index, left_x, right_x)` in the same coordinate space as `pointer_x`.
/// Hosts can supply one hovered box for local half-tab hit testing or every
/// box for a strip-wide nearest-edge target.
pub fn nearest_tab_drop_boundary(
    pointer_x: f32,
    tabs: &[(usize, f32, f32)],
) -> Option<(usize, f32)> {
    let (last_index, _, last_right) = tabs.iter().max_by_key(|(index, _, _)| *index)?;
    tabs.iter()
        .map(|(index, left, _)| (*index, *left))
        .chain(std::iter::once((*last_index + 1, *last_right)))
        .min_by(|left, right| {
            (left.1 - pointer_x)
                .abs()
                .total_cmp(&(right.1 - pointer_x).abs())
        })
}

/// Selects a tab index when it is within the current tab list.
pub fn activate_tab_index(tab_count: usize, active: &mut usize, ix: usize) -> bool {
    if ix >= tab_count {
        return false;
    }
    *active = ix;
    true
}

/// A tab-strip context menu action that closes a set of tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCloseAction {
    Others,
    After,
    All,
}

/// Whether a close action has any meaningful rows for the selected tab.
pub fn tab_close_action_available(
    action: TabCloseAction,
    tab_count: usize,
    selected: usize,
) -> bool {
    match action {
        TabCloseAction::Others | TabCloseAction::All => tab_count > 1,
        TabCloseAction::After => selected.saturating_add(1) < tab_count,
    }
}

/// The pinned-tab rule shared by Desktop and Web tab menus. The active target
/// remains for Close others; Close after preserves tabs through the target;
/// Close all preserves only pinned tabs.
pub fn tab_close_action_keeps(
    action: TabCloseAction,
    index: usize,
    selected: usize,
    tab: &Tab,
) -> bool {
    match action {
        TabCloseAction::Others => index == selected || tab.pinned,
        TabCloseAction::After => index <= selected || tab.pinned,
        TabCloseAction::All => tab.pinned,
    }
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

/// Converts a pointer coordinate on one split axis to a normalized position.
/// Returns `None` for invalid geometry so a host can ignore a stale drag.
pub fn normalized_split_position(origin: f64, size: f64, coordinate: f64) -> Option<f64> {
    if !origin.is_finite() || !size.is_finite() || size <= 0.0 || !coordinate.is_finite() {
        return None;
    }
    Some(((coordinate - origin) / size).clamp(0.0, 1.0))
}

/// Rebalances the two children adjacent to a split divider at a normalized
/// pointer position. Shares are preserved for other children and each side
/// keeps at least five percent of the split's total size.
pub fn split_dimensions_at(dimensions: &[f64], before: usize, position: f64) -> Option<Vec<f64>> {
    if before >= dimensions.len().saturating_sub(1)
        || !position.is_finite()
        || dimensions
            .iter()
            .any(|dimension| !dimension.is_finite() || *dimension <= 0.0)
    {
        return None;
    }
    let total = dimensions.iter().sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    let minimum = total * 0.05;
    let pair = dimensions[before] + dimensions[before + 1];
    if pair <= minimum * 2.0 {
        return None;
    }
    let preceding = dimensions[..before].iter().sum::<f64>();
    let first = (position.clamp(0.0, 1.0) * total - preceding).clamp(minimum, pair - minimum);
    let mut resized = dimensions.to_vec();
    resized[before] = first;
    resized[before + 1] = pair - first;
    Some(resized)
}

/// Advances a selected list index. Desktop search uses a clamped cursor that
/// clears when moving above the first row; palette-style lists wrap around.
pub fn selection_step(
    selected: Option<usize>,
    item_count: usize,
    delta: isize,
    wrap: bool,
) -> Option<usize> {
    if item_count == 0 || delta == 0 {
        return None;
    }
    if wrap {
        let current = selected
            .map(|index| index % item_count)
            .unwrap_or(if delta > 0 { item_count - 1 } else { 0 });
        return Some(if delta > 0 {
            (current + 1) % item_count
        } else {
            current.checked_sub(1).unwrap_or(item_count - 1)
        });
    }
    match selected {
        None if delta > 0 => Some(0),
        None => None,
        Some(index) if delta < 0 && index == 0 => None,
        Some(index) if delta < 0 => Some(index - 1),
        Some(index) => Some((index + 1).min(item_count - 1)),
    }
}

/// Returns the adjacent tab index, wrapping at either end of a tab strip.
pub fn cycle_tab_index(active: usize, tab_count: usize, forward: bool) -> Option<usize> {
    if tab_count <= 1 {
        return None;
    }
    let active = active.min(tab_count - 1);
    Some(if forward {
        (active + 1) % tab_count
    } else {
        active.checked_sub(1).unwrap_or(tab_count - 1)
    })
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
        Self::split_with_node(
            direction,
            children,
            Map::from_iter([
                ("id".to_string(), json!(0)),
                ("type".to_string(), json!("split")),
            ]),
        )
    }

    /// Creates a split with metadata supplied by the host that persists layouts.
    pub fn split_with_node(
        direction: SplitDirection,
        children: Vec<Layout>,
        node: Map<String, Value>,
    ) -> Layout {
        Layout::Split {
            direction,
            children,
            node,
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
        self.insert_beside_with_node(
            id,
            new,
            direction,
            after,
            Map::from_iter([
                ("id".to_string(), json!(0)),
                ("type".to_string(), json!("split")),
            ]),
        )
    }

    /// Inserts a group beside another, using host-supplied metadata if a new split is needed.
    pub fn insert_beside_with_node(
        &mut self,
        id: u64,
        new: u64,
        direction: SplitDirection,
        after: bool,
        split_node: Map<String, Value>,
    ) -> Option<bool> {
        let Layout::Split {
            direction: own,
            children,
            node,
        } = self
        else {
            return None;
        };
        let ix = children.iter().position(|c| c.contains(id))?;
        if matches!(children[ix], Layout::Group(g) if g == id) {
            if *own == direction {
                let old_len = children.len();
                let insertion = if after { ix + 1 } else { ix };
                let dimensions = layout_dimensions(node, old_len);
                children.insert(insertion, Layout::Group(new));
                if let Some(mut dimensions) = dimensions {
                    let half = dimensions[ix] / 2.0;
                    dimensions[ix] = half;
                    dimensions.insert(insertion, half);
                    node.insert("dimensions".into(), json!(dimensions));
                }
                return Some(false);
            }
            let pair = if after {
                vec![Layout::Group(id), Layout::Group(new)]
            } else {
                vec![Layout::Group(new), Layout::Group(id)]
            };
            children[ix] = Layout::split_with_node(direction, pair, split_node);
            return Some(true);
        }
        children[ix].insert_beside_with_node(id, new, direction, after, split_node)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let Layout::Split { children, node, .. } = self else {
            return false;
        };
        let Some(ix) = children.iter().position(|c| c.contains(id)) else {
            return false;
        };
        if matches!(children[ix], Layout::Group(g) if g == id) {
            let old_len = children.len();
            let dimensions = layout_dimensions(node, old_len);
            children.remove(ix);
            if let Some(mut dimensions) = dimensions {
                dimensions.remove(ix);
                node.insert("dimensions".into(), json!(dimensions));
            }
            return true;
        }
        let removed = children[ix].remove(id);
        if removed && matches!(&children[ix], Layout::Split { children, .. } if children.is_empty())
        {
            let old_len = children.len();
            let dimensions = layout_dimensions(node, old_len);
            children.remove(ix);
            if let Some(mut dimensions) = dimensions {
                dimensions.remove(ix);
                node.insert("dimensions".into(), json!(dimensions));
            }
        }
        removed
    }

    /// Returns the nearest sibling group set in the requested direction.
    pub fn neighbors(&self, id: u64, direction: SplitDirection, forward: bool) -> Vec<u64> {
        let Some(path) = self.path_to(id) else {
            return Vec::new();
        };
        for depth in (0..path.len()).rev() {
            let Some(Layout::Split {
                direction: own,
                children,
                ..
            }) = self.node_at(&path[..depth])
            else {
                continue;
            };
            if *own != direction {
                continue;
            }
            let ix = path[depth];
            let sibling = if forward {
                children.get(ix + 1)
            } else {
                ix.checked_sub(1).and_then(|i| children.get(i))
            };
            if let Some(sibling) = sibling {
                let mut out = Vec::new();
                sibling.groups(&mut out);
                return out;
            }
        }
        Vec::new()
    }
}

/// Reads only a well-formed child-size vector. Unknown or malformed layout
/// metadata is left untouched so a host can retain its own representation.
fn layout_dimensions(node: &Map<String, Value>, child_count: usize) -> Option<Vec<f64>> {
    let dimensions = node.get("dimensions")?.as_array()?;
    if dimensions.len() != child_count {
        return None;
    }
    let dimensions = dimensions
        .iter()
        .map(|value| value.as_f64())
        .collect::<Option<Vec<_>>>()?;
    dimensions
        .iter()
        .all(|dimension| dimension.is_finite() && *dimension > 0.0)
        .then_some(dimensions)
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

    /// Selects a tab by its current strip index.
    pub fn activate(&mut self, ix: usize) -> bool {
        activate_tab_index(self.tabs.len(), &mut self.active_tab, ix)
    }

    /// Toggles a tab's pinned state and returns the new value.
    pub fn toggle_pin(&mut self, ix: usize) -> Option<bool> {
        let tab = self.tabs.get_mut(ix)?;
        tab.pinned = !tab.pinned;
        Some(tab.pinned)
    }

    pub fn open(&mut self, path: String, mode: ViewMode) -> usize {
        self.open_target(None, path, mode)
    }

    pub fn open_node(&mut self, node: NodeId, path: String, mode: ViewMode) -> usize {
        self.open_target(Some(node), path, mode)
    }

    /// Opens a fresh tab for `node`, even when another tab already shows it.
    pub fn open_node_in_new_tab(&mut self, node: NodeId, path: String, mode: ViewMode) -> usize {
        let mut tab = Tab::unloaded(path, mode);
        tab.node = Some(node);
        if let Some(entry) = tab.history.last_mut() {
            entry.node = Some(node);
        }
        let at = place_new_tab(&mut self.tabs, self.active_tab, tab);
        self.active_tab = at;
        at
    }

    /// Navigates the active tab to a node and appends its navigation history.
    pub fn navigate_node(&mut self, node: NodeId, path: String, mode: ViewMode) -> bool {
        let Some(tab) = self.active_mut() else {
            return false;
        };
        tab.open_node(node, path, mode, 0);
        true
    }

    pub fn back(&mut self) -> Option<HistoryEntry> {
        self.active_mut()?.back().cloned()
    }

    pub fn forward(&mut self) -> Option<HistoryEntry> {
        self.active_mut()?.forward().cloned()
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
                if let Some(entry) = current_history_entry_mut(&mut tab.history, tab.history_ix) {
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
        if remove_tab_at(&mut self.tabs, &mut self.active_tab, ix).is_none() {
            return false;
        }
        if self.tabs.is_empty() {
            self.tabs.push(Tab::empty(ViewMode::Reading));
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceModel {
    pub groups: Vec<TabGroup>,
    pub layout: Layout,
    pub active_group: u64,
    pub dock: WorkspaceDockState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabTransferOutcome {
    pub source_empty: bool,
    pub target_index: usize,
}

impl WorkspaceModel {
    pub fn new(mode: ViewMode) -> Self {
        let id = 1;
        Self {
            groups: vec![TabGroup::new(id, mode)],
            layout: Layout::split(SplitDirection::Vertical, vec![Layout::Group(id)]),
            active_group: id,
            dock: WorkspaceDockState::default(),
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

    /// Activates a group only when it is present in both the model and layout.
    pub fn activate_group(&mut self, group: u64) -> bool {
        if self.group(group).is_none() || !self.layout.contains(group) {
            return false;
        }
        self.active_group = group;
        true
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

    pub fn open_node_in_new_tab(
        &mut self,
        group: u64,
        node: NodeId,
        path: String,
        mode: ViewMode,
    ) -> Option<usize> {
        self.group_mut(group)
            .map(|g| g.open_node_in_new_tab(node, path, mode))
    }

    /// Appends an empty tab to the active group and makes it active.
    pub fn new_tab(&mut self, mode: ViewMode) -> Option<usize> {
        let group = self.active_mut()?;
        let index = append_empty_tab(&mut group.tabs, mode);
        group.active_tab = index;
        Some(index)
    }

    pub fn navigate_node(
        &mut self,
        group: u64,
        node: NodeId,
        path: String,
        mode: ViewMode,
    ) -> bool {
        self.group_mut(group)
            .is_some_and(|g| g.navigate_node(node, path, mode))
    }

    pub fn back(&mut self, group: u64) -> Option<HistoryEntry> {
        self.group_mut(group)?.back()
    }

    pub fn forward(&mut self, group: u64) -> Option<HistoryEntry> {
        self.group_mut(group)?.forward()
    }

    pub fn close(&mut self, group: u64, ix: usize) -> bool {
        self.group_mut(group).is_some_and(|g| g.close(ix))
    }

    pub fn activate_tab(&mut self, group: u64, ix: usize) -> bool {
        self.group_mut(group).is_some_and(|g| g.activate(ix))
    }

    pub fn toggle_tab_pin(&mut self, group: u64, ix: usize) -> Option<bool> {
        self.group_mut(group)?.toggle_pin(ix)
    }

    pub fn split(&mut self, group: u64, direction: SplitDirection, after: bool) -> Option<u64> {
        let new = self.groups.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        let before = self.group(group)?.active()?.mode;
        self.layout.insert_beside(group, new, direction, after)?;
        self.groups.push(TabGroup::new(new, before));
        self.active_group = new;
        Some(new)
    }

    /// Splits beside `group`, copies its current tab into the new group, and
    /// activates that group. Host views are created separately from the
    /// returned group state.
    pub fn split_with_active_tab(
        &mut self,
        group: u64,
        direction: SplitDirection,
        after: bool,
    ) -> Option<u64> {
        let active = self.group(group)?.active()?.clone();
        let new = self.split(group, direction, after)?;
        let target = self.group_mut(new)?;
        target.tabs = vec![active];
        target.active_tab = 0;
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

    /// Resizes the two children adjacent to a divider in the layout tree.
    /// `position` is normalized across the whole split and the same minimum
    /// pane share used by both hosts is enforced by `split_dimensions_at`.
    pub fn resize_split_at(&mut self, path: &[usize], before: usize, position: f64) -> bool {
        let Some(Layout::Split { children, node, .. }) = self.layout.node_at(path) else {
            return false;
        };
        let count = children.len();
        let dimensions = layout_dimensions(node, count).unwrap_or_else(|| vec![1.0; count]);
        let Some(dimensions) = split_dimensions_at(&dimensions, before, position) else {
            return false;
        };
        let Some(Layout::Split { node, .. }) = self.layout.node_at_mut(path) else {
            return false;
        };
        node.insert("dimensions".into(), json!(dimensions));
        true
    }

    /// Transfers a tab between distinct groups, activates it in the target,
    /// and removes the source group if its last tab moved away. UI hosts keep
    /// ownership of editor/view synchronization and persistence.
    pub fn transfer_tab(
        &mut self,
        source_group: u64,
        source_index: usize,
        target_group: u64,
        boundary: usize,
    ) -> Option<TabTransferOutcome> {
        if source_group == target_group
            || self.group(source_group).is_none()
            || self.group(target_group).is_none()
            || !self.layout.contains(source_group)
            || !self.layout.contains(target_group)
        {
            return None;
        }
        let source_index_in_model = self
            .groups
            .iter()
            .position(|group| group.id == source_group)?;
        let target_index_in_model = self
            .groups
            .iter()
            .position(|group| group.id == target_group)?;
        let outcome = if source_index_in_model < target_index_in_model {
            let (left, right) = self.groups.split_at_mut(target_index_in_model);
            transfer_tab_between_strips(
                &mut left[source_index_in_model].tabs,
                &mut left[source_index_in_model].active_tab,
                source_index,
                &mut right[0].tabs,
                &mut right[0].active_tab,
                boundary,
            )?
        } else {
            let (left, right) = self.groups.split_at_mut(source_index_in_model);
            transfer_tab_between_strips(
                &mut right[0].tabs,
                &mut right[0].active_tab,
                source_index,
                &mut left[target_index_in_model].tabs,
                &mut left[target_index_in_model].active_tab,
                boundary,
            )?
        };
        let source_empty = outcome.source_empty;
        if source_empty && !self.close_group(source_group) {
            return None;
        }
        if !self.activate_group(target_group) {
            return None;
        }
        Some(TabTransferOutcome {
            source_empty,
            target_index: outcome.target_index,
        })
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
    fn linked_new_tab_keeps_existing_node_tab_and_opens_duplicate_after_active() {
        let mut group = TabGroup::new(1, ViewMode::Source);
        group
            .active_mut()
            .unwrap()
            .open_node(NodeId(1), "A.md".into(), ViewMode::Source, 0);
        group.open_node(NodeId(2), "B.md".into(), ViewMode::Source);

        let ix = group.open_node_in_new_tab(NodeId(2), "B.md".into(), ViewMode::Source);

        assert_eq!(ix, 2);
        assert_eq!(group.tabs.len(), 3);
        assert_eq!(group.tabs[1].node, Some(NodeId(2)));
        assert_eq!(group.tabs[2].node, Some(NodeId(2)));
        assert_eq!(group.active_tab, 2);
        assert_eq!(group.active().unwrap().history[0].node, Some(NodeId(2)));
    }

    #[test]
    fn following_a_link_and_history_navigation_stay_in_the_active_tab() {
        let mut group = TabGroup::new(1, ViewMode::Reading);
        group
            .active_mut()
            .unwrap()
            .open_node(NodeId(1), "A.md".into(), ViewMode::Reading, 0);
        assert!(group.navigate_node(NodeId(2), "B.md".into(), ViewMode::Reading));

        assert_eq!(group.tabs.len(), 1);
        assert_eq!(group.active().unwrap().path.as_deref(), Some("B.md"));
        assert_eq!(group.back().unwrap().path, "A.md");
        assert_eq!(group.forward().unwrap().path, "B.md");
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
