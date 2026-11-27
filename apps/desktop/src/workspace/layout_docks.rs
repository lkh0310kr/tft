//! Pure helpers for reading and updating dock state in persisted workspace JSON.

use super::{leaf_id, SIDEBAR_WIDTH};
use serde_json::{json, Value};

/// The view type of the sidebar's current tab: the `tabs` group's
/// `currentTab` child (the first when the key is absent).
#[cfg(test)]
pub(crate) fn dock_current_view(layout: &Value, dock: &str) -> Option<String> {
    let mut groups = Vec::new();
    fn find_groups<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        if node.get("type").and_then(Value::as_str) == Some("tabs") {
            out.push(node);
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                find_groups(child, out);
            }
        }
    }
    find_groups(layout.get(dock)?, &mut groups);
    let group = groups.first()?;
    let ix = group.get("currentTab").and_then(Value::as_u64).unwrap_or(0) as usize;
    let leaf = group.get("children")?.as_array()?.get(ix)?;
    leaf.get("state")?.get("type")?.as_str().map(str::to_string)
}

pub(crate) fn default_dock_json(current: Option<&str>, tab: usize) -> Value {
    let leaf = |kind: &str, state: Value, icon: &str, title: &str| {
        let mut state = state;
        if current.is_none() {
            if let Some(map) = state.as_object_mut() {
                map.shift_remove("file");
            }
        }
        json!({
            "id": leaf_id(),
            "type": "leaf",
            "state": {"type": kind, "state": state, "icon": icon, "title": title},
        })
    };
    let children = vec![leaf(
        "file-explorer",
        json!({"showSearch": false, "searchQuery": ""}),
        "lucide-folder-closed",
        "Files",
    )];
    let tab = tab.min(children.len().saturating_sub(1));
    json!({
        "id": leaf_id(),
        "type": "split",
        "children": [{"id": leaf_id(), "type": "tabs", "children": children, "currentTab": tab}],
        "direction": "horizontal",
        "width": SIDEBAR_WIDTH as i64,
    })
}
