//! Optional folder-oriented radial explorer, isolated from GraphView.

use gpui::{AppContext, Context, Entity, FocusHandle, Subscription, Window};
use radial_tree_explorer::{RadialTree, RadialTreeEvent, TreeViewOptions};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use tft_vault::Vault;

static FONTS_REGISTERED: OnceLock<()> = OnceLock::new();

#[derive(Default)]
pub(super) struct RadialTreeState {
    pub(super) view: Option<Entity<RadialTree>>,
    pub(super) focus: Option<FocusHandle>,
    pub(super) active: bool,
    paths: Vec<Option<String>>,
    _subscription: Option<Subscription>,
}

#[derive(Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    notes: Vec<(String, String)>,
}

impl Folder {
    fn folder_mut(&mut self, path: &str) -> &mut Folder {
        let mut current = self;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            current = current.folders.entry(part.to_string()).or_default();
        }
        current
    }

    fn encode(&self, name: &str, paths: &mut Vec<Option<String>>) -> Value {
        paths.push(None);
        let mut children = vec![Value::String(name.to_string())];
        for (name, folder) in &self.folders {
            children.push(folder.encode(name, paths));
        }
        let mut notes = self.notes.clone();
        notes.sort_by(|a, b| a.0.cmp(&b.0));
        for (label, path) in notes {
            paths.push(Some(path));
            children.push(Value::String(label));
        }
        Value::Array(children)
    }
}

pub(super) fn folder_tree(vault: &Vault) -> (Value, Vec<Option<String>>) {
    let mut root = Folder::default();
    for folder in vault.folders() {
        root.folder_mut(folder);
    }
    for file in vault.markdown_files() {
        let (parent, leaf) = file
            .path
            .rsplit_once('/')
            .unwrap_or(("", file.path.as_str()));
        let label = std::path::Path::new(leaf)
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| leaf.to_string());
        root.folder_mut(parent)
            .notes
            .push((label, file.path.clone()));
    }
    let mut paths = Vec::new();
    let data = root.encode(&vault.name(), &mut paths);
    (data, paths)
}

impl super::Workspace {
    pub(super) fn show_radial_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.as_ref() else {
            return;
        };
        FONTS_REGISTERED.get_or_init(|| {
            radial_tree_explorer::register_fonts(&window.text_system());
        });
        let (data, paths) = folder_tree(&state.vault());
        let focus = cx.focus_handle();
        let view =
            cx.new(|_| RadialTree::new(data, window, focus.clone(), TreeViewOptions::default()));
        self.radial_tree.paths = paths;
        self.radial_tree.focus = Some(focus);
        self.radial_tree._subscription =
            Some(cx.subscribe(&view, |this, _, event: &RadialTreeEvent, cx| {
                if let Some(Some(path)) = this.radial_tree.paths.get(event.0).cloned() {
                    this.open_file(path, None, cx);
                }
            }));
        self.radial_tree.view = Some(view);
        self.show_graph(false, window, cx);
        self.radial_tree.active = true;
        cx.notify();
    }

    pub(super) fn show_force_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.radial_tree.active = false;
        self.show_graph(false, window, cx);
    }
}
