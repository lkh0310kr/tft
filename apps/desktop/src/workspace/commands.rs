use super::attachments::download_modal;
use super::{
    FocusExplorer, GoBack, GoForward, MainView, Modal, OpenCommandPalette, OpenGraph, OpenSearch,
    ToggleMode, TreeActivate, TreeDown, TreeLeft, TreeRight, TreeUp, ViewMode, Workspace,
};
use crate::editor::Editor;
use crate::popout::PopoutWindow;
use crate::zoom;
use gpui::{Action, App, ClipboardItem, Context, Entity, Focusable, Global, KeyBinding, Window};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tft_app::SplitDirection;
use tft_commands::{CommandRegistry, hotkey_label, keystroke};
use tft_formats::config::Hotkey;
use tft_formats::FileKind;
use tft_text::table::TableCommand;
use tft_text::{BlockInsert, LinePrefix, LinkSyntax};
use tft_vault::TrashOption;

/// Runs the command with this Obsidian id (bound from the registry's hotkeys).
#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct RunCommand {
    pub id: String,
}

/// The command registry with the vault's `hotkeys.json` applied.
pub struct Commands(pub Arc<CommandRegistry>);

impl Global for Commands {}

/// Command history and the transient slash-command suppression state.
#[derive(Default)]
pub(super) struct CommandState {
    recent: Vec<String>,
    running_from_slash: bool,
}

/// The ids that need an editor on screen with the keyboard in it (Obsidian's
/// `editorCallback`); [`Workspace::editing_or_report`] guards the hotkey path too.
pub const EDITOR_ONLY: &[&str] = &[
    "editor:add-cursor-above",
    "editor:add-cursor-below",
    "editor:clear-formatting",
    "editor:context-menu",
    "editor:cycle-list-checklist",
    "editor:delete-paragraph",
    "editor:fold-all",
    "editor:fold-less",
    "editor:fold-more",
    "editor:follow-link",
    "editor:indent-list",
    "editor:insert-callout",
    "editor:insert-codeblock",
    "editor:insert-embed",
    "editor:insert-footnote",
    "editor:insert-horizontal-rule",
    "editor:insert-link",
    "editor:insert-mathblock",
    "editor:insert-table",
    "editor:insert-tag",
    "editor:insert-wikilink",
    "editor:open-link-in-new-leaf",
    "editor:open-link-in-new-split",
    "editor:open-link-in-new-window",
    "editor:rename-heading",
    "editor:set-heading",
    "editor:set-heading-0",
    "editor:set-heading-1",
    "editor:set-heading-2",
    "editor:set-heading-3",
    "editor:set-heading-4",
    "editor:set-heading-5",
    "editor:set-heading-6",
    "editor:swap-line-down",
    "editor:swap-line-up",
    "editor:toggle-blockquote",
    "editor:toggle-bold",
    "editor:toggle-bullet-list",
    "editor:toggle-checklist-status",
    "editor:toggle-code",
    "editor:toggle-comments",
    "editor:toggle-fold",
    "editor:toggle-highlight",
    "editor:toggle-highlight-blue",
    "editor:toggle-highlight-green",
    "editor:toggle-highlight-orange",
    "editor:toggle-highlight-purple",
    "editor:toggle-highlight-red",
    "editor:toggle-highlight-yellow",
    "editor:toggle-inline-math",
    "editor:toggle-italics",
    "editor:toggle-numbered-list",
    "editor:toggle-strikethrough",
    "editor:unfold-all",
    "editor:unindent-list",
    "markdown:add-alias",
];

/// The ids that act on the active file, offered only while one is open
/// (Obsidian 1.15.0's empty-tab palette listed none of them).
pub const ACTIVE_FILE_ONLY: &[&str] = &[
    "workspace:copy-path",
    "workspace:copy-full-path",
    "workspace:copy-url",
    "open-with-default-app:open",
    "open-with-default-app:show",
];

