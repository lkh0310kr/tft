//! The Files view. `FileTree` is the pure model; `FileExplorer` draws one
//! element per visible row and reports actions as [`ExplorerEvent`]. Rows
//! rename in place; F2/Delete and the file menu act on the focused row.

use crate::assets::icon;
use crate::file_menu::{self, Menu, Pick, Target};
use crate::inline_field;
use crate::theme::{Theme, HEADER_HEIGHT, ICON_SIZE};
use gpui::{
    actions, canvas, div, prelude::*, px, uniform_list, App, Bounds, Context, DragMoveEvent,
    ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle, Focusable, FontWeight,
    KeyBinding, MouseButton, MouseDownEvent, Pixels, Point, ScrollStrategy, SharedString,
    UTF16Selection, UniformListScrollHandle, Window,
};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

pub use tft_app::explorer::{
    display_name, drop_folder, drop_moves, kind_name, move_into_label, FileTree, SortOrder,
};

actions!(
    explorer,
    [
        FocusUp,
        FocusDown,
        FocusLeft,
        FocusRight,
        Activate,
        ToggleFolder,
        LeaveTree,
        RenameFocused,
        DeleteFocused
    ]
);

/// Key bindings for the tree; called once at startup. The context is the
/// explorer's own, beneath the workspace's.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Explorer");
    cx.bind_keys([
        KeyBinding::new("up", FocusUp, ctx),
        KeyBinding::new("down", FocusDown, ctx),
        KeyBinding::new("left", FocusLeft, ctx),
        KeyBinding::new("right", FocusRight, ctx),
        KeyBinding::new("enter", Activate, ctx),
        KeyBinding::new("space", ToggleFolder, ctx),
        KeyBinding::new("escape", LeaveTree, ctx),
        // Tab leaves the tree for the main area; the workspace's Tab binding
        // is what enters the tree from there. Shift+Tab is the same trip.
        KeyBinding::new("tab", LeaveTree, ctx),
        KeyBinding::new("shift-tab", LeaveTree, ctx),
        KeyBinding::new("f2", RenameFocused, ctx),
        KeyBinding::new("delete", DeleteFocused, ctx),
    ]);
}

#[derive(Clone)]
pub struct FileDrag {
    pub path: String,
    pub folder: bool,
    pub name: SharedString,
    pub action: Rc<RefCell<Option<SharedString>>>,
}

impl FileDrag {
    pub fn set_action(&self, action: Option<SharedString>) {
        *self.action.borrow_mut() = action;
    }
}

/// The ghost under the pointer: Obsidian's `drag-ghost` (2026-11-07
/// `explorer-drag-look`): 85 % black, `6px 8px`, radius 4, a shadow, at most
/// 300 px; the icon (`lucide-file`, `lucide-folder-open` for a folder) at 0.7
/// and the name at 13 px weight 500, then the action at 12 px and 0.7.
///
/// gpui draws a drag's view where the row was grabbed, so the view pads
/// itself by the grab point to sit 5 px below and right of the pointer,
/// where Obsidian's sits (`s03`: the ghost at the pointer plus 5).
pub struct FileGhost(pub FileDrag, pub Point<Pixels>);

impl Render for FileGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let action = self.0.action.borrow().clone();
        let white = gpui::white();
        let mut faded = white;
        faded.a = 0.7;
        let grab = self.1;
        div().pl(grab.x + px(5.)).pt(grab.y + px(5.)).child(
            div()
                .max_w(px(300.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(4.))
                .bg(gpui::hsla(0., 0., 0., 0.85))
                .shadow_md()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(div().mr(px(4.)).child(icon(
                            if self.0.folder { "folder-open" } else { "file" },
                            ICON_SIZE,
                            faded,
                        )))
                        .child(self.0.name.clone()),
                )
                .children(action.map(|a| {
                    div()
                        .pt(px(2.))
                        .text_size(px(12.))
                        .text_color(faded)
                        .child(a)
                })),
        )
    }
}

