//! Workspace adapter methods exposed to the Desktop control socket.

use super::empty_tab::{empty_tab_actions, EmptyAction};
use super::file_menu;
use super::tab_strip::TabTitle;
use super::{context_names, Commands, MainView, Modal, PendingOp, Tab, ViewMode, Workspace};
use crate::explorer::kind_name;
use gpui::{App, Context, Window};
use serde_json::{json, Map, Value};
use std::path::{Component, Path, PathBuf};
use tft_commands::keystroke;

/// Startup readiness exposed to the control socket. This is separate from Vault
/// readiness: startup flags must finish arranging the window before clients proceed.
#[derive(Default)]
pub(super) struct ControlApiState {
    started: bool,
}

impl ControlApiState {
    fn mark_started(&mut self) {
        self.started = true;
    }

    fn is_started(&self) -> bool {
        self.started
    }
}

impl Workspace {
    /// Called once startup flags have run, so control clients can wait for a settled window.
    pub fn mark_started(&mut self) {
        self.control_api_state.mark_started();
    }

    pub fn is_started(&self) -> bool {
        self.control_api_state.is_started()
    }

    /// Builds the reading model a note opened in an editing mode is still
    /// waiting on, so a state dump describes the note rather than nothing.
    pub fn ensure_reading_built(&mut self, cx: &mut Context<Self>) {
        self.active_reading().update(cx, |r, cx| r.ensure_built(cx));
    }

    /// An absolute path for a vault-relative one, refusing anything that
    /// points outside the vault. This is the only path the socket reads and
    /// writes through, and the vault folder is the only thing TFT may
    /// touch.
    pub fn vault_path(&self, path: &str) -> Result<PathBuf, String> {
        let root = self
            .vault_root()
            .ok_or_else(|| "No vault is open".to_string())?;
        vault_path_in(&root, path)
    }

