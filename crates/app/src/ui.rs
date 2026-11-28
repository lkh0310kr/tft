//! The chrome several views draw the same way: tooltips, icon buttons,
//! menu frames, prompts and pickers.

use crate::assets::icon;
use crate::explorer::SortOrder;
use crate::theme::{Theme, FONT_UI_MEDIUM, FONT_UI_SMALL, HEADER_HEIGHT, ICON_SIZE};
use crate::workspace::SplitDirection;
use gpui::{
    canvas, div, prelude::*, px, AnyElement, AnyView, App, Bounds, Div, ElementId, Modifiers,
    Pixels, Render, SharedString, Stateful, Window,
};
use std::cell::Cell;
use std::rc::Rc;

/// Reports hover transitions with the element's window-space bounds. The
/// caller owns preview policy, delays, and the preview content.
pub fn hover_row(
    element: Stateful<Div>,
    report: impl Fn(bool, Bounds<Pixels>, Modifiers, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    hover_row_bordered(element, 0.0, report)
}

/// `hover_row` for an element whose own border must be included in the anchor.
pub fn hover_row_bordered(
    element: Stateful<Div>,
    border: f32,
    report: impl Fn(bool, Bounds<Pixels>, Modifiers, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let set_bounds = bounds.clone();
    element
        .relative()
        .child(
            canvas(move |bounds, _, _| set_bounds.set(bounds), |_, _, _, _| {})
                .absolute()
                .top(px(-border))
                .left(px(-border))
                .right(px(-border))
                .bottom(px(-border)),
        )
        .on_hover(move |over, window, cx| {
            report(*over, bounds.get(), window.modifiers(), window, cx)
        })
}

/// The tab shell shared by the native workspace and the GPUI Web composition.
/// The caller supplies tab actions and inner content; active/group colors and
/// geometry remain one implementation.
///
/// Square and full-height, as Zed's tab strip is: the tab fills the title bar's
/// [`HEADER_HEIGHT`] edge to edge, with only the inner content inset.
pub fn workspace_tab(
    id: impl Into<ElementId>,
    active: bool,
    group_active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .group("tab")
        .flex()
        .flex_row()
        .h(px(HEADER_HEIGHT))
        .w(px(200.))
        .flex_shrink_0()
        .relative()
        .bg(if active {
            theme.tokens.tab_background_active
        } else {
            theme.tokens.tab_container_background
        })
        .text_size(px(FONT_UI_SMALL))
        .text_color(if active && group_active {
            theme.tokens.tab_text_color_focused_active_current
        } else if active {
            theme.tokens.tab_text_color_focused_active
        } else {
            theme.tokens.tab_text_color_focused
        })
}

/// Outer frame for a workspace tab strip. Hosts add their corner treatment,
/// navigation controls, tabs, and platform window controls as children.
pub fn workspace_tab_strip_shell(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_end()
        .h(px(HEADER_HEIGHT))
        .bg(theme.tokens.tab_container_background)
        .relative()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(1.))
                .bg(theme.tokens.tab_outline_color),
        )
}

/// Shared title/close area inside a workspace tab. The caller supplies the
/// title and close action while padding and inactive hover treatment stay in
/// one place.
pub fn workspace_tab_inner(active: bool, theme: &Theme) -> Div {
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
        .when(!active, |container| {
            container.group_hover("tab", |style| {
                style.bg(theme.tokens.tab_inner_background_hover)
            })
        })
}

/// Shared title truncation and close-slot layout inside a workspace tab.
/// Hosts keep their own close/pin actions and provide the resulting elements.
pub fn workspace_tab_content(
    title: impl IntoElement,
    close_slot: impl IntoElement,
    active: bool,
    theme: &Theme,
) -> Div {
    workspace_tab_inner(active, theme)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(title),
        )
        .child(close_slot)
}

