//! The one place that turns "Mod" into a keystroke, and the platform chords
//! that sit beside it.
//!
//! Policy: **Mod is the platform's primary modifier** — Cmd on macOS, Ctrl on
//! Windows and Linux. That is the `Mod` of `hotkeys.json` and `commands.txt`,
//! and GPUI spells it `secondary-` in a native build.
//!
//! A wasm build cannot know the browser's operating system: it is not macOS
//! to `cfg!`, so GPUI resolves `secondary` to Ctrl even in a Mac browser.
//! The browser build therefore binds the Cmd spelling as well, and keeps both
//! sides of the platform-specific chords below.
//!
//! Literal `Ctrl` appears only where Obsidian itself uses Ctrl on every
//! platform (Ctrl+Tab and Ctrl+PageUp/PageDown for tabs, Ctrl+N/Ctrl+P in the
//! suggestion popup) — those are bound at their call sites, not here.

use gpui::{Action, App, KeyBinding};

/// Binds `key` as Mod+key: {Cmd, Ctrl}+key on native, both spellings on wasm.
pub fn bind_mod<T: Action + Clone>(cx: &mut App, ctx: Option<&str>, key: &str, action: T) {
    bind(cx, ctx, "secondary", key, &action);
    #[cfg(target_arch = "wasm32")]
    bind(cx, ctx, "cmd", key, &action);
}

/// Binds `key` with the platform's word-movement modifier: Option on macOS,
/// Ctrl elsewhere. The browser build binds Option and Ctrl both, since either
/// host may be behind it.
pub fn bind_word<T: Action + Clone>(cx: &mut App, ctx: Option<&str>, key: &str, action: T) {
    #[cfg(any(target_os = "macos", target_arch = "wasm32"))]
    bind(cx, ctx, "alt", key, &action);
    #[cfg(not(target_os = "macos"))]
    bind(cx, ctx, "ctrl", key, &action);
}

fn bind<T: Action + Clone>(cx: &mut App, ctx: Option<&str>, modifier: &str, key: &str, action: &T) {
    let keystroke = format!("{modifier}-{key}");
    cx.bind_keys([KeyBinding::new(&keystroke, action.clone(), ctx)]);
}
