//! GPUI Web entry flow for configuring Drive, opening a Vault and saving notes.

use crate::web_workspace::{WebWorkspace, WebWorkspaceEvent};
use gpui::{div, prelude::*, px, App, Context, Entity, Render, SharedString, Subscription, Window};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tft_app::formats::config::{
    AppConfig, AppearanceConfig, GraphConfig, Hotkeys, PagePreviewConfig, SwitcherConfig,
};
use tft_app::prompt::{Prompt, PromptEvent, PromptSpec};
use tft_app::text_input::TextInput;
use tft_app::theme::Theme;
use tft_app::ui;
use tft_app::{FileOperationRequest, NodeId, VaultData};
use tft_drive_web::{self as drive, IndexedFile, VaultIndex};

/// TFT's default public OAuth client, used when the browser has no saved
/// configuration. It is a public identifier, not a secret.
const DEFAULT_CLIENT_ID: &str =
    "130397210046-intlp507spke9u1b659ef1ccp85s270v.apps.googleusercontent.com";

struct PendingVault {
    id: String,
    name: String,
    data: VaultData,
    entries: Vec<IndexedFile>,
    initial_node: Option<NodeId>,
    appearance: AppearanceConfig,
    app_config: AppConfig,
    switcher_config: SwitcherConfig,
    page_preview_config: PagePreviewConfig,
    graph_config: GraphConfig,
    graph_config_disk_json: Option<String>,
    hotkeys: Hotkeys,
    workspace_state: Option<String>,
}

struct CreatedDriveNote {
    id: String,
    name: String,
    path: String,
    contents: String,
    kind: tft_formats::FileKind,
}

struct CreatedDriveFolder {
    id: String,
    name: String,
    path: String,
}

#[derive(Clone)]
enum PendingDrivePrompt {
    CreateNote {
        parent: String,
    },
    CreateCanvas {
        parent: String,
    },
    CreateFolder {
        parent: String,
    },
    CanvasEdgeLabel {
        file: NodeId,
        edge_id: String,
    },
    RenameInput {
        from: String,
    },
    MoveInput {
        from: String,
    },
    RenameHeading {
        path: String,
        line: u32,
    },
    DuplicateFile {
        source_path: String,
        source_id: String,
        parent_path: String,
        parent_id: String,
        kind: tft_formats::FileKind,
    },
    DeleteFile {
        path: String,
        node: NodeId,
        id: String,
    },
    Rename {
        from: String,
        to: String,
        rewrites: Vec<tft_cache::Rewrite>,
    },
    MoveConfirm {
        from: String,
        to: String,
        destination: String,
        file_id: String,
        source_parent_id: String,
        destination_parent_id: String,
        rewrites: Vec<tft_cache::Rewrite>,
    },
}

pub struct DriveApp {
    client_id: Entity<TextInput>,
    api_key: Entity<TextInput>,
    status: String,
    signed_in: bool,
    busy: bool,
    pending: Option<PendingVault>,
    pending_note: Option<CreatedDriveNote>,
    pending_folder: Option<CreatedDriveFolder>,
    workspace: Option<Entity<WebWorkspace>>,
    workspace_subscription: Option<Subscription>,
    prompt: Entity<Prompt>,
    _prompt_subscription: Subscription,
    prompt_open: bool,
    pending_drive_prompt: Option<PendingDrivePrompt>,
    pending_rename_start: Option<(String, String)>,
    pending_move_start: Option<(String, String)>,
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
    pending_workspace_state: Option<String>,
    workspace_saving: bool,
    vault_workspace_epoch: u64,
    last_graph_config: Option<String>,
    pending_heading_rename: Option<(String, u32, String)>,
}