/// Shared close/pin slot for a workspace tab. Hosts attach close or unpin
/// behavior to the returned button.
pub fn workspace_tab_close_button(
    id: impl Into<ElementId>,
    active: bool,
    pinned: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .group("tab-close")
        .flex_shrink_0()
        .w(px(18.))
        .h(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .cursor_pointer()
        .when(!active && !pinned, |button| {
            button
                .invisible()
                .group_hover("tab", |style| style.visible())
        })
        .child(
            icon(
                if pinned { "pin" } else { "x" },
                14.,
                theme.tokens.tab_text_color_focused_active,
            )
            .group_hover("tab-close", |style| {
                style.text_color(theme.tokens.text_normal)
            }),
        )
}

/// The inactive tab's hairline divider, deferred by the host so the next tab's
/// opaque surface cannot cover its center pixel.
pub fn workspace_tab_divider(theme: &Theme) -> Div {
    div()
        .absolute()
        .right(px(-0.5))
        .top_0()
        .w(px(1.))
        .h_full()
        .bg(theme.tokens.tab_divider_color)
}

/// Accent insertion marker shown while dragging a tab through a strip.
pub fn workspace_tab_drop_indicator(after: bool, theme: &Theme) -> Div {
    div()
        .absolute()
        .when(after, |marker| marker.right(px(-3.)))
        .when(!after, |marker| marker.left(px(-3.)))
        .bottom(px(6.))
        .w(px(6.))
        .h(px(25.))
        .rounded(px(3.))
        .bg(theme.tokens.interactive_accent)
}

/// Shared square new-tab button; the host supplies the tab-creation action.
pub fn workspace_new_tab_button(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
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
        .hover(|style| style.bg(theme.tokens.background_modifier_hover))
        .child(
            icon("plus", ICON_SIZE, theme.tokens.icon_color)
                .opacity(theme.tokens.icon_opacity)
                .group_hover("new-tab", |style| {
                    style
                        .opacity(theme.tokens.icon_opacity_hover)
                        .text_color(theme.tokens.icon_color_hover)
                }),
        )
}

/// Horizontal, shrinkable tab viewport shared by each host's tab strip.
pub fn workspace_tab_strip_scroll(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_end()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_x_scroll()
}

/// The horizontal tab viewport, its host-provided tab elements, the new-tab
/// action, and an optional trailing host control in one shared composition.
pub fn workspace_tab_strip_tabs(
    id: impl Into<ElementId>,
    tabs: impl IntoIterator<Item = AnyElement>,
    new_tab: impl IntoElement,
    trailing: Option<AnyElement>,
) -> Stateful<Div> {
    let strip = workspace_tab_strip_scroll(id).children(tabs).child(new_tab);
    if let Some(trailing) = trailing {
        strip.child(trailing)
    } else {
        strip
    }
}

/// Fixed history-control cell at the start of a workspace tab strip.
/// Hosts attach their own back/forward actions to the two icon buttons.
pub fn workspace_tab_history_controls(back: impl IntoElement, forward: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .gap(px(0.))
        .pl(px(4.))
        .pr(px(4.))
        .child(back)
        .child(forward)
}

/// Navigation controls and their divider, shared by each host's tab strip.
pub fn workspace_tab_strip_navigation(
    back: impl IntoElement,
    forward: impl IntoElement,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .child(workspace_tab_history_controls(back, forward))
        .child(workspace_tab_strip_divider(theme))
}

/// Base flex layout for a row or column of workspace groups.
pub fn workspace_split_container(
    id: impl Into<ElementId>,
    direction: SplitDirection,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .when(direction == SplitDirection::Vertical, |layout| {
            layout.flex_row()
        })
        .when(direction == SplitDirection::Horizontal, |layout| {
            layout.flex_col()
        })
}

/// Base geometry and separator for one group inside a workspace split.
pub fn workspace_split_cell(direction: SplitDirection, separated: bool, theme: &Theme) -> Div {
    div()
        .relative()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .when(separated && direction == SplitDirection::Vertical, |cell| {
            cell.border_l_1()
        })
        .when(
            separated && direction == SplitDirection::Horizontal,
            |cell| cell.border_t_1(),
        )
        .when(separated, |cell| {
            cell.border_color(theme.tokens.background_modifier_border)
        })
}

/// Hairline separating history navigation from the horizontally scrolling tabs.
pub fn workspace_tab_strip_divider(theme: &Theme) -> Div {
    div()
        .h_full()
        .w(px(1.))
        .flex_shrink_0()
        .bg(theme.tokens.tab_divider_color)
}

/// Shared geometry and typography for the document header below a workspace tab strip.
/// Desktop and Web provide their own navigation and view actions as children.
pub fn workspace_view_header(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(crate::theme::HEADER_HEIGHT))
        .border_b_1()
        .border_color(theme.tokens.background_modifier_border)
        .pr(px(16.))
        .gap(px(10.))
        .text_size(px(FONT_UI_SMALL))
        .text_color(theme.tokens.text_normal)
}

