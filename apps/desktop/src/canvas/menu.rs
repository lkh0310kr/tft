//! The canvas's menus: a row of icon buttons that floats above the
//! selection (and sits at the bottom of the view as the card menu), and the
//! right-click list anchored at the pointer.
//!
//! This is the mechanism only. Which items Obsidian shows in each menu is
//! being probed; the lists `view.rs` builds from these pieces are
//! placeholders until that list arrives. Every button names an item id that
//! [`CanvasView::menu_pick`] dispatches on, so a menu's contents are a
//! `Vec` of items and nothing else.

use crate::canvas::view::CanvasView;
use crate::theme::Theme;
use gpui::{
    anchored, deferred, div, prelude::*, px, AnyElement, Context, Div, Pixels, Point, SharedString,
};

/// A button in an icon row: the id the pick reports, its Lucide icon, and
/// the tooltip.
#[derive(Debug, Clone, Copy)]
pub struct IconItem {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
}

/// An entry of a right-click list: the id the pick reports and its text.
#[derive(Debug, Clone, Copy)]
pub struct ListItem {
    pub id: &'static str,
    pub label: &'static str,
}

/// A button of an icon row, screen pixels.
pub const ICON_BUTTON: f32 = 32.0;
/// Gap between buttons and the row's padding.
pub const ICON_GAP: f32 = 0.0;
pub const ICON_PAD: f32 = 0.0;
/// A list entry whose label is this draws a separator line instead.
pub const SEPARATOR: &str = "---";

/// The width an icon row of `n` buttons takes.
pub fn icon_row_width(n: usize) -> f32 {
    n as f32 * ICON_BUTTON + n.saturating_sub(1) as f32 * ICON_GAP + 2.0 * ICON_PAD
}

/// The height of an icon row.
pub fn icon_row_height() -> f32 {
    ICON_BUTTON + 2.0 * ICON_PAD
}

/// A row of icon buttons; the caller positions it. `prefix` keys the
/// buttons' element ids, so two rows on screen do not share hover state.
pub fn icon_row(
    prefix: &'static str,
    items: &[IconItem],
    theme: &Theme,
    cx: &mut Context<CanvasView>,
) -> Div {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(ICON_GAP))
        .p(px(ICON_PAD))
        .rounded(px(4.))
        // `.canvas-menu`: the primary background in a border-coloured frame;
        // its buttons are `clickable-icon`s.
        .bg(theme.tokens.background_primary)
        .border_1()
        .border_color(theme.tokens.background_modifier_border)
        .shadow_md()
        .occlude();
    for (ix, item) in items.iter().enumerate() {
        let id = item.id;
        let group: SharedString = format!("{prefix}-{ix}").into();
        row = row.child(
            crate::ui::canvas_icon_button((prefix, ix), group, item.icon, item.label, theme)
                .on_click(cx.listener(move |this, _, window, cx| this.menu_pick(id, window, cx))),
        );
    }
    row
}

/// The right-click list at `position`, drawn over everything else in the
/// style of the file explorer's menu. A press outside it closes it.
pub fn context_menu(
    items: &[ListItem],
    position: Point<Pixels>,
    theme: &Theme,
    cx: &mut Context<CanvasView>,
) -> AnyElement {
    let mut list = crate::ui::menu_panel("canvas-context-menu", theme)
        .min_w(px(200.))
        .py_1()
        .occlude()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_context_menu(cx)));
    for (ix, item) in items.iter().enumerate() {
        let id = item.id;
        if item.label == SEPARATOR {
            list = list.child(crate::ui::menu_separator(theme));
            continue;
        }
        list = list.child(
            crate::ui::menu_row(("canvas-context-item", ix), theme)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_context_menu(cx);
                    this.menu_pick(id, window, cx);
                }))
                .child(item.label),
        );
    }
    deferred(
        anchored()
            .position(position)
            .snap_to_window_with_margin(px(8.))
            .child(list),
    )
    .with_priority(1)
    .into_any_element()
}
