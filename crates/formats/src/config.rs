//! `.tft/*.json` configuration files.
//!
//! 기준 앱 writes these sparsely (only keys that differ from defaults), so every
//! typed field is optional and unknown keys are kept in `extra`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Default name of the configuration directory; overridable per vault.
pub const DEFAULT_CONFIG_DIR: &str = ".tft";

/// `app.json`: Editor plus Files & links settings (key names exactly as 기준 앱 writes them).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    // Removed and unmodeled settings stay in `extra` to preserve them.
    /// `system` | `local` | `none`
    #[serde(
        default,
        rename = "trashOption",
        skip_serializing_if = "Option::is_none"
    )]
    pub trash_option: Option<String>,
    #[serde(
        default,
        rename = "alwaysUpdateLinks",
        skip_serializing_if = "Option::is_none"
    )]
    pub always_update_links: Option<bool>,
    /// `root` | `current` | `folder`
    #[serde(
        default,
        rename = "newFileLocation",
        skip_serializing_if = "Option::is_none"
    )]
    pub new_file_location: Option<String>,
    #[serde(
        default,
        rename = "newFileFolderPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub new_file_folder_path: Option<String>,
    #[serde(
        default,
        rename = "useMarkdownLinks",
        skip_serializing_if = "Option::is_none"
    )]
    pub use_markdown_links: Option<bool>,
    /// `shortest` | `relative` | `absolute`
    #[serde(
        default,
        rename = "newLinkFormat",
        skip_serializing_if = "Option::is_none"
    )]
    pub new_link_format: Option<String>,
    /// Vault root, a folder path, or `./subfolder` for a subfolder under the current folder.
    #[serde(
        default,
        rename = "attachmentFolderPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub attachment_folder_path: Option<String>,
    /// Excluded files. `null` (the last rule deleted) is kept apart from a
    /// missing key and `[]`.
    #[serde(
        default,
        rename = "userIgnoreFilters",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub user_ignore_filters: Option<Option<Vec<String>>>,
    /// Export to PDF's last values.
    #[serde(
        default,
        rename = "pdfExportSettings",
        skip_serializing_if = "Option::is_none"
    )]
    pub pdf_export_settings: Option<PdfExportSettings>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AppConfig {
    /// The Excluded files rules, none when the key is missing or `null`.
    pub fn ignore_filters(&self) -> &[String] {
        self.user_ignore_filters
            .as_ref()
            .and_then(Option::as_deref)
            .unwrap_or(&[])
    }

    /// Replaces the Excluded files rules the way 기준 앱's Settings does:
    /// deleting the last one leaves the key as `null`, not `[]`.
    pub fn set_ignore_filters(&mut self, filters: Vec<String>) {
        self.user_ignore_filters = Some((!filters.is_empty()).then_some(filters));
    }
}

/// A key that is present, `null` included: `Some(None)` for `null`, so a
/// double option tells it from a missing key (which `default` makes `None`).
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// `app.json`'s `pdfExportSettings`, in the key order 기준 앱 writes:
/// `includeName` (only once the toggle has been touched), `pageSize`,
/// `landscape`, `margin` (a **string**: `"0"` Default, `"1"` None, `"2"`
/// Minimal) and `downscalePercent`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PdfExportSettings {
    #[serde(
        default,
        rename = "includeName",
        skip_serializing_if = "Option::is_none"
    )]
    pub include_name: Option<bool>,
    #[serde(default, rename = "pageSize", skip_serializing_if = "Option::is_none")]
    pub page_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landscape: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<String>,
    #[serde(
        default,
        rename = "downscalePercent",
        skip_serializing_if = "Option::is_none"
    )]
    pub downscale_percent: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `appearance.json`: the base color scheme only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// `dark` | `light` | `system`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One binding in `hotkeys.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    /// Any of `Mod`, `Ctrl`, `Meta`, `Shift`, `Alt`.
    pub modifiers: Vec<String>,
    /// A `KeyboardEvent.key` name such as `P`, `Enter`, `ArrowLeft`.
    pub key: String,
}

