//! Browser Google Drive adapter shared by TFT's browser frontends.

#![cfg(target_arch = "wasm32")]

use serde::Deserialize;
use tft_app::formats::config::AppearanceConfig;
use tft_app::formats::files::VaultFile;
use tft_app::formats::FileKind;
use tft_domain::vault::{BackendObjectId, FileMeta, NodeId, NodeKind, ScannedNode, VaultData};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/drive.js")]
extern "C" {
    #[wasm_bindgen(js_name = getConfig)]
    fn get_config() -> String;
    #[wasm_bindgen(js_name = modifierKeyLabel)]
    fn modifier_key_label_js() -> String;
    #[wasm_bindgen(js_name = saveConfig)]
    fn save_config_js(client_id: &str, api_key: &str) -> bool;
    #[wasm_bindgen(js_name = initTokenClient, catch)]
    fn init_token_client(client_id: &str) -> Result<(), JsValue>;
    #[wasm_bindgen(js_name = requestToken)]
    fn request_token() -> js_sys::Promise;
    #[wasm_bindgen(js_name = tryResumeToken)]
    fn try_resume_token(client_id: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = pickFolder)]
    fn pick_folder_js(api_key: &str, app_id: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = driveFetch)]
    fn drive_fetch(url: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = updateTextFile)]
    fn update_text_file(id: &str, content: &str, etag: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = driveBlobUrl)]
    fn drive_blob_url(id: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = driveFileBytes)]
    fn drive_file_bytes(id: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = toggleAudio)]
    fn toggle_audio_js(id: &str, url: &str) -> String;
    #[wasm_bindgen(js_name = audioState)]
    fn audio_state_js(id: &str) -> String;
    #[wasm_bindgen(js_name = seekAudio)]
    fn seek_audio_js(id: &str, position: f64);
    #[wasm_bindgen(js_name = toggleAudioMute)]
    fn toggle_audio_mute_js(id: &str, url: &str) -> String;
    #[wasm_bindgen(js_name = openVideoPlayer)]
    fn open_video_player_js(url: &str, title: &str);
    #[wasm_bindgen(js_name = openDriveFile)]
    fn open_drive_file_js(id: &str);
    #[wasm_bindgen(js_name = createFolder)]
    fn create_folder(name: &str, parent: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = createTextFile)]
    fn create_text_file(name: &str, parent: &str, content: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = renameFile)]
    fn rename_file(id: &str, name: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = deleteFile)]
    fn delete_file(id: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = duplicateFile)]
    fn duplicate_file(id: &str, parent: &str, name: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = moveFile)]
    fn move_file(id: &str, add_parent: &str, remove_parent: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = trashFile)]
    fn trash_file(id: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = internalLinkAt)]
    fn internal_link_at(target: JsValue) -> String;
    #[wasm_bindgen(js_name = copyText)]
    fn copy_text(text: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = confirmAction)]
    fn confirm_action(message: &str) -> bool;
    #[wasm_bindgen(js_name = savedVault)]
    fn saved_vault() -> String;
    #[wasm_bindgen(js_name = saveVault)]
    fn save_vault(id: &str, name: &str);
    #[wasm_bindgen(js_name = savedWorkspace)]
    fn saved_workspace(root_id: &str) -> String;
    #[wasm_bindgen(js_name = saveWorkspace)]
    fn save_workspace(root_id: &str, state: &str) -> bool;
    #[wasm_bindgen(js_name = clearWorkspace)]
    fn clear_workspace(root_id: &str, state: &str);
    #[wasm_bindgen(js_name = getPref)]
    fn get_pref(key: &str, fallback: &str) -> String;
    #[wasm_bindgen(js_name = setPref)]
    fn set_pref(key: &str, value: &str);
    #[wasm_bindgen(js_name = prefersDark)]
    fn prefers_dark() -> bool;
    #[wasm_bindgen(js_name = sanitizeHtml)]
    fn sanitize_html(html: &str) -> String;
}

