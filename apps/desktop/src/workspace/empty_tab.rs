//! Empty-tab content and the command-backed actions it offers.

use super::{Commands, Workspace};
use crate::theme::Theme;
use gpui::{div, prelude::*, px, Context, SharedString};
use tft_commands::{hotkey_label, CommandRegistry};

/// What an empty tab offers, in Obsidian's order: element id, pill label and command.
/// What must match is the text and that each pill runs the command it names.
pub(super) const EMPTY_TAB_ACTIONS: [(&str, &str, &str); 3] = [
    ("empty-new-note", "New note", "file-explorer:new-file"),
    ("empty-new-canvas", "New canvas", "canvas:new-file"),
    ("empty-go-to-file", "Go to file", "switcher:open"),
];

/// One pill of the empty tab.
pub(super) struct EmptyAction {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    /// The command's first hotkey as `hotkeys.json` writes it, from the registry,
    /// or `None` for a command with none.
    pub(super) hotkey: Option<String>,
    pub(super) command: &'static str,
}

impl EmptyAction {
    /// The pill's text: the label, then the hotkey when there is one.
    pub(super) fn text(&self) -> String {
        match &self.hotkey {
            Some(keys) => format!("{} {keys}", self.label),
            None => self.label.to_string(),
        }
    }
}

/// The pills an empty tab draws; a command the registry does not have is left out
/// rather than drawn dead.
pub(super) fn empty_tab_actions(registry: &CommandRegistry) -> Vec<EmptyAction> {
    EMPTY_TAB_ACTIONS
        .iter()
        .filter_map(|(id, label, command)| {
            let known = registry.get(command)?;
            Some(EmptyAction {
                id,
                label,
                hotkey: known.hotkeys.first().map(hotkey_label),
                command,
            })
        })
        .collect()
}

impl Workspace {
    /// The empty tab's three actions, presented as a plain vertical list.
    pub(super) fn render_empty_tab(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let registry = cx.global::<Commands>().0.clone();
        let actions: Vec<_> = empty_tab_actions(&registry)
            .into_iter()
            .map(|action| {
                let command = action.command;
                crate::ui::workspace_empty_action(
                    action.id,
                    action.label,
                    action.hotkey.map(SharedString::from),
                    theme,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)),
                )
            })
            .collect();
        crate::ui::workspace_empty_tab(theme)
            .child(div().w(px(240.)).flex().flex_col().children(actions))
            .into_any_element()
    }
}
