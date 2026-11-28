//! Optional folder-oriented radial explorer, isolated from GraphView.

use gpui::{AppContext, Context, Entity, FocusHandle, Subscription, Window};
use radial_tree_explorer::{RadialTree, RadialTreeEvent, TreeViewOptions};
use serde_json::Value;
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

pub(super) fn folder_tree(vault: &Vault) -> (Value, Vec<Option<String>>) {
    let folders = vault.folders();
    let files = vault.markdown_files().cloned().collect::<Vec<_>>();
    tft_app::explorer::radial_tree_data(&vault.name(), &folders, &files)
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