#[derive(Clone, Debug, Deserialize)]
pub struct DriveItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub size: Option<String>,
    #[serde(rename = "modifiedTime")]
    pub modified_time: Option<String>,
    #[serde(rename = "createdTime")]
    pub created_time: Option<String>,
    pub etag: Option<String>,
}

impl DriveItem {
    pub fn modified_millis(&self) -> u64 {
        millis(&self.modified_time)
    }

    pub fn created_millis(&self) -> u64 {
        millis(&self.created_time)
    }
}

#[derive(Clone, Debug)]
pub struct IndexedFile {
    pub node: NodeId,
    pub path: String,
    pub kind: FileKind,
    pub id: String,
    pub etag: Option<String>,
}

#[derive(Debug, Default)]
pub struct VaultIndex {
    pub data: VaultData,
    pub files: Vec<VaultFile>,
    pub entries: Vec<IndexedFile>,
    pub folders: Vec<String>,
}

#[derive(Deserialize)]
struct ListPage {
    #[serde(default)]
    files: Vec<DriveItem>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

fn text(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&value, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| format!("{value:?}"))
}

fn encode(value: &str) -> String {
    js_sys::encode_uri_component(value)
        .as_string()
        .unwrap_or_default()
}

fn millis(value: &Option<String>) -> u64 {
    value
        .as_deref()
        .map(js_sys::Date::parse)
        .filter(|n| n.is_finite() && *n > 0.0)
        .map(|n| n as u64)
        .unwrap_or(0)
}

async fn fetch(url: &str) -> Result<String, String> {
    JsFuture::from(drive_fetch(url))
        .await
        .map_err(text)?
        .as_string()
        .ok_or_else(|| "Drive returned a non-text response".to_string())
}

fn list_url(folder_id: &str, page: Option<&str>) -> String {
    let q = format!("'{folder_id}' in parents and trashed=false");
    let mut url = format!(
        "https://www.googleapis.com/drive/v3/files?q={}&pageSize=1000&supportsAllDrives=true&includeItemsFromAllDrives=true&fields=files(id,name,mimeType,size,modifiedTime,createdTime),nextPageToken",
        encode(&q)
    );
    if let Some(page) = page {
        url.push_str("&pageToken=");
        url.push_str(&encode(page));
    }
    url
}

/// Every page of one folder's direct children. The caller controls how many
/// folders are in flight at once.
async fn list_children(folder_id: &str) -> Result<Vec<DriveItem>, String> {
    let mut files = Vec::new();
    let mut page = None;
    loop {
        let body = fetch(&list_url(folder_id, page.as_deref())).await?;
        let parsed: ListPage =
            serde_json::from_str(&body).map_err(|e| format!("list parse: {e}"))?;
        files.extend(parsed.files);
        page = parsed.next_page_token;
        if page.is_none() {
            break;
        }
    }
    Ok(files)
}

/// One folder's direct children, as nodes ready for `VaultData`. This is the
/// unit of a lazy tree: a folder is listed the first time the user expands it.
pub async fn list_folder(
    folder_id: &str,
    parent: &BackendObjectId,
) -> Result<Vec<ScannedNode>, String> {
    let mut scanned = Vec::new();
    for item in list_children(folder_id).await? {
        if item.name.starts_with('.') {
            continue;
        }
        match item.mime_type.as_deref() {
            Some("application/vnd.google-apps.folder") => {
                let id = item.id.clone();
                scanned.push(ScannedNode {
                    backend: BackendObjectId::Drive(id),
                    parent: Some(parent.clone()),
                    name: item.name,
                    kind: NodeKind::Folder,
                    revision: item.etag,
                });
            }
            Some(mime) if mime.starts_with("application/vnd.google-apps.") => {}
            _ => {
                let mtime = millis(&item.modified_time);
                scanned.push(ScannedNode {
                    backend: BackendObjectId::Drive(item.id),
                    parent: Some(parent.clone()),
                    name: item.name.clone(),
                    kind: NodeKind::File(FileMeta {
                        kind: FileKind::from_path(&item.name),
                        size: item
                            .size
                            .as_deref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        mtime,
                        ctime: millis(&item.created_time).max(mtime),
                    }),
                    revision: item.etag,
                });
            }
        }
    }
    Ok(scanned)
}

