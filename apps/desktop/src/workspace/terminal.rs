//! The terminal pane: spawning the shell, painting its view, and the tab that holds it.

use gpui::{px, App, Context, Edges, Entity, FocusHandle, Hsla, Window};
use tft_app::{place_new_tab, Tab};
use tft_terminal::{ColorPalette, TerminalConfig, TerminalSession, TerminalView};

use super::{ViewMode, Workspace};
use crate::theme::Theme;

/// Where the terminal session lives for the workspace. There is one shell,
/// shared by every group that shows a terminal, as the Search pane is.
#[derive(Default)]
pub(super) struct TerminalPaneState {
    pub(super) session: Option<TerminalSession>,
}

impl Workspace {
    /// `terminal:open`: a terminal tab in the active group, its shell spawned
    /// the first time and after it exits, and the keyboard on the grid.
    pub fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        if let Some(ix) = self
            .active_group_state
            .tabs
            .iter()
            .position(|tab| tab.mode == ViewMode::Terminal)
        {
            if ix != self.active_group_state.active_tab {
                self.activate_tab(ix, cx);
            }
        } else {
            self.flush_save(cx);
            self.active_group_state.stash_active_tab();
            self.active_group_state.active_tab = place_new_tab(
                &mut self.active_group_state.tabs,
                self.active_group_state.active_tab,
                Tab::empty(ViewMode::Terminal),
            );
            self.load_tab(cx);
        }
        self.ensure_terminal(cx);
        self.focus_main(window, cx);
        cx.notify();
    }

    /// Starts the shell when there is none, or when the last one exited.
    pub(super) fn ensure_terminal(&mut self, cx: &mut Context<Self>) {
        if self
            .terminal
            .session
            .as_ref()
            .is_some_and(|session| !session.has_exited())
        {
            return;
        }
        // Dropping an exited session kills its child and closes the PTY.
        self.terminal.session = None;
        let theme = cx.global::<Theme>().clone();
        let config = terminal_config(&theme);
        let cwd = self
            .vault_root()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        match TerminalSession::spawn(&cwd, None, config, cx) {
            Ok(session) => self.terminal.session = Some(session),
            Err(err) => self.report(format!("Could not open the terminal: {err}"), cx),
        }
    }

    /// The group whose area paints the terminal, as [`Workspace::search_group`].
    pub(super) fn terminal_group(&self) -> Option<u64> {
        if self.active_group_state.mode == ViewMode::Terminal {
            return Some(self.active_group_id());
        }
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.into_iter().find(|id| {
            self.group_ix(*id).is_some_and(|ix| {
                let group = &self.groups[ix];
                group
                    .state
                    .tabs
                    .get(group.state.active_tab)
                    .is_some_and(|tab| tab.mode == ViewMode::Terminal)
            })
        })
    }

    pub(super) fn terminal_view(&self) -> Option<Entity<TerminalView>> {
        self.terminal
            .session
            .as_ref()
            .map(|session| session.view().clone())
    }

    pub(super) fn terminal_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.terminal
            .session
            .as_ref()
            .map(|session| session.view().read(cx).focus_handle().clone())
    }

    /// The theme's colours and monospace face as the terminal's config. The
    /// session keeps its own grid size; only colours and the font change.
    pub(super) fn refresh_terminal_theme(&self, cx: &mut App) {
        let Some(session) = self.terminal.session.as_ref() else {
            return;
        };
        let theme = cx.global::<Theme>();
        let mut config = session.config(cx).clone();
        config.font_family = theme.mono_font.to_string();
        config.colors = terminal_colors(theme);
        session.update_config(config, cx);
    }
}

/// The terminal's colours, from the same tokens the rest of the app paints
/// with. Bright colours reuse the normal ones; text tokens cover white/black.
pub(super) fn terminal_colors(theme: &Theme) -> ColorPalette {
    let to_rgb = |c: Hsla| {
        let c = gpui::Rgba::from(c);
        let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        (b(c.r), b(c.g), b(c.b))
    };
    let t = &theme.tokens;
    let (bg_r, bg_g, bg_b) = to_rgb(t.background_primary);
    let (fg_r, fg_g, fg_b) = to_rgb(t.text_normal);
    let (muted_r, muted_g, muted_b) = to_rgb(t.text_muted);
    let (red_r, red_g, red_b) = to_rgb(t.color_red);
    let (green_r, green_g, green_b) = to_rgb(t.color_green);
    let (yellow_r, yellow_g, yellow_b) = to_rgb(t.color_yellow);
    let (blue_r, blue_g, blue_b) = to_rgb(t.color_blue);
    let (magenta_r, magenta_g, magenta_b) = to_rgb(t.color_purple);
    let (cyan_r, cyan_g, cyan_b) = to_rgb(t.color_cyan);
    let (cursor_r, cursor_g, cursor_b) = to_rgb(t.interactive_accent);
    ColorPalette::builder()
        .background(bg_r, bg_g, bg_b)
        .foreground(fg_r, fg_g, fg_b)
        .cursor(cursor_r, cursor_g, cursor_b)
        .black(bg_r, bg_g, bg_b)
        .red(red_r, red_g, red_b)
        .green(green_r, green_g, green_b)
        .yellow(yellow_r, yellow_g, yellow_b)
        .blue(blue_r, blue_g, blue_b)
        .magenta(magenta_r, magenta_g, magenta_b)
        .cyan(cyan_r, cyan_g, cyan_b)
        .white(muted_r, muted_g, muted_b)
        .bright_black(muted_r, muted_g, muted_b)
        .bright_red(red_r, red_g, red_b)
        .bright_green(green_r, green_g, green_b)
        .bright_yellow(yellow_r, yellow_g, yellow_b)
        .bright_blue(blue_r, blue_g, blue_b)
        .bright_magenta(magenta_r, magenta_g, magenta_b)
        .bright_cyan(cyan_r, cyan_g, cyan_b)
        .bright_white(fg_r, fg_g, fg_b)
        .build()
}

fn terminal_config(theme: &Theme) -> TerminalConfig {
    TerminalConfig {
        cols: 80,
        rows: 24,
        font_family: theme.mono_font.to_string(),
        font_size: px(theme.base_font_size),
        scrollback: 10_000,
        line_height_multiplier: 1.2,
        padding: Edges::all(px(8.)),
        colors: terminal_colors(theme),
    }
}
