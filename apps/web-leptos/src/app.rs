//! Phase 1/2 web shell: Drive index, file tree, Markdown reading and source editing.

use crate::drive::{self, IndexedFile};
use leptos::ev;
use leptos::prelude::*;
use std::collections::HashSet;
use tft_app::explorer::{FileTree, SortOrder};
use tft_app::formats::paths::folder_of;
use tft_app::formats::FileKind;
use tft_app::markdown::html::percent_decode;
use tft_app::search::{fuzzy_match, Search, SearchFile};
use tft_app::{LeftTab, NodeId, Tab, VaultData, ViewMode, WorkspaceModel};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

fn render_markdown(text: &str) -> String {
    tft_app::markdown::render_html(&tft_app::markdown::parse(text))
}

fn image_srcs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<img") {
        let tag = &rest[at..];
        let end = tag.find('>').unwrap_or(tag.len());
        let tag = &tag[..end];
        if let Some(start) = tag.find("src=\"") {
            let value = &tag[start + 5..];
            if let Some(end) = value.find('"') {
                out.push(value[..end].to_string());
            }
        }
        rest = &rest[at + 4..];
    }
    out
}

fn resolve_asset<'a>(note: &str, src: &str, entries: &'a [IndexedFile]) -> Option<&'a IndexedFile> {
    if src.starts_with("http:") || src.starts_with("https:") || src.starts_with("data:") {
        return None;
    }
    let decoded = percent_decode(src);
    let clean = decoded.trim_start_matches("./").trim_start_matches('/');
    let folder = folder_of(note);
    let candidates = [
        decoded.to_string(),
        clean.to_string(),
        if folder.is_empty() {
            clean.to_string()
        } else {
            format!("{folder}/{clean}")
        },
    ];
    entries.iter().find(|entry| {
        candidates
            .iter()
            .any(|candidate| entry.path.eq_ignore_ascii_case(candidate))
    })
}

fn resolve_note_entry<'a>(
    note: &str,
    href: &str,
    entries: &'a [IndexedFile],
) -> Option<&'a IndexedFile> {
    let href = href.split('#').next().unwrap_or(href).trim();
    let href = percent_decode(href);
    let href = href.trim_start_matches("./").trim_start_matches('/');
    let folder = folder_of(note);
    let with_md = if href.to_ascii_lowercase().ends_with(".md") {
        href.to_string()
    } else {
        format!("{href}.md")
    };
    let relative = if folder.is_empty() {
        with_md.clone()
    } else {
        format!("{folder}/{with_md}")
    };
    let candidates = [href.to_string(), with_md.clone(), relative];
    entries.iter().find(|entry| {
        candidates
            .iter()
            .any(|candidate| entry.path.eq_ignore_ascii_case(candidate))
    })
}

async fn render_note_html(note: &str, text: &str, entries: &[IndexedFile]) -> String {
    let mut html = render_markdown(text);
    for src in image_srcs(&html) {
        let Some(entry) = resolve_asset(note, &src, entries) else {
            continue;
        };
        if let Ok(url) = drive::blob_url(&entry.id).await {
            html = html.replace(&format!("src=\"{src}\""), &format!("src=\"{url}\""));
        }
    }
    drive::sanitized(&html)
}

fn token_css(dark: bool) -> String {
    let t = if dark {
        tft_app::design::dark()
    } else {
        tft_app::design::light()
    };
    format!(
        ":root{{color-scheme:{}}}{}",
        if dark { "dark" } else { "light" },
        t.css_vars()
    )
}