/// Icon used by the document header to switch between reading and editing.
pub fn workspace_document_mode_icon(mode: crate::workspace::ViewMode) -> &'static str {
    if mode == crate::workspace::ViewMode::Reading {
        "book-open"
    } else {
        "pencil"
    }
}

/// Shared document-header assembly. The host supplies the breadcrumb/title
/// lane and the platform-specific tool lane.
pub fn workspace_document_header(
    title_lane: impl IntoElement,
    tools_lane: impl IntoElement,
    theme: &Theme,
) -> Div {
    workspace_view_header(theme)
        .child(title_lane)
        .child(tools_lane)
}

/// Shared breadcrumb and title lane below a workspace tab strip.
pub fn workspace_view_header_title() -> Div {
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
}

/// Shared tool lane at the end of a workspace document header.
pub fn workspace_view_header_tools(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .h_full()
        .items_center()
        .pl(px(10.))
        .gap(px(2.))
}

/// Shared selector button for the force graph and radial tree engines.
/// Hosts attach their own navigation action while sharing icon and selected styling.
pub fn workspace_graph_engine_button(
    id: impl Into<ElementId>,
    group: impl Into<SharedString>,
    radial_tree: bool,
    selected: bool,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_with_id(
        id,
        group,
        if radial_tree {
            "folder-tree"
        } else {
            "git-fork"
        },
        theme,
    )
    .when(selected, |button| {
        button.bg(theme.tokens.background_modifier_hover)
    })
}

/// One folder label in a document breadcrumb; the host attaches navigation.
pub fn workspace_breadcrumb_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(4.))
        .py(px(3.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|style| style.bg(theme.tokens.background_modifier_hover))
        .child(
            div()
                .text_color(theme.tokens.text_muted)
                .child(label.into()),
        )
}

/// Slash separator between folder crumbs and the current document title.
pub fn workspace_breadcrumb_separator(theme: &Theme) -> Div {
    div()
        .px(px(2.))
        .text_color(theme.tokens.text_muted)
        .child("/")
}

/// Lays out breadcrumb buttons and separators with one shared trail geometry.
pub fn workspace_breadcrumb_trail(
    items: impl IntoIterator<Item = AnyElement>,
    theme: &Theme,
) -> Div {
    let mut items = items.into_iter().peekable();
    let mut trail = div().flex().flex_row().items_center().whitespace_nowrap();
    while let Some(item) = items.next() {
        trail = trail.child(item);
        if items.peek().is_some() {
            trail = trail.child(workspace_breadcrumb_separator(theme));
        }
    }
    trail
}

/// Centered empty-tab content shared by the native and Web workspaces.
/// Hosts add their own available actions so platform-specific commands stay
/// with the host while spacing, typography, and hover treatment stay aligned.
pub fn workspace_empty_tab(theme: &Theme) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .text_color(theme.tokens.text_muted)
}

/// Shared action-list geometry for an empty workspace tab. Hosts decide which
/// actions their platform supports and provide the corresponding rows.
pub fn workspace_empty_action_list(actions: impl IntoIterator<Item = AnyElement>) -> Div {
    div().w(px(240.)).flex().flex_col().children(actions)
}

/// Centered notice used when a restored workspace view is not implemented by
/// the current host. The host supplies the available follow-up action.
pub fn workspace_notice(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    action: impl IntoElement,
    theme: &Theme,
) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.tokens.text_muted)
        .child(
            div()
                .w(px(360.))
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(FONT_UI_MEDIUM))
                        .text_color(theme.tokens.text_normal)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(px(FONT_UI_SMALL))
                        .text_center()
                        .child(detail.into()),
                )
                .child(div().pt_2().child(action.into_element())),
        )
}