/// Command ids the workspace can run; the palette lists only these, as Obsidian hides failed checks.
pub const RUNNABLE: &[&str] = &[
    "editor:save-file",
    "editor:toggle-source",
    "editor:open-search",
    "editor:open-search-replace",
    "editor:add-cursor-above",
    "editor:add-cursor-below",
    "editor:toggle-bold",
    "editor:toggle-italics",
    "editor:toggle-strikethrough",
    "editor:toggle-highlight",
    "editor:toggle-code",
    "editor:toggle-inline-math",
    "editor:toggle-comments",
    "editor:toggle-blockquote",
    "editor:toggle-bullet-list",
    "editor:toggle-numbered-list",
    "editor:toggle-checklist-status",
    "editor:cycle-list-checklist",
    "editor:toggle-highlight-red",
    "editor:toggle-highlight-orange",
    "editor:toggle-highlight-yellow",
    "editor:toggle-highlight-green",
    "editor:toggle-highlight-blue",
    "editor:toggle-highlight-purple",
    "editor:clear-formatting",
    "editor:insert-callout",
    "editor:insert-codeblock",
    "editor:insert-mathblock",
    "editor:insert-horizontal-rule",
    "editor:insert-tag",
    "editor:insert-footnote",
    "editor:set-heading",
    "editor:toggle-fold",
    "editor:fold-all",
    "editor:unfold-all",
    "editor:fold-more",
    "editor:fold-less",
    "editor:set-heading-0",
    "editor:set-heading-1",
    "editor:set-heading-2",
    "editor:set-heading-3",
    "editor:set-heading-4",
    "editor:set-heading-5",
    "editor:set-heading-6",
    "editor:indent-list",
    "editor:unindent-list",
    "editor:delete-paragraph",
    "editor:swap-line-up",
    "editor:swap-line-down",
    "editor:insert-link",
    "editor:insert-wikilink",
    "editor:insert-embed",
    "editor:follow-link",
    "editor:open-link-in-new-leaf",
    "switcher:open",
    "global-search:open",
    "graph:open",
    "graph:open-local",
    "graph:animate",
    "canvas:new-file",
    "canvas:jump-to-group",
    "canvas:convert-to-file",
    "markdown:toggle-preview",
    "markdown:add-metadata-property",
    "markdown:add-alias",
    "markdown:clear-metadata-properties",
    "editor:toggle-fold-properties",
    "editor:context-menu",
    "editor:focus",
    "editor:insert-table",
    "editor:rename-heading",
    "editor:table-row-before",
    "editor:table-row-after",
    "editor:table-row-up",
    "editor:table-row-down",
    "editor:table-row-copy",
    "editor:table-row-delete",
    "editor:table-col-before",
    "editor:table-col-after",
    "editor:table-col-left",
    "editor:table-col-right",
    "editor:table-col-copy",
    "editor:table-col-delete",
    "editor:table-col-align-left",
    "editor:table-col-align-center",
    "editor:table-col-align-right",
    "app:go-back",
    "app:go-forward",
    "app:open-settings",
    "app:switch-vault",
    "app:open-another-vault",
    "app:open-sandbox-vault",
    "app:toggle-left-sidebar",
    "file-explorer:open",
    "file-explorer:reveal-active-file",
    "file-explorer:new-file",
    "file-explorer:new-file-in-current-tab",
    "file-explorer:new-file-in-new-pane",
    "file-explorer:new-folder",
    "file-explorer:move-file",
    "file-explorer:duplicate-file",
    "workspace:edit-file-title",
    "workspace:new-tab",
    "workspace:close",
    "workspace:close-others",
    "workspace:toggle-pin",
    "workspace:next-tab",
    "workspace:previous-tab",
    "workspace:goto-tab-1",
    "workspace:goto-tab-2",
    "workspace:goto-tab-3",
    "workspace:goto-tab-4",
    "workspace:goto-tab-5",
    "workspace:goto-tab-6",
    "workspace:goto-tab-7",
    "workspace:goto-tab-8",
    "workspace:goto-last-tab",
    "workspace:undo-close-pane",
    "workspace:split-vertical",
    "workspace:split-horizontal",
    "workspace:move-to-new-window",
    "workspace:open-in-new-window",
    "workspace:new-window",
    "workspace:close-window",
    "workspace:close-tab-group",
    "workspace:close-others-tab-group",
    "editor:focus-left",
    "editor:focus-right",
    "editor:focus-top",
    "editor:focus-bottom",
    "editor:open-link-in-new-split",
    "editor:open-link-in-new-window",
    "app:delete-file",
    "workspace:copy-path",
    "workspace:copy-full-path",
    "workspace:copy-url",
    "workspace:show-trash",
    "open-with-default-app:open",
    "open-with-default-app:show",
    "theme:toggle-light-dark",
    "editor:attach-file",
    "editor:download-attachments",
    "app:open-file",
    "app:show-debug-info",
    "app:show-release-notes",
    "app:migrate-frontmatter",
    "workspace:export-pdf",
    "canvas:export-as-image",
    "window:zoom-in",
    "window:zoom-out",
    "window:reset-zoom",
    "window:toggle-always-on-top",
    "app:reload",
];

