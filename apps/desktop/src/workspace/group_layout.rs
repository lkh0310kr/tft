//! Split sizing, divider interaction, and focus navigation between layout groups.

use super::layout_write::set_dimension;
use super::Workspace;
use gpui::Context;
use serde_json::Value;
use tft_app::{split_dimensions_at, Layout, SplitDirection};

/// The payload of a divider drag: which split and the boundary between two children.
#[derive(Debug, Clone)]
pub(super) struct DividerDrag {
    /// The popout window whose tree the split is in; `None` for the main area's.
    pub(super) window: Option<u64>,
    pub(super) path: Vec<usize>,
    pub(super) before: usize,
}

/// The divider being dragged, `(window, split path, before)`. Kept in the
/// workspace so the handle stays accent-coloured while the pointer is over a
/// pane and hover no longer reaches it.
#[derive(Default)]
pub(super) struct DividerDragState {
    pub(super) active: Option<(Option<u64>, Vec<usize>, usize)>,
}

impl Workspace {
    /// How many groups the main area holds (`groups` also counts popouts').
    pub(super) fn main_group_count(&self) -> usize {
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.len()
    }

    /// The `dimension` of a layout node: a group's on the group, a split's on the split.
    pub(super) fn dimension_of(&self, node: &Layout) -> Option<f64> {
        match node {
            Layout::Group(id) => self.groups[self.group_ix(*id)?].dimension(),
            Layout::Split { node, .. } => node.get("dimension").and_then(Value::as_f64),
        }
    }

    /// Sets the `dimension` of the node at `path` of a window's tree (main area for `None`).
    pub(super) fn set_dimension_at(
        &mut self,
        window: Option<u64>,
        path: &[usize],
        dimension: Option<f64>,
    ) {
        match self.tree_mut(window).node_at_mut(path) {
            Some(Layout::Group(id)) => {
                let id = *id;
                if let Some(ix) = self.group_ix(id) {
                    self.groups[ix].set_dimension(dimension);
                }
            }
            Some(Layout::Split { node, .. }) => set_dimension(node, dimension),
            None => {}
        }
    }

    /// After a child left the split at `path`, the rest share its room: scaled to a
    /// hundred, or dropped when one is left, as Obsidian wrote.
    pub(super) fn renormalise_dimensions(&mut self, window: Option<u64>, path: &[usize]) {
        let Some(Layout::Split { children, .. }) = self.tree(window).node_at(path) else {
            return;
        };
        let dims: Vec<Option<f64>> = children.iter().map(|c| self.dimension_of(c)).collect();
        let n = dims.len();
        let scaled: Vec<Option<f64>> = if n <= 1 {
            vec![None; n]
        } else if dims.iter().all(Option::is_some) {
            let sum: f64 = dims.iter().flatten().sum();
            if sum <= 0.0 {
                return;
            }
            dims.iter().map(|d| d.map(|d| d * 100.0 / sum)).collect()
        } else {
            return;
        };
        for (ix, d) in scaled.into_iter().enumerate() {
            let mut child = path.to_vec();
            child.push(ix);
            self.set_dimension_at(window, &child, d);
        }
    }

    /// A divider dragged to `position`: the pair's room is redistributed and every
    /// child's share written as a `dimension`, as Obsidian does.
    pub(super) fn drag_divider(
        &mut self,
        drag: &DividerDrag,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.divider_drag_state.active = Some((drag.window, drag.path.clone(), drag.before));
        let Some(Layout::Split {
            direction,
            children,
            ..
        }) = self.tree(drag.window).node_at(&drag.path)
        else {
            return;
        };
        let n = children.len();
        if drag.before + 1 >= n {
            return;
        }
        let row = *direction == SplitDirection::Vertical;
        let (origin, size) = if row {
            (bounds.origin.x, bounds.size.width)
        } else {
            (bounds.origin.y, bounds.size.height)
        };
        let along = if row { position.x } else { position.y };
        let Some(position) = tft_app::normalized_split_position(
            f32::from(origin) as f64,
            f32::from(size) as f64,
            f32::from(along) as f64,
        ) else {
            return;
        };
        let dims: Vec<f64> = match children
            .iter()
            .map(|c| self.dimension_of(c))
            .collect::<Option<Vec<f64>>>()
        {
            Some(dims) => dims,
            None => vec![100.0 / n as f64; n],
        };
        let Some(dims) = split_dimensions_at(&dims, drag.before, position) else {
            return;
        };
        for (ix, d) in dims.into_iter().enumerate() {
            let mut child = drag.path.clone();
            child.push(ix);
            self.set_dimension_at(drag.window, &child, Some(d));
        }
        cx.notify();
    }

    /// `editor:focus-left` and siblings: the keyboard moves to the group on that side,
    /// the most recently used one.
    pub(super) fn focus_neighbor(
        &mut self,
        direction: SplitDirection,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        // Within the active group's own window: a popout's groups are neighbours of
        // each other, not of the main area's.
        let active = self.active_group_id();
        let candidates = self
            .tree(self.window_of_group(active))
            .neighbors(active, direction, forward);
        let target = self
            .group_registry
            .most_recent_matching(|g| candidates.contains(&g))
            .or_else(|| candidates.first().copied());
        if let Some(target) = target {
            self.activate_group(target, cx);
        }
    }
}
