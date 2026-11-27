//! Workspace ownership and lifecycle for the Desktop Settings window.

use super::{VaultState, Workspace};
use crate::settings::{SettingsView, SettingsWindow};
use gpui::Entity;
use gpui::{App, Context, Window};
use serde_json::Value;
use std::sync::Arc;

/// Settings view entity and the window that presents it.
pub(super) struct SettingsWindowState {
    view: Entity<SettingsView>,
    window: Option<gpui::WindowHandle<SettingsWindow>>,
}

impl SettingsWindowState {
    pub(super) fn new(view: Entity<SettingsView>) -> Self {
        Self { view, window: None }
    }

    pub(super) fn describe(&self, rows: usize, cx: &App) -> Value {
        self.view.read(cx).describe(rows, cx)
    }

    pub(super) fn set_vault_state(&self, state: Arc<VaultState>, cx: &mut Context<Workspace>) {
        self.view
            .update(cx, |settings, cx| settings.set_state(state, cx));
    }
}

impl Workspace {
    /// Opens a Settings dropdown's options, for `--menu`.
    pub fn open_settings_menu(&mut self, key: &str, cx: &mut Context<Self>) {
        self.settings_state
            .view
            .update(cx, |s, cx| s.open_menu(key, cx));
    }

    /// Opens the Settings window, and brings it forward if it is already up.
    ///
    /// Obsidian's Settings is a modal; TFT's is a window of its own, so
    /// this takes nothing away from the workspace: the vault behind it
    /// keeps its keys and its focus.
    pub fn open_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_editor_suggest(cx);
        self.vault_switcher_state.close_menu();
        let view = self.settings_state.view.clone();
        if let Some(handle) = self.settings_state.window {
            let raised = handle
                .update(cx, |_, window, cx| {
                    view.update(cx, |s, cx| s.open(window, cx));
                    window.activate_window();
                })
                .is_ok();
            if raised {
                cx.notify();
                return;
            }
        }
        match SettingsWindow::open(view, cx.weak_entity(), cx) {
            Ok(handle) => self.settings_state.window = Some(handle),
            Err(err) => self.report(format!("Could not open Settings: {err}"), cx),
        }
        cx.notify();
    }

    /// Takes the Settings window down, from its own Escape or its ×.
    pub(super) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_state.window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        cx.notify();
    }

    /// Whether the Settings window is up. A handle left behind by a window
    /// the desktop closed reads as gone, which is what makes this the test
    /// rather than `settings_window.is_some()`.
    pub fn settings_up(&self, cx: &App) -> bool {
        self.settings_state
            .window
            .is_some_and(|handle| handle.read(cx).is_ok())
    }

    /// The Settings window, for the control socket: input goes there while
    /// it is up, since that is the window a caller is looking at.
    pub fn settings_window(&self, cx: &App) -> Option<gpui::AnyWindowHandle> {
        self.settings_up(cx)
            .then(|| self.settings_state.window.map(Into::into))
            .flatten()
    }
}
