//! Desktop document mode, history, reader edits, and local-buffer save coordination.

use super::{GoBack, GoForward, ToggleMode, VaultState, ViewMode, Workspace};
use crate::editor::Editor;
use crate::reading::{self, TaskMarker};
use gpui::{Context, Entity, Task, Window};
use std::time::Duration;
use tft_app::{
    current_history_entry_mut, history_step, push_history_entry, HistoryDirection, HistoryEntry,
};
use tft_markdown::Subpath;

/// The pending delayed write for the active Markdown buffer. Replacing or cancelling
/// the task preserves the existing GPUI task-drop cancellation behavior.
#[derive(Default)]
pub(super) struct DocumentSaveState {
    task: Option<Task<()>>,
}

impl DocumentSaveState {
    fn schedule(&mut self, task: Task<()>) {
        self.task = Some(task);
    }

    pub(super) fn cancel(&mut self) {
        self.task = None;
    }
}

impl Workspace {
    /// Shows the graph (`graph:open`); a second Ctrl+G returns to the note.
    pub fn show_graph(&mut self, local: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.radial_tree.active = false;
        self.graph.update(cx, |g, cx| g.set_local(local, cx));
        self.set_mode(ViewMode::Graph, window, cx);
    }

    pub fn set_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        // A terminal is a pane, not a note: Reading, Graph and the editor
        // toggles do not replace it under the shell. The tab's close is the
        // way out; `terminal:open` is the way back in.
        if self.active_group_state.mode == ViewMode::Terminal && mode != ViewMode::Terminal {
            return;
        }
        self.reset_preview(cx);
        if mode == ViewMode::Graph && self.active_group_state.mode != ViewMode::Graph {
            self.active_group_state.mode_before_graph = self.active_group_state.mode;
        }
        // The history entry tracks the current state, so navigating away and back restores the mode.
        self.record_history_state(cx);
        if !mode.is_editing() && self.active_group_state.mode.is_editing() {
            // Reading shows what the editor has: save and rebuild now rather than waiting
            // for the autosave.
            if self.active_editor().read(cx).is_dirty() {
                self.flush_save(cx);
                if let (Some(state), Some(current)) = (
                    self.vault_session.state.clone(),
                    self.active_group_state.current.clone(),
                ) {
                    self.show_file(&current, &state, cx);
                }
            }
        }
        // The fold is shared: editor and reading view collapse the same sections.
        if mode.is_editing() != self.active_group_state.mode.is_editing() {
            self.sync_folds(mode.is_editing(), cx);
        }
        self.active_group_state.mode = mode;
        if mode.is_editing() {
            self.active_editor().update(cx, |e, cx| {
                e.set_live_preview(mode == ViewMode::LivePreview, cx)
            });
        }
        self.focus_main(window, cx);
        cx.notify();
    }

    /// Opens the editor's find panel, switching to editing first; an empty `query`
    /// keeps the panel's own or the selection.
    pub fn open_find(
        &mut self,
        query: &str,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.active_group_state.mode.is_editing() {
            let mode = self.editing_mode();
            self.set_mode(mode, window, cx);
        }
        self.active_editor()
            .update(cx, |e, cx| e.open_find(query, replace, window, cx));
    }

    /// The editing mode a note opens in: Editing view in Live Preview.
    pub fn editing_mode(&self) -> ViewMode {
        ViewMode::LivePreview
    }

    pub fn default_mode(&self) -> ViewMode {
        self.editing_mode()
    }

    /// Opens a vault path by exact match or suffix (`--note`); `Name#Heading` or
    /// `Name#^id` also scrolls to that section.
    pub fn open_by_name(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.vault_session.state.clone() else {
            return false;
        };
        let link = tft_markdown::parse_linktext(name);
        let wanted = link.path;
        let found = state
            .vault()
            .files()
            .iter()
            .map(|f| f.path.as_str())
            .find(|p| {
                *p == wanted
                    || p.ends_with(wanted)
                    || p.trim_end_matches(".md") == wanted
                    || p.trim_end_matches(".md").ends_with(&format!("/{wanted}"))
            })
            .map(str::to_string);
        match found {
            Some(path) => {
                self.open_file(path, link.subpath, cx);
                true
            }
            None => false,
        }
    }

    /// Opens the first markdown note, if any.
    pub fn open_first(&mut self, cx: &mut Context<Self>) {
        let first = self
            .vault_session
            .state
            .as_ref()
            .and_then(|s| s.vault().markdown_files().next().map(|f| f.path.clone()));
        if let Some(path) = first {
            self.open_file(path, None, cx);
        }
    }

    /// Opens a vault-relative path in the main area and records it in history.
    pub fn open_file(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        self.reset_preview(cx);
        let Some(state) = self.vault_session.state.clone() else {
            self.active_group_state.current = Some(path);
            return;
        };
        // A note opened from a terminal is another tab: the shell keeps its
        // pane, as a note opened beside a running terminal does in Zed.
        if self.active_group_state.mode == ViewMode::Terminal {
            self.new_tab(cx);
        }
        // A pinned tab keeps its file: anything opened from it — a link
        // followed, a switcher pick — goes to a new tab (observed).
        if self.active_group_state.active_tab_pinned()
            && self.active_group_state.current.as_deref() != Some(path.as_str())
        {
            self.new_tab(cx);
        }
        // The graph is a toggle, not a place a note can open in: a note
        // asked for from anywhere shows the note.
        if self.active_group_state.mode == ViewMode::Graph {
            self.active_group_state.mode = self.active_group_state.mode_before_graph;
        }
        if self.active_group_state.current.as_deref() != Some(path.as_str()) {
            self.flush_save(cx);
            // The entry being left keeps the state it is being left in, so
            // coming back to it comes back to where the reader was.
            self.record_history_state(cx);
            push_history_entry(
                &mut self.active_group_state.history,
                &mut self.active_group_state.history_ix,
                HistoryEntry::new(path.clone(), self.active_group_state.mode),
            );
            self.layout_persistence.record_open_file(&path);
        }
        self.show_file(&path, &state, cx);
        // Opening a note is a move into the main area, wherever it was asked
        // for: an explorer row, a search hit, the switcher, a link, the
        // socket. Obsidian hands the keyboard to the note it just opened, and
        // without this the click that opened it leaves focus on the
        // workspace root, where typing and the arrow keys reach nothing.
        self.request_focus_main(cx);
        if self.active_group_state.current.as_deref() != Some(path.as_str()) {
            // It did not open, so it is not somewhere to go back to either.
            self.forget_history(&path);
            return;
        }
        if let Some(sub) = subpath {
            self.scroll_to_subpath(&path, &sub, &state, cx);
        }
    }

    // ----- file operations -------------------------------------------------------

    pub(super) fn toggle_mode(
        &mut self,
        _: &ToggleMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = self
            .active_group_state
            .mode
            .toggle_reading_editing(self.editing_mode());
        self.set_mode(mode, window, cx);
    }

    pub(super) fn go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.record_history_state(cx);
        if let Some(history_ix) = history_step(
            self.active_group_state.history_ix,
            self.active_group_state.history.len(),
            HistoryDirection::Back,
        ) {
            self.active_group_state.history_ix = history_ix;
            self.navigate_history(cx);
        }
    }

    pub(super) fn go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        self.record_history_state(cx);
        if let Some(history_ix) = history_step(
            self.active_group_state.history_ix,
            self.active_group_state.history.len(),
            HistoryDirection::Forward,
        ) {
            self.active_group_state.history_ix = history_ix;
            self.navigate_history(cx);
        }
    }

    pub(super) fn navigate_history(&mut self, cx: &mut Context<Self>) {
        let (Some(state), Some(entry)) = (
            self.vault_session.state.clone(),
            self.active_group_state
                .history
                .get(self.active_group_state.history_ix.wrapping_sub(1))
                .cloned(),
        ) else {
            return;
        };
        if self.active_group_state.mode == ViewMode::Graph {
            self.active_group_state.mode = self.active_group_state.mode_before_graph;
        }
        self.flush_save(cx);
        // Obsidian's history carries the view state, so back/forward restore the note's mode too.
        self.active_group_state.mode = entry.mode;
        self.show_file(&entry.path, &state, cx);
        if entry.offset > 0 {
            self.reveal_offset(entry.offset, cx);
        }
        self.request_focus_main(cx);
    }

    /// Writes the view state into the entry the history stands on: the caret while
    /// editing, the top block in Reading, where there is no caret.
    pub(super) fn record_history_state(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.active_group_state.current.clone() else {
            return;
        };
        let mode = if self.active_group_state.mode == ViewMode::Graph {
            self.active_group_state.mode_before_graph
        } else {
            self.active_group_state.mode
        };
        let offset = if mode == ViewMode::Reading {
            self.active_reading().read(cx).top_offset().unwrap_or(0)
        } else {
            self.active_editor().read(cx).selection().start
        };
        let Some(entry) = current_history_entry_mut(
            &mut self.active_group_state.history,
            self.active_group_state.history_ix,
        ) else {
            return;
        };
        if entry.path == current {
            entry.mode = mode;
            entry.offset = offset;
        }
    }

    /// A task checkbox clicked in Reading takes the editor's path: one undo step, the
    /// autosave writes it, and the reader rebuilds from the buffer.
    pub(super) fn toggle_task(&mut self, path: &str, marker: TaskMarker, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        // A click that outlived its note must write nothing; the marker's line and
        // column belong to that note's bytes.
        if self.active_group_state.current.as_deref() != Some(path)
            || self.active_editor().read(cx).name.as_ref() != path
        {
            return;
        }
        if !self.active_editor().update(cx, |e, cx| {
            e.toggle_task_at(marker.line, marker.col, None, cx)
        }) {
            return;
        }
        self.rebuild_reading_from_editor(path, &state, cx);
    }

    /// A pill's remove control in Reading takes [`Self::toggle_task`]'s path, then
    /// rebuilds the reader from the buffer.
    pub(super) fn remove_property_item(
        &mut self,
        path: &str,
        key: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        // A click that outlived its note must write nothing; the key and index belong
        // to that note's block.
        if self.active_group_state.current.as_deref() != Some(path)
            || self.active_editor().read(cx).name.as_ref() != path
        {
            return;
        }
        if !self
            .active_editor()
            .update(cx, |e, cx| e.remove_property_item(key, index, None, cx))
        {
            return;
        }
        self.rebuild_reading_from_editor(path, &state, cx);
    }

    /// Rebuilds the reading view from the editor's buffer after a reader's edit the
    /// autosave has not written yet.
    pub(super) fn rebuild_reading_from_editor(
        &mut self,
        path: &str,
        state: &VaultState,
        cx: &mut Context<Self>,
    ) {
        let text = self.active_editor().read(cx).text().to_string();
        let model = reading::build_with(path, &text, state, state.build_options());
        self.active_reading()
            .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
        cx.notify();
    }

    /// Writes the editor's buffer back two seconds after the last edit, as Obsidian's autosave does.
    pub(super) fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |ws, cx| ws.flush_save(cx)).ok();
        });
        self.document_save.schedule(task);
    }

    /// Writes the editor's buffer now if dirty, with the file's own line endings and
    /// BOM around the edited text.
    pub fn flush_save(&mut self, cx: &mut Context<Self>) {
        self.document_save.cancel();
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        self.flush_canvas_save(&state, cx);
        let (dirty, name) = {
            let editor = self.active_editor().read(cx);
            (editor.is_dirty(), editor.name.to_string())
        };
        // Only the open note is written back; a buffer that is not a vault file
        // (the synthetic stress note) never touches disk.
        if !dirty || self.active_group_state.current.as_deref() != Some(name.as_str()) {
            return;
        }
        let editor = self.active_editor().clone();
        self.write_buffer(&editor, cx);
    }

    /// Writes an editor's buffer to its file, unless the file changed or went away underneath it.
    pub(super) fn write_buffer(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let name = editor.read(cx).name.to_string();
        let contents = editor.read(cx).file_contents();
        // The one place a note's bytes are written; the path is checked for links,
        // so a save can never land outside the vault.
        let absolute = match state.vault().write_path(&name) {
            Ok(path) => path,
            Err(err) => {
                self.report(format!("{name} not saved: {err}"), cx);
                return;
            }
        };
        // The vault is the source of truth: if the file is not what this note loaded
        // or last saved, someone else wrote it, and overwriting would throw that away.
        // A deleted note is not recreated; the buffer keeps its edits and says so.
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if editor.read(cx).disk_unchanged(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{name} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.report(
                    format!("{name} was deleted; not recreated, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {name}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        match tft_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                editor.update(cx, |e, _| e.mark_saved(&contents));
            }
            Err(err) => {
                self.report(format!("Could not save {name}: {err}"), cx);
            }
        }
        cx.notify();
    }
}
