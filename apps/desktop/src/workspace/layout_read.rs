//! Pure decoding of persisted workspace layout JSON into application tab/layout values.

use super::release_notes;
use serde_json::{Map, Value};
use tft_app::{HistoryEntry, SplitDirection, Tab, ViewMode};

/// A `tabs` node of `workspace.json` read into tabs, before any group
/// exists for it: each leaf's id with its tab, the `currentTab`, and the
/// node's own keys less `children` and `currentTab`, which are rewritten.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GroupSpec {
    pub(crate) tabs: Vec<(String, Tab)>,
    pub(crate) active_tab: usize,
    pub(crate) node: Map<String, Value>,
}

/// `main` as read: the tree of splits over the groups, without the groups
/// existing yet. Pure, so the reading can be tested without a window.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LayoutSpec {
    Group(GroupSpec),
    Split {
        direction: SplitDirection,
        children: Vec<LayoutSpec>,
        node: Map<String, Value>,
    },
}

/// A `window` node of `floating` as read: its groups as a split whose root
/// `node` map is empty, and the window's own keys beside it. Pure, like
/// [`LayoutSpec`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WindowSpec {
    pub(crate) tree: LayoutSpec,
    pub(crate) node: Map<String, Value>,
}

/// Reads `floating`: every `window` child, read as `main` is read -- a
/// window whose files are all gone is dropped with them -- with its own
/// keys less `children` and `direction`, which the tree carries. A child of
/// any other type is left out; Obsidian writes no other kind.
pub(crate) fn read_floating(
    floating: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Vec<WindowSpec> {
    floating
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|w| w.get("type").and_then(Value::as_str) == Some("window"))
        .filter_map(|w| {
            let object = w.as_object()?;
            let children = read_children(w, default_mode, known);
            if children.is_empty() {
                return None;
            }
            let direction = SplitDirection::parse(w.get("direction").and_then(Value::as_str));
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("direction");
            Some(WindowSpec {
                tree: LayoutSpec::Split {
                    direction,
                    children,
                    node: Map::new(),
                },
                node: own,
            })
        })
        .collect()
}

/// A split's (or a window's) children read, the ones with nothing left
/// dropped.
fn read_children(
    node: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Vec<LayoutSpec> {
    node.get("children")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter_map(|child| read_main(child, default_mode, known))
                .collect()
        })
        .unwrap_or_default()
}

/// Reads `main`. A `tabs` node with no leaf left (every file gone) is
/// dropped, and a split with no child left goes with its children; a split
/// with one child stays nested as Obsidian keeps it. `known` says whether a
/// leaf's file is still in the vault.
pub(crate) fn read_main(
    node: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Option<LayoutSpec> {
    let object = node.as_object()?;
    match node.get("type").and_then(Value::as_str) {
        Some("tabs") => {
            let tabs: Vec<(String, Tab)> = node
                .get("children")
                .and_then(Value::as_array)
                .map(|leaves| {
                    leaves
                        .iter()
                        .filter_map(|leaf| tab_from_leaf(leaf, default_mode, known))
                        .collect()
                })
                .unwrap_or_default();
            if tabs.is_empty() {
                return None;
            }
            let active_tab = node
                .get("currentTab")
                .and_then(Value::as_u64)
                .map(|t| t as usize)
                .unwrap_or(0)
                .min(tabs.len() - 1);
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("currentTab");
            Some(LayoutSpec::Group(GroupSpec {
                tabs,
                active_tab,
                node: own,
            }))
        }
        Some("split") => {
            let children = read_children(node, default_mode, known);
            if children.is_empty() {
                return None;
            }
            let direction = SplitDirection::parse(node.get("direction").and_then(Value::as_str));
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("direction");
            Some(LayoutSpec::Split {
                direction,
                children,
                node: own,
            })
        }
        _ => None,
    }
}

/// The tab a `leaf` node of `workspace.json` stands for, with the leaf's
/// id, or `None` for a view TFT has no tab for or a file that is gone.
pub(crate) fn tab_from_leaf(
    leaf: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Option<(String, Tab)> {
    let id = leaf.get("id").and_then(Value::as_str)?;
    let view = leaf
        .get("state")
        .and_then(|s| s.get("type"))
        .and_then(Value::as_str);
    let inner = leaf.get("state").and_then(|s| s.get("state"));
    let file = inner.and_then(|s| s.get("file")).and_then(Value::as_str);
    let pinned = leaf
        .get("state")
        .and_then(|s| s.get("pinned"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut tab = match (view, file) {
        (_, Some(file)) => {
            if !known(file) {
                return None;
            }
            let mode = match inner.and_then(|s| s.get("mode")).and_then(Value::as_str) {
                Some("preview") => ViewMode::Reading,
                Some("source")
                    if inner
                        .and_then(|s| s.get("source"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false) =>
                {
                    ViewMode::Source
                }
                Some("source") => ViewMode::LivePreview,
                _ => default_mode,
            };
            Tab {
                node: None,
                path: Some(file.to_string()),
                mode,
                history: vec![HistoryEntry::new(file.to_string(), mode)],
                history_ix: 1,
                leaf: Some(leaf.clone()),
                view_state: inner.and_then(|s| s.get("viewState")).cloned(),
                pinned: false,
            }
        }
        (Some("empty"), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(default_mode)
        },
        (Some("graph"), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(ViewMode::Graph)
        },
        (Some(release_notes::VIEW_TYPE), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(default_mode)
        },
        _ => return None,
    };
    tab.pinned = pinned;
    Some((id.to_string(), tab))
}
