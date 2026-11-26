//! GPUI Web entry flow for configuring Drive, opening a Vault and saving notes.

use crate::web_workspace::{WebWorkspace, WebWorkspaceEvent};
use gpui::{
    actions, div, prelude::*, px, App, Context, Entity, KeyBinding, Render, SharedString,
    Subscription, Window,
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tft_app::formats::config::AppearanceConfig;
use tft_app::text_input::TextInput;
use tft_app::theme::Theme;
use tft_app::ui;
use tft_app::{FileOperationRequest, NodeId, VaultData};
use tft_drive_web::{self as drive, IndexedFile, VaultIndex};

actions!(drive_app, [SaveCurrent]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", SaveCurrent, None),
        KeyBinding::new("ctrl-s", SaveCurrent, None),
    ]);
}

struct PendingVault {
    id: String,
    name: String,
    data: VaultData,
    entries: Vec<IndexedFile>,
    initial_node: Option<NodeId>,
    appearance: AppearanceConfig,
}

struct CreatedDriveNote {
    id: String,
    name: String,
    contents: String,
}

pub struct DriveApp {
    client_id: Entity<TextInput>,
    api_key: Entity<TextInput>,
    status: String,
    signed_in: bool,
    busy: bool,
    pending: Option<PendingVault>,
    pending_note: Option<CreatedDriveNote>,
    workspace: Option<Entity<WebWorkspace>>,
    workspace_subscription: Option<Subscription>,
    vault_id: String,
    vault_epoch: u64,
    vault_name: String,
    entries: Vec<IndexedFile>,
    last_saved: String,
    save_generation: HashMap<NodeId, u64>,
    saving_nodes: HashSet<NodeId>,
    pending_saves: HashMap<NodeId, (u64, String)>,
    dirty_nodes: HashMap<NodeId, String>,
    close_when_saved: bool,
}