/// The folder a drop over a row lands in: a folder row is its own target,
/// a file row its folder's (`""` at the root), which is what Obsidian
/// washed when a file was held over a file row (`s02`, `s03`).
pub enum ExplorerEvent {
    SortChanged(SortOrder),
    FoldersExpanded(Vec<String>),
    Open(String),
    /// The pointer came onto a file's row (`over`) or left it: the row's
    /// bounds anchor the page preview.
    Hover {
        path: String,
        over: bool,
        anchor: Bounds<Pixels>,
        modifiers: gpui::Modifiers,
    },
    /// A row dropped in a folder (`""` for the vault root).
    MoveInto {
        path: String,
        folder: String,
    },
    /// Enter in a row being renamed in place: the name typed into the
    /// label, without the extension the row never showed. The workspace
    /// validates it and renames; the field stays open until it says the
    /// rename went through, which is what Obsidian does with a name the
    /// filesystem refuses (the 2026-10-25 pass typed a colliding name and
    /// an empty one and the editor stayed up with neither a notice nor a
    /// modal).
    RenameTo {
        path: String,
        name: String,
    },
    /// A Ctrl+click: the file goes to a new tab.
    OpenInNewTab(String),
    /// Context-menu actions; paths are vault-relative, folders without a
    /// trailing slash (`""` is the vault root).
    NewNote {
        folder: String,
    },
    NewFolder {
        parent: String,
    },
    /// "New canvas" from a folder row, which takes the folder the click was
    /// on rather than the vault root the command uses.
    NewCanvas {
        folder: String,
    },
    Rename(String),
    Move(String),
    Duplicate(String),
    Delete(String),
    /// "Open to the right": a new tab group beside the active one showing
    /// this file.
    OpenToRight(String),
    /// "Open in new window": this file in a popout window of its own.
    OpenInNewWindow(String),
    /// "Copy path": the vault-relative path onto the clipboard.
    CopyPath(String),
    /// "Copy Path": the absolute filesystem path onto the clipboard.
    CopyAbsolutePath(String),
    /// "Open in default app": hand the file to the desktop.
    OpenInDefaultApp(String),
    /// "Show in system explorer": the file or folder selected in the
    /// desktop's file manager.
    ShowInSystemExplorer(String),
    /// "Search in folder": the Search pane scoped to this folder.
    SearchInFolder(String),
    /// Escape in the tree: the keyboard goes back to the main area.
    LeaveFocus,
}

/// The [`ExplorerEvent`] a menu row's pick stands for, on the row the menu
/// was opened on. `None` for a pick the explorer's menus never offer
/// ([`Pick::RevealInNavigation`] is the Backlinks pane's).
fn event_for(pick: Pick, target: &Target) -> Option<ExplorerEvent> {
    use ExplorerEvent as E;
    let path = || target.path().to_string();
    // Creation and folder search act on the folder the row stands for; a
    // file row stands for the folder that holds it, which is what a context
    // menu's "New note" means on a file.
    let folder = || match target {
        Target::File(p) => p
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default(),
        _ => path(),
    };
    Some(match pick {
        Pick::OpenInNewTab => E::OpenInNewTab(path()),
        Pick::OpenToRight => E::OpenToRight(path()),
        Pick::OpenInNewWindow => E::OpenInNewWindow(path()),
        Pick::Duplicate => E::Duplicate(path()),
        Pick::Move => E::Move(path()),
        Pick::CopyPath => E::CopyPath(path()),
        Pick::CopyAbsolutePath => E::CopyAbsolutePath(path()),
        Pick::OpenInDefaultApp => E::OpenInDefaultApp(path()),
        Pick::ShowInSystemExplorer => E::ShowInSystemExplorer(path()),
        Pick::Rename => E::Rename(path()),
        Pick::Delete => E::Delete(path()),
        Pick::NewNote => E::NewNote { folder: folder() },
        Pick::NewFolder => E::NewFolder { parent: folder() },
        Pick::NewCanvas => E::NewCanvas { folder: folder() },
        Pick::SearchInFolder => E::SearchInFolder(folder()),
        // "Reveal file in navigation" reveals *in* the explorer, the tab
        // rows are the tab header's, the splits are the view header's, and the rest are the editor menu's rows,
        // which act on the editor.
        Pick::RevealInNavigation
        | Pick::CloseTab
        | Pick::CloseOthers
        | Pick::CloseAfter
        | Pick::CloseAll
        | Pick::AddFileProperty
        | Pick::Find
        | Pick::Replace
        | Pick::TogglePin
        | Pick::MoveToNewWindow
        | Pick::SplitRight
        | Pick::SplitDown
        | Pick::AddLink
        | Pick::AddExternalLink
        | Pick::SearchForSelection
        | Pick::Cut
        | Pick::Copy
        | Pick::Paste
        | Pick::PastePlain
        | Pick::SelectAll => return None,
    })
}

pub struct FileExplorer {
    tree: FileTree,
    active: Option<String>,
    scroll: UniformListScrollHandle,
    menu: Option<Menu>,
    sort_menu_open: bool,
    /// The keyboard the open menu holds, and where it goes back to.
    menu_focus: file_menu::MenuFocus,
    focus_handle: FocusHandle,
    /// The row the keyboard is on (Obsidian's `has-focus`), by path so it
    /// survives a re-sort; it may be hidden under a collapsed folder, and
    /// then the next key starts over from the open file.
    focused: Option<String>,
    /// The row being renamed in place, if any (Obsidian's
    /// `is-being-renamed`).
    renaming: Option<RowEdit>,
    /// The keyboard an open field holds, and where it goes back to: the
    /// same "focus while open, restore after" helper a menu uses.
    rename_focus: file_menu::MenuFocus,
    /// The vault's name, which the ghost says for a drop at the root.
    vault_name: String,
    /// While a row is dragged: the row, and the folder the pointer is over
    /// (`""` for the root), with where the pointer was when a row claimed
    /// it, so the list's own handler can tell a row's move from its own.
    dragging: Option<String>,
    drop: Option<String>,
    drop_claim: Option<Point<Pixels>>,
}

/// A row whose label is being typed into.
struct RowEdit {
    /// The file or folder the field opened on. The edit is dropped if that
    /// row leaves the tree, so a commit can only ever rename what the row
    /// showed.
    path: String,
    /// The text and the input method's composition in it.
    editing: inline_field::Editing,
    /// Where the label was last painted, which is what a click inside it
    /// is measured against. `None` until the first paint.
    layout: Option<inline_field::Layout>,
}