/// Shared search surface used by the workspace Search view and the Files dock.
pub fn workspace_search_view(
    id: impl Into<ElementId>,
    input: impl IntoElement,
    controls: impl IntoElement,
    results: impl IntoElement,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .size_full()
        .bg(theme.tokens.background_primary)
        .child(
            div()
                .w_full()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().flex_1().min_w_0().child(input))
                .child(controls),
        )
        .child(div().flex_1().min_h_0().child(results))
}

/// One action in the shared empty-tab list. Callers attach their command or
/// platform event with `on_click`.
pub fn workspace_empty_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    hotkey: Option<SharedString>,
    theme: &Theme,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .w_full()
        .h(px(36.))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(8.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|style| style.bg(theme.tokens.background_modifier_hover))
        .text_size(px(FONT_UI_SMALL))
        .child(div().text_color(theme.tokens.text_normal).child(label))
        .children(hotkey.map(|keys| div().text_color(theme.tokens.text_faint).child(keys)))
}

/// Base layout for the left workspace dock. Hosts provide the width and add
/// their own resize handle, window controls, and dock contents.
pub fn workspace_dock(width: Pixels, theme: &Theme) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(width)
        .h_full()
        .bg(theme.tokens.tab_container_background)
}

/// Shared left-dock composition. Hosts provide their platform header, panel,
/// profile footer, and resize interaction while the dock's vertical layout
/// stays consistent.
pub fn workspace_dock_shell(
    width: Pixels,
    header: impl IntoElement,
    panel: impl IntoElement,
    footer: impl IntoElement,
    resize_handle: impl IntoElement,
    theme: &Theme,
) -> Div {
    workspace_dock(width, theme)
        .child(header)
        .child(div().flex_1().min_h_0().overflow_hidden().child(panel))
        .child(footer)
        .child(resize_handle)
}

/// Shared footer row at the bottom of the workspace dock.
pub fn workspace_dock_footer(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(HEADER_HEIGHT))
        .px(px(8.))
        .gap(px(2.))
        .border_t_1()
        .border_color(theme.tokens.divider_color)
}

/// Vault profile button shared by native and browser workspace footers.
pub fn workspace_dock_profile_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .min_w_0()
        .flex_1()
        .h(px(24.))
        .pr(px(6.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|style| style.bg(theme.tokens.background_modifier_hover))
        .child(
            div()
                .flex_shrink_0()
                .w(px(28.))
                .h(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    icon("chevrons-up-down", ICON_SIZE, theme.tokens.icon_color)
                        .opacity(theme.tokens.icon_opacity),
                ),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(FONT_UI_SMALL))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.tokens.vault_profile_color)
                .child(label.into()),
        )
}

/// Shared resize affordance at the inner edge of a workspace dock.
pub fn workspace_dock_resize_handle(
    id: impl Into<ElementId>,
    left: bool,
    dragging: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let accent = theme.tokens.interactive_accent;
    div()
        .id(id)
        .absolute()
        .top_0()
        .h_full()
        .w(px(3.))
        .when(left, |d| d.right(px(0.)).border_r_1())
        .when(!left, |d| d.left(px(0.)).border_l_1())
        .border_color(theme.tokens.background_modifier_border)
        .cursor_col_resize()
        .occlude()
        .hover(move |s| s.bg(accent).border_color(accent))
        .active(move |s| s.bg(accent).border_color(accent))
        .when(dragging, move |d| d.bg(accent).border_color(accent))
}

/// Computes the left dock width from a pointer position while preserving the
/// minimum width for both the dock and the workspace content area.
pub fn workspace_dock_width_at(pointer_x: f32, origin_x: f32, total_width: f32) -> f32 {
    const MIN_DOCK_WIDTH: f32 = 150.0;
    let max = (total_width - MIN_DOCK_WIDTH * 2.0).max(MIN_DOCK_WIDTH);
    (pointer_x - origin_x).clamp(MIN_DOCK_WIDTH, max)
}

