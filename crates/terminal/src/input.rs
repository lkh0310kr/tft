//! Keyboard input handling for the terminal emulator.
//!
//! This module provides [`keystroke_to_bytes`], which converts GPUI keyboard
//! events into terminal escape sequences that can be written to the PTY.
//!
//! # Key Mappings
//!
//! ## Special Keys
//!
//! | Key | Sequence | Notes |
//! |-----|----------|-------|
//! | Enter | `\r` (0x0D) | Carriage return |
//! | Escape | `\x1b` (0x1B) | ESC |
//! | Backspace | `\x7f` (0x7F) | DEL |
//! | Tab | `\t` (0x09) | Horizontal tab |
//! | Shift+Tab | `\x1b[Z` | Backtab |
//! | Space | ` ` (0x20) | Space |
//! | Ctrl+Space | `\x00` | NUL |
//!
//! ## Arrow Keys
//!
//! Arrow key sequences depend on application cursor mode:
//!
//! | Key | Normal Mode | App Cursor Mode |
//! |-----|-------------|-----------------|
//! | Up | `\x1b[A` | `\x1bOA` |
//! | Down | `\x1b[B` | `\x1bOB` |
//! | Right | `\x1b[C` | `\x1bOC` |
//! | Left | `\x1b[D` | `\x1bOD` |
//!
//! ## Navigation Keys
//!
//! | Key | Sequence |
//! |-----|----------|
//! | Home | `\x1b[H` |
//! | End | `\x1b[F` |
//! | PageUp | `\x1b[5~` |
//! | PageDown | `\x1b[6~` |
//! | Insert | `\x1b[2~` |
//! | Delete | `\x1b[3~` |
//!
//! ## Function Keys
//!
//! | Key | Sequence |
//! |-----|----------|
//! | F1-F4 | `\x1bOP` - `\x1bOS` |
//! | F5-F12 | `\x1b[15~` - `\x1b[24~` |
//!
//! ## Control Combinations
//!
//! Ctrl+A through Ctrl+Z map to ASCII control characters 0x01-0x1A:
//!
//! | Combination | Byte |
//! |-------------|------|
//! | Ctrl+A | 0x01 |
//! | Ctrl+C | 0x03 (interrupt) |
//! | Ctrl+D | 0x04 (EOF) |
//! | Ctrl+Z | 0x1A (suspend) |
//!
//! ## Alt Combinations
//!
//! Alt+key sends ESC followed by the key: `\x1b` + key
//!
//! # Terminal Mode Effects
//!
//! The [`TermMode`] flags affect key sequences:
//!
//! - **APP_CURSOR**: Changes arrow key sequences from CSI to SS3 format
//!
//! # Example
//!
//! ```
//! use gpui::Keystroke;
//! use alacritty_terminal::term::TermMode;
//! use gpui_terminal::input::keystroke_to_bytes;
//!
//! // Enter key
//! let keystroke = Keystroke::parse("enter").unwrap();
//! assert_eq!(keystroke_to_bytes(&keystroke, TermMode::empty()), Some(b"\r".to_vec()));
//!
//! // Ctrl+C (interrupt)
//! let keystroke = Keystroke::parse("ctrl-c").unwrap();
//! assert_eq!(keystroke_to_bytes(&keystroke, TermMode::empty()), Some(vec![0x03]));
//! ```

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

/// What the platform did to the key: pressed it, held it down (auto-repeat),
/// or let it go. Kitty's progressive keyboard protocol reports all three when
/// the application asked for event types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

impl KeyEventKind {
    fn kitty_code(self) -> u8 {
        match self {
            KeyEventKind::Press => 1,
            KeyEventKind::Repeat => 2,
            KeyEventKind::Release => 3,
        }
    }
}

