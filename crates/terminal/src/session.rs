//! A shell on a PTY, with the GPUI view that paints and types into it.
//!
//! The terminal engine expects a [`std::io::Read`]/[`std::io::Write`] pair;
//! this module owns the process that feeds them: it opens a PTY, starts the
//! user's shell in the vault, keeps the master for resizes, and kills the
//! child when the session is dropped.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result};
use gpui::{App, AppContext, Entity, EntityInputHandler, Window};
use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::view::{TerminalConfig, TerminalView};

/// The size a PTY starts at before the first frame measures the pane.
const INITIAL_COLS: usize = 80;
const INITIAL_ROWS: usize = 24;

/// One running shell and its view. The view is a GPUI entity; the session
/// keeps the PTY and the child alive for as long as the workspace holds it.
pub struct TerminalSession {
    view: Entity<TerminalView>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
}

impl TerminalSession {
    /// Starts `shell` (the user's when `None`) in `cwd` and builds its view.
    pub fn spawn(
        cwd: &Path,
        shell: Option<&str>,
        config: TerminalConfig,
        cx: &mut App,
    ) -> Result<TerminalSession> {
        let shell = shell
            .map(str::to_string)
            .or_else(|| std::env::var("SHELL").ok())
            .or_else(|| std::env::var("COMSPEC").ok())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "cmd.exe".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: INITIAL_ROWS as u16,
                cols: INITIAL_COLS as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("could not open a PTY")?;

        let mut command = CommandBuilder::new(&shell);
        if cwd.is_dir() {
            command.cwd(cwd);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "tft");
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("could not start {shell}"))?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .context("could not take the PTY writer")?;
        let reader = pair
            .master
            .try_clone_reader()
            .context("could not clone the PTY reader")?;
        let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));

        let master_for_resize = master.clone();
        let exited = Arc::new(AtomicBool::new(false));
        let exit_flag = exited.clone();
        let view = cx.new(|cx| {
            TerminalView::new(writer, reader, config, cx)
                .with_resize_callback(move |cols, rows| {
                    let _ = master_for_resize.lock().resize(PtySize {
                        rows: rows as u16,
                        cols: cols as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                })
                .with_exit_callback(move |_, _| {
                    exit_flag.store(true, Ordering::SeqCst);
                })
        });

        Ok(TerminalSession {
            view,
            master,
            child,
            exited,
        })
    }

    pub fn view(&self) -> &Entity<TerminalView> {
        &self.view
    }

    /// The shell process has ended; its output stays in the grid.
    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// A colour scheme and font the terminal was started with.
    pub fn config<'a>(&self, cx: &'a App) -> &'a TerminalConfig {
        self.view.read(cx).config()
    }

    /// Replaces the running shell's colours and font without restarting it.
    pub fn update_config(&self, config: TerminalConfig, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.update_config(config, cx));
    }

    /// Types text into the running shell through the same input handler an
    /// IME commits through; the control socket's `type` uses this path.
    pub fn write(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.replace_text_in_range(None, text, window, cx);
        });
    }

    /// Sends a resize when the pane's measured size changes outside paint.
    pub fn resize(&self, cols: usize, rows: usize) {
        let _ = self.master.lock().resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
