//! Pure encoding of tab and layout state into persisted workspace JSON.

use super::ViewMode;
use super::release_notes;
use super::tab_strip::TabTitle;
use serde_json::{Map, Value, json};
use tft_app::{Layout, SplitDirection, Tab};
use tft_formats::FileKind;

pub(crate) fn desktop_split_node() -> Map<String, Value> {
    Map::from_iter([
        ("id".into(), json!(leaf_id())),
        ("type".into(), json!("split")),
    ])
}

/// Creates a layout split with the unique metadata required by the Desktop file format.
pub(super) fn desktop_split(direction: SplitDirection, children: Vec<Layout>) -> Layout {
    Layout::split_with_node(direction, children, desktop_split_node())
}

/// Writes or drops a node's `dimension`, in the slot Obsidian gives it: after `type`.
pub(super) fn set_dimension(node: &mut Map<String, Value>, dimension: Option<f64>) {
    match dimension {
        Some(d) => {
            if node.contains_key("dimension") {
                node["dimension"] = json!(d);
            } else {
                let at = node
                    .keys()
                    .position(|k| k == "type")
                    .map(|i| i + 1)
                    .unwrap_or(node.len());
                node.shift_insert(at, "dimension".into(), json!(d));
            }
        }
        None => {
            node.shift_remove("dimension");
        }
    }
}

/// A fresh leaf id in Obsidian's shape: sixteen hex digits.
pub(crate) fn leaf_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ (COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    format!("{x:016x}")
}

/// A `tabs` node over its leaves: the keys it was read with (or a fresh
/// `id`), `children` and `currentTab` rewritten at the end, the latter only
/// when it is not the first (as Obsidian leaves it out then). A group read
/// stacked comes back horizontal: `"stacked"` is dropped rather than kept.
pub(crate) fn tabs_node_json(
    mut map: Map<String, Value>,
    leaves: Vec<Value>,
    active_tab: usize,
) -> Value {
    if !map.contains_key("id") {
        map.shift_insert(0, "id".into(), json!(leaf_id()));
    }
    map.insert("type".into(), json!("tabs"));
    map.shift_remove("children");
    map.shift_remove("currentTab");
    map.shift_remove("stacked");
    map.insert("children".into(), Value::Array(leaves));
    if active_tab > 0 {
        map.insert("currentTab".into(), json!(active_tab));
    }
    Value::Object(map)
}

/// A `split` node over its children, `direction` after them as Obsidian
/// writes it.
pub(crate) fn split_node_json(
    mut map: Map<String, Value>,
    children: Vec<Value>,
    direction: SplitDirection,
) -> Value {
    if !map.contains_key("id") {
        map.shift_insert(0, "id".into(), json!(leaf_id()));
    }
    map.insert("type".into(), json!("split"));
    map.shift_remove("children");
    map.shift_remove("direction");
    map.insert("children".into(), Value::Array(children));
    map.insert("direction".into(), json!(direction.as_str()));
    Value::Object(map)
}

/// A `window` node of `floating`, in Obsidian's key order: `id`, `type`,
/// `children`, `direction`, then the rest of the window's own keys (the
/// bounds, `maximize`, `zoom`, anything unknown) as they came.
pub(crate) fn window_node_json(
    own: &Map<String, Value>,
    children: Vec<Value>,
    direction: SplitDirection,
) -> Value {
    let mut map = Map::new();
    map.insert(
        "id".into(),
        own.get("id").cloned().unwrap_or_else(|| json!(leaf_id())),
    );
    map.insert("type".into(), json!("window"));
    map.insert("children".into(), Value::Array(children));
    map.insert("direction".into(), json!(direction.as_str()));
    for (key, value) in own {
        if !matches!(key.as_str(), "id" | "type" | "children" | "direction") {
            map.insert(key.clone(), value.clone());
        }
    }
    Value::Object(map)
}

/// The `floating` split over the windows: the node the file had with its
/// children replaced, or a fresh one.
pub(crate) fn floating_json(existing: Option<&Map<String, Value>>, windows: Vec<Value>) -> Value {
    let mut map = existing.cloned().unwrap_or_default();
    if !map.contains_key("id") {
        map.shift_insert(0, "id".into(), json!(leaf_id()));
    }
    map.insert("type".into(), json!("floating"));
    map.shift_remove("children");
    map.insert("children".into(), Value::Array(windows));
    Value::Object(map)
}

