//! A Web-hosted GPUI composition over TFT's shared vault and workspace models.
//! Storage and authentication are supplied by the host; editing stays in the
//! production Editor and document bytes live in `VaultData`.

use gpui::{
    actions, div, prelude::*, px, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    Focusable, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render,
    SharedString, Subscription, Window,
};
#[cfg(feature = "browser-test")]
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tft_app::editor::Editor;
use tft_app::explorer::{FileTree, SortOrder};
use tft_app::reading::{
    self, EmbedSource, PropertyItemRemoved, ReadingEvent, ReadingView, TaskToggled,
};
use tft_app::text_input::{InputEvent, TextInput};
use tft_app::theme::Theme;
use tft_app::{
    BackendObjectId, FileMeta, FileOperationRequest, LeftTab, NodeId, NodeKind, VaultData,
    ViewMode, WorkspaceModel,
};
use tft_cache::LinkResolver;
#[cfg(feature = "browser-test")]
use tft_formats::files::VaultFile;

actions!(web_workspace, [OpenQuickSwitcher]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-p", OpenQuickSwitcher, None),
        KeyBinding::new("ctrl-p", OpenQuickSwitcher, None),
    ]);
}

#[derive(Clone)]
struct SearchHit {
    node: NodeId,
    path: String,
    snippets: Vec<String>,
}

struct SearchDocument {
    node: NodeId,
    path: String,
    contents: Arc<String>,
    metadata: tft_cache::CachedMetadata,
}

#[derive(Clone)]
struct QuickSwitcherHit {
    node: NodeId,
    path: String,
    label: String,
    score: f64,
    alias: bool,
}

#[derive(Clone)]
struct FileTreeContextMenu {
    position: Point<Pixels>,
    node: Option<NodeId>,
    key: String,
    path: String,
    is_folder: bool,
}

#[derive(Clone)]
struct TabContextMenu {
    position: Point<Pixels>,
    index: usize,
}

#[cfg(feature = "browser-test")]
#[derive(Clone)]
pub struct NoteDocument {
    pub path: String,
    pub contents: String,
}

/// Shared Files / tabs / Editor composition. The host supplies the initial
/// snapshot and owns persistence; this view updates the shared in-memory model.
pub struct WebWorkspace {
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    search_input: Entity<TextInput>,
    quick_switcher_input: Entity<TextInput>,
    quick_switcher_open: bool,
    quick_switcher_selected: usize,
    pending_quick_switcher_focus_restore: bool,
    pending_quick_switcher_open: Option<(NodeId, String)>,
    search_hits: Vec<SearchHit>,
    selected_search_hit: usize,
    pending_search_open: Option<usize>,
    search_error: Option<String>,
    search_generation: u64,
    search_documents: Vec<SearchDocument>,
    recent_files: Vec<String>,
    vault: VaultData,
    tree: FileTree,
    tree_focus: FocusHandle,
    tree_focused_key: Option<String>,
    file_tree_context_menu: Option<FileTreeContextMenu>,
    tab_context_menu: Option<TabContextMenu>,
    model: WorkspaceModel,
    sort_menu_open: bool,
    vault_name: String,
    drive_controls: bool,
    pending_link: Option<(String, bool)>,
    last_link_event: String,
    _subscriptions: Vec<Subscription>,
}

pub enum WebWorkspaceEvent {
    Edited { node: NodeId, contents: String },
    FileOperationRequested(FileOperationRequest),
    SaveRequested,
    SwitchVaultRequested,
}

impl EventEmitter<WebWorkspaceEvent> for WebWorkspace {}