/// `hotkeys.json`: command id to its user overrides; an empty list unbinds
/// the command. Insertion order is kept, like [`PropertyTypes`]: 기준 앱
/// writes the file in the order the bindings were made, so a sorted map
/// would reorder the whole file the first time TFT saved it.
pub type Hotkeys = IndexMap<String, Vec<Hotkey>>;

/// `core-plugins.json`. Current builds write a map of id to enabled; older
/// builds wrote an array of enabled ids. Both are accepted.
///
/// The map keeps the file's order: 기준 앱 writes the plugins in its own
/// registration order, not alphabetically, so a sorted map would rewrite
/// every line the first time TFT saved the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CorePlugins {
    Map(IndexMap<String, bool>),
    List(Vec<String>),
}

impl CorePlugins {
    pub fn is_enabled(&self, id: &str) -> bool {
        match self {
            CorePlugins::Map(m) => m.get(id).copied().unwrap_or(false),
            CorePlugins::List(l) => l.iter().any(|x| x == id),
        }
    }
}

/// A color in `graph.json`: alpha plus a packed `0xRRGGBB`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphColor {
    #[serde(default = "one")]
    pub a: f32,
    #[serde(default)]
    pub rgb: u32,
}

fn one() -> f32 {
    1.0
}

/// One entry of `colorGroups`: nodes matching a search query get a color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphColorGroup {
    #[serde(default)]
    pub query: String,
    pub color: GraphColor,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `graph.json`: the global graph's filters, groups, display and force
/// settings. `collapse-*` keys and anything newer stay in `extra`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, rename = "showTags", skip_serializing_if = "Option::is_none")]
    pub show_tags: Option<bool>,
    #[serde(
        default,
        rename = "showAttachments",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_attachments: Option<bool>,
    #[serde(
        default,
        rename = "hideUnresolved",
        skip_serializing_if = "Option::is_none"
    )]
    pub hide_unresolved: Option<bool>,
    #[serde(
        default,
        rename = "showOrphans",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_orphans: Option<bool>,
    #[serde(default, rename = "colorGroups", skip_serializing_if = "Vec::is_empty")]
    pub color_groups: Vec<GraphColorGroup>,
    #[serde(default, rename = "showArrow", skip_serializing_if = "Option::is_none")]
    pub show_arrow: Option<bool>,
    #[serde(
        default,
        rename = "textFadeMultiplier",
        skip_serializing_if = "Option::is_none"
    )]
    pub text_fade_multiplier: Option<f32>,
    #[serde(
        default,
        rename = "nodeSizeMultiplier",
        skip_serializing_if = "Option::is_none"
    )]
    pub node_size_multiplier: Option<f32>,
    #[serde(
        default,
        rename = "lineSizeMultiplier",
        skip_serializing_if = "Option::is_none"
    )]
    pub line_size_multiplier: Option<f32>,
    #[serde(
        default,
        rename = "centerStrength",
        skip_serializing_if = "Option::is_none"
    )]
    pub center_strength: Option<f32>,
    #[serde(
        default,
        rename = "repelStrength",
        skip_serializing_if = "Option::is_none"
    )]
    pub repel_strength: Option<f32>,
    #[serde(
        default,
        rename = "linkStrength",
        skip_serializing_if = "Option::is_none"
    )]
    pub link_strength: Option<f32>,
    #[serde(
        default,
        rename = "linkDistance",
        skip_serializing_if = "Option::is_none"
    )]
    pub link_distance: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl GraphConfig {
    /// Complete `graph.json` document used when a Vault does not have one yet.
    pub fn full_document(&self) -> Value {
        let collapsed = |key: &str| self.extra.get(key).and_then(Value::as_bool).unwrap_or(true);
        let number = |value: Option<f32>, default: Value| match value {
            Some(value) => graph_json_number(value),
            None => default,
        };
        let mut groups = serde_json::json!(self.color_groups);
        integerize_graph_numbers(&mut groups);
        serde_json::json!({
            "collapse-filter": collapsed("collapse-filter"),
            "search": self.search.clone().unwrap_or_default(),
            "showTags": self.show_tags.unwrap_or(false),
            "showAttachments": self.show_attachments.unwrap_or(false),
            "hideUnresolved": self.hide_unresolved.unwrap_or(false),
            "showOrphans": self.show_orphans.unwrap_or(true),
            "collapse-color-groups": collapsed("collapse-color-groups"),
            "colorGroups": groups,
            "collapse-display": collapsed("collapse-display"),
            "showArrow": self.show_arrow.unwrap_or(false),
            "textFadeMultiplier": number(self.text_fade_multiplier, serde_json::json!(0)),
            "nodeSizeMultiplier": number(self.node_size_multiplier, serde_json::json!(1)),
            "lineSizeMultiplier": number(self.line_size_multiplier, serde_json::json!(1)),
            "collapse-forces": collapsed("collapse-forces"),
            "centerStrength": number(
                self.center_strength,
                serde_json::json!(0.518713248970312),
            ),
            "repelStrength": number(self.repel_strength, serde_json::json!(10)),
            "linkStrength": number(self.link_strength, serde_json::json!(1)),
            "linkDistance": number(self.link_distance, serde_json::json!(250)),
            "scale": number(self.scale, serde_json::json!(1)),
            "close": self.close.unwrap_or(false),
        })
    }

    /// Sparse update to merge into an existing `graph.json`. Empty groups and
    /// search are explicit so removing the last entry clears the old value.
    pub fn merge_document(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(object) = value.as_object_mut() {
            if self.color_groups.is_empty() {
                object.insert("colorGroups".into(), serde_json::json!([]));
            }
            if self.search.is_none() {
                object.insert("search".into(), serde_json::json!(""));
            }
        }
        integerize_graph_numbers(&mut value);
        value
    }
}