pub async fn index_vault(root_id: &str) -> Result<VaultIndex, String> {
    let meta = fetch(&format!(
        "https://www.googleapis.com/drive/v3/files/{root_id}?fields=id,name,mimeType&supportsAllDrives=true"
    ))
    .await?;
    let meta: serde_json::Value =
        serde_json::from_str(&meta).map_err(|e| format!("folder metadata: {e}"))?;
    if meta.get("mimeType").and_then(|v| v.as_str()) != Some("application/vnd.google-apps.folder") {
        return Err(format!(
            "Drive id is not a folder: {}",
            meta.get("mimeType")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ));
    }

    let mut index = VaultIndex {
        data: VaultData::new("drive"),
        ..VaultIndex::default()
    };
    index
        .data
        .set_root_backend(BackendObjectId::Drive(root_id.to_string()));
    let scanned = list_folder(root_id, &BackendObjectId::Drive(root_id.to_string())).await?;
    index.data.apply_partial_scan(scanned);
    let entries = index
        .data
        .file_ids()
        .into_iter()
        .filter_map(|(node, file)| {
            let BackendObjectId::Drive(id) = index.data.backend(node)? else {
                return None;
            };
            Some(IndexedFile {
                node,
                path: file.path,
                kind: file.kind,
                id: id.clone(),
                etag: None,
            })
        })
        .collect();
    index.files = index.data.files();
    index.folders = index.data.folders();
    index.entries = entries;
    index.folders.sort();
    index.files.sort_by(|a, b| a.path.cmp(&b.path));
    index.entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(index)
}

/// Reads the root `.tft/appearance.json` without adding hidden
/// configuration files to the visible Vault tree.
pub async fn read_appearance_config(root_id: &str) -> Result<AppearanceConfig, String> {
    let tft_folder = find_child(root_id, ".tft", true).await?;
    let Some(tft_folder) = tft_folder else {
        return Ok(AppearanceConfig::default());
    };
    let appearance = find_child(&tft_folder.id, "appearance.json", false).await?;
    let Some(appearance) = appearance else {
        return Ok(AppearanceConfig::default());
    };
    let contents = read_file(&appearance.id).await?;
    serde_json::from_str(&contents).map_err(|error| format!("appearance.json: {error}"))
}

/// Reads a typed configuration document inside the root `.tft` folder.
/// Missing files use the format defaults; malformed files stay visible as an
/// error to callers instead of silently becoming writable defaults.
async fn read_config<T: for<'de> Deserialize<'de> + Default>(
    root_id: &str,
    file_name: &str,
) -> Result<T, String> {
    let Some(tft_folder) = find_child(root_id, ".tft", true).await? else {
        return Ok(T::default());
    };
    let Some(file) = find_child(&tft_folder.id, file_name, false).await? else {
        return Ok(T::default());
    };
    let contents = read_file(&file.id).await?;
    serde_json::from_str(contents.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("{file_name}: {error}"))
}

pub async fn read_app_config(root_id: &str) -> Result<tft_formats::config::AppConfig, String> {
    read_config(root_id, "app.json").await
}

pub async fn read_switcher_config(
    root_id: &str,
) -> Result<tft_formats::config::SwitcherConfig, String> {
    read_config(root_id, "switcher.json").await
}

pub async fn read_page_preview_config(
    root_id: &str,
) -> Result<tft_formats::config::PagePreviewConfig, String> {
    read_config(root_id, "page-preview.json").await
}

pub async fn read_hotkeys_config(root_id: &str) -> Result<tft_formats::config::Hotkeys, String> {
    read_config(root_id, "hotkeys.json").await
}

