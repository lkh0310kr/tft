//! Per-group document header controls and title presentation.

use super::release_notes;
use super::render::GroupView;
use super::tab_strip::TabTitle;
use super::{leaf_title, OpenGraph, ToggleMode, ViewMode, Workspace};
use crate::theme::Theme;
use gpui::{div, prelude::*, px, Context};

/// Whether the view header carries the mode switch: no on an empty tab (nothing to
/// read or edit), and the graph has its own toggle.
pub(super) fn header_shows_mode_switch(current: &Option<String>, mode: ViewMode) -> bool {
    tft_app::workspace::workspace_header_shows_mode_switch(current.as_deref(), mode)
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
        let crumbs = tft_app::workspace::workspace_breadcrumbs(view.current.as_deref());
        let breadcrumb = crate::ui::workspace_breadcrumb_trail(
            crumbs
                .iter()
                .cloned()
                .enumerate()
                .map(|(ix, (label, path))| {
                    crate::ui::workspace_breadcrumb_item(
                        ("document-header-folder-crumb", ix),
                        label,
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_group(id, cx);
                        this.reveal_in_navigation(&path, window, cx);
                    }))
                    .into_any_element()
                })
                .collect::<Vec<_>>(),
            theme,
        );
        // Obsidian's view action shows the current mode: a book while reading, a pencil while editing.
        let mode_icon = crate::ui::workspace_document_mode_icon(view.mode);
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
        crate::ui::workspace_document_header(
            crate::ui::workspace_view_header_title()
                // `.view-header-title-parent`: `--text-muted`.
                .when(!crumbs.is_empty(), |d| {
                    d.child(breadcrumb)
                        .child(crate::ui::workspace_breadcrumb_separator(theme))
                })
                .child(self.render_title(view, title.clone(), title_color, theme, cx)),
            crate::ui::workspace_view_header_tools(("document-header-tools", id)).child(
                if release_notes_leaf || view.mode == ViewMode::Terminal {
                    // The release notes and a terminal are not notes: no graph and no mode switch.
                    div()
                } else {
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(2.))
                        .child(
                            crate::ui::workspace_graph_engine_button(
                                "graph",
                                "graph",
                                false,
                                view.mode == ViewMode::Graph && !self.radial_tree.active,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.activate_group(id, cx);
                                    if this.radial_tree.active {
                                        this.show_force_graph(window, cx)
                                    } else {
                                        this.open_graph_action(&OpenGraph, window, cx)
                                    }
                                },
                            )),
                        )
                        .child(
                            crate::ui::workspace_graph_engine_button(
                                "radial-tree",
                                "radial-tree",
                                true,
                                view.mode == ViewMode::Graph && self.radial_tree.active,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.activate_group(id, cx);
                                    if this.radial_tree.active {
                                        this.show_force_graph(window, cx)
                                    } else {
                                        this.show_radial_tree(window, cx)
                                    }
                                },
                            )),
                        )
                        .when(header_shows_mode_switch(&view.current, view.mode), |d| {
                            d.child(crate::ui::icon_button("mode", mode_icon, theme).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.activate_group(id, cx);
                                    this.toggle_mode(&ToggleMode, window, cx)
                                }),
                            ))
                        })
                },
            ),
            theme,
        )
    }
}
