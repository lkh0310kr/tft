//! Empty-tab content and the command-backed actions it offers.

use super::{Commands, Workspace};
use crate::theme::{FONT_UI_SMALL, Theme};
use gpui::{Context, div, prelude::*, px};
use tft_commands::{CommandRegistry, hotkey_label};

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
    /// The empty tab: the three pills Obsidian offers, each running its command, under
    /// TFT's mark. The body is the same however the tab emptied.
    pub(super) fn render_empty_tab(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let registry = cx.global::<Commands>().0.clone();
        let pills: Vec<_> =
            empty_tab_actions(&registry)
                .into_iter()
                .map(|action| {
                    let command = action.command;
                    // Obsidian's `mod-pill`: fully rounded, the label in the UI size with
                    // the hotkey muted behind it.
                    div()
                        .id(action.id)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(14.))
                        .py(px(6.))
                        .rounded(px(14.))
                        .bg(theme.tokens.background_modifier_hover)
                        .border_1()
                        .border_color(theme.tokens.background_modifier_border)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                        .text_size(px(FONT_UI_SMALL))
                        .child(
                            div()
                                .text_color(theme.tokens.text_normal)
                                .child(action.label),
                        )
                        .children(
                            action
                                .hotkey
                                .map(|keys| div().text_color(theme.tokens.text_faint).child(keys)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_command(command, window, cx)
                        }))
                })
                .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.tokens.text_muted)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .children(pills),
            )
            .into_any_element()
    }
}