fn graph_json_number(value: f32) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        serde_json::json!(value as i64)
    } else {
        serde_json::json!(value)
    }
}

fn integerize_graph_numbers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(number_value) = number.as_f64() {
                *value = graph_json_number(number_value as f32);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(integerize_graph_numbers),
        Value::Object(fields) => fields.values_mut().for_each(integerize_graph_numbers),
        _ => {}
    }
}

/// `switcher.json`: the quick switcher's options.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SwitcherConfig {
    #[serde(
        default,
        rename = "showExistingOnly",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_existing_only: Option<bool>,
    #[serde(
        default,
        rename = "showAttachments",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_attachments: Option<bool>,
    #[serde(
        default,
        rename = "showAllFileTypes",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_all_file_types: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// The other core-plugin option files 기준 앱 1.14.5 writes from its own
// options tabs, observed on 2026-10-30 (`tools/reference-probe/results/
// 2026-10-30-editor-menus.json`, step `s04-plugin-tabs`: every toggle
// clicked twice, every dropdown walked through its options and back, the
// `.tft` folder diffed after each click).
//
// Each is sparse and appears only once a control writes, each field is
// named exactly as 기준 앱 names it, and the fields are declared in the
// order 기준 앱's own first write put them in. A row whose key the probe
// could not reach -- a file suggester, a slider, a textarea, a button --
// has no field here, and TFT writes nothing for it rather than guessing
// a name; `tft::settings::plugin_items` draws those rows tagged.

