//! The Settings window: Appearance and Editor, read from `.tft/`.
//! Controls emit [`SettingChange`] and the workspace merges the file.

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
        ("Light", "light"),
        ("Dark", "dark")
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

/// `theme:toggle-light-dark` swaps `dark` and `light`, preserving other
/// keys. Under `system`, a missing key or any unrecognized value writes
/// nothing and answers `None`; the caller flips the session's scheme.
pub fn toggle_light_dark(appearance: &AppearanceConfig) -> Option<AppearanceConfig> {
    let swapped = match appearance.theme.as_deref() {
        Some("dark") => "light",
        Some("light") => "dark",
        _ => return None,
    };
    let mut next = appearance.clone();
    next.theme = Some(swapped.to_string());
    Some(next)
}

/// What a control draws.
#[derive(Debug, Clone, PartialEq)]
enum Control {
    None,
    Dropdown(SharedString),
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
    /// The modal itself, when nothing inside it is open.
    Modal,
}

/// Escape takes the innermost thing open: an open dropdown, then a page,
/// and the modal only when neither is. Obsidian's dropdown is a native
/// `<select>`, so Escape over an open option list closes the list and
/// leaves the dialog where it was.
fn escape_closes(popover_open: bool) -> Escape {
    if popover_open {
        Escape::Popover
    } else {
        Escape::Modal
    }
}

#[derive(Debug, Clone)]
struct Item {
    heading: bool,
    name: SharedString,
    desc: SharedString,
    control: Control,
    /// What a dropdown writes, for the controls that are not toggles.
    typed: Option<Typed>,
}

fn heading(name: &str) -> Item {
    Item {
        heading: true,
        name: name.to_string().into(),
        desc: "".into(),
        control: Control::None,
        typed: None,
    }
}

fn item(name: &str, desc: &str, control: Control) -> Item {
    Item {
        heading: false,
        name: name.to_string().into(),
        desc: desc.to_string().into(),
        control,
        typed: None,
    }
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
            items_scroll: ScrollHandle::new(),
            menu: None,
            corners: WindowCorners::default(),
        }
    }

    pub fn set_state(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        self.state = Some(state);
        cx.notify();
    }

    /// Shows the modal and takes focus so Escape closes it.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The view outlives the modal, so a dropdown or a text field left
        // open by the last session would come back expanded. Every way out
        // of the modal commits the field first, so there is nothing to write
        // here, only state to drop.
        self.menu = None;
        self.focus_tab(window, cx);
        cx.notify();
    }

    /// What the list has in hand, for the control socket's state dump. A
    /// dropdown and a text field are reported by the config key they write.
    pub fn describe(&self, rows: usize, _cx: &App) -> Value {
        let listed = self.described_rows(rows, _cx);
        let total = self.row_count();
        json!({
            "page": Value::Null,
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
                    Control::Dropdown(v) => ("dropdown", json!(v.as_ref())),
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

    fn close_action(&mut self, _: &Close, _window: &mut Window, cx: &mut Context<Self>) {
        match escape_closes(self.menu.is_some()) {
            Escape::Popover => {
                self.menu = None;
                cx.notify();
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
    // ----- the single list -----------------------------------------------

    /// Every setting, in one list: Appearance, then Editor.
    fn items(&self) -> Vec<Item> {
        let Some(state) = self.state.as_ref() else {
            return vec![item("No vault is open", "", Control::None)];
        };
        let mut items = vec![heading("Appearance")];
        items.extend(Self::appearance_items(state));
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

    fn render_control(
        &self,
        item: &Item,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match &item.control {
            Control::None => div().into_any_element(),
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
                    .child(self.render_control(item, theme, cx)),
            )
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        self.corners = WindowCorners::of(window);
        let items = self.items();
        let mut list = div()
            .id("settings-items")
            .overflow_y_scroll()
            .track_scroll(&self.items_scroll)
            .size_full();
        for item in &items {
            list = list.child(self.render_item(item, &theme, cx));
        }
        let body = list.into_any_element();
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
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.tokens.text_normal)
                                    .child("Settings"),
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
                            .child("Changes are written to the vault's .tft folder as you use them."),
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
    /// taking the modal with it.
    #[test]
    fn escape_closes_the_innermost_thing_open() {
        assert_eq!(escape_closes(false), Escape::Modal);
        assert_eq!(escape_closes(true), Escape::Popover);
    }

    /// `dark` and `light` swap while the file's other keys are kept; under
    /// `system`, no key, or an unrecognized value, nothing is written.
    #[test]
    fn toggle_light_dark_swaps_the_two_base_themes_and_leaves_system_alone() {
        let (dir, _) = vault();
        let path = dir.path().join(".tft/appearance.json");
        std::fs::write(
            &path,
            "{\n  \"accentColor\": \"#ff5721\",\n  \"theme\": \"dark\",\n  \"probeUnknown\": true\n}",
        )
        .unwrap();
        let state = VaultState::load(&dir.path().to_path_buf()).unwrap();
        let next = toggle_light_dark(&state.appearance()).expect("a swap");
        state
            .apply_setting(&SettingChange::Appearance(next))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"accentColor\": \"#ff5721\",\n  \"theme\": \"light\",\n  \"probeUnknown\": true\n}"
        );
        let back = toggle_light_dark(&state.appearance()).expect("a swap");
        assert_eq!(back.theme.as_deref(), Some("dark"));
        for theme in [Some("system"), None, Some("probe")] {
            let config = AppearanceConfig {
                theme: theme.map(str::to_string),
                ..AppearanceConfig::default()
            };
            assert_eq!(toggle_light_dark(&config), None, "{theme:?}");
        }
    }

    /// A vault to build the tabs against; `VaultState::load` wants a real
    /// folder.
    fn vault() -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".tft")).unwrap();
        std::fs::write(root.join("Note.md"), "# Note\n").unwrap();
        let state = VaultState::load(&root.to_path_buf()).unwrap();
        (dir, state)
    }
}
