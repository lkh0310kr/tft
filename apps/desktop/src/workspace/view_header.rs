//! Per-group document header controls and title presentation.

use super::release_notes;
use super::render::GroupView;
use super::tab_strip::TabTitle;
use super::{OpenGraph, ToggleMode, ViewMode, Workspace, leaf_title};
use crate::theme::Theme;
use gpui::{Context, div, prelude::*, px};

/// Whether the view header carries the mode switch: no on an empty tab (nothing to
/// read or edit), and the graph has its own toggle.
pub(super) fn header_shows_mode_switch(current: &Option<String>, mode: ViewMode) -> bool {
    current.is_some() && !matches!(mode, ViewMode::Graph | ViewMode::Search)
}

impl Workspace {
    /// The view header: the note's folder and name, the graph toggle and the
    /// mode toggle. The back/forward arrows live in the tab strip, as Zed
    /// places them. Its buttons act on their group, made active first.
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
        let crumbs: Vec<(String, String)> = view
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/').map(|(dir, _)| dir))
            .map(|dir| {
                let mut path = String::new();
                dir.split('/')
                    .filter(|part| !part.is_empty())
                    .map(|part| {
                        if !path.is_empty() {
                            path.push('/');
                        }
                        path.push_str(part);
                        (part.to_string(), path.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut breadcrumb = div().flex().flex_row().items_center().whitespace_nowrap();
        for (ix, (label, path)) in crumbs.iter().cloned().enumerate() {
            breadcrumb = breadcrumb.child(
                div()
                    .id(("document-header-folder-crumb", ix))
                    .px(px(4.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_group(id, cx);
                        this.reveal_in_navigation(&path, window, cx);
                    }))
                    .child(div().text_color(theme.tokens.text_muted).child(label)),
            );
            if ix + 1 < crumbs.len() {
                breadcrumb = breadcrumb.child(
                    div()
                        .px(px(2.))
                        .text_color(theme.tokens.text_muted)
                        .child("/"),
                );
            }
        }
        // Obsidian's view action shows the current mode: a book while reading, a pencil while editing.
        let mode_icon = match view.mode {
            ViewMode::Reading => "book-open",
            _ => "pencil",
        };
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
        // Obsidian's view header: 40 px, the title in the small UI size.
        crate::ui::workspace_view_header(theme)
            .border_b_1()
            .border_color(theme.tokens.background_modifier_border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(12.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    // `.view-header-title-parent`: `--text-muted`.
                    .when(!crumbs.is_empty(), |d| {
                        d.child(breadcrumb)
                    })
                    .child(self.render_title(view, title.clone(), title_color, theme, cx)),
            )
            .child(
                div()
                    .id("document-header-tools")
                    .flex()
                    .flex_row()
                    .h_full()
                    .items_center()
                    .pl(px(10.))
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