/// `canvas.json`: the Canvas plugin's options.
///
/// 기준 앱's first write held both snap keys at once
/// (`{"snapToObjects": true, "snapToGrid": false}`), with each dropdown's
/// key appended as it was picked. Two rows of the tab are not here: the
/// folder name the `folder` option reveals, and the zoom threshold slider,
/// neither of whose key the probe observed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CanvasConfig {
    /// "Snap to objects", on when absent.
    #[serde(
        default,
        rename = "snapToObjects",
        skip_serializing_if = "Option::is_none"
    )]
    pub snap_to_objects: Option<bool>,
    /// "Snap to grid", on when absent.
    #[serde(
        default,
        rename = "snapToGrid",
        skip_serializing_if = "Option::is_none"
    )]
    pub snap_to_grid: Option<bool>,
    /// "Default location for new canvas files": `root`, `current`, `folder`.
    #[serde(
        default,
        rename = "newFileLocation",
        skip_serializing_if = "Option::is_none"
    )]
    pub new_file_location: Option<String>,
    /// "Default mouse wheel behavior": `pan` or `zoom`.
    #[serde(
        default,
        rename = "defaultWheelBehavior",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_wheel_behavior: Option<String>,
    /// "Default Ctrl + Drag behavior": `menu`, `card`, `note`, `media`,
    /// `webpage` or `group`.
    #[serde(
        default,
        rename = "defaultModDragBehavior",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_mod_drag_behavior: Option<String>,
    /// "Show card names": `always`, `hover` or `never`.
    #[serde(
        default,
        rename = "cardLabelVisibility",
        skip_serializing_if = "Option::is_none"
    )]
    pub card_label_visibility: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `page-preview.json`: one key per surface the hover preview can be made
/// to require Ctrl on, each named after the surface rather than after the
/// row. Every surface is on when absent but Reading view, which is off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PagePreviewConfig {
    /// "Reading view".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
    /// "Editing view".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<bool>,
    /// "Tab header".
    #[serde(
        default,
        rename = "tab-header",
        skip_serializing_if = "Option::is_none"
    )]
    pub tab_header: Option<bool>,
    /// "Files".
    #[serde(
        default,
        rename = "file-explorer",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_explorer: Option<bool>,
    /// "Graph view".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A core plugin as the Core plugins tab lists it (names and descriptions
/// observed in 기준 앱 1.14.4) and whether a fresh vault has it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorePlugin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub default_on: bool,
}

/// The core plugins in the order the Core plugins tab shows them.
pub const CORE_PLUGINS: &[CorePlugin] = &[
    CorePlugin {
        id: "canvas",
        name: "Canvas",
        description: "Arrange and connect notes on an infinite canvas.",
        default_on: true,
    },
    CorePlugin {
        id: "file-explorer",
        name: "Files",
        description: "Browse the files and folders in your vault.",
        default_on: true,
    },
    CorePlugin {
        id: "graph",
        name: "Graph view",
        description: "Visualize the relationships between your notes.",
        default_on: true,
    },
    CorePlugin {
        id: "page-preview",
        name: "Page preview",
        description: "Hover an internal link to preview its content. In editor mode, press Ctrl/Cmd while hovering.",
        default_on: true,
    },
    CorePlugin {
        id: "switcher",
        name: "Quick switcher",
        description: "Jump to other files with your keyboard.",
        default_on: true,
    },
    CorePlugin {
        id: "global-search",
        name: "Search",
        description: "Search for a keyword in all the notes.",
        default_on: true,
    },
    CorePlugin {
        id: "slash-command",
        name: "Slash commands",
        description: "Trigger commands in the editor by using the forward slash key.",
        default_on: false,
    },
];

impl CorePlugins {
    /// Enabled state from the file, or the fresh-vault default when the
    /// file does not mention the plugin (list form: only listed ids are on).
    pub fn is_enabled_or_default(this: Option<&CorePlugins>, plugin: &CorePlugin) -> bool {
        match this {
            Some(CorePlugins::Map(m)) => m.get(plugin.id).copied().unwrap_or(plugin.default_on),
            Some(CorePlugins::List(l)) => l.iter().any(|x| x == plugin.id),
            None => plugin.default_on,
        }
    }
}