impl Focusable for FileExplorer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<ExplorerEvent> for FileExplorer {}

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> FileExplorer {
        FileExplorer {
            tree: FileTree::default(),
            active: None,
            scroll: UniformListScrollHandle::new(),
            menu: None,
            sort_menu_open: false,
            menu_focus: file_menu::MenuFocus::new(cx),
            focus_handle: cx.focus_handle(),
            focused: None,
            renaming: None,
            rename_focus: file_menu::MenuFocus::new(cx),
            vault_name: String::new(),
            dragging: None,
            drop: None,
            drop_claim: None,
        }
    }

    /// Gives the tree the keyboard, on the row it had, else the open file's
    /// row, else the first row: what `file-explorer:open` and Tab do.
    pub fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_rename(cx);
        if self.focused_ix().is_none() {
            let ix = self
                .active
                .as_deref()
                .and_then(|path| self.tree.visible_index(path))
                .or_else(|| self.tree.row(0).map(|_| 0));
            if let Some(ix) = ix {
                self.set_focused_ix(ix, cx);
            }
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Expands to `path`, puts the focus ring on its row and gives the tree
    /// the keyboard: `file-explorer:reveal-active-file`.
    pub fn reveal_and_focus(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.end_rename(cx);
        self.reveal(path, cx);
        self.focused = Some(path.to_string());
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Turns a row's own label into a field with the whole name selected:
    /// what F2 on a focused row and the menu's "Rename..." both do in
    /// Obsidian (`tools/reference-probe/results/2026-10-25-explorer-dnd.json`,
    /// steps `s09` and `s11`). The name is the one the row shows, so a
    /// file's extension is neither selected nor typed over.
    pub fn start_rename(&mut self, path: &str, cx: &mut Context<Self>) {
        self.reveal(path, cx);
        let Some(row) = self
            .tree
            .visible_index(path)
            .and_then(|ix| self.tree.row(ix).cloned())
        else {
            return;
        };
        self.focused = Some(row.path.clone());
        self.renaming = Some(RowEdit {
            path: row.path,
            editing: inline_field::Editing::selected(row.name),
            layout: None,
        });
        cx.notify();
    }

    /// Closes the field, with the keyboard going back to the tree. Called
    /// when the rename went through, and by Escape.
    pub fn end_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }

    /// The visible index of the focused row, if it is on a visible row.
    fn focused_ix(&self) -> Option<usize> {
        self.tree.visible_index(self.focused.as_deref()?)
    }

    fn set_focused_ix(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.tree.row(ix) {
            self.focused = Some(row.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn focus_down(&mut self, _: &FocusDown, _: &mut Window, cx: &mut Context<Self>) {
        self.focus_vertical(true, cx);
    }

    fn focus_up(&mut self, _: &FocusUp, _: &mut Window, cx: &mut Context<Self>) {
        self.focus_vertical(false, cx);
    }

    fn focus_vertical(&mut self, down: bool, cx: &mut Context<Self>) {
        let focused_key = self
            .focused_ix()
            .and_then(|ix| self.tree.row(ix).map(|row| row.key.clone()));
        if let Some(key) =
            self.tree
                .focus_vertical_key(focused_key.as_deref(), self.active.as_deref(), down)
        {
            self.set_focused_key(&key, cx);
        }
    }

    fn set_focused_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let index = self.tree.rows().position(|row| row.key == key);
        if let Some(index) = index {
            self.set_focused_ix(index, cx);
        }
    }

    /// Right opens a collapsed folder, steps into an open one, and does
    /// nothing on a file.
    fn focus_right(&mut self, _: &FocusRight, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self
            .focused_ix()
            .and_then(|ix| self.tree.row(ix).map(|row| row.key.clone()))
        else {
            return;
        };
        if let Some(next) = self.tree.focus_horizontal_key(&key, true) {
            self.set_focused_key(&next, cx);
        }
    }

    /// Left closes an open folder; on a file or a closed folder it climbs
    /// to the parent and closes that, or does nothing at the root.
    fn focus_left(&mut self, _: &FocusLeft, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self
            .focused_ix()
            .and_then(|ix| self.tree.row(ix).map(|row| row.key.clone()))
        else {
            return;
        };
        if let Some(next) = self.tree.focus_horizontal_key(&key, false) {
            self.set_focused_key(&next, cx);
        }
    }

    /// Enter opens the file or toggles the folder under the ring.
    fn activate(&mut self, _: &Activate, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) else {
            return;
        };
        if row.is_folder {
            self.tree.toggle(&row.path);
            self.emit_expanded_folders(cx);
            cx.notify();
        } else {
            cx.emit(ExplorerEvent::Open(row.path));
        }
    }

    /// Space toggles a folder and leaves a file alone.
    fn toggle_folder(&mut self, _: &ToggleFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) else {
            return;
        };
        if row.is_folder {
            self.tree.toggle(&row.path);
            self.emit_expanded_folders(cx);
            cx.notify();
        }
    }

    fn leave_tree(&mut self, _: &LeaveTree, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ExplorerEvent::LeaveFocus);
    }

    fn rename_focused(&mut self, _: &RenameFocused, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) {
            cx.emit(ExplorerEvent::Rename(row.path));
        }
    }

    fn delete_focused(&mut self, _: &DeleteFocused, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) {
            cx.emit(ExplorerEvent::Delete(row.path));
        }
    }

    /// A rebuilt tree: the folders that were open stay open, the sort order
    /// carries over and the open file keeps its highlight. Nothing is
    /// expanded or scrolled, so a file written elsewhere in the vault does
    /// not move the list.
    pub fn set_tree(&mut self, mut tree: FileTree, cx: &mut Context<Self>) {
        tree.preserve_view_from(&self.tree);
        self.tree = tree;
        // A row that is no longer in the tree cannot paint the field that
        // is on it, and a field nothing paints would hold the keyboard
        // with nothing on screen.
        let gone = self
            .renaming
            .as_ref()
            .is_some_and(|edit| self.tree.visible_index(&edit.path).is_none());
        if gone {
            self.renaming = None;
        }
        cx.notify();
    }

    pub fn set_vault_name(&mut self, name: String) {
        self.vault_name = name;
    }

    /// The pointer over a row while a row is dragged: the folder it would
    /// land in becomes the target, and the ghost says so.
    fn drag_over_row(
        &mut self,
        ix: usize,
        event: &DragMoveEvent<FileDrag>,
        cx: &mut Context<Self>,
    ) {
        if !event.bounds.contains(&event.event.position) {
            return;
        }
        let Some(row) = self.tree.row(ix) else {
            return;
        };
        let drag = event.drag(cx).clone();
        let folder = drop_folder(&row.path, row.is_folder);
        self.drop_claim = Some(event.event.position);
        self.dragged_over(drag, Some(folder), cx);
    }

    /// The pointer anywhere while a row is dragged: inside the list but on
    /// no row is the vault root (`s04`); outside it, no target at all.
    fn drag_over_list(&mut self, event: &DragMoveEvent<FileDrag>, cx: &mut Context<Self>) {
        if self.drop_claim == Some(event.event.position) {
            return;
        }
        let drag = event.drag(cx).clone();
        let inside = event.bounds.contains(&event.event.position);
        self.dragged_over(drag, inside.then(String::new), cx);
    }

    fn dragged_over(&mut self, drag: FileDrag, folder: Option<String>, cx: &mut Context<Self>) {
        drag.set_action(
            folder
                .as_deref()
                .map(|f| move_into_label(f, &self.vault_name).into()),
        );
        let dragging = Some(drag.path.clone());
        if self.drop != folder || self.dragging != dragging {
            self.drop = folder;
            self.dragging = dragging;
            cx.notify();
        }
        // The ghost reads the action when it is drawn.
        cx.refresh_windows();
    }

    fn dropped(&mut self, drag: &FileDrag, cx: &mut Context<Self>) {
        let target = self.drop.take();
        self.dragging = None;
        self.drop_claim = None;
        if let Some(folder) = target {
            if drop_moves(&drag.path, drag.folder, &folder) {
                cx.emit(ExplorerEvent::MoveInto {
                    path: drag.path.clone(),
                    folder,
                });
            }
        }
        cx.notify();
    }

    /// The file that just became the open one: mark it without changing
    /// folder expansion or scrolling the tree.
    pub fn follow(&mut self, path: &str, cx: &mut Context<Self>) {
        self.set_active(path, cx);
    }

    /// Marks `path` as the open file without touching the tree: its row is
    /// highlighted if a folder is not hiding it.
    pub fn set_active(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(path) {
            self.active = Some(path.to_string());
            cx.notify();
        }
    }

    /// No file is open: nothing is highlighted.
    pub fn clear_active(&mut self, cx: &mut Context<Self>) {
        if self.active.take().is_some() {
            cx.notify();
        }
    }

    pub fn set_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        if self.tree.sort() != order {
            self.tree.set_sort(order);
            cx.notify();
        }
    }

    pub fn set_expanded_folders(
        &mut self,
        folders: &std::collections::HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        self.tree.expand_folders(folders);
        cx.notify();
    }

    fn emit_expanded_folders(&self, cx: &mut Context<Self>) {
        let mut folders: Vec<_> = self.tree.expanded_folders().into_iter().collect();
        folders.sort();
        cx.emit(ExplorerEvent::FoldersExpanded(folders));
    }

    /// Marks `path` as the open file and scrolls it into view.
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        self.active = Some(path.to_string());
        self.show(path, cx);
    }

    fn render_sort_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.sort_menu_open {
            return None;
        }
        let current = self.tree.sort();
        let mut list = crate::ui::menu("explorer-sort-menu", theme)
            .absolute()
            .top(px(HEADER_HEIGHT as f32))
            .left(px(4.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.sort_menu_open = false;
                cx.notify();
            }));
        for (index, order) in SortOrder::ALL.into_iter().enumerate() {
            if index > 0 && index % 2 == 0 {
                list = list.child(crate::ui::file_tree_sort_separator(theme));
            }
            list = list.child(
                crate::ui::file_tree_sort_item(
                    "explorer-sort-option",
                    index,
                    order,
                    current,
                    theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.tree.sort() != order {
                        this.tree.set_sort(order);
                        cx.emit(ExplorerEvent::SortChanged(order));
                    }
                    this.sort_menu_open = false;
                    cx.notify();
                })),
            );
        }
        Some(list.into_any_element())
    }

    /// Scrolls `path` into view without claiming it is the open file.
    ///
    /// A folder is never the open file, so revealing one ??a bookmarked
    /// folder, or a folder just created ??leaves the highlight on the note
    /// that is open, as Obsidian does.
    pub fn reveal_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        self.show(path, cx);
    }

    fn show(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.tree.reveal(path) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// What the tree is showing, for the control socket's state dump: the
    /// rows a collapsed folder hides are not in it, because they are not on
    /// screen either.
    pub fn describe(&self, rows: usize) -> Value {
        let total = self.tree.len();
        let shown: Vec<Value> = self
            .tree
            .rows()
            .take(rows)
            .map(|row| {
                json!({
                    "path": row.path,
                    "name": row.name,
                    "depth": row.depth,
                    "folder": row.is_folder,
                    "kind": kind_name(row.kind),
                })
            })
            .collect();
        json!({
            "active": self.active,
            "focused": self.focused,
            "any_expanded": self.tree.any_expanded(),
            "rows": shown,
            "rows_total": if total > shown.len() { Some(total) } else { None },
            "menu": self.menu.as_ref().map(file_menu::describe),
            "renaming": self.renaming.as_ref().map(|edit| {
                let field = &edit.editing.field;
                json!({
                    "path": edit.path,
                    "text": field.text(),
                    "cursor": field.cursor(),
                    "selection": [field.selection().start, field.selection().end],
                })
            }),
        })
    }

    /// A click on a folder toggles it and puts the keyboard on it; a click
    /// on the open file's row puts the keyboard on that row; a click that
    /// opens a file hands the keyboard to the editor, as Obsidian's does.
    fn click(&mut self, ix: usize, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        // A press on another row is the end of an open rename: the field
        // keeps what it has, the way Escape does.
        self.end_rename(cx);
        let Some(row) = self.tree.row(ix).cloned() else {
            return;
        };
        if row.is_folder {
            self.tree.toggle(&row.path);
            self.focused = Some(row.path);
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else if new_tab {
            cx.emit(ExplorerEvent::OpenInNewTab(row.path));
        } else if self.active.as_deref() == Some(row.path.as_str()) {
            self.focused = Some(row.path);
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else {
            self.focused = Some(row.path.clone());
            cx.emit(ExplorerEvent::Open(row.path));
        }
    }

    fn open_menu(&mut self, ix: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        // The menu takes the keyboard, so an open field would be left
        // holding a caret it can no longer be typed into.
        self.end_rename(cx);
        if let Some(row) = self.tree.row(ix).cloned() {
            let target = match row.is_folder {
                true => Target::Folder(row.path),
                false => Target::File(row.path),
            };
            self.menu = Some(Menu {
                position,
                submenu: false,
                anchor: gpui::Anchor::TopLeft,
                target,
            });
            cx.notify();
        }
    }

    /// A right-click on the empty space below the last row: Obsidian's
    /// four-item menu, rooted at the vault.
    fn open_blank_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = Some(Menu {
            position,
            submenu: false,
            anchor: gpui::Anchor::TopLeft,
            target: Target::Blank,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The file menu over the row (or the empty space) it was opened on; a
    /// pick becomes the [`ExplorerEvent`] the workspace acts on.
    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.menu.as_ref()?;
        let theme = cx.global::<Theme>().clone();
        let target = menu.target.clone();
        let focus = self.menu_focus.handle.clone();
        // The explorer sits at the window's left, so its submenus open to
        // the right; no window is at hand here and none is needed.
        Some(file_menu::render(
            menu,
            &theme,
            &focus,
            px(f32::MAX),
            cx,
            |this, cx| this.close_menu(cx),
            move |_, pick, _, cx| {
                if let Some(event) = event_for(pick, &target) {
                    cx.emit(event);
                }
            },
            |this, open, cx| {
                if let Some(menu) = this.menu.as_mut() {
                    if menu.submenu != open {
                        menu.submenu = open;
                        cx.notify();
                    }
                }
            },
        ))
    }

    /// The label of the row being renamed, painted as a shaped line so the
    /// caret and the selection sit in the running text and a click lands
    /// the caret where the pointer was -- the same painting the view
    /// header's title field uses.
    fn render_rename_field(
        &self,
        ix: usize,
        name: SharedString,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let read_entity = cx.entity();
        let paint_entity = cx.entity();
        let read_focus = self.rename_focus.handle.clone();
        let paint_focus = self.rename_focus.handle.clone();
        let color = theme.tokens.nav_item_color_active;
        let caret_color = theme.tokens.text_normal;
        let mut selection_color = theme.tokens.interactive_accent;
        selection_color.a = if theme.dark { 0.35 } else { 0.25 };
        let field = canvas(
            move |bounds, window, cx| {
                let explorer = read_entity.read(cx);
                let edit = explorer.renaming.as_ref();
                let text: SharedString = match edit {
                    Some(edit) => edit.editing.field.text().to_string().into(),
                    None => name.clone(),
                };
                let scroll_x = edit
                    .and_then(|e| e.layout.as_ref())
                    .map(|l| l.scroll_x)
                    .unwrap_or(px(0.));
                inline_field::shape(
                    text,
                    edit.map(|e| &e.editing),
                    color,
                    read_focus.is_focused(window),
                    bounds,
                    scroll_x,
                    window,
                )
            },
            move |bounds, prepaint: inline_field::Prepaint, window, cx| {
                window.handle_input(
                    &paint_focus,
                    ElementInputHandler::new(bounds, paint_entity.clone()),
                    cx,
                );
                let layout =
                    inline_field::paint(prepaint, bounds, selection_color, caret_color, window, cx);
                paint_entity.update(cx, |explorer, _| {
                    if let Some(edit) = explorer.renaming.as_mut() {
                        edit.layout = Some(layout);
                    }
                });
            },
        )
        .size_full();
        // The press is taken here rather than on the row: a row being
        // renamed does not open on a click, and the caret has to land
        // where the pointer was.
        div()
            .id(("rename-field", ix))
            .size_full()
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.rename_clicked(event, window, cx);
                }),
            )
            .child(field)
    }

    /// A press inside the open field: the caret goes to the offset the
    /// pointer landed on, and a double press takes the word it landed in.
    fn rename_clicked(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.renaming.as_mut() else {
            return;
        };
        let Some(layout) = edit.layout.clone() else {
            return;
        };
        let Some(at) = layout.offset_for(event.position, edit.editing.field.text()) else {
            return;
        };
        edit.editing
            .field
            .click(at, event.click_count, event.modifiers.shift);
        window.focus(&self.rename_focus.handle, cx);
        cx.notify();
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let theme = cx.global::<Theme>().clone();
        let focused_ix = self.focused_ix();
        range
            .filter_map(|ix| {
                let row = self.tree.row(ix)?;
                let active = self.active.as_deref() == Some(row.path.as_str());
                let focused = focused_ix == Some(ix);
                let name: SharedString = row.name.clone().into();
                let renaming = self.renaming.as_ref().is_some_and(|e| e.path == row.path);
                let tag: Option<SharedString> = row.tag.clone().map(Into::into);
                // A drag in progress: the dragged row is `is-being-dragged`
                // (the accent fill, white text); the target folder's whole
                // block ??its row and the rows inside it ??is
                // `is-being-dragged-over`, the accent at a tenth with a 4 px
                // radius, and the folder's own name turns accent
                // (2026-11-07 `explorer-drag-look`). The root target has no
                // wash.
                let dragged = self.dragging.as_deref() == Some(row.path.as_str());
                let in_block = |path: &str| {
                    self.drop.as_deref().is_some_and(|t| {
                        !t.is_empty() && (path == t || path.starts_with(&format!("{t}/")))
                    })
                };
                let washed = in_block(&row.path);
                let block_top = washed && self.drop.as_deref() == Some(row.path.as_str());
                let drag = FileDrag {
                    path: row.path.clone(),
                    folder: row.is_folder,
                    name: row.name.clone().into(),
                    action: Rc::new(RefCell::new(None)),
                };
                // TFT's row: 13 px text in a 24 px row, square and edge to edge
                // in the sidebar, which is Zed's tree rather than Obsidian's
                // 25 px rows with an 8 px radius and a gap between them.
                let t = &theme.tokens;
                let row_el =
                    crate::ui::file_tree_row(("row", ix), row.depth, active, focused, &theme);
                // A file's row previews it; a folder's previews nothing.
                let row_el = if row.is_folder {
                    row_el
                } else {
                    let path = row.path.clone();
                    let this = cx.weak_entity();
                    // The row's 2 px ring border is part of its box.
                    crate::hover_preview::hover_row_bordered(
                        row_el,
                        2.,
                        move |over, anchor, modifiers, _, cx| {
                            let path = path.clone();
                            this.update(cx, |_, cx| {
                                cx.emit(ExplorerEvent::Hover {
                                    path,
                                    over,
                                    anchor,
                                    modifiers,
                                })
                            })
                            .ok();
                        },
                    )
                };
                Some(
                    row_el
                        .children(crate::ui::file_tree_indent_guides(row.depth, &theme))
                        .when(block_top, |d| d.text_color(t.text_accent))
                        .when(dragged, |d| {
                            d.bg(t.interactive_accent).text_color(gpui::white())
                        })
                        .when(!renaming, |d| {
                            d.on_drag(drag, |drag, grab, _, cx| {
                                cx.new(|_| FileGhost(drag.clone(), grab))
                            })
                            .on_drag_move(cx.listener(
                                move |this, event: &DragMoveEvent<FileDrag>, _, cx| {
                                    this.drag_over_row(ix, event, cx)
                                },
                            ))
                        })
                        .when(!renaming, |d| {
                            d.on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, window, cx| {
                                    let modifiers = event.modifiers();
                                    let new_tab = modifiers.control || modifiers.platform;
                                    this.click(ix, new_tab, window, cx)
                                },
                            ))
                        })
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                // The empty space below the rows has its own
                                // right-click; without this the row's menu
                                // would open and then be replaced by that one.
                                cx.stop_propagation();
                                this.open_menu(ix, event.position, cx)
                            }),
                        )
                        // The wash runs through the rows of the block, covering
                        // each row's 2 px ring border, so the block reads as one
                        // box; the rows themselves sit flush against each other.
                        .when(washed, |d| {
                            let mut wash = t.interactive_accent;
                            wash.a = 0.1;
                            d.child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .left(px(-2.))
                                    .right(px(-2.))
                                    .bottom(px(-2.))
                                    .bg(wash),
                            )
                        })
                        .child(crate::ui::file_tree_row_marker(
                            row.is_folder,
                            self.tree.is_collapsed(&row.path),
                            &theme,
                        ))
                        .child(
                            crate::ui::file_tree_row_label()
                                .when(!renaming, |d| d.child(name.clone()))
                                // The field paints itself on a canvas,
                                // which has no size of its own: without a
                                // height to fill it draws nothing at all.
                                .when(renaming, |d| {
                                    d.h_full().child(self.render_rename_field(
                                        ix,
                                        name,
                                        theme.clone(),
                                        cx,
                                    ))
                                }),
                        )
                        .children(tag.map(|tag| crate::ui::file_tree_row_tag(tag, &theme))),
                )
            })
            .collect()
    }
}