pub async fn read_graph_config(
    root_id: &str,
) -> Result<Option<tft_formats::config::GraphConfig>, String> {
    let Some(folder) = find_child(root_id, ".tft", true).await? else {
        return Ok(None);
    };
    let Some(file) = find_child(&folder.id, "graph.json", false).await? else {
        return Ok(None);
    };
    let contents = read_file(&file.id).await?;
    serde_json::from_str(contents.trim_start_matches('\u{feff}'))
        .map(Some)
        .map_err(|error| format!("graph.json: {error}"))
}

/// Writes the shared Graph settings to `.tft/graph.json`, preserving unknown
/// keys, BOM, and CRLF/LF. Malformed existing JSON is never overwritten.
pub async fn write_graph_config(
    root_id: &str,
    config: &tft_formats::config::GraphConfig,
) -> Result<(), String> {
    let folder = match find_child(root_id, ".tft", true).await? {
        Some(folder) => folder.id,
        None => create_entry(".tft", root_id, None).await?,
    };
    let existing = find_child(&folder, "graph.json", false).await?;
    let (document, file_id, bom, crlf) = if let Some(file) = existing {
        let raw = read_file(&file.id).await?;
        let bom = raw.starts_with('\u{feff}');
        let source = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        let crlf = source.contains("\r\n");
        let document: serde_json::Value = serde_json::from_str(source)
            .map_err(|error| format!("graph.json was not changed: {error}"))?;
        let Some(object) = document.as_object() else {
            return Err("graph.json was not changed: root must be an object".into());
        };
        (
            serde_json::Value::Object(object.clone()),
            Some(file.id),
            bom,
            crlf,
        )
    } else {
        (config.full_document(), None, false, false)
    };
    let mut document = document;
    let update = if file_id.is_some() {
        config.merge_document()
    } else {
        serde_json::Value::Null
    };
    if let Some(object) = document.as_object_mut() {
        if let Some(updates) = update.as_object() {
            object.extend(updates.clone());
        }
    }
    let mut output = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    if crlf {
        output = output.replace('\n', "\r\n");
    }
    if bom {
        output.insert(0, '\u{feff}');
    }
    if let Some(file_id) = file_id {
        write_file(&file_id, &output, None).await?;
    } else {
        create_entry("graph.json", &folder, Some(&output)).await?;
    }
    Ok(())
}

async fn find_child(parent: &str, name: &str, folder: bool) -> Result<Option<DriveItem>, String> {
    let query = format!("'{parent}' in parents and name = '{name}' and trashed = false");
    let url = format!(
        "https://www.googleapis.com/drive/v3/files?q={}&pageSize=1000&supportsAllDrives=true&includeItemsFromAllDrives=true&fields=files(id,name,mimeType),nextPageToken",
        encode(&query)
    );
    let body = fetch(&url).await?;
    let page: ListPage = serde_json::from_str(&body).map_err(|error| error.to_string())?;
    Ok(page.files.into_iter().find(|item| {
        item.name == name
            && item
                .mime_type
                .as_deref()
                .is_some_and(|mime| (mime == "application/vnd.google-apps.folder") == folder)
    }))
}

/// Reads only the Web workspace extension, leaving the native workspace layout
/// opaque so each host can keep its own layout representation.
pub async fn read_web_workspace_state(root_id: &str) -> Result<Option<String>, String> {
    let Some(folder) = find_child(root_id, ".tft", true).await? else {
        return Ok(None);
    };
    let Some(file) = find_child(&folder.id, "workspace.json", false).await? else {
        return Ok(None);
    };
    let contents = read_file(&file.id).await?;
    let parsed: serde_json::Value = serde_json::from_str(contents.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("workspace.json: {error}"))?;
    Ok(parsed.get("tftWeb").map(ToString::to_string))
}

