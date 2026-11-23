//! The Settings window: Appearance and Editor, read from `.obsidian/`.
//! Toggles carry the key they write so display and disk cannot drift; a click
//! emits [`SettingChange`] and the workspace merges the file.

use crate::theme::Theme;
use crate::workspace::{
    client_frame, frame_window_options, window_title_row, VaultState, WindowCorners, Workspace,
};
use gpui::{
    actions, deferred, div, prelude::*, px, size, App, Bounds, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, KeyBinding, ScrollHandle, SharedString, Subscription,
    WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tft_formats::config::{AppConfig, AppearanceConfig};

actions!(settings, [Close]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Close, Some("Settings"))]);
}

/// A boolean setting: the key Obsidian writes, its absent default, and its
/// read/write accessors. `invert` covers controls worded the other way round.
macro_rules! flags {
    ($ty:ident, $config:ty, $($name:ident, $key:literal, $field:ident, $default:literal, $invert:literal;)+) => {
        #[derive(Clone, Copy, Debug)]
        struct $ty {
            key: &'static str,
            default: bool,
            invert: bool,
            get: fn(&$config) -> Option<bool>,
            set: fn(&mut $config, bool),
        }

        /// Two flags are the same setting when they write the same key;
        /// the accessors are function pointers, which do not compare.
        impl PartialEq for $ty {
            fn eq(&self, other: &Self) -> bool {
                self.key == other.key
            }
        }

        impl Eq for $ty {}

        impl $ty {
            /// Every flag of this file, for the test that checks the keys.
            #[cfg(test)]
            const ALL: &'static [$ty] = &[$($name),+];

            /// What the control shows, from the config in hand.
            fn is_on(&self, config: &$config) -> bool {
                (self.get)(config).unwrap_or(self.default) != self.invert
            }

            /// `config` with the control flipped.
            fn toggled(&self, config: &$config) -> $config {
                let mut next = config.clone();
                (self.set)(&mut next, !self.is_on(config) != self.invert);
                next
            }
        }

        $(const $name: $ty = $ty {
            key: $key,
            default: $default,
            invert: $invert,
            get: |c| c.$field,
            set: |c, v| c.$field = Some(v),
        };)+
    };
}

flags!(
    AppFlag,
    AppConfig,
    SPELLCHECK, "spellcheck", spellcheck, true, false;
);

/// What a toggle writes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Flag {
    App(AppFlag),
}

/// A dropdown's options: what each row reads and what it writes.
///
/// The labels are Obsidian's, as the Settings tab shows them; the values
/// are what it puts in the file, which the phase 3 probe confirmed for
/// every one of these keys. See
/// [[Specification#Observed settings write behavior, Obsidian 1.14.4]].
type Options = &'static [(&'static str, &'static str)];

/// A setting that is not a toggle: the key it writes and the one pair of
/// accessors its control reads through and writes through, so what a row
/// shows and what it writes cannot drift apart.
#[derive(Clone, Copy)]
struct Typed {
    key: &'static str,
    options: Options,
    /// The value in the file now, as one of `options`' written values.
    read: fn(&VaultState) -> String,
    /// The file with this setting set to `value`.
    write: fn(&VaultState, &str) -> SettingChange,
}

impl Typed {
    /// The option the file's value names, or the first, which is Obsidian's
    /// default for every one of these keys.
    fn selected(&self, state: &VaultState) -> usize {
        let value = (self.read)(state);
        self.options
            .iter()
            .position(|(_, v)| *v == value)
            .unwrap_or(0)
    }

    /// The selected option's static label, which the tests read back.
    #[cfg(test)]
    fn label(&self, state: &VaultState) -> &'static str {
        self.options[self.selected(state)].0
    }

    /// Option `ix`'s label.
    fn option_label(&self, ix: usize) -> String {
        self.options[ix].0.to_string()
    }
}

impl PartialEq for Typed {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Typed {}

impl std::fmt::Debug for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Typed").field("key", &self.key).finish()
    }
}

/// Reads a string key of `app.json`, with the value Obsidian writes when it
macro_rules! appearance_choice {
    ($name:ident, $key:literal, $field:ident, $default:literal, $options:expr) => {
        const $name: Typed = Typed {
            key: $key,
            options: $options,
            read: |state| {
                state
                    .appearance()
                    .$field
                    .clone()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| $default.to_string())
            },
            write: |state, value| {
                let mut next = state.appearance().clone();
                next.$field = Some(value.to_string());
                SettingChange::Appearance(next)
            },
        };
    };
}

appearance_choice!(
    BASE_COLOR_SCHEME,
    "theme",
    theme,
    "system",
    &[
        ("Adapt to system", "system"),
        ("Light", "moonstone"),
        ("Dark", "obsidian")
    ]
);

/// Every dropdown that writes.
const TYPED_ALL: &[Typed] = &[BASE_COLOR_SCHEME];

/// A config file with one setting changed, for the workspace to write.
///
/// `AppConfig` dwarfs the other files, but the enum only ever travels
/// inside `SettingsEvent::Changed(Box<SettingChange>)`, so the size is
/// paid once on the heap rather than per event.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum SettingChange {
    App(AppConfig),
    Appearance(AppearanceConfig),
}