/// Convert a GPUI keystroke to terminal escape sequence bytes.
///
/// This function translates GPUI keyboard events into the appropriate byte sequences
/// expected by terminal applications. It handles special keys, control characters,
/// and application cursor mode.
///
/// # Arguments
///
/// * `keystroke` - The GPUI keystroke to convert
/// * `mode` - The current terminal mode (affects arrow key sequences)
///
/// # Returns
///
/// An optional vector of bytes representing the terminal escape sequence.
/// Returns `None` if the keystroke should not produce any output.
///
/// # Examples
///
/// ```
/// use gpui::Keystroke;
/// use alacritty_terminal::term::TermMode;
/// use gpui_terminal::input::keystroke_to_bytes;
///
/// let keystroke = Keystroke::parse("enter").unwrap();
/// let bytes = keystroke_to_bytes(&keystroke, TermMode::empty());
/// assert_eq!(bytes, Some(b"\r".to_vec()));
/// ```
pub fn keystroke_to_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    keystroke_to_bytes_for_event(keystroke, mode, KeyEventKind::Press)
}

/// [`keystroke_to_bytes`] that also says whether the key was pressed,
/// repeated, or released. Only the kitty protocol distinguishes them; a
/// release in a legacy application produces nothing.
pub fn keystroke_to_bytes_for_event(
    keystroke: &Keystroke,
    mode: TermMode,
    kind: KeyEventKind,
) -> Option<Vec<u8>> {
    if keystroke.modifiers.control
        && !keystroke.modifiers.alt
        && matches!(keystroke.key.as_str(), "tab" | "pageup" | "pagedown")
    {
        return None;
    }
    if let Some(bytes) = kitty_key(keystroke, mode, kind) {
        // An empty vector means the protocol consumed the event without
        // producing a report (a release without event types, or printable
        // text that takes the input handler's commit).
        return Some(bytes);
    }
    if kind == KeyEventKind::Release {
        return None;
    }
    if let Some(bytes) = modified_key(keystroke, mode) {
        return Some(bytes);
    }
    // Cmd on macOS, the Windows key and Super are the application's:
    // Cmd-W closes the tab, Cmd-C copies, and neither types into the shell.
    if keystroke.modifiers.platform {
        return None;
    }

    // Handle special keys first
    match keystroke.key.as_str() {
        // Basic control characters
        "space" => {
            if keystroke.modifiers.control {
                return Some(b"\x00".to_vec()); // Ctrl+Space = NUL
            }
            // Plain space is text and reaches the shell through the input
            // handler, as every other printable character does.
        }
        "enter" => return Some(b"\r".to_vec()),
        "escape" => return Some(b"\x1b".to_vec()),
        "backspace" => return Some(b"\x7f".to_vec()),
        "tab" => {
            if keystroke.modifiers.control {
                return None;
            }
            // Shift+Tab sends a different sequence
            if keystroke.modifiers.shift {
                return Some(b"\x1b[Z".to_vec());
            }
            return Some(b"\t".to_vec());
        }

        // Arrow keys - check APP_CURSOR mode
        "up" => {
            if mode.contains(TermMode::APP_CURSOR) {
                return Some(b"\x1bOA".to_vec());
            }
            return Some(b"\x1b[A".to_vec());
        }
        "down" => {
            if mode.contains(TermMode::APP_CURSOR) {
                return Some(b"\x1bOB".to_vec());
            }
            return Some(b"\x1b[B".to_vec());
        }
        "right" => {
            if mode.contains(TermMode::APP_CURSOR) {
                return Some(b"\x1bOC".to_vec());
            }
            return Some(b"\x1b[C".to_vec());
        }
        "left" => {
            if mode.contains(TermMode::APP_CURSOR) {
                return Some(b"\x1bOD".to_vec());
            }
            return Some(b"\x1b[D".to_vec());
        }

        // Navigation keys
        "home" => {
            if mode.contains(TermMode::APP_CURSOR) {
                return Some(b"\x1bOH".to_vec());
            }
            return Some(b"\x1b[H".to_vec());
        }
        "end" => {
            if mode.contains(TermMode::APP_CURSOR) {
                return Some(b"\x1bOF".to_vec());
            }
            return Some(b"\x1b[F".to_vec());
        }
        "pageup" => return Some(b"\x1b[5~".to_vec()),
        "pagedown" => return Some(b"\x1b[6~".to_vec()),
        "insert" => return Some(b"\x1b[2~".to_vec()),
        "delete" => return Some(b"\x1b[3~".to_vec()),

        // Function keys
        "f1" => return Some(b"\x1bOP".to_vec()),
        "f2" => return Some(b"\x1bOQ".to_vec()),
        "f3" => return Some(b"\x1bOR".to_vec()),
        "f4" => return Some(b"\x1bOS".to_vec()),
        "f5" => return Some(b"\x1b[15~".to_vec()),
        "f6" => return Some(b"\x1b[17~".to_vec()),
        "f7" => return Some(b"\x1b[18~".to_vec()),
        "f8" => return Some(b"\x1b[19~".to_vec()),
        "f9" => return Some(b"\x1b[20~".to_vec()),
        "f10" => return Some(b"\x1b[21~".to_vec()),
        "f11" => return Some(b"\x1b[23~".to_vec()),
        "f12" => return Some(b"\x1b[24~".to_vec()),

        _ => {}
    }

    // Handle Ctrl+key combinations
    if keystroke.modifiers.control {
        let key = keystroke.key.as_str();

        // Ctrl+A through Ctrl+Z map to 0x01 through 0x1a
        if key.len() == 1 {
            let ch = key.chars().next().unwrap();
            if ch.is_ascii_alphabetic() {
                // Convert to uppercase and then to control character
                let upper = ch.to_ascii_uppercase();
                let ctrl_char = (upper as u8) - b'@';
                return Some(vec![ctrl_char]);
            }

            // Special Ctrl combinations
            match ch {
                '[' => return Some(b"\x1b".to_vec()),  // Ctrl+[
                '\\' => return Some(b"\x1c".to_vec()), // Ctrl+\
                ']' => return Some(b"\x1d".to_vec()),  // Ctrl+]
                '^' => return Some(b"\x1e".to_vec()),  // Ctrl+^
                '_' => return Some(b"\x1f".to_vec()),  // Ctrl+_
                '?' => return Some(b"\x7f".to_vec()),  // Ctrl+?
                _ => {}
            }
        }
    }

    // Handle Alt+key combinations
    if keystroke.modifiers.alt {
        let key = keystroke.key.as_str();
        if key.len() == 1 {
            // Alt+key sends ESC followed by the key
            let ch = key.chars().next().unwrap();
            if ch.is_ascii() {
                let mut bytes = vec![b'\x1b'];
                bytes.push(ch as u8);
                return Some(bytes);
            }
        }
    }

    // Printable text — letters, digits, punctuation, space, and IME
    // commits — reaches the shell through `EntityInputHandler`. gpui sends
    // input-context text there after keybindings; sending it here as well
    // would duplicate it, and a composing jamo would arrive before the
    // syllable the input method commits.
    // If we get here, the keystroke doesn't produce any output
    None
}