/// Updates the Web extension while preserving all existing workspace keys and
/// the file's BOM and newline convention. Invalid JSON is never overwritten.
pub async fn write_web_workspace_state(root_id: &str, state_json: &str) -> Result<(), String> {
    let state: serde_json::Value = serde_json::from_str(state_json)
        .map_err(|error| format!("Web workspace state: {error}"))?;
    let Some(state) = state.as_object() else {
        return Err("Web workspace state must be a JSON object".into());
    };
    let folder = match find_child(root_id, ".tft", true).await? {
        Some(folder) => folder.id,
        None => create_entry(".tft", root_id, None).await?,
    };
    let existing = find_child(&folder, "workspace.json", false).await?;
    let (mut document, file_id, bom, crlf) = if let Some(file) = existing {
        let raw = read_file(&file.id).await?;
        let bom = raw.starts_with('\u{feff}');
        let source = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        let crlf = source.contains("\r\n");
        let document: serde_json::Value = serde_json::from_str(source)
            .map_err(|error| format!("workspace.json was not changed: {error}"))?;
        let Some(object) = document.as_object() else {
            return Err("workspace.json was not changed: root must be an object".into());
        };
        (
            serde_json::Value::Object(object.clone()),
            Some(file.id),
            bom,
            crlf,
        )
    } else {
        (serde_json::json!({}), None, false, false)
    };
    document["tftWeb"] = serde_json::Value::Object(state.clone());
    let mut output = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    if crlf {
        output = output.replace('\n', "\r\n");
    }
    if bom {
        output.insert(0, '\u{feff}');
    }
    if let Some(file_id) = file_id {
        write_file(&file_id, &output, None).await?;
    } else {
        create_entry("workspace.json", &folder, Some(&output)).await?;
    }
    Ok(())
}

async fn create_entry(name: &str, parent: &str, content: Option<&str>) -> Result<String, String> {
    let promise = match content {
        Some(content) => create_text_file(name, parent, content),
        None => create_folder(name, parent),
    };
    let body = JsFuture::from(promise)
        .await
        .map_err(text)?
        .as_string()
        .unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .ok_or_else(|| format!("Drive create returned no id: {body}"))
}

pub async fn create_test_vault() -> Result<(String, String), String> {
    let root = create_entry("TFT Test Vault", "", None).await?;
    let notes = create_entry("Notes", &root, None).await?;
    create_entry(
        "Welcome.md",
        &root,
        Some("# TFT test vault\n\nCreated by the web Phase 2 check.\n\n- [ ] edit this note\n"),
    )
    .await?;
    create_entry(
        "Notes/Idea.md",
        &notes,
        Some("# An idea\n\nThis note is inside a folder, so the tree can show it.\n"),
    )
    .await?;
    Ok((root, "TFT Test Vault".into()))
}

pub async fn create_note(parent: &str, name: &str) -> Result<(), String> {
    create_note_with_id(parent, name).await.map(|_| ())
}

pub async fn create_note_with_id(parent: &str, name: &str) -> Result<String, String> {
    let title = name.strip_suffix(".md").unwrap_or(name);
    let content = format!("# {title}\n\n");
    create_entry(name, parent, Some(&content)).await
}

pub async fn create_empty_file_with_id(parent: &str, name: &str) -> Result<String, String> {
    create_entry(name, parent, Some("")).await
}

pub async fn create_folder_entry(parent: &str, name: &str) -> Result<String, String> {
    create_entry(name, parent, None).await
}

pub async fn rename_entry(id: &str, name: &str) -> Result<(), String> {
    rename_entry_with_etag(id, name).await.map(|_| ())
}

pub async fn rename_entry_with_etag(id: &str, name: &str) -> Result<Option<String>, String> {
    let body = JsFuture::from(rename_file(id, name))
        .await
        .map_err(text)?
        .as_string()
        .unwrap_or_default();
    Ok(serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("etag")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        }))
}

pub async fn delete_entry(id: &str) -> Result<(), String> {
    JsFuture::from(delete_file(id))
        .await
        .map_err(text)
        .map(|_| ())
}

pub async fn duplicate_entry(id: &str, parent: &str, name: &str) -> Result<DriveItem, String> {
    let body = JsFuture::from(duplicate_file(id, parent, name))
        .await
        .map_err(text)?
        .as_string()
        .unwrap_or_default();
    serde_json::from_str(&body).map_err(|error| format!("Drive duplicate response: {error}"))
}

