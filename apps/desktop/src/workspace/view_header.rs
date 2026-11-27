//! Per-group document header controls and title presentation.

use super::release_notes;
use super::render::GroupView;
use super::tab_strip::TabTitle;
use super::{GoBack, GoForward, OpenGraph, ToggleMode, ViewMode, Workspace, leaf_title};
use crate::theme::Theme;
use gpui::{Context, SharedString, div, prelude::*, px};

/// Whether the view header carries the mode switch: no on an empty tab (nothing to
/// read or edit), and the graph has its own toggle.
pub(super) fn header_shows_mode_switch(current: &Option<String>, mode: ViewMode) -> bool {
    current.is_some() && !matches!(mode, ViewMode::Graph | ViewMode::Search)
}

impl Workspace {
    /// The view header: back/forward, the note's folder and name, the graph toggle and
    /// the mode toggle. Its buttons act on their group, made active first.
    pub(super) fn render_header(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let id = view.id;
        // The leaf's name, the same string its tab carries: "New tab" with nothing open.
        let title = view
            .tabs
            .get(view.active_tab)
            .map(|t| t.title())
            .unwrap_or_else(|| leaf_title(view.current.as_deref(), view.mode));
        let crumbs: SharedString = view
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/').map(|(dir, _)| dir.replace('/', " / ")))
            .unwrap_or_default()
            .into();
        // Obsidian's view action shows the current mode: a book while reading, a pencil while editing.
        let mode_icon = match view.mode {
            ViewMode::Reading => "book-open",
            _ => "pencil",
        };
        let can_back = view.history_ix > 1;
        let can_forward = view.history_ix < view.history_len;
        let release_notes_leaf = view
            .tabs
            .get(view.active_tab)
            .and_then(release_notes::version_of)
            .is_some();
        // `.view-header-title` is `--text-normal`; a non-active group keeps its title muted.
        let title_color = if view.active {
            theme.tokens.text_normal
        } else {
            theme.tokens.text_muted
        };
        // Obsidian's view header: 40 px, 12 px side padding, history arrows at 40 %
        // when unusable, the title in the small UI size.
        crate::ui::workspace_view_header(theme)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(
                        crate::ui::icon_button("back", "arrow-left", theme)
                            .when(!can_back, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_back(&GoBack, window, cx)
                            })),
                    )
                    .child(
                        crate::ui::icon_button("forward", "arrow-right", theme)
                            .when(!can_forward, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_forward(&GoForward, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    // `.view-header-title-parent`: `--text-muted`.
                    .when(!crumbs.is_empty(), |d| {
                        d.child(
                            div()
                                .text_color(theme.tokens.text_muted)
                                .child(crumbs.clone()),
                        )
                        .child(div().text_color(theme.tokens.text_muted).child("/"))
                    })
                    .child(self.render_title(view, title.clone(), title_color, theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .child(if release_notes_leaf {
                        // The release notes are not a note: no graph and no mode switch.
                        div()
                    } else {
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(2.))
                            .child(
                                crate::ui::icon_button("graph", "git-fork", theme)
                                    .when(
                                        view.mode == ViewMode::Graph && !self.radial_tree.active,
                                        |d| d.bg(theme.tokens.background_modifier_hover),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        if this.radial_tree.active {
                                            this.show_force_graph(window, cx)
                                        } else {
                                            this.open_graph_action(&OpenGraph, window, cx)
                                        }
                                    })),
                            )
                            .child(
                                crate::ui::icon_button("radial-tree", "folder-tree", theme)
                                    .when(
                                        view.mode == ViewMode::Graph && self.radial_tree.active,
                                        |d| d.bg(theme.tokens.background_modifier_hover),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        if this.radial_tree.active {
                                            this.show_force_graph(window, cx)
                                        } else {
                                            this.show_radial_tree(window, cx)
                                        }
                                    })),
                            )
                            .when(header_shows_mode_switch(&view.current, view.mode), |d| {
                                d.child(crate::ui::icon_button("mode", mode_icon, theme).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.activate_group(id, cx);
                                        this.toggle_mode(&ToggleMode, window, cx)
                                    }),
                                ))
                            })
                    }),
            )
    }
}