/// Binds every runnable command's hotkeys in the Workspace context and the reading
/// view's scroll keys; call once at startup with `hotkeys.json` applied.
pub fn bind_keys(cx: &mut App, registry: &CommandRegistry) {
    let ctx = Some("Workspace");
    let bindings: Vec<KeyBinding> = RUNNABLE
        .iter()
        .filter_map(|id| registry.get(id))
        .flat_map(|c| {
            c.hotkeys.iter().filter_map(|h| {
                let keys = keystroke(h)?;
                Some(KeyBinding::new(&keys, RunCommand { id: c.id.clone() }, ctx))
            })
        })
        .collect();
    cx.bind_keys(bindings);
    // The view zoom keys are Obsidian's View-menu accelerators (Ctrl+=/-/0),
    // not command hotkeys; bound here so the registry shows them unbound, and
    // a canvas or graph keeps these keys for its own zoom.
    let zoom = |key: &str, id: &str| KeyBinding::new(key, RunCommand { id: id.into() }, ctx);
    cx.bind_keys([
        zoom("secondary-=", "window:zoom-in"),
        zoom("secondary-shift-=", "window:zoom-in"),
        zoom("secondary--", "window:zoom-out"),
        zoom("secondary-0", "window:reset-zoom"),
    ]);
    // The command palette is opened by `Mod+Shift+P` rather than by a
    // registered command's hotkey: Obsidian 1.14.4 no longer registers
    // `command-palette:open`, and the removed-features tests hold TFT to
    // that. It is bound here the way the view zoom keys above are.
    if let Some(keys) = keystroke(&Hotkey {
        modifiers: vec!["Mod".into(), "Shift".into()],
        key: "P".into(),
    }) {
        cx.bind_keys([KeyBinding::new(&keys, OpenCommandPalette, ctx)]);
    }
    // Tab steps into the Files tree from the main area; the tree's own Tab
    // hands the keyboard back (Explorer's binding, which wins over this one).
    // The editor, text fields and an open menu keep Tab for themselves.
    let into_tree = Some("Workspace && !Editor && !TextInput && !Explorer && !Menu");
    cx.bind_keys([
        KeyBinding::new("tab", FocusExplorer, into_tree),
        KeyBinding::new("shift-tab", FocusExplorer, into_tree),
    ]);
    // The Tags, All properties and Outline trees: arrows walk and fold,
    // Enter and Space act like a click. Home, End, Escape, F2 and Delete are not theirs.
    let tree = Some("DockTree");
    cx.bind_keys([
        KeyBinding::new("up", TreeUp, tree),
        KeyBinding::new("down", TreeDown, tree),
        KeyBinding::new("left", TreeLeft, tree),
        KeyBinding::new("right", TreeRight, tree),
        KeyBinding::new("enter", TreeActivate, tree),
        KeyBinding::new("space", TreeActivate, tree),
    ]);
    crate::reading::view::bind_keys(cx);
    download_modal::bind_keys(cx);
}

