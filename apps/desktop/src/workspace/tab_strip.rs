//! Tab strip rendering, tab title formatting, menu routing and pin actions.

use super::render::GroupView;
use super::tab_drag::{DROP_BAR_WIDTH, DragPart, TabDrag, TabDragPreview, measure_for_drag};
use super::{
    FileKind, HEADER_HEIGHT, ICON_SIZE, Pick, RIBBON_WIDTH, Tab, ViewMode, WINDOW_RADIUS,
    WindowFrame, Workspace, display_name, file_menu, icon, release_notes, window_handle,
};
use crate::theme::Theme;
use gpui::{
    Anchor, App, Context, DragMoveEvent, MouseDownEvent, Pixels, SharedString, Window, deferred,
    div, prelude::*, px,
};
use std::time::Instant;

pub(super) trait TabTitle {
    fn title(&self) -> SharedString;
}

impl TabTitle for Tab {
    fn title(&self) -> SharedString {
        match release_notes::version_of(self) {
            Some(version) => release_notes::title(&version).into(),
            None => leaf_title(self.path.as_deref(), self.mode),
        }
    }
}

/// The tab strip's name for a file: a note's name without `.md`, and a
/// canvas without `.canvas`, a PDF without `.pdf` or an audio or video file
/// without its extension too (observed: `tone.mp3` is "tone", `clip.mp4`
/// "clip"), which is how Obsidian titles its tabs -- no view of its own ever
/// shows an extension (the PDF's is observed: `doc.pdf` is the tab "doc" and
/// the leaf's `title` "doc").
pub(super) fn tab_display_name(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if (ext.eq_ignore_ascii_case("canvas")
                || ext.eq_ignore_ascii_case("pdf")
                || matches!(
                    FileKind::from_extension(&ext.to_ascii_lowercase()),
                    FileKind::Audio | FileKind::Video
                ))
                && !stem.is_empty() =>
        {
            stem
        }
        _ => display_name(file_name),
    }
}

/// The name a tab, view header and window title share for a leaf.
pub(crate) fn leaf_title(path: Option<&str>, mode: ViewMode) -> SharedString {
    match (path, mode) {
        (Some(path), _) => tab_display_name(path.rsplit('/').next().unwrap_or(path))
            .to_string()
            .into(),
        (None, ViewMode::Graph) => "Graph view".into(),
        (None, ViewMode::Search) => "Search".into(),
        (None, _) => "New tab".into(),
    }
}

