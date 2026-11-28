//! Desktop vault switcher UI and its window/file-picker actions.

use super::Workspace;
use crate::assets::icon;
use crate::theme::{Theme, FONT_UI_SMALL};
use gpui::{anchored, deferred, div, point, prelude::*, px, Context, Window};
use std::path::{Path, PathBuf};
use tft_vault::VaultRegistry;

#[derive(Default)]
pub(super) struct VaultSwitcherState {
    menu_open: bool,
    theme_menu_open: bool,
    hovered: bool,
}

impl VaultSwitcherState {
    pub(super) fn is_menu_open(&self) -> bool {
        self.menu_open
    }
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Workspace {
    /// The active vault name toggles a menu of remembered vaults and actions.
    pub(super) fn toggle_vault_menu(&mut self, cx: &mut Context<Self>) {
        self.vault_switcher_state.menu_open = !self.vault_switcher_state.menu_open;
        cx.notify();
    }

    pub(super) fn open_theme_menu(&mut self, cx: &mut Context<Self>) {
        self.vault_switcher_state.menu_open = false;
        self.vault_switcher_state.theme_menu_open = true;
        cx.notify();
    }

    fn toggle_theme_menu(&mut self, cx: &mut Context<Self>) {
        self.vault_switcher_state.theme_menu_open = !self.vault_switcher_state.theme_menu_open;
        cx.notify();
    }

    /// Puts `root` at the head of the switcher's list. The list lives
    /// outside every vault; a machine with nowhere to keep it has no list.
    pub(super) fn remember_vault(&mut self, root: &Path, cx: &mut Context<Self>) {
        let outcome = (|| -> std::io::Result<()> {
            let Some(mut registry) = VaultRegistry::open()? else {
                return Ok(());
            };
            registry.record_open(root, now_millis());
            registry.save()
        })();
        if let Err(err) = outcome {
            self.report(
                format!("Could not record the vault for the switcher: {err}"),
                cx,
            );
        }
    }

    /// Removes a remembered entry only; its folder and the currently open vault stay untouched.
    fn forget_remembered_vault(&mut self, id: &str, cx: &mut Context<Self>) {
        let result = (|| -> std::io::Result<()> {
            let Some(mut registry) = VaultRegistry::open()? else {
                return Ok(());
            };
            if registry.forget(id) {
                registry.save()?;
            }
            Ok(())
        })();
        if let Err(err) = result {
            self.report(
                format!("Could not remove the vault from the list: {err}"),
                cx,
            );
        }
        cx.notify();
    }

    /// "Open vault...": asks the desktop for a folder and opens it as the vault;
    /// cancelling changes nothing.
    pub(super) fn open_another_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.vault_switcher_state.menu_open = false;
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add vault".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let root = match picked.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.remove(0),
                Ok(Ok(_)) => return,
                Ok(Err(err)) => {
                    this.update(cx, |ws, cx| {
                        ws.report(format!("Could not open the folder dialog: {err}"), cx)
                    })
                    .ok();
                    return;
                }
                Err(_) => return,
            };
            this.update_in(cx, |ws, window, cx| ws.load_vault(root, window, cx))
                .ok();
        })
        .detach();
    }

    /// The left sidebar's footer: vault switcher and color scheme selector.
    pub(super) fn render_vault_profile(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let menu = self.render_vault_menu(theme, window, cx);
        let theme_menu = self.render_theme_menu(theme, cx);
        crate::ui::workspace_dock_footer(theme)
            .child(
                crate::ui::workspace_dock_profile_button(
                    "vault-switcher",
                    self.vault_session.name.clone(),
                    theme,
                )
                .when(self.vault_switcher_state.menu_open, |d| {
                    d.bg(theme.tokens.background_modifier_hover)
                })
                .on_hover(cx.listener(|this, over: &bool, _, _| {
                    this.vault_switcher_state.hovered = *over;
                }))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_vault_menu(cx)))
                // The menu's anchor is this box: an absolute element left to the
                // row's centring would hang from its middle.
                .child(div().absolute().top_0().left_0().size_0().children(menu)),
            )
            .child(
                crate::ui::icon_button("vault-settings", "settings", theme)
                    .tooltip(crate::ui::tooltip("Color scheme"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_theme_menu(cx)))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_0()
                            .children(theme_menu),
                    ),
            )
    }

    fn render_theme_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.vault_switcher_state.theme_menu_open {
            return None;
        }
        let selected = self
            .vault_session
            .state
            .as_ref()
            .and_then(|state| state.appearance().theme.clone())
            .unwrap_or_else(|| "system".into());
        let options = [("system", "System"), ("light", "Light"), ("dark", "Dark")];
        let mut menu = div()
            .id("color-scheme-menu")
            .flex()
            .flex_col()
            .min_w(px(160.))
            .py_1()
            .bg(theme.tokens.background_secondary)
            .border_1()
            .border_color(theme.tokens.background_modifier_border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.tokens.text_normal)
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.vault_switcher_state.theme_menu_open = false;
                cx.notify();
            }));
        for (scheme, label) in options {
            let is_selected = selected == scheme;
            menu = menu.child(
                div()
                    .id(match scheme {
                        "system" => "color-scheme-system",
                        "light" => "color-scheme-light",
                        _ => "color-scheme-dark",
                    })
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(6.))
                    .mx_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|d| d.bg(theme.tokens.background_modifier_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.vault_switcher_state.theme_menu_open = false;
                        this.set_color_scheme(scheme, cx);
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .children(
                                is_selected.then(|| icon("check", 14., theme.tokens.text_muted)),
                            ),
                    )
                    .child(label),
            );
        }
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(point(px(0.), px(-6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn render_vault_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.vault_switcher_state.menu_open {
            return None;
        }
        let current = self.vault_session.state.as_ref().map(|s| s.root.clone());
        let registry = VaultRegistry::open().ok().flatten();
        let known: Vec<(String, String, PathBuf)> = registry
            .as_ref()
            .map(|r| {
                r.vaults()
                    .into_iter()
                    .filter(|(_, e)| e.path.is_dir())
                    .map(|(id, e)| (id.to_string(), e.name(), e.path.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let item = |id: gpui::ElementId, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(5.))
                .rounded(px(4.))
                .mx_1()
                .cursor_pointer()
                .hover(|s| s.bg(theme.tokens.background_modifier_hover))
        };
        let check = |on: bool, theme: &Theme| {
            div()
                .w(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .children(on.then(|| icon("check", 14., theme.tokens.text_muted)))
        };
        // A machine with dozens of vaults has a menu taller than the window, so cap it
        // at the window and scroll, as Obsidian's `.menu-scroll` does.
        let tallest = (window.viewport_size().height - px(96.)).max(px(160.));
        let mut list = div()
            .id("vault-menu")
            .flex()
            .flex_col()
            .min_w(px(240.))
            .max_w(px(360.))
            .max_h(tallest)
            .overflow_y_scroll()
            .py_1()
            .bg(theme.tokens.background_secondary)
            .border_1()
            .border_color(theme.tokens.background_modifier_border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.tokens.text_normal)
            .occlude()
            // A press on the switcher row is a press outside the menu, and the click
            // that ends it toggles; the row's own handler closes when the press is its.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.vault_switcher_state.hovered {
                    return;
                }
                this.vault_switcher_state.menu_open = false;
                cx.notify();
            }));
        if known.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py(px(5.))
                    .text_color(theme.tokens.text_faint)
                    .child("No vaults opened yet"),
            );
        }
        for (ix, (id, name, path)) in known.into_iter().enumerate() {
            let showing = current.as_deref() == Some(path.as_path());
            let tooltip = crate::ui::tooltip(path.display().to_string());
            let forget_id = id.clone();
            list = list.child(
                item(("vault-menu-item", ix).into(), theme)
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.vault_switcher_state.menu_open = false;
                        if showing {
                            cx.notify();
                        } else if path.is_dir() {
                            this.load_vault(path.clone(), window, cx);
                        } else {
                            this.report(format!("{} is not a folder any more", path.display()), cx);
                        }
                    }))
                    .child(check(showing, theme))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name),
                    )
                    .child(
                        div()
                            .id(("vault-menu-remove", ix))
                            .flex_shrink_0()
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_color(theme.tokens.text_muted)
                            .hover(|s| {
                                s.bg(theme.tokens.background_modifier_hover)
                                    .text_color(theme.canvas.presets[0])
                            })
                            .tooltip(crate::ui::tooltip("Remove from list"))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.vault_switcher_state.menu_open = false;
                                this.forget_remembered_vault(&forget_id, cx);
                            }))
                            .child(icon("x", 14., theme.tokens.icon_color)),
                    ),
            );
        }
        // Add another folder as a vault without leaving the switcher.
        list = list
            .child(
                div()
                    .my_1()
                    .h(px(1.))
                    .bg(theme.tokens.background_modifier_border),
            )
            .child(
                item("vault-menu-add".into(), theme)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_another_vault(window, cx)),
                    )
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .child(icon("folder-plus", 14., theme.tokens.text_muted)),
                    )
                    .child("Add vault"),
            );
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(point(px(0.), px(-6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