impl Workspace {
    /// The command's name as the palette shows it, for a message about it.
    fn command_name(&self, id: &str, cx: &App) -> String {
        cx.global::<Commands>()
            .0
            .get(id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    pub(super) fn run_command_action(
        &mut self,
        action: &RunCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.id.clone();
        self.run_command(&id, window, cx);
    }

    /// Runs a command by its Obsidian id; unknown or not-yet-implemented
    /// ids are reported in the status bar.
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if RUNNABLE.contains(&id) && !self.command_state.running_from_slash {
            self.command_state.recent.retain(|r| r != id);
            self.command_state.recent.insert(0, id.to_string());
            self.command_state.recent.truncate(20);
        }
        match id {
            "editor:save-file" => self.flush_save(cx),
            "editor:open-search" => self.open_find("", false, window, cx),
            "editor:open-search-replace" => self.open_find("", true, window, cx),
            "editor:add-cursor-above" | "editor:add-cursor-below" => {
                if self.editing_or_report(id, window, cx) {
                    let above = id == "editor:add-cursor-above";
                    self.active_editor()
                        .update(cx, |e, cx| e.add_cursor(above, window, cx));
                }
            }
            "editor:toggle-source" => {
                let mode = match self.active_group_state.mode {
                    ViewMode::LivePreview => ViewMode::Source,
                    _ => ViewMode::LivePreview,
                };
                self.set_mode(mode, window, cx);
            }
            "switcher:open" => self.open_switcher("", window, cx),
            "global-search:open" => self.open_search_action(&OpenSearch, window, cx),
            "graph:open" => self.open_graph_action(&OpenGraph, window, cx),
            // Asked for by id with no file open -- the palette does not offer
            // it, and `can_run` says so -- this opens nothing, which is what
            // Obsidian's `checkCallback` does rather than falling back to the
            // global graph.
            "graph:open-local" if self.active_group_state.current.is_none() => {}
            "graph:open-local" => self.show_graph(true, window, cx),
            // Both check callbacks: asked for by id where the palette would
            // not offer them, they do nothing.
            "graph:animate" if !self.global_graph_active(cx) => {}
            "graph:animate" => self.graph.update(cx, |g, cx| g.animate(cx)),
            "canvas:new-file" => self.new_canvas("", cx),
            "canvas:jump-to-group" => self.jump_to_group(window, cx),
            "canvas:convert-to-file" => self.convert_card_to_file(cx),
            "markdown:toggle-preview" => self.toggle_mode(&ToggleMode, window, cx),
            // The properties block is the same editable widget in reading
            // view, and `editor:toggle-fold-properties` was observed running
            // there with the preview leaf active (Specification.md, "Folding"),
            // so these three ask only that the note hold the keyboard.
            //
            // Obsidian opens an unnamed property row and writes nothing until
            // it is named, which is what `add_property_row` does.
            "markdown:add-metadata-property" => {
                if self.note_has_keys(window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.add_property_row(window, cx));
                }
            }
            "markdown:add-alias" => self.add_alias(window, cx),
            "editor:rename-heading" => {
                if self.editing_or_report(id, window, cx) {
                    self.start_rename_heading(window, cx);
                }
            }
            "markdown:clear-metadata-properties" => {
                if self.note_has_keys(window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.clear_properties(window, cx));
                }
            }
            "theme:toggle-light-dark" => self.toggle_light_dark(cx),
            // "Show context menu under cursor": a right click's menu, at the
            // caret. In the reading view Obsidian 1.15.0 neither offered it
            // nor opened anything, so a hotkey there is silent too.
            "editor:context-menu" => {
                if self.active_group_state.mode.is_editing() && self.note_has_keys(window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.context_menu_at_caret(cx));
                }
            }
            // "Focus on last note": the keyboard back on the note's editor
            // the main area shows, wherever it was -- a dock tree, the
            // search field. It never switches tabs: over a canvas, an empty
            // tab or a note in the reading view Obsidian 1.15.0 did nothing
            // visible, so only an editor on screen takes it.
            "editor:focus" => {
                if self.main_view(cx) == MainView::Editor {
                    self.focus_main(window, cx);
                }
            }
            "editor:toggle-fold-properties" => {
                if self.note_has_keys(window, cx) {
                    if self.active_group_state.mode.is_editing() {
                        self.active_editor()
                            .update(cx, |e, cx| e.toggle_properties_fold(cx));
                    } else {
                        self.active_reading()
                            .update(cx, |r, cx| r.toggle_properties_fold(cx));
                    }
                }
            }
            // The five fold commands pass their check only with a markdown
            // view focused in an editing mode, as observed; none has a
            // default hotkey.
            "editor:toggle-fold" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor().update(cx, |e, cx| e.toggle_fold(cx));
                }
            }
            "editor:fold-all" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor().update(cx, |e, cx| e.fold_all(cx));
                }
            }
            "editor:unfold-all" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor().update(cx, |e, cx| e.unfold_all(cx));
                }
            }
            "editor:fold-more" | "editor:fold-less" => {
                if self.editing_or_report(id, window, cx) {
                    let more = id == "editor:fold-more";
                    self.active_editor()
                        .update(cx, |e, cx| e.fold_step(more, cx));
                }
            }
            "editor:insert-table" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.insert_table(window, cx));
                }
            }
            // The wrapping toggles write the pair of markers Obsidian's markup uses around the selection.
            "editor:toggle-bold"
            | "editor:toggle-italics"
            | "editor:toggle-strikethrough"
            | "editor:toggle-highlight"
            | "editor:toggle-code"
            | "editor:toggle-inline-math"
            | "editor:toggle-comments" => {
                if self.editing_or_report(id, window, cx) {
                    let marker = match id {
                        "editor:toggle-bold" => "**",
                        "editor:toggle-italics" => "*",
                        "editor:toggle-strikethrough" => "~~",
                        "editor:toggle-highlight" => "==",
                        "editor:toggle-code" => "`",
                        "editor:toggle-inline-math" => "$",
                        _ => "%%",
                    };
                    self.active_editor()
                        .update(cx, |e, cx| e.toggle_wrap(marker, window, cx));
                }
            }
            "editor:toggle-bullet-list"
            | "editor:toggle-numbered-list"
            | "editor:toggle-blockquote" => {
                if self.editing_or_report(id, window, cx) {
                    let kind = match id {
                        "editor:toggle-bullet-list" => LinePrefix::Bullet,
                        "editor:toggle-numbered-list" => LinePrefix::Ordered,
                        _ => LinePrefix::Quote,
                    };
                    self.active_editor()
                        .update(cx, |e, cx| e.toggle_line_prefix(kind, window, cx));
                }
            }
            "editor:toggle-checklist-status" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.toggle_checklist(window, cx));
                }
            }
            "editor:cycle-list-checklist" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.cycle_checklist(window, cx));
                }
            }
            // The six colours write one emoji after the opening `==`.
            "editor:toggle-highlight-red"
            | "editor:toggle-highlight-orange"
            | "editor:toggle-highlight-yellow"
            | "editor:toggle-highlight-green"
            | "editor:toggle-highlight-blue"
            | "editor:toggle-highlight-purple" => {
                if self.editing_or_report(id, window, cx) {
                    let color = id
                        .rsplit('-')
                        .next()
                        .and_then(tft_markdown::HighlightColor::from_name);
                    self.active_editor()
                        .update(cx, |e, cx| e.toggle_highlight(color, window, cx));
                }
            }
            "editor:clear-formatting" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.clear_formatting(window, cx));
                }
            }
            "editor:insert-callout"
            | "editor:insert-codeblock"
            | "editor:insert-mathblock"
            | "editor:insert-horizontal-rule"
            | "editor:insert-tag"
            | "editor:insert-footnote" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(kind) = BlockInsert::from_id(id) {
                        self.active_editor()
                            .update(cx, |e, cx| e.insert_block(kind, window, cx));
                    }
                }
            }
            // "Toggle heading" is a real palette entry whose check passes and changes
            // nothing (Specification.md, "Observed editing command behavior"); doing
            // nothing is parity, not a bug.
            "editor:set-heading" => {
                self.editing_or_report(id, window, cx);
            }
            "editor:set-heading-0"
            | "editor:set-heading-1"
            | "editor:set-heading-2"
            | "editor:set-heading-3"
            | "editor:set-heading-4"
            | "editor:set-heading-5"
            | "editor:set-heading-6" => {
                if self.editing_or_report(id, window, cx) {
                    let level = id
                        .rsplit('-')
                        .next()
                        .and_then(|n| n.parse::<u8>().ok())
                        .unwrap_or(0);
                    self.active_editor()
                        .update(cx, |e, cx| e.set_heading(level, window, cx));
                }
            }
            "editor:indent-list" | "editor:unindent-list" => {
                if self.editing_or_report(id, window, cx) {
                    let indent = id == "editor:indent-list";
                    self.active_editor()
                        .update(cx, |e, cx| e.indent_list(indent, window, cx));
                }
            }
            "editor:delete-paragraph" => {
                if self.editing_or_report(id, window, cx) {
                    self.active_editor()
                        .update(cx, |e, cx| e.delete_paragraph(window, cx));
                }
            }
            "editor:swap-line-up" | "editor:swap-line-down" => {
                if self.editing_or_report(id, window, cx) {
                    let up = id == "editor:swap-line-up";
                    self.active_editor()
                        .update(cx, |e, cx| e.swap_lines(up, window, cx));
                }
            }
            "editor:insert-link" | "editor:insert-wikilink" | "editor:insert-embed" => {
                if self.editing_or_report(id, window, cx) {
                    let kind = match id {
                        "editor:insert-link" => LinkSyntax::Markdown,
                        "editor:insert-wikilink" => LinkSyntax::Wikilink,
                        _ => LinkSyntax::Embed,
                    };
                    self.active_editor()
                        .update(cx, |e, cx| e.insert_link(kind, window, cx));
                }
            }
            "editor:follow-link" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.active_editor().read(cx).link_target_at_cursor() {
                        self.follow_link(target, false, cx);
                    }
                }
            }
            "editor:open-link-in-new-leaf" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.active_editor().read(cx).link_target_at_cursor() {
                        self.follow_link(target, true, cx);
                    }
                }
            }
            "editor:attach-file" => self.attach_file(window, cx),
            "editor:download-attachments" => self.start_download_attachments(window, cx),
            "app:open-file" => self.open_file_dialog(window, cx),
            _ if TableCommand::from_id(id).is_some() => {
                let cmd = TableCommand::from_id(id).expect("checked above");
                if self.editing_or_report(id, window, cx) {
                    let ran = self
                        .active_editor()
                        .update(cx, |e, cx| e.table_command(cmd, window, cx));
                    if !ran {
                        self.report("The cursor is not in a table", cx);
                    }
                }
            }
            "app:go-back" => self.go_back(&GoBack, window, cx),
            "app:go-forward" => self.go_forward(&GoForward, window, cx),
            "app:open-settings" => self.open_settings_menu_from_button(window, cx),
            "app:toggle-left-sidebar" => {
                self.left_dock.visible = !self.left_dock.visible;
                if !self.left_dock.visible {
                    self.leave_search_field(window, cx);
                }
                cx.notify();
            }
            "file-explorer:open" => {
                self.leave_search_field(window, cx);
                self.left_dock.visible = true;
                self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
                cx.notify();
            }
            "file-explorer:reveal-active-file" => match self.active_group_state.current.clone() {
                Some(current) => self.reveal_in_navigation(&current, window, cx),
                None => {
                    self.leave_search_field(window, cx);
                    self.left_dock.visible = true;
                    self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
                    cx.notify();
                }
            },
            "file-explorer:new-file"
            | "file-explorer:new-file-in-current-tab"
            | "file-explorer:new-file-in-new-pane" => {
                // Obsidian's "Create new note" goes to a new tab unless the active one
                // is empty; "to the right" is a new tab in TFT's one strip.
                let fresh_tab = match id {
                    "file-explorer:new-file-in-current-tab" => false,
                    "file-explorer:new-file-in-new-pane" => true,
                    _ => self.active_group_state.current.is_some(),
                };
                // New notes go to the vault folder.
                let folder = String::new();
                if fresh_tab {
                    self.new_tab(cx);
                }
                self.new_note(&folder, window, cx);
            }
            "file-explorer:new-folder" => self.start_new_folder("", window, cx),
            "file-explorer:move-file" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.start_move(&current, window, cx);
                }
            }
            "file-explorer:duplicate-file" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.duplicate(&current, cx);
                }
            }
            "workspace:edit-file-title" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.start_rename(&current, window, cx);
                }
            }
            "workspace:new-tab" => self.new_tab(cx),
            // A pinned tab is unpinned by the close, not closed (observed).
            "workspace:close" if self.active_group_state.active_tab_pinned() => self.toggle_pin(cx),
            "workspace:close" => self.close_tab(self.active_group_state.active_tab, cx),
            "workspace:close-others" => self.close_other_tabs(cx),
            "workspace:toggle-pin" => self.toggle_pin(cx),
            "workspace:move-to-new-window" => self.move_to_new_window(window, cx),
            "workspace:open-in-new-window" => self.open_in_new_window(window, cx),
            "workspace:new-window" => {
                self.new_window(window, cx);
            }
            "workspace:close-window" => self.close_window(window, cx),
            "workspace:split-vertical" => self.split(SplitDirection::Vertical, window, cx),
            "workspace:split-horizontal" => self.split(SplitDirection::Horizontal, window, cx),
            "workspace:close-tab-group" => {
                let id = self.active_group_id();
                self.close_group(id, true, cx);
            }
            // Obsidian's "Close others in tab group" closes the group's other tabs, as `close-others` does.
            "workspace:close-others-tab-group" => self.close_other_tabs(cx),
            "editor:focus-left" => self.focus_neighbor(SplitDirection::Vertical, false, cx),
            "editor:focus-right" => self.focus_neighbor(SplitDirection::Vertical, true, cx),
            "editor:focus-top" => self.focus_neighbor(SplitDirection::Horizontal, false, cx),
            "editor:focus-bottom" => self.focus_neighbor(SplitDirection::Horizontal, true, cx),
            // "Open link under cursor to the right": split right, then follow the link there.
            "editor:open-link-in-new-split" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.active_editor().read(cx).link_target_at_cursor() {
                        self.split(SplitDirection::Vertical, window, cx);
                        self.follow_link(target, false, cx);
                    }
                }
            }
            "editor:open-link-in-new-window" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.active_editor().read(cx).link_target_at_cursor() {
                        self.open_link_in_new_window(target, window, cx);
                    }
                }
            }
            "workspace:next-tab" => self.cycle_tab(1, cx),
            "workspace:previous-tab" => {
                self.cycle_tab(self.active_group_state.tabs.len().saturating_sub(1), cx)
            }
            "workspace:goto-last-tab" => {
                self.activate_tab(self.active_group_state.tabs.len().saturating_sub(1), cx)
            }
            "workspace:undo-close-pane" => self.undo_close_tab(cx),
            _ if id.starts_with("workspace:goto-tab-") => {
                let n: usize = id
                    .rsplit('-')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                if n >= 1 && n <= self.active_group_state.tabs.len() {
                    self.activate_tab(n - 1, cx);
                }
            }
            "app:delete-file" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.start_delete(&current, window, cx);
                }
            }
            // The file menu's rows for the active file: Copy path, Copy full path,
            // Open in default app, Show in explorer. The menu is silent, but each
            // command reports what it copied.
            "workspace:copy-path" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.copy_path(&current, cx);
                    self.report("Copied to your clipboard", cx);
                }
            }
            "workspace:copy-full-path" => {
                if let (Some(state), Some(current)) = (
                    self.vault_session.state.clone(),
                    self.active_group_state.current.clone(),
                ) {
                    let text = full_path_text(state.vault().absolute(&current));
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.report("Copied to your clipboard", cx);
                }
            }
            "workspace:copy-url" => {
                if let (Some(state), Some(current)) = (
                    self.vault_session.state.clone(),
                    self.active_group_state.current.clone(),
                ) {
                    let name = vault_folder_name(&state.root);
                    let url = crate::uri::open_url(&name, &current);
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    self.report("URL copied to your clipboard", cx);
                }
            }
            // "Show trash", offered only under `trashOption: "local"`: the `.trash`
            // folder, or Obsidian's notice when it is missing.
            "workspace:show-trash" => {
                if let Some(state) = self.vault_session.state.clone() {
                    if state.trash_option() == TrashOption::Local {
                        let trash = state.vault().absolute(TRASH_FOLDER);
                        if trash.exists() {
                            self.show_in_system_explorer(TRASH_FOLDER, cx);
                        } else {
                            self.report(missing_file_notice(&full_path_text(trash)), cx);
                        }
                    }
                }
            }
            "open-with-default-app:open" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.open_in_default_app(&current, cx);
                }
            }
            "open-with-default-app:show" => {
                if let Some(current) = self.active_group_state.current.clone() {
                    self.show_in_system_explorer(&current, cx);
                }
            }
            "app:switch-vault" => self.toggle_vault_menu(cx),
            "app:open-another-vault" => self.open_another_vault(window, cx),
            "app:open-sandbox-vault" => self.open_sandbox_vault(window, cx),
            "app:show-debug-info" => self.show_debug_info(window, cx),
            "app:migrate-frontmatter" => self.open_migrate_frontmatter(window, cx),
            "workspace:export-pdf" | "canvas:export-as-image" => {
                self.run_export_command(id, window, cx);
            }
            "app:show-release-notes" => self.show_release_notes(cx),
            // Zoom in/out/reset: half a level between -2.5 and +3, on the vault
            // window (see [[Specification#Window zoom]]).
            "app:reload" => self.reload_app(cx),
            "window:zoom-in" => self.zoom_main(zoom::stepped(self.main_window.zoom_level(), 1), cx),
            "window:zoom-out" => {
                self.zoom_main(zoom::stepped(self.main_window.zoom_level(), -1), cx)
            }
            "window:reset-zoom" => self.zoom_main(0.0, cx),
            // A popout's command only (`can_run`): that window above others, a
            // second run back; nothing is written.
            "window:toggle-always-on-top" => {
                if self.can_run(id, window, cx) {
                    let on = !zoom::always_on_top(window);
                    if !zoom::set_always_on_top(window, on) {
                        self.report(
                            "This desktop does not let a window keep itself on top".to_string(),
                            cx,
                        );
                    }
                }
            }
            _ => {
                let name = self.command_name(id, cx);
                self.report(format!("{name}: not available in TFT yet"), cx);
            }
        }
    }

    /// What a slash popup lists: the commands as they stand, ordered by this session's recent ones.
    pub(super) fn slash_commands(&self, window: &Window, cx: &App) -> crate::editor::SlashCommands {
        let registry = cx.global::<Commands>().0.clone();
        let entries = registry
            .iter()
            .filter(|c| self.can_run(&c.id, window, cx))
            .map(|c| tft_suggest::CommandEntry {
                id: c.id.clone(),
                name: c.name.clone(),
                hotkeys: c.hotkeys.iter().map(hotkey_label).collect(),
            })
            .collect();
        crate::editor::SlashCommands {
            entries,
            pinned: Vec::new(),
            recent: self.command_state.recent.clone(),
        }
    }

    /// Runs the command a slash popup row named once its `/query` is out; it is not
    /// recorded as recent, and the removal plus the command's edits are one undo step.
    pub(super) fn run_slash_command(
        &mut self,
        editor: &Entity<Editor>,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_state.running_from_slash = true;
        self.run_command(id, window, cx);
        self.command_state.running_from_slash = false;
        editor.update(cx, |e, _| e.finish_slash_command());
    }

    /// Whether a command can run now, which is what the palette lists and the socket
    /// reports; Obsidian hides commands whose check callback fails.
    pub fn can_run(&self, id: &str, window: &Window, cx: &App) -> bool {
        if !RUNNABLE.contains(&id) {
            return false;
        }
        // A command whose core plugin is off is gone from the registry; this only
        // matters for a direct id.
        if let Some(plugin) = tft_commands::plugin_of(id) {
            if !self
                .vault_session
                .state
                .as_ref()
                .is_none_or(|state| state.core_plugin_enabled(plugin))
            {
                return false;
            }
        }
        // Obsidian's table commands are check callbacks: hidden outside Live Preview
        // and when the command has nothing to do.
        if let Some(cmd) = TableCommand::from_id(id) {
            return self.active_group_state.mode.is_editing()
                && self.active_editor().read(cx).table_command_applies(cmd);
        }
        // `graph:open-local` is a checkCallback not listed with no file open; only
        // that case was observed (see [[Specification#`graph:open-local` is not offered with no file open]]).
        if id == "graph:open-local" {
            return self.active_group_state.current.is_some();
        }
        // Offered only while a global graph is the active leaf, not with a note active
        // or a local graph.
        if id == "graph:animate" {
            return self.global_graph_active(cx);
        }
        // "Toggle window always on top" is offered only in a popout that is not maximized.
        if id == "window:toggle-always-on-top" {
            return window.window_handle().downcast::<PopoutWindow>().is_some()
                && !window.is_maximized();
        }
        // The active file's commands are check callbacks: offered only with a note or image open.
        if ACTIVE_FILE_ONLY.contains(&id) {
            return self.active_group_state.current.is_some();
        }
        // "Show trash" only where there is a trash of the vault's own.
        if id == "workspace:show-trash" {
            return self
                .vault_session
                .state
                .as_ref()
                .is_some_and(|state| state.trash_option() == TrashOption::Local);
        }
        // "Focus on last note" whenever a Markdown leaf exists, in any mode or tab.
        if id == "editor:focus" {
            return self.has_markdown_leaf();
        }
        // Both are check callbacks: "Rename this heading..." needs the caret on an ATX
        // heading while editing; "Add alias" is left out of reading view, whose block is
        // read-only in TFT.
        if id == "editor:rename-heading" {
            return self.active_group_state.mode.is_editing()
                && self.note_has_keys(window, cx)
                && self.active_editor().read(cx).heading_to_rename().is_some();
        }
        if let Some(can) = self.attachments_can_run(id) {
            return can;
        }
        if let Some(can) = self.exports_can_run(id, cx) {
            return can;
        }
        if EDITOR_ONLY.contains(&id) {
            return self.active_group_state.mode.is_editing() && self.note_has_keys(window, cx);
        }
        true
    }

    /// Whether any tab of any group, popouts included, is a Markdown leaf.
    fn has_markdown_leaf(&self) -> bool {
        let live = std::iter::once((
            self.active_group_state.current.as_deref(),
            self.active_group_state.mode == ViewMode::Graph,
        ));
        let mine = self
            .active_group_state
            .tabs
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix != self.active_group_state.active_tab)
            .map(|(_, t)| (t.path.as_deref(), t.mode == ViewMode::Graph));
        let others = self
            .groups
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix != self.active_group)
            .flat_map(|(_, g)| g.state.tabs.iter())
            .map(|t| (t.path.as_deref(), t.mode == ViewMode::Graph));
        any_markdown_leaf(live.chain(mine).chain(others))
    }

    /// True when the note holds the keyboard: no modal and focus on its pane. An editor
    /// hotkey otherwise fires wherever the keyboard is and writes into a note behind it.
    pub(super) fn note_has_keys(&self, window: &Window, cx: &App) -> bool {
        self.overlay.modal == Modal::None
            && (self
                .active_editor()
                .read(cx)
                .focus_handle_ref()
                .is_focused(window)
                || self.active_reading().focus_handle(cx).is_focused(window))
    }

    /// True when an editor command can run. Obsidian hides a failed check callback; TFT
    /// says why it did nothing instead, but only when the note had the keys -- a command
    /// arriving in a dock field or modal is addressed there.
    fn editing_or_report(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.note_has_keys(window, cx) {
            return false;
        }
        if self.active_group_state.mode.is_editing() {
            return true;
        }
        let name = self.command_name(id, cx);
        self.report(format!("{name}: only available while editing"), cx);
        false
    }

    pub(super) fn open_search_action(
        &mut self,
        _: &OpenSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.search.read(cx).query(cx);
        self.show_search(&query, Some(window), cx);
    }

    /// Whether the open file is a Markdown note.
    pub(super) fn active_note_is_markdown(&self) -> bool {
        self.active_group_state
            .current
            .as_deref()
            .is_some_and(|path| FileKind::from_path(path) == FileKind::Markdown)
    }

    /// `graph:animate`'s check: a global graph is the active leaf.
    fn global_graph_active(&self, cx: &App) -> bool {
        self.main_view(cx) == MainView::Graph
            && !self.radial_tree.active
            && !self.graph.read(cx).is_local()
    }

    pub(super) fn open_graph_action(
        &mut self,
        _: &OpenGraph,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // "Open graph view" toggles the *global* graph. Asked for it with a
        // local graph on screen, Obsidian shows the global one rather than
        // leaving graph mode: `graph:open-local` is not a toggle either, so
        // treating any graph as the one to close made the pair inconsistent.
        if self.active_group_state.mode == ViewMode::Graph && !self.graph.read(cx).is_local() {
            let back = self.active_group_state.mode_before_graph;
            self.set_mode(back, window, cx);
        } else {
            self.show_graph(false, window, cx);
        }
    }
}