impl WebWorkspace {
    pub fn from_vault(
        vault: VaultData,
        initial_node: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_path = initial_node.and_then(|node| vault.path(node).map(str::to_owned));
        let mut tree = FileTree::from_vault(&vault);
        tree.expand_all();
        let mut model = WorkspaceModel::new(ViewMode::Source);
        let editor = cx.new(|cx| Editor::new(window, cx));
        let reading = cx.new(ReadingView::new);
        let search_input = cx.new(|cx| TextInput::new("Search vault…", cx));
        let quick_switcher_input = cx.new(|cx| TextInput::new("Search files by name…", cx));
        let tree_focus = cx.focus_handle();
        let search_documents = build_search_index(&vault);
        let subscriptions = vec![
            cx.subscribe(
                &quick_switcher_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => {
                        this.quick_switcher_selected = 0;
                        cx.notify();
                    }
                    InputEvent::Down => {
                        let count = this.quick_switcher_results(cx).len();
                        if count > 0 {
                            this.quick_switcher_selected =
                                (this.quick_switcher_selected + 1) % count;
                            cx.notify();
                        }
                    }
                    InputEvent::Up => {
                        let count = this.quick_switcher_results(cx).len();
                        if count > 0 {
                            this.quick_switcher_selected = this
                                .quick_switcher_selected
                                .checked_sub(1)
                                .unwrap_or(count - 1);
                            cx.notify();
                        }
                    }
                    InputEvent::Submit => this.open_quick_switcher_selection(cx),
                    InputEvent::Cancel => {
                        this.quick_switcher_open = false;
                        this.pending_quick_switcher_focus_restore = true;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &search_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.rerun_search(cx),
                    InputEvent::Down if !this.search_hits.is_empty() => {
                        this.selected_search_hit =
                            (this.selected_search_hit + 1) % this.search_hits.len();
                        cx.notify();
                        this.publish_test_state();
                    }
                    InputEvent::Up if !this.search_hits.is_empty() => {
                        this.selected_search_hit = this
                            .selected_search_hit
                            .checked_sub(1)
                            .unwrap_or(this.search_hits.len() - 1);
                        cx.notify();
                        this.publish_test_state();
                    }
                    InputEvent::Submit if !this.search_hits.is_empty() => {
                        this.pending_search_open = Some(this.selected_search_hit);
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &editor,
                |this, _, event: &tft_app::editor::EditorEvent, cx| {
                    if matches!(event, tft_app::editor::EditorEvent::Edited) {
                        let Some(node) = this
                            .model
                            .active()
                            .and_then(|group| group.active())
                            .and_then(|tab| tab.node)
                        else {
                            return;
                        };
                        let contents = this.editor.read(cx).file_contents();
                        this.vault.set_text(node, contents.clone());
                        if let Some(path) = this.vault.path(node).map(str::to_owned) {
                            this.upsert_search_document(node, path, contents.clone());
                        }
                        cx.emit(WebWorkspaceEvent::Edited { node, contents });
                        this.publish_test_state();
                        return;
                    }
                    let (target, new_tab) = match event {
                        tft_app::editor::EditorEvent::OpenLink(target) => (target, false),
                        tft_app::editor::EditorEvent::OpenLinkInNewTab(target) => (target, true),
                        _ => return,
                    };
                    match target {
                        reading::LinkTarget::Internal(target) => {
                            this.pending_link = Some((target.clone(), new_tab));
                            cx.notify();
                        }
                        reading::LinkTarget::External(url) => cx.open_url(url),
                        _ => {}
                    }
                },
            ),
            cx.subscribe(&reading, |this, _, event: &ReadingEvent, cx| match event {
                ReadingEvent::OpenLink(reading::LinkTarget::Internal(target)) => {
                    this.last_link_event = target.clone();
                    this.pending_link = Some((target.clone(), false));
                    cx.notify();
                }
                ReadingEvent::OpenLinkInNewTab(reading::LinkTarget::Internal(target)) => {
                    this.last_link_event = target.clone();
                    this.pending_link = Some((target.clone(), true));
                    cx.notify();
                }
                ReadingEvent::OpenLink(reading::LinkTarget::External(url))
                | ReadingEvent::OpenLinkInNewTab(reading::LinkTarget::External(url)) => {
                    cx.open_url(url);
                }
                ReadingEvent::OpenLink(reading::LinkTarget::Footnote(number)) => {
                    if let Some(offset) = this
                        .reading
                        .read(cx)
                        .model()
                        .footnotes
                        .get(number.saturating_sub(1))
                        .copied()
                    {
                        this.reading
                            .update(cx, |reader, cx| reader.scroll_to_offset(offset, cx));
                    }
                }
                _ => {}
            }),
            cx.subscribe(&reading, |this, _, event: &TaskToggled, cx| {
                this.toggle_task(&event.path, event.marker, cx);
            }),
            cx.subscribe(&reading, |this, _, event: &PropertyItemRemoved, cx| {
                this.remove_property_item(&event.path, &event.key, event.index, cx);
            }),
        ];

        let mut recent_files = Vec::new();
        if let (Some(node), Some(path)) = (initial_node, initial_path) {
            recent_files.push(path.clone());
            if let Some(tab) = model.active_mut().and_then(|group| group.active_mut()) {
                tab.open_node(node, path.clone(), ViewMode::Source, 0);
            }
            let text = document_text(&vault, node);
            editor.update(cx, |editor, cx| editor.set_text(path, text, cx));
        }

        let focus = editor.read(cx).focus_handle_ref().clone();
        window.focus(&focus, cx);
        let view = Self {
            editor,
            reading,
            search_input,
            quick_switcher_input,
            quick_switcher_open: false,
            quick_switcher_selected: 0,
            pending_quick_switcher_focus_restore: false,
            pending_quick_switcher_open: None,
            search_hits: Vec::new(),
            selected_search_hit: 0,
            pending_search_open: None,
            search_error: None,
            search_generation: 0,
            search_documents,
            recent_files,
            vault,
            tree,
            tree_focus,
            tree_focused_key: None,
            file_tree_context_menu: None,
            tab_context_menu: None,
            model,
            pending_link: None,
            last_link_event: String::new(),
            sort_menu_open: false,
            vault_name: "Vault".into(),
            drive_controls: false,
            _subscriptions: subscriptions,
        };
        view.publish_test_state();
        view
    }

    fn rerun_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).text().trim().to_owned();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        self.search_hits.clear();
        self.selected_search_hit = 0;
        self.search_error = None;
        if query.is_empty() {
            cx.notify();
            self.publish_test_state();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            this.update(cx, |this, cx| {
                if this.search_generation == generation {
                    this.run_search(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn open_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_switcher_open = true;
        self.quick_switcher_selected = 0;
        self.quick_switcher_input
            .update(cx, |input, cx| input.set_text("", cx));
        let focus = self.quick_switcher_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn quick_switcher_results(&self, cx: &App) -> Vec<QuickSwitcherHit> {
        let query = self.quick_switcher_input.read(cx).text().trim().to_owned();
        let mut results = Vec::new();
        for document in &self.search_documents {
            let name = document
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&document.path)
                .trim_end_matches(".md")
                .to_owned();
            let score = if query.is_empty() {
                Some(0.0)
            } else {
                let by_name = tft_app::file_switcher::label_match(&query, &name);
                let by_path = tft_app::file_switcher::label_match(&query, &document.path)
                    .map(|(score, _)| 10_000.0 + 9_000.0 * score / (score + 50_000.0));
                match (by_name, by_path) {
                    (Some((name_score, _)), Some(path_score)) => Some(name_score.max(path_score)),
                    (Some((name_score, _)), None) => Some(name_score),
                    (None, Some(path_score)) => Some(path_score),
                    (None, None) => None,
                }
            };
            if let Some(score) = score {
                results.push(QuickSwitcherHit {
                    node: document.node,
                    path: document.path.clone(),
                    label: name,
                    score,
                    alias: false,
                });
            }
            if !query.is_empty() {
                for alias in tft_cache::frontmatter_aliases(&document.metadata) {
                    if let Some((score, _)) =
                        tft_app::file_switcher::label_match(&query, &alias)
                    {
                        results.push(QuickSwitcherHit {
                            node: document.node,
                            path: document.path.clone(),
                            label: alias,
                            score: score - 500.0,
                            alias: true,
                        });
                    }
                }
            }
        }
        results.sort_by(|a, b| {
            if query.is_empty() {
                let a_recent = self
                    .recent_files
                    .iter()
                    .position(|path| path == &a.path)
                    .unwrap_or(usize::MAX);
                let b_recent = self
                    .recent_files
                    .iter()
                    .position(|path| path == &b.path)
                    .unwrap_or(usize::MAX);
                a_recent
                    .cmp(&b_recent)
                    .then_with(|| tft_search::natural_cmp(&a.path, &b.path))
            } else {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.alias.cmp(&b.alias))
                    .then_with(|| a.label.len().cmp(&b.label.len()))
                    .then_with(|| tft_search::natural_cmp(&a.path, &b.path))
            }
        });
        results.truncate(50);
        results
    }

    fn open_quick_switcher_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(hit) = self
            .quick_switcher_results(cx)
            .get(self.quick_switcher_selected)
            .cloned()
        {
            self.pending_quick_switcher_open = Some((hit.node, hit.path));
            self.quick_switcher_open = false;
            cx.notify();
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        self.store_editor_text(active_node, cx);
        let query = self.search_input.read(cx).text().trim().to_owned();
        let search = match tft_search::Search::new(&query) {
            Ok(search) => search,
            Err(error) => {
                self.search_error = Some(error.to_string());
                cx.notify();
                self.publish_test_state();
                return;
            }
        };
        if search.is_empty() {
            cx.notify();
            self.publish_test_state();
            return;
        }
        for file in &self.search_documents {
            let contents = file.contents.as_str();
            let inputs = tft_search::inputs(&file.metadata, contents);
            let search_file = tft_search::SearchFile {
                path: &file.path,
                content: contents,
                tags: &inputs.tags,
                frontmatter: file.metadata.frontmatter.as_ref(),
                blocks: &inputs.blocks,
                sections: &inputs.sections,
                tasks: &inputs.tasks,
            };
            let Some(matches) = search.matches(&search_file) else {
                continue;
            };
            let snippets = tft_search::match_lines(contents, &matches.content)
                .into_iter()
                .take(2)
                .map(|line| line.snippet)
                .collect();
            self.search_hits.push(SearchHit {
                node: file.node,
                path: file.path.clone(),
                snippets,
            });
        }
        self.search_hits
            .sort_by(|a, b| tft_search::natural_cmp(&a.path, &b.path));
        cx.notify();
        self.publish_test_state();
    }

    #[cfg(feature = "browser-test")]
    pub fn from_documents(
        documents: Vec<NoteDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_path = documents.first().map(|doc| doc.path.clone());
        let files = documents
            .iter()
            .map(|doc| VaultFile {
                path: doc.path.clone(),
                kind: tft_formats::FileKind::from_path(&doc.path),
                size: doc.contents.len() as u64,
                mtime: 0,
                ctime: 0,
            })
            .collect::<Vec<_>>();
        let mut folders = BTreeSet::new();
        let mut texts = std::collections::HashMap::new();
        for doc in &documents {
            if let Some((directory, _)) = doc.path.rsplit_once('/') {
                let mut parent = String::new();
                for segment in directory.split('/') {
                    if !parent.is_empty() {
                        parent.push('/');
                    }
                    parent.push_str(segment);
                    folders.insert(parent.clone());
                }
            }
            texts.insert(doc.path.clone(), Arc::new(doc.contents.clone()));
        }

        let vault = VaultData::from_flat(files, folders.into_iter().collect(), texts);
        let initial_node = initial_path.and_then(|path| vault.lookup(&path).ok());
        Self::from_vault(vault, initial_node, window, cx)
    }

    fn open(
        &mut self,
        node: NodeId,
        path: String,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.vault.node(node).map(|node| &node.kind),
            Some(NodeKind::File(meta)) if meta.kind == tft_formats::FileKind::Markdown
        ) {
            return;
        }
        self.recent_files.retain(|recent| recent != &path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(50);
        if !new_tab {
            let existing = self
                .model
                .active()
                .and_then(|group| group.tabs.iter().position(|tab| tab.node == Some(node)));
            if let Some(index) = existing {
                if self
                    .model
                    .active()
                    .is_some_and(|group| group.active_tab != index)
                {
                    self.activate_tab(index, window, cx);
                }
                return;
            }
        }
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        let mode = self
            .model
            .active()
            .and_then(|group| group.active())
            .map_or(ViewMode::Source, |tab| tab.mode);
        if new_tab || active_node != Some(node) {
            self.store_editor_text(active_node, cx);
            if new_tab {
                self.model
                    .open_node_in_new_tab(self.model.active_group, node, path.clone(), mode);
            } else {
                self.model
                    .open_node(self.model.active_group, node, path.clone(), mode);
            }
            let text = document_text(&self.vault, node);
            self.editor
                .update(cx, |editor, cx| editor.set_text(path, text, cx));
            if mode.is_editing() {
                self.focus_editor(window, cx);
            } else {
                self.refresh_reading(cx);
            }
            cx.notify();
            self.publish_test_state();
        }
    }

    fn follow_link(
        &mut self,
        node: NodeId,
        path: String,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_tab {
            return self.open(node, path, true, window, cx);
        }
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        let mode = self
            .model
            .active()
            .and_then(|group| group.active())
            .map_or(ViewMode::Source, |tab| tab.mode);
        self.store_editor_text(active_node, cx);
        self.model
            .navigate_node(self.model.active_group, node, path.clone(), mode);
        let text = document_text(&self.vault, node);
        self.editor
            .update(cx, |editor, cx| editor.set_text(path, text, cx));
        if mode.is_editing() {
            self.focus_editor(window, cx);
        } else {
            self.refresh_reading(cx);
        }
        cx.notify();
        self.publish_test_state();
    }

    fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.model.active() else {
            return;
        };
        if index >= group.tabs.len() || index == group.active_tab {
            return;
        }
        let active_node = group.active().and_then(|tab| tab.node);
        self.store_editor_text(active_node, cx);
        if let Some(group) = self.model.active_mut() {
            group.active_tab = index;
        }
        if let Some(tab) = self.model.active().and_then(|group| group.active()) {
            let path = tab.path.clone().unwrap_or_default();
            let text = tab
                .node
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            self.editor
                .update(cx, |editor, cx| editor.set_text(path, text, cx));
        }
        if self
            .model
            .active()
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode == ViewMode::Reading)
        {
            self.refresh_reading(cx);
        }
        if self
            .model
            .active()
            .and_then(|group| group.active())
            .is_some_and(|tab| tab.mode != ViewMode::Reading)
        {
            self.focus_editor(window, cx);
        }
        cx.notify();
        self.publish_test_state();
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        self.store_editor_text(active_node, cx);
        if !self.model.close(self.model.active_group, index) {
            return;
        }
        if let Some(tab) = self.model.active().and_then(|group| group.active()) {
            let path = tab.path.clone().unwrap_or_default();
            let text = tab
                .node
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            self.editor
                .update(cx, |editor, cx| editor.set_text(path, text, cx));
            if tab.mode == ViewMode::Reading {
                self.refresh_reading(cx);
            } else {
                self.focus_editor(window, cx);
            }
        }
        cx.notify();
        self.publish_test_state();
    }

    fn toggle_tab_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .model
            .active_mut()
            .and_then(|group| group.tabs.get_mut(index))
        {
            tab.pinned = !tab.pinned;
            cx.notify();
            self.publish_test_state();
        }
    }

    fn close_tabs_except(
        &mut self,
        keep: impl Fn(usize, &tft_app::Tab) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group_id = self.model.active_group;
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        let remove = self
            .model
            .active()
            .map(|group| {
                group
                    .tabs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, tab)| (!keep(index, tab)).then_some(index))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if remove.is_empty() {
            return;
        }
        self.store_editor_text(active_node, cx);
        for index in remove.into_iter().rev() {
            self.model.close(group_id, index);
        }
        if let Some(tab) = self.model.active().and_then(|group| group.active()) {
            let path = tab.path.clone().unwrap_or_default();
            let text = tab
                .node
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            self.editor
                .update(cx, |editor, cx| editor.set_text(path, text, cx));
            if tab.mode == ViewMode::Reading {
                self.refresh_reading(cx);
            } else {
                self.focus_editor(window, cx);
            }
        }
        cx.notify();
        self.publish_test_state();
    }

    fn new_tab(&mut self, cx: &mut Context<Self>) {
        let active_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        self.store_editor_text(active_node, cx);
        if self.model.new_tab(ViewMode::Source).is_none() {
            return;
        }
        self.editor
            .update(cx, |editor, cx| editor.set_text("", String::new(), cx));
        cx.notify();
        self.publish_test_state();
    }

    fn store_editor_text(&mut self, node: Option<NodeId>, cx: &App) {
        if let Some(node) = node {
            let contents = self.editor.read(cx).file_contents();
            self.vault.set_text(node, contents.clone());
            if let Some(path) = self.vault.path(node).map(str::to_owned) {
                self.upsert_search_document(node, path, contents);
            }
        }
    }

    fn upsert_search_document(&mut self, node: NodeId, path: String, contents: String) {
        let contents = Arc::new(contents);
        let metadata = tft_cache::index_markdown(&contents);
        let document = SearchDocument {
            node,
            path,
            contents,
            metadata,
        };
        if let Some(existing) = self
            .search_documents
            .iter_mut()
            .find(|existing| existing.node == node)
        {
            *existing = document;
        } else {
            self.search_documents.push(document);
            self.search_documents
                .sort_by(|a, b| tft_search::natural_cmp(&a.path, &b.path));
        }
    }

    pub fn current_document(&self, cx: &App) -> Option<(NodeId, String)> {
        let tab = self.model.active()?.active()?;
        Some((tab.node?, self.editor.read(cx).file_contents()))
    }

    pub fn can_create_root_note(&self, name: &str) -> Result<(), String> {
        self.vault
            .validate_new_child_name(self.vault.root, name)
            .map_err(|error| match error {
                tft_app::RenameError::InvalidName => format!("Invalid file name: {name}"),
                tft_app::RenameError::DestinationExists => {
                    format!("A file or folder named {name} already exists.")
                }
                _ => "The Vault root is unavailable.".into(),
            })
    }

    pub fn insert_drive_note(
        &mut self,
        id: String,
        name: String,
        contents: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<NodeId, String> {
        self.can_create_root_note(&name)?;
        let parent = self
            .vault
            .backend(self.vault.root)
            .cloned()
            .ok_or_else(|| "The Vault root has no Drive identity.".to_string())?;
        let node = self.vault.upsert_object(
            BackendObjectId::Drive(id),
            Some(parent),
            name,
            NodeKind::File(FileMeta {
                kind: tft_formats::FileKind::Markdown,
                size: contents.len() as u64,
                mtime: 0,
                ctime: 0,
            }),
            None,
        );
        self.vault.rebuild_indexes();
        self.vault.set_text(node, contents);
        let mut tree = FileTree::from_vault(&self.vault);
        tree.preserve_view_from(&self.tree);
        self.tree = tree;
        let path = self
            .vault
            .path(node)
            .ok_or_else(|| "The new note has no Vault path.".to_string())?
            .to_owned();
        self.upsert_search_document(node, path.clone(), document_text(&self.vault, node));
        self.open(node, path, false, window, cx);
        Ok(node)
    }

    pub fn current_path(&self) -> Option<&str> {
        self.model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.path.as_deref())
    }

    pub fn set_vault_name(&mut self, name: String, cx: &mut Context<Self>) {
        if self.vault_name != name {
            self.vault_name = name;
            cx.notify();
        }
    }

    pub fn set_drive_controls(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.drive_controls != enabled {
            self.drive_controls = enabled;
            cx.notify();
        }
    }

    pub fn mark_saved(&mut self, node: NodeId, contents: String, cx: &mut Context<Self>) {
        if !self
            .vault
            .text(node)
            .is_some_and(|latest| latest.as_str() == contents)
        {
            return;
        }
        self.vault.set_text(node, contents.clone());
        let is_active = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node)
            == Some(node);
        if is_active {
            self.editor.update(cx, |editor, cx| {
                if editor.matches_file(&contents) {
                    editor.mark_saved(&contents);
                }
                cx.notify();
            });
        }
    }

