//! Workspace sidebar toggle and dock resize controls.

use super::tab_drag::NoDragPreview;
use super::Workspace;
use crate::theme::Theme;
use gpui::{div, prelude::*, px, Context, FocusHandle, Focusable, Pixels, Window};
use tft_app::{place_new_tab, Tab, ViewMode};

/// State for the Files dock's visibility, sizing, and keyboard focus.
pub(super) struct DockState {
    pub(super) focus: FocusHandle,
    pub(super) keyboard: bool,
    pub(super) visible: bool,
    pub(super) width: f64,
}

impl DockState {
    pub(super) fn new(focus: FocusHandle, width: f64) -> Self {
        Self {
            focus,
            keyboard: false,
            visible: true,
            width,
        }
    }
}

/// The payload of a sidebar resize: which dock's inner edge is being dragged.
#[derive(Debug, Clone)]
pub(super) struct DockDrag {
    pub(super) left: bool,
}

/// The narrowest a sidebar can be dragged, and the narrowest recorded `width` is believed.
pub(super) const MIN_SIDEBAR_WIDTH: f64 = 150.;

impl Workspace {
    /// Opens or reuses the current group's central Search tab and focuses its query.
    pub fn show_search(
        &mut self,
        query: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self
            .active_group_state
            .tabs
            .iter()
            .position(|tab| tab.mode == ViewMode::Search)
        {
            self.activate_tab(ix, cx);
        } else {
            self.flush_save(cx);
            self.active_group_state.stash_active_tab();
            self.active_group_state.active_tab = place_new_tab(
                &mut self.active_group_state.tabs,
                self.active_group_state.active_tab,
                Tab::empty(ViewMode::Search),
            );
            self.load_tab(cx);
        }
        self.search.update(cx, |s, cx| s.set_query(query, cx));
        if let Some(window) = window {
            window.focus(&self.search.focus_handle(cx), cx);
        } else {
            self.focus.refocus_main = true;
        }
        cx.notify();
    }

    /// A sidebar's collapse toggle; it stays on screen when the sidebar is collapsed.
    pub(super) fn sidebar_toggle(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let command = "app:toggle-left-sidebar";
        crate::ui::icon_button("toggle-left", "panel-left", theme)
            .on_click(cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)))
    }

    /// A sidebar's resize handle: the 3 px strip on its inner edge, accent-coloured on hover.
    pub(super) fn dock_handle(&self, left: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(if left {
                "left-dock-handle"
            } else {
                "right-dock-handle"
            })
            .absolute()
            .top_0()
            .h_full()
            .w(px(3.))
            .when(left, |d| d.right(px(0.)).border_r_1())
            .when(!left, |d| d.left(px(0.)).border_l_1())
            .border_color(theme.tokens.background_modifier_border)
            .cursor_col_resize()
            .occlude()
            .hover(|s| {
                s.bg(theme.tokens.interactive_accent)
                    .border_color(theme.tokens.interactive_accent)
            })
            .on_drag(DockDrag { left }, |_, _, _, cx| cx.new(|_| NoDragPreview))
    }

    /// A sidebar handle dragged: the dock follows the pointer between a header row's
    /// width and what leaves the main area room.
    pub(super) fn drag_dock(
        &mut self,
        left: bool,
        bounds: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let x = f64::from(f32::from(position.x));
        let row = f64::from(f32::from(bounds.size.width));
        let max = (row - 2. * MIN_SIDEBAR_WIDTH).max(MIN_SIDEBAR_WIDTH);
        if left {
            let origin = f64::from(f32::from(bounds.origin.x));
            self.left_dock.width = (x - origin).clamp(MIN_SIDEBAR_WIDTH, max);
        }
        cx.notify();
    }
}