fn note_title(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

#[derive(Clone, Debug)]
struct ContextMenu {
    node: NodeId,
    path: String,
    x: f64,
    y: f64,
}

#[component]
pub fn App() -> impl IntoView {
    let (client_id, api_key, app_id) = drive::config();
    let configured = !client_id.is_empty();
    let signed_in = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let status = RwSignal::new(if configured {
        "Sign in, then pick or enter a Drive folder.".to_string()
    } else {
        "Fill clientId in config.js.".to_string()
    });
    let log = RwSignal::new(String::new());
    let manual_id = RwSignal::new(String::new());
    let saved = RwSignal::new(drive::last_vault());

    let entries = RwSignal::new(Vec::<IndexedFile>::new());
    let vault_data = RwSignal::new(None::<VaultData>);
    let tree = RwSignal::new(FileTree::default());
    let sort = RwSignal::new(SortOrder::Alphabetical);
    let vault_name = RwSignal::new(String::new());
    let vault_id = RwSignal::new(String::new());

    let workspace = RwSignal::new(WorkspaceModel::new(ViewMode::Reading));
    let active_node = Memo::new(move |_| {
        workspace.with(|w| w.active().and_then(|g| g.active()).and_then(|t| t.node))
    });
    let active_path = Memo::new(move |_| {
        workspace.with(|w| {
            w.active()
                .and_then(|g| g.active())
                .and_then(|t| t.path.clone())
        })
    });
    let editing = Memo::new(move |_| {
        workspace.with(|w| {
            w.active()
                .and_then(|g| g.active())
                .is_some_and(|t| t.mode.is_editing())
        })
    });
    let set_editing = move |on: bool| {
        workspace.update(|w| {
            if let Some(tab) = w.active_mut().and_then(|g| g.active_mut()) {
                tab.mode = if on {
                    ViewMode::Source
                } else {
                    ViewMode::Reading
                };
            }
        });
    };
    let note_text = RwSignal::new(String::new());
    let note_html = RwSignal::new(String::new());
    let binary_url = RwSignal::new(None::<String>);
    let binary_kind = RwSignal::new(None::<FileKind>);
    let draft = RwSignal::new(String::new());
    let query = RwSignal::new(String::new());
    let settings_open = RwSignal::new(false);
    let switcher_open = RwSignal::new(false);
    let switcher_query = RwSignal::new(String::new());
    let save_tick = RwSignal::new(0u32);
    let context_menu = RwSignal::new(None::<ContextMenu>);
    let theme = RwSignal::new(drive::pref("theme", "system"));
    let spellcheck = RwSignal::new(drive::pref("spellcheck", "true") != "false");
    let system_dark = RwSignal::new(drive::system_is_dark());

    let note = move |msg: String| {
        log.update(|l| {
            l.push_str(&msg);
            l.push('\n');
        });
    };

    let key_handle = window_event_listener(ev::keydown, move |ev| {
        if (ev.ctrl_key() || ev.meta_key()) && ev.key().eq_ignore_ascii_case("o") {
            ev.prevent_default();
            switcher_query.set(String::new());
            switcher_open.set(true);
        }
        if (ev.ctrl_key() || ev.meta_key()) && ev.key().eq_ignore_ascii_case("s") {
            ev.prevent_default();
            save_tick.update(|n| *n += 1);
        }
        if (ev.ctrl_key() || ev.meta_key()) && ev.key().eq_ignore_ascii_case("e") {
            ev.prevent_default();
            if active_node.get().is_some() {
                draft.set(note_text.get());
                set_editing(!editing.get());
            }
        }
        if ev.key() == "Escape" {
            switcher_open.set(false);
            settings_open.set(false);
        }
    });
    on_cleanup(move || key_handle.remove());

    let open_vault = move |id: String, name: String| {
        busy.set(true);
        status.set(format!("Indexing {name}…"));
        note(format!("Indexing folder {id} ({name})…"));
        spawn_local(async move {
            match drive::index_vault(&id).await {
                Ok(index) => {
                    let data = index.data;
                    let count = index.entries.len();
                    let folder_count = data.folders().len();
                    let conflicts = data.conflicts().len();
                    let conflict_note = if conflicts == 0 {
                        String::new()
                    } else {
                        format!(", {conflicts} name conflicts")
                    };
                    let entries_for_search = index.entries.clone();
                    tree.set(FileTree::from_vault(&data));
                    tree.update(|t| t.expand_all());
                    entries.set(index.entries);
                    vault_data.set(Some(data));
                    vault_name.set(name.clone());
                    vault_id.set(id.clone());
                    drive::remember_vault(&id, &name);
                    saved.set(Some((id.clone(), name.clone())));
                    workspace.set(WorkspaceModel::new(ViewMode::Reading));
                    note_text.set(String::new());
                    note_html.set(String::new());
                    binary_url.set(None);
                    binary_kind.set(None);
                    draft.set(String::new());
                    set_editing(false);
                    query.set(String::new());
                    sort.set(SortOrder::Alphabetical);
                    status.set(format!(
                        "Indexed {count} files, {folder_count} folders{conflict_note}; reading notes…"
                    ));
                    note(format!(
                        "Indexed {count} files, {folder_count} folders{conflict_note}."
                    ));
                    spawn_local(async move {
                        let loaded = drive::load_markdown(&entries_for_search).await;
                        let notes = loaded.len();
                        vault_data.update(|data| {
                            let Some(data) = data else {
                                return;
                            };
                            for (node, text) in loaded {
                                data.set_text(node, text);
                            }
                        });
                        status.set(format!(
                            "Indexed {count} files, {folder_count} folders, {notes} notes searchable."
                        ));
                    });
                }
                Err(e) => {
                    status.set(format!("Index failed: {e}"));
                    note(format!("Index failed: {e}"));
                }
            }
            busy.set(false);
        });
    };

    let sign_in = {
        let client_id = client_id.clone();
        move |_| {
            let client_id = client_id.clone();
            busy.set(true);
            status.set("Waiting for Google sign-in…".into());
            spawn_local(async move {
                match drive::sign_in(&client_id).await {
                    Ok(()) => {
                        signed_in.set(true);
                        status.set("Signed in with drive.file.".into());
                        note("Signed in.".into());
                    }
                    Err(e) => {
                        status.set("Sign-in failed.".into());
                        note(format!("Sign-in failed: {e}"));
                        if e.contains("idpiframe") || e.contains("origin") {
                            note("Add this page to the OAuth client's Authorized JavaScript origins.".into());
                        }
                    }
                }
                busy.set(false);
            });
        }
    };

    let pick = {
        let api_key = api_key.clone();
        let app_id = app_id.clone();
        move |_| {
            let api_key = api_key.clone();
            let app_id = app_id.clone();
            busy.set(true);
            status.set("Opening Google Picker…".into());
            spawn_local(async move {
                match drive::pick_folder(&api_key, &app_id).await {
                    Ok((id, name)) => {
                        note(format!("Picked {name} ({id})."));
                        open_vault(id, name);
                        return;
                    }
                    Err(e) => {
                        status.set("Picker failed.".into());
                        note(format!("Picker failed: {e}"));
                        if e.contains("developer key") || e.contains("API key") {
                            note("Picker rejected the API key: enable Google Picker API, include it in the key's API restrictions, and use HTTP-referrer (not IP) restrictions.".into());
                        }
                        note("You can paste a folder ID below instead.".into());
                    }
                }
                busy.set(false);
            });
        }
    };

    let open_saved = move |_| {
        if let Some((id, name)) = saved.get() {
            open_vault(id, name);
        }
    };

    let open_manual = move |_| {
        let id = manual_id.get().trim().to_string();
        if id.is_empty() {
            note("Paste a Drive folder ID first.".into());
        } else {
            open_vault(id, "Drive folder".into());
        }
    };

    let create_test = move |_| {
        busy.set(true);
        status.set("Creating a TFT Test Vault in Drive…".into());
        spawn_local(async move {
            match drive::create_test_vault().await {
                Ok((id, name)) => open_vault(id, name),
                Err(e) => {
                    status.set("Test vault creation failed.".into());
                    note(format!("Create test vault failed: {e}"));
                    busy.set(false);
                }
            }
        });
    };

    let open_note = move |node: NodeId| {
        let Some(entry) = entries.with(|es| es.iter().find(|e| e.node == node).cloned()) else {
            return;
        };
        let path = entry.path.clone();
        let kind = entry.kind;
        let mode = workspace.with(|w| {
            w.active()
                .and_then(|g| g.tabs.iter().find(|t| t.node == Some(node)).map(|t| t.mode))
                .unwrap_or(ViewMode::Reading)
        });
        workspace.update(|w| {
            w.open_node(1, node, path.clone(), mode);
        });
        note_html.set(String::new());
        note_text.set(String::new());
        if matches!(
            kind,
            FileKind::Pdf | FileKind::Image | FileKind::Audio | FileKind::Video
        ) {
            binary_url.set(None);
            binary_kind.set(Some(kind));
            status.set(format!("Loading {path}…"));
            let note_path = path.clone();
            spawn_local(async move {
                match drive::blob_url(&entry.id).await {
                    Ok(url) => {
                        binary_url.set(Some(url));
                        status.set(format!("Viewing {note_path}"));
                    }
                    Err(e) => {
                        status.set("File load failed.".into());
                        note(format!("Load {note_path} failed: {e}"));
                    }
                }
            });
            return;
        }
        if kind != FileKind::Markdown {
            status.set("This file type is not viewable in the web shell yet.".into());
            return;
        }
        binary_url.set(None);
        binary_kind.set(None);
        status.set(format!("Reading {path}…"));
        let entries_snapshot = entries.get();
        let note_path = path.clone();
        spawn_local(async move {
            match drive::read_file(&entry.id).await {
                Ok(text) => {
                    let html = render_note_html(&note_path, &text, &entries_snapshot).await;
                    note_html.set(html);
                    note_text.set(text.clone());
                    draft.set(text);
                    status.set(format!("Reading {note_path}"));
                }
                Err(e) => {
                    status.set("Read failed.".into());
                    note(format!("Read {note_path} failed: {e}"));
                }
            }
        });
    };

    let toggle = move |key: String| {
        tree.update(|t| t.toggle_key(&key));
    };

    let new_tab = move |_| {
        workspace.update(|w| {
            if let Some(group) = w.active_mut() {
                group.tabs.push(Tab::empty(ViewMode::Reading));
                group.active_tab = group.tabs.len() - 1;
            }
        });
        note_text.set(String::new());
        note_html.set(String::new());
        binary_url.set(None);
        binary_kind.set(None);
        draft.set(String::new());
    };

    let close_tab = move |node: NodeId| {
        let was_active = active_node.get() == Some(node);
        workspace.update(|w| {
            let Some(group) = w.active_mut() else {
                return;
            };
            if let Some(ix) = group.tabs.iter().position(|t| t.node == Some(node)) {
                group.close(ix);
            }
        });
        if was_active {
            note_text.set(String::new());
            note_html.set(String::new());
            set_editing(false);
            if let Some(next) = active_node.get() {
                open_note(next);
            }
        }
    };

    let focus_node = move |node: NodeId, path: String| {
        let mode = workspace.with(|w| {
            w.active()
                .and_then(|g| g.tabs.iter().find(|t| t.node == Some(node)).map(|t| t.mode))
                .unwrap_or(ViewMode::Reading)
        });
        workspace.update(|w| {
            w.open_node(1, node, path, mode);
        });
    };

    let save_note = move |_: ()| {
        let Some(node) = active_node.get() else {
            return;
        };
        let Some(entry) = entries.with(|es| es.iter().find(|e| e.node == node).cloned()) else {
            return;
        };
        let body = draft.get();
        let note_path = entry.path.clone();
        let entries_snapshot = entries.get();
        busy.set(true);
        status.set("Saving…".into());
        spawn_local(async move {
            match drive::write_file(&entry.id, &body, entry.etag.as_deref()).await {
                Ok(etag) => {
                    entries.update(|es| {
                        if let Some(e) = es.iter_mut().find(|e| e.node == node) {
                            if etag.is_some() {
                                e.etag = etag;
                            }
                        }
                    });
                    let html = render_note_html(&note_path, &body, &entries_snapshot).await;
                    note_html.set(html);
                    vault_data.update(|data| {
                        if let Some(data) = data {
                            data.set_text(node, body.clone());
                        }
                    });
                    note_text.set(body);
                    set_editing(false);
                    status.set("Saved to Drive.".into());
                }
                Err(e) => {
                    status.set("Save failed.".into());
                    note(format!("Save failed: {e}"));
                }
            }
            busy.set(false);
        });
    };

    let switcher_results = move || {
        let q = switcher_query.get().trim().to_ascii_lowercase();
        entries.with(|es| {
            if q.is_empty() {
                es.iter().take(20).cloned().collect::<Vec<_>>()
            } else {
                es.iter()
                    .filter(|e| fuzzy_match(&q, &e.path).is_some())
                    .take(20)
                    .cloned()
                    .collect()
            }
        })
    };

    Effect::new(move |_| {
        if save_tick.get() > 0 && editing.get() {
            save_note(());
        }
    });

    let reindex = move || {
        let id = vault_id.get();
        let name = vault_name.get();
        if !id.is_empty() {
            open_vault(id, name);
        }
    };

    let new_note = move |_: ()| {
        let root = vault_id.get();
        if root.is_empty() {
            note("Open a vault first.".into());
            return;
        }
        let mut name = drive::ask("New note name", "Untitled");
        name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if !name.to_ascii_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        busy.set(true);
        status.set(format!("Creating {name}…"));
        spawn_local(async move {
            match drive::create_note(&root, &name).await {
                Ok(()) => {
                    note(format!("Created {name}."));
                    reindex();
                }
                Err(e) => {
                    status.set("Create failed.".into());
                    note(format!("Create {name} failed: {e}"));
                    busy.set(false);
                }
            }
        });
    };

    let rename_note = move |_: ()| {
        let Some(node) = active_node.get() else {
            return;
        };
        let Some(entry) = entries.with(|es| es.iter().find(|e| e.node == node).cloned()) else {
            return;
        };
        let current = entry
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&entry.path)
            .to_string();
        let mut name = drive::ask("Rename note", &current);
        name = name.trim().to_string();
        if name.is_empty() || name == current {
            return;
        }
        if !name.to_ascii_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        busy.set(true);
        status.set("Renaming…".into());
        spawn_local(async move {
            match drive::rename_entry(&entry.id, &name).await {
                Ok(()) => {
                    note(format!("Renamed to {name}."));
                    close_tab(node);
                    reindex();
                }
                Err(e) => {
                    status.set("Rename failed.".into());
                    note(format!("Rename failed: {e}"));
                    busy.set(false);
                }
            }
        });
    };

    let delete_note = move |_: ()| {
        let Some(node) = active_node.get() else {
            return;
        };
        let Some(entry) = entries.with(|es| es.iter().find(|e| e.node == node).cloned()) else {
            return;
        };
        let path = entry.path.clone();
        if !drive::confirm(&format!("Delete {path}?")) {
            return;
        }
        busy.set(true);
        status.set("Deleting…".into());
        spawn_local(async move {
            match drive::delete_entry(&entry.id).await {
                Ok(()) => {
                    note(format!("Deleted {path}."));
                    close_tab(node);
                    reindex();
                }
                Err(e) => {
                    status.set("Delete failed.".into());
                    note(format!("Delete failed: {e}"));
                    busy.set(false);
                }
            }
        });
    };

    let rows = move || tree.with(|t| t.rows().cloned().collect::<Vec<_>>());
    let results = move || {
        let q = query.get().trim().to_string();
        if q.is_empty() {
            return Vec::new();
        }
        let lower = q.to_ascii_lowercase();
        let mut hits = HashSet::new();
        if let Ok(search) = Search::new(&q) {
            vault_data.with(|data| {
                let Some(data) = data else {
                    return;
                };
                for (id, content) in data.text_entries() {
                    let Some(path) = data.path(id) else {
                        continue;
                    };
                    let file = SearchFile {
                        path,
                        content: content.as_str(),
                        ..SearchFile::default()
                    };
                    if search.matches(&file).is_some() {
                        hits.insert(id);
                    }
                }
            });
        }
        entries.with(|es| {
            es.iter()
                .filter(|e| fuzzy_match(&lower, &e.path).is_some() || hits.contains(&e.node))
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let effective_dark = move || match theme.get().as_str() {
        "dark" => true,
        "light" => false,
        _ => system_dark.get(),
    };

    view! {
        <style>{move || token_css(effective_dark())}</style>
        <div class="app">
            <header>
                <div class="brand">"TFT"</div>
                <div class="vault-name">{move || vault_name.get()}</div>
                <div class="status">{move || status.get()}</div>
                <button class="icon-button" title="Settings" on:click=move |_| settings_open.set(true)>
                    <span class="lucide" style="--icon:url('/assets/icons/settings.svg')"></span>
                </button>
                <button on:click=sign_in disabled=move || busy.get()>
                    {move || if signed_in.get() { "Signed in" } else { "Sign in" }}
                </button>
                <button on:click=pick disabled=move || busy.get() || !signed_in.get()>"Pick vault"</button>
                <button on:click=open_saved disabled=move || busy.get() || !signed_in.get() || saved.get().is_none()>
                    "Reopen"
                </button>
            </header>

            <div class="tab-strip">
                {move || {
                    workspace.with(|w| {
                        w.active()
                            .map(|g| g.tabs.clone())
                            .unwrap_or_default()
                    })
                    .into_iter()
                    .map(|tab| {
                        let node = tab.node;
                        let path = tab.path.clone();
                        let path_key = path.clone().unwrap_or_default();
                        let active = node.is_some() && active_node.get() == node;
                        let dirty = active && editing.get() && draft.get() != note_text.get();
                        let open = move |_| {
                            if let Some(node) = node {
                                open_note(node);
                            }
                        };
                        let close = move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            if let Some(node) = node {
                                close_tab(node);
                            }
                        };
                        view! {
                            <div class=("tab", active) title=path_key on:click=open>
                                <span class="lucide" style="--icon:url('/assets/icons/file-text.svg')"></span>
                                <span class="tab-title">{tab.default_title()}</span>
                                {dirty.then_some("•")}
                                <button class="tab-close" on:click=close>"×"</button>
                            </div>
                        }
                    })
                    .collect_view()
                }}
                <button class="tab-add" title="New tab" on:click=new_tab>"+"</button>
            </div>

            {if configured {
                ().into_any()
            } else {
                view! { <div class="banner">"Fill clientId in config.js."</div> }.into_any()
            }}

            <div class="open-id">
                <input
                    placeholder="Drive folder ID (fallback when Picker fails)"
                    prop:value=move || manual_id.get()
                    on:input=move |ev| manual_id.set(event_target_value(&ev))
                />
                <button on:click=open_manual disabled=move || busy.get() || !signed_in.get()>"Open ID"</button>
                <button on:click=create_test disabled=move || busy.get() || !signed_in.get()>
                    "Create test vault"
                </button>
            </div>

            <div class="workspace">
                <aside>
                    <div class="dock-tabs">
                        <button
                            class=("dock-tab", move || workspace.with(|w| w.left_tab) == LeftTab::Files)
                            on:click=move |_| workspace.update(|w| w.left_tab = LeftTab::Files)
                        >
                            "Files"
                        </button>
                        <button
                            class=("dock-tab", move || workspace.with(|w| w.left_tab) == LeftTab::Search)
                            on:click=move |_| workspace.update(|w| w.left_tab = LeftTab::Search)
                        >
                            "Search"
                        </button>
                    </div>
                    {move || match workspace.with(|w| w.left_tab) {
                        LeftTab::Files => {
                            let list = rows()
                                .into_iter()
                                .map(|row| {
                                    let path = row.path.clone();
                                    let key = row.key.clone();
                                    let node = row.node;
                                    let is_folder = row.is_folder;
                                    let conflict = row.conflict;
                                    let selected_row = node.is_some() && active_node.get() == node;
                                    let collapsed = is_folder && tree.with(|t| t.is_collapsed_key(&key));
                                    let style = format!("padding-left:{}px", 8 + row.depth * 14);
                                    let click = {
                                        let key = key.clone();
                                        move |_| {
                                            if is_folder {
                                                toggle(key.clone());
                                            } else if conflict {
                                                status.set("Two items share this name; rename one first.".into());
                                            } else if let Some(node) = node {
                                                open_note(node);
                                            }
                                        }
                                    };
                                    let menu = {
                                        let path = path.clone();
                                        move |ev: web_sys::MouseEvent| {
                                            let Some(node) = node else {
                                                return;
                                            };
                                            if is_folder || conflict {
                                                return;
                                            }
                                            ev.prevent_default();
                                            context_menu.set(Some(ContextMenu {
                                                node,
                                                path: path.clone(),
                                                x: ev.client_x() as f64,
                                                y: ev.client_y() as f64,
                                            }));
                                        }
                                    };
                                    view! {
                                        <li
                                            class=("tree-row", selected_row)
                                            class=("conflict", conflict)
                                            style=style
                                            on:click=click
                                            on:contextmenu=menu
                                        >
                                            <span class="twisty">
                                                {if is_folder { if collapsed { "▸" } else { "▾" } } else { "" }}
                                            </span>
                                            <span
                                                class="lucide"
                                                style=format!("--icon:url('/assets/icons/{}.svg')", if is_folder { "folder" } else { "file-text" })
                                            ></span>
                                            <span class="name">{row.name}</span>
                                            {row.tag.map(|tag| view! { <span class="tag">{tag}</span> })}
                                        </li>
                                    }
                                })
                                .collect_view();
                            view! {
                                <div class="dock-pane">
                                    <div class="dock-header">
                                        <span>"Files"</span>
                                        <select
                                            class="sort"
                                            on:change=move |ev| {
                                                if let Some(order) = SortOrder::parse(&event_target_value(&ev)) {
                                                    sort.set(order);
                                                    tree.update(|t| t.set_sort(order));
                                                }
                                            }
                                        >
                                            {SortOrder::ALL.into_iter().map(|order| {
                                                view! {
                                                    <option value=order.as_str() selected=move || sort.get() == order>
                                                        {order.label()}
                                                    </option>
                                                }
                                            }).collect_view()}
                                        </select>
                                        <button
                                            class="icon-button"
                                            title="Collapse all"
                                            on:click=move |_| tree.update(|t| t.collapse_all())
                                        >"⊟"</button>
                                        <button
                                            class="icon-button"
                                            title="Expand all"
                                            on:click=move |_| tree.update(|t| t.expand_all())
                                        >"⊞"</button>
                                        <span>{move || entries.get().len()}</span>
                                    </div>
                                    <ul class="tree">{list}</ul>
                                </div>
                            }.into_any()
                        }
                        LeftTab::Search => {
                            let list = results()
                                .into_iter()
                                .map(|file| {
                                    let path = file.path.clone();
                                    let node = file.node;
                                    let selected_row = active_path.get().as_deref() == Some(path.as_str());
                                    let open = move |_| open_note(node);
                                    view! {
                                        <li class=("tree-row", selected_row) on:click=open>
                                            <span class="name">{file.path}</span>
                                        </li>
                                    }
                                })
                                .collect_view();
                            view! {
                                <div class="dock-pane">
                                    <input
                                        class="dock-search"
                                        placeholder="Search files"
                                        prop:value=move || query.get()
                                        on:input=move |ev| query.set(event_target_value(&ev))
                                    />
                                    <ul class="tree">{list}</ul>
                                </div>
                            }.into_any()
                        }
                    }}
                </aside>

                <main class="note">
                    {move || {
                        if let Some(url) = binary_url.get() {
                            let path = active_path.get().unwrap_or_default();
                            let heading = note_title(&path);
                            let kind = binary_kind.get();
                            let body = match kind {
                                Some(FileKind::Pdf) => view! {
                                    <iframe class="pdf-view" src=url title=heading.clone()></iframe>
                                }.into_any(),
                                Some(FileKind::Image) => view! {
                                    <img class="image-view" src=url alt=heading.clone() />
                                }.into_any(),
                                Some(FileKind::Audio) => view! {
                                    <audio class="media-view" controls src=url></audio>
                                }.into_any(),
                                Some(FileKind::Video) => view! {
                                    <video class="media-view" controls src=url></video>
                                }.into_any(),
                                _ => view! { <div class="empty">"Unsupported file."</div> }.into_any(),
                            };
                            return view! {
                                <div class="note-head">
                                    <div>
                                        <h1>{heading}</h1>
                                        <div class="path">{path}</div>
                                    </div>
                                </div>
                                {body}
                            }.into_any();
                        }
                        let Some(path) = active_path.get() else {
                            return view! {
                                <div class="empty">
                                    <h1>"Open a vault"</h1>
                                    <p>"Sign in, pick a Drive folder, then choose a note."</p>
                                </div>
                            }.into_any();
                        };
                        let heading = note_title(&path);
                        view! {
                            <div class="note-head">
                                <div>
                                    <h1>{heading}</h1>
                                    <div class="path">{path}</div>
                                </div>
                                <div class="note-actions">
                                    <button on:click=move |_| new_note(()) disabled=move || busy.get()>"New"</button>
                                    <button on:click=move |_| rename_note(()) disabled=move || busy.get()>"Rename"</button>
                                    <button on:click=move |_| delete_note(()) disabled=move || busy.get()>"Delete"</button>
                                    <button
                                        on:click=move |_| {
                                            draft.set(note_text.get());
                                            set_editing(!editing.get());
                                        }
                                        disabled=move || busy.get()
                                    >
                                        {move || if editing.get() { "Reading" } else { "Edit" }}
                                    </button>
                                </div>
                            </div>
                            {move || {
                                if editing.get() {
                                    view! {
                                        <div class="editor">
                                            <textarea
                                                prop:value=move || draft.get()
                                                spellcheck=move || if spellcheck.get() { "true" } else { "false" }
                                                on:input=move |ev| draft.set(event_target_value(&ev))
                                                on:blur=move |_| save_note(())
                                            ></textarea>
                                            <div class="editor-actions">
                                                <button on:click=move |_| save_note(()) disabled=move || busy.get()>"Save"</button>
                                                <button
                                                    on:click=move |_| {
                                                        draft.set(note_text.get());
                                                        set_editing(false);
                                                    }
                                                    disabled=move || busy.get()
                                                >
                                                    "Cancel"
                                                </button>
                                            </div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <article
                                            class="markdown"
                                            inner_html=move || note_html.get()
                                            on:click=move |ev: web_sys::MouseEvent| {
                                                let Some(target) = ev.target() else {
                                                    return;
                                                };
                                                let href = drive::link_at(JsValue::from(target));
                                                if href.is_empty() {
                                                    return;
                                                }
                                                ev.prevent_default();
                                                ev.stop_propagation();
                                                let entries_now = entries.get();
                                                let Some(note) = active_path.get() else {
                                                    return;
                                                };
                                                if let Some(entry) = resolve_note_entry(&note, &href, &entries_now) {
                                                    open_note(entry.node);
                                                }
                                            }
                                        ></article>
                                    }.into_any()
                                }
                            }}
                        }.into_any()
                    }}
                </main>
            </div>

            {move || switcher_open.get().then(|| {
                let list = switcher_results()
                    .into_iter()
                    .map(|file| {
                        let node = file.node;
                        let open = move |_| {
                            open_note(node);
                            switcher_open.set(false);
                        };
                        view! {
                            <li class="switcher-row" on:click=open>
                                <span>{note_title(&file.path)}</span>
                                <span class="path">{file.path}</span>
                            </li>
                        }
                    })
                    .collect_view();
                view! {
                    <div class="modal-backdrop" on:click=move |_| switcher_open.set(false)>
                        <div class="switcher" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                            <input
                                class="switcher-input"
                                placeholder="Open a note"
                                prop:value=move || switcher_query.get()
                                on:input=move |ev| switcher_query.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        if let Some(first) = switcher_results().first() {
                                            open_note(first.node);
                                            switcher_open.set(false);
                                        }
                                    }
                                }
                            />
                            <ul class="switcher-list">{list}</ul>
                        </div>
                    </div>
                }
            })}

            {move || settings_open.get().then(|| {
                view! {
                    <div class="modal-backdrop" on:click=move |_| settings_open.set(false)>
                        <div class="settings" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                            <div class="settings-head">
                                <h2>"Settings"</h2>
                                <button class="icon-button" on:click=move |_| settings_open.set(false)>"×"</button>
                            </div>
                            <section>
                                <h3>"Appearance"</h3>
                                <div class="setting-row">
                                    <div>
                                        <div>"Theme"</div>
                                        <div class="desc">"Light, Dark or follow the desktop."</div>
                                    </div>
                                    <div class="segmented">
                                        <button
                                            class=("seg", move || theme.get() == "dark")
                                            on:click=move |_| {
                                                theme.set("dark".into());
                                                drive::set_pref_value("theme", "dark");
                                            }
                                        >"Dark"</button>
                                        <button
                                            class=("seg", move || theme.get() == "light")
                                            on:click=move |_| {
                                                theme.set("light".into());
                                                drive::set_pref_value("theme", "light");
                                            }
                                        >"Light"</button>
                                        <button
                                            class=("seg", move || theme.get() == "system")
                                            on:click=move |_| {
                                                system_dark.set(drive::system_is_dark());
                                                theme.set("system".into());
                                                drive::set_pref_value("theme", "system");
                                            }
                                        >"System"</button>
                                    </div>
                                </div>
                            </section>
                            <section>
                                <h3>"Editor"</h3>
                                <label class="setting-row">
                                    <div>
                                        <div>"Spellcheck"</div>
                                        <div class="desc">"Use the browser's spell checker in Source mode."</div>
                                    </div>
                                    <input
                                        type="checkbox"
                                        prop:checked=move || spellcheck.get()
                                        on:change=move |ev| {
                                            let on = event_target_checked(&ev);
                                            spellcheck.set(on);
                                            drive::set_pref_value("spellcheck", if on { "true" } else { "false" });
                                        }
                                    />
                                </label>
                            </section>
                        </div>
                    </div>
                }
            })}

            {move || context_menu.get().map(|menu| {
                let menu_node = menu.node;
                let rename_path = menu.path.clone();
                let delete_path = menu.path.clone();
                let copy_path = menu.path.clone();
                view! {
                    <div class="context-backdrop" on:click=move |_| context_menu.set(None)></div>
                    <div
                        class="context-menu"
                        style=format!("left:{}px;top:{}px", menu.x, menu.y)
                        on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()
                    >
                        <button on:click=move |_| {
                            context_menu.set(None);
                            open_note(menu_node);
                        }>"Open"</button>
                        <button on:click=move |_| {
                            context_menu.set(None);
                            focus_node(menu_node, rename_path.clone());
                            rename_note(());
                        }>"Rename"</button>
                        <button on:click=move |_| {
                            context_menu.set(None);
                            focus_node(menu_node, delete_path.clone());
                            delete_note(());
                        }>"Delete"</button>
                        <button on:click=move |_| {
                            context_menu.set(None);
                            let path = copy_path.clone();
                            spawn_local(async move {
                                let _ = drive::copy_to_clipboard(&path).await;
                            });
                        }>"Copy path"</button>
                    </div>
                }
            })}

            <details class="log-panel">
                <summary>"Log"</summary>
                <pre>{move || log.get()}</pre>
            </details>
        </div>
    }
}