    /// The vault's own spelling of a vault-relative path, if the index
    /// carries one: that spelling is the key every pane is looked up by, so
    /// the caller's casing is only a request for it.
    pub fn vault_file_path(&self, path: &str) -> Option<String> {
        let state = self.vault_session.state.as_ref()?;
        state
            .vault()
            .files()
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(path))
            .map(|f| f.path.clone())
    }

    /// The open vault's files, for the socket's `list_files`: the same list
    /// the explorer is built from, under `folder` when one is given.
    pub fn list_vault_files(&self, folder: Option<&str>) -> Vec<Value> {
        let Some(state) = self.vault_session.state.as_ref() else {
            return Vec::new();
        };
        // Vault paths keep their case and compare without it, so a folder
        // stored as `Sub` answers to `sub` as well.
        let prefix = folder
            .map(|f| f.trim_matches('/'))
            .filter(|f| !f.is_empty())
            .map(|f| format!("{}/", f.to_lowercase()));
        state
            .vault()
            .files()
            .iter()
            .filter(|f| {
                prefix
                    .as_deref()
                    .is_none_or(|p| f.path.to_lowercase().starts_with(p))
            })
            .map(|f| json!({"path": f.path, "kind": kind_name(f.kind), "size": f.size}))
            .collect()
    }

    /// Every command in the registry with the hotkeys `hotkeys.json` leaves
    /// it, and whether [`Workspace::run_command`] would run it *now*.
    ///
    /// `runnable` is the check callback's answer, so it moves with the
    /// window: an editing command reads false from a reading leaf, which
    /// is the same thing the palette shows.
    pub fn list_commands(&self, window: &Window, cx: &App) -> Vec<Value> {
        cx.global::<Commands>()
            .0
            .iter()
            .map(|c| {
                let hotkeys: Vec<String> = c.hotkeys.iter().filter_map(keystroke).collect();
                json!({
                    "id": c.id,
                    "name": c.name,
                    "hotkeys": hotkeys,
                    "runnable": self.can_run(&c.id, window, cx),
                })
            })
            .collect()
    }

    /// The mode a socket request names, if it names one at all.
    pub fn open_mode(&self, mode: Option<&str>) -> Result<Option<ViewMode>, String> {
        match mode {
            None => Ok(None),
            Some(name) => ViewMode::parse(name)
                .map(Some)
                .ok_or_else(|| format!("unknown mode: {name}")),
        }
    }
    pub fn describe(&self, text: bool, rows: usize, window: &Window, cx: &App) -> Value {
        let mut vault = Map::new();
        vault.insert(
            "root".into(),
            json!(self
                .vault_session
                .state
                .as_ref()
                .map(|s| s.root.display().to_string())),
        );
        vault.insert("name".into(), json!(self.vault_session.name.to_string()));
        vault.insert("loaded".into(), json!(self.vault_session.state.is_some()));
        vault.insert(
            "menu_open".into(),
            json!(self.vault_switcher_state.is_menu_open()),
        );
        vault.insert(
            "loading".into(),
            json!(self.vault_session.loading.as_ref().map(|l| l.to_string())),
        );

        let mut file = Map::new();
        file.insert("path".into(), json!(self.active_group_state.current));
        file.insert(
            "detached".into(),
            json!(self
                .active_group_state
                .detached
                .as_ref()
                .map(|n| n.to_string())),
        );
        file.insert("mode".into(), json!(self.active_group_state.mode.as_str()));
        file.insert(
            "dirty".into(),
            json!(self.active_editor().read(cx).is_dirty()),
        );
        put_capped(
            &mut file,
            "history",
            self.active_group_state
                .history
                .iter()
                .map(|e| json!(e.path))
                .collect(),
            rows,
        );
        file.insert(
            "history_ix".into(),
            json!(self.active_group_state.history_ix),
        );

        let tab_json = |t: &Tab| json!({"path": t.path, "mode": t.mode.as_str(), "title": t.title(), "pinned": t.pinned});
        let tabs = json!({
            "active": self.active_group_state.active_tab,
            "items": self.active_group_state.tab_snapshots().iter().map(tab_json).collect::<Vec<Value>>(),
        });
        // The tab groups and how they are split; `tabs` above is the active
        // group's, as it was before there could be more than one.
        let groups = json!({
            "active": self.active_group_id(),
            // The popout window the active group is in, `null` for the
            // main window: the window the socket's input goes to.
            "active_window": self.window_of_group(self.active_group_id()),
            "layout": self.layout_tree_json(&self.layout_tree),
            // The popout windows the layout holds; `open` says whether a
            // gpui window is showing one (a restored popout has none yet).
            "windows": self
                .windows
                .iter()
                .map(|w| {
                    json!({
                        "id": w.id,
                        // A window being described from inside its own
                        // update is out of the app's table for the moment.
                        "open": w.is_open(cx)
                            || w.handle.is_some_and(|h| {
                                gpui::AnyWindowHandle::from(h) == window.window_handle()
                            }),
                        "title": w.title,
                        "bounds": w.bounds().map(|b| {
                            json!({"x": b.x, "y": b.y, "width": b.width, "height": b.height})
                        }),
                        "layout": self.layout_tree_json(&w.tree),
                    })
                })
                .collect::<Vec<Value>>(),
            "items": self
                .groups
                .iter()
                .enumerate()
                .map(|(ix, g)| {
                    let view = self.group_view(ix);
                    json!({
                        "id": g.state.id,
                        "active_tab": view.active_tab,
                        "dimension": g.dimension(),
                        "tabs": view.tabs.iter().map(tab_json).collect::<Vec<Value>>(),
                    })
                })
                .collect::<Vec<Value>>(),
        });

        let mut left = Map::new();
        left.insert("visible".into(), json!(self.left_dock.visible));
        left.insert("width".into(), json!(self.left_dock.width));
        left.insert("tab".into(), json!("files"));
        left.insert("explorer".into(), self.explorer.read(cx).describe(rows));
        left.insert("search".into(), self.search.read(cx).describe(rows, cx));

        // Whether the graph is on screen is the shell's to say; the view
        // itself only knows what it would draw.
        let mut graph = self.graph.read(cx).describe();
        if let Some(object) = graph.as_object_mut() {
            object.insert(
                "open".into(),
                json!(matches!(self.active_group_state.mode, ViewMode::Graph)),
            );
        }
        // Page preview's popovers, first to last: each one's window, the
        // note its links resolve against, the anchor's line box, the left
        // edge the box is drawn at, what it shows, and whether it edits.
        let viewport_width = window.viewport_size().width.as_f32();
        let popovers: Vec<Value> = self.preview_state.model
            .shown
            .iter()
            .map(|s| {
                json!({
                    "window": s.window,
                    "from": s.from,
                    "anchor": [s.anchor.left().as_f32(), s.anchor.top().as_f32(), s.anchor.size.height.as_f32()],
                    "left": crate::hover_preview::left_edge(s.anchor.left().as_f32(), viewport_width),
                    "height": s.height.get(),
                    "content": match &s.content {
                        crate::hover_preview::Content::Missing(line) => json!({"missing": line}),
                        crate::hover_preview::Content::Blocks { blocks } => json!({"blocks": blocks.len()}),
                    },
                    "over": s.over,
                    "editing": s.editing.as_ref().map(|e| {
                        let e = e.read(cx);
                        json!({
                            "path": e.name.to_string(),
                            "dirty": e.is_dirty(),
                            "selection": [e.selection().start, e.selection().end],
                        })
                    }),
                })
            })
            .collect();

        json!({
            "vault": Value::Object(vault),
            "file": Value::Object(file),
            "tabs": tabs,
            "groups": groups,
            "docks": {"left": Value::Object(left)},
            // The one open file menu: the editor's text or a link in the
            // reading view.
            "file_menu": self.file_menu_state.menu.as_ref().map(file_menu::describe),
            "modal": self.describe_modal(rows, cx),
            "editor": self.active_editor().read(cx).describe(text, cx),
            "reading": self.active_reading().read(cx).describe(cx),
            "graph": graph,
            "canvas": self.active_canvas().read(cx).describe(cx),
            "pdf": self.active_pdf().read(cx).state_json(cx),
            "status": self.feedback.status_text(),
            "status_bar": self.status_bar_text(cx),
            "status_bar_items": self.status_bar_items(cx),
            // The empty tab's pills, by the text they carry, while that is
            // what the main area shows.
            "empty_tab": (self.active_group_state.current.is_none() && self.main_view(cx) == MainView::Empty).then(|| {
                empty_tab_actions(&cx.global::<Commands>().0)
                    .iter()
                    .map(EmptyAction::text)
                    .collect::<Vec<String>>()
            }),
            // The view header's title while it is being renamed in place,
            // so a driver can see the typed name before Enter commits it.
            "view_title": self.title_editing.editing.as_ref().map(|edit| json!({
                "group": edit.group,
                "file": edit.path,
                "text": edit.editing.field.text(),
                "cursor": edit.editing.field.cursor(),
            })),
            "notices": self.feedback.notices.texts(),
            // The window each of those draws in, the popout's id or null
            // for the vault window.
            "notices_window": self.feedback.notices.windows(&|id| self.popout_open(id)),
            // Page preview: the link under the pointer, and the popover up
            // (its window, the note its links resolve against, the anchor's
            // line box and the left edge the box is drawn at).
            "hover_preview": {
                "pending": self.preview_state.model.pending.as_ref().map(|p| json!({
                    "target": p.target,
                    "from": p.from,
                    "window": p.window,
                    "depth": p.depth,
                })),
                "shown": popovers.first().cloned(),
                "popovers": popovers,
                "over_popover": self.preview_state.model.shown.iter().any(|s| s.over),
            },
            "focus": {
                "name": self.focus_name(window, cx),
                "context_stack": context_names(window),
            },
            "window": {
                "width": window.viewport_size().width.as_f32(),
                "height": window.viewport_size().height.as_f32(),
                "scale": window.scale_factor(),
                "active": window.is_window_active(),
                "title": self.described_title(window),
                // The popout being described, `null` for the main window.
                "popout": self.popout_of_window(window.window_handle()),
            },
        })
    }

    fn describe_modal(&self, rows: usize, cx: &App) -> Value {
        let mut modal = Map::new();
        // Settings is a window rather than a modal, but it is still the
        // thing in front of the caller, and the socket's contract has said
        // "settings" here since before it moved out of the workspace.
        let settings_up = self.settings_up(cx);
        modal.insert(
            "kind".into(),
            json!(match self.overlay.modal {
                _ if settings_up => "settings",
                Modal::None => "none",
                Modal::Switcher => "switcher",
                Modal::CommandPalette => "command-palette",
                Modal::Prompt => "prompt",
                Modal::CopyResults => "copy-results",
                Modal::Chooser(_) => "chooser",
                Modal::Downloads => "download-attachments",
                Modal::PdfExport => "export-pdf",
                Modal::ImageExport => "export-image",
            }),
        );
        // The popout it draws in, by id, or null for the vault window (and
        // with nothing up). Settings is a window of its own.
        modal.insert(
            "window".into(),
            json!(self.overlay.window.flatten().filter(|_| !settings_up)),
        );
        // The four list modals fill these in; with no modal up they stay
        // empty rather than disappearing.
        modal.insert("query".into(), Value::Null);
        modal.insert("selected".into(), Value::Null);
        modal.insert("error".into(), Value::Null);
        modal.insert("rows".into(), Value::Array(Vec::new()));
        let view = match self.overlay.modal {
            _ if settings_up => Some(self.settings_state.describe(rows, cx)),
            Modal::Switcher => Some(self.switcher.read(cx).describe(rows, cx)),
            Modal::CommandPalette => Some(self.command_palette.read(cx).describe(rows, cx)),
            Modal::Prompt => Some(self.prompt.view.read(cx).describe(rows, cx)),
            Modal::CopyResults => Some(self.copy_results.read(cx).describe()),
            Modal::Chooser(_) => Some(self.chooser.read(cx).describe(rows, cx)),
            Modal::Downloads => self.describe_downloads(cx),
            Modal::PdfExport => Some(self.pdf_export.read(cx).describe()),
            Modal::ImageExport => Some(self.image_export.read(cx).describe()),
            Modal::None => None,
        };
        if let Some(Value::Object(view)) = view {
            for (key, value) in view {
                modal.insert(key, value);
            }
        }
        // Settings has tabs rather than rows; the tab it is on is the part
        // a caller steers by.
        modal.insert(
            "settings_tab".into(),
            modal.get("tab").cloned().unwrap_or(Value::Null),
        );
        modal.insert(
            "pending_op".into(),
            match &self.prompt.pending {
                None => Value::Null,
                Some(PendingOp::Rename(path)) => json!({"op": "rename", "path": path}),
                Some(PendingOp::Move(path)) => json!({"op": "move", "path": path}),
                Some(PendingOp::Delete(path)) => json!({"op": "delete", "path": path}),
                Some(PendingOp::NewFolder { parent }) => {
                    json!({"op": "new-folder", "parent": parent})
                }
                Some(PendingOp::UpdateLinks { from, to }) => {
                    json!({"op": "update-links", "from": from, "to": to})
                }
                Some(PendingOp::RenameHeading { path, line }) => {
                    json!({"op": "rename-heading", "path": path, "line": line})
                }
            },
        );
        Value::Object(modal)
    }
}