/// Shared top row for a workspace dock. Hosts insert their own tabs and
/// window-specific controls into the returned container.
pub fn workspace_dock_header(theme: &Theme) -> Div {
    div()
        .h(px(HEADER_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .bg(theme.tokens.sidebar_tab_container_background)
        .border_b_1()
        .border_color(theme.tokens.tab_outline_color)
}

/// Shared icon tab used by the Files and Search docks in Desktop and Web.
pub fn dock_tab(
    id: &'static str,
    icon_name: &'static str,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let tokens = &theme.tokens;
    div()
        .id(id)
        .group(id)
        .flex_shrink_0()
        .w(px(28.))
        .h(px(25.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .rounded(px(4.))
        .when(active, |d| d.bg(tokens.background_modifier_hover))
        .hover(|s| s.bg(tokens.background_modifier_hover))
        .child(
            icon(
                icon_name,
                ICON_SIZE,
                if active {
                    tokens.icon_color_focused
                } else {
                    tokens.icon_color
                },
            )
            .opacity(if active {
                tokens.icon_opacity_active
            } else {
                tokens.icon_opacity
            })
            .when(!active, |s| {
                s.group_hover(id, |s| {
                    s.opacity(tokens.icon_opacity_hover)
                        .text_color(tokens.icon_color_hover)
                })
            }),
        )
}

/// A compact GPUI button used by the shared Workspace shell.
pub fn action_button(label: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(label)
        .px_3()
        .py_2()
        .rounded_md()
        .bg(theme.tokens.background_modifier_hover)
        .text_size(px(13.))
        .cursor_pointer()
        .hover(|style| style.bg(theme.tokens.background_modifier_hover))
        .child(label)
}

/// The base file-tree row used by the native Explorer and Web workspace.
/// Callers attach their own navigation, rename and context-menu actions.
pub fn file_tree_row(
    id: impl Into<ElementId>,
    depth: usize,
    active: bool,
    focused: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let t = &theme.tokens;
    div()
        .id(id)
        .w_full()
        .h(px(26.))
        .flex()
        .flex_row()
        .items_center()
        .border_2()
        .border_color(if focused {
            t.background_modifier_border_focus
        } else {
            gpui::transparent_black()
        })
        .pl(px(8. + depth as f32 * 20.))
        .pr(px(8.))
        .cursor_pointer()
        .text_size(px(FONT_UI_SMALL))
        .text_color(if active {
            t.nav_item_color_active
        } else {
            t.nav_item_color
        })
        .when(active, |d| d.bg(t.nav_item_background_active))
        .hover(|s| {
            s.bg(t.nav_item_background_hover)
                .text_color(t.nav_item_color_hover)
        })
        .relative()
}

/// Folder expander slot used by both file-tree hosts.
pub fn file_tree_row_marker(is_folder: bool, collapsed: bool, theme: &Theme) -> Div {
    div()
        .w(px(14.))
        .mr(px(4.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .opacity(theme.tokens.icon_opacity)
        .when(is_folder, |slot| {
            slot.child(icon(
                if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                14.,
                theme.tokens.nav_collapse_icon_color,
            ))
        })
}

/// Indentation guides at the same 20 px rhythm as file-tree row indentation.
pub fn file_tree_indent_guides(depth: usize, theme: &Theme) -> Vec<Div> {
    (0..depth)
        .map(|level| {
            div()
                .absolute()
                .left(px(8. + level as f32 * 20.))
                .top_0()
                .bottom_0()
                .w(px(1.))
                .bg(theme.tokens.background_modifier_border)
        })
        .collect()
}

/// Single-line, ellipsized label slot shared by file-tree rows.
pub fn file_tree_row_label() -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
}

/// Extension tag at the trailing edge of a file-tree row.
pub fn file_tree_row_tag(tag: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex_shrink_0()
        .ml_2()
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.tokens.nav_tag_color)
        .child(tag.into())
}

/// Shared Files dock toolbar frame; hosts provide controls and their actions.
pub fn file_tree_toolbar(theme: &Theme) -> Div {
    div()
        .h(px(HEADER_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .border_b_1()
        .border_color(theme.tokens.tab_outline_color)
        .relative()
}

/// One shared sort option for the Files tree's host-owned menu.
pub fn file_tree_sort_item(
    prefix: &'static str,
    index: usize,
    order: SortOrder,
    current: SortOrder,
    theme: &Theme,
) -> Stateful<Div> {
    menu_item(prefix, index, None, order.label(), order == current, theme)
}

/// Sort menu section divider, paired with [`file_tree_sort_item`].
pub fn file_tree_sort_separator(theme: &Theme) -> Div {
    menu_separator(theme)
}

/// The small label a button shows on hover.
pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        // Obsidian's `.tooltip`: `--tooltip-color` on `--tooltip-background`
        // under `--tooltip-shadow`, no border (probe of 2026-11-04).
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme.tokens.tooltip_background)
            .shadow_md()
            .text_size(px(12.))
            .text_color(theme.tokens.tooltip_color)
            .child(self.text.clone())
    }
}

/// A tooltip builder for `.tooltip(..)`.
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| Tooltip { text: text.clone() }).into()
}

/// One `clickable-icon`. `glyph` draws a text symbol instead of a Lucide
/// icon; `label` adds the hover tooltip; `active` paints the hover wash and
/// the focused icon colour.
#[allow(clippy::too_many_arguments)]
fn icon_button_in(
    id: impl Into<ElementId>,
    group: SharedString,
    glyph: Option<&'static str>,
    name: &'static str,
    label: Option<SharedString>,
    active: bool,
    width: f32,
    height: f32,
    icon_size: f32,
    font_size: f32,
    theme: &Theme,
) -> Stateful<Div> {
    let t = &theme.tokens;
    let color = if active {
        t.icon_color_focused
    } else {
        t.icon_color
    };
    let opacity = if active {
        t.icon_opacity_active
    } else {
        t.icon_opacity
    };
    let mut button = div()
        .id(id)
        .group(group.clone())
        .flex_shrink_0()
        .w(px(width))
        .h(px(height))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .when(active, |d| d.bg(t.background_modifier_hover))
        .when_some(label, |d, text| d.tooltip(tooltip(text)));
    button = match glyph {
        Some(glyph) => button
            .text_size(px(font_size))
            .text_color(color)
            .hover(|s| {
                s.bg(t.background_modifier_hover)
                    .text_color(t.icon_color_hover)
            })
            .child(glyph),
        None => button.hover(|s| s.bg(t.background_modifier_hover)).child(
            icon(name, icon_size, color)
                .opacity(opacity)
                .group_hover(group, |s| {
                    s.opacity(t.icon_opacity_hover)
                        .text_color(t.icon_color_hover)
                }),
        ),
    };
    button
}

/// The view header's back/forward/link buttons: 28 by 24, no tooltip.
pub fn icon_button(id: &'static str, name: &'static str, theme: &Theme) -> Stateful<Div> {
    icon_button_in(
        id,
        id.into(),
        None,
        name,
        None,
        false,
        28.,
        24.,
        ICON_SIZE,
        13.,
        theme,
    )
}

/// Icon button with a caller-generated ID for repeated controls such as pane headers.
pub fn icon_button_with_id(
    id: impl Into<ElementId>,
    group: impl Into<SharedString>,
    name: &'static str,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_in(
        id,
        group.into(),
        None,
        name,
        None,
        false,
        28.,
        24.,
        ICON_SIZE,
        13.,
        theme,
    )
}