/// What `workspace:copy-full-path` ("Copy current file path from system
/// root") puts on the clipboard: the file's path from the root of the file
/// system, in the platform's own form. The vault root is kept as it was
/// opened, so a relative one is made absolute against the working folder
/// here; symbolic links are not resolved, since the path the vault was
/// opened by is the one its owner knows it by.
pub(super) fn full_path_text(absolute: PathBuf) -> String {
    std::path::absolute(&absolute)
        .unwrap_or(absolute)
        .to_string_lossy()
        .into_owned()
}

/// The vault's own trash folder, at the vault root, where
/// `trashOption: "local"` moves a deleted file.
pub(super) const TRASH_FOLDER: &str = ".trash";

/// The vault's name as `workspace:copy-url` writes it: its folder's name,
/// read from the absolute root so a vault opened as `.` (or as the empty
/// path, which is the working folder too) is still named. The last plain
/// component is the name: `absolute` keeps a trailing `.`.
pub(super) fn vault_folder_name(root: &Path) -> String {
    let absolute = match root.as_os_str().is_empty() {
        true => std::env::current_dir(),
        false => std::path::absolute(root),
    }
    .unwrap_or_else(|_| root.to_path_buf());
    absolute
        .components()
        .rev()
        .find_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Obsidian's notice for a path that is not there, as "Show trash" showed
/// it with no `.trash` folder: the absolute path in curly quotes.
pub(super) fn missing_file_notice(absolute: &str) -> String {
    format!("The file or folder \u{201c}{absolute}\u{201d} does not exist.")
}

/// Whether any of `leaves` is a Markdown leaf, which is what
/// `editor:focus` ("Focus on last note") is offered on: each is a tab's file
/// and whether the tab shows the graph instead. A canvas, a base, a PDF,
/// an image or an empty tab is not one (Obsidian 1.15.0: with an empty tab
/// and a canvas only, the palette did not list it).
pub(super) fn any_markdown_leaf<'a>(
    mut leaves: impl Iterator<Item = (Option<&'a str>, bool)>,
) -> bool {
    leaves.any(|(path, graph)| {
        !graph && path.is_some_and(|p| FileKind::from_path(p) == FileKind::Markdown)
    })
}
