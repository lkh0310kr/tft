//! Desktop file operations and their prompt/UI coordination against the local VaultState.

use super::vault_state::RenameOutcome;
use super::{basename, plural, VaultState, ViewMode, Workspace};
use crate::canvas::CanvasView;
use crate::explorer;
use crate::prompt::Prompt;
use crate::prompt::{PromptAction, PromptButton, PromptSpec};
use crate::settings::SettingChange;
use gpui::{Context, Entity, Window};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tft_app::FileOperationRequest;
use tft_formats::{validate_renamed_file, FileKind};
use tft_vault::TrashOption;

/// Obsidian's "Update links" modal, to the drag pass that caught it
/// (`tools/reference-probe/results/2026-10-25-explorer-dnd.json`, step
/// `s05`): the title, two lines that name neither the file nor the new name,
/// and three buttons in that order with no Cancel. The probe read the
/// second line as "This will affect 1 link in 1 file.", so Obsidian
/// pluralises rather than writing "link(s)".
pub(super) fn update_links_prompt(files: usize, links: usize) -> PromptSpec {
    PromptSpec {
        title: "Update links".into(),
        detail: vec![
            "Do you want to update internal links that link to this file?".into(),
            format!(
                "This will affect {links} link{} in {files} file{}.",
                if links == 1 { "" } else { "s" },
                if files == 1 { "" } else { "s" }
            )
            .into(),
        ],
        input: None,
        buttons: vec![
            PromptButton::new("Always update", PromptAction::Tertiary),
            PromptButton::new("Just once", PromptAction::Submit),
            PromptButton::new("Do not update", PromptAction::Secondary),
        ],
        ..PromptSpec::default()
    }
}

/// The file operation the open prompt will finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PendingOp {
    Rename(String),
    Move(String),
    Delete(String),
    NewFolder {
        parent: String,
    },
    /// `alwaysUpdateLinks` is off: asking whether to rewrite links.
    UpdateLinks {
        from: String,
        to: String,
    },
    /// `editor:rename-heading`: the note and the zero-based line of the ATX heading.
    RenameHeading {
        path: String,
        line: u32,
    },
}

/// The prompt view and the file operation its response continues.
pub(super) struct PromptState {
    pub(super) view: Entity<Prompt>,
    pub(super) pending: Option<PendingOp>,
}

impl PromptState {
    pub(super) fn new(view: Entity<Prompt>) -> Self {
        Self {
            view,
            pending: None,
        }
    }
}

/// The path a file or folder at `from` takes when a rename field's `value` becomes
/// its name, or why the filesystem would refuse it. A field holds the stem, so both
/// the stem and the full name are validated; one validator for both rename paths.
pub(super) fn renamed_path(from: &str, value: &str, is_folder: bool) -> Result<String, String> {
    let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let name = from.rsplit('/').next().unwrap_or(from);
    let ext = if is_folder {
        None
    } else {
        name.rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|_| basename(name) != name)
    };
    validate_renamed_file(value, ext).map_err(|err| err.to_string())?;
    let new_name = match ext {
        Some(ext) => format!("{value}.{ext}"),
        None => value.to_string(),
    };
    Ok(if folder.is_empty() {
        new_name
    } else {
        format!("{folder}/{new_name}")
    })
}

impl Workspace {
    /// [`renamed_path`] for a vault path, asked whether it is a folder.
    pub(super) fn renamed_path(&self, from: &str, value: &str) -> Result<String, String> {
        let is_folder = self
            .vault_session
            .state
            .as_ref()
            .map(|s| s.vault().absolute(from).is_dir())
            .unwrap_or(false);
        renamed_path(from, value, is_folder)
    }