/// Kitty keyboard protocol (and its disambiguate mode): the application
/// opted into unambiguous key reports, so Escape/Enter/Tab/modified keys are
/// sent as `CSI codepoint ; modifier u` instead of their legacy bytes.
///
/// The optional progressive flags are honoured too: alternative keys
/// (`codepoint:shifted`), associated text (`;text`), all keys as escape
/// sequences, and press/repeat/release event types.
fn kitty_key(keystroke: &Keystroke, mode: TermMode, kind: KeyEventKind) -> Option<Vec<u8>> {
    if !mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
        return None;
    }
    let report_events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    // Without event types a release is not part of the protocol: consume it
    // rather than letting the legacy table type a second character.
    if kind == KeyEventKind::Release && !report_events {
        return Some(Vec::new());
    }
    let m = &keystroke.modifiers;
    let key = keystroke.key.as_str();
    let codepoint = match key {
        "escape" => 27,
        "enter" => 13,
        "tab" => 9,
        "backspace" => 127,
        "space" => 32,
        "up" => 57352,
        "down" => 57353,
        "left" => 57354,
        "right" => 57355,
        "home" => 57356,
        "end" => 57357,
        "pageup" => 57358,
        "pagedown" => 57359,
        "insert" => 57360,
        "delete" => 57361,
        "f1" => 57364,
        "f2" => 57365,
        "f3" => 57366,
        "f4" => 57367,
        "f5" => 57368,
        "f6" => 57369,
        "f7" => 57370,
        "f8" => 57371,
        "f9" => 57372,
        "f10" => 57373,
        "f11" => 57374,
        "f12" => 57375,
        other if other.chars().count() == 1 => other.chars().next()? as u32,
        _ => return None,
    };
    let printable = key.chars().count() == 1 || key == "space";
    // A shifted letter normally reaches the shell through the input
    // handler as text; with alternate-key reporting the key itself becomes
    // a protocol report instead.
    let shift_report = m.shift && mode.contains(TermMode::REPORT_ALTERNATE_KEYS);
    let plain_text = printable && !m.control && !m.alt && !m.platform && !shift_report;
    if plain_text && !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
        // Plain printable text still goes through the input handler's
        // commit, unless the application asked for every key as an escape
        // sequence. Modified printables are protocol reports.
        return None;
    }
    let modifiers = 1
        + u32::from(m.shift)
        + (u32::from(m.alt) << 1)
        + (u32::from(m.control) << 2)
        + (u32::from(m.platform) << 3);
    // Alternate key: the shifted codepoint, reported after a colon. Kitty
    // defines it only for keys whose shifted form is a different character.
    let alternate = mode
        .contains(TermMode::REPORT_ALTERNATE_KEYS)
        .then(|| shifted_codepoint(keystroke, codepoint))
        .flatten();
    let key_field = match alternate {
        Some(shifted) => format!("{codepoint}:{shifted}"),
        None => codepoint.to_string(),
    };
    let event = if report_events {
        format!(":{}", kind.kitty_code())
    } else {
        String::new()
    };
    // Associated text: the codepoints the key produced, colon-separated.
    let associated = mode
        .contains(TermMode::REPORT_ASSOCIATED_TEXT)
        .then(|| associated_text(keystroke, printable))
        .flatten();
    let seq = if modifiers == 1 && event.is_empty() && associated.is_none() {
        format!("\x1b[{key_field}u")
    } else if let Some(text) = associated {
        format!("\x1b[{key_field};{modifiers}{event};{text}u")
    } else {
        format!("\x1b[{key_field};{modifiers}{event}u")
    };
    Some(seq.into_bytes())
}