    fn toggle_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mode) = self
            .model
            .active()
            .and_then(|group| group.active())
            .map(|tab| tab.mode)
        else {
            return;
        };
        let next = mode.toggle_reading_editing(ViewMode::Source);
        if let Some(tab) = self.model.active_mut().and_then(|group| group.active_mut()) {
            if let (Some(node), Some(path)) = (tab.node, tab.path.clone()) {
                tab.open_node(node, path, next, 0);
            } else {
                tab.mode = next;
            }
        }
        if next == ViewMode::Reading {
            self.refresh_reading(cx);
        } else {
            self.focus_editor(window, cx);
        }
        cx.notify();
        self.publish_test_state();
    }

    fn navigate_history(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current_node = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| tab.node);
        self.store_editor_text(current_node, cx);
        let entry = if forward {
            self.model.forward(self.model.active_group)
        } else {
            self.model.back(self.model.active_group)
        };
        let Some(entry) = entry else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };
        let mode = entry.mode;
        let text = document_text(&self.vault, node);
        self.editor
            .update(cx, |editor, cx| editor.set_text(entry.path, text, cx));
        if mode.is_editing() {
            self.focus_editor(window, cx);
        } else {
            self.refresh_reading(cx);
        }
        cx.notify();
        self.publish_test_state();
    }

    fn refresh_reading(&mut self, cx: &mut Context<Self>) {
        let Some((node, path)) = self
            .model
            .active()
            .and_then(|group| group.active())
            .and_then(|tab| Some((tab.node?, tab.path.clone()?)))
        else {
            return;
        };
        let contents = self.editor.read(cx).file_contents();
        self.vault.set_text(node, contents.clone());
        let source = VaultEmbedSource::new(&self.vault);
        let model =
            reading::build_with(&path, &contents, &source, reading::BuildOptions::default());
        self.reading
            .update(cx, |reader, cx| reader.set_note(Some(path), model, cx));
        self.publish_test_state();
    }

    fn toggle_task(&mut self, path: &str, marker: reading::TaskMarker, cx: &mut Context<Self>) {
        let active_path = self.current_path().map(str::to_owned);
        if active_path.as_deref() != Some(path) {
            return;
        }
        let changed = self.editor.update(cx, |editor, cx| {
            editor.toggle_task_at(marker.line, marker.col, None, cx)
        });
        if changed {
            self.refresh_reading(cx);
            cx.notify();
        }
    }

    fn remove_property_item(
        &mut self,
        path: &str,
        key: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let active_path = self.current_path().map(str::to_owned);
        if active_path.as_deref() != Some(path) {
            return;
        }
        let changed = self.editor.update(cx, |editor, cx| {
            editor.remove_property_item(key, index, None, cx)
        });
        if changed {
            self.refresh_reading(cx);
            cx.notify();
        }
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut App) {
        let focus = self.editor.read(cx).focus_handle_ref().clone();
        window.focus(&focus, cx);
    }

    fn tree_focus_index(&self) -> Option<usize> {
        let key = self.tree_focused_key.as_deref()?;
        self.tree.rows().position(|row| row.key == key)
    }

    fn handle_tree_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.file_tree_context_menu.is_some() {
            self.file_tree_context_menu = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.tree.is_empty() {
            if matches!(event.keystroke.key.as_str(), "escape" | "tab") {
                self.focus_editor(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let current_key = self
            .tree_focus_index()
            .and_then(|index| self.tree.row(index).map(|row| row.key.clone()))
            .or_else(|| {
                self.current_path()
                    .and_then(|path| self.tree.visible_index(path))
                    .and_then(|index| self.tree.row(index).map(|row| row.key.clone()))
            })
            .or_else(|| self.tree.row(0).map(|row| row.key.clone()));
        let key = event.keystroke.key.as_str();
        let mut handled = true;
        match key {
            "down" | "up" => {
                if let Some(next) = self.tree.focus_vertical_key(
                    self.tree_focused_key.as_deref(),
                    self.current_path(),
                    key == "down",
                ) {
                    self.tree_focused_key = Some(next);
                    cx.notify();
                }
            }
            "right" | "left" => {
                if let Some(current) = current_key.as_deref() {
                    if let Some(next) = self.tree.focus_horizontal_key(current, key == "right") {
                        self.tree_focused_key = Some(next);
                        cx.notify();
                    }
                }
            }
            "enter" | "space" => {
                if let Some(row) = current_key
                    .as_deref()
                    .and_then(|key| self.tree.rows().find(|row| row.key == key).cloned())
                {
                    if row.is_folder {
                        self.tree.toggle_key(&row.key);
                        self.tree_focused_key = Some(row.key);
                        cx.notify();
                    } else if key == "enter" {
                        if let Some(node) = row.node {
                            self.open(node, row.path, false, window, cx);
                        }
                    }
                }
            }
            "escape" | "tab" => self.focus_editor(window, cx),
            _ => handled = false,
        }
        if handled {
            cx.stop_propagation();
        }
    }

    fn publish_test_state(&self) {
        #[cfg(all(feature = "browser-test", target_arch = "wasm32"))]
        {
            use wasm_bindgen::JsValue;
            let state = js_sys::Object::new();
            let tabs = js_sys::Array::new();
            let search_results = js_sys::Array::new();
            let visible_paths = js_sys::Array::new();
            for row in self.tree.rows() {
                visible_paths.push(&JsValue::from_str(&row.path));
            }
            for hit in &self.search_hits {
                search_results.push(&JsValue::from_str(&hit.path));
            }
            if let Some(group) = self.model.active() {
                for tab in &group.tabs {
                    tabs.push(&JsValue::from_str(tab.path.as_deref().unwrap_or("")));
                }
            }
            let active = self.model.active().and_then(|group| group.active());
            let path = active.and_then(|tab| tab.path.as_deref()).unwrap_or("");
            let node = active
                .and_then(|tab| tab.node)
                .map_or(-1, |node| node.0 as i32);
            let mode = active.map_or("source", |tab| match tab.mode {
                ViewMode::Reading => "reading",
                ViewMode::LivePreview => "live-preview",
                ViewMode::Source => "source",
                ViewMode::Graph => "graph",
                ViewMode::Search => "search",
                ViewMode::Terminal => "terminal",
            });
            let left_tab = match self.model.left_tab {
                LeftTab::Files => "files",
                LeftTab::Search => "search",
            };
            let last_link = self.last_link_event.clone();
            let vault_text = active
                .and_then(|tab| tab.node)
                .map_or_else(String::new, |node| document_text(&self.vault, node));
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("activePath"),
                &JsValue::from_str(path),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("activeNode"),
                &JsValue::from_f64(node as f64),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("vaultText"),
                &JsValue::from_str(&vault_text),
            );
            let _ = js_sys::Reflect::set(&state, &JsValue::from_str("tabs"), &tabs);
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("leftTab"),
                &JsValue::from_str(left_tab),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("leftVisible"),
                &JsValue::from_bool(self.model.left_visible),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("sortOrder"),
                &JsValue::from_str(self.tree.sort().as_str()),
            );
            let _ =
                js_sys::Reflect::set(&state, &JsValue::from_str("visiblePaths"), &visible_paths);
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("sortMenuOpen"),
                &JsValue::from_bool(self.sort_menu_open),
            );
            let _ =
                js_sys::Reflect::set(&state, &JsValue::from_str("searchResults"), &search_results);
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("selectedSearchHit"),
                &JsValue::from_f64(self.selected_search_hit as f64),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("activeMode"),
                &JsValue::from_str(mode),
            );
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str("lastLinkEvent"),
                &JsValue::from_str(&last_link),
            );
            let _ = js_sys::Reflect::set(
                &js_sys::global(),
                &JsValue::from_str("__tftWorkspaceState"),
                &state,
            );
        }
    }
}

