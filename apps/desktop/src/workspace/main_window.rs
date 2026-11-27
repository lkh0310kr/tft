//! Main Desktop window zoom commands and application.

use super::Workspace;
use crate::zoom;
use gpui::{AnyWindowHandle, Context, SharedString};

/// Native main-window identity and presentation state. Popouts are separate GPUI
/// windows but route main-window actions through this state.
pub(super) struct MainWindowState {
    handle: AnyWindowHandle,
    zoom_level: f32,
    title: SharedString,
}

impl MainWindowState {
    pub(super) fn new(handle: AnyWindowHandle) -> Self {
        Self {
            handle,
            zoom_level: 0.0,
            title: SharedString::default(),
        }
    }

    pub(super) fn handle(&self) -> AnyWindowHandle {
        self.handle
    }

    pub(super) fn zoom_level(&self) -> f32 {
        self.zoom_level
    }

    pub(super) fn set_zoom_level(&mut self, level: f32) -> bool {
        if self.zoom_level == level {
            return false;
        }
        self.zoom_level = level;
        true
    }

    pub(super) fn title(&self) -> SharedString {
        self.title.clone()
    }

    pub(super) fn set_title(&mut self, title: SharedString) -> bool {
        if self.title == title {
            return false;
        }
        self.title = title;
        true
    }
}

impl Workspace {
    /// Sets the vault window's zoom level, records it in TFT's window file and applies it.
    pub(super) fn zoom_main(&mut self, level: f32, cx: &mut Context<Self>) {
        if !self.main_window.set_zoom_level(level) {
            return;
        }
        if let Some(state) = &self.vault_session.state {
            zoom::save_level(&state.root, level);
        }
        self.apply_main_zoom(cx);
    }

    /// Applies [`Self::zoom_level`] after this update: the command may have run in a
    /// popout while this workspace is being updated.
    pub(super) fn apply_main_zoom(&self, cx: &mut Context<Self>) {
        let main = self.main_window.handle();
        let level = self.main_window.zoom_level();
        cx.defer(move |cx| {
            main.update(cx, |_, window, cx| {
                zoom::set_level(window, cx, level);
            })
            .ok();
        });
    }
}