/// The shifted key's codepoint for `REPORT_ALTERNATE_KEYS`, when it differs
/// from the unshifted one.
fn shifted_codepoint(keystroke: &Keystroke, codepoint: u32) -> Option<u32> {
    if !keystroke.modifiers.shift {
        return None;
    }
    let shifted = keystroke
        .key_char
        .as_deref()
        .filter(|text| text.chars().count() == 1)
        .map(|text| text.chars().next().unwrap() as u32)?;
    (shifted != codepoint).then_some(shifted)
}

/// The text a key produced, for `REPORT_ASSOCIATED_TEXT`: one codepoint per
/// colon-separated field, as kitty defines it.
fn associated_text(keystroke: &Keystroke, printable: bool) -> Option<String> {
    if !printable {
        return None;
    }
    let text = keystroke
        .key_char
        .as_deref()
        .filter(|text| !text.is_empty())?;
    Some(
        text.chars()
            .map(|c| (c as u32).to_string())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// Keys a terminal sends that the plain table does not cover: Cmd's
/// conveniences (line start/end, kill line), Option's word motion, and the
/// xterm `CSI 1 ; <modifier>` forms for Shift/Ctrl/Alt-modified keys.
fn modified_key(keystroke: &Keystroke, _mode: TermMode) -> Option<Vec<u8>> {
    let key = keystroke.key.as_str();
    let m = &keystroke.modifiers;

    if m.platform {
        // Cmd with Control or Option belongs to the application (TFT's
        // Mod+Alt+Left/Right back and forward), not the shell.
        if m.control || m.alt {
            return None;
        }
        return match key {
            // Terminal.app/Zed conventions: Cmd-Left/Right are the shell's
            // line start/end, Cmd-Backspace clears the line, Cmd-Delete
            // kills to the end, Cmd-K clears the screen.
            "left" => Some(vec![0x01]),
            "right" => Some(vec![0x05]),
            "backspace" => Some(vec![0x15]),
            "delete" => Some(vec![0x0b]),
            "k" => Some(vec![0x0c]),
            _ => None,
        };
    }

    // Option is Meta on a terminal: word motion and word deletion.
    if m.alt && !m.control && !m.shift {
        match key {
            "left" => return Some(b"\x1bb".to_vec()),
            "right" => return Some(b"\x1bf".to_vec()),
            "backspace" => return Some(b"\x1b\x7f".to_vec()),
            "delete" => return Some(b"\x1bd".to_vec()),
            "enter" => return Some(b"\x1b\r".to_vec()),
            _ => {}
        }
    }
    if m.shift && !m.control && !m.alt && key == "enter" {
        return Some(vec![0x0a]);
    }
    // Ctrl-Backspace deletes the previous word, as Zed binds it.
    if m.control && !m.alt && key == "backspace" {
        return Some(vec![0x17]);
    }
    // Ctrl+Tab and Ctrl+PageUp/PageDown are TFT's tab shortcuts: leave them
    // to the workspace rather than typing a control character into the shell.
    if m.control && !m.alt && matches!(key, "tab" | "pageup" | "pagedown") {
        return None;
    }

    let modified = m.shift || m.alt || m.control;
    if !modified {
        return None;
    }
    let code = 1 + u32::from(m.shift) + (u32::from(m.alt) << 1) + (u32::from(m.control) << 2);
    let seq = match key {
        "up" => format!("\x1b[1;{code}A"),
        "down" => format!("\x1b[1;{code}B"),
        "right" => format!("\x1b[1;{code}C"),
        "left" => format!("\x1b[1;{code}D"),
        "home" => format!("\x1b[1;{code}H"),
        "end" => format!("\x1b[1;{code}F"),
        "insert" => format!("\x1b[2;{code}~"),
        "delete" => format!("\x1b[3;{code}~"),
        "pageup" => format!("\x1b[5;{code}~"),
        "pagedown" => format!("\x1b[6;{code}~"),
        _ => return None,
    };
    Some(seq.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enter_key() {
        let keystroke = Keystroke::parse("enter").unwrap();
        let bytes = keystroke_to_bytes(&keystroke, TermMode::empty());
        assert_eq!(bytes, Some(b"\r".to_vec()));
    }

    #[test]
    fn test_escape_key() {
        let keystroke = Keystroke::parse("escape").unwrap();
        let bytes = keystroke_to_bytes(&keystroke, TermMode::empty());
        assert_eq!(bytes, Some(b"\x1b".to_vec()));
    }

    #[test]
    fn test_backspace_key() {
        let keystroke = Keystroke::parse("backspace").unwrap();
        let bytes = keystroke_to_bytes(&keystroke, TermMode::empty());
        assert_eq!(bytes, Some(b"\x7f".to_vec()));
    }

    #[test]
    fn test_tab_key() {
        let keystroke = Keystroke::parse("tab").unwrap();
        let bytes = keystroke_to_bytes(&keystroke, TermMode::empty());
        assert_eq!(bytes, Some(b"\t".to_vec()));
    }

    #[test]
    fn test_shift_tab() {
        let keystroke = Keystroke::parse("shift-tab").unwrap();
        let bytes = keystroke_to_bytes(&keystroke, TermMode::empty());
        assert_eq!(bytes, Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn test_arrow_keys_normal_mode() {
        let mode = TermMode::empty();

        let up = Keystroke::parse("up").unwrap();
        assert_eq!(keystroke_to_bytes(&up, mode), Some(b"\x1b[A".to_vec()));

        let down = Keystroke::parse("down").unwrap();
        assert_eq!(keystroke_to_bytes(&down, mode), Some(b"\x1b[B".to_vec()));

        let right = Keystroke::parse("right").unwrap();
        assert_eq!(keystroke_to_bytes(&right, mode), Some(b"\x1b[C".to_vec()));

        let left = Keystroke::parse("left").unwrap();
        assert_eq!(keystroke_to_bytes(&left, mode), Some(b"\x1b[D".to_vec()));
    }

    #[test]
    fn test_arrow_keys_app_cursor_mode() {
        let mode = TermMode::APP_CURSOR;

        let up = Keystroke::parse("up").unwrap();
        assert_eq!(keystroke_to_bytes(&up, mode), Some(b"\x1bOA".to_vec()));

        let down = Keystroke::parse("down").unwrap();
        assert_eq!(keystroke_to_bytes(&down, mode), Some(b"\x1bOB".to_vec()));

        let right = Keystroke::parse("right").unwrap();
        assert_eq!(keystroke_to_bytes(&right, mode), Some(b"\x1bOC".to_vec()));

        let left = Keystroke::parse("left").unwrap();
        assert_eq!(keystroke_to_bytes(&left, mode), Some(b"\x1bOD".to_vec()));
    }

    #[test]
    fn test_navigation_keys() {
        let mode = TermMode::empty();

        let home = Keystroke::parse("home").unwrap();
        assert_eq!(keystroke_to_bytes(&home, mode), Some(b"\x1b[H".to_vec()));

        let end = Keystroke::parse("end").unwrap();
        assert_eq!(keystroke_to_bytes(&end, mode), Some(b"\x1b[F".to_vec()));

        let pageup = Keystroke::parse("pageup").unwrap();
        assert_eq!(keystroke_to_bytes(&pageup, mode), Some(b"\x1b[5~".to_vec()));

        let pagedown = Keystroke::parse("pagedown").unwrap();
        assert_eq!(
            keystroke_to_bytes(&pagedown, mode),
            Some(b"\x1b[6~".to_vec())
        );

        let insert = Keystroke::parse("insert").unwrap();
        assert_eq!(keystroke_to_bytes(&insert, mode), Some(b"\x1b[2~".to_vec()));

        let delete = Keystroke::parse("delete").unwrap();
        assert_eq!(keystroke_to_bytes(&delete, mode), Some(b"\x1b[3~".to_vec()));
    }

    #[test]
    fn test_function_keys() {
        let mode = TermMode::empty();

        let f1 = Keystroke::parse("f1").unwrap();
        assert_eq!(keystroke_to_bytes(&f1, mode), Some(b"\x1bOP".to_vec()));

        let f2 = Keystroke::parse("f2").unwrap();
        assert_eq!(keystroke_to_bytes(&f2, mode), Some(b"\x1bOQ".to_vec()));

        let f5 = Keystroke::parse("f5").unwrap();
        assert_eq!(keystroke_to_bytes(&f5, mode), Some(b"\x1b[15~".to_vec()));

        let f12 = Keystroke::parse("f12").unwrap();
        assert_eq!(keystroke_to_bytes(&f12, mode), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn test_ctrl_combinations() {
        let mode = TermMode::empty();

        // Ctrl+A = 0x01
        let ctrl_a = Keystroke::parse("ctrl-a").unwrap();
        assert_eq!(keystroke_to_bytes(&ctrl_a, mode), Some(vec![0x01]));

        // Ctrl+C = 0x03
        let ctrl_c = Keystroke::parse("ctrl-c").unwrap();
        assert_eq!(keystroke_to_bytes(&ctrl_c, mode), Some(vec![0x03]));

        // Ctrl+Z = 0x1a
        let ctrl_z = Keystroke::parse("ctrl-z").unwrap();
        assert_eq!(keystroke_to_bytes(&ctrl_z, mode), Some(vec![0x1a]));

        // Ctrl+Space = 0x00
        let ctrl_space = Keystroke::parse("ctrl-space").unwrap();
        assert_eq!(keystroke_to_bytes(&ctrl_space, mode), Some(vec![0x00]));
    }

    #[test]
    fn test_alt_combinations() {
        let mode = TermMode::empty();

        // Alt+a sends ESC followed by 'a'
        let alt_a = Keystroke::parse("alt-a").unwrap();
        assert_eq!(keystroke_to_bytes(&alt_a, mode), Some(b"\x1ba".to_vec()));

        // Alt+x sends ESC followed by 'x'
        let alt_x = Keystroke::parse("alt-x").unwrap();
        assert_eq!(keystroke_to_bytes(&alt_x, mode), Some(b"\x1bx".to_vec()));
    }

    #[test]
    fn test_regular_characters() {
        let mode = TermMode::empty();

        // Printable text is committed by the input handler, not the key table.
        for key in ["a", "z", "0", "space"] {
            assert_eq!(
                keystroke_to_bytes(&Keystroke::parse(key).unwrap(), mode),
                None,
                "{key}"
            );
        }
    }

    #[test]
    fn test_space_key() {
        let mode = TermMode::empty();

        let space = Keystroke::parse("space").unwrap();
        assert_eq!(keystroke_to_bytes(&space, mode), None);
    }

    #[test]
    fn platform_shortcuts_stay_with_the_application() {
        let mode = TermMode::empty();
        for keys in ["cmd-c", "cmd-v", "cmd-w", "cmd-p"] {
            let keystroke = Keystroke::parse(keys).unwrap();
            assert_eq!(keystroke_to_bytes(&keystroke, mode), None, "{keys}");
        }
        // The shell still receives the control keys those letters stand for.
        assert_eq!(
            keystroke_to_bytes(&Keystroke::parse("ctrl-c").unwrap(), mode),
            Some(vec![0x03])
        );
        assert_eq!(
            keystroke_to_bytes(&Keystroke::parse("ctrl-w").unwrap(), mode),
            Some(vec![0x17])
        );
    }

    #[test]
    fn modified_keys_match_terminal_conventions() {
        let mode = TermMode::empty();
        let bytes = |keys: &str| keystroke_to_bytes(&Keystroke::parse(keys).unwrap(), mode);

        // Cmd's line and deletion conveniences (Terminal.app/Zed).
        assert_eq!(bytes("cmd-left"), Some(vec![0x01]));
        assert_eq!(bytes("cmd-right"), Some(vec![0x05]));
        assert_eq!(bytes("cmd-backspace"), Some(vec![0x15]));
        assert_eq!(bytes("cmd-delete"), Some(vec![0x0b]));
        // Option is Meta: word motion.
        assert_eq!(bytes("alt-left"), Some(b"\x1bb".to_vec()));
        assert_eq!(bytes("alt-right"), Some(b"\x1bf".to_vec()));
        assert_eq!(bytes("alt-backspace"), Some(b"\x1b\x7f".to_vec()));
        assert_eq!(bytes("alt-enter"), Some(b"\x1b\r".to_vec()));
        // xterm modifier forms for the arrows and editing keys.
        assert_eq!(bytes("ctrl-left"), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(bytes("ctrl-right"), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(bytes("shift-up"), Some(b"\x1b[1;2A".to_vec()));
        assert_eq!(bytes("shift-ctrl-right"), Some(b"\x1b[1;6C".to_vec()));
        assert_eq!(bytes("alt-down"), Some(b"\x1b[1;3B".to_vec()));
        assert_eq!(bytes("ctrl-delete"), Some(b"\x1b[3;5~".to_vec()));
        // Word deletion, and the keys TFT keeps for its own tab switching.
        assert_eq!(bytes("ctrl-backspace"), Some(vec![0x17]));
        assert_eq!(bytes("ctrl-tab"), None);
        assert_eq!(bytes("ctrl-pageup"), None);
        // Cmd shortcuts that are not terminal keys stay with the app.
        assert_eq!(bytes("cmd-w"), None);
        assert_eq!(bytes("cmd-p"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_composing_key_char_is_left_to_the_input_method() {
        let composing = Keystroke {
            key: "g".into(),
            key_char: Some("ㅇ".into()),
            ..Keystroke::default()
        };
        assert_eq!(keystroke_to_bytes(&composing, TermMode::empty()), None);
        let ascii = Keystroke {
            key: "g".into(),
            key_char: Some("g".into()),
            ..Keystroke::default()
        };
        assert_eq!(keystroke_to_bytes(&ascii, TermMode::empty()), None);
    }

    #[test]
    fn kitty_keyboard_protocol_is_used_when_enabled() {
        let mode = TermMode::DISAMBIGUATE_ESC_CODES;
        let bytes = |keys: &str| keystroke_to_bytes(&Keystroke::parse(keys).unwrap(), mode);
        assert_eq!(bytes("enter"), Some(b"\x1b[13u".to_vec()));
        assert_eq!(bytes("shift-enter"), Some(b"\x1b[13;2u".to_vec()));
        assert_eq!(bytes("ctrl-enter"), Some(b"\x1b[13;5u".to_vec()));
        assert_eq!(bytes("escape"), Some(b"\x1b[27u".to_vec()));
        assert_eq!(bytes("ctrl-a"), Some(b"\x1b[97;5u".to_vec()));
        assert_eq!(bytes("shift-up"), Some(b"\x1b[57352;2u".to_vec()));
        // Plain printable text still takes the input-handler commit path.
        assert_eq!(bytes("a"), None);

        let reporting = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_EVENT_TYPES;
        assert_eq!(
            keystroke_to_bytes(&Keystroke::parse("enter").unwrap(), reporting),
            Some(b"\x1b[13;1:1u".to_vec())
        );
    }

    #[test]
    fn kitty_progressive_flags_report_alternate_keys_text_and_events() {
        let mut keystroke = Keystroke::parse("shift-a").unwrap();
        keystroke.key_char = Some("A".into());

        let alternate = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALTERNATE_KEYS;
        assert_eq!(
            keystroke_to_bytes(&keystroke, alternate),
            Some(b"\x1b[97:65;2u".to_vec())
        );

        let text = TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_ASSOCIATED_TEXT;
        let mut plain_a = Keystroke::parse("a").unwrap();
        plain_a.key_char = Some("a".into());
        assert_eq!(
            keystroke_to_bytes(&plain_a, text),
            Some(b"\x1b[97;1;97u".to_vec())
        );

        let events = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_EVENT_TYPES;
        assert_eq!(
            keystroke_to_bytes_for_event(
                &Keystroke::parse("enter").unwrap(),
                events,
                KeyEventKind::Repeat
            ),
            Some(b"\x1b[13;1:2u".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes_for_event(
                &Keystroke::parse("enter").unwrap(),
                events,
                KeyEventKind::Release
            ),
            Some(b"\x1b[13;1:3u".to_vec())
        );
        // A release in a legacy terminal types nothing at all.
        assert_eq!(
            keystroke_to_bytes_for_event(
                &Keystroke::parse("enter").unwrap(),
                TermMode::empty(),
                KeyEventKind::Release
            ),
            None
        );

        // Report-all sends plain text as escape sequences instead of
        // leaving it to the input handler.
        let all = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC;
        let mut letter = Keystroke::parse("a").unwrap();
        letter.key_char = Some("a".into());
        assert_eq!(keystroke_to_bytes(&letter, all), Some(b"\x1b[97u".to_vec()));
    }

    #[test]
    fn the_shifted_special_key_reports_its_modifier() {
        let mode = TermMode::DISAMBIGUATE_ESC_CODES;
        assert_eq!(
            keystroke_to_bytes(&Keystroke::parse("shift-tab").unwrap(), mode),
            Some(b"\x1b[9;2u".to_vec())
        );
    }
}