impl DriveApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (configured_client_id, api_key, _) = drive::config();
        let client_id = if configured_client_id.trim().is_empty() {
            DEFAULT_CLIENT_ID.to_string()
        } else {
            configured_client_id
        };
        let client_id_input = cx.new(|cx| TextInput::new("Google OAuth Client ID", cx));
        let api_key_input = cx.new(|cx| TextInput::new("Google Picker API key", cx));
        client_id_input.update(cx, |input, cx| input.set_text(client_id.clone(), cx));
        api_key_input.update(cx, |input, cx| input.set_text(api_key, cx));
        let prompt = cx.new(Prompt::new);
        let prompt_subscription =
            cx.subscribe(&prompt, |this, _, event: &PromptEvent, cx| match event {
                PromptEvent::Submit(name) => this.submit_drive_prompt(name.clone(), cx),
                PromptEvent::Cancel => {
                    this.prompt_open = false;
                    this.pending_drive_prompt = None;
                    cx.notify();
                }
                PromptEvent::Secondary => this.submit_drive_secondary(cx),
                _ => {}
            });
        let mut app = Self {
            client_id: client_id_input,
            api_key: api_key_input,
            status: "Configure Google Drive access to open a Vault.".into(),
            signed_in: false,
            busy: false,
            pending: None,
            pending_note: None,
            pending_folder: None,
            workspace: None,
            workspace_subscription: None,
            prompt,
            _prompt_subscription: prompt_subscription,
            prompt_open: false,
            pending_drive_prompt: None,
            pending_rename_start: None,
            pending_move_start: None,
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
            pending_workspace_state: None,
            workspace_saving: false,
            vault_workspace_epoch: 0,
            last_graph_config: None,
            pending_heading_rename: None,
        };
        if !client_id.trim().is_empty() {
            app.busy = true;
            app.status = "Restoring your Google session…".into();
            cx.spawn(async move |this, cx| {
                let result = async {
                    drive::try_resume_sign_in(&client_id).await?;
                    match drive::last_vault() {
                        Some((id, name)) => load_vault(id, name).await.map(Some),
                        None => Ok(None),
                    }
                }
                .await;
                this.update(cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(Some(vault)) => {
                            app.signed_in = true;
                            app.status = format!("Restored {}.", vault.name);
                            app.pending = Some(vault);
                        }
                        Ok(None) => {
                            app.signed_in = true;
                            app.status = "Signed in. Choose a Drive folder to open as a Vault.".into();
                        }
                        Err(error) => {
                            app.status = format!(
                                "Could not resume the Google session: {error}. Reconnect to continue to your last Vault."
                            );
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        app
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let client_id = self.client_id.read(cx).text().trim().to_string();
        let api_key = self.api_key.read(cx).text().trim().to_string();
        if client_id.is_empty() {
            self.status = "Enter the OAuth Client ID and save the configuration first.".into();
            cx.notify();
            return;
        }
        let config_persisted = drive::save_config(&client_id, &api_key);
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
        self.status = if config_persisted {
            "Waiting for Google sign-in…".into()
        } else {
            "Waiting for Google sign-in… Drive settings could not be saved in browser storage."
                .into()
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::finish_sign_in(promise).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(()) => {
                        app.signed_in = true;
                        if drive::last_vault().is_some() {
                            app.open_last_vault(cx);
                        } else {
                            // First run after sign-in: go straight to Picker.
                            app.pick_vault(cx);
                        }
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

    fn schedule_workspace_save(&mut self, state: String, cx: &mut Context<Self>) {
        if !self.vault_id.is_empty() && !drive::save_local_workspace_state(&self.vault_id, &state) {
            self.status =
                "Browser storage is unavailable; workspace recovery relies on Google Drive.".into();
            cx.notify();
        }
        self.pending_workspace_state = Some(state);
        if self.workspace_saving {
            return;
        }
        let epoch = self.vault_workspace_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(450))
                .await;
            this.update(cx, |app, cx| {
                if app.vault_workspace_epoch != epoch || app.workspace_saving {
                    return;
                }
                app.start_workspace_save(cx);
            })
            .ok();
        })
        .detach();
    }

    fn start_workspace_save(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.pending_workspace_state.take() else {
            return;
        };
        if self.vault_id.is_empty() {
            return;
        }
        let root_id = self.vault_id.clone();
        let saved_state = state.clone();
        let graph_config = serde_json::from_str::<serde_json::Value>(&state)
            .ok()
            .and_then(|value| value.get("graphConfig").cloned())
            .and_then(|value| serde_json::from_value::<GraphConfig>(value).ok());
        let graph_config_json = graph_config
            .as_ref()
            .and_then(|config| serde_json::to_string(config).ok());
        let save_graph_config = graph_config_json
            .as_ref()
            .is_some_and(|config| self.last_graph_config.as_ref() != Some(config));
        let has_graph_config = graph_config.is_some();
        let graph_config_to_save = save_graph_config.then_some(graph_config).flatten();
        let graph_config_json_to_save = save_graph_config.then_some(graph_config_json).flatten();
        let epoch = self.vault_workspace_epoch;
        self.workspace_saving = true;
        cx.spawn(async move |this, cx| {
            let graph_result = if let Some(config) = graph_config_to_save.as_ref() {
                Some(drive::write_graph_config(&root_id, config).await)
            } else {
                None
            };
            let graph_saved = graph_result.as_ref().is_none_or(Result::is_ok);
            let state_for_drive = if graph_saved && has_graph_config {
                let mut value = serde_json::from_str::<serde_json::Value>(&state).ok();
                if let Some(object) = value.as_mut().and_then(serde_json::Value::as_object_mut) {
                    object.remove("graphConfig");
                }
                value
                    .and_then(|value| serde_json::to_string(&value).ok())
                    .unwrap_or_else(|| state.clone())
            } else {
                state.clone()
            };
            let result = drive::write_web_workspace_state(&root_id, &state_for_drive).await;
            this.update(cx, |app, cx| {
                if app.vault_workspace_epoch != epoch {
                    return;
                }
                app.workspace_saving = false;
                match result {
                    Ok(()) => {
                        if graph_saved {
                            drive::clear_local_workspace_state(&root_id, &saved_state);
                        }
                        if let (Some(config_json), Some(Ok(()))) =
                            (graph_config_json_to_save, graph_result.as_ref())
                        {
                            app.last_graph_config = Some(config_json);
                        }
                        match graph_result {
                            Some(Err(error)) => {
                                app.status = format!(
                                    "Workspace saved, but Graph settings could not be saved: {error}"
                                );
                            }
                            Some(Ok(())) => {
                                app.status = "Workspace and Graph settings saved to Google Drive.".into();
                            }
                            None => app.status = "Workspace saved to Google Drive.".into(),
                        }
                    }
                    Err(error) => {
                        app.status = format!("Workspace state could not be saved: {error}");
                    }
                }
                if app.pending_workspace_state.is_some() {
                    app.start_workspace_save(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    fn submit_drive_prompt(&mut self, name: String, cx: &mut Context<Self>) {
        self.prompt_open = false;
        match self.pending_drive_prompt.clone() {
            Some(PendingDrivePrompt::CreateNote { parent }) => {
                self.create_note_named(&parent, name, cx)
            }
            Some(PendingDrivePrompt::CreateCanvas { parent }) => {
                self.create_canvas_named(&parent, name, cx)
            }
            Some(PendingDrivePrompt::CreateFolder { parent }) => {
                self.create_folder_named(&parent, name, cx)
            }
            Some(PendingDrivePrompt::CanvasEdgeLabel { file, edge_id }) => {
                if let Some(workspace) = self.workspace.as_ref() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.set_canvas_edge_label(file, edge_id, name, cx)
                    });
                }
                self.pending_drive_prompt = None;
            }
            Some(PendingDrivePrompt::RenameInput { from }) => {
                self.pending_drive_prompt = None;
                let name = name.trim();
                if !name.is_empty() {
                    let (parent, file_name) = from.rsplit_once('/').unwrap_or(("", &from));
                    let renamed = match file_name.rsplit_once('.') {
                        Some((_, extension)) => format!("{name}.{extension}"),
                        None => name.to_owned(),
                    };
                    let to = if parent.is_empty() {
                        renamed
                    } else {
                        format!("{parent}/{renamed}")
                    };
                    if to != from {
                        self.pending_rename_start = Some((from, to));
                    }
                }
            }
            Some(PendingDrivePrompt::MoveInput { from }) => {
                self.pending_drive_prompt = None;
                self.pending_move_start = Some((from, name.trim().to_owned()));
            }
            Some(PendingDrivePrompt::RenameHeading { path, line }) => {
                self.pending_drive_prompt = None;
                self.pending_heading_rename = Some((path, line, name.trim().to_owned()));
            }
            Some(PendingDrivePrompt::DuplicateFile {
                source_path,
                source_id,
                parent_path,
                parent_id,
                kind,
            }) => self.duplicate_drive_file(
                source_path,
                source_id,
                parent_path,
                parent_id,
                kind,
                name,
                cx,
            ),
            Some(PendingDrivePrompt::DeleteFile { path, node, id }) => {
                self.pending_drive_prompt = None;
                if self.dirty_nodes.contains_key(&node)
                    || self.saving_nodes.contains(&node)
                    || self.pending_saves.contains_key(&node)
                {
                    self.status = format!("Save {path} before moving it to trash.");
                    return;
                }
                self.trash_drive_file(path, node, id, cx);
            }
            Some(PendingDrivePrompt::Rename { from, to, rewrites }) => {
                self.pending_drive_prompt = None;
                self.finish_drive_rename(from, to, rewrites, true, cx);
            }
            Some(PendingDrivePrompt::MoveConfirm {
                from,
                to,
                destination,
                file_id,
                source_parent_id,
                destination_parent_id,
                rewrites,
            }) => {
                self.pending_drive_prompt = None;
                self.finish_drive_move(
                    from,
                    to,
                    destination,
                    file_id,
                    source_parent_id,
                    destination_parent_id,
                    rewrites,
                    true,
                    cx,
                );
            }
            None => {}
        }
        cx.notify();
    }

    fn submit_drive_secondary(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.pending_drive_prompt,
            Some(PendingDrivePrompt::DeleteFile { .. })
        ) {
            self.pending_drive_prompt = None;
            self.prompt_open = false;
            cx.notify();
            return;
        }
        let Some(pending) = self.pending_drive_prompt.take() else {
            return;
        };
        self.prompt_open = false;
        match pending {
            PendingDrivePrompt::Rename { from, to, rewrites } => {
                self.finish_drive_rename(from, to, rewrites, false, cx)
            }
            PendingDrivePrompt::MoveConfirm {
                from,
                to,
                destination,
                file_id,
                source_parent_id,
                destination_parent_id,
                rewrites,
            } => self.finish_drive_move(
                from,
                to,
                destination,
                file_id,
                source_parent_id,
                destination_parent_id,
                rewrites,
                false,
                cx,
            ),
            other => self.pending_drive_prompt = Some(other),
        }
    }

    fn trash_drive_file(&mut self, path: String, node: NodeId, id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.dirty_nodes.contains_key(&node)
            || self.saving_nodes.contains(&node)
            || self.pending_saves.contains_key(&node)
        {
            self.status = format!("Save {path} before moving it to trash.");
            cx.notify();
            return;
        }
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        self.busy = true;
        self.status = format!("Moving {path} to Google Drive trash…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::trash_entry(&id).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(()) => {
                        app.entries.retain(|entry| entry.node != node && entry.path != path);
                        app.dirty_nodes.remove(&node);
                        app.pending_saves.remove(&node);
                        app.saving_nodes.remove(&node);
                        app.save_generation.remove(&node);
                        app.status = match workspace.update(cx, |workspace, cx| {
                            workspace.remove_drive_file(node, &path, cx)
                        }) {
                            Ok(()) => format!("Moved {path} to Google Drive trash."),
                            Err(error) => format!("Moved {path} to Drive trash, but could not update the open workspace: {error}"),
                        };
                    }
                    Err(error) => app.status = format!("Could not move {path} to trash: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn finish_drive_heading_rename(
        &mut self,
        path: String,
        line: u32,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if name.is_empty() || self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let own_text = {
            let workspace = workspace.read(cx);
            let Some((_node, contents)) = workspace.current_document(cx) else {
                return;
            };
            if workspace.current_path() != Some(path.as_str()) {
                self.status =
                    "The active note changed before the heading rename was applied.".into();
                cx.notify();
                return;
            }
            contents
        };
        let plan = workspace
            .read(cx)
            .plan_heading_rename(&path, line, name, &own_text);
        let Some(plan) = plan else {
            return;
        };
        let heading_name = plan.heading.text.clone();
        let result = workspace.update(cx, |workspace, cx| {
            workspace.apply_heading_rename(&path, &plan, window, cx)
        });
        match result {
            Ok(()) => {
                self.status = format!("Renamed heading '{heading_name}' in {path}.");
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    fn start_drive_rename(
        &mut self,
        from: &str,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let rewrites = match workspace.read(cx).plan_drive_rename(from, to) {
            Ok(rewrites) => rewrites,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        let update_automatically = workspace.read(cx).always_update_links();
        if rewrites.is_empty() || update_automatically {
            self.finish_drive_rename(from.to_owned(), to.to_owned(), rewrites, true, cx);
            return;
        }
        let link_count = rewrites.iter().map(|rewrite| rewrite.links).sum::<usize>();
        let file_count = rewrites.len();
        self.pending_drive_prompt = Some(PendingDrivePrompt::Rename {
            from: from.to_owned(),
            to: to.to_owned(),
            rewrites,
        });
        self.prompt_open = true;
        self.prompt.update(cx, |prompt, cx| {
            prompt.open(
                PromptSpec {
                    title: "Update links?".into(),
                    detail: vec![format!(
                        "Update {link_count} link{} in {file_count} file{} to follow this rename?",
                        if link_count == 1 { "" } else { "s" },
                        if file_count == 1 { "" } else { "s" },
                    )
                    .into()],
                    confirm: "Update links".into(),
                    secondary: Some("Don't update".into()),
                    no_enter: true,
                    ..PromptSpec::default()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn request_drive_move(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let suggestions = workspace
            .read(cx)
            .move_destination_suggestions(&path)
            .unwrap_or_default();
        self.pending_drive_prompt = Some(PendingDrivePrompt::MoveInput { from: path.clone() });
        self.prompt_open = true;
        self.prompt.update(cx, |prompt, cx| {
            prompt.open(
                PromptSpec {
                    title: "Move file to...".into(),
                    detail: vec![path.into()],
                    input: Some(String::new()),
                    placeholder: "Type a folder".into(),
                    suggestions,
                    confirm: "Move".into(),
                    ..PromptSpec::default()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn start_drive_move(
        &mut self,
        from: String,
        destination: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let destination = destination.trim_matches('/').to_owned();
        let name = from.rsplit('/').next().unwrap_or(&from);
        let to = if destination.is_empty() {
            name.to_owned()
        } else {
            format!("{destination}/{name}")
        };
        if to == from {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let (rewrites, file_id, source_parent_id, destination_parent_id, update_automatically) = {
            let workspace = workspace.read(cx);
            let rewrites = match workspace.plan_drive_rename(&from, &to) {
                Ok(rewrites) => rewrites,
                Err(error) => {
                    self.status = error;
                    cx.notify();
                    return;
                }
            };
            let source_parent = from
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            let Some(file_id) = workspace.drive_file_id(&from) else {
                self.status = format!("Could not find the Drive file for {from}.");
                cx.notify();
                return;
            };
            let Some(source_parent_id) = workspace.drive_folder_id(source_parent) else {
                self.status = format!("Could not find the source Drive folder for {from}.");
                cx.notify();
                return;
            };
            let Some(destination_parent_id) = workspace.drive_folder_id(&destination) else {
                self.status = format!("Could not find destination folder: {destination}");
                cx.notify();
                return;
            };
            (
                rewrites,
                file_id,
                source_parent_id,
                destination_parent_id,
                workspace.always_update_links(),
            )
        };
        if rewrites.is_empty() || update_automatically {
            self.finish_drive_move(
                from,
                to,
                destination,
                file_id,
                source_parent_id,
                destination_parent_id,
                rewrites,
                true,
                cx,
            );
            return;
        }
        let link_count = rewrites.iter().map(|rewrite| rewrite.links).sum::<usize>();
        let file_count = rewrites.len();
        self.pending_drive_prompt = Some(PendingDrivePrompt::MoveConfirm {
            from: from.clone(),
            to,
            destination,
            file_id,
            source_parent_id,
            destination_parent_id,
            rewrites,
        });
        self.prompt_open = true;
        self.prompt.update(cx, |prompt, cx| {
            prompt.open(
                PromptSpec {
                    title: "Update links?".into(),
                    detail: vec![format!(
                        "Move {from} and update {link_count} link{} in {file_count} file{}?",
                        if link_count == 1 { "" } else { "s" },
                        if file_count == 1 { "" } else { "s" },
                    )
                    .into()],
                    confirm: "Update links".into(),
                    secondary: Some("Don't update".into()),
                    no_enter: true,
                    ..PromptSpec::default()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn finish_drive_move(
        &mut self,
        from: String,
        to: String,
        destination: String,
        file_id: String,
        source_parent_id: String,
        destination_parent_id: String,
        rewrites: Vec<tft_cache::Rewrite>,
        update_links: bool,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let rewrite_writes = if update_links {
            let mut writes = Vec::new();
            for rewrite in rewrites {
                let Some(id) = workspace.read(cx).drive_file_id(&rewrite.old_path) else {
                    self.status =
                        format!("Could not find the Drive file for {}.", rewrite.old_path);
                    cx.notify();
                    return;
                };
                writes.push((id, rewrite));
            }
            writes
        } else {
            Vec::new()
        };
        self.busy = true;
        self.status = format!("Moving {from} to {destination} in Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let move_etag =
                    drive::move_entry(&file_id, &destination_parent_id, &source_parent_id).await?;
                let mut applied = Vec::new();
                let mut failed = Vec::new();
                for (id, rewrite) in rewrite_writes {
                    match drive::write_file(&id, &rewrite.text, None).await {
                        Ok(etag) => applied.push((rewrite, etag, id)),
                        Err(error) => failed.push(format!("{}: {error}", rewrite.old_path)),
                    }
                }
                Ok::<_, String>((applied, failed, move_etag))
            }
            .await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok((applied, failed, move_etag)) => {
                        let rewritten = applied
                            .iter()
                            .map(|(rewrite, _, _)| rewrite.clone())
                            .collect::<Vec<_>>();
                        let update_result = workspace.update(cx, |workspace, cx| {
                            workspace.apply_drive_move(&from, &to, &destination, &rewritten, cx)
                        });
                        for (_, etag, id) in &applied {
                            if let Some(entry) =
                                app.entries.iter_mut().find(|entry| &entry.id == id)
                            {
                                entry.etag = etag.clone();
                            }
                        }
                        if let Some(entry) =
                            app.entries.iter_mut().find(|entry| entry.id == file_id)
                        {
                            entry.etag = move_etag;
                            entry.path = to.clone();
                        }
                        if let Err(error) = update_result {
                            app.status = format!(
                                "Drive moved the file, but the workspace could not update: {error}"
                            );
                        } else if failed.is_empty() {
                            app.status = format!("Moved {from} to {to}.");
                        } else {
                            app.status = format!(
                                "Moved {from} to {to}; some links could not be updated: {}",
                                failed.join("; ")
                            );
                        }
                    }
                    Err(error) => app.status = format!("Could not move {from}: {error}"),
                }
                app.prompt_open = false;
                app.pending_drive_prompt = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn duplicate_drive_file(
        &mut self,
        source_path: String,
        source_id: String,
        parent_path: String,
        parent_id: String,
        kind: tft_formats::FileKind,
        input: String,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let mut name = input.trim().to_owned();
        if name.is_empty() {
            self.prompt_open = true;
            self.prompt.update(cx, |prompt, cx| {
                prompt.set_error("Enter a name for the copy.", cx)
            });
            cx.notify();
            return;
        }
        let source_name = source_path.rsplit('/').next().unwrap_or(&source_path);
        if let Some((_, extension)) = source_name.rsplit_once('.') {
            if !name
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", extension.to_ascii_lowercase()))
            {
                name.push('.');
                name.push_str(extension);
            }
        }
        if let Err(error) = workspace.read(cx).can_create_note(&parent_path, &name) {
            self.prompt_open = true;
            self.prompt
                .update(cx, |prompt, cx| prompt.set_error(error, cx));
            cx.notify();
            return;
        }
        self.pending_drive_prompt = None;
        self.busy = true;
        self.status = format!("Copying {source_path} in Google Drive…");
        cx.notify();
        let workspace = workspace.clone();
        let vault_id = parent_id.clone();
        cx.spawn(async move |this, cx| {
            let result = async {
                let copied = drive::duplicate_entry(&source_id, &vault_id, &name).await?;
                let contents = if matches!(kind, tft_formats::FileKind::Markdown | tft_formats::FileKind::Canvas) {
                    match drive::read_file(&copied.id).await {
                        Ok(contents) => Some(contents),
                        Err(error) => {
                            let cleanup = drive::delete_entry(&copied.id).await;
                            return Err(match cleanup {
                                Ok(()) => format!("Could not read the copy and removed it: {error}"),
                                Err(cleanup_error) => format!("Could not read the copy: {error}; cleanup failed: {cleanup_error}"),
                            });
                        }
                    }
                } else {
                    None
                };
                Ok::<_, String>((copied, contents))
            }
            .await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok((copied, contents)) => {
                        let copied_path = if parent_path.is_empty() {
                            copied.name.clone()
                        } else {
                            format!("{parent_path}/{}", copied.name)
                        };
                        let size = copied
                            .size
                            .as_deref()
                            .and_then(|value| value.parse::<u64>().ok())
                            .unwrap_or(0);
                        match workspace.update(cx, |workspace, cx| {
                            workspace.insert_drive_copy(
                                copied.id.clone(),
                                copied.name.clone(),
                                copied_path.clone(),
                                kind,
                                size,
                                copied.modified_millis(),
                                copied.created_millis(),
                                contents,
                                cx,
                            )
                        }) {
                            Ok(node) => {
                                app.entries.push(IndexedFile {
                                    node,
                                    path: copied_path.clone(),
                                    kind,
                                    id: copied.id,
                                    etag: copied.etag,
                                });
                                app.status = format!("Created copy {copied_path}.");
                            }
                            Err(error) => {
                                let copied_id = copied.id.clone();
                                let failure = format!("TFT could not index the Drive copy: {error}");
                                app.status = format!("{failure}; removing the unindexed copy…");
                                cx.spawn(async move |this, cx| {
                                    let cleanup = drive::delete_entry(&copied_id).await;
                                    this.update(cx, |app, cx| {
                                        app.status = match cleanup {
                                            Ok(()) => format!("{failure}. The unindexed copy was removed."),
                                            Err(cleanup_error) => format!("{failure}; cleanup failed: {cleanup_error}"),
                                        };
                                        cx.notify();
                                    })
                                    .ok();
                                })
                                .detach();
                            }
                        }
                    }
                    Err(error) => app.status = format!("Could not copy {source_path}: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn finish_drive_rename(
        &mut self,
        from: String,
        to: String,
        _rewrites: Vec<tft_cache::Rewrite>,
        update_links: bool,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let rewrites = match workspace.read(cx).plan_drive_rename(&from, &to) {
            Ok(rewrites) => rewrites,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        let Some(file_id) = workspace.read(cx).drive_file_id(&from) else {
            self.status = format!("Could not find the Drive file for {from}.");
            cx.notify();
            return;
        };
        let rewrite_writes = if update_links {
            let mut writes = Vec::new();
            for rewrite in rewrites {
                let Some(id) = workspace.read(cx).drive_file_id(&rewrite.old_path) else {
                    self.status =
                        format!("Could not find the Drive file for {}.", rewrite.old_path);
                    cx.notify();
                    return;
                };
                writes.push((id, rewrite));
            }
            writes
        } else {
            Vec::new()
        };
        let name = to.rsplit('/').next().unwrap_or(&to).to_owned();
        let workspace = workspace.clone();
        self.busy = true;
        self.status = format!("Renaming {from} in Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let rename_etag = drive::rename_entry_with_etag(&file_id, &name).await?;
                let mut applied = Vec::new();
                let mut failed = Vec::new();
                for (id, rewrite) in rewrite_writes {
                    match drive::write_file(&id, &rewrite.text, None).await {
                        Ok(etag) => applied.push((rewrite, etag, id)),
                        Err(error) => failed.push(format!("{}: {error}", rewrite.old_path)),
                    }
                }
                Ok::<_, String>((applied, failed, rename_etag))
            }
            .await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok((applied, failed, rename_etag)) => {
                        let rewritten = applied
                            .iter()
                            .map(|(rewrite, _, _)| rewrite.clone())
                            .collect::<Vec<_>>();
                        let update_result = workspace.update(cx, |workspace, cx| {
                            workspace.apply_drive_rename(&from, &to, &rewritten, cx)
                        });
                        for (_, etag, id) in &applied {
                            if let Some(entry) = app.entries.iter_mut().find(|entry| &entry.id == id)
                            {
                                entry.etag = etag.clone();
                            }
                        }
                        if let Some(entry) = app.entries.iter_mut().find(|entry| entry.id == file_id)
                        {
                            entry.etag = rename_etag;
                        }
                        if let Err(error) = update_result {
                            app.status = format!("Drive renamed the file, but the workspace could not update: {error}");
                        } else if failed.is_empty() {
                            app.status = format!("Renamed {from} to {to}.");
                        } else {
                            app.status = format!("Renamed {from} to {to}; some links could not be updated: {}", failed.join("; "));
                        }
                        for entry in &mut app.entries {
                            if entry.path == from {
                                entry.path = to.clone();
                            }
                        }
                    }
                    Err(error) => app.status = format!("Could not rename {from}: {error}"),
                }
                app.prompt_open = false;
                app.pending_drive_prompt = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_note(&mut self, parent_path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.create_note_with_initial(parent_path, "Untitled", window, cx);
    }

    fn create_note_with_initial(
        &mut self,
        parent_path: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.pending_drive_prompt = Some(PendingDrivePrompt::CreateNote {
            parent: parent_path.to_owned(),
        });
        self.prompt_open = true;
        self.prompt.update(cx, |prompt, cx| {
            prompt.open(
                PromptSpec {
                    title: "New Markdown note".into(),
                    input: Some(initial.to_owned()),
                    placeholder: "Name".into(),
                    confirm: "Create".into(),
                    ..PromptSpec::default()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn create_note_named(&mut self, parent_path: &str, name: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let mut name = name;
        name = name.trim().to_owned();
        if name.is_empty() {
            self.prompt_open = true;
            self.prompt
                .update(cx, |prompt, cx| prompt.set_error("Enter a note name.", cx));
            cx.notify();
            return;
        }
        if !name.to_ascii_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        if let Err(error) = workspace.read(cx).can_create_note(parent_path, &name) {
            let error = error.to_string();
            self.status = error.clone();
            self.prompt_open = true;
            self.prompt
                .update(cx, |prompt, cx| prompt.set_error(error, cx));
            cx.notify();
            return;
        }
        let title = name.strip_suffix(".md").unwrap_or(&name).to_owned();
        let contents = format!("# {title}\n\n");
        let (parent, path) = {
            let workspace = workspace.read(cx);
            let Some(parent) = workspace.drive_folder_id(parent_path) else {
                self.status = format!("Could not find Drive folder: {parent_path}");
                cx.notify();
                return;
            };
            let path = if parent_path.is_empty() {
                name.clone()
            } else {
                format!("{parent_path}/{name}")
            };
            (parent, path)
        };
        self.pending_drive_prompt = None;
        self.busy = true;
        self.status = format!("Creating {path} in Drive…");
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
                            path: path.clone(),
                            contents,
                            kind: tft_formats::FileKind::Markdown,
                        });
                        app.status = format!("Created {} in Drive.", path);
                    }
                    Err(error) => app.status = format!("Could not create note: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_folder(&mut self, parent_path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.pending_drive_prompt = Some(PendingDrivePrompt::CreateFolder {
            parent: parent_path.to_owned(),
        });
        self.prompt_open = true;
        self.prompt.update(cx, |prompt, cx| {
            prompt.open(
                PromptSpec {
                    title: "New folder".into(),
                    input: Some("Untitled folder".into()),
                    placeholder: "Folder name".into(),
                    confirm: "Create".into(),
                    ..PromptSpec::default()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn create_canvas(&mut self, parent_path: &str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let (parent, name, path) = {
            let workspace = workspace.read(cx);
            let Some(parent) = workspace.drive_folder_id(parent_path) else {
                self.status = format!("Could not find Drive folder: {parent_path}");
                cx.notify();
                return;
            };
            let mut index = 0usize;
            let (name, path) = loop {
                let stem = if index == 0 {
                    "Untitled".to_owned()
                } else {
                    format!("Untitled {index}")
                };
                let name = format!("{stem}.canvas");
                if workspace.can_create_note(parent_path, &name).is_ok() {
                    let path = if parent_path.is_empty() {
                        name.clone()
                    } else {
                        format!("{parent_path}/{name}")
                    };
                    break (name, path);
                }
                index += 1;
            };
            (parent, name, path)
        };
        self.busy = true;
        self.status = format!("Creating {path} in Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::create_empty_file_with_id(&parent, &name).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(id) => {
                        app.pending_note = Some(CreatedDriveNote {
                            id,
                            name: name.clone(),
                            path: path.clone(),
                            contents: String::new(),
                            kind: tft_formats::FileKind::Canvas,
                        });
                        app.status = format!("Created {} in Drive.", path);
                    }
                    Err(error) => app.status = format!("Could not create canvas: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_canvas_with_initial(
        &mut self,
        parent_path: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.pending_drive_prompt = Some(PendingDrivePrompt::CreateCanvas {
            parent: parent_path.to_owned(),
        });
        self.prompt_open = true;
        self.prompt.update(cx, |prompt, cx| {
            prompt.open(
                PromptSpec {
                    title: "New canvas".into(),
                    input: Some(initial.to_owned()),
                    placeholder: "Name".into(),
                    confirm: "Create".into(),
                    ..PromptSpec::default()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn create_canvas_named(&mut self, parent_path: &str, mut name: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        name = name.trim().to_owned();
        if name.is_empty() {
            self.prompt_open = true;
            self.prompt.update(cx, |prompt, cx| {
                prompt.set_error("Enter a canvas name.", cx)
            });
            cx.notify();
            return;
        }
        if !name.to_ascii_lowercase().ends_with(".canvas") {
            name.push_str(".canvas");
        }
        if let Err(error) = workspace.read(cx).can_create_note(parent_path, &name) {
            let error = error.to_string();
            self.status = error.clone();
            self.prompt_open = true;
            self.prompt
                .update(cx, |prompt, cx| prompt.set_error(error, cx));
            cx.notify();
            return;
        }
        let (parent, path) = {
            let workspace = workspace.read(cx);
            let Some(parent) = workspace.drive_folder_id(parent_path) else {
                self.status = format!("Could not find Drive folder: {parent_path}");
                cx.notify();
                return;
            };
            let path = if parent_path.is_empty() {
                name.clone()
            } else {
                format!("{parent_path}/{name}")
            };
            (parent, path)
        };
        self.pending_drive_prompt = None;
        self.busy = true;
        self.status = format!("Creating {path} in Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::create_empty_file_with_id(&parent, &name).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(id) => {
                        app.pending_note = Some(CreatedDriveNote {
                            id,
                            name: name.clone(),
                            path: path.clone(),
                            contents: String::new(),
                            kind: tft_formats::FileKind::Canvas,
                        });
                        app.status = format!("Created {} in Drive.", path);
                    }
                    Err(error) => app.status = format!("Could not create canvas: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_folder_named(&mut self, parent_path: &str, name: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.prompt_open = true;
            self.prompt.update(cx, |prompt, cx| {
                prompt.set_error("Enter a folder name.", cx)
            });
            cx.notify();
            return;
        }
        if let Err(error) = workspace.read(cx).can_create_note(parent_path, &name) {
            let error = error.to_string();
            self.status = error.clone();
            self.prompt_open = true;
            self.prompt
                .update(cx, |prompt, cx| prompt.set_error(error, cx));
            cx.notify();
            return;
        }
        let Some(parent_id) = workspace.read(cx).drive_folder_id(parent_path) else {
            self.status = format!("Could not find Drive folder: {parent_path}");
            cx.notify();
            return;
        };
        let path = if parent_path.is_empty() {
            name.clone()
        } else {
            format!("{parent_path}/{name}")
        };
        self.pending_drive_prompt = None;
        self.busy = true;
        self.status = format!("Creating folder {path} in Drive…");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = drive::create_folder_entry(&parent_id, &name).await;
            this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(id) => {
                        app.pending_folder = Some(CreatedDriveFolder {
                            id,
                            name,
                            path: path.clone(),
                        });
                        app.status = format!("Created folder {path} in Drive.");
                    }
                    Err(error) => app.status = format!("Could not create folder: {error}"),
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

    /// A folder expanded for the first time: fetch only its direct children.
    fn load_folder_children(&mut self, node: NodeId, path: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let Some(folder_id) = workspace.read(cx).folder_drive_id(node) else {
            workspace.update(cx, |workspace, cx| {
                workspace.folder_children_failed(node, cx)
            });
            return;
        };
        let epoch = self.vault_epoch;
        cx.spawn(async move |this, cx| {
            let parent = tft_app::BackendObjectId::Drive(folder_id.clone());
            let result = drive::list_folder(&folder_id, &parent).await;
            let _ = this.update(cx, |app, cx| {
                if app.vault_epoch != epoch {
                    return;
                }
                match result {
                    Ok(scanned) => {
                        workspace.update(cx, |workspace, cx| {
                            workspace.add_folder_children(node, &path, scanned, cx);
                        });
                        app.entries = workspace.read(cx).drive_entries();
                    }
                    Err(error) => {
                        workspace.update(cx, |workspace, cx| {
                            workspace.folder_children_failed(node, cx)
                        });
                        app.status = format!("Could not list {path}: {error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A note opened before its background read arrived: fetch just that file
    /// and let the workspace fill the editor if it is still the active note.
    fn load_note_text(&mut self, node: NodeId, path: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        // The entries table is normally populated by the folder expansion
        // that revealed this row, but the Vault model already knows the Drive
        // backend id even if the table lags one frame.
        let Some(id) = self
            .entries
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| entry.id.clone())
            .or_else(|| workspace.read(cx).folder_drive_id(node))
        else {
            workspace.update(cx, |workspace, cx| workspace.note_text_failed(node, cx));
            return;
        };
        let epoch = self.vault_epoch;
        cx.spawn(async move |this, cx| {
            let Ok(text) = drive::read_file(&id).await else {
                let _ = this.update(cx, |app, cx| {
                    if app.vault_epoch != epoch {
                        return;
                    }
                    workspace.update(cx, |workspace, cx| workspace.note_text_failed(node, cx));
                });
                return;
            };
            let _ = this.update(cx, |app, cx| {
                if app.vault_epoch != epoch {
                    return;
                }
                workspace.update(cx, |workspace, cx| {
                    // `open` may have written the empty placeholder into the
                    // Vault before this read finished; ingest must replace it.
                    workspace.ingest_text(node, path, text, cx);
                });
            });
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
        self.vault_workspace_epoch = self.vault_workspace_epoch.wrapping_add(1);
        self.pending_workspace_state = None;
        self.workspace_saving = false;
        self.last_graph_config = None;
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
    // The tree and the host configuration are independent reads; start them
    // together so a Picker selection reaches the workspace in one round trip
    // instead of a chain of sequential Drive calls.
    let (
        index,
        appearance,
        app_config,
        switcher_config,
        page_preview_config,
        hotkeys,
        drive_workspace_result,
        graph_config_from_file,
    ) = futures::join!(
        drive::index_vault(&id),
        drive::read_appearance_config(&id),
        drive::read_app_config(&id),
        drive::read_switcher_config(&id),
        drive::read_page_preview_config(&id),
        drive::read_hotkeys_config(&id),
        drive::read_web_workspace_state(&id),
        drive::read_graph_config(&id),
    );
    let index: VaultIndex = index?;
    let appearance = appearance.unwrap_or_default();
    let app_config = app_config.unwrap_or_default();
    let switcher_config = switcher_config.unwrap_or_default();
    let page_preview_config = page_preview_config.unwrap_or_default();
    let hotkeys = hotkeys.unwrap_or_default();
    let graph_config_from_file = graph_config_from_file?;
    let local_workspace_state = drive::local_workspace_state(&id);
    let workspace_state = match (local_workspace_state, drive_workspace_result) {
        (Some(local_state), drive_result) => {
            let drive_has_local_state =
                drive_result.as_ref().ok().and_then(Option::as_deref) == Some(local_state.as_str());
            if !drive_has_local_state {
                // The local copy is the recovery source when Drive reads fail.
                // A failed read must not prevent a fresh write attempt: if the
                // outage was transient this restores Drive immediately, while
                // malformed or inaccessible workspace files still fail safely
                // in write_web_workspace_state and keep the local copy.
                if drive::write_web_workspace_state(&id, &local_state)
                    .await
                    .is_ok()
                {
                    drive::clear_local_workspace_state(&id, &local_state);
                }
            }
            Some(local_state)
        }
        (None, Ok(drive_state)) => drive_state,
        (None, Err(error)) => {
            return Err(format!("Could not restore the saved workspace: {error}"));
        }
    };
    let workspace_graph_config = || {
        workspace_state
            .as_deref()
            .and_then(|state| serde_json::from_str::<serde_json::Value>(state).ok())
            .and_then(|state| state.get("graphConfig").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
    };
    // A graphConfig still present in tftWeb is either a local recovery copy
    // or a legacy Web setting not yet migrated to graph.json. Successful
    // graph.json syncs remove this duplicate, making that file canonical.
    let graph_config = workspace_graph_config()
        .or_else(|| graph_config_from_file.clone())
        .unwrap_or_default();
    // Metadata alone opens the workspace; the active note's body arrives
    // through the same on-demand read every other file uses.
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
        app_config,
        switcher_config,
        page_preview_config,
        graph_config,
        graph_config_disk_json: graph_config_from_file
            .as_ref()
            .and_then(|config| serde_json::to_string(config).ok()),
        hotkeys,
        workspace_state,
    })
}

impl Render for DriveApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        self.publish_debug_state(cx);
        if !self.busy {
            if let Some((path, line, name)) = self.pending_heading_rename.take() {
                self.finish_drive_heading_rename(path, line, &name, window, cx);
            }
            if let Some((from, to)) = self.pending_rename_start.take() {
                self.start_drive_rename(&from, &to, window, cx);
            }
            if let Some((from, destination)) = self.pending_move_start.take() {
                self.start_drive_move(from, destination, window, cx);
            }
        }
        if self.workspace.is_none() {
            if let Some(vault) = self.pending.take() {
                self.vault_epoch = self.vault_epoch.wrapping_add(1);
                self.vault_workspace_epoch = self.vault_workspace_epoch.wrapping_add(1);
                cx.set_global(Theme::from_appearance(
                    &vault.appearance,
                    drive::system_is_dark(),
                ));
                self.vault_id = vault.id;
                self.vault_name = vault.name;
                self.entries = vault.entries;
                self.last_graph_config = vault.graph_config_disk_json;
                let vault_name = self.vault_name.clone();
                let workspace_state = vault.workspace_state;
                let app_config = vault.app_config;
                let switcher_config = vault.switcher_config;
                let page_preview_config = vault.page_preview_config;
                let graph_config = vault.graph_config;
                let hotkeys = vault.hotkeys;
                let workspace = cx.new(|cx| {
                    let mut workspace = WebWorkspace::from_vault_with_state(
                        vault.data,
                        vault.initial_node,
                        workspace_state,
                        app_config,
                        switcher_config,
                        page_preview_config,
                        graph_config,
                        hotkeys,
                        window,
                        cx,
                    );
                    workspace.set_vault_name(vault_name, cx);
                    workspace.set_drive_controls(true, cx);
                    workspace
                });
                self.workspace_subscription = Some(cx.subscribe_in(
                    &workspace,
                    window,
                    |this, _, event: &WebWorkspaceEvent, window, cx| match event {
                        WebWorkspaceEvent::Edited { node, contents } => {
                            this.schedule_auto_save(*node, contents.clone(), cx)
                        }
                        WebWorkspaceEvent::FileOperationRequested(
                            FileOperationRequest::CreateNote { parent },
                        ) => this.create_note(parent, window, cx),
                        WebWorkspaceEvent::FileOperationRequested(
                            FileOperationRequest::CreateFolder { parent },
                        ) => this.create_folder(parent, window, cx),
                        WebWorkspaceEvent::CreateCanvasRequested { parent } => {
                            this.create_canvas(parent, cx)
                        }
                        WebWorkspaceEvent::CreateFileFromLink { parent, name, kind } => {
                            if *kind == tft_formats::FileKind::Canvas {
                                this.create_canvas_with_initial(parent, name, window, cx)
                            } else {
                                this.create_note_with_initial(parent, name, window, cx)
                            }
                        }
                        WebWorkspaceEvent::CanvasEdgeLabelRequested {
                            file,
                            edge_id,
                            label,
                        } => {
                            this.pending_drive_prompt = Some(PendingDrivePrompt::CanvasEdgeLabel {
                                file: *file,
                                edge_id: edge_id.clone(),
                            });
                            this.prompt_open = true;
                            this.prompt.update(cx, |prompt, cx| {
                                prompt.open(
                                    PromptSpec {
                                        title: "Canvas edge label".into(),
                                        input: Some(label.clone()),
                                        placeholder: "Label".into(),
                                        confirm: "Save".into(),
                                        ..PromptSpec::default()
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }
                        WebWorkspaceEvent::RenameFileRequested { from, to } => {
                            this.start_drive_rename(from, to, window, cx)
                        }
                        WebWorkspaceEvent::RenameFilePromptRequested { path } => {
                            if this.busy {
                                return;
                            }
                            let file_name = path.rsplit('/').next().unwrap_or(path);
                            let initial = file_name
                                .rsplit_once('.')
                                .map(|(stem, _)| stem)
                                .unwrap_or(file_name)
                                .to_owned();
                            this.pending_drive_prompt =
                                Some(PendingDrivePrompt::RenameInput { from: path.clone() });
                            this.prompt_open = true;
                            this.prompt.update(cx, |prompt, cx| {
                                prompt.open(
                                    PromptSpec {
                                        title: "Rename file".into(),
                                        input: Some(initial),
                                        placeholder: "File name".into(),
                                        confirm: "Rename".into(),
                                        ..PromptSpec::default()
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }
                        WebWorkspaceEvent::MoveFilePromptRequested { path } => {
                            this.request_drive_move(path.clone(), window, cx);
                        }
                        WebWorkspaceEvent::RenameHeadingRequested {
                            path,
                            line,
                            initial,
                        } => {
                            this.pending_drive_prompt = Some(PendingDrivePrompt::RenameHeading {
                                path: path.clone(),
                                line: *line,
                            });
                            this.prompt_open = true;
                            this.prompt.update(cx, |prompt, cx| {
                                prompt.open(
                                    PromptSpec {
                                        title: "Rename heading".into(),
                                        input: Some(initial.clone()),
                                        confirm: "Save".into(),
                                        ..PromptSpec::default()
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }
                        WebWorkspaceEvent::DuplicateFileRequested { path } => {
                            if this.busy {
                                return;
                            }
                            let Some(source) =
                                this.entries.iter().find(|entry| entry.path == *path)
                            else {
                                this.status = format!("Could not find the Drive file for {path}.");
                                cx.notify();
                                return;
                            };
                            let source_id = source.id.clone();
                            let kind = source.kind;
                            let parent_path = path
                                .rsplit_once('/')
                                .map(|(parent, _)| parent.to_owned())
                                .unwrap_or_default();
                            let Some(workspace) = this.workspace.as_ref() else {
                                return;
                            };
                            let Some(parent_id) = workspace.read(cx).drive_folder_id(&parent_path)
                            else {
                                this.status = format!("Could not find Drive folder: {parent_path}");
                                cx.notify();
                                return;
                            };
                            let file_name = path.rsplit('/').next().unwrap_or(path);
                            let initial = match file_name.rsplit_once('.') {
                                Some((stem, extension)) => format!("{stem} copy.{extension}"),
                                None => format!("{file_name} copy"),
                            };
                            this.pending_drive_prompt = Some(PendingDrivePrompt::DuplicateFile {
                                source_path: path.clone(),
                                source_id,
                                parent_path,
                                parent_id,
                                kind,
                            });
                            this.prompt_open = true;
                            this.prompt.update(cx, |prompt, cx| {
                                prompt.open(
                                    PromptSpec {
                                        title: "Duplicate file".into(),
                                        input: Some(initial),
                                        placeholder: "Copy name".into(),
                                        confirm: "Duplicate".into(),
                                        ..PromptSpec::default()
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }
                        WebWorkspaceEvent::DeleteFileRequested { path } => {
                            if this.busy {
                                return;
                            }
                            let Some(file) = this.entries.iter().find(|entry| entry.path == *path)
                            else {
                                this.status = format!("Could not find the Drive file for {path}.");
                                cx.notify();
                                return;
                            };
                            let (node, id) = (file.node, file.id.clone());
                            if this.dirty_nodes.contains_key(&node)
                                || this.saving_nodes.contains(&node)
                                || this.pending_saves.contains_key(&node)
                            {
                                this.status = format!("Save {path} before moving it to trash.");
                                cx.notify();
                                return;
                            }
                            this.pending_drive_prompt = Some(PendingDrivePrompt::DeleteFile {
                                path: path.clone(),
                                node,
                                id,
                            });
                            this.prompt_open = true;
                            this.prompt.update(cx, |prompt, cx| {
                                prompt.open(
                                    PromptSpec {
                                        title: "Move to trash?".into(),
                                        detail: vec![path.clone().into()],
                                        confirm: "Move to trash".into(),
                                        secondary: Some("Cancel".into()),
                                        no_enter: true,
                                        ..PromptSpec::default()
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }
                        WebWorkspaceEvent::SaveRequested => this.save_current(cx),
                        WebWorkspaceEvent::WorkspaceStateChanged(state) => {
                            this.schedule_workspace_save(state.clone(), cx)
                        }
                        WebWorkspaceEvent::SwitchVaultRequested => this.close_vault(cx),
                        WebWorkspaceEvent::NoteTextRequested { node, path } => {
                            this.load_note_text(*node, path.clone(), cx)
                        }
                        WebWorkspaceEvent::FolderChildrenRequested { node, path } => {
                            this.load_folder_children(*node, path.clone(), cx)
                        }
                    },
                ));
                if let Some((node, path)) = workspace.read(cx).active_missing_text() {
                    self.load_note_text(node, path, cx);
                }
                self.workspace = Some(workspace);
            }
        }

        if let Some(note) = self.pending_note.take() {
            if let Some(workspace) = self.workspace.clone() {
                let result = workspace.update(cx, |workspace, cx| {
                    if note.kind == tft_formats::FileKind::Canvas {
                        workspace.insert_drive_canvas(
                            note.id.clone(),
                            note.name.clone(),
                            note.path.clone(),
                            note.contents.clone(),
                            window,
                            cx,
                        )
                    } else {
                        workspace.insert_drive_note(
                            note.id.clone(),
                            note.name.clone(),
                            note.path.clone(),
                            note.contents.clone(),
                            window,
                            cx,
                        )
                    }
                });
                match result {
                    Ok(node) => {
                        self.entries.push(IndexedFile {
                            node,
                            path: note.path.clone(),
                            kind: note.kind,
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

        if let Some(folder) = self.pending_folder.take() {
            if let Some(workspace) = self.workspace.clone() {
                let result = workspace.update(cx, |workspace, cx| {
                    workspace.insert_drive_folder(
                        folder.id.clone(),
                        folder.name.clone(),
                        folder.path.clone(),
                        cx,
                    )
                });
                match result {
                    Ok(()) => self.status = format!("Created folder {}.", folder.path),
                    Err(error) => self.status = format!("Could not open new folder: {error}"),
                }
            }
        }

        #[cfg(feature = "drive-smoke-test")]
        self.publish_test_state(cx);

        let theme = cx.global::<Theme>().clone();
        let tokens = &theme.tokens;
        if let Some(workspace) = self.workspace.as_ref() {
            let workspace = workspace.clone();
            let prompt = self.prompt.clone();
            return div()
                .size_full()
                .relative()
                .bg(tokens.background_primary)
                .text_color(tokens.text_normal)
                .child(workspace)
                .when(self.prompt_open, |root| {
                    root.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(gpui::hsla(0., 0., 0., 0.42))
                            .child(prompt),
                    )
                });
        }

        let status = self.status.clone();
        let busy = self.busy;
        let signed_in = self.signed_in;
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(tokens.background_primary)
            .text_color(tokens.text_normal)
            .child(
                div()
                    .w(px(360.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(tokens.background_modifier_border)
                    .bg(tokens.background_secondary)
                    .p_6()
                    .child(div().text_size(px(24.)).child("TFT"))
                    .when(!signed_in, |panel| {
                        panel.child(button("Sign in with Google", &theme).when(!busy, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)))
                        }))
                    })
                    .when(signed_in, |panel| {
                        panel.child(button("Choose Drive folder", &theme).when(!busy, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| this.pick_vault(cx)))
                        }))
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

#[cfg(debug_assertions)]
impl DriveApp {
    /// Debug builds publish a small, token-free diagnostic object so a local
    /// browser can report why a Drive Vault did or did not produce files.
    fn publish_debug_state(&self, cx: &App) {
        use wasm_bindgen::JsValue;
        let path = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.read(cx).current_path().map(str::to_owned))
            .unwrap_or_default();
        let state = js_sys::Object::new();
        for (key, value) in [
            ("signedIn", JsValue::from_bool(self.signed_in)),
            ("busy", JsValue::from_bool(self.busy)),
            ("status", JsValue::from_str(&self.status)),
            ("vaultName", JsValue::from_str(&self.vault_name)),
            ("vaultId", JsValue::from_str(&self.vault_id)),
            ("entries", JsValue::from_f64(self.entries.len() as f64)),
            ("workspace", JsValue::from_bool(self.workspace.is_some())),
            ("activePath", JsValue::from_str(&path)),
        ] {
            let _ = js_sys::Reflect::set(&state, &JsValue::from_str(key), &value);
        }
        let _ = js_sys::Reflect::set(
            &js_sys::global(),
            &JsValue::from_str("__tftDriveDebug"),
            &state,
        );
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