impl SettingChange {
    /// The file this change is written to, for an error message.
    pub fn file(&self) -> &'static str {
        match self {
            SettingChange::App(_) => "app.json",
            SettingChange::Appearance(_) => "appearance.json",
        }
    }
}

/// The flag a toggle command flips: `editor:toggle-spellcheck` is the
/// Editor tab's "Spellcheck" row run from the palette, so it writes through
/// the very flag the row does.
fn command_flag(id: &str) -> Option<Flag> {
    match id {
        "editor:toggle-spellcheck" => Some(Flag::App(SPELLCHECK)),
        _ => None,
    }
}

/// What a toggle command writes: its row's file with the flag flipped, or
/// `None` for an id that is not one of them. The config is read and let go
/// in here, so the caller holds no read guard when it writes the change.
pub fn toggle_command(id: &str, state: &VaultState) -> Option<SettingChange> {
    command_flag(id).map(|flag| flag.toggled(state))
}

/// `theme:toggle-light-dark` over a base theme the file names: `obsidian`
/// (dark) and `moonstone` (light) swapped, the other keys kept. Under
/// `system`, a missing key (which Obsidian reads as `system`) or anything
/// else it writes nothing and answers `None`; the caller flips the
/// session's scheme instead (Obsidian 1.15.0, all four starting points).
pub fn toggle_light_dark(appearance: &AppearanceConfig) -> Option<AppearanceConfig> {
    let swapped = match appearance.theme.as_deref() {
        Some("obsidian") => "moonstone",
        Some("moonstone") => "obsidian",
        _ => return None,
    };
    let mut next = appearance.clone();
    next.theme = Some(swapped.to_string());
    Some(next)
}

impl Flag {
    /// The config file with this setting flipped.
    fn toggled(&self, state: &VaultState) -> SettingChange {
        match self {
            Flag::App(flag) => SettingChange::App(flag.toggled(&state.app())),
        }
    }
}

/// What a control draws.
#[derive(Debug, Clone, PartialEq)]
enum Control {
    None,
    Toggle(bool),
    Dropdown(SharedString),
    /// "Spellcheck languages": a pill per chosen language (each with a ×
    /// while there is more than one), a "Restore default" icon once the
    /// list is not Obsidian's `["en-US"]`, and a "+" dropdown of the rest.
    /// Read from the session's spellchecker when drawn.
    Languages,
}

/// Whether an open dropdown swallows this click, which closes it and spends
/// the click: the one gesture does not also do what it landed on. The next
/// click, with nothing open, goes through.
fn popover_swallows_click(menu: &mut Option<&'static str>) -> bool {
    menu.take().is_some()
}

/// What Escape closes, innermost first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// An open dropdown's option list.
    Popover,
    /// A navigable row's page, which backs out to the tab's own rows.
    Page,
    /// The modal itself, when nothing inside it is open.
    Modal,
}

/// Escape takes the innermost thing open: an open dropdown, then a page,
/// and the modal only when neither is. Obsidian's dropdown is a native
/// `<select>`, so Escape over an open option list closes the list and
/// leaves the dialog where it was.
fn escape_closes(popover_open: bool, page_open: bool) -> Escape {
    if popover_open {
        Escape::Popover
    } else if page_open {
        Escape::Page
    } else {
        Escape::Modal
    }
}

/// A page a tab opens in place of its rows. Obsidian's rows for these are
/// `.setting-item.mod-navigable.tappable`, and the page has a titlebar of
/// its own with a back chevron (probed 2026-11-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// "Spellcheck dictionary", behind the Spellcheck row's cog: the words
    /// "Add to dictionary" kept, each with a × that removes it. Obsidian
    /// opens it as a modal over Settings; here it is a page of Settings.
    SpellcheckDictionary,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::SpellcheckDictionary => "Spellcheck dictionary",
        }
    }
}

#[derive(Debug, Clone)]
struct Item {
    heading: bool,
    name: SharedString,
    desc: SharedString,
    control: Control,
    /// What a click writes; `None` for the controls that are still
    /// read-only.
    flag: Option<Flag>,
    /// What a dropdown writes, for the controls that are not toggles.
    typed: Option<Typed>,
    /// The page a `lucide-settings` clickable icon before the control
    /// opens (`extra-setting-button`): the Spellcheck row's dictionary.
    cog: Option<Page>,
}

fn heading(name: &str) -> Item {
    Item {
        heading: true,
        name: name.to_string().into(),
        desc: "".into(),
        control: Control::None,
        flag: None,
        typed: None,
        cog: None,
    }
}

fn item(name: &str, desc: &str, control: Control) -> Item {
    Item {
        heading: false,
        name: name.to_string().into(),
        desc: desc.to_string().into(),
        control,
        flag: None,
        typed: None,
        cog: None,
    }
}

/// A toggle drawn from the flag it writes, so its state and its click are
/// the same key.
fn toggle(name: &str, desc: &str, on: bool, flag: Flag) -> Item {
    Item {
        flag: Some(flag),
        ..item(name, desc, Control::Toggle(on))
    }
}

fn app_toggle(name: &str, desc: &str, app: &AppConfig, flag: AppFlag) -> Item {
    toggle(name, desc, flag.is_on(app), Flag::App(flag))
}