/// The explorer's header buttons: [`icon_button`] with a tooltip and the
/// `is-active` state.
pub fn header_button(
    id: &'static str,
    name: &'static str,
    label: &'static str,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_in(
        id,
        id.into(),
        None,
        name,
        Some(label.into()),
        active,
        28.,
        24.,
        ICON_SIZE,
        13.,
        theme,
    )
}

/// Header button with a caller-generated ID for repeated pane actions.
pub fn header_button_with_id(
    id: impl Into<ElementId>,
    group: impl Into<SharedString>,
    name: &'static str,
    label: impl Into<SharedString>,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_in(
        id,
        group.into(),
        None,
        name,
        Some(label.into()),
        active,
        28.,
        24.,
        ICON_SIZE,
        13.,
        theme,
    )
}

/// A 32 px canvas-menu button with a tooltip.
pub fn canvas_icon_button(
    id: (&'static str, usize),
    group: SharedString,
    name: &'static str,
    label: &'static str,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_in(
        id,
        group,
        None,
        name,
        Some(label.into()),
        false,
        32.,
        32.,
        ICON_SIZE,
        13.,
        theme,
    )
}

/// A PDF toolbar button: the `clickable-icon` at `width` × `height`, with
/// its enabled, active and tooltip states. The caller adds the click.
#[allow(clippy::too_many_arguments)]
pub fn toolbar_button(
    id: &'static str,
    name: &str,
    tip: &'static str,
    enabled: bool,
    active: bool,
    width: f32,
    height: f32,
    icon_size: f32,
    radius: f32,
    theme: &Theme,
) -> Stateful<Div> {
    let t = &theme.tokens;
    div()
        .id(id)
        .group(id)
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius))
        .when(active, |d| d.bg(t.background_modifier_active_hover))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(t.background_modifier_hover))
        })
        .tooltip(tooltip(tip))
        .child(if !enabled {
            icon(name, icon_size, t.text_faint)
        } else if active {
            icon(name, icon_size, t.icon_color_active).opacity(t.icon_opacity_active)
        } else {
            icon(name, icon_size, t.icon_color)
                .opacity(t.icon_opacity)
                .group_hover(id, |s| {
                    s.opacity(t.icon_opacity_hover)
                        .text_color(t.icon_color_hover)
                })
        })
}

