//! The terminal pane: spawning the shell, painting its view, and the tab that holds it.

use std::collections::HashMap;

use gpui::{px, App, Context, Edges, Entity, EntityId, FocusHandle, Hsla, Subscription, Window};
use serde_json::{json, Value};
use tft_app::{place_new_tab, Tab};
use tft_terminal::{
    ColorPalette, TerminalConfig, TerminalLink, TerminalSession, TerminalTitle, TerminalView,
};

use super::{ViewMode, Workspace};
use crate::theme::Theme;

/// Every terminal tab's shell, keyed by the id stored in the tab's
/// `viewState`. Tabs own their sessions: a new terminal tab starts a new
/// shell, and closing it ends that shell.
pub(super) struct TerminalPaneState {
    sessions: HashMap<u64, TerminalSession>,
    next_id: u64,
    /// Keeps each session's title and link subscriptions alive; removing
    /// the session ends them with it.
    subscriptions: HashMap<u64, Vec<Subscription>>,
}

impl Default for TerminalPaneState {
    fn default() -> Self {
        TerminalPaneState {
            sessions: HashMap::new(),
            next_id: 1,
            subscriptions: HashMap::new(),
        }
    }
}

impl TerminalPaneState {
    fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn session(&self, id: u64) -> Option<&TerminalSession> {
        self.sessions.get(&id)
    }

    pub(super) fn view(&self, id: u64) -> Option<Entity<TerminalView>> {
        self.session(id).map(|session| session.view().clone())
    }

    pub(super) fn focus(&self, id: u64, cx: &App) -> Option<FocusHandle> {
        self.session(id)
            .map(|session| session.view().read(cx).focus_handle().clone())
    }

    /// Whether the keyboard sits on any terminal view, even one whose tab is
    /// no longer the painted one.
    pub(super) fn any_focus(&self, window: &Window, cx: &App) -> bool {
        self.sessions
            .values()
            .any(|session| session.view().read(cx).focus_handle().is_focused(window))
    }

    /// A fresh session id for a terminal tab copied into a split or popout.
    pub(super) fn new_view_state(&mut self) -> Value {
        let id = self.allocate();
        json!({ "terminal": id })
    }

    fn close(&mut self, id: u64) {
        // Dropping the session kills its shell; its subscriptions end with
        // the view they point at.
        self.sessions.remove(&id);
        self.subscriptions.remove(&id);
    }

    /// Ends every shell and forgets every subscription; a vault switch starts
    /// from no terminals.
    pub(super) fn close_all(&mut self) {
        self.sessions.clear();
        self.subscriptions.clear();
    }
}

/// The session id a terminal tab carries in its `viewState`.
pub(super) fn terminal_id(tab: &Tab) -> Option<u64> {
    tab.view_state
        .as_ref()
        .and_then(|value| value.get("terminal"))
        .and_then(Value::as_u64)
}

fn set_terminal_id(tab: &mut Tab, id: u64) {
    let mut state = tab
        .view_state
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    state.insert("terminal".into(), json!(id));
    tab.view_state = Some(Value::Object(state));
}

fn set_tab_title(tab: &mut Tab, title: &str) {
    let mut state = tab
        .view_state
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if title.is_empty() {
        state.shift_remove("title");
    } else {
        state.insert("title".into(), json!(title));
    }
    tab.view_state = Some(Value::Object(state));
}