/// Enter and Escape in a row being renamed. Enter only *reports* the name:
/// the workspace owns the validator and the link rewrite, and closes the
/// field with [`FileExplorer::end_rename`] once the rename has happened.
/// Escape leaves the name as it was and the row keeps the focus ring, which
/// is what the 2026-10-25 pass saw.
impl inline_field::Host for FileExplorer {
    fn editing(&mut self) -> Option<&mut inline_field::Editing> {
        self.renaming.as_mut().map(|edit| &mut edit.editing)
    }

    fn commit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.renaming.as_ref() else {
            return;
        };
        cx.emit(ExplorerEvent::RenameTo {
            path: edit.path.clone(),
            name: edit.editing.field.text().trim().to_string(),
        });
    }

    fn cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.end_rename(cx);
    }
}

/// Typed, composed and pasted text in a row being renamed; the halves are
/// [`inline_field`]'s, shared with the view header's title field. The
/// handler is registered by the label's own element against the field's own
/// focus handle, so it only ever sees text meant for that row.
impl EntityInputHandler for FileExplorer {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        inline_field::text_for_range(
            &self.renaming.as_ref()?.editing,
            range_utf16,
            adjusted_range,
        )
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(inline_field::selected_text_range(
            &self.renaming.as_ref()?.editing,
        ))
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        inline_field::marked_text_range(&self.renaming.as_ref()?.editing)
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(edit) = self.renaming.as_mut() {
            edit.editing.marked = None;
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.renaming.as_mut() else {
            return;
        };
        inline_field::replace_text_in_range(&mut edit.editing, range_utf16, new_text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.renaming.as_mut() else {
            return;
        };
        inline_field::replace_and_mark_text_in_range(
            &mut edit.editing,
            range_utf16,
            new_text,
            new_selected_range,
        );
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let edit = self.renaming.as_ref()?;
        Some(inline_field::bounds_for_range(
            &edit.editing,
            edit.layout.as_ref()?,
            range_utf16,
            element_bounds,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for FileExplorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let count = self.tree.len();
        // An open menu holds the keyboard, so Escape reaches it first; a
        // closed one hands the keyboard back where it was.
        let menu_open = self.menu.is_some();
        self.menu_focus.sync(menu_open, window, cx);
        // A row being renamed holds the keyboard under the crate's field
        // context, so Enter, Escape and the editing keys go to the field
        // and the tree's own keys are off while it is open.
        let renaming = self.renaming.is_some();
        self.rename_focus.sync(renaming, window, cx);
        // A drag that ended anywhere but in the list leaves nothing behind.
        if !cx.has_active_drag() && (self.dragging.is_some() || self.drop.is_some()) {
            self.dragging = None;
            self.drop = None;
            self.drop_claim = None;
        }
        let menu = self.render_menu(cx);
        let sort_menu = self.render_sort_menu(&theme, cx);
        let toolbar = crate::ui::file_tree_toolbar(&theme)
            .child(
                crate::ui::header_button(
                    "explorer-sort",
                    "arrow-up-narrow-wide",
                    "Change sort order",
                    self.sort_menu_open,
                    &theme,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sort_menu_open = !this.sort_menu_open;
                    cx.notify();
                })),
            )
            .child(
                crate::ui::header_button(
                    "explorer-reveal",
                    "gallery-vertical",
                    "Auto-reveal current file",
                    false,
                    &theme,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(path) = this.active.clone() {
                        this.reveal(&path, cx);
                    }
                })),
            )
            .child(
                crate::ui::header_button(
                    "explorer-expand",
                    "chevrons-up-down",
                    if self.tree.any_expanded() {
                        "Collapse all"
                    } else {
                        "Expand all"
                    },
                    false,
                    &theme,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.tree.any_expanded() {
                        this.tree.collapse_all();
                    } else {
                        this.tree.expand_all();
                    }
                    this.emit_expanded_folders(cx);
                    cx.notify();
                })),
            )
            .child(
                crate::ui::header_button("explorer-new-note", "plus", "New note", false, &theme)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(ExplorerEvent::NewNote {
                            folder: String::new(),
                        });
                    })),
            )
            .children(sort_menu);
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .when(renaming, |d| {
                inline_field::bind_keys(
                    d.key_context("TextInput")
                        .track_focus(&self.rename_focus.handle),
                    cx,
                )
            })
            .when(!renaming, |d| {
                d.key_context("Explorer").track_focus(&self.focus_handle)
            });
        root.on_action(cx.listener(Self::focus_up))
            .on_action(cx.listener(Self::focus_down))
            .on_action(cx.listener(Self::focus_left))
            .on_action(cx.listener(Self::focus_right))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::toggle_folder))
            .on_action(cx.listener(Self::leave_tree))
            .on_action(cx.listener(Self::rename_focused))
            .on_action(cx.listener(Self::delete_focused))
            .child(toolbar)
            // The list is wrapped so that the space below the last row
            // takes a right-click of its own: `uniform_list` hands an event
            // outside every row straight through, and a row's own handler
            // stops propagation so only one of the two menus opens.
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.open_blank_menu(event.position, cx)
                        }),
                    )
                    // Rows claim a drag over them first; anywhere else in
                    // the list is the root, and the drop lands wherever the
                    // target last was.
                    .on_drag_move(cx.listener(|this, event: &DragMoveEvent<FileDrag>, _, cx| {
                        this.drag_over_list(event, cx)
                    }))
                    .on_drop(cx.listener(|this, drag: &FileDrag, _, cx| this.dropped(drag, cx)))
                    .child(
                        uniform_list("file-explorer", count, cx.processor(Self::render_rows))
                            .track_scroll(&self.scroll)
                            .size_full(),
                    ),
            )
            .children(menu)
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    /// The path an event acts on. The match is exhaustive on purpose: a new
    /// [`ExplorerEvent`] will not compile until it says which row it carries,
    /// which is what keeps a menu from ever acting on the wrong one.
    fn payload(event: &ExplorerEvent) -> Option<&str> {
        match event {
            ExplorerEvent::Open(p)
            | ExplorerEvent::OpenInNewTab(p)
            | ExplorerEvent::Rename(p)
            | ExplorerEvent::Move(p)
            | ExplorerEvent::Duplicate(p)
            | ExplorerEvent::Delete(p)
            | ExplorerEvent::OpenToRight(p)
            | ExplorerEvent::OpenInNewWindow(p)
            | ExplorerEvent::CopyPath(p)
            | ExplorerEvent::CopyAbsolutePath(p)
            | ExplorerEvent::OpenInDefaultApp(p)
            | ExplorerEvent::ShowInSystemExplorer(p)
            | ExplorerEvent::SearchInFolder(p) => Some(p),
            ExplorerEvent::NewNote { folder } | ExplorerEvent::NewCanvas { folder } => Some(folder),
            ExplorerEvent::NewFolder { parent } => Some(parent),
            ExplorerEvent::RenameTo { path, .. } | ExplorerEvent::MoveInto { path, .. } => {
                Some(path)
            }
            ExplorerEvent::SortChanged(_)
            | ExplorerEvent::FoldersExpanded(_)
            | ExplorerEvent::LeaveFocus
            | ExplorerEvent::Hover { .. } => None,
        }
    }

    /// Every row of the explorer's three menus has an event, and the event
    /// carries the folder or file the menu was opened on -- the vault root
    /// for the empty space -- so a menu can never act on the wrong row.
    #[test]
    fn every_item_carries_the_row_it_was_opened_on() {
        for (target, want) in [
            (Target::Folder("Notes/Deep".into()), "Notes/Deep"),
            (
                Target::File("Notes/Deep/Plan.md".into()),
                "Notes/Deep/Plan.md",
            ),
            (Target::Blank, ""),
        ] {
            for item in file_menu::items(&target) {
                let event = event_for(item.pick, &target)
                    .unwrap_or_else(|| panic!("{} has no event on {target:?}", item.label));
                // A file's creation and folder-search rows act on the folder
                // that holds it; every other row acts on the row itself.
                let want = match (&target, item.pick) {
                    (
                        Target::File(_),
                        Pick::NewNote | Pick::NewFolder | Pick::NewCanvas | Pick::SearchInFolder,
                    ) => "Notes/Deep",
                    _ => want,
                };
                assert_eq!(
                    payload(&event),
                    Some(want),
                    "{} acts on the wrong row",
                    item.label
                );
            }
        }
    }

    /// The one pick the explorer never offers is the one it has no event
    /// for: "Reveal file in navigation" reveals *in* the explorer.
    #[test]
    fn reveal_in_navigation_is_not_an_explorer_event() {
        assert!(event_for(Pick::RevealInNavigation, &Target::File("Plan.md".into())).is_none());
    }
}