/// The graph view's text-glyph buttons.
pub fn glyph_button(
    id: impl Into<ElementId>,
    glyph: &'static str,
    size: f32,
    font_size: f32,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_in(
        id,
        SharedString::from(glyph),
        Some(glyph),
        "",
        None,
        false,
        size,
        size,
        font_size,
        font_size,
        theme,
    )
}

/// Obsidian's `.menu`: the menu background in a border-coloured frame,
/// rows in the normal text colour at the small UI size.
pub fn menu_panel(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .bg(theme.tokens.menu_background)
        .border_1()
        .border_color(theme.tokens.menu_border_color)
        .rounded_md()
        .shadow_lg()
        .text_size(px(FONT_UI_SMALL))
        .text_color(theme.tokens.text_normal)
}

/// The hairline between two menu sections.
pub fn menu_separator(theme: &Theme) -> Div {
    div()
        .my_1()
        .h(px(1.))
        .bg(theme.tokens.background_modifier_border)
}

/// A menu row: the hover wash, the 3 px horizontal inset and the icon gap
/// every menu's items share.
pub fn menu_row(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .cursor_pointer()
        .text_color(theme.tokens.text_normal)
        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
}

/// The panel a prompt, picker or chooser floats in, at `width` pixels.
pub fn modal_panel(id: impl Into<ElementId>, width: f32, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(width))
        .flex()
        .flex_col()
        .bg(theme.tokens.prompt_background)
        .border_1()
        .border_color(theme.tokens.prompt_border_color)
        .rounded_lg()
        .shadow_lg()
}

/// Shared frame for file and command palettes.
pub fn palette_panel(id: impl Into<ElementId>, width: f32, theme: &Theme) -> Stateful<Div> {
    modal_panel(id, width, theme).overflow_hidden()
}

/// Search field slot shared by both palettes: the field sits flat on the
/// panel, with no divider under it.
pub fn palette_search_slot(_theme: &Theme) -> Div {
    div().w_full().px(px(6.)).py(px(4.))
}

/// Inset list region shared by both palettes.
pub fn palette_results_slot() -> Div {
    div().w_full().px(px(6.)).py(px(6.))
}