/// `types.json`: property name to type.
///
/// The map keeps insertion order because 기준 앱 writes the file that way,
/// seeded with `aliases`, `cssclasses` and `tags` before any key the user
/// assigns; a sorted map would rewrite the whole file on the first save.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PropertyTypes {
    #[serde(default)]
    pub types: IndexMap<String, String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl PropertyTypes {
    /// The entries 기준 앱 adds to `types.json` whenever it writes the file
    /// and finds them missing: the built-in keys, each pinned to its reserved
    /// type. Observed on the gate vault, 기준 앱 1.15.0, 2026-11-09: a file
    /// TFT had written with one key came back as that key, then these
    /// three, then the keys the session assigned -- so they are appended
    /// after whatever the file holds, in this order, and before the new key.
    pub const DEFAULTS: [(&'static str, &'static str); 3] = [
        ("aliases", "aliases"),
        ("cssclasses", "multitext"),
        ("tags", "tags"),
    ];

    /// Appends the missing defaults. Every writer of the file calls this
    /// first, so a new `types.json` opens with the three and an old one
    /// gains them where 기준 앱 would put them.
    pub fn seed_defaults(&mut self) {
        for (key, ty) in Self::DEFAULTS {
            if !self.types.contains_key(key) {
                self.types.insert(key.to_string(), ty.to_string());
            }
        }
    }
}

/// The type names 기준 앱 1.14.4 registers (`registeredTypeWidgets`), which
/// are the values `types.json` carries. `text`, `multitext`, `number`,
/// `checkbox`, `date`, `datetime`, `tags` and `aliases` are the property
/// types; `file`, `folder` and `property` are 기준 앱's other widget types,
/// and the properties type menu does not offer them. There is no
/// `cssclasses` type:
/// that key infers as `multitext`.
pub const PROPERTY_TYPES: &[&str] = &[
    "text",
    "multitext",
    "number",
    "checkbox",
    "date",
    "datetime",
    "tags",
    "aliases",
    "file",
    "folder",
    "property",
];

/// Merges a typed config value into the file's JSON, keeping unknown keys
/// and key order.
pub fn merge_config(old: Value, fresh: Value) -> Value {
    match (old, fresh) {
        (Value::Object(mut merged), Value::Object(fields)) => {
            for (key, value) in fields {
                match merged.get(&key) {
                    Some(prev) if same_config_value(prev, &value) => {}
                    Some(prev) if prev.is_object() && value.is_object() => {
                        if let Some(slot) = merged.get_mut(&key) {
                            let prev = std::mem::replace(slot, Value::Null);
                            *slot = merge_config(prev, value);
                        }
                    }
                    _ => {
                        merged.insert(key, value);
                    }
                }
            }
            Value::Object(merged)
        }
        (_, fresh) => fresh,
    }
}