    /// Creates `Untitled.md` (or the next free number), opens it for editing and asks
    /// for its name, as Ctrl+N does.
    pub(super) fn dispatch_file_operation(
        &mut self,
        request: FileOperationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            FileOperationRequest::CreateNote { parent } => self.new_note(&parent, window, cx),
            FileOperationRequest::CreateFolder { parent } => {
                self.start_new_folder(&parent, window, cx)
            }
        }
    }

    pub(super) fn new_note(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "md");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                let mode = self.editing_mode();
                self.set_mode(mode, window, cx);
                self.start_rename(&path, window, cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `canvas:new-file`: an empty `Untitled.canvas` (or next free number), opened like a
    /// new note; Obsidian writes it empty (see [[Specification#Observed canvas write behavior]]).
    pub(super) fn new_canvas(&mut self, folder: &str, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "canvas");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                if self.active_group_state.current.is_some() {
                    self.new_tab(cx);
                }
                self.open_file(path.clone(), None, cx);
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Whether any group's editor holds `path` with bytes other than `on_disk` --
    /// unsaved edits a write would discard.
    pub(super) fn buffer_differs(&self, path: &str, on_disk: &str, cx: &mut Context<Self>) -> bool {
        let mine = {
            let editor = self.active_editor().read(cx);
            editor.name.as_ref() == path && editor.text() != on_disk
        };
        mine || self.groups.iter().any(|group| {
            let editor = group.editor.read(cx);
            editor.name.as_ref() == path && editor.text() != on_disk
        })
    }

    /// Writes the canvas file now if dirty, through the editor save's checks: the path
    /// is checked for links, and a file written by someone else is left alone.
    pub(super) fn flush_canvas_save(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let (dirty, path) = {
            let canvas = self.active_canvas().read(cx);
            (canvas.is_dirty(), canvas.path().map(str::to_string))
        };
        let Some(path) = path.filter(|_| dirty) else {
            return;
        };
        let absolute = match state.vault().write_path(&path) {
            Ok(absolute) => absolute,
            Err(err) => {
                self.report(format!("{path} not saved: {err}"), cx);
                return;
            }
        };
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if self.active_canvas().read(cx).matches_file(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{path} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {path}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        let contents = self.active_canvas().read(cx).to_json();
        match tft_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                self.active_canvas()
                    .update(cx, |c, _| c.mark_saved(contents));
                self.report(format!("{path} saved"), cx);
            }
            Err(err) => self.report(format!("Could not save {path}: {err}"), cx),
        }
        cx.notify();
    }

    /// A canvas view moved: the tab records where it looks, so `workspace.json` gets it
    /// whether or not the tab is live. The wheel reaches an inactive pane.
    pub(super) fn record_canvas_view_state(
        &mut self,
        canvas: &Entity<CanvasView>,
        view_state: Value,
        cx: &mut Context<Self>,
    ) {
        let path = canvas.read(cx).path().map(str::to_string);
        if path.is_none() {
            return;
        }
        if canvas == self.active_canvas() {
            if let Some(tab) = self
                .active_group_state
                .tabs
                .get_mut(self.active_group_state.active_tab)
            {
                if self.active_group_state.current == path {
                    tab.view_state = Some(view_state);
                }
            }
            return;
        }
        if let Some(group) = self.groups.iter_mut().find(|g| g.canvas == *canvas) {
            let active_tab = group.state.active_tab;
            if let Some(tab) = group.state.tabs.get_mut(active_tab) {
                if tab.path == path {
                    tab.view_state = Some(view_state);
                }
            }
        }
    }

    /// Creates the note a link or the switcher names and opens it.
    pub(super) fn create_note_for(
        &mut self,
        name: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let mut path = state.new_note_path(name, self.active_group_state.current.as_deref());
        // A dangling link naming a canvas makes it, empty, as the embed's "Click to
        // create" offers (the click itself was not observed).
        if FileKind::from_path(name) == FileKind::Canvas {
            if let Some(canvas) = path.strip_suffix(".md") {
                path = canvas.to_string();
            }
        }
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                if let Some(window) = window {
                    let mode = self.editing_mode();
                    self.set_mode(mode, window, cx);
                }
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    pub(super) fn create_from_link(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_note_for(name, Some(window), cx);
    }

    /// Enter in a tree row's own label: the same validator and rename the prompt's box
    /// uses. A name the filesystem refuses leaves the field open and says nothing, as
    /// Obsidian's does -- the row is where the answer belongs.
    pub(super) fn commit_row_rename(
        &mut self,
        from: String,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(to) = self.renamed_path(&from, value) else {
            return;
        };
        // The filesystem's refusals, a taken name above all, must be known before the
        // field closes, since staying open is how it reports them.
        let allowed = self
            .vault_session
            .state
            .as_ref()
            .map(|s| s.check_rename(&from, &to))
            .unwrap_or(Ok(()));
        if allowed.is_err() {
            return;
        }
        self.explorer.update(cx, |e, cx| e.end_rename(cx));
        self.rename_with_links(&from, &to, window, cx);
    }

    pub(super) fn start_rename(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let is_folder = self
            .vault_session
            .state
            .as_ref()
            .map(|s| s.vault().absolute(path).is_dir())
            .unwrap_or(false);
        let name = path.rsplit('/').next().unwrap_or(path);
        let initial = if is_folder { name } else { basename(name) };
        self.open_prompt(
            PendingOp::Rename(path.to_string()),
            PromptSpec {
                title: if is_folder {
                    "Rename folder".into()
                } else {
                    "Rename file".into()
                },
                detail: vec![path.to_string().into()],
                input: Some(initial.to_string()),
                placeholder: "Name".into(),
                confirm: "Rename".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// A row dropped on a folder in the Files view: it moves there under the same
    /// name, numbered `Name 1` if taken. Links follow the rename rules; a drag-move
    /// has no undo.
    pub(super) fn drop_into_folder(
        &mut self,
        path: &str,
        folder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let to = {
            let vault = state.vault();
            let is_folder = vault.absolute(path).is_dir();
            if !explorer::drop_moves(path, is_folder, folder) {
                return;
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            let (stem, ext) = match name.rsplit_once('.') {
                Some((stem, ext)) if !is_folder && !stem.is_empty() => (stem, ext),
                _ => (name, ""),
            };
            vault.available_path(folder, stem, ext)
        };
        self.rename_with_links(path, &to, window, cx);
    }

    pub(super) fn start_move(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let is_folder = state.vault().absolute(path).is_dir();
        let own_prefix = format!("{path}/");
        let parent = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut suggestions = vec!["/".to_string()];
        suggestions.extend(
            state
                .vault()
                .folders()
                .iter()
                .filter(|f| *f != path && !f.starts_with(&own_prefix) && f.as_str() != parent)
                .cloned(),
        );
        if parent.is_empty() {
            suggestions.remove(0);
        }
        self.open_prompt(
            PendingOp::Move(path.to_string()),
            PromptSpec {
                title: if is_folder {
                    "Move folder to...".into()
                } else {
                    "Move file to...".into()
                },
                detail: vec![path.to_string().into()],
                input: Some(String::new()),
                placeholder: "Type a folder".into(),
                suggestions,
                confirm: "Move".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    pub(super) fn start_new_folder(
        &mut self,
        parent: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = self
            .vault_session
            .state
            .as_ref()
            .map(|s| s.vault().available_path(parent, "Untitled", ""))
            .unwrap_or_else(|| "Untitled".to_string());
        let initial = initial.rsplit('/').next().unwrap_or(&initial).to_string();
        self.open_prompt(
            PendingOp::NewFolder {
                parent: parent.to_string(),
            },
            PromptSpec {
                title: "New folder".into(),
                detail: vec![if parent.is_empty() {
                    "In the vault folder".into()
                } else {
                    format!("In {parent}").into()
                }],
                input: Some(initial),
                placeholder: "Folder name".into(),
                confirm: "Create".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    pub(super) fn start_delete(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        if !state.confirm_delete() {
            self.delete_now(path, window, cx);
            return;
        }
        let where_to = match state.trash_option() {
            TrashOption::System => "It will be moved to the system trash.",
            TrashOption::Local => "It will be moved to the vault's .trash folder.",
            TrashOption::None => "It will be deleted permanently.",
        };
        let name = path.rsplit('/').next().unwrap_or(path);
        self.open_prompt(
            PendingOp::Delete(path.to_string()),
            PromptSpec {
                title: format!("Delete \"{name}\"?").into(),
                detail: vec![where_to.into()],
                input: None,
                confirm: "Delete".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    pub(super) fn delete_now(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let was_current = self.active_group_state.current.as_deref() == Some(path)
            || self
                .active_group_state
                .current
                .as_deref()
                .is_some_and(|c| c.starts_with(&format!("{path}/")));
        if was_current {
            // Nothing may write the buffer back into a gone file, and no pane may keep describing it.
            self.detach_current(false, &state, cx);
            // The main area may stop painting the view; focus left on it would swallow
            // every key, so `focus_main` picks the right place instead.
            self.focus_main(window, cx);
        }
        self.forget_history(path);
        match state.trash(path) {
            // Obsidian shows nothing for a delete that worked.
            Ok(changes) => self.after_changes(changes, cx),
            Err(err) => self.report(err, cx),
        }
    }

    pub(super) fn duplicate(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let is_folder = state.vault().absolute(path).is_dir();
        match state.duplicate(path) {
            // Nothing is reported for a copy that worked; the new file or folder is the feedback.
            Ok((copy, changes)) if is_folder => {
                // An empty folder's copy changes no file, so `after_changes` would leave the tree as it was.
                if changes.files.is_empty() {
                    self.refresh_vault_views(cx);
                } else {
                    self.after_changes(changes, cx);
                }
                self.explorer.update(cx, |e, cx| e.reveal_folder(&copy, cx));
                cx.notify();
            }
            Ok((copy, changes)) => {
                self.after_changes(changes, cx);
                self.open_file(copy, None, cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Enter in the prompt: finish the pending operation with `value`.
    pub(super) fn prompt_submitted(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(op) = self.prompt.pending.clone() else {
            self.close_modal(window, cx);
            return;
        };
        let value = value.trim();
        match op {
            PendingOp::RenameHeading { path, line } => {
                self.finish_rename_heading(&path, line, value, window, cx);
            }
            PendingOp::Rename(from) => {
                // The box holds the stem, the same path and validator the view-header inline field uses.
                let to = match self.renamed_path(&from, value) {
                    Ok(to) => to,
                    Err(err) => {
                        self.prompt.view.update(cx, |p, cx| p.set_error(err, cx));
                        return;
                    }
                };
                self.rename_with_links(&from, &to, window, cx);
            }
            PendingOp::Move(from) => {
                let folder = value.trim_matches('/');
                let name = from.rsplit('/').next().unwrap_or(&from);
                let to = if folder.is_empty() {
                    name.to_string()
                } else {
                    format!("{folder}/{name}")
                };
                self.rename_with_links(&from, &to, window, cx);
            }
            PendingOp::Delete(path) => {
                self.prompt.pending = None;
                self.close_modal(window, cx);
                self.delete_now(&path, window, cx);
            }
            PendingOp::NewFolder { parent } => {
                let path = if parent.is_empty() {
                    value.to_string()
                } else {
                    format!("{parent}/{value}")
                };
                let result = self
                    .vault_session
                    .state
                    .as_ref()
                    .map(|s| s.create_folder(&path))
                    .unwrap_or(Ok(()));
                match result {
                    Ok(()) => {
                        self.prompt.pending = None;
                        self.close_modal(window, cx);
                        self.refresh_vault_views(cx);
                        self.explorer.update(cx, |e, cx| e.reveal_folder(&path, cx));
                        self.report(format!("Created folder {path}"), cx);
                    }
                    Err(err) => self.prompt.view.update(cx, |p, cx| p.set_error(err, cx)),
                }
            }
            PendingOp::UpdateLinks { from, to } => self.finish_rename(&from, &to, true, window, cx),
        }
    }

    /// The prompt's secondary button: "Do not update" the links.
    pub(super) fn prompt_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(PendingOp::UpdateLinks { from, to }) = self.prompt.pending.clone() {
            self.finish_rename(&from, &to, false, window, cx);
        }
    }

    /// The prompt's "Always update": writes `alwaysUpdateLinks: true` first and then
    /// renames. A write that cannot reach disk takes the rename with it.
    pub(super) fn prompt_tertiary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PendingOp::UpdateLinks { from, to }) = self.prompt.pending.clone() else {
            return;
        };
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let mut app = state.app().clone();
        app.always_update_links = Some(true);
        if let Err(err) = state.apply_setting(&SettingChange::App(app)) {
            self.prompt
                .view
                .update(cx, |p, cx| p.set_error(format!("app.json: {err}"), cx));
            return;
        }
        self.finish_rename(&from, &to, true, window, cx);
    }

    /// Escape or a click outside: on the rename prompt this is "Do not update", leaving
    /// the rename standing and the links alone.
    pub(super) fn prompt_cancelled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(PendingOp::UpdateLinks { from, to }) = self.prompt.pending.clone() {
            self.finish_rename(&from, &to, false, window, cx);
            return;
        }
        self.prompt.pending = None;
        self.close_modal(window, cx);
    }

    pub(super) fn rename_with_links(
        &mut self,
        from: &str,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        if from == to {
            self.prompt.pending = None;
            self.close_modal(window, cx);
            return;
        }
        if state.update_links_automatically() {
            self.finish_rename(from, to, true, window, cx);
            return;
        }
        let (files, links) = state.rename_preview(from, to);
        if links == 0 {
            self.finish_rename(from, to, false, window, cx);
            return;
        }
        self.open_prompt(
            PendingOp::UpdateLinks {
                from: from.to_string(),
                to: to.to_string(),
            },
            update_links_prompt(files, links),
            window,
            cx,
        );
    }

    pub(super) fn finish_rename(
        &mut self,
        from: &str,
        to: &str,
        update_links: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        // The open note's edits must be on disk before its file moves or links are rewritten.
        self.flush_save(cx);
        match state.rename(from, to, update_links) {
            Ok(outcome) => {
                self.prompt.pending = None;
                self.close_modal(window, cx);
                self.apply_rename(outcome, cx);
            }
            Err(err) => self.prompt.view.update(cx, |p, cx| p.set_error(err, cx)),
        }
    }

    /// Follows the open note and the history to their new paths, then refreshes like a filesystem event.
    pub(super) fn apply_rename(&mut self, outcome: RenameOutcome, cx: &mut Context<Self>) {
        let renamed: HashMap<&str, &str> = outcome
            .renamed
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        // A note's fold record follows it to its new path.
        if let Some(root) = self.vault_session.state.as_ref().map(|s| s.root.clone()) {
            if let Some(store) = self.fold_store.store(&root) {
                for (from, to) in &renamed {
                    store.rename(from, to);
                }
            }
        }
        for entry in &mut self.active_group_state.history {
            if let Some(new) = renamed.get(entry.path.as_str()) {
                entry.path = new.to_string();
            }
        }
        for tab in self.stashed_tabs_mut() {
            for entry in &mut tab.history {
                if let Some(new) = renamed.get(entry.path.as_str()) {
                    entry.path = new.to_string();
                }
            }
            if let Some(path) = tab.path.as_mut() {
                if let Some(new) = renamed.get(path.as_str()) {
                    *path = new.to_string();
                }
            }
        }
        // Every group's views follow the rename, the active one's included.
        for group in &self.groups {
            group.editor.update(cx, |e, _| {
                if let Some(new) = renamed.get(e.name.as_ref()) {
                    e.set_name(new.to_string());
                }
            });
            group.reading.update(cx, |r, _| {
                if let Some(new) = r.path.as_deref().and_then(|p| renamed.get(p)) {
                    r.path = Some(new.to_string());
                }
            });
        }
        if let Some(current) = self.active_group_state.current.clone() {
            if let Some(new) = renamed.get(current.as_str()) {
                self.active_group_state.current = Some(new.to_string());
            }
        }
        // Obsidian says nothing about the rename; it shows one Notice only when links
        // were rewritten -- exactly `Updated 1 link in 1 file.`
        let links = (outcome.rewritten_links > 0).then(|| {
            format!(
                "Updated {} link{} in {} file{}.",
                outcome.rewritten_links,
                plural(outcome.rewritten_links),
                outcome.rewritten_files,
                plural(outcome.rewritten_files),
            )
        });
        // A note that changed underneath keeps its own bytes; Obsidian never skips, so this notice stays.
        let skipped = match outcome.stale.len() {
            0 => None,
            n => Some(format!(
                "{n} file{} changed on disk and {} left alone",
                plural(n),
                if n == 1 { "was" } else { "were" }
            )),
        };
        self.after_changes(outcome.changes, cx);
        if let Some(current) = self.active_group_state.current.clone() {
            self.explorer.update(cx, |e, cx| e.follow(&current, cx));
        }
        let notice = match (links, skipped) {
            (None, None) => None,
            (Some(l), None) => Some(l),
            (None, Some(s)) => Some(s),
            (Some(l), Some(s)) => Some(format!("{l} {s}")),
        };
        if let Some(notice) = notice {
            self.report(notice, cx);
        }
    }

    /// `editor:rename-heading`: the file-rename modal's shape, titled "Rename heading",
    /// Save then Cancel. Off an ATX heading line it opens nothing, as Obsidian's run on
    /// a setext heading did.
    pub(super) fn start_rename_heading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_group_state.current.clone() else {
            return;
        };
        let (heading, line) = {
            let e = self.active_editor().read(cx);
            (e.heading_to_rename(), e.cursor().line as u32)
        };
        let Some(heading) = heading else {
            return;
        };
        self.open_prompt(
            PendingOp::RenameHeading { path, line },
            PromptSpec {
                title: "Rename heading".into(),
                input: Some(heading.text),
                buttons: vec![
                    PromptButton::new("Save", PromptAction::Submit).cta(),
                    PromptButton::new("Cancel", PromptAction::Cancel),
                ],
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// Save in "Rename heading": an empty name writes nothing. Otherwise the note takes
    /// the heading line and its links as one undo step and is written at once; other
    /// linking notes go through [`VaultState::write_link_rewrites`].
    pub(super) fn finish_rename_heading(
        &mut self,
        path: &str,
        line: u32,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if name.is_empty() {
            return;
        }
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        self.prompt.pending = None;
        self.close_modal(window, cx);
        let own_text = {
            let e = self.active_editor().read(cx);
            (e.name.as_ref() == path && self.active_group_state.current.as_deref() == Some(path))
                .then(|| e.text().to_string())
        };
        let Some(own_text) = own_text else {
            return;
        };
        // Both guards are dropped before anything takes a write lock.
        let plan = {
            let texts = state.texts();
            state
                .cache()
                .plan_heading_rename(path, &own_text, line, name, &|p| {
                    texts.get(p).map(|t| t.to_string())
                })
        };
        let Some(plan) = plan else {
            return;
        };
        if !plan.own.is_empty() {
            let heading = plan.heading.clone();
            let edits = plan.own.clone();
            self.active_editor().update(cx, |e, cx| {
                e.apply_heading_rename(edits, &heading, window, cx)
            });
            self.flush_save(cx);
        }
        match state.write_link_rewrites(&plan.rewrites) {
            Ok((changes, stale)) => {
                self.after_changes(changes, cx);
                if !stale.is_empty() {
                    let n = stale.len();
                    self.report(
                        format!(
                            "{n} file{} changed on disk and {} left alone",
                            plural(n),
                            if n == 1 { "was" } else { "were" }
                        ),
                        cx,
                    );
                }
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `markdown:add-alias`: Live Preview writes the `aliases` key at once; Source mode
    /// reveals the File properties pane's aliases input. Reading view is not offered
    /// (see `can_run`).
    pub(super) fn add_alias(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_group_state.mode.is_editing() || !self.note_has_keys(window, cx) {
            return;
        }
        let in_table = self.active_group_state.mode == ViewMode::LivePreview
            && self.active_editor().read(cx).draws_properties_table();
        let _ = in_table;
        let changed = self
            .active_editor()
            .update(cx, |e, cx| e.add_alias(true, window, cx));
        // The key reached disk within 0.3 s in Obsidian, not on the autosave's debounce.
        if changed {
            self.flush_save(cx);
        }
        cx.notify();
    }

    /// Renames the open note (same folder and extension) right away; `--rename` for smoke tests.
    pub fn rename_current(&mut self, new_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.active_group_state.current.clone() else {
            return;
        };
        let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let name = from.rsplit('/').next().unwrap_or(&from);
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("md");
        let to = if folder.is_empty() {
            format!("{new_name}.{ext}")
        } else {
            format!("{folder}/{new_name}.{ext}")
        };
        self.finish_rename(&from, &to, true, window, cx);
    }

    /// Opens a file prompt for the open note (`--prompt`): `rename`, `move`, `delete` or `folder`.
    pub fn open_file_prompt(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.active_group_state.current.clone() else {
            return;
        };
        match kind {
            "move" => self.start_move(&current, window, cx),
            "delete" => self.start_delete(&current, window, cx),
            "folder" => self.start_new_folder("", window, cx),
            _ => self.start_rename(&current, window, cx),
        }
    }
}