/// A palette result row. The caller supplies its contents and click action.
pub fn palette_result_row(
    id: impl Into<ElementId>,
    selected: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .h(px(38.))
        .px(px(10.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .rounded(px(4.))
        .cursor_pointer()
        .when(selected, |d| d.bg(theme.tokens.background_modifier_hover))
        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
}

/// A dialog's panel: the secondary background in a border-coloured frame,
/// the large radius and shadow every one of TFT's dialogs shares.
pub fn dialog_panel(id: &'static str, width: f32, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(width))
        .flex()
        .flex_col()
        .p_4()
        .bg(theme.tokens.background_secondary)
        .border_1()
        .border_color(theme.tokens.background_modifier_border)
        .rounded_lg()
        .shadow_lg()
}

/// The title row of a [`dialog_panel`], with its close button.
pub fn dialog_header(
    id: &'static str,
    title: &'static str,
    theme: &Theme,
    on_close: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.tokens.text_normal)
                .child(title),
        )
        .child(
            div()
                .id((id, 1usize))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                .on_click(on_close)
                .child(icon("x", 16., theme.tokens.text_muted)),
        )
}

/// A dialog button: `primary` is Obsidian's `mod-cta`, `error` its
/// `mod-destructive` accent.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    error: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(FONT_UI_SMALL))
        .when(primary && !error, |d| {
            d.bg(theme.tokens.interactive_accent)
                .text_color(theme.tokens.text_on_accent)
                .hover(|s| s.bg(theme.tokens.interactive_accent_hover))
        })
        .when(error, |d| {
            d.bg(theme.tokens.text_error)
                .text_color(theme.tokens.text_on_accent)
        })
        .when(!primary && !error, |d| {
            d.bg(theme.tokens.background_modifier_hover)
                .text_color(theme.tokens.text_normal)
                .hover(|s| s.bg(theme.tokens.interactive_hover))
        })
        .child(label)
}

/// The keyboard focus ring drawn over a row.
pub fn focus_ring(radius: f32, theme: &Theme) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded(px(radius))
        .border_2()
        .border_color(theme.tokens.background_modifier_border_focus)
}

/// Obsidian's 34 by 19 settings toggle.
pub fn toggle(id: (&'static str, usize), on: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .relative()
        .w(px(34.))
        .h(px(19.))
        .rounded_full()
        .bg(if on {
            theme.tokens.interactive_accent
        } else {
            theme.tokens.background_modifier_border_hover
        })
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 17. } else { 2. }))
                .size(px(15.))
                .rounded_full()
                .bg(theme.tokens.toggle_thumb_color),
        )
}

/// A menu in Obsidian's shape.
pub fn menu(id: &'static str, theme: &Theme) -> Stateful<Div> {
    menu_panel(id, theme)
        .min_w(px(172.))
        .p(px(6.))
        .rounded(px(8.))
        .occlude()
}

/// One `menu-item` with an optional icon or check.
pub fn menu_item(
    prefix: &'static str,
    ix: usize,
    icon_name: Option<&'static str>,
    title: &'static str,
    checked: bool,
    theme: &Theme,
) -> Stateful<Div> {
    menu_item_with_enabled(prefix, ix, icon_name, title, checked, true, theme)
}

/// A menu item with a shared disabled appearance. Disabled rows do not react
/// to hover and use the muted text/icon tokens.
pub fn menu_item_with_enabled(
    prefix: &'static str,
    ix: usize,
    icon_name: Option<&'static str>,
    title: impl Into<SharedString>,
    checked: bool,
    enabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let icon_color = if enabled {
        theme.tokens.text_muted
    } else {
        theme.tokens.text_faint
    };
    let glyph: Option<gpui::Svg> = if checked {
        Some(icon("check", 14., icon_color))
    } else {
        icon_name.map(|n| icon(n, ICON_SIZE, icon_color))
    };
    div()
        .id((prefix, ix))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .when(enabled, |row| {
            row.cursor_pointer()
                .hover(|s| s.bg(theme.tokens.background_modifier_hover))
        })
        .when(!enabled, |row| {
            row.cursor_default().text_color(theme.tokens.text_faint)
        })
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .children(glyph),
        )
        .child(div().whitespace_nowrap().child(title.into()))
}