fn tab_title_locked(tab: &Tab) -> bool {
    tab.view_state
        .as_ref()
        .and_then(|value| value.get("titleLocked"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// A name the user typed overrides the shell's own OSC 0/2 title until they
/// clear it, so a command that retitles its window cannot rename their tab.
fn set_tab_title_locked(tab: &mut Tab, locked: bool) {
    let mut state = tab
        .view_state
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if locked {
        state.insert("titleLocked".into(), json!(true));
    } else {
        state.shift_remove("titleLocked");
    }
    tab.view_state = Some(Value::Object(state));
}

impl Workspace {
    /// `terminal:open`: a new terminal tab with its own shell. Existing
    /// terminal tabs keep their sessions; invoke this again for another one.
    pub fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_preview(cx);
        let id = self.terminal.allocate();
        self.flush_save(cx);
        self.active_group_state.stash_active_tab();
        let mut tab = Tab::empty(ViewMode::Terminal);
        set_terminal_id(&mut tab, id);
        self.active_group_state.active_tab = place_new_tab(
            &mut self.active_group_state.tabs,
            self.active_group_state.active_tab,
            tab,
        );
        // `load_tab` starts the shell for the id the tab carries.
        self.load_tab(cx);
        self.focus_main(window, cx);
        cx.notify();
    }

    /// `terminal:toggle`: covers and reveals the terminal pane. Our terminal
    /// is a tab, so hiding it selects another tab in the group without
    /// ending the shell; invoking it again opens a terminal tab.
    pub fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_group_state.mode == ViewMode::Terminal {
            if let Some(ix) = self
                .active_group_state
                .tabs
                .iter()
                .position(|tab| tab.mode != ViewMode::Terminal)
            {
                self.activate_tab(ix, cx);
            }
            return;
        }
        self.open_terminal(window, cx);
    }

    /// `terminal:close`: ends the active terminal's shell and closes its tab.
    pub fn close_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_group_state.mode != ViewMode::Terminal {
            return;
        }
        self.close_tab(self.active_group_state.active_tab, cx);
        self.focus_main(window, cx);
    }

    /// `terminal:rename`: asks for a name for the active terminal's tab.
    pub fn start_rename_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_group_state.mode != ViewMode::Terminal {
            return;
        }
        let Some(tab) = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)
        else {
            return;
        };
        let current = tab
            .view_state
            .as_ref()
            .and_then(|value| value.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                terminal_id(tab)
                    .and_then(|id| self.terminal.session(id))
                    .and_then(|session| session.title())
            })
            .unwrap_or_else(|| "Terminal".to_string());
        self.open_prompt(
            super::PendingOp::RenameTerminal,
            crate::prompt::PromptSpec {
                title: "Rename terminal".into(),
                input: Some(current),
                confirm: "Rename".into(),
                ..crate::prompt::PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// The prompt's answer for `terminal:rename`: every tab holding this
    /// session takes the name; an empty name hands the tab back to the
    /// shell's title.
    pub(super) fn rename_terminal_to(&mut self, title: &str, cx: &mut Context<Self>) {
        let Some(tab) = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)
        else {
            return;
        };
        let Some(id) = terminal_id(tab) else {
            return;
        };
        let locked = !title.is_empty();
        let apply = |tabs: &mut Vec<Tab>| {
            for tab in tabs.iter_mut() {
                if terminal_id(tab) == Some(id) {
                    set_tab_title(tab, title);
                    set_tab_title_locked(tab, locked);
                }
            }
        };
        apply(&mut self.active_group_state.tabs);
        for group in self.groups.iter_mut() {
            apply(&mut group.state.tabs);
        }
        cx.notify();
    }

    /// Starts the active terminal tab's shell when it has none, or when the
    /// last one exited.
    pub(super) fn ensure_terminal_for_active(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)
        else {
            return;
        };
        let id = match terminal_id(tab) {
            Some(id) => id,
            None => {
                let id = self.terminal.allocate();
                set_terminal_id(
                    &mut self.active_group_state.tabs[self.active_group_state.active_tab],
                    id,
                );
                id
            }
        };
        self.ensure_terminal(id, cx);
    }

    pub(super) fn ensure_terminal(&mut self, id: u64, cx: &mut Context<Self>) {
        if self
            .terminal
            .session(id)
            .is_some_and(|session| !session.has_exited())
        {
            return;
        }
        // Dropping an exited session kills its child and closes the PTY.
        self.terminal.close(id);
        let theme = cx.global::<Theme>().clone();
        let config = terminal_config(&theme);
        let cwd = self
            .vault_root()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        match TerminalSession::spawn(&cwd, None, config, cx) {
            Ok(session) => {
                let view = session.view().clone();
                self.terminal.sessions.insert(id, session);
                let subscription = cx.subscribe(&view, |this, view, event: &TerminalTitle, cx| {
                    this.apply_terminal_title(view.entity_id(), &event.0, cx);
                });
                let link_subscription = cx.subscribe(&view, |this, _, event: &TerminalLink, cx| {
                    this.open_terminal_link(event.target.clone(), event.cwd.clone(), cx);
                });
                self.terminal
                    .subscriptions
                    .entry(id)
                    .or_default()
                    .extend([subscription, link_subscription]);
            }
            Err(err) => self.report(format!("Could not open the terminal: {err}"), cx),
        }
    }

    pub(super) fn active_terminal_session(&self) -> Option<&TerminalSession> {
        let tab = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)?;
        self.terminal.session(terminal_id(tab)?)
    }

    pub(super) fn terminal_focus(&self, cx: &App) -> Option<FocusHandle> {
        let tab = self
            .active_group_state
            .tabs
            .get(self.active_group_state.active_tab)?;
        self.terminal.focus(terminal_id(tab)?, cx)
    }

    /// The theme's colours and monospace face as the terminal's config. Each
    /// session keeps its own grid size; only colours and the font change.
    pub(super) fn refresh_terminal_theme(&self, cx: &mut App) {
        let theme = cx.global::<Theme>().clone();
        let colors = terminal_colors(&theme);
        for session in self.terminal.sessions.values() {
            let mut config = session.config(cx).clone();
            config.font_family = theme.mono_font.to_string();
            config.colors = colors.clone();
            session.update_config(config, cx);
        }
    }

    /// Kills the shells of terminal tabs that are being closed.
    pub(super) fn close_terminal_tabs<'a>(&mut self, tabs: impl Iterator<Item = &'a Tab>) {
        let ids: Vec<u64> = tabs.filter_map(terminal_id).collect();
        for id in ids {
            self.terminal.close(id);
        }
    }

    /// A link or path clicked in the terminal: URLs go to the browser, paths
    /// open the note in TFT or the file in the desktop's default app. A
    /// relative path resolves against the shell's OSC 7 working directory
    /// first, then the vault root.
    fn open_terminal_link(
        &mut self,
        target: String,
        cwd: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let target = target
            .strip_prefix("file://")
            .unwrap_or(&target)
            .to_string();
        let external = ["http://", "https://", "mailto:", "obsidian://"];
        if external.iter().any(|prefix| target.starts_with(prefix)) {
            cx.open_url(&target);
            return;
        }
        let path = std::path::Path::new(&target);
        let relative = !path.is_absolute() && !target.starts_with("~/");
        if relative {
            // Where the command ran comes first: `cat docs/Note.md` is the
            // shell's docs, not necessarily the vault's.
            if let Some(cwd) = cwd.as_deref() {
                let joined = cwd.join(path);
                if self.open_terminal_path(&joined, cx) {
                    return;
                }
            }
            if self.open_by_name(&target, cx) {
                return;
            }
            if let Some(path) = self.vault_file_path(&target) {
                self.open_file(path, None, cx);
                return;
            }
        }
        if path.is_absolute() {
            if let Some(root) = self.vault_root() {
                if let Ok(relative) = path.strip_prefix(&root) {
                    if let Some(path) = self.vault_file_path(&relative.to_string_lossy()) {
                        self.open_file(path, None, cx);
                        return;
                    }
                }
            }
            if self.open_terminal_path(path, cx) {
                return;
            }
        }
        let path = if let Some(rest) = target.strip_prefix("~/") {
            std::env::home_dir().map(|home| home.join(rest))
        } else {
            Some(std::path::PathBuf::from(&target))
        };
        if let Some(path) = path.filter(|path| path.exists()) {
            self.open_externally(&path, cx);
            return;
        }
        self.report(format!("Could not open {target}"), cx);
    }

    /// Opens an absolute path inside the vault as a note; outside it, with
    /// the desktop's default app.
    fn open_terminal_path(&mut self, path: &std::path::Path, cx: &mut Context<Self>) -> bool {
        if let Some(root) = self.vault_root() {
            if let Ok(relative) = path.strip_prefix(&root) {
                if let Some(note) = self.vault_file_path(&relative.to_string_lossy()) {
                    self.open_file(note, None, cx);
                    return true;
                }
            }
        }
        if path.exists() {
            self.open_externally(path, cx);
            return true;
        }
        false
    }

    fn open_externally(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let mut command = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else if cfg!(target_os = "windows") {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        } else {
            std::process::Command::new("xdg-open")
        };
        if let Err(err) = command
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            self.report(format!("Could not open {}: {err}", path.display()), cx);
        }
    }

    /// An OSC 0/2 title from a terminal: write it into every tab holding that
    /// session, so the strip, view header and window title all follow it.
    fn apply_terminal_title(&mut self, view: EntityId, title: &str, cx: &mut Context<Self>) {
        let Some(id) = self
            .terminal
            .sessions
            .iter()
            .find(|(_, session)| session.view().entity_id() == view)
            .map(|(id, _)| *id)
        else {
            return;
        };
        for tab in self.active_group_state.tabs.iter_mut() {
            if terminal_id(tab) == Some(id) && !tab_title_locked(tab) {
                set_tab_title(tab, title);
            }
        }
        for group in self.groups.iter_mut() {
            for tab in group.state.tabs.iter_mut() {
                if terminal_id(tab) == Some(id) && !tab_title_locked(tab) {
                    set_tab_title(tab, title);
                }
            }
        }
        cx.notify();
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
        shell: None,
        env: Vec::new(),
        login_shell: true,
        cursor_blink: true,
        copy_on_select: false,
        option_as_meta: true,
        bell: true,
    }
}