pub async fn move_entry(
    id: &str,
    add_parent: &str,
    remove_parent: &str,
) -> Result<Option<String>, String> {
    let body = JsFuture::from(move_file(id, add_parent, remove_parent))
        .await
        .map_err(text)?
        .as_string()
        .unwrap_or_default();
    Ok(serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("etag")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        }))
}

pub async fn trash_entry(id: &str) -> Result<(), String> {
    JsFuture::from(trash_file(id))
        .await
        .map_err(text)
        .map(|_| ())
}

pub fn link_at(target: JsValue) -> String {
    internal_link_at(target)
}

pub async fn copy_to_clipboard(value: &str) -> Result<(), String> {
    JsFuture::from(copy_text(value))
        .await
        .map_err(text)
        .map(|_| ())
}

pub fn confirm(message: &str) -> bool {
    confirm_action(message)
}

pub async fn load_markdown(entries: &[IndexedFile]) -> Vec<(NodeId, String)> {
    use futures::stream::{self, StreamExt};
    let targets: Vec<(NodeId, String)> = entries
        .iter()
        .filter(|e| e.kind == FileKind::Markdown)
        .map(|e| (e.node, e.id.clone()))
        .collect();
    stream::iter(targets)
        .map(|(node, id)| async move { (node, read_file(&id).await) })
        .buffer_unordered(8)
        .filter_map(|(node, body)| async move { body.ok().map(|body| (node, body)) })
        .collect()
        .await
}

/// Reads every Markdown entry and fails instead of silently opening an empty
/// document when a file could not be fetched.
pub async fn read_markdown_strict(
    entries: &[IndexedFile],
) -> Result<Vec<(NodeId, String)>, String> {
    use futures::stream::{self, StreamExt};
    let targets = entries
        .iter()
        .filter(|entry| entry.kind == FileKind::Markdown)
        .map(|entry| (entry.node, entry.path.clone(), entry.id.clone()))
        .collect::<Vec<_>>();
    let loaded = stream::iter(targets)
        .map(|(node, path, id)| async move { (node, path, read_file(&id).await) })
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    loaded
        .into_iter()
        .map(|(node, path, result)| {
            result
                .map(|text| (node, text))
                .map_err(|error| format!("{path}: {error}"))
        })
        .collect()
}

pub async fn blob_url(id: &str) -> Result<String, String> {
    JsFuture::from(drive_blob_url(id))
        .await
        .map_err(text)?
        .as_string()
        .ok_or_else(|| "Drive media returned no URL".to_string())
}

pub async fn file_bytes(id: &str) -> Result<Vec<u8>, String> {
    let value = JsFuture::from(drive_file_bytes(id)).await.map_err(text)?;
    Ok(js_sys::Uint8Array::new(&value).to_vec())
}

/// Toggles a browser-native audio element from a user click. The JavaScript
/// call itself invokes `play()` synchronously to retain browser activation.
pub fn toggle_audio(id: &str, url: &str) -> String {
    toggle_audio_js(id, url)
}

pub fn audio_state(id: &str) -> String {
    audio_state_js(id)
}

pub fn seek_audio(id: &str, position: f64) {
    seek_audio_js(id, position);
}

pub fn toggle_audio_mute(id: &str, url: &str) -> String {
    toggle_audio_mute_js(id, url)
}

pub fn open_video_player(url: &str, title: &str) {
    open_video_player_js(url, title);
}

/// Opens a Drive file in Google's browser preview. Call from a user click so
/// popup blockers recognize the new tab as user initiated.
pub fn open_drive_file(id: &str) {
    open_drive_file_js(id);
}

pub async fn read_file(id: &str) -> Result<String, String> {
    fetch(&format!(
        "https://www.googleapis.com/drive/v3/files/{id}?alt=media&supportsAllDrives=true"
    ))
    .await
}

