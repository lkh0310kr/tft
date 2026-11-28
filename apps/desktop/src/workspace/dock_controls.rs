//! Workspace sidebar toggle and dock resize controls.

use super::tab_drag::NoDragPreview;
use super::Workspace;
use crate::theme::Theme;
use gpui::{prelude::*, px, Context, FocusHandle, Focusable, Pixels, Window};
use std::ops::{Deref, DerefMut};
use tft_app::{place_new_tab, Tab, ViewMode, WorkspaceDockState};

/// State for the Files dock's visibility, sizing, and keyboard focus.
pub(super) struct DockState {
    pub(super) focus: FocusHandle,
    pub(super) keyboard: bool,
    pub(super) common: WorkspaceDockState,
    pub(super) width: f64,
    /// The inner edge is being dragged: keep the handle accent-coloured while
    /// the pointer is over the editor and hover no longer reaches it.
    pub(super) dragging: bool,
}

impl DockState {
    pub(super) fn new(focus: FocusHandle, width: f64) -> Self {
        Self {
            focus,
            keyboard: false,
            common: WorkspaceDockState::default(),
            width,
            dragging: false,
        }
    }
}

impl Deref for DockState {
    type Target = WorkspaceDockState;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl DerefMut for DockState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
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
            .ml(px(4.))
            .on_click(cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)))
    }

    /// A sidebar's resize handle: the 3 px strip on its inner edge, accent-coloured on
    /// hover and while it is being dragged.
    pub(super) fn dock_handle(&self, left: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        let dragging = self.left_dock.dragging;
        crate::ui::workspace_dock_resize_handle(
            if left {
                "left-dock-handle"
            } else {
                "right-dock-handle"
            },
            left,
            dragging,
            theme,
        )
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
        if left {
            let origin = f64::from(f32::from(bounds.origin.x));
            self.left_dock.width = f64::from(crate::ui::workspace_dock_width_at(
                x as f32,
                origin as f32,
                row as f32,
            ));
        }
        self.left_dock.dragging = true;
        cx.notify();
    }
}