/// A dropdown that writes: what it shows and what a pick writes both come
/// from `typed`, so a row cannot show one key and write another.
fn choice(name: &str, desc: &str, state: &VaultState, typed: Typed) -> Item {
    choice_at(name, desc, typed, typed.selected(state))
}

/// [`choice`] showing option `ix`, for a dropdown whose selection is not
/// only the file's: a local option is picked, not stored.
fn choice_at(name: &str, desc: &str, typed: Typed, ix: usize) -> Item {
    Item {
        typed: Some(typed),
        ..item(name, desc, Control::Dropdown(typed.option_label(ix).into()))
    }
}

pub enum SettingsEvent {
    Close,
    /// A control changed a setting: the config file to write, with the one
    /// key changed. The workspace writes it and applies what it can. Boxed
    /// because a whole `AppConfig` dwarfs the other events.
    Changed(Box<SettingChange>),
}

pub struct SettingsView {
    focus_handle: FocusHandle,
    state: Option<Arc<VaultState>>,
    /// The page open in place of the single settings list.
    page: Option<Page>,
    /// Where the list was scrolled to.
    items_scroll: ScrollHandle,
    /// The dropdown whose options are showing, by the key it writes.
    menu: Option<&'static str>,
    /// Which window corners the panel rounds, read off the window each
    /// render, under the frame's title row.
    corners: WindowCorners,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> SettingsView {
        SettingsView {
            focus_handle: cx.focus_handle(),
            state: None,
            page: None,
            items_scroll: ScrollHandle::new(),
            menu: None,
            corners: WindowCorners::default(),
        }
    }

    pub fn set_state(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        self.state = Some(state);
        cx.notify();
    }