fn document_text(vault: &VaultData, node: NodeId) -> String {
    vault
        .text(node)
        .map(|text| text.as_ref().clone())
        .unwrap_or_default()
}

fn build_search_index(vault: &VaultData) -> Vec<SearchDocument> {
    let mut documents = vault
        .files()
        .iter()
        .filter(|file| file.kind == tft_formats::FileKind::Markdown)
        .filter_map(|file| {
            let node = vault.lookup(&file.path).ok()?;
            let contents = vault.text(node)?.clone();
            let metadata = tft_cache::index_markdown(&contents);
            Some(SearchDocument {
                node,
                path: file.path.clone(),
                contents,
                metadata,
            })
        })
        .collect::<Vec<_>>();
    documents.sort_by(|a, b| tft_search::natural_cmp(&a.path, &b.path));
    documents
}

impl Render for WebWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pending_quick_switcher_focus_restore {
            self.pending_quick_switcher_focus_restore = false;
            self.focus_editor(window, cx);
        }
        if let Some((node, path)) = self.pending_quick_switcher_open.take() {
            self.open(node, path, false, window, cx);
        }
        if let Some(index) = self.pending_search_open.take() {
            if let Some(hit) = self.search_hits.get(index) {
                self.open(hit.node, hit.path.clone(), false, window, cx);
            }
        }
        if let Some((target, new_tab)) = self.pending_link.take() {
            let source_path = self.current_path().unwrap_or_default().to_owned();
            let resolver = vault_resolver(&self.vault);
            if let Some(path) = resolver.resolve(&target, &source_path).map(str::to_owned) {
                if let Ok(node) = self.vault.lookup(&path) {
                    self.follow_link(node, path, new_tab, window, cx);
                }
            }
        }
        let theme = cx.global::<Theme>().clone();
        let tokens = &theme.tokens;
        let quick_switcher_results = self.quick_switcher_results(cx);
        let quick_switcher_input = self.quick_switcher_input.clone();
        let active_tab = self.model.active().and_then(|group| group.active());
        let active_node = active_tab.and_then(|tab| tab.node);
        let active_index = self.model.active().map_or(0, |group| group.active_tab);
        let can_back = active_tab.is_some_and(|tab| tab.history_ix > 1);
        let can_forward = active_tab.is_some_and(|tab| tab.history_ix < tab.history.len());
        let rows = self
            .tree
            .rows()
            .map(|row| {
                (
                    row.node,
                    row.key.clone(),
                    row.path.clone(),
                    row.name.clone(),
                    row.depth,
                    row.is_folder,
                    row.node.is_some() && row.node == active_node,
                    self.tree_focused_key.as_deref() == Some(row.key.as_str()),
                    self.tree.is_collapsed_key(&row.key),
                )
            })
            .collect::<Vec<_>>();
        let tabs = self
            .model
            .active()
            .into_iter()
            .flat_map(|group| group.tabs.iter())
            .enumerate()
            .map(|(index, tab)| {
                (
                    index,
                    tab.path.clone().unwrap_or_default(),
                    tft_app::workspace::workspace_leaf_title(tab.path.as_deref(), tab.mode),
                    tab.pinned,
                    index == active_index,
                )
            })
            .collect::<Vec<_>>();
        let editor = self.editor.clone();
        let reading = self.reading.clone();
        let search_input = self.search_input.clone();
        let searching = self.model.left_tab == LeftTab::Search
            && !self.search_input.read(cx).text().trim().is_empty();
        let search_hits = self.search_hits.clone();
        let selected_search_hit = self.selected_search_hit;
        let search_error = self.search_error.clone();
        let is_reading = active_tab.is_some_and(|tab| tab.mode == ViewMode::Reading);
        let is_empty_tab = active_tab.is_some_and(|tab| {
            tab.path.is_none()
                && !matches!(
                    tab.mode,
                    ViewMode::Graph | ViewMode::Search | ViewMode::Terminal
                )
        });
        let main_content = if is_empty_tab {
            tft_app::ui::workspace_empty_tab(&theme)
                .child(
                    div()
                        .w(px(240.))
                        .flex()
                        .flex_col()
                        .child(
                            tft_app::ui::workspace_empty_action(
                                "web-empty-new-note",
                                "New note",
                                None,
                                &theme,
                            )
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(WebWorkspaceEvent::FileOperationRequested(
                                    FileOperationRequest::CreateNote {
                                        parent: String::new(),
                                    },
                                ))
                            })),
                        )
                        .child(
                            tft_app::ui::workspace_empty_action(
                                "web-empty-go-to-file",
                                "Go to file",
                                None,
                                &theme,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.open_quick_switcher(window, cx);
                                },
                            )),
                        ),
                )
                .into_any_element()
        } else if is_reading {
            reading.clone().into_any_element()
        } else {
            editor.clone().into_any_element()
        };
        let mode_icon = if is_reading { "book-open" } else { "pencil" };
        let active_title = tft_app::workspace::workspace_leaf_title(
            active_tab.and_then(|tab| tab.path.as_deref()),
            active_tab.map_or(ViewMode::Source, |tab| tab.mode),
        );
        let breadcrumbs = tft_app::workspace::workspace_breadcrumbs(
            active_tab.and_then(|tab| tab.path.as_deref()),
        );
        let mut breadcrumb = div().flex().flex_row().items_center().whitespace_nowrap();
        for (index, (label, path)) in breadcrumbs.iter().cloned().enumerate() {
            breadcrumb = breadcrumb.child(
                tft_app::ui::workspace_breadcrumb_item(
                    ("web-document-folder-crumb", index),
                    label,
                    &theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tree.reveal(&path);
                    this.model.left_tab = LeftTab::Files;
                    this.model.left_visible = true;
                    cx.notify();
                    this.publish_test_state();
                })),
            );
            if index + 1 < breadcrumbs.len() {
                breadcrumb = breadcrumb.child(tft_app::ui::workspace_breadcrumb_separator(&theme));
            }
        }
        let files_content = if searching {
            div()
                .id("web-search-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .when_some(search_error, |container, error| {
                    container.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(tokens.text_muted)
                            .child(error),
                    )
                })
                .children(search_hits.into_iter().enumerate().map(|(index, hit)| {
                    let node = hit.node;
                    let path = hit.path.clone();
                    let title = hit.path.rsplit('/').next().unwrap_or(&hit.path).to_owned();
                    tft_app::ui::file_tree_row(
                        SharedString::from(format!("search-{}", hit.path)),
                        0,
                        index == selected_search_hit,
                        false,
                        &theme,
                    )
                    .py_1()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open(node, path.clone(), false, window, cx);
                    }))
                    .child(
                        div().flex().flex_col().min_w_0().child(title).children(
                            hit.snippets.into_iter().map(|snippet| {
                                div()
                                    .text_size(px(11.))
                                    .text_color(tokens.text_muted)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(snippet)
                            }),
                        ),
                    )
                }))
                .into_any_element()
        } else {
            div()
                .id("web-files-scroll")
                .key_context("WebExplorer")
                .track_focus(&self.tree_focus)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.handle_tree_key(event, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        window.focus(&this.tree_focus, cx);
                        this.file_tree_context_menu = Some(FileTreeContextMenu {
                            position: event.position,
                            node: None,
                            key: String::new(),
                            path: String::new(),
                            is_folder: false,
                        });
                        cx.notify();
                    }),
                )
                .children(rows.into_iter().map(
                    |(node, key, path, name, depth, folder, active, focused, collapsed)| {
                        let row_path = path.clone();
                        let menu_key = key.clone();
                        let menu_path = path.clone();
                        let menu_node = node;
                        tft_app::ui::file_tree_row(
                            SharedString::from(format!("file-{key}")),
                            depth,
                            active,
                            focused,
                            &theme,
                        )
                        .on_click(
                            cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                                this.file_tree_context_menu = None;
                                this.tree_focused_key = Some(key.clone());
                                if folder {
                                    window.focus(&this.tree_focus, cx);
                                    this.tree.toggle_key(&key);
                                    cx.notify();
                                } else if let Some(node) = node {
                                    let modifiers = event.modifiers();
                                    this.open(
                                        node,
                                        row_path.clone(),
                                        modifiers.control || modifiers.platform,
                                        window,
                                        cx,
                                    );
                                }
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                window.focus(&this.tree_focus, cx);
                                this.sort_menu_open = false;
                                this.file_tree_context_menu = Some(FileTreeContextMenu {
                                    position: event.position,
                                    node: menu_node,
                                    key: menu_key.clone(),
                                    path: menu_path.clone(),
                                    is_folder: folder,
                                });
                                cx.notify();
                            }),
                        )
                        .children(tft_app::ui::file_tree_indent_guides(depth, &theme))
                        .child(tft_app::ui::file_tree_row_marker(folder, collapsed, &theme))
                        .child(tft_app::ui::file_tree_row_label().child(name))
                    },
                ))
                .into_any_element()
        };

        let files_tab = self.model.left_tab == LeftTab::Files;
        let search_tab = self.model.left_tab == LeftTab::Search;
        let drive_controls = self.drive_controls;
        let sort_order = self.tree.sort();
        let any_expanded = self.tree.any_expanded();
        let sort_menu = self.sort_menu_open.then(|| {
            tft_app::ui::menu("web-sort-menu", &theme)
                .absolute()
                .top(px(76.))
                .left(px(40.))
                .children(
                    SortOrder::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, order)| {
                            tft_app::ui::menu_item(
                                "web-sort-option",
                                index,
                                None,
                                order.label(),
                                order == sort_order,
                                &theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.tree.set_sort(order);
                                    this.sort_menu_open = false;
                                    cx.notify();
                                    this.publish_test_state();
                                },
                            ))
                        }),
                )
                .into_any_element()
        });
        let file_tree_context_menu = self.file_tree_context_menu.clone().map(|target| {
            let viewport = window.bounds().size;
            let parent = if target.is_folder {
                target.path.clone()
            } else {
                target
                    .path
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_owned())
                    .unwrap_or_default()
            };
            let mut menu = tft_app::ui::menu("web-file-tree-context-menu", &theme)
                .absolute()
                .left(
                    target
                        .position
                        .x
                        .min((viewport.width - px(210.)).max(px(0.))),
                )
                .top(
                    target
                        .position
                        .y
                        .min((viewport.height - px(150.)).max(px(0.))),
                );

            if let Some(node) = target.node.filter(|_| !target.is_folder) {
                let path = target.path.clone();
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-file-tree-menu-open-tab",
                        0,
                        Some("file-plus"),
                        "Open in new tab",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.file_tree_context_menu = None;
                        this.open(node, path.clone(), true, window, cx);
                    })),
                );

                let path = target.path.clone();
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-file-tree-menu-reveal",
                        1,
                        Some("folder-open"),
                        "Reveal in navigation",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.file_tree_context_menu = None;
                        this.tree.reveal(&path);
                        this.tree_focused_key = this
                            .tree
                            .visible_index(&path)
                            .and_then(|index| this.tree.row(index).map(|row| row.key.clone()));
                        this.model.left_tab = LeftTab::Files;
                        this.model.left_visible = true;
                        cx.notify();
                        this.publish_test_state();
                    })),
                );

                let path = target.path.clone();
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-file-tree-menu-copy-path",
                        2,
                        Some("clipboard"),
                        "Copy relative path",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        this.file_tree_context_menu = None;
                        cx.notify();
                    })),
                );
            }

            let new_note_parent = parent.clone();
            menu = menu.child(
                tft_app::ui::menu_item(
                    "web-file-tree-menu-new-note",
                    3,
                    Some("file-plus"),
                    "New note",
                    false,
                    &theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.file_tree_context_menu = None;
                    cx.emit(WebWorkspaceEvent::FileOperationRequested(
                        FileOperationRequest::CreateNote {
                            parent: new_note_parent.clone(),
                        },
                    ));
                })),
            );

            if target.is_folder {
                let key = target.key.clone();
                let collapsed = self.tree.is_collapsed_key(&key);
                let title = if collapsed {
                    "Expand folder"
                } else {
                    "Collapse folder"
                };
                menu = menu.child(tft_app::ui::menu_separator(&theme)).child(
                    tft_app::ui::menu_item(
                        "web-file-tree-menu-toggle-folder",
                        4,
                        Some(if collapsed { "folder-open" } else { "folder" }),
                        title,
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tree.toggle_key(&key);
                        this.file_tree_context_menu = None;
                        cx.notify();
                        this.publish_test_state();
                    })),
                );
            }
            menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        });

        let quick_switcher_overlay = self.quick_switcher_open.then(|| {
            let count = quick_switcher_results.len();
            let panel = tft_app::ui::palette_panel("web-quick-switcher", 620., &theme)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.quick_switcher_open = false;
                    this.pending_quick_switcher_focus_restore = true;
                    cx.notify();
                }))
                .child(tft_app::ui::palette_search_slot(&theme).child(quick_switcher_input))
                .when(count > 0, |panel| {
                    panel.child(
                        tft_app::ui::palette_results_slot()
                            .id("web-quick-switcher-results")
                            .max_h(px(468.))
                            .overflow_y_scroll()
                            .children(quick_switcher_results.iter().enumerate().map(
                                |(index, hit)| {
                                    let node = hit.node;
                                    let path = hit.path.clone();
                                    let name = hit.label.clone();
                                    let detail = if hit.alias {
                                        Some(format!(
                                            "alias of {}",
                                            path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".md")
                                        ))
                                    } else {
                                        path.rsplit_once('/')
                                            .map(|(parent, _)| parent.to_owned())
                                    };
                                    tft_app::ui::palette_result_row(
                                        SharedString::from(format!("web-quick-switcher-{index}")),
                                        index == self.quick_switcher_selected,
                                        &theme,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.quick_switcher_selected = index;
                                        this.pending_quick_switcher_open =
                                            Some((node, path.clone()));
                                        this.quick_switcher_open = false;
                                        cx.notify();
                                    }))
                                    .child(tft_app::assets::icon(
                                        "files",
                                        16.,
                                        theme.tokens.text_muted,
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(14.))
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(name),
                                    )
                                    .children(detail.map(
                                        |detail| {
                                            div()
                                                .max_w(px(220.))
                                                .flex_shrink_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .text_size(px(11.))
                                                .text_color(theme.tokens.text_muted)
                                                .child(detail)
                                        },
                                    ))
                                },
                            )),
                    )
                })
                .when(count == 0, |panel| {
                    panel.child(
                        tft_app::ui::palette_results_slot().child(
                            div()
                                .h(px(38.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .text_size(px(12.))
                                .text_color(theme.tokens.text_muted)
                                .child("No matching files."),
                        ),
                    )
                });
            div()
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(90.))
                .child(panel)
        });

        let tab_context_menu = self.tab_context_menu.clone().and_then(|target| {
            let group = self.model.active()?;
            let tab = group.tabs.get(target.index)?;
            let index = target.index;
            let total = group.tabs.len();
            let pinned = tab.pinned;
            let viewport = window.bounds().size;
            let mut menu = tft_app::ui::menu("web-tab-context-menu", &theme)
                .absolute()
                .left(
                    target
                        .position
                        .x
                        .min((viewport.width - px(210.)).max(px(0.))),
                )
                .top(
                    target
                        .position
                        .y
                        .min((viewport.height - px(230.)).max(px(0.))),
                )
                .child(
                    tft_app::ui::menu_item(
                        "web-tab-close",
                        0,
                        Some("x"),
                        "Close",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.close_tab(index, window, cx);
                    })),
                );
            if total > 1 {
                menu = menu.child(tft_app::ui::menu_separator(&theme)).child(
                    tft_app::ui::menu_item(
                        "web-tab-close-others",
                        1,
                        Some("x"),
                        "Close others",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.close_tabs_except(|ix, tab| ix == index || tab.pinned, window, cx);
                    })),
                );
                if index + 1 < total {
                    menu = menu.child(
                        tft_app::ui::menu_item(
                            "web-tab-close-after",
                            2,
                            Some("x"),
                            "Close tabs after",
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.tab_context_menu = None;
                            this.close_tabs_except(
                                |ix, tab| ix <= index || tab.pinned,
                                window,
                                cx,
                            );
                        })),
                    );
                }
                menu = menu.child(
                    tft_app::ui::menu_item(
                        "web-tab-close-all",
                        3,
                        Some("x"),
                        "Close all",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_context_menu = None;
                        this.close_tabs_except(|_, tab| tab.pinned, window, cx);
                    })),
                );
            }
            Some(
                menu.child(tft_app::ui::menu_separator(&theme)).child(
                    tft_app::ui::menu_item(
                        "web-tab-toggle-pin",
                        4,
                        Some("pin"),
                        if pinned { "Unpin" } else { "Pin" },
                        pinned,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.toggle_tab_pin(index, cx);
                    })),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
            )
        });

        div()
            .size_full()
            .relative()
            .on_action(cx.listener(|this, _: &OpenQuickSwitcher, window, cx| {
                this.open_quick_switcher(window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.file_tree_context_menu.take().is_some()
                        || this.tab_context_menu.take().is_some()
                    {
                        cx.notify();
                    }
                }),
            )
            .flex()
            .flex_col()
            .bg(tokens.background_primary)
            .text_color(tokens.text_normal)
            .font_family(theme.ui_font.clone())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(self.model.left_visible, |layout| {
                        layout.child(
                            tft_app::ui::workspace_dock(px(tft_app::theme::SIDEBAR_WIDTH), &theme)
                                .border_r_1()
                                .border_color(tokens.tab_outline_color)
                                .child(
                                    tft_app::ui::workspace_dock_header(&theme)
                                        .gap(px(3.))
                                        .px_2()
                                        .child(
                                            tft_app::ui::dock_tab(
                                                "web-dock-files",
                                                "folder-closed",
                                                files_tab,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.model.left_tab = LeftTab::Files;
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        )
                                        .child(
                                            tft_app::ui::dock_tab(
                                                "web-dock-search",
                                                "search",
                                                search_tab,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.model.left_tab = LeftTab::Search;
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            tft_app::ui::icon_button(
                                                "web-sidebar-toggle",
                                                "panel-left",
                                                &theme,
                                            )
                                            .opacity(0.72)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.model.left_visible = false;
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .h(px(tft_app::theme::HEADER_HEIGHT))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap(px(2.))
                                        .border_b_1()
                                        .border_color(tokens.tab_outline_color)
                                        .child(
                                            tft_app::ui::header_button(
                                                "web-explorer-sort",
                                                "arrow-up-narrow-wide",
                                                "Change sort order",
                                                self.sort_menu_open,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.sort_menu_open = !this.sort_menu_open;
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        )
                                        .child(
                                            tft_app::ui::header_button(
                                                "web-explorer-reveal",
                                                "gallery-vertical",
                                                "Auto-reveal current file",
                                                false,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    if let Some(path) =
                                                        this.current_path().map(str::to_owned)
                                                    {
                                                        this.tree.reveal(&path);
                                                        this.model.left_tab = LeftTab::Files;
                                                    }
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        )
                                        .child(
                                            tft_app::ui::header_button(
                                                "web-explorer-expand",
                                                "chevrons-up-down",
                                                if any_expanded {
                                                    "Collapse all"
                                                } else {
                                                    "Expand all"
                                                },
                                                false,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    if this.tree.any_expanded() {
                                                        this.tree.collapse_all();
                                                    } else {
                                                        this.tree.expand_all();
                                                    }
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        )
                                        .child(
                                            tft_app::ui::icon_button(
                                                "web-new-note",
                                                "plus",
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|_, _, _, cx| {
                                                    cx.emit(
                                                        WebWorkspaceEvent::FileOperationRequested(
                                                            FileOperationRequest::CreateNote {
                                                                parent: String::new(),
                                                            },
                                                        ),
                                                    )
                                                }),
                                            ),
                                        ),
                                )
                                .when(search_tab, |panel| {
                                    panel.child(div().w_full().px_2().py_1().child(search_input))
                                })
                                .child(files_content)
                                .child(
                                    div()
                                        .h(px(tft_app::theme::HEADER_HEIGHT))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .px_2()
                                        .gap(px(3.))
                                        .border_t_1()
                                        .border_color(tokens.divider_color)
                                        .child(
                                            tft_app::ui::icon_button(
                                                "web-vault-switcher",
                                                "chevrons-up-down",
                                                &theme,
                                            )
                                            .opacity(0.65)
                                            .on_click(
                                                cx.listener(|_, _, _, cx| {
                                                    cx.emit(WebWorkspaceEvent::SwitchVaultRequested)
                                                }),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .whitespace_nowrap()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .text_size(px(13.))
                                                .text_color(tokens.vault_profile_color)
                                                .child(self.vault_name.clone()),
                                        ),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                tft_app::ui::workspace_tab_strip_shell(&theme)
                                    .flex_shrink_0()
                                    .when(!self.model.left_visible, |strip| {
                                        strip.child(
                                            tft_app::ui::icon_button(
                                                "web-sidebar-restore",
                                                "panel-left",
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.model.left_visible = true;
                                                    cx.notify();
                                                    this.publish_test_state();
                                                }),
                                            ),
                                        )
                                    })
                                    .child(tft_app::ui::workspace_tab_history_controls(
                                        tft_app::ui::icon_button(
                                            "web-tab-back",
                                            "arrow-left",
                                            &theme,
                                        )
                                        .when(!can_back, |button| button.opacity(0.4))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate_history(false, window, cx)
                                            }),
                                        ),
                                        tft_app::ui::icon_button(
                                            "web-tab-forward",
                                            "arrow-right",
                                            &theme,
                                        )
                                        .when(!can_forward, |button| button.opacity(0.4))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate_history(true, window, cx)
                                            }),
                                        ),
                                    ))
                                    .child(tft_app::ui::workspace_tab_strip_divider(&theme))
                                    .child(
                                        tft_app::ui::workspace_tab_strip_scroll(
                                            "web-tab-strip-scroll",
                                        )
                                        .children(tabs.into_iter().map(
                                            |(index, path, title, pinned, active)| {
                                                tft_app::ui::workspace_tab(
                                                    SharedString::from(format!(
                                                        "tab-{index}-{path}"
                                                    )),
                                                    active,
                                                    true,
                                                    &theme,
                                                )
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.activate_tab(index, window, cx)
                                                    },
                                                ))
                                                .on_mouse_down(
                                                    gpui::MouseButton::Middle,
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.close_tab(index, window, cx)
                                                    }),
                                                )
                                                .on_mouse_down(
                                                    MouseButton::Right,
                                                    cx.listener(move |this, event, _, cx| {
                                                        cx.stop_propagation();
                                                        this.file_tree_context_menu = None;
                                                        this.tab_context_menu = Some(TabContextMenu {
                                                            position: event.position,
                                                            index,
                                                        });
                                                        cx.notify();
                                                    }),
                                                )
                                                .child(
                                                    tft_app::ui::workspace_tab_inner(
                                                        active, &theme,
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .whitespace_nowrap()
                                                            .overflow_hidden()
                                                            .text_ellipsis()
                                                            .child(title),
                                                    )
                                                    .child(
                                                        tft_app::ui::workspace_tab_close_button(
                                                            format!("close-tab-{index}"),
                                                            active,
                                                            pinned,
                                                            &theme,
                                                        )
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                cx.stop_propagation();
                                                                this.close_tab(index, window, cx);
                                                            },
                                                        )),
                                                    ),
                                                )
                                            },
                                        ))
                                        .child(
                                            tft_app::ui::icon_button("web-new-tab", "plus", &theme)
                                                .w(px(32.))
                                                .h_full()
                                                .rounded(px(0.))
                                                .hover(|style| {
                                                    style.bg(tokens.background_modifier_hover)
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.new_tab(cx);
                                                })),
                                        ),
                                    ),
                            )
                            .child(
                                tft_app::ui::workspace_view_header(&theme)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .pl(px(12.))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .when(!breadcrumbs.is_empty(), |header| {
                                                header.child(breadcrumb).child(
                                                    tft_app::ui::workspace_breadcrumb_separator(
                                                        &theme,
                                                    ),
                                                )
                                            })
                                            .child(active_title),
                                    )
                                    .when(
                                        tft_app::workspace::workspace_header_shows_mode_switch(
                                            active_tab.and_then(|tab| tab.path.as_deref()),
                                            active_tab.map_or(ViewMode::Source, |tab| tab.mode),
                                        ),
                                        |header| {
                                            header.child(
                                                tft_app::ui::icon_button(
                                                    "web-mode", mode_icon, &theme,
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_view(window, cx)
                                                })),
                                            )
                                        },
                                    )
                                    .when(drive_controls, |header| {
                                        header.child(
                                            tft_app::ui::header_button(
                                                "web-save-drive",
                                                "check",
                                                "Save to Drive",
                                                false,
                                                &theme,
                                            )
                                            .on_click(
                                                cx.listener(|_, _, _, cx| {
                                                    cx.emit(WebWorkspaceEvent::SaveRequested)
                                                }),
                                            ),
                                        )
                                    }),
                            )
                            .child(div().flex_1().min_h_0().child(main_content)),
                    ),
            )
            .children(sort_menu)
            .children(file_tree_context_menu)
            .children(tab_context_menu)
            .children(quick_switcher_overlay)
    }
}

fn vault_resolver(vault: &VaultData) -> LinkResolver {
    LinkResolver::new(vault.files().iter().map(|file| file.path.clone()))
}

struct VaultEmbedSource<'a> {
    vault: &'a VaultData,
    resolver: LinkResolver,
}

impl<'a> VaultEmbedSource<'a> {
    fn new(vault: &'a VaultData) -> Self {
        Self {
            vault,
            resolver: vault_resolver(vault),
        }
    }
}

impl EmbedSource for VaultEmbedSource<'_> {
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String> {
        self.resolver
            .resolve(linktext, source_path)
            .map(str::to_owned)
    }

    fn read(&self, path: &str) -> Option<String> {
        let node = self.vault.lookup(path).ok()?;
        self.vault.text(node).map(|text| text.as_ref().clone())
    }

    fn absolute(&self, path: &str) -> PathBuf {
        PathBuf::from(path)
    }
}