pub async fn write_file(
    id: &str,
    content: &str,
    etag: Option<&str>,
) -> Result<Option<String>, String> {
    let body = JsFuture::from(update_text_file(id, content, etag.unwrap_or("")))
        .await
        .map_err(text)?
        .as_string()
        .unwrap_or_default();
    Ok(serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("etag").and_then(|v| v.as_str()).map(str::to_string)))
}

pub fn config() -> (String, String, String) {
    let value: serde_json::Value =
        serde_json::from_str(&get_config()).unwrap_or(serde_json::Value::Null);
    let get = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    (get("clientId"), get("apiKey"), get("appId"))
}

/// Browser platform modifier used by workspace shortcut hints.
pub fn modifier_key_label() -> String {
    modifier_key_label_js()
}

pub async fn sign_in(client_id: &str) -> Result<(), String> {
    JsFuture::from(begin_sign_in(client_id)?)
        .await
        .map_err(text)?;
    Ok(())
}

/// Starts OAuth synchronously so browser transient user activation is retained.
/// Call this directly from the user's click handler, then await the Promise.
pub fn begin_sign_in(client_id: &str) -> Result<js_sys::Promise, String> {
    init_token_client(client_id).map_err(text)?;
    Ok(request_token())
}

pub async fn finish_sign_in(promise: js_sys::Promise) -> Result<(), String> {
    JsFuture::from(promise).await.map_err(text)?;
    Ok(())
}

/// Attempts to obtain a token from the existing Google browser session without
/// showing account or consent UI. A user-initiated sign-in remains the fallback.
pub async fn try_resume_sign_in(client_id: &str) -> Result<(), String> {
    JsFuture::from(try_resume_token(client_id))
        .await
        .map_err(text)?;
    Ok(())
}

pub fn save_config(client_id: &str, api_key: &str) -> bool {
    save_config_js(client_id, api_key)
}

pub async fn pick_folder(api_key: &str, app_id: &str) -> Result<(String, String), String> {
    let value = JsFuture::from(begin_pick_folder(api_key, app_id))
        .await
        .map_err(text)?;
    resolve_picked_folder(value)
}

pub fn resolve_picked_folder(value: JsValue) -> Result<(String, String), String> {
    let id = js_sys::Reflect::get(&value, &JsValue::from_str("id"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let name = js_sys::Reflect::get(&value, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    if id.is_empty() {
        Err("Picker returned no folder id".into())
    } else {
        Ok((id, name))
    }
}

/// Starts the Picker synchronously from its user's click handler.
pub fn begin_pick_folder(api_key: &str, app_id: &str) -> js_sys::Promise {
    pick_folder_js(api_key, app_id)
}

pub async fn finish_pick_folder(promise: js_sys::Promise) -> Result<(String, String), String> {
    let value = JsFuture::from(promise).await.map_err(text)?;
    resolve_picked_folder(value)
}

pub fn last_vault() -> Option<(String, String)> {
    let raw = saved_vault();
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let id = value.get("id")?.as_str()?.to_string();
    let name = value
        .get("name")?
        .as_str()
        .unwrap_or("Drive vault")
        .to_string();
    Some((id, name))
}

pub fn remember_vault(id: &str, name: &str) {
    save_vault(id, name);
}

pub fn local_workspace_state(root_id: &str) -> Option<String> {
    let state = saved_workspace(root_id);
    (!state.is_empty()).then_some(state)
}

pub fn save_local_workspace_state(root_id: &str, state: &str) -> bool {
    save_workspace(root_id, state)
}

pub fn clear_local_workspace_state(root_id: &str, state: &str) {
    clear_workspace(root_id, state);
}

pub fn pref(key: &str, fallback: &str) -> String {
    get_pref(key, fallback)
}

pub fn set_pref_value(key: &str, value: &str) {
    set_pref(key, value);
}

pub fn system_is_dark() -> bool {
    prefers_dark()
}

pub fn sanitized(html: &str) -> String {
    sanitize_html(html)
}