impl Workspace {
    /// A group's tab strip. `first`/`last` mark the top-left/top-right corners, whose
    /// strips carry the collapsed-sidebar toggles and clear the window controls.
    pub(super) fn render_tab_strip(
        &self,
        view: &GroupView,
        window: Option<u64>,
        first: bool,
        last: bool,
        frame: WindowFrame,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let tabs = &view.tabs;
        let active_ix = view.active_tab;
        let group = view.id;
        let group_active = view.active;
        let frameless = frame.frameless;
        let mac = frame.mac_frame();
        // A popout has no sidebars or ribbon: its corner strips draw no toggle (observed).
        let popout = window.is_some();
        let left_open = !popout && self.left_dock.visible;
        // The window buttons overlay the right end of the last group's header; macOS has none.
        let under_controls = frameless && !mac && last;
        // The top-left strip keeps clear of the macOS traffic lights when the left sidebar is collapsed.
        let left_space = if first && !left_open {
            frame.left_space()
        } else {
            0.
        };
        let at_left = first && !left_open;
        let at_right = last;
        div()
            .flex()
            .flex_row()
            .items_end()
            .when(at_left && frame.corners.tl, |d| d.rounded_tl(WINDOW_RADIUS))
            .when(at_right && frame.corners.tr, |d| {
                d.rounded_tr(WINDOW_RADIUS)
            })
            .h(px(HEADER_HEIGHT))
            .pl(px(8. + left_space))
            .pr(if under_controls {
                px(3. * RIBBON_WIDTH)
            } else {
                px(8.)
            })
            .bg(theme.tokens.tab_container_background)
            .relative()
            .overflow_hidden()
            // A dragged tab over the strip moves the drop bar to the nearest boundary;
            // dropped, it lands there.
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.strip_drag_moved(group, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.drop_tab_at_bar(drag, group, None, cx)
            }))
            .when(self.tab_drag_state.active, |d| {
                d.child(measure_for_drag(
                    &self.tab_drag_state.bounds,
                    DragPart::Strip(group),
                ))
            })
            // The strip's outline is a bottom hairline, so the active tab paints over it
            // and joins the header without a seam; other tabs stop a pixel short.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(theme.tokens.tab_outline_color),
            )
            // Without the ribbon a collapsed left sidebar would have no way back;
            // macOS keeps the toggle here, after the traffic lights.
            .when(first && !popout && !left_open, |d| {
                d.child(self.sidebar_toggle(theme, cx).mb(px(8.)).mr(px(2.)))
            })
            // The tabs scroll sideways in their own box, so a crowded strip keeps every
            // tab reachable.
            .child(
                div()
                    .id(("tab-strip-scroll", group))
                    .flex()
                    .flex_row()
                    .items_end()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .overflow_x_scroll()
                    .children(tabs.iter().enumerate().map(|(ix, tab)| {
                        let active = ix == active_ix;
                        let pinned = tab.pinned;
                        let divider = !active && ix + 1 != active_ix;
                        let title = tab.title();
                        let drag = TabDrag {
                            group,
                            tab: ix,
                            title: title.clone(),
                        };
                        let this = cx.weak_entity();
                        // A tab showing a file previews it, anchored on the whole tab.
                        let preview = {
                            let this = this.clone();
                            let path = tab.path.clone();
                            let window = self.window_of_group(group);
                            move |over, anchor, modifiers, _: &mut Window, cx: &mut App| {
                                let Some(path) = path.clone() else {
                                    return;
                                };
                                this.update(cx, |this, cx| {
                                    this.row_hover(
                                        crate::hover_preview::Source::TabHeader,
                                        window,
                                        crate::hover_preview::exact(&path),
                                        path,
                                        over,
                                        anchor,
                                        modifiers,
                                        cx,
                                    )
                                })
                                .ok();
                            }
                        };
                        // Obsidian's tab, squared off for TFT: a 32 px row in the
                        // content colour, no corner arcs, the close button a small
                        // square. The inner box, not the tab, takes the hover wash.
                        crate::hover_preview::hover_row(
                            crate::ui::workspace_tab(("tab", ix), active, group_active, theme),
                            preview,
                        )
                        // The divider on an inactive tab's right edge, as Obsidian's
                        // `::after`: a 1 px line astride the edge, parting the strip from
                        // the new-tab button after the last tab. Deferred so the next tab's
                        // opaque background cannot clip its half.
                        .when(divider, |d| {
                            d.child(deferred(
                                div()
                                    .absolute()
                                    .right(px(-0.5))
                                    .top_0()
                                    .w(px(1.))
                                    .h_full()
                                    .bg(theme.tokens.tab_divider_color),
                            ))
                        })
                        // The active tab's bottom edge is a straight line.
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.activate_group(group, cx);
                            this.activate_tab(ix, cx)
                        }))
                        // Picking the tab up: the ghost is its title, and the workspace
                        // shows every group's zones.
                        .on_drag(drag, move |drag, grab, window, cx| {
                            let picked = drag.clone();
                            this.update(cx, |w, cx| w.begin_tab_drag(picked, cx)).ok();
                            let title = drag.title.clone();
                            let source = window.window_handle();
                            cx.new(|_| TabDragPreview {
                                title,
                                grab,
                                source,
                            })
                        })
                        // Where the tab is, for the strip's drop bar; a dropped tab lands
                        // there, else in front of it.
                        .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, _| {
                            if this.tab_drag_state.active {
                                this.tab_drag_state.tab_bounds.insert((group, ix), e.bounds);
                            }
                        }))
                        .when(self.tab_drag_state.active, |d| {
                            d.child(measure_for_drag(
                                &self.tab_drag_state.bounds,
                                DragPart::Tab(group, ix),
                            ))
                        })
                        .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                            this.drop_tab_at_bar(drag, group, Some(ix), cx)
                        }))
                        .on_mouse_down(
                            gpui::MouseButton::Middle,
                            cx.listener(move |this, _, _, cx| {
                                this.activate_group(group, cx);
                                this.close_tab(ix, cx)
                            }),
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_tab_menu(group, ix, e.position, cx)
                            }),
                        )
                        // The inner box: title and close button, padded `0 4px 0 8px`,
                        // washed on hover when the tab is not active.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.))
                                .pl(px(10.))
                                .pr(px(8.))
                                .when(!active, |d| {
                                    d.group_hover("tab", |s| {
                                        s.bg(theme.tokens.tab_inner_background_hover)
                                    })
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(title),
                                )
                                // The close button: no wash of its own, its cross
                                // brightens from muted to normal under the pointer.
                                .child(
                                    div()
                                        .id(("tab-close", ix))
                                        .group("tab-close")
                                        .flex_shrink_0()
                                        .w(px(18.))
                                        .h(px(18.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(2.))
                                        // A pinned tab shows its pin in the close button's
                                        // place; a click unpins.
                                        .when(!active && !pinned, |d| {
                                            d.invisible().group_hover("tab", |s| s.visible())
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.activate_group(group, cx);
                                            if pinned {
                                                this.activate_tab(ix, cx);
                                                this.toggle_pin(cx);
                                            } else {
                                                this.close_tab(ix, cx);
                                            }
                                        }))
                                        .child(
                                            icon(
                                                if pinned { "pin" } else { "x" },
                                                14.,
                                                theme.tokens.tab_text_color_focused_active,
                                            )
                                            .group_hover("tab-close", |s| {
                                                s.text_color(theme.tokens.text_normal)
                                            }),
                                        ),
                                ),
                        )
                    }))
                    .child(
                        div()
                            .id("new-tab")
                            .flex_shrink_0()
                            .ml(px(2.))
                            .w(px(32.))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(0.))
                            .cursor_pointer()
                            .group("new-tab")
                            .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_group(group, cx);
                                this.new_tab(cx)
                            }))
                            // A `clickable-icon`: `--icon-color`, not the text colour.
                            .child(
                                icon("plus", ICON_SIZE, theme.tokens.icon_color)
                                    .opacity(theme.tokens.icon_opacity)
                                    .group_hover("new-tab", |s| {
                                        s.opacity(theme.tokens.icon_opacity_hover)
                                            .text_color(theme.tokens.icon_color_hover)
                                    }),
                            ),
                    )
                    .child(window_handle(
                        div().id(("tab-strip-space", group)).flex_1().h_full(),
                        frameless,
                    )),
            )
            // The drop bar: an accent bar astride the boundary the dragged tab would land
            // at, deferred past the late-painted dividers.
            .when_some(
                self.tab_drag_state.bar.filter(|b| b.group == group),
                |d, bar| {
                    let left = bar.shown_x(Instant::now()) - bar.strip_x - DROP_BAR_WIDTH / 2.;
                    d.child(deferred(
                        div()
                            .absolute()
                            .left(px(left))
                            .bottom(px(6.))
                            .w(px(DROP_BAR_WIDTH))
                            .h(px(25.))
                            .rounded(px(DROP_BAR_WIDTH / 2.))
                            .bg(theme.tokens.interactive_accent),
                    ))
                },
            )
    }

    /// A right click on tab `ix`: the tab header's menu at the click. The click leaves
    /// the active tab where it was, so the rows name their tab.
    pub(super) fn open_tab_menu(
        &mut self,
        id: u64,
        ix: usize,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.stash_active_group();
        let Some(gix) = self.group_ix(id) else {
            return;
        };
        let active = gix == self.active_group && ix == self.groups[gix].state.active_tab;
        let group = &self.groups[gix];
        let Some(tab) = group.state.tabs.get(ix) else {
            return;
        };
        self.file_menu_state.menu = Some(file_menu::Menu {
            position,
            submenu: false,
            anchor: Anchor::TopLeft,
            target: file_menu::Target::Tab {
                group: id,
                ix,
                path: tab.path.clone().unwrap_or_default(),
                tabs: group.state.tabs.len(),
                pinned: tab.pinned,
                active,
            },
        });
        cx.notify();
    }

    /// A tab header row picked: its group made active; rows that close or move act on
    /// its index, the rest on the active tab.
    pub(super) fn tab_menu_pick(
        &mut self,
        pick: Pick,
        group: u64,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_group(group, cx);
        if self.active_group_id() != group || ix >= self.active_group_state.tabs.len() {
            return;
        }
        match pick {
            Pick::CloseTab => self.close_tab(ix, cx),
            Pick::CloseOthers => self.close_tabs_but(|i, t| i == ix || t.pinned, Some(ix), cx),
            Pick::CloseAfter => self.close_tabs_but(|i, t| i <= ix || t.pinned, Some(ix), cx),
            Pick::CloseAll => self.close_all_tabs(cx),
            Pick::TogglePin => {
                self.active_group_state.stash_active_tab();
                self.active_group_state.tabs[ix].pinned = !self.active_group_state.tabs[ix].pinned;
                self.stash_active_group();
                cx.notify();
            }
            Pick::MoveToNewWindow => {
                self.activate_tab(ix, cx);
                self.move_to_new_window(window, cx);
            }
            _ => {}
        }
    }

    /// `workspace:toggle-pin`: the active tab pinned or unpinned.
    pub(super) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        self.active_group_state.stash_active_tab();
        if let Some(tab) = self
            .active_group_state
            .tabs
            .get_mut(self.active_group_state.active_tab)
        {
            tab.pinned = !tab.pinned;
            self.stash_active_group();
            cx.notify();
        }
    }
}