/// The `workspace.json` leaf for a tab: the one it was restored from with
/// its state brought up to date, or a new one.
pub(crate) fn leaf_json(tab: &Tab) -> Value {
    let (mode, source) = match tab.mode {
        ViewMode::Reading => ("preview", false),
        ViewMode::Source => ("source", true),
        ViewMode::LivePreview | ViewMode::Graph | ViewMode::Search => ("source", false),
    };
    let title = tab.title();
    let mut leaf = tab
        .leaf
        .clone()
        .filter(|l| {
            let file = l
                .get("state")
                .and_then(|s| s.get("state"))
                .and_then(|s| s.get("file"))
                .and_then(Value::as_str);
            let view = l
                .get("state")
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str);
            match (&tab.path, tab.mode) {
                (Some(p), _) => file == Some(p.as_str()),
                (None, ViewMode::Graph) => view == Some("graph"),
                (None, _) if release_notes::version_of(tab).is_some() => {
                    view == Some(release_notes::VIEW_TYPE)
                }
                (None, _) => view == Some("empty"),
            }
        })
        .unwrap_or_else(|| match (&tab.path, tab.mode) {
            (Some(p), _) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {
                    "type": leaf_view_type(p),
                    "state": {"file": p},
                    "icon": leaf_icon(p),
                    "title": title,
                },
            }),
            (None, ViewMode::Graph) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "graph", "state": {}, "icon": "lucide-git-fork", "title": title},
            }),
            (None, ViewMode::Search) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "empty", "state": {}, "icon": "lucide-file", "title": "New tab"},
            }),
            (None, _) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "empty", "state": {}, "icon": "lucide-file", "title": title},
            }),
        });
    if let Some(path) = &tab.path {
        if leaf_view_type(path) == "markdown" {
            if let Some(inner) = leaf
                .get_mut("state")
                .and_then(|s| s.get_mut("state"))
                .and_then(Value::as_object_mut)
            {
                inner.insert("mode".into(), json!(mode));
                inner.insert("source".into(), json!(source));
            }
        }
        // A canvas leaf records where the view looked, when it is known; a
        // tab restored with a `viewState` and never moved keeps the one it
        // was read with.
        if leaf_view_type(path) == "canvas" {
            if let Some(view_state) = &tab.view_state {
                if let Some(inner) = leaf
                    .get_mut("state")
                    .and_then(|s| s.get_mut("state"))
                    .and_then(Value::as_object_mut)
                {
                    inner.insert("viewState".into(), view_state.clone());
                }
            }
        }
        if let Some(state) = leaf.get_mut("state").and_then(Value::as_object_mut) {
            state.insert("title".into(), json!(title));
        }
    }
    set_pinned(&mut leaf, tab.pinned);
    leaf
}

/// Obsidian writes a pinned leaf's `"pinned": true` twice — on the leaf
/// after `type`, and in its `state` after the inner `state` — and an
/// unpinned one without either key, never `false` (observed,
/// `tools/reference-probe/results/2026-11-09-tab-header-menu.json`). A key
/// already in place is left where it is.
pub(crate) fn set_pinned(leaf: &mut Value, pinned: bool) {
    fn put_after(map: &mut serde_json::Map<String, Value>, after: &str, pinned: bool) {
        if !pinned {
            map.shift_remove("pinned");
            return;
        }
        if map.contains_key("pinned") {
            map.insert("pinned".into(), json!(true));
            return;
        }
        let at = map
            .keys()
            .position(|k| k == after)
            .map_or(map.len(), |i| i + 1);
        map.shift_insert(at, "pinned".into(), json!(true));
    }
    let Some(outer) = leaf.as_object_mut() else {
        return;
    };
    put_after(outer, "type", pinned);
    if let Some(state) = outer.get_mut("state").and_then(Value::as_object_mut) {
        put_after(state, "state", pinned);
    }
}

/// The Lucide icon Obsidian records on a file's leaf: the dashboard for a
/// canvas, the text file for a PDF (observed:
/// `tools/reference-probe/results/2026-10-26-pdf.json`), the plain file
/// otherwise.
///
/// An audio file's is `lucide-file-audio` and a video's the plain file
/// (observed: `tools/reference-probe/results/2026-11-15-embeds-media.json`,
/// `s09`).
///
/// OBSERVE: what it writes for images and the rest.
pub(crate) fn leaf_icon(path: &str) -> &'static str {
    match FileKind::from_path(path) {
        FileKind::Canvas => "lucide-layout-dashboard",
        FileKind::Pdf => "lucide-file-text",
        FileKind::Audio => "lucide-file-audio",
        _ => "lucide-file",
    }
}

/// The leaf view type Obsidian gives a file of this kind.
fn leaf_view_type(path: &str) -> &'static str {
    match FileKind::from_path(path) {
        FileKind::Markdown => "markdown",
        FileKind::Image => "image",
        FileKind::Pdf => "pdf",
        FileKind::Canvas => "canvas",
        FileKind::Audio => "audio",
        FileKind::Video => "video",
        _ => "markdown",
    }
}