/// Inserts `list` under `key`, capped to `rows`, with a sibling
/// `{key}_total`. The total is always there, `null` when the cap dropped
/// nothing, so a caller has one rule for every list in the dump.
fn put_capped(into: &mut Map<String, Value>, key: &str, list: Vec<Value>, rows: usize) {
    let total = list.len();
    into.insert(
        key.to_string(),
        Value::Array(list.into_iter().take(rows).collect()),
    );
    into.insert(
        format!("{key}_total"),
        if total > rows {
            json!(total)
        } else {
            Value::Null
        },
    );
}

pub(super) fn vault_path_in(root: &Path, path: &str) -> Result<PathBuf, String> {
    let mut absolute = root.to_path_buf();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => absolute.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(format!("{path} leaves the vault")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{path} is not a vault-relative path"));
            }
        }
    }
    if absolute.as_path() == root {
        return Err(format!("{path} names the vault folder, not a file"));
    }
    let real_root = root
        .canonicalize()
        .map_err(|error| format!("{}: {error}", root.display()))?;
    let mut existing: &Path = absolute.as_path();
    let inside = loop {
        if existing.symlink_metadata().is_ok() {
            break match existing.canonicalize() {
                Ok(real) => real.starts_with(&real_root),
                // A dangling link has nothing to resolve; so has a path the
                // process may not look at. Neither can be shown to stay in
                // the vault, so neither is written through.
                Err(_) => false,
            };
        }
        match existing.parent() {
            // The walk stops at `root` at the latest: it canonicalized
            // above, so it exists.
            Some(parent) if parent.starts_with(root) => existing = parent,
            _ => break false,
        }
    };
    if !inside {
        return Err(format!("{path} leaves the vault"));
    }
    Ok(absolute)
}
