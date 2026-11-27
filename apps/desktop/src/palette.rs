//! The command palette (`Mod+Shift+P`): fuzzy search over the commands the
//! workspace can run right now, this session's recently run commands first
//! while the query is empty, each row carrying its hotkeys.
//!
//! The list is exactly what the slash popup would offer (`slash_commands`),
//! ranked by the same [`rank_commands`] the popup uses, so a command hidden
//! by its check callback is hidden here too.

use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, ScrollStrategy, SharedString, StyledText, Subscription,
    UniformListScrollHandle, Window,
};
use serde_json::{json, Value};
use std::ops::Range;
use tft_suggest::slash::{rank_commands, Ranked};
use tft_suggest::CommandEntry;

pub enum PaletteEvent {
    /// Run the command with this id.
    Run(String),
    Close,
}

pub struct CommandPalette {
    input: Entity<TextInput>,
    commands: Vec<CommandEntry>,
    recent: Vec<String>,
    items: Vec<Ranked>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> CommandPalette {
        let input = cx.new(|cx| TextInput::new("Type a command…", cx));
        let subscriptions =
            vec![
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.refresh(cx),
                    InputEvent::Submit => this.run_selected(cx),
                    InputEvent::Cancel => cx.emit(PaletteEvent::Close),
                    InputEvent::Up => this.select(this.selected.saturating_sub(1), cx),
                    InputEvent::Down => this.select(this.selected + 1, cx),
                }),
            ];
        CommandPalette {
            input,
            commands: Vec::new(),
            recent: Vec::new(),
            items: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows the palette over `commands`, with `recent` first while the query
    /// is empty, and focuses its input.
    pub fn open(
        &mut self,
        commands: Vec<CommandEntry>,
        recent: Vec<String>,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commands = commands;
        self.recent = recent;
        self.input.update(cx, |i, cx| i.set_text(query, cx));
        window.focus(&self.input.focus_handle(cx), cx);
        self.refresh(cx);
    }

    /// What the palette is showing, for the control socket's state dump.
    pub fn describe(&self, rows: usize, cx: &App) -> Value {
        let total = self.items.len();
        let shown: Vec<&str> = self
            .items
            .iter()
            .take(rows)
            .filter_map(|item| self.commands.get(item.index))
            .map(|c| c.name.as_str())
            .collect();
        json!({
            "query": self.input.read(cx).text(),
            "selected": self.selected,
            "rows": shown,
            "rows_total": if total > shown.len() { Some(total) } else { None },
            "error": Value::Null,
        })
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().trim().to_string();
        self.items = rank_commands(&self.commands, &[], &self.recent, &query);
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        self.selected = ix.min(self.items.len() - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn run_selected(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(self.selected) else {
            return;
        };
        let Some(command) = self.commands.get(item.index) else {
            return;
        };
        cx.emit(PaletteEvent::Run(command.id.clone()));
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let theme = cx.global::<Theme>().clone();
        range
            .filter_map(|ix| {
                let item = self.items.get(ix)?;
                let command = self.commands.get(item.index)?;
                let selected = ix == self.selected;
                let highlights: Vec<_> = item
                    .ranges
                    .iter()
                    .map(|r| {
                        (
                            r.clone(),
                            HighlightStyle {
                                font_weight: Some(FontWeight::BOLD),
                                ..HighlightStyle::default()
                            },
                        )
                    })
                    .collect();
                let name = StyledText::new(SharedString::from(command.name.clone()))
                    .with_highlights(highlights);
                let hotkeys: Option<SharedString> = (!command.hotkeys.is_empty())
                    .then(|| command.hotkeys.join(", ").into());
                Some(
                    div()
                        .id(("palette-row", ix))
                        .w_full()
                        .h(px(34.))
                        .px_3()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.tokens.background_modifier_hover))
                        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = ix;
                            this.run_selected(cx);
                        }))
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(theme.tokens.text_normal)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(name),
                        )
                        .children(hotkeys.map(|h| {
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(theme.tokens.text_muted)
                                .child(h)
                        })),
                )
            })
            .collect()
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let count = self.items.len();
        let rows = (count.min(12) as f32) * 34.;
        crate::ui::modal_panel("command-palette", 620., &theme)
            .p_2()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
            .child(self.input.clone())
            .when(count > 0, |d| {
                d.child(
                    uniform_list("palette-list", count, cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .h(px(rows))
                        .mt_2(),
                )
            })
            .when(count == 0 && !self.input.read(cx).text().is_empty(), |d| {
                d.child(
                    div()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(theme.tokens.text_muted)
                        .child("No matching commands."),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> CommandEntry {
        CommandEntry {
            id: id.into(),
            name: name.into(),
            hotkeys: Vec::new(),
        }
    }

    fn commands() -> Vec<CommandEntry> {
        vec![
            entry("editor:save-file", "Save current file"),
            entry("graph:open", "Open graph view"),
            entry("switcher:open", "Open quick switcher"),
            entry("editor:toggle-bold", "Toggle bold"),
        ]
    }

    fn listed(commands: &[CommandEntry], recent: &[String], query: &str) -> Vec<String> {
        rank_commands(commands, &[], recent, query)
            .into_iter()
            .map(|r| commands[r.index].name.clone())
            .collect()
    }

    /// An empty query is the recently run commands in this session's order,
    /// then every other command by name.
    #[test]
    fn an_empty_query_lists_recent_then_everything_by_name() {
        let recent = vec!["switcher:open".to_string(), "graph:open".to_string()];
        assert_eq!(
            listed(&commands(), &recent, ""),
            [
                "Open quick switcher",
                "Open graph view",
                "Save current file",
                "Toggle bold",
            ]
        );
    }

    /// A query matches command names, not ids, and highlights come back for
    /// the matched characters.
    #[test]
    fn a_query_fuzzy_matches_the_palette_name() {
        let ranked = rank_commands(&commands(), &[], &[], "ogv");
        assert_eq!(commands()[ranked[0].index].id, "graph:open");
        assert!(!ranked[0].ranges.is_empty());
        assert!(listed(&commands(), &[], "save").contains(&"Save current file".to_string()));
        assert!(listed(&commands(), &[], "zzz").is_empty());
    }

    /// A command the caller left out (its check callback failed) is not
    /// listed even when the query matches it: the palette takes the list as
    /// given, exactly as the slash popup does.
    #[test]
    fn the_palette_lists_only_the_commands_it_is_handed() {
        let available = vec![entry("graph:open", "Open graph view")];
        assert_eq!(listed(&available, &[], "save"), Vec::<String>::new());
        assert_eq!(listed(&available, &[], "graph"), ["Open graph view"]);
    }
}
