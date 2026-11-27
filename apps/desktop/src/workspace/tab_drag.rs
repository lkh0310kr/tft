//! Tab drag payloads, hit-test geometry, drop indicators, and drag previews.

use crate::theme::{FONT_UI_SMALL, Theme};
use gpui::{Context, CursorStyle, Pixels, SharedString, Window, canvas, div, prelude::*, px};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tft_app::SplitDirection;

/// Live state for one tab drag, shared by drag coordination and frame rendering.
#[derive(Default)]
pub(super) struct TabDragState {
    pub(super) active: bool,
    pub(super) payload: Option<TabDrag>,
    pub(super) bounds: Rc<RefCell<DragBounds>>,
    pub(super) drop: Option<(u64, DropZone)>,
    pub(super) bar: Option<DropBar>,
    pub(super) tab_bounds: HashMap<(u64, usize), gpui::Bounds<Pixels>>,
}

/// The payload of a tab drag: the group's strip and tab index. The tab stays until the drop.
#[derive(Debug, Clone)]
pub(super) struct TabDrag {
    pub(super) group: u64,
    pub(super) tab: usize,
    pub(super) title: SharedString,
}

/// What a tab drag measured, each in its own window's coordinates, for the hit test across windows.
#[derive(Default)]
pub(super) struct DragBounds {
    pub(super) strips: HashMap<u64, gpui::Bounds<Pixels>>,
    pub(super) contents: HashMap<u64, gpui::Bounds<Pixels>>,
    pub(super) tabs: HashMap<(u64, usize), gpui::Bounds<Pixels>>,
}

/// Which of [`DragBounds`]' maps a measured element goes in.
#[derive(Clone, Copy)]
pub(super) enum DragPart {
    Strip(u64),
    Content(u64),
    Tab(u64, usize),
}

/// An invisible child that writes where its parent was laid out into `bounds`, for cross-window drags.
pub(super) fn measure_for_drag(
    bounds: &Rc<RefCell<DragBounds>>,
    part: DragPart,
) -> impl IntoElement {
    let bounds = bounds.clone();
    canvas(
        move |b, _, _| {
            let mut all = bounds.borrow_mut();
            match part {
                DragPart::Strip(g) => all.strips.insert(g, b),
                DragPart::Content(g) => all.contents.insert(g, b),
                DragPart::Tab(g, ix) => all.tabs.insert((g, ix), b),
            };
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Where a tab dragged over a group's content would land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    /// The zone `position` falls in: an outer band on each side splits, the middle
    /// joins the strip; the nearer edge wins a corner.
    pub(super) fn at(
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
    ) -> DropZone {
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        let x = f32::from(position.x - bounds.origin.x);
        let y = f32::from(position.y - bounds.origin.y);
        let band = w.min(h) * DROP_EDGE_SHARE;
        let edges = [
            (DropZone::Left, x),
            (DropZone::Right, w - x),
            (DropZone::Top, y),
            (DropZone::Bottom, h - y),
        ];
        let (zone, distance) = edges
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((DropZone::Center, 0.0));
        if distance < band {
            zone
        } else {
            DropZone::Center
        }
    }

    /// The split a side zone makes, and whether the new group goes after the dropped-on one.
    pub(super) fn split(self) -> Option<(SplitDirection, bool)> {
        match self {
            DropZone::Center => None,
            DropZone::Left => Some((SplitDirection::Vertical, false)),
            DropZone::Right => Some((SplitDirection::Vertical, true)),
            DropZone::Top => Some((SplitDirection::Horizontal, false)),
            DropZone::Bottom => Some((SplitDirection::Horizontal, true)),
        }
    }
}

/// How much of a group's shorter side, from each edge, drops as a split.
pub(super) const DROP_EDGE_SHARE: f32 = 0.3;

/// Where a tab dragged over a strip would land: before tab `at` at the nearest
/// boundary, with an accent bar that slides there from `from`.
#[derive(Debug, Clone, Copy)]
pub(super) struct DropBar {
    pub(super) group: u64,
    pub(super) at: usize,
    pub(super) x: f32,
    pub(super) from: f32,
    pub(super) since: Instant,
    /// The strip's left edge, which the bar's position is drawn from.
    pub(super) strip_x: f32,
}

impl DropBar {
    /// Where the bar is drawn at `now`, easing toward `x` so it lands softly.
    pub(super) fn shown_x(&self, now: Instant) -> f32 {
        let t =
            now.saturating_duration_since(self.since).as_secs_f32() / DROP_BAR_SLIDE.as_secs_f32();
        if t >= 1. {
            return self.x;
        }
        let eased = 1. - (1. - t) * (1. - t);
        self.from + (self.x - self.from) * eased
    }

    pub(super) fn settled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= DROP_BAR_SLIDE
    }
}

/// How long the drop bar takes to slide to a new boundary.
pub(super) const DROP_BAR_SLIDE: Duration = Duration::from_millis(120);
/// The drop bar's width; it straddles the boundary between two tabs.
pub(super) const DROP_BAR_WIDTH: f32 = 3.;

/// The ghost that follows the pointer while a tab is dragged: the tab's title on a
/// card placed at the cursor's bottom right, so it covers neither strip nor bar.
/// gpui draws it where the drag started less `grab`; the card is offset past that.
pub(super) struct TabDragPreview {
    pub(super) title: SharedString,
    /// Where in the tab the press landed.
    pub(super) grab: gpui::Point<gpui::Pixels>,
    /// The window it was picked up in; gpui paints the active drag in every window,
    /// so others would show a frozen ghost (seen on X11).
    pub(super) source: gpui::AnyWindowHandle,
}

/// From the pointer's hotspot to the ghost's top left: the grabbing hand's bottom right corner.
pub(super) const DRAG_GHOST_GAP: f32 = 8.;

impl Render for TabDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.window_handle() != self.source {
            return div().into_any_element();
        }
        let theme = cx.global::<Theme>().clone();
        // Padding on a wrapper, not a margin: the layout drops a root element's margin.
        div()
            .pl(self.grab.x + px(DRAG_GHOST_GAP))
            .pt(self.grab.y + px(DRAG_GHOST_GAP))
            // The drag's view paints last, and a cursor asked for without a hitbox
            // wins; this is where the grabbing hand comes from.
            .child(canvas(
                |_, _, _| (),
                |_, _, window, _| window.set_window_cursor_style(CursorStyle::ClosedHand),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(8.))
                    .max_w(px(200.))
                    .rounded(px(4.))
                    .bg(theme.tokens.background_primary)
                    .border_1()
                    .border_color(theme.tokens.background_modifier_border)
                    .shadow_md()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.tokens.text_normal)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.title.clone()),
            )
            .into_any_element()
    }
}

/// The drag's on-screen stand-in: nothing, the divider moves in place.
pub(super) struct NoDragPreview;

impl Render for NoDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