impl DriveApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (client_id, api_key, _) = drive::config();
        let client_id_input = cx.new(|cx| TextInput::new("Google OAuth Client ID", cx));
        let api_key_input = cx.new(|cx| TextInput::new("Google Picker API key", cx));
        client_id_input.update(cx, |input, cx| input.set_text(client_id, cx));
        api_key_input.update(cx, |input, cx| input.set_text(api_key, cx));
        Self {
            client_id: client_id_input,
            api_key: api_key_input,
            status: "Configure Google Drive access to open a Vault.".into(),
            signed_in: false,
            busy: false,
            pending: None,
            pending_note: None,
            workspace: None,
            workspace_subscription: None,
            vault_id: String::new(),
            vault_epoch: 0,
            vault_name: String::new(),
            entries: Vec::new(),
            last_saved: String::new(),
            save_generation: HashMap::new(),
            saving_nodes: HashSet::new(),
            pending_saves: HashMap::new(),
            dirty_nodes: HashMap::new(),
            close_when_saved: false,
        }
    }

    fn configure(&mut self, cx: &mut Context<Self>) {
        let client_id = self.client_id.read(cx).text().trim().to_string();
        let api_key = self.api_key.read(cx).text().trim().to_string();
        if client_id.is_empty() {
            self.status = "Enter the OAuth Client ID first.".into();
        } else {
            drive::save_config(&client_id, &api_key);
            self.status = "Drive configuration saved in this browser.".into();
        }
        cx.notify();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let client_id = self.client_id.read(cx).text().trim().to_string();
        let api_key = self.api_key.read(cx).text().trim().to_string();
        if client_id.is_empty() {
            self.status = "Enter the OAuth Client ID and save the configuration first.".into();
            cx.notify();
            return;
        }
        drive::save_config(&client_id, &api_key);
        // Call GIS inside the click callback to retain the browser's transient
        // activation. Only the resulting Promise is awaited asynchronously.
        let promise = match drive::begin_sign_in(&client_id) {
            Ok(promise) => promise,
            Err(error) => {
                self.status = format!("Sign-in could not start: {error}");
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.status = "Waiting for Google sign-in…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::finish_sign_in(promise).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(()) => {
                        app.signed_in = true;
                        app.status = "Signed in. Choose a Drive folder to open as a Vault.".into();
                    }
                    Err(error) => {
                        app.status = format!("Sign-in failed: {error}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn pick_vault(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in {
            self.status = "Sign in before choosing a Vault.".into();
            cx.notify();
            return;
        }
        let (_, api_key, app_id) = drive::config();
        // As with sign-in, open the Picker synchronously from its click event.
        let promise = drive::begin_pick_folder(&api_key, &app_id);
        self.busy = true;
        self.status = "Opening Google Drive folder picker…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let (id, name) = drive::finish_pick_folder(promise).await?;
                load_vault(id, name).await
            }
            .await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(vault) => {
                        app.status = format!(
                            "Opened {} · {} Markdown files loaded.",
                            vault.name,
                            vault
                                .entries
                                .iter()
                                .filter(|entry| entry.kind == tft_formats::FileKind::Markdown)
                                .count()
                        );
                        app.pending = Some(vault);
                    }
                    Err(error) => app.status = format!("Could not open Vault: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_last_vault(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in {
            self.status = "Sign in before reopening a Vault.".into();
            cx.notify();
            return;
        }
        let Some((id, name)) = drive::last_vault() else {
            self.status = "There is no previously opened Vault in this browser.".into();
            cx.notify();
            return;
        };
        self.load_vault_id(id, name, cx);
    }

    fn load_vault_id(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        self.busy = true;
        self.status = format!("Loading {name}…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = load_vault(id, name).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(vault) => {
                        app.status = format!(
                            "Opened {} · {} Markdown files loaded.",
                            vault.name,
                            vault
                                .entries
                                .iter()
                                .filter(|entry| entry.kind == tft_formats::FileKind::Markdown)
                                .count()
                        );
                        app.pending = Some(vault);
                    }
                    Err(error) => app.status = format!("Could not open Vault: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_current(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let Some((node, contents)) = workspace.read(cx).current_document(cx) else {
            self.status = "Choose a Markdown file before saving.".into();
            cx.notify();
            return;
        };
        let Some(file) = self.entries.iter().find(|file| file.node == node) else {
            self.status = "The active note is not a Drive file.".into();
            cx.notify();
            return;
        };
        if file.kind != tft_formats::FileKind::Markdown {
            self.status = "Only Markdown notes can be saved from this Web editor.".into();
            cx.notify();
            return;
        }
        let generation = self.next_save_generation(node);
        self.pending_saves.remove(&node);
        self.dirty_nodes.insert(node, contents.clone());
        self.save_to_drive(node, contents, generation, cx);
    }

    fn create_note(&mut self, parent_path: &str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !parent_path.is_empty() {
            self.status =
                "Folder-scoped note creation is not wired to the Drive adapter yet.".into();
            cx.notify();
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let mut name = drive::ask("New Markdown note name", "Untitled");
        name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        if !name.to_ascii_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        if let Err(error) = workspace.read(cx).can_create_root_note(&name) {
            self.status = error;
            cx.notify();
            return;
        }
        let title = name.strip_suffix(".md").unwrap_or(&name).to_owned();
        let contents = format!("# {title}\n\n");
        let parent = self.vault_id.clone();
        self.busy = true;
        self.status = format!("Creating {name} in Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::create_note_with_id(&parent, &name).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(id) => {
                        app.pending_note = Some(CreatedDriveNote {
                            id,
                            name: name.clone(),
                            contents,
                        });
                        app.status = format!("Created {} in Drive.", name);
                    }
                    Err(error) => app.status = format!("Could not create note: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn next_save_generation(&mut self, node: NodeId) -> u64 {
        let generation = self.save_generation.entry(node).or_default();
        *generation = generation.wrapping_add(1);
        *generation
    }

    fn schedule_auto_save(&mut self, node: NodeId, contents: String, cx: &mut Context<Self>) {
        let generation = self.next_save_generation(node);
        self.dirty_nodes.insert(node, contents.clone());
        self.status = "Unsaved changes · saving shortly…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |app, cx| {
                if app.save_generation.get(&node) == Some(&generation) {
                    app.save_to_drive(node, contents, generation, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn save_to_drive(
        &mut self,
        node: NodeId,
        contents: String,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.saving_nodes.contains(&node) {
            self.pending_saves.insert(node, (generation, contents));
            return;
        }
        let Some(file) = self.entries.iter().find(|file| file.node == node) else {
            self.status = "The edited note is no longer in this Drive Vault.".into();
            cx.notify();
            return;
        };
        let id = file.id.clone();
        let etag = file.etag.clone();
        let path = file.path.clone();
        let vault_epoch = self.vault_epoch;
        self.saving_nodes.insert(node);
        self.status = format!("Saving {path} to Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::write_file(&id, &contents, etag.as_deref()).await;
            this.update(cx, |app, cx| {
                if app.vault_epoch != vault_epoch {
                    return;
                }
                app.saving_nodes.remove(&node);
                let current_generation =
                    app.save_generation.get(&node).copied().unwrap_or_default();
                let mut saved = false;
                match result {
                    Ok(new_etag) => {
                        if let Some(file) = app.entries.iter_mut().find(|file| file.node == node) {
                            if new_etag.is_some() {
                                file.etag = new_etag;
                            }
                        }
                        if let Some(workspace) = app.workspace.as_ref() {
                            workspace.update(cx, |workspace, cx| {
                                workspace.mark_saved(node, contents.clone(), cx);
                            });
                        }
                        app.last_saved = contents.clone();
                        saved = true;
                    }
                    Err(error) => app.status = format!("Save failed: {error}"),
                }
                let pending = app.pending_saves.remove(&node);
                if let Some((pending_generation, pending_contents)) = pending {
                    if saved && pending_generation == current_generation {
                        app.save_to_drive(node, pending_contents, pending_generation, cx);
                        return;
                    }
                }
                if saved && generation == current_generation {
                    app.status = "Saved the Markdown source to Google Drive.".into();
                    if app.dirty_nodes.get(&node) == Some(&contents) {
                        app.dirty_nodes.remove(&node);
                    }
                } else if !saved {
                    app.close_when_saved = false;
                }
                if app.close_when_saved && app.dirty_nodes.is_empty() {
                    app.finish_close_vault(cx);
                    return;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn close_vault(&mut self, cx: &mut Context<Self>) {
        if !self.dirty_nodes.is_empty() {
            self.close_when_saved = true;
            let dirty = self
                .dirty_nodes
                .iter()
                .map(|(node, contents)| (*node, contents.clone()))
                .collect::<Vec<_>>();
            for (node, contents) in dirty {
                let generation = self.next_save_generation(node);
                self.pending_saves.remove(&node);
                self.save_to_drive(node, contents, generation, cx);
            }
            self.status = "Saving edited notes before switching Vault…".into();
            cx.notify();
            return;
        }
        self.finish_close_vault(cx);
    }

    fn finish_close_vault(&mut self, cx: &mut Context<Self>) {
        self.vault_epoch = self.vault_epoch.wrapping_add(1);
        self.workspace = None;
        self.workspace_subscription = None;
        self.entries.clear();
        self.save_generation.clear();
        self.saving_nodes.clear();
        self.pending_saves.clear();
        self.dirty_nodes.clear();
        self.close_when_saved = false;
        self.pending_note = None;
        self.vault_id.clear();
        self.vault_name.clear();
        cx.set_global(Theme::from_appearance(
            &AppearanceConfig::default(),
            drive::system_is_dark(),
        ));
        self.status = "Choose a Drive folder to open another Vault.".into();
        cx.notify();
    }
}

async fn load_vault(id: String, name: String) -> Result<PendingVault, String> {
    let mut index: VaultIndex = drive::index_vault(&id).await?;
    let appearance = drive::read_appearance_config(&id).await.unwrap_or_default();
    let loaded = drive::read_markdown_strict(&index.entries).await?;
    for (node, contents) in loaded {
        index.data.set_text(node, contents);
    }
    let initial_node = index
        .entries
        .iter()
        .find(|entry| entry.kind == tft_formats::FileKind::Markdown)
        .map(|entry| entry.node);
    drive::remember_vault(&id, &name);
    Ok(PendingVault {
        id,
        name,
        data: index.data,
        entries: index.entries,
        initial_node,
        appearance,
    })
}

impl Render for DriveApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.workspace.is_none() {
            if let Some(vault) = self.pending.take() {
                self.vault_epoch = self.vault_epoch.wrapping_add(1);
                cx.set_global(Theme::from_appearance(
                    &vault.appearance,
                    drive::system_is_dark(),
                ));
                self.vault_id = vault.id;
                self.vault_name = vault.name;
                self.entries = vault.entries;
                let vault_name = self.vault_name.clone();
                let workspace = cx.new(|cx| {
                    let mut workspace =
                        WebWorkspace::from_vault(vault.data, vault.initial_node, window, cx);
                    workspace.set_vault_name(vault_name, cx);
                    workspace.set_drive_controls(true, cx);
                    workspace
                });
                self.workspace_subscription = Some(cx.subscribe(
                    &workspace,
                    |this, _, event: &WebWorkspaceEvent, cx| match event {
                        WebWorkspaceEvent::Edited { node, contents } => {
                            this.schedule_auto_save(*node, contents.clone(), cx)
                        }
                        WebWorkspaceEvent::FileOperationRequested(
                            FileOperationRequest::CreateNote { parent },
                        ) => this.create_note(parent, cx),
                        WebWorkspaceEvent::SaveRequested => this.save_current(cx),
                        WebWorkspaceEvent::SwitchVaultRequested => this.close_vault(cx),
                    },
                ));
                self.workspace = Some(workspace);
            }
        }

        if let Some(note) = self.pending_note.take() {
            if let Some(workspace) = self.workspace.clone() {
                let result = workspace.update(cx, |workspace, cx| {
                    workspace.insert_drive_note(
                        note.id.clone(),
                        note.name.clone(),
                        note.contents.clone(),
                        window,
                        cx,
                    )
                });
                match result {
                    Ok(node) => {
                        self.entries.push(IndexedFile {
                            node,
                            path: note.name.clone(),
                            kind: tft_formats::FileKind::Markdown,
                            id: note.id,
                            etag: None,
                        });
                        self.status = format!("Created and opened {}.", note.name);
                    }
                    Err(error) => {
                        self.status = format!("Could not open new note: {error}");
                    }
                }
            }
        }

        #[cfg(feature = "drive-smoke-test")]
        self.publish_test_state(cx);

        let theme = cx.global::<Theme>().clone();
        let tokens = &theme.tokens;
        if let Some(workspace) = self.workspace.as_ref() {
            let workspace = workspace.clone();
            return div()
                .size_full()
                .bg(tokens.background_primary)
                .text_color(tokens.text_normal)
                .on_action(cx.listener(|this, _: &SaveCurrent, _, cx| this.save_current(cx)))
                .child(workspace);
        }

        let client_id = self.client_id.clone();
        let api_key = self.api_key.clone();
        let status = self.status.clone();
        let busy = self.busy;
        let signed_in = self.signed_in;
        let last_vault = drive::last_vault();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(tokens.background_primary)
            .text_color(tokens.text_normal)
            .child(
                div()
                    .w(px(560.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(tokens.background_modifier_border)
                    .bg(tokens.background_secondary)
                    .p_6()
                    .child(div().text_size(px(24.)).child("TFT"))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(tokens.text_muted)
                            .child("Open and edit your Markdown Vault from Google Drive."),
                    )
                    .child(client_id)
                    .child(api_key)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button("Save configuration", &theme)
                                    .on_click(cx.listener(|this, _, _, cx| this.configure(cx))),
                            )
                            .child(button("Sign in with Google", &theme).when(!busy, |button| {
                                button.on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)))
                            })),
                    )
                    .when(signed_in, |panel| {
                        panel.child(
                            div()
                                .flex()
                                .gap_2()
                                .child(button("Choose Drive folder", &theme).when(
                                    !busy,
                                    |button| {
                                        button.on_click(
                                            cx.listener(|this, _, _, cx| this.pick_vault(cx)),
                                        )
                                    },
                                ))
                                .when(last_vault.is_some(), |row| {
                                    row.child(button("Reopen last Vault", &theme).when(
                                        !busy,
                                        |button| {
                                            button.on_click(cx.listener(|this, _, _, cx| {
                                                this.open_last_vault(cx)
                                            }))
                                        },
                                    ))
                                }),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(tokens.text_muted)
                            .child(SharedString::from(status)),
                    ),
            )
    }
}

#[cfg(feature = "drive-smoke-test")]
impl DriveApp {
    fn publish_test_state(&self, cx: &App) {
        use wasm_bindgen::JsValue;
        let state = js_sys::Object::new();
        let path = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.read(cx).current_path().map(str::to_owned))
            .unwrap_or_default();
        let active_text = self
            .workspace
            .as_ref()
            .and_then(|workspace| {
                workspace
                    .read(cx)
                    .current_document(cx)
                    .map(|(_, text)| text)
            })
            .unwrap_or_default();
        let values = [
            ("signedIn", JsValue::from_bool(self.signed_in)),
            ("status", JsValue::from_str(&self.status)),
            ("vaultName", JsValue::from_str(&self.vault_name)),
            ("activePath", JsValue::from_str(&path)),
            ("activeText", JsValue::from_str(&active_text)),
            ("lastSaved", JsValue::from_str(&self.last_saved)),
            ("themeDark", JsValue::from_bool(cx.global::<Theme>().dark)),
        ];
        for (key, value) in values {
            let _ = js_sys::Reflect::set(&state, &JsValue::from_str(key), &value);
        }
        let _ = js_sys::Reflect::set(
            &js_sys::global(),
            &JsValue::from_str("__tftDriveAppState"),
            &state,
        );
    }
}

pub(super) fn button(label: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    ui::action_button(label, theme)
}