fn same_config_value(old: &Value, new: &Value) -> bool {
    match (old, new) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a == b || a as f32 == b as f32,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_config_value(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|other| same_config_value(v, other)))
        }
        _ => old == new,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ignore_filters_keep_missing_null_and_empty_apart() {
        for text in [
            r#"{"x":1}"#,
            r#"{"userIgnoreFilters":null,"x":1}"#,
            r#"{"userIgnoreFilters":[],"x":1}"#,
            r#"{"userIgnoreFilters":["Archive/","/\\.png$/"],"x":1}"#,
        ] {
            let app: AppConfig = serde_json::from_str(text).unwrap();
            assert_eq!(serde_json::to_string(&app).unwrap(), text);
        }
        let mut app: AppConfig =
            serde_json::from_str(r#"{"userIgnoreFilters":["Archive/"]}"#).unwrap();
        assert_eq!(app.ignore_filters(), ["Archive/"]);
        app.set_ignore_filters(vec![]);
        assert!(app.ignore_filters().is_empty());
        assert_eq!(
            serde_json::to_string(&app).unwrap(),
            r#"{"userIgnoreFilters":null}"#
        );
        app.set_ignore_filters(vec!["Drafts/".into()]);
        assert_eq!(
            serde_json::to_string(&app).unwrap(),
            r#"{"userIgnoreFilters":["Drafts/"]}"#
        );
    }

    #[test]
    fn pdf_export_settings_keep_references_order_a_string_margin_and_unknown_keys() {
        let text = r#"{"spellcheck":true,"pdfExportSettings":{"includeName":true,"pageSize":"A4","landscape":false,"margin":"2","downscalePercent":80,"future":1}}"#;
        let app: AppConfig = serde_json::from_str(text).unwrap();
        let pdf = app.pdf_export_settings.clone().unwrap();
        assert_eq!(pdf.page_size.as_deref(), Some("A4"));
        assert_eq!(pdf.margin.as_deref(), Some("2"));
        assert_eq!(pdf.downscale_percent, Some(80));
        assert_eq!(serde_json::to_string(&app).unwrap(), text);
        // The first export from `{}` writes four keys, `includeName` not
        // among them.
        let first = PdfExportSettings {
            page_size: Some("Letter".into()),
            landscape: Some(false),
            margin: Some("0".into()),
            downscale_percent: Some(100),
            ..PdfExportSettings::default()
        };
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            r#"{"pageSize":"Letter","landscape":false,"margin":"0","downscalePercent":100}"#
        );
    }

    #[test]
    fn graph_json_round_trips_groups_and_collapse_keys() {
        let text = r#"{"collapse-filter":true,"showOrphans":true,"colorGroups":[{"query":"tag:#x","color":{"a":1,"rgb":16711680}}],"repelStrength":10,"scale":1,"close":true}"#;
        let cfg: GraphConfig = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.show_orphans, Some(true));
        assert_eq!(cfg.color_groups[0].query, "tag:#x");
        assert_eq!(cfg.color_groups[0].color.rgb, 0xff0000);
        assert_eq!(cfg.extra["collapse-filter"], serde_json::json!(true));
        let back: serde_json::Value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(back["collapse-filter"], serde_json::json!(true));
        assert_eq!(
            back["colorGroups"][0]["color"]["rgb"],
            serde_json::json!(16711680)
        );
    }

    #[test]
    fn app_json_keeps_unknown_keys() {
        let text = r#"{"readableLineLength":false,"vimMode":true,"someFutureKey":[1,2]}"#;
        let cfg: AppConfig = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.extra["readableLineLength"], serde_json::json!(false));
        assert_eq!(cfg.extra["vimMode"], serde_json::json!(true));
        assert_eq!(cfg.extra["someFutureKey"], serde_json::json!([1, 2]));
        let back: Value = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(text).unwrap());
    }

    #[test]
    fn core_plugins_accepts_both_shapes() {
        let map: CorePlugins =
            serde_json::from_str(r#"{"file-explorer":true,"graph":false}"#).unwrap();
        assert!(map.is_enabled("file-explorer"));
        assert!(!map.is_enabled("graph"));
        let list: CorePlugins = serde_json::from_str(r#"["file-explorer","graph"]"#).unwrap();
        assert!(list.is_enabled("graph"));
        assert!(!list.is_enabled("canvas"));
    }

    /// The removed core plugins are gone from the list the Core plugins tab
    /// and the command gate read.
    #[test]
    fn removed_core_plugins_are_not_listed() {
        for id in [
            "sync",
            "bases",
            "note-composer",
            "command-palette",
            "file-recovery",
            "templates",
            "daily-notes",
            "audio-recorder",
            "footnotes",
            "publish",
            "random-note",
            "slides",
            "webviewer",
            "word-count",
            "workspaces",
            "zk-prefixer",
            "bookmarks",
            "backlink",
        ] {
            assert!(
                !CORE_PLUGINS.iter().any(|p| p.id == id),
                "{id} survived its feature's removal"
            );
        }
    }

    #[test]
    fn switcher_json() {
        let s: SwitcherConfig = serde_json::from_str(r#"{"showAttachments":false}"#).unwrap();
        assert_eq!(s.show_attachments, Some(false));
        assert_eq!(s.show_existing_only, None);
        let plugin = CORE_PLUGINS.iter().find(|p| p.id == "graph").unwrap();
        assert!(CorePlugins::is_enabled_or_default(None, plugin));
        let off: CorePlugins = serde_json::from_str(r#"{"graph":false}"#).unwrap();
        assert!(!CorePlugins::is_enabled_or_default(Some(&off), plugin));
    }

    /// Every core-plugin options file: the keys 기준 앱 1.14.5 writes read
    /// into their own fields, an unknown key comes back where it was, and
    /// the bytes round trip exactly. The files the probe saw are the input,
    /// each with one key 기준 앱 has never written appended.
    #[test]
    fn core_plugin_option_files_round_trip_with_an_unknown_key() {
        macro_rules! round_trip {
            ($ty:ty, $text:literal) => {{
                let cfg: $ty = serde_json::from_str($text).unwrap();
                assert_eq!(
                    cfg.extra["someFutureKey"],
                    serde_json::json!({ "nested": [1, 2] }),
                    "{} dropped the unknown key",
                    stringify!($ty)
                );
                let back: Value =
                    serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
                assert_eq!(
                    back,
                    serde_json::from_str::<Value>($text).unwrap(),
                    "{} did not round trip",
                    stringify!($ty)
                );
                cfg
            }};
        }

        let canvas = round_trip!(
            CanvasConfig,
            r#"{"snapToObjects":true,"snapToGrid":false,"newFileLocation":"folder","defaultWheelBehavior":"zoom","defaultModDragBehavior":"group","cardLabelVisibility":"never","someFutureKey":{"nested":[1,2]}}"#
        );
        assert_eq!(canvas.snap_to_objects, Some(true));
        assert_eq!(canvas.snap_to_grid, Some(false));
        assert_eq!(canvas.new_file_location.as_deref(), Some("folder"));
        assert_eq!(canvas.default_wheel_behavior.as_deref(), Some("zoom"));
        assert_eq!(canvas.default_mod_drag_behavior.as_deref(), Some("group"));
        assert_eq!(canvas.card_label_visibility.as_deref(), Some("never"));

        let preview = round_trip!(
            PagePreviewConfig,
            r#"{"search":true,"preview":false,"editor":true,"tab-header":false,"file-explorer":true,"properties":false,"bookmarks":true,"outline":false,"graph":false,"someFutureKey":{"nested":[1,2]}}"#
        );
        assert_eq!(preview.tab_header, Some(false));
        assert_eq!(preview.file_explorer, Some(true));
        assert_eq!(preview.graph, Some(false));

        // A file none of them has ever been written to is all defaults and
        // serializes to nothing at all, the way the sparse files do.
        assert_eq!(
            serde_json::to_string(&CanvasConfig::default()).unwrap(),
            "{}"
        );
        assert_eq!(
            serde_json::to_string(&PagePreviewConfig::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn hotkeys_shape() {
        let text = r#"{"editor:toggle-bold":[{"modifiers":["Mod"],"key":"B"}],"app:reload":[]}"#;
        let hk: Hotkeys = serde_json::from_str(text).unwrap();
        assert_eq!(hk["editor:toggle-bold"][0].key, "B");
        assert!(hk["app:reload"].is_empty());
    }

    #[test]
    fn types_json_is_seeded_the_way_reference_seeds_it() {
        // The bytes 기준 앱 1.15.0 left on the gate vault (2026-11-09) after
        // a session that found `status` alone in the file and assigned two
        // more keys: the one it found, the three defaults, the new ones.
        let observed = "{\n  \"types\": {\n    \"status\": \"date\",\n    \"aliases\": \"aliases\",\n    \"cssclasses\": \"multitext\",\n    \"tags\": \"tags\",\n    \"draft\": \"checkbox\",\n    \"tft\": \"multitext\"\n  }\n}";
        let mut types = PropertyTypes::default();
        types.types.insert("status".into(), "date".into());
        types.seed_defaults();
        types.types.insert("draft".into(), "checkbox".into());
        types.seed_defaults();
        types.types.insert("tft".into(), "multitext".into());
        assert_eq!(serde_json::to_string_pretty(&types).unwrap(), observed);
        // A fresh file opens with the three.
        let mut fresh = PropertyTypes::default();
        fresh.seed_defaults();
        fresh.types.insert("status".into(), "multitext".into());
        assert_eq!(
            fresh.types.keys().collect::<Vec<_>>(),
            ["aliases", "cssclasses", "tags", "status"]
        );
        // Seeding twice adds nothing.
        let again = fresh.clone();
        fresh.seed_defaults();
        assert_eq!(fresh, again);
    }
}