    /// Opens a navigable row's page.
    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.page = Some(page);
        self.focus_tab(window, cx);
        cx.notify();
    }

    /// Back out of a page to the single settings list.
    fn close_page(&mut self, cx: &mut Context<Self>) {
        self.page = None;
        cx.notify();
    }

    /// Shows the modal and takes focus so Escape closes it.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The view outlives the modal, so a dropdown or a text field left
        // open by the last session would come back expanded. Every way out
        // of the modal commits the field first, so there is nothing to write
        // here, only state to drop.
        self.menu = None;
        // A page left open by the last session would come back instead of
        // the list.
        self.page = None;
        self.focus_tab(window, cx);
        cx.notify();
    }

    /// What the list has in hand, for the control socket's state dump. A
    /// dropdown and a text field are reported by the config key they write.
    pub fn describe(&self, rows: usize, _cx: &App) -> Value {
        let listed = self.described_rows(rows, _cx);
        let total = self.row_count();
        json!({
            "page": self.page.map(Page::title),
            "menu": self.menu,
            "rows": listed,
            "rows_total": (total > rows).then_some(total),
        })
    }

    /// How many rows the list has, listed or not.
    fn row_count(&self) -> usize {
        self.items().len()
    }

    /// The open tab's rows, for the control socket's state dump: what each
    /// one is called, what kind of control it carries and where that
    /// control stands, and the tag saying how far TFT gets with it.
    ///
    fn described_rows(&self, rows: usize, _cx: &App) -> Vec<Value> {
        self.items()
            .into_iter()
            .take(rows)
            .map(|item| {
                let (control, value) = match &item.control {
                    Control::None => ("none", Value::Null),
                    Control::Toggle(on) => ("toggle", json!(on)),
                    Control::Dropdown(v) => ("dropdown", json!(v.as_ref())),
                    Control::Languages => ("languages", Value::Null),
                };
                json!({
                    "kind": if item.heading { "heading" } else { "setting" },
                    "name": item.name.as_ref(),
                    "desc": item.desc.as_ref(),
                    "control": control,
                    "value": value,
                })
            })
            .collect()
    }

    fn focus_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        match escape_closes(self.menu.is_some(), self.page.is_some()) {
            Escape::Popover => {
                self.menu = None;
                cx.notify();
            }
            Escape::Page => {
                self.dismiss_open_controls(cx);
                self.close_page(cx);
                self.focus_tab(window, cx);
            }
            Escape::Modal => {
                self.dismiss_open_controls(cx);
                cx.emit(SettingsEvent::Close);
            }
        }
    }

    fn close_from_frame(&mut self, cx: &mut Context<Self>) {
        self.dismiss_open_controls(cx);
        cx.emit(SettingsEvent::Close);
    }

    fn dismiss_open_controls(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// Whether this click was spent closing an open dropdown, and so must
    /// not also do whatever it landed on.
    ///
    /// Obsidian's dropdown is a native `<select>`, whose option list is the
    /// platform's own window: the click that dismisses it never reaches the
    /// page, so the toggle under it does not flip. TFT's popover is part
    /// of the same window, so the rule has to be spelled out — every
    /// clickable in the modal asks this first, the popover's own option rows
    /// excepted, since a click there is the pick.
    fn click_dismissed_a_popover(&mut self, cx: &mut Context<Self>) -> bool {
        if !popover_swallows_click(&mut self.menu) {
            return false;
        }
        cx.notify();
        true
    }

    /// A click on a toggle: ask the workspace to write the flipped value.
    /// The modal redraws from the vault's config once the write lands, so a
    /// failed write leaves the control where it was.
    fn toggle_flag(&mut self, flag: Flag, _window: &Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        cx.emit(SettingsEvent::Changed(Box::new(flag.toggled(&state))));
    }

    // ----- the single list -----------------------------------------------

    /// Every setting, in one list: Appearance, then Editor.
    fn items(&self) -> Vec<Item> {
        let Some(state) = self.state.as_ref() else {
            return vec![item("No vault is open", "", Control::None)];
        };
        let mut items = vec![heading("Appearance")];
        items.extend(Self::appearance_items(state));
        items.push(heading("Editor"));
        items.extend(Self::editor_items(state));
        items
    }

    /// The Appearance section. Only the base color scheme remains: light,
    /// dark, or the system's own, with no theme, snippet, accent or font
    /// customization.
    fn appearance_items(state: &VaultState) -> Vec<Item> {
        vec![choice(
            "Base color scheme",
            "Choose TFT's default color scheme.",
            state,
            BASE_COLOR_SCHEME,
        )]
    }

    fn editor_items(state: &VaultState) -> Vec<Item> {
        let app = &state.app();
        vec![
            Item {
                cog: Some(Page::SpellcheckDictionary),
                ..app_toggle("Spellcheck", "Turn on the spellchecker.", app, SPELLCHECK)
            },
            item(
                "Spellcheck languages",
                "Choose the languages for the spellchecker to use.",
                Control::Languages,
            ),
        ]
    }

    fn render_control(
        &self,
        item: &Item,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match &item.control {
            Control::None => div().into_any_element(),
            Control::Toggle(on) => {
                let on = *on;
                let toggle = div()
                    .relative()
                    .w(px(36.))
                    .h(px(20.))
                    .rounded_full()
                    // `.checkbox-container`: `--background-modifier-border-hover`,
                    // `--interactive-accent` while on (probe of 2026-11-02).
                    .bg(if on {
                        theme.tokens.interactive_accent
                    } else {
                        theme.tokens.background_modifier_border_hover
                    })
                    .border_1()
                    .border_color(if on {
                        theme.tokens.interactive_accent
                    } else {
                        theme.tokens.background_modifier_border_hover
                    })
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .left(px(if on { 17. } else { 2. }))
                            .size(px(14.))
                            .rounded_full()
                            .bg(theme.tokens.toggle_thumb_color),
                    );
                let Some(flag) = item.flag.clone() else {
                    return toggle.into_any_element();
                };
                div()
                    .id(SharedString::from(format!("settings-toggle-{}", item.name)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.click_dismissed_a_popover(cx) {
                            return;
                        }
                        this.toggle_flag(flag.clone(), window, cx)
                    }))
                    .child(toggle)
                    .into_any_element()
            }
            Control::Dropdown(value) => {
                let shown = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(3.))
                    .rounded_sm()
                    // `select.dropdown`: `--text-normal` on
                    // `--dropdown-background`.
                    .bg(theme.tokens.dropdown_background)
                    .border_1()
                    .border_color(theme.tokens.background_modifier_border)
                    .text_size(px(12.))
                    .text_color(theme.tokens.text_normal)
                    .child(value.clone())
                    .child(div().text_color(theme.tokens.text_muted).child("▾"));
                let Some(typed) = item.typed else {
                    return shown.into_any_element();
                };
                let selected = self.selected_of(typed);
                let open = self.menu == Some(typed.key);
                div()
                    .relative()
                    .child(
                        div()
                            .id(SharedString::from(format!("settings-menu-{}", typed.key)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // The root closes whatever is open on any
                                // click; this one has to survive it, or the
                                // menu would shut again as the click bubbles.
                                cx.stop_propagation();
                                // A click with a popover open is spent
                                // closing it — this one's own second click
                                // included, which is how the property type
                                // menu closes too.
                                if this.click_dismissed_a_popover(cx) {
                                    return;
                                }
                                this.menu = Some(typed.key);
                                cx.notify();
                            }))
                            .child(shown),
                    )
                    .when(open, |d| {
                        // Deferred, or the rows after this one paint over
                        // the options: a later sibling wins otherwise.
                        d.child(deferred(
                            div()
                                .absolute()
                                .top(px(26.))
                                .right(px(0.))
                                .min_w(px(200.))
                                .flex()
                                .flex_col()
                                .bg(theme.tokens.background_secondary)
                                .border_1()
                                .border_color(theme.tokens.background_modifier_border)
                                .rounded_md()
                                .shadow_lg()
                                .py_1()
                                .children(typed.options.iter().enumerate().map(|(ix, _)| {
                                    div()
                                        .id(SharedString::from(format!(
                                            "settings-option-{}-{ix}",
                                            typed.key
                                        )))
                                        .px_3()
                                        .py_1()
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .when(ix == selected, |d| {
                                            d.bg(theme.tokens.background_modifier_hover)
                                        })
                                        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                                        .text_color(theme.tokens.text_normal)
                                        .child(SharedString::from(typed.option_label(ix)))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.pick(typed, ix, window, cx)
                                        }))
                                })),
                        ))
                    })
                    .into_any_element()
            }
            Control::Languages => self.render_languages(theme, cx),
        }
    }

    /// Opens the dropdown that writes `key`, for `--menu`: clicking one is
    /// what does it in the app.
    pub fn open_menu(&mut self, key: &str, cx: &mut Context<Self>) {
        self.menu = TYPED_ALL.iter().find(|t| t.key == key).map(|t| t.key);
        cx.notify();
    }
    /// The option a dropdown shows: the one the file names.
    fn selected_of(&self, typed: Typed) -> usize {
        self.state.as_ref().map(|s| typed.selected(s)).unwrap_or(0)
    }

    /// Writes the option a dropdown's row names. The value comes from the
    /// same table the control reads through, so the two cannot drift.
    fn pick(&mut self, typed: Typed, option: usize, _window: &Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some((_, value)) = typed.options.get(option) else {
            return;
        };
        if (typed.read)(&state) != *value {
            cx.emit(SettingsEvent::Changed(Box::new((typed.write)(
                &state, value,
            ))));
        }
        cx.notify();
    }

    fn render_item(&self, item: &Item, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        if item.heading {
            return div()
                .pt_5()
                .pb_1()
                .text_size(px(15.))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.tokens.text_normal)
                .child(item.name.clone())
                .into_any_element();
        }
        div()
            .id(SharedString::from(format!("settings-row-{}", item.name)))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_6()
            .py(px(10.))
            .border_t_1()
            .border_color(theme.tokens.background_modifier_border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(theme.tokens.text_normal)
                            .child(item.name.clone()),
                    )
                    .when(!item.desc.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.tokens.text_muted)
                                .child(item.desc.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .children(item.cog.map(|page| self.cog_button(page, theme, cx)))
                    .child(self.render_control(item, theme, cx)),
            )
            .into_any_element()
    }

    /// A `lucide-settings` clickable icon that opens one of the pages.
    fn cog_button(
        &self,
        page: Page,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("settings-cog-{}", page.title())))
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme.tokens.background_modifier_hover))
            .child(crate::assets::icon(
                "settings",
                16.,
                theme.tokens.icon_color,
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if this.click_dismissed_a_popover(cx) {
                    return;
                }
                this.open_page(page, window, cx)
            }))
    }

    /// "Spellcheck dictionary": the custom words, one row each, the word
    /// and a `lucide-x` "Remove from dictionary" before it, as Obsidian's
    /// `spellchecker-dictionary-item` rows have them (probed 2026-11-15).
    fn render_dictionary_page(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let words = crate::spellcheck::custom_words(cx);
        let mut list = div()
            .id("settings-spell-dictionary")
            .overflow_y_scroll()
            .size_full()
            .flex()
            .flex_col();
        if words.is_empty() {
            list = list.child(
                div()
                    .py_4()
                    .text_size(px(12.))
                    .text_color(theme.tokens.text_muted)
                    .child("No words added."),
            );
        }
        for (ix, word) in words.into_iter().enumerate() {
            let remove = word.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .py(px(2.))
                    .child(
                        div()
                            .id(SharedString::from(format!("settings-spell-remove-{ix}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                            .tooltip(crate::ui::tooltip("Remove from dictionary"))
                            .child(crate::assets::icon("x", 14., theme.tokens.icon_color))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                crate::spellcheck::remove_word(&remove, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(theme.tokens.text_normal)
                            .child(word),
                    ),
            );
        }
        list
    }

    /// "Spellcheck languages": the chosen languages as pills, a restore
    /// icon away from the default, and the "+" dropdown of the others a
    /// dictionary was found for.
    fn render_languages(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        const KEY: &str = "spellcheck-languages";
        let chosen = crate::spellcheck::languages(cx);
        let missing = crate::spellcheck::missing(cx);
        let others: Vec<String> = crate::spellcheck::available(cx)
            .into_iter()
            .filter(|a| !chosen.iter().any(|c| c.eq_ignore_ascii_case(a)))
            .collect();
        let several = chosen.len() > 1;
        let mut pills = Vec::new();
        for (ix, code) in chosen.iter().enumerate() {
            let without: Vec<String> = chosen.iter().filter(|c| *c != code).cloned().collect();
            let absent = missing.iter().any(|m| m == code);
            let name = crate::spellcheck::display_name(code);
            let label = if absent {
                format!("{name} (no dictionary)")
            } else {
                name
            };
            pills.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded_sm()
                    .bg(theme.tokens.background_modifier_hover)
                    .text_size(px(12.))
                    .text_color(if absent {
                        theme.tokens.text_faint
                    } else {
                        theme.tokens.text_normal
                    })
                    .child(SharedString::from(label))
                    .when(several, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!(
                                    "settings-spell-lang-remove-{ix}"
                                )))
                                .cursor_pointer()
                                .child(crate::assets::icon("x", 12., theme.tokens.icon_color))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if this.click_dismissed_a_popover(cx) {
                                        return;
                                    }
                                    crate::spellcheck::set_languages(without.clone(), cx);
                                    cx.notify();
                                })),
                        )
                    }),
            );
        }
        let open = self.menu == Some(KEY);
        let at_default = chosen == tft_spell::default_languages();
        let mut rows = Vec::new();
        for (ix, code) in others.into_iter().enumerate() {
            let mut next = chosen.clone();
            next.push(code.clone());
            rows.push(
                div()
                    .id(SharedString::from(format!("settings-option-{KEY}-{ix}")))
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                    .text_color(theme.tokens.text_normal)
                    .child(SharedString::from(crate::spellcheck::display_name(&code)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.menu = None;
                        crate::spellcheck::set_languages(next.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(div().flex().flex_row().flex_wrap().gap_1().children(pills))
            .when(!at_default, |d| {
                d.child(
                    div()
                        .id("settings-spell-lang-restore")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                        .tooltip(crate::ui::tooltip("Restore default"))
                        .child(crate::assets::icon(
                            "rotate-ccw",
                            16.,
                            theme.tokens.icon_color,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if this.click_dismissed_a_popover(cx) {
                                return;
                            }
                            crate::spellcheck::set_languages(tft_spell::default_languages(), cx);
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("settings-menu-spellcheck-languages")
                            .px_2()
                            .py(px(3.))
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(theme.tokens.dropdown_background)
                            .border_1()
                            .border_color(theme.tokens.background_modifier_border)
                            .text_size(px(12.))
                            .text_color(theme.tokens.text_normal)
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                if this.click_dismissed_a_popover(cx) {
                                    return;
                                }
                                this.menu = Some(KEY);
                                cx.notify();
                            })),
                    )
                    .when(open, |d| {
                        // Deferred, or the rows after this one paint over it.
                        d.child(deferred(
                            div()
                                .id("settings-spell-lang-options")
                                .absolute()
                                .top(px(26.))
                                .right(px(0.))
                                .min_w(px(200.))
                                .max_h(px(300.))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .bg(theme.tokens.background_secondary)
                                .border_1()
                                .border_color(theme.tokens.background_modifier_border)
                                .rounded_md()
                                .shadow_lg()
                                .py_1()
                                .children(rows),
                        ))
                    }),
            )
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        self.corners = WindowCorners::of(window);
        let page = self.page;
        let body: gpui::AnyElement = match page {
            Some(Page::SpellcheckDictionary) => {
                self.render_dictionary_page(&theme, cx).into_any_element()
            }
            None => {
                let items = self.items();
                let mut list = div()
                    .id("settings-items")
                    .overflow_y_scroll()
                    .track_scroll(&self.items_scroll)
                    .size_full();
                for item in &items {
                    list = list.child(self.render_item(item, &theme, cx));
                }
                list.into_any_element()
            }
        };
        // The title row sits over the content column alone, as the vault
        // manager's does over its splash: the nav runs the full height and
        // meets the top-left corner itself. The row is the content's own
        // ground rather than a bar, and its close goes through the panel,
        // so an open field is written first.
        let view = cx.entity().downgrade();
        let title_row = window_title_row(
            "Settings",
            theme.tokens.modal_background,
            &theme,
            window,
            move |_, cx| {
                view.update(cx, |view, cx| view.close_from_frame(cx)).ok();
            },
        );
        div()
            .id("settings")
            .key_context("Settings")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close_action))
            .on_click(cx.listener(|this, _, _, cx| this.dismiss_open_controls(cx)))
            // The backdrop is outside the panel, and it closes the modal on
            // mouse up; writing on the way down is what keeps a click there
            // from losing what was typed.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_open_controls(cx)))
            .flex()
            .flex_col()
            // The panel fills its window, which the desktop sizes: no
            // border, rounding or shadow of its own, because the frame
            // around it is the real one.
            .size_full()
            .relative()
            .bg(theme.tokens.modal_background)
            .map(|d| self.corners.apply(d))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .map(|d| self.corners.right().apply(d))
                    .children(title_row)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .px_6()
                            .pb_4()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .pt_2()
                            .pb_2()
                            .child(
                                // `.setting-page-titlebar`: a page replaces
                                // the tab's name with a back chevron and
                                // its own.
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .when_some(page, |d, page| {
                                        d.child(
                                            div()
                                                .id("settings-page-back")
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .size(px(24.))
                                                .rounded_sm()
                                                .cursor_pointer()
                                                .hover(|s| {
                                                    s.bg(theme.tokens.background_modifier_hover)
                                                })
                                                .child(crate::assets::icon(
                                                    "chevron-left",
                                                    18.,
                                                    theme.tokens.icon_color,
                                                ))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    if this.click_dismissed_a_popover(cx) {
                                                        return;
                                                    }
                                                    let _ = page;
                                                    this.close_page(cx)
                                                })),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_size(px(18.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.tokens.text_normal)
                                            .child(match page {
                                                Some(page) => page.title(),
                                                None => "Settings",
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .mb_2()
                            .px_3()
                            .py(px(6.))
                            .rounded_sm()
                            .bg(theme.tokens.background_modifier_hover)
                            .text_size(px(11.))
                            .text_color(theme.tokens.text_muted)
                            .child("Changes are written to the vault's .obsidian folder as you use them."),
                    )
                    .child(div().flex_1().min_h_0().child(body)),
                    ),
            )
    }
}

/// The Settings window's size. Obsidian's modal is `min(90vw, 1100px)`
/// wide and `80vh` tall; a window of its own opens at that width and a
/// height to match, centered on the screen, and the desktop takes it from
/// there.
pub const WINDOW_SIZE: (f32, f32) = (1100., 720.);

/// The Settings window's root view: the frame around [`SettingsView`].
///
/// The panel itself belongs to the workspace, which keeps it between
/// openings and listens to it for the settings it writes, so this holds
/// nothing but the handle to it.
pub struct SettingsWindow {
    view: Entity<SettingsView>,
    focus_handle: FocusHandle,
    /// Takes this window down with the workspace that opened it. The panel
    /// writes through the workspace, so one left behind by a closed
    /// workspace would show a vault that is gone and write nothing.
    _workspace_gone: Option<Subscription>,
}

impl SettingsWindow {
    /// Opens the window on `tab`, with the workspace's own panel inside it.
    pub fn open(
        view: Entity<SettingsView>,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) -> anyhow::Result<WindowHandle<SettingsWindow>> {
        let (w, h) = WINDOW_SIZE;
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        let handle = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(480.))),
                // The same frame the main window has, with a title row of
                // its own for the window buttons; its close goes through
                // the panel, so an open field is written first.
                ..frame_window_options("Settings")
            },
            move |window, cx| {
                view.update(cx, |view, cx| view.open(window, cx));
                // A text field is written on the way out of it, and the
                // title bar's close is a way out the panel never sees.
                let open_field = view.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    open_field
                        .update(cx, |view, cx| view.dismiss_open_controls(cx))
                        .ok();
                    true
                });
                cx.new(|cx| SettingsWindow {
                    view,
                    focus_handle: cx.focus_handle(),
                    _workspace_gone: None,
                })
            },
        )?;
        if let Some(workspace) = workspace.upgrade() {
            let window = handle;
            let gone = cx.observe_release(&workspace, move |_, cx| {
                window
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            });
            handle
                .update(cx, |root, _, _| root._workspace_gone = Some(gone))
                .ok();
        }
        Ok(handle)
    }

    /// Escape with nothing inside the panel focused still closes the
    /// window, through the panel so an open field is written first.
    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        self.view.update(cx, |view, cx| view.close_from_frame(cx));
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let corners = WindowCorners::of(window);
        // The title row is the panel's, drawn over its content column.
        let root = div()
            .id("settings-window")
            .key_context("Settings")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.tokens.modal_background)
            .map(|d| corners.apply(d))
            .font_family(theme.ui_font.clone())
            .text_size(px(crate::theme::FONT_UI_MEDIUM))
            .text_color(theme.tokens.text_normal)
            .child(div().flex_1().min_h_0().w_full().child(self.view.clone()));
        client_frame(root.into_any_element(), &theme, window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every toggle of every file writes the key Obsidian writes, flips
    /// only that key, and then reads back the other way round.
    #[test]
    fn every_toggle_writes_its_own_key_and_nothing_else() {
        macro_rules! check {
            ($ty:ident, $config:ty) => {
                for flag in $ty::ALL {
                    let before = <$config>::default();
                    let after = flag.toggled(&before);
                    let json = serde_json::to_value(&after).unwrap();
                    let object = json.as_object().expect("an object");
                    assert_eq!(
                        object.len(),
                        1,
                        "{} changed more than its own key: {json}",
                        flag.key
                    );
                    assert_eq!(
                        object.get(flag.key),
                        Some(&serde_json::Value::Bool(!flag.default)),
                        "{} wrote the wrong key or value: {json}",
                        flag.key
                    );
                    assert_eq!(
                        flag.is_on(&after),
                        !flag.is_on(&before),
                        "{} did not flip",
                        flag.key
                    );
                }
            };
        }
        check!(AppFlag, AppConfig);
    }

    #[test]
    fn every_dropdown_writes_its_own_key_and_nothing_else() {
        let (_dir, state) = vault();
        for typed in TYPED_ALL {
            for (label, value) in typed.options {
                let change = (typed.write)(&state, value);
                let json = match &change {
                    SettingChange::App(c) => serde_json::to_value(c).unwrap(),
                    SettingChange::Appearance(c) => serde_json::to_value(c).unwrap(),
                };
                let object = json.as_object().expect("an object").clone();
                assert_eq!(
                    object.len(),
                    1,
                    "{} changed more than its own key for {label}: {json}",
                    typed.key
                );
                let written = object.get(typed.key).expect("its own key");
                let want = match *value {
                    "true" => serde_json::Value::Bool(true),
                    "false" => serde_json::Value::Bool(false),
                    v => serde_json::Value::String(v.to_string()),
                };
                assert_eq!(
                    written, &want,
                    "{} wrote the wrong value for {label}: {json}",
                    typed.key
                );
            }
        }
    }

    #[test]
    fn a_dropdown_reads_back_what_it_wrote() {
        // The control's label and its write go through the same table, so
        // picking an option must leave that option showing.
        let (dir, mut state) = vault();
        for typed in TYPED_ALL {
            for (ix, (label, value)) in typed.options.iter().enumerate() {
                state.apply_setting(&(typed.write)(&state, value)).unwrap();
                state = VaultState::load(&dir.path().to_path_buf()).unwrap();
                assert_eq!(
                    typed.selected(&state),
                    ix,
                    "{} did not read back {label}",
                    typed.key
                );
                assert_eq!(typed.label(&state), *label);
            }
        }
    }

    /// Escape takes the innermost thing open, so a dropdown closes without
    /// taking the modal with it and a page backs out to its tab.
    #[test]
    fn escape_closes_the_innermost_thing_open() {
        assert_eq!(escape_closes(false, false), Escape::Modal);
        assert_eq!(escape_closes(true, false), Escape::Popover);
        // A page is outside a dropdown it holds and inside the modal.
        assert_eq!(escape_closes(false, true), Escape::Page);
        assert_eq!(escape_closes(true, true), Escape::Popover);
    }

    /// Only the spellcheck row is a toggle command now; the removed rows'
    /// ids are silent.
    #[test]
    fn only_the_spellcheck_toggle_is_a_command() {
        let (_dir, state) = vault();
        let flag = SettingsView::editor_items(&state)
            .into_iter()
            .find(|i| i.name.as_ref() == "Spellcheck")
            .and_then(|i| i.flag)
            .expect("the spellcheck row");
        assert_eq!(
            toggle_command("editor:toggle-spellcheck", &state),
            Some(flag.toggled(&state))
        );
        for id in [
            "editor:toggle-readable-line-length",
            "editor:toggle-line-numbers",
            "editor:toggle-bold",
        ] {
            assert_eq!(toggle_command(id, &state), None, "{id}");
        }
    }

    /// `editor:toggle-spellcheck` over two runs, from no key and from
    /// `false`: `app.json`'s bytes after each are Obsidian 1.15.0's (the
    /// 2026-11-09 `attachments-app-info` probe, `s01`), the key written
    /// even when it goes back to the default.
    #[test]
    fn toggle_spellcheck_writes_obsidians_bytes_over_two_runs() {
        for (start, first, second) in [
            (
                "{}",
                "{\n  \"spellcheck\": false\n}",
                "{\n  \"spellcheck\": true\n}",
            ),
            (
                "{\n  \"spellcheck\": false\n}",
                "{\n  \"spellcheck\": true\n}",
                "{\n  \"spellcheck\": false\n}",
            ),
        ] {
            let (dir, _) = vault();
            let path = dir.path().join(".obsidian/app.json");
            std::fs::write(&path, start).unwrap();
            let state = VaultState::load(&dir.path().to_path_buf()).unwrap();
            let change = toggle_command("editor:toggle-spellcheck", &state).unwrap();
            state.apply_setting(&change).unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                first,
                "from {start}"
            );
            let change = toggle_command("editor:toggle-spellcheck", &state).unwrap();
            state.apply_setting(&change).unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                second,
                "from {start}"
            );
        }
    }

    /// `obsidian` and `moonstone` swap, the file's other keys kept; under
    /// `system`, no key, or a value Obsidian does not know, nothing is
    /// written.
    #[test]
    fn toggle_light_dark_swaps_the_two_base_themes_and_leaves_system_alone() {
        let (dir, _) = vault();
        let path = dir.path().join(".obsidian/appearance.json");
        std::fs::write(
            &path,
            "{\n  \"accentColor\": \"#ff5721\",\n  \"theme\": \"obsidian\",\n  \"probeUnknown\": true\n}",
        )
        .unwrap();
        let state = VaultState::load(&dir.path().to_path_buf()).unwrap();
        let next = toggle_light_dark(&state.appearance()).expect("a swap");
        state
            .apply_setting(&SettingChange::Appearance(next))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"accentColor\": \"#ff5721\",\n  \"theme\": \"moonstone\",\n  \"probeUnknown\": true\n}"
        );
        let back = toggle_light_dark(&state.appearance()).expect("a swap");
        assert_eq!(back.theme.as_deref(), Some("obsidian"));
        for theme in [Some("system"), None, Some("probe")] {
            let config = AppearanceConfig {
                theme: theme.map(str::to_string),
                ..AppearanceConfig::default()
            };
            assert_eq!(toggle_light_dark(&config), None, "{theme:?}");
        }
    }

    #[test]
    fn a_toggle_shows_the_file_first_and_the_default_when_the_key_is_absent() {
        let mut config = AppConfig::default();
        assert!(SPELLCHECK.is_on(&config), "spellcheck is on by default");
        config.spellcheck = Some(false);
        assert!(!SPELLCHECK.is_on(&config));
    }

    /// A vault to build the tabs against; `VaultState::load` wants a real
    /// folder.
    fn vault() -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join("Note.md"), "# Note\n").unwrap();
        let state = VaultState::load(&root.to_path_buf()).unwrap();
        (dir, state)
    }

    fn toggles(state: &VaultState) -> Vec<String> {
        [
            SettingsView::appearance_items(state),
            SettingsView::editor_items(state),
        ]
        .into_iter()
        .flatten()
        .filter(|i| i.flag.is_some())
        .map(|i| i.name.to_string())
        .collect()
    }

    /// Every row that writes carries a flag.
    #[test]
    fn every_toggle_writes_something() {
        let (_dir, state) = vault();
        let toggles = toggles(&state);
        assert!(!toggles.is_empty(), "no toggles were built");
        for name in &toggles {
            assert!(!name.is_empty(), "a toggle has no name");
        }
    }
}
