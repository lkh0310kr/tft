//! The file context menu, shared by the Files view and the editor. A view
//! reports a [`Pick`] and the path; the menu only lists rows it can run.
//! Unobserved submenus are omitted rather than guessed. "Show in system
//! explorer" lives here too, with its platform command table tested below.

use crate::theme::Theme;
use gpui::{
    actions, anchored, deferred, div, prelude::*, px, relative, Anchor, AnyElement, App, Context,
    FocusHandle, KeyBinding, Pixels, Point, SharedString, Stateful, Window,
};
use serde_json::{json, Value};
use std::cell::Cell;

use std::path::{Path, PathBuf};
use std::rc::Rc;

actions!(menu, [CloseMenu]);

/// Escape closes an open menu: observed, and what every probe run relies
/// on to take one menu down before raising the next. The binding lives in
/// the `Menu` key context, which every menu drawn through [`menu_box`]
/// wears while it holds the keyboard.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseMenu, Some("Menu"))]);
}

/// The keyboard a host hands to its menus: the focus handle the open menu
/// tracks, and where focus was before, so closing the menu -- by Escape,
/// a pick or a press outside -- puts it back.
///
/// The host calls [`MenuFocus::sync`] from its render with whether a menu
/// is open: the first render with one open takes focus (remembering the
/// previous handle), the first render with none restores it. Routing it
/// through the render means every way a menu closes restores focus the
/// same way, with no window needed at the close site.
pub struct MenuFocus {
    pub handle: FocusHandle,
    restore: Option<FocusHandle>,
}

impl MenuFocus {
    pub fn new(cx: &mut App) -> MenuFocus {
        MenuFocus {
            handle: cx.focus_handle(),
            restore: None,
        }
    }

    pub fn sync(&mut self, open: bool, window: &mut Window, cx: &mut App) {
        match (open, self.restore.is_some()) {
            (true, false) => {
                self.restore = Some(window.focused(cx).unwrap_or_else(|| self.handle.clone()));
                window.focus(&self.handle, cx);
            }
            (false, true) => {
                if let Some(previous) = self.restore.take() {
                    if self.handle.is_focused(window) {
                        window.focus(&previous, cx);
                    }
                }
            }
            _ => {}
        }
    }
}

thread_local! {
    /// Whether the pointer rests on an open submenu. It outlives a frame
    /// because gpui reports a hover only when it *changes*: a flag made
    /// afresh in each render would read `false` again after the re-render
    /// the submenu's own opening causes, and never see the hover again.
    static OVER_SUBMENU: Rc<Cell<bool>> = Rc::new(Cell::new(false));
}

/// [`menu_box`] for a menu with a submenu drawn beside the list.
/// gpui decides "outside" from the list's own bounds, and a submenu stands
/// beside them, so a press on one of its rows would close the menu on the
/// way down, before the click that picks the row ever lands. `over_submenu`
/// is set while the pointer rests on the submenu, and a press then is not
/// outside.
pub fn menu_box_with_submenu<V: 'static>(
    list: Stateful<gpui::Div>,
    focus: &FocusHandle,
    cx: &mut Context<V>,
    on_close: impl Fn(&mut V, &mut Context<V>) + 'static,
    over_submenu: Rc<Cell<bool>>,
) -> Stateful<gpui::Div> {
    let on_close = Rc::new(on_close);
    list.track_focus(focus)
        .key_context("Menu")
        .occlude()
        .on_action(cx.listener({
            let on_close = on_close.clone();
            move |this, _: &CloseMenu, _, cx| on_close(this, cx)
        }))
        .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
            if !over_submenu.get() {
                on_close(this, cx)
            }
        }))
}

/// Obsidian's `data-section` on a menu item. Items are grouped by it in the
/// order below, and a separator is drawn wherever the section changes — which
/// is how the captured menus get their separators without any being written
/// down one at a time ([[Specification#The file explorer's context menus]]).
/// [`Section::Danger`] takes **two**: Obsidian emits an empty section before
/// it, and both explorer menus were captured with the two consecutive
/// separators that leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `close`: the tab header menu's Close, Close others and Close all.
    Close,
    /// `pane`: the tab header menu's Pin.
    Pane,
    /// `find`: a markdown tab's Find... and Replace....
    Find,
    /// The folder row's and the empty space's creation items.
    ActionPrimary,
    /// The file row's "Open in..." items.
    Open,
    /// `selection-link`: the editor menu's two link-making rows, which
    /// Obsidian draws whether or not anything is selected.
    SelectionLink,
    /// `selection`: the editor menu's rows built from what is selected,
    /// and the Format / Edit link / Edit tag row that sits with them.
    Selection,
    /// `clipboard`: the editor menu's Cut, Copy, Paste, Paste as plain text
    /// and Select all.
    Clipboard,
    Action,
    /// Obsidian leaves "Copy path" with no section at all.
    Unsectioned,
    System,
    Danger,
}

/// What a clicked row asks for. The path it acts on is the menu's
/// [`Target`], so a menu can never act on the wrong row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    // ----- the tab header's own rows ------------------------------------------
    /// "Close": the right-clicked tab, pinned or not.
    CloseTab,
    /// "Close others": every other tab of its group but the pinned ones.
    CloseOthers,
    /// "Close tabs after": the tabs to its right but the pinned ones.
    CloseAfter,
    /// "Close all": every tab of its group but the pinned ones.
    CloseAll,
    /// "Pin" / "Unpin": `workspace:toggle-pin` on the tab.
    TogglePin,
    /// "Move to new window": `workspace:move-to-new-window` on the tab.
    MoveToNewWindow,
    /// A markdown tab's "Add file property", "Find..." and "Replace...":
    /// `markdown:add-metadata-property`, `editor:open-search` and
    /// `editor:open-search-replace`.
    AddFileProperty,
    Find,
    Replace,
    OpenInNewTab,
    OpenToRight,
    /// "Open in new window": the file in a popout window of its own
    /// (`workspace:open-in-new-window` on the view header's menu, where
    /// it copies the leaf; a fresh tab on the row's file everywhere else).
    OpenInNewWindow,
    /// The view header's "Split right" and "Split down":
    /// `workspace:split-vertical` and `split-horizontal` on the leaf's
    /// own group.
    SplitRight,
    SplitDown,
    /// "Make a copy".
    Duplicate,
    Move,
    CopyPath,
    CopyAbsolutePath,
    OpenInDefaultApp,
    ShowInSystemExplorer,
    /// "Reveal file in navigation": the Files view scrolled to the row.
    RevealInNavigation,
    Rename,
    Delete,
    /// The creation items, in the target folder.
    NewNote,
    NewFolder,
    NewCanvas,
    SearchInFolder,
    // ----- the editor menu's rows ------------------------------------------
    /// "Add link": the wikilink shape around the selection
    /// (`editor:insert-wikilink`). Obsidian's own label for that command is
    /// "Add internal link", and the row wears `lucide-link`, so the pairing
    /// is read off the icon and the label rather than observed directly.
    AddLink,
    /// "Add external link": the Markdown link shape (`editor:insert-link`),
    /// the row Obsidian wears `lucide-external-link` on.
    AddExternalLink,
    /// Search for “<selection>”: the Search pane, the selected text as its
    /// query, the way a folder row's "Search in folder" opens it.
    SearchForSelection,
    Cut,
    Copy,
    Paste,
    /// "Paste as plain text". TFT's clipboard is read as text and pasted
    /// verbatim, with no HTML or Markdown conversion on the way in, so this
    /// is the same paste — which is what the row promises.
    PastePlain,
    SelectAll,
}

/// One row of a context menu: what Obsidian draws, and what a click reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: SharedString,
    pub icon: &'static str,
    pub section: Section,
    pub pick: Pick,
    /// Drawn greyed and inert, the way Obsidian draws Cut and Copy with
    /// nothing selected (`is-disabled` on the row).
    pub disabled: bool,
    /// `is-warning`: the label in the warning colour.
    pub warning: bool,
    /// `mod-checked`: a check at the row's right.
    pub checked: bool,
    /// `has-submenu`: the rows a hover opens beside this one.
    pub submenu: Vec<Item>,
}

/// What was right-clicked. Paths are vault-relative, folders without a
/// trailing slash (`""` is the vault root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A file row of the Files view.
    File(String),
    /// A folder row of the Files view; `""` is the vault root.
    Folder(String),
    /// The empty space below the Files view's last row, which Obsidian gives
    /// the four `action-primary` items alone, rooted at the vault.
    Blank,
    /// A row standing for a file somewhere other than the Files view: a
    /// Links row or a mentions group of the Outgoing links pane. Obsidian's menu there is the file menu without "Make a copy",
    /// "Delete" and the version history, with Rename moved up beside Move
    /// and "Reveal file in navigation" added at the end
    /// ([[Specification#Observed outgoing links pane behavior, Obsidian 1.14.4 (installer build 1.13.7)]]).
    Linked(String),
    /// A right click in the editor's text, in either editing mode: the
    /// probe raised the same rows in Live Preview and in Source mode.
    Editor(EditorClick),
    /// The view header's **More options** over a base leaf
    /// (`tools/reference-probe/results/2026-11-01-bases-ui.json`, `s02`
    /// `moreOptions`): the file menu with the leaf's two splits in place
    /// of the open-in rows, Rename beside Move, and "Delete
    /// file" alone in the danger pair. The path is the leaf's file.
    View(String),
    /// A right click on a Search pane result: a file row (`file_row`) or
    /// one of its match rows, both standing for the file
    /// (`tools/reference-probe/results/2026-11-16-bases-cells-search-menu.json`,
    /// `s07`). The two menus are the same but for "Delete file", which only
    /// the file row draws; a match row's rows act on the file, never on the
    /// line.
    Search { path: String, file_row: bool },
    /// A right click on a tab header
    /// (`tools/reference-probe/results/2026-11-09-tab-header-menu.json`):
    /// the close rows, Pin, the leaf's window and split rows, then the
    /// view header's file rows. A right click does not make the tab
    /// active, and an inactive tab's menu stops after Move to new window.
    /// `group` and `ix` name the tab; `path` is its file, empty for a tab
    /// with none; `tabs` is how many its group holds, since Close others
    /// and Close all are drawn only beside another tab, and Close tabs
    /// after only with one to its right; `pinned` names the Pin row.
    Tab {
        group: u64,
        ix: usize,
        path: String,
        tabs: usize,
        pinned: bool,
        active: bool,
    },
}

/// A right click in the editor: what it landed on, and the selection as it
/// stood when it arrived.
///
/// The two selection fields are not one field twice. Obsidian built the
/// Search row from the selection **as it was at the click** and left Cut and
/// Copy to the selection that **survived** it: a right click outside a
/// selection moves the caret, which takes the selection away, and the menu
/// it raised still carried "Search for “…”" while Cut and Copy were drawn
/// disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorClick {
    pub spot: Spot,
    /// The text selected when the click arrived, empty with no selection.
    /// It is what names the Search row.
    pub selection: String,
    /// Whether a selection survived the click: a click inside one keeps it,
    /// a click anywhere else collapses it. Cut and Copy follow this.
    pub selected: bool,
}

/// What a right click in the editor landed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spot {
    /// Plain text — and an external Markdown link, and a task checkbox in
    /// Live Preview, both of which raised the plain menu too.
    Text,
    /// A tag. Obsidian puts "Edit tag" where the plain menu's Format row
    /// sits; both are left out here (Format is a submenu, Edit tag has no
    /// command behind it), so a tag's rows are the plain menu's. The name
    /// rides along for the probe-shaped row to come.
    Tag(String),
    /// A wikilink that resolves to this vault file: the file menu's rows
    /// wrap the text rows.
    Link(String),
}

impl Target {
    /// The path every item of the menu acts on; the vault root for
    /// [`Target::Blank`], and nothing for an editor click that is not on a
    /// link to a file.
    pub fn path(&self) -> &str {
        match self {
            Target::File(p) | Target::Folder(p) | Target::Linked(p) | Target::View(p) => p,
            Target::Tab { path, .. } | Target::Search { path, .. } => path,
            Target::Blank => "",
            Target::Editor(click) => match &click.spot {
                Spot::Link(path) => path,
                Spot::Text | Spot::Tag(_) => "",
            },
        }
    }
}

/// Where a right click in the editor leaves the caret: `None` to keep the
/// selection as it is, `Some(at)` to collapse the caret onto the click.
///
/// Observed: with the caret at the document start a right click on a word
/// moved it to that word; a right click *inside* a selection kept the
/// selection; a right click *outside* one moved the caret to the click.
/// A click exactly on either end of the selection was not probed — it is
/// taken as inside, the way a pointer landing on the edge of the painted
/// highlight reads (OBSERVE).
#[cfg(test)]
fn right_click_caret(selection: &std::ops::Range<usize>, at: usize) -> Option<usize> {
    if !selection.is_empty() && selection.start <= at && at <= selection.end {
        None
    } else {
        Some(at)
    }
}

/// The menu for a target, in Obsidian's order.
///
/// One of the captured items is left out rather than drawn dead, because
/// TFT has no command behind it: **Open version history** (no file
/// history). Listing a row that cannot run is what the command registry
/// refuses to do, and the menu refuses it too; the gaps are entries in
/// `docs/UI parity backlog.md`. Dropping the only item of a section
/// drops its separator
/// with it, so the `view` section's separator is absent here where the
/// capture has it. **Open in new window** was a third until the popout
/// windows landed (2026-11-09); it wears `lucide-picture-in-picture-2`
/// in every capture.
///
/// Obsidian's file row has no creation items at all — TFT's menu used to
/// offer "New note" and "New folder" on a file, which is the one thing here
/// that was wrong rather than missing.
/// The Files tree's one menu. A file, a folder and the blank space below the
/// last row show the same rows in the same order; a row that cannot act on
/// the target is disabled rather than left out, as Zed's project panel does.
/// No icons: the tree's menu is a plain list.
fn tree_items(target: &Target) -> Vec<Item> {
    use Pick as P;
    use Section as S;
    let file = matches!(target, Target::File(_));
    let folder = matches!(target, Target::Folder(_));
    let row = |label: &'static str, section: Section, pick: Pick, enabled: bool| Item {
        label: SharedString::new_static(label),
        icon: "",
        section,
        pick,
        disabled: !enabled,
        warning: false,
        checked: false,
        submenu: Vec::new(),
    };
    vec![
        // A file's creation rows act on the folder that holds it, so they
        // stay enabled there too: a context menu greys out only what has no
        // sensible target, never an item whose target is one level up.
        row("New File", S::ActionPrimary, P::NewNote, true),
        row("New folder", S::ActionPrimary, P::NewFolder, true),
        row("New canvas", S::ActionPrimary, P::NewCanvas, true),
        row("Open in new tab", S::Open, P::OpenInNewTab, file),
        row("Open to the right", S::Open, P::OpenToRight, file),
        row("Open in new window", S::Open, P::OpenInNewWindow, file),
        row("Duplicate", S::Action, P::Duplicate, file || folder),
        row("Move to...", S::Action, P::Move, file || folder),
        // A file's "Search in folder" is the folder above it.
        row(
            "Search in folder",
            S::Action,
            P::SearchInFolder,
            file || folder,
        ),
        row("Copy Path", S::Unsectioned, P::CopyAbsolutePath, file || folder),
        row("Copy Relative Path", S::Unsectioned, P::CopyPath, file || folder),
        row("Open in default app", S::System, P::OpenInDefaultApp, file),
        row(
            reveal_label(),
            S::System,
            P::ShowInSystemExplorer,
            file || folder,
        ),
        row("Rename...", S::Danger, P::Rename, file || folder),
        row("Delete", S::Danger, P::Delete, file || folder),
    ]
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Reveal in File Explorer"
    } else {
        "Reveal in File Manager"
    }
}

pub fn items(target: &Target) -> Vec<Item> {
    use Pick as P;
    use Section as S;
    match target {
        // One list for the whole Files tree, as Zed's project panel has: the
        // same rows on a file, a folder and the empty space below them, with
        // the rows that cannot act on that target disabled and dimmed rather
        // than absent. The menu draws no icons, so its rows line up like a
        // plain list.
        Target::File(_) | Target::Folder(_) | Target::Blank => tree_items(target),
        // The 2026-10-22 probe read this menu's labels and separators off a
        // Links row but not its icons, so these are the explorer's for the
        // same labels. "Reveal file in navigation" has no explorer twin;
        // `folder-open` is a guess marked OBSERVE until a probe reads it.
        Target::Linked(_) => vec![
            item("Open in new tab", "file-plus", S::Open, P::OpenInNewTab),
            item(
                "Open to the right",
                "separator-vertical",
                S::Open,
                P::OpenToRight,
            ),
            item(
                "Open in new window",
                "picture-in-picture-2",
                S::Open,
                P::OpenInNewWindow,
            ),
            item("Rename...", "pen-line", S::Action, P::Rename),
            item("Move file to...", "folder-tree", S::Action, P::Move),
            item("Copy path", "clipboard", S::Unsectioned, P::CopyPath),
            item(
                "Open in default app",
                "arrow-up-right",
                S::System,
                P::OpenInDefaultApp,
            ),
            item(
                "Show in system explorer",
                "arrow-up-right",
                S::System,
                P::ShowInSystemExplorer,
            ),
            item(
                "Reveal file in navigation",
                "folder-open",
                S::System,
                P::RevealInNavigation,
            ),
        ],
        Target::Editor(click) => editor_items(click),
        // Observed on Obsidian 1.15.1 by real right clicks (`s07`), in the
        // order drawn: the three open rows, Rename / Move in
        // `action` (Rename's `lucide-edit-3` is `pen-line`), the three
        // `system` rows with Reveal's `lucide-folder-open`, the danger pair
        // with "Delete file" on a file row only, and "Copy path" last,
        // unsectioned. Copy path is a submenu there (as TFT URL / from
        // vault folder / from system root) and one row here, as on every
        // other menu of this module.
        Target::Search { file_row, .. } => {
            let mut rows = vec![
                item("Open in new tab", "file-plus", S::Open, P::OpenInNewTab),
                item(
                    "Open to the right",
                    "separator-vertical",
                    S::Open,
                    P::OpenToRight,
                ),
                item(
                    "Open in new window",
                    "picture-in-picture-2",
                    S::Open,
                    P::OpenInNewWindow,
                ),
                item("Rename...", "pen-line", S::Action, P::Rename),
                item("Move file to...", "folder-tree", S::Action, P::Move),
                item(
                    "Open in default app",
                    "arrow-up-right",
                    S::System,
                    P::OpenInDefaultApp,
                ),
                item(
                    "Show in system explorer",
                    "arrow-up-right",
                    S::System,
                    P::ShowInSystemExplorer,
                ),
                item(
                    "Reveal file in navigation",
                    "folder-open",
                    S::System,
                    P::RevealInNavigation,
                ),
            ];
            if *file_row {
                rows.push(Item {
                    warning: true,
                    ..item("Delete file", "trash-2", S::Danger, P::Delete)
                });
            }
            rows.push(item("Copy path", "clipboard", S::Unsectioned, P::CopyPath));
            rows
        }
        // Observed on a base leaf; "Open version history" and the "Open
        // linked view" submenu are left out, as the file row's are, and
        // dropping the two `view` rows drops their separator. "Copy path" is a submenu there (as TFT URL / from
        // vault folder / from system root) and one row here, like the file
        // row's. Obsidian draws `lucide-edit-3` on Rename, which is
        // `pen-line`.
        Target::Tab {
            ix,
            path,
            tabs,
            pinned,
            active,
            ..
        } => tab_items(path, *ix, *tabs, *pinned, *active),
        Target::View(_) => vec![
            item("Split right", "separator-vertical", S::Open, P::SplitRight),
            item("Split down", "separator-horizontal", S::Open, P::SplitDown),
            item(
                "Open in new window",
                "picture-in-picture-2",
                S::Open,
                P::OpenInNewWindow,
            ),
            item("Rename...", "pen-line", S::Action, P::Rename),
            item("Move file to...", "folder-tree", S::Action, P::Move),
        ]
        .into_iter()
        .chain([
            item("Copy path", "clipboard", S::Unsectioned, P::CopyPath),
            item(
                "Open in default app",
                "arrow-up-right",
                S::System,
                P::OpenInDefaultApp,
            ),
            item(
                "Show in system explorer",
                "arrow-up-right",
                S::System,
                P::ShowInSystemExplorer,
            ),
            item(
                "Reveal file in navigation",
                "folder-open",
                S::System,
                P::RevealInNavigation,
            ),
            item("Delete file", "trash-2", S::Danger, P::Delete),
        ])
        .collect(),
    }
}

/// The tab header's menu, as the 2026-11-09 probe drew it. Left out for
/// want of anything behind them: **Link with tab...** (no linked panes,
/// and the probe saw no trace of what it starts), a markdown tab's two
/// view-mode rows (their labels in the other modes were not captured),
/// **Export to PDF...**, a canvas's **Export as image**, **Open version
/// history** and the **Open linked view** submenu; Copy path keeps the one
/// flat row the other menus give it.
fn tab_items(path: &str, ix: usize, tabs: usize, pinned: bool, active: bool) -> Vec<Item> {
    use Pick as P;
    use Section as S;
    let mut out = vec![item("Close", "x", S::Close, P::CloseTab)];
    if tabs > 1 {
        out.push(item("Close others", "x", S::Close, P::CloseOthers));
        if ix + 1 < tabs {
            out.push(item("Close tabs after", "x", S::Close, P::CloseAfter));
        }
        out.push(item("Close all", "x", S::Close, P::CloseAll));
    }
    out.push(item(
        if pinned { "Unpin" } else { "Pin" },
        "pin",
        S::Pane,
        P::TogglePin,
    ));
    out.push(item(
        "Move to new window",
        "picture-in-picture",
        S::Open,
        P::MoveToNewWindow,
    ));
    if !active {
        return out;
    }
    out.push(item(
        "Split right",
        "separator-vertical",
        S::Open,
        P::SplitRight,
    ));
    out.push(item(
        "Split down",
        "separator-horizontal",
        S::Open,
        P::SplitDown,
    ));
    if path.is_empty() {
        return out;
    }
    let markdown = path.to_ascii_lowercase().ends_with(".md");
    for row in items(&Target::View(path.to_string())) {
        if row.pick == P::SplitRight || row.pick == P::SplitDown {
            continue;
        }
        // Add file property follows Move, which a markdown tab always has.
        let after_move = row.pick == P::Move;
        let before_copy = row.pick == P::CopyPath;
        if markdown && before_copy {
            out.push(item("Find...", "file-search", S::Find, P::Find));
            out.push(item("Replace...", "file-search", S::Find, P::Replace));
        }
        out.push(row);
        if markdown && after_move {
            out.push(item(
                "Add file property",
                "circle-plus",
                S::Action,
                P::AddFileProperty,
            ));
        }
    }
    out
}

/// One row, enabled.
fn item(label: &'static str, icon: &'static str, section: Section, pick: Pick) -> Item {
    Item {
        warning: false,
        checked: false,
        submenu: Vec::new(),
        label: SharedString::new_static(label),
        icon,
        section,
        pick,
        disabled: false,
    }
}

/// The editor menu for a click, in Obsidian's order.
///
/// Four of the captured rows open submenus nobody has read — **Format**,
/// **Paragraph**, **Insert** and, on a wikilink, **Copy path** — so the
/// three text ones are left out whole and Copy path keeps the one flat row
/// the rest of TFT's menus give it. Three more are left out for want of
/// a command: **Edit link** and **Edit tag** (no inline rename of a link or
/// a tag) and
/// **Start presentation** (no presentation mode). Dropping the only row of a section drops its
/// separator with it, which is why the pair the capture has between Start
/// presentation and the system rows is one separator here.
fn editor_items(click: &EditorClick) -> Vec<Item> {
    use Pick as P;
    use Section as S;
    let on_link = matches!(click.spot, Spot::Link(_));
    let mut out = Vec::new();
    if on_link {
        out.push(item(
            "Open in new tab",
            "file-plus",
            S::Open,
            P::OpenInNewTab,
        ));
        out.push(item(
            "Open to the right",
            "separator-vertical",
            S::Open,
            P::OpenToRight,
        ));
        out.push(item(
            "Open in new window",
            "picture-in-picture-2",
            S::Open,
            P::OpenInNewWindow,
        ));
    }
    out.push(item("Add link", "link", S::SelectionLink, P::AddLink));
    out.push(item(
        "Add external link",
        "external-link",
        S::SelectionLink,
        P::AddExternalLink,
    ));
    if !click.selection.is_empty() {
        // Obsidian quotes the selected text in curly quotes.
        out.push(Item {
            label: format!("Search for \u{201c}{}\u{201d}", click.selection).into(),
            icon: "search",
            section: S::Selection,
            pick: P::SearchForSelection,
            disabled: false,
            warning: false,
            checked: false,
            submenu: Vec::new(),
        });
    }
    // `lucide-box-select` is Lucide's deprecated alias of `square-dashed`,
    // from its own alias data, the way `lucide-edit-3` is `pen-line`.
    for (label, icon, pick, needs_selection) in [
        ("Cut", "scissors", P::Cut, true),
        ("Copy", "copy", P::Copy, true),
        ("Paste", "clipboard-check", P::Paste, false),
        (
            "Paste as plain text",
            "clipboard-type",
            P::PastePlain,
            false,
        ),
        ("Select all", "square-dashed", P::SelectAll, false),
    ] {
        out.push(Item {
            disabled: needs_selection && !click.selected,
            ..item(label, icon, S::Clipboard, pick)
        });
    }
    if on_link {
        out.push(item("Copy path", "clipboard", S::Unsectioned, P::CopyPath));
        out.push(item("Rename...", "pen-line", S::Action, P::Rename));
        out.push(item("Move file to...", "folder-tree", S::Action, P::Move));
        out.push(item(
            "Open in default app",
            "arrow-up-right",
            S::System,
            P::OpenInDefaultApp,
        ));
        out.push(item(
            "Show in system explorer",
            "arrow-up-right",
            S::System,
            P::ShowInSystemExplorer,
        ));
        out.push(item(
            "Reveal file in navigation",
            "folder-open",
            S::System,
            P::RevealInNavigation,
        ));
    }
    out
}

/// How many separators sit above an item given the section above it: one
/// wherever the section changes, and two above [`Section::Danger`] for the
/// empty section Obsidian puts there.
pub fn separators_above(previous: Option<Section>, section: Section) -> usize {
    match previous {
        None => 0,
        Some(p) if p == section => 0,
        Some(_) if section == Section::Danger => 2,
        Some(_) => 1,
    }
}

/// The menu as a reader sees it: a `---` per separator, then each item's
/// label. Comparing this against the spec's own tables is what the tests
/// do, and what the control socket reports.
pub fn drawn(target: &Target) -> Vec<String> {
    let mut out = Vec::new();
    let mut previous = None;
    for item in items(target) {
        for _ in 0..separators_above(previous, item.section) {
            out.push("---".to_string());
        }
        previous = Some(item.section);
        out.push(item.label.to_string());
    }
    out
}

/// An open menu: where it is and what it was opened on.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub position: Point<Pixels>,
    /// Whether a row's submenu is showing beside it: the pointer rests on
    /// the row, or did last.
    pub submenu: bool,
    /// Which corner of the list sits at `position`: the top-left for a
    /// right click, the top-right for a menu hung under a button at the
    /// window's right edge, the way the view header's **More options**
    /// opens right-aligned under its button.
    pub anchor: Anchor,
    pub target: Target,
}

/// The open menu as the control socket sees it: what it was opened on and
/// the rows a reader would see, `"---"` per separator. Without this a
/// harness can right-click but cannot read back what came up, which is what
/// kept the menus untestable from outside the process.
pub fn describe(menu: &Menu) -> Value {
    let target = match &menu.target {
        Target::File(_) => "file",
        Target::Folder(_) => "folder",
        Target::Blank => "blank",
        Target::Linked(_) => "linked",
        Target::Search { file_row: true, .. } => "search-file",
        Target::Search {
            file_row: false, ..
        } => "search-match",
        Target::Editor(click) => match click.spot {
            Spot::Text => "editor",
            Spot::Tag(_) => "editor-tag",
            Spot::Link(_) => "editor-link",
        },
        Target::View(_) => "view",
        Target::Tab { .. } => "tab",
    };
    let items = items(&menu.target);
    let disabled: Vec<String> = items
        .iter()
        .filter(|i| i.disabled)
        .map(|i| i.label.to_string())
        .collect();
    let mut value = json!({
        "target": target,
        "path": menu.target.path(),
        "items": drawn(&menu.target),
        "disabled": disabled,
        "submenu": menu.submenu,
    });
    // What the editor's menu was built from, which no other target has.
    if let Target::Editor(click) = &menu.target {
        value["selection"] = json!(click.selection);
        value["selected"] = json!(click.selected);
        if let Spot::Tag(tag) = &click.spot {
            value["tag"] = json!(tag);
        }
    }
    value
}

/// The menu drawn at its position over everything else: a press outside it
/// runs `on_close`; a click on a row runs `on_close` and then `on_pick` with
/// the row's [`Pick`]. The view owns the [`Menu`] and clears it in
/// `on_close`; this only draws.
#[allow(clippy::too_many_arguments)]
pub fn render<V: 'static>(
    menu: &Menu,
    theme: &Theme,
    focus: &FocusHandle,
    viewport_width: Pixels,
    cx: &mut Context<V>,
    on_close: impl Fn(&mut V, &mut Context<V>) + 'static,
    on_pick: impl Fn(&mut V, Pick, &mut Window, &mut Context<V>) + 'static,
    on_submenu: impl Fn(&mut V, bool, &mut Context<V>) + 'static,
) -> AnyElement {
    let on_close = Rc::new(on_close);
    let on_pick = Rc::new(on_pick);
    let on_submenu = Rc::new(on_submenu);
    let submenu_open = menu.submenu;
    // A submenu opens beside its row on the right, or on the left when the
    // menu stands near the window's right edge and there is no room.
    let flip_submenu = menu.position.x + px(180. + 140.) > viewport_width;
    let over_submenu = OVER_SUBMENU.with(Rc::clone);
    if !menu.submenu {
        // A menu closed under the pointer never hears the hover end.
        over_submenu.set(false);
    }
    let mut list = menu_box_with_submenu(
        // Obsidian's `.menu`: `--menu-background` inside a
        // `--menu-border-color` line, rows in `--text-normal` at the small
        // UI size, icons `--text-muted`, a warning row `--text-error`, the
        // hover wash `--background-modifier-hover` (probe of 2026-11-02).
        crate::ui::menu_panel("file-menu", theme)
            .min_w(px(180.))
            .py_1(),
        focus,
        cx,
        {
            let on_close = on_close.clone();
            move |this, cx| on_close(this, cx)
        },
        over_submenu.clone(),
    );
    let mut previous: Option<Section> = None;
    // A menu whose rows all go without icons (the Files tree's) keeps no
    // empty icon column: the labels start at the panel's padding.
    let any_icons = items(&menu.target).iter().any(|i| !i.icon.is_empty());
    let hotkey = |pick: Pick| {
        if !matches!(menu.target, Target::File(_) | Target::Folder(_) | Target::Blank) {
            return None;
        }
        let id = match pick {
            Pick::NewNote => Some("file-explorer:new-file"),
            Pick::Rename => Some("workspace:edit-file-title"),
            _ => None,
        }?;
        cx.global::<crate::workspace::Commands>()
            .0
            .get(id)?
            .hotkeys
            .first()
            .map(tft_commands::hotkey_label)
    };
    for (ix, item) in items(&menu.target).into_iter().enumerate() {
        // Obsidian emits two separators before the danger pair but draws
        // only the first: the second is `display: none` (its rect is zero
        // in every capture, 2026-11-07 `s06`). The model keeps both, so
        // [`drawn`] still matches the DOM; the eye sees one line.
        for n in 0..separators_above(previous, item.section).min(1) {
            list = list.child(
                div()
                    .id(("file-menu-separator", ix * 2 + n))
                    .my_1()
                    .h(px(1.))
                    .w_full()
                    .bg(theme.tokens.background_modifier_border),
            );
        }
        previous = Some(item.section);
        let pick = item.pick;
        let on_close_row = on_close.clone();
        let on_pick_row = on_pick.clone();
        let has_submenu = !item.submenu.is_empty();
        let on_hover = on_submenu.clone();
        // Obsidian's `is-disabled` row: greyed, no hover, and a click on it
        // closes nothing and does nothing.
        let faded = theme.tokens.text_faint;
        let color = if item.warning {
            theme.tokens.text_error
        } else {
            theme.tokens.text_normal
        };
        let mut row = crate::ui::menu_row(("file-menu-item", ix), theme)
            .relative()
            .text_color(color)
            .when(item.disabled, |d| d.text_color(faded).opacity(0.6))
            .when(!item.disabled, |d| {
                d.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        on_hover(this, has_submenu, cx);
                    }
                }))
                .when(!has_submenu, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        on_close_row(this, cx);
                        on_pick_row(this, pick, window, cx);
                    }))
                })
            })
            // "No suggestions..." has no icon; its text keeps the column.
            .when(any_icons, |d| {
                d.child(if item.icon.is_empty() {
                    div().size(px(16.)).flex_shrink_0().into_any_element()
                } else {
                    crate::assets::icon(
                        item.icon,
                        16.,
                        if item.disabled {
                            faded
                        } else {
                            theme.tokens.text_muted
                        },
                    )
                    .into_any_element()
                })
            })
            .child(div().flex_1().child(item.label.clone()))
            .when_some(hotkey(item.pick), |d, hint| {
                d.child(div().flex_shrink_0().text_color(theme.tokens.text_muted).child(hint))
            })
            .when(item.checked, |d| {
                d.child(crate::assets::icon("check", 14., theme.tokens.text_muted))
            })
            .when(has_submenu, |d| {
                d.child(crate::assets::icon(
                    "chevron-right",
                    14.,
                    theme.tokens.text_muted,
                ))
            });
        // The submenu stands beside its row, open while the pointer rested
        // on that row last; its rows pick like the menu's own.
        if has_submenu && submenu_open {
            let mut sub = crate::ui::menu_panel(("file-submenu", ix), theme)
                .absolute()
                .when(!flip_submenu, |d| d.left(relative(1.)))
                .when(flip_submenu, |d| d.right(relative(1.)))
                .top(px(-6.))
                .min_w(px(126.))
                .py_1()
                .occlude()
                .on_hover({
                    let over_submenu = over_submenu.clone();
                    move |hovered: &bool, _, _| over_submenu.set(*hovered)
                });
            for (sx, subitem) in item.submenu.iter().enumerate() {
                let pick = subitem.pick;
                let on_close = on_close.clone();
                let on_pick = on_pick.clone();
                sub = sub.child(
                    crate::ui::menu_row(("file-submenu-item", ix * 64 + sx), theme)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            on_close(this, cx);
                            on_pick(this, pick, window, cx);
                        }))
                        .child(crate::assets::icon(
                            subitem.icon,
                            16.,
                            theme.tokens.text_muted,
                        ))
                        .child(div().flex_1().child(subitem.label.clone()))
                        .when(subitem.checked, |d| {
                            d.child(crate::assets::icon("check", 14., theme.tokens.text_muted))
                        }),
                );
            }
            row = row.child(sub);
        }
        list = list.child(row);
    }
    deferred(
        anchored()
            .position(menu.position)
            .anchor(menu.anchor)
            .snap_to_window_with_margin(px(8.))
            .child(list),
    )
    .with_priority(1)
    .into_any_element()
}

// ----- "Show in system explorer" --------------------------------------------

/// A `file://` URL for an absolute path, every byte outside RFC 3986's
/// unreserved set and the `/` percent-encoded — spaces, `#`, `?`, `%` and
/// anything non-ASCII included — so a file manager reads the name back
/// exactly, which `ShowItems` needs since it selects the file by it. Only
/// the Linux call wants one, so a Windows path's drive and backslashes are
/// not special-cased here.
pub fn file_url(absolute: &Path) -> String {
    let text = absolute.to_string_lossy();
    let mut url = String::from("file://");
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(byte as char)
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

/// One command line that selects a file in the desktop's file manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reveal {
    pub program: &'static str,
    pub args: Vec<String>,
}

/// The command lines that select `absolute` in the desktop's file manager,
/// in the order to try them: each but the last must spawn and exit 0 or the
/// next is tried; the last is spawned and let go, the way "Open in default
/// app" is, because the handler it starts outlives this process.
///
/// Linux asks the session bus's `org.freedesktop.FileManager1.ShowItems`
/// through `dbus-send`, which every mainstream file manager answers, and
/// falls back to `xdg-open` on the containing folder where nothing does.
/// macOS has `open -R`; Windows has `explorer /select,<path>`, one argument
/// with the comma attached. No crate is added for this.
pub fn reveal_commands(absolute: &Path) -> Vec<Reveal> {
    if cfg!(target_os = "macos") {
        vec![Reveal {
            program: "open",
            args: vec!["-R".into(), absolute.display().to_string()],
        }]
    } else if cfg!(target_os = "windows") {
        vec![Reveal {
            program: "explorer",
            args: vec![format!("/select,{}", absolute.display())],
        }]
    } else {
        linux_reveal_commands(absolute)
    }
}

fn linux_reveal_commands(absolute: &Path) -> Vec<Reveal> {
    let parent: PathBuf = absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| absolute.to_path_buf());
    vec![
        Reveal {
            program: "dbus-send",
            args: vec![
                "--session".into(),
                "--print-reply".into(),
                "--dest=org.freedesktop.FileManager1".into(),
                "/org/freedesktop/FileManager1".into(),
                "org.freedesktop.FileManager1.ShowItems".into(),
                format!("array:string:{}", file_url(absolute)),
                "string:".into(),
            ],
        },
        Reveal {
            program: "xdg-open",
            args: vec![parent.display().to_string()],
        },
    ]
}

/// Runs [`reveal_commands`] in order and reports the error of the last,
/// if it could not be spawned either. Blocking: the D-Bus call waits for the
/// file manager's reply, so callers run this off the UI thread.
pub fn show_in_system_explorer(absolute: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let commands = reveal_commands(absolute);
    let last = commands.len().saturating_sub(1);
    let mut err = String::new();
    for (ix, reveal) in commands.into_iter().enumerate() {
        let mut command = Command::new(reveal.program);
        command
            .args(&reveal.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if ix == last {
            return command
                .spawn()
                .map(drop)
                .map_err(|e| format!("{}: {e}", reveal.program));
        }
        match command.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => err = format!("{} exited with {status}", reveal.program),
            Err(e) => err = format!("{}: {e}", reveal.program),
        }
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource;

    /// The Files tree has one menu: a file row, a folder row and the empty
    /// space below them all draw the same rows in the same order, with no
    /// icons, and a row that cannot act on the clicked target is disabled.
    /// This is TFT's own choice (Zed's project panel does the same), not the
    /// per-target tables Obsidian draws.
    #[test]
    fn the_tree_menus_are_one_list_with_disabled_rows() {
        let want = [
            "New File",
            "New folder",
            "New canvas",
            "---",
            "Open in new tab",
            "Open to the right",
            "Open in new window",
            "---",
            "Duplicate",
            "Move to...",
            "Search in folder",
            "---",
            "Copy Path",
            "Copy Relative Path",
            "---",
            "Open in default app",
            reveal_label(),
            "---",
            "---",
            "Rename...",
            "Delete",
        ];
        let file = Target::File("Notes/Plan.md".into());
        let folder = Target::Folder("Notes".into());
        let blank = Target::Blank;
        for target in [file.clone(), folder.clone(), blank.clone()] {
            assert_eq!(drawn(&target), want, "{target:?}");
            assert!(items(&target).iter().all(|i| i.icon.is_empty()));
        }
        let disabled = |target: &Target| -> Vec<Pick> {
            items(target)
                .into_iter()
                .filter(|i| i.disabled)
                .map(|i| i.pick)
                .collect()
        };
        // Every row has a target on a file: creation and search act on the
        // folder above it, so none of them is disabled.
        assert!(disabled(&file).is_empty());
        assert!(disabled(&folder).contains(&Pick::OpenInNewTab));
        assert!(!disabled(&folder).contains(&Pick::NewNote));
        assert!(disabled(&blank).contains(&Pick::CopyPath));
        assert!(!disabled(&blank).contains(&Pick::NewFolder));
    }

    /// The 2026-10-22 probe's `s13-context-menu`, read off a Links row of
    /// the Outgoing links pane (`results/2026-10-22-outgoing-links.json`),
    /// whole: Rename sits with Move under one separator rather
    /// than in a `danger` pair, there is no Make a copy and no Delete, and
    /// "Reveal file in navigation" closes the list.
    #[test]
    fn a_linked_rows_menu_is_the_probes_list() {
        assert_eq!(
            drawn(&Target::Linked("Notes/Plan.md".into())),
            [
                "Open in new tab",
                "Open to the right",
                "Open in new window",
                "---",
                "Rename...",
                "Move file to...",
                "---",
                "Copy path",
                "---",
                "Open in default app",
                "Show in system explorer",
                "Reveal file in navigation",
            ]
        );
    }

    /// A Search pane result's two menus, as Obsidian 1.15.1 drew them on a
    /// real right click (`2026-11-16-bases-cells-search-menu.json`, `s07`):
    /// the file row's ends "Delete file" in the danger pair, then "Copy
    /// path"; a match row's is the same without Delete, and both act on
    /// the file.
    #[test]
    fn a_search_results_menus_are_the_probes_lists() {
        let match_row = [
            "Open in new tab",
            "Open to the right",
            "Open in new window",
            "---",
            "Rename...",
            "Move file to...",
            "---",
            "Open in default app",
            "Show in system explorer",
            "Reveal file in navigation",
            "---",
            "Copy path",
        ];
        let target = |file_row| Target::Search {
            path: "search/One.md".into(),
            file_row,
        };
        assert_eq!(drawn(&target(false)), match_row);
        let mut file_row: Vec<&str> = match_row[..11].to_vec();
        file_row.extend(["---", "Delete file", "---", "Copy path"]);
        assert_eq!(drawn(&target(true)), file_row);
        assert_eq!(target(false).path(), "search/One.md");
        let delete = items(&target(true))
            .into_iter()
            .find(|i| i.pick == Pick::Delete)
            .expect("the file row deletes");
        assert!(delete.warning);
        assert_eq!(delete.icon, "trash-2");
        assert!(!items(&target(false)).iter().any(|i| i.pick == Pick::Delete));
    }

    /// The view header's More options over a leaf, minus the two `view`
    /// rows and their separator: the splits and the window row, Rename with
    /// Move, Copy path, the three system rows, and "Delete
    /// file" after the danger pair.
    #[test]
    fn a_leafs_more_options_is_the_probes_list_without_the_rows_it_cannot_run() {
        assert_eq!(
            drawn(&Target::View("Shelf/Rocks.md".into())),
            [
                "Split right",
                "Split down",
                "Open in new window",
                "---",
                "Rename...",
                "Move file to...",
                "---",
                "Copy path",
                "---",
                "Open in default app",
                "Show in system explorer",
                "Reveal file in navigation",
                "---",
                "---",
                "Delete file",
            ]
        );
        assert_eq!(
            Target::View("Shelf/Rocks.md".into()).path(),
            "Shelf/Rocks.md"
        );
    }

    fn tab(path: &str, ix: usize, tabs: usize, pinned: bool, active: bool) -> Target {
        Target::Tab {
            group: 1,
            ix,
            path: path.into(),
            tabs,
            pinned,
            active,
        }
    }

    /// The tab header's menu over the middle of three markdown tabs, the
    /// active one (`2026-11-09-tab-header-menu.json`, cases 1 and 2), minus
    /// the rows `tab_items` names as left out — which takes the `view`
    /// section's separator with them.
    #[test]
    fn a_markdown_tabs_menu_is_the_probes_list_without_the_rows_it_cannot_run() {
        let target = tab("Notes/Alpha.md", 1, 3, false, true);
        assert_eq!(
            drawn(&target),
            [
                "Close",
                "Close others",
                "Close tabs after",
                "Close all",
                "---",
                "Pin",
                "---",
                "Move to new window",
                "Split right",
                "Split down",
                "Open in new window",
                "---",
                "Rename...",
                "Move file to...",
                "Add file property",
                "---",
                "Find...",
                "Replace...",
                "---",
                "Copy path",
                "---",
                "Open in default app",
                "Show in system explorer",
                "Reveal file in navigation",
                "---",
                "---",
                "Delete file",
            ]
        );
        assert_eq!(target.path(), "Notes/Alpha.md");
    }

    /// The last of two tabs has no Close tabs after; a canvas has no
    /// markdown rows; a pinned tab's row reads Unpin.
    #[test]
    fn a_last_canvas_tab_has_no_close_after_and_no_markdown_rows() {
        assert_eq!(
            drawn(&tab("Board.canvas", 1, 2, true, true)),
            [
                "Close",
                "Close others",
                "Close all",
                "---",
                "Unpin",
                "---",
                "Move to new window",
                "Split right",
                "Split down",
                "Open in new window",
                "---",
                "Rename...",
                "Move file to...",
                "---",
                "Copy path",
                "---",
                "Open in default app",
                "Show in system explorer",
                "Reveal file in navigation",
                "---",
                "---",
                "Delete file",
            ]
        );
    }

    /// An inactive tab's menu stops after Move to new window, and an empty
    /// tab's after the splits (cases 2 and 4).
    #[test]
    fn an_inactive_tab_and_an_empty_tab_have_the_short_menus() {
        assert_eq!(
            drawn(&tab("Alpha.md", 0, 3, false, false)),
            [
                "Close",
                "Close others",
                "Close tabs after",
                "Close all",
                "---",
                "Pin",
                "---",
                "Move to new window",
            ]
        );
        assert_eq!(
            drawn(&tab("", 0, 1, false, true)),
            [
                "Close",
                "---",
                "Pin",
                "---",
                "Move to new window",
                "Split right",
                "Split down",
            ]
        );
    }

    fn drawn_of(items: &[Item]) -> Vec<String> {
        let mut out = Vec::new();
        let mut previous = None;
        for item in items {
            for _ in 0..separators_above(previous, item.section) {
                out.push("---".to_string());
            }
            previous = Some(item.section);
            out.push(item.label.to_string());
        }
        out
    }

    fn plain_click() -> Target {
        Target::Editor(EditorClick {
            spot: Spot::Text,
            selection: String::new(),
            selected: false,
        })
    }

    /// The rows of the editor's clipboard section, which every editor menu
    /// carries, with the labels Obsidian drew disabled marked.
    fn clipboard_rows(target: &Target) -> Vec<(String, bool)> {
        items(target)
            .into_iter()
            .filter(|i| i.section == Section::Clipboard)
            .map(|i| (i.label.to_string(), i.disabled))
            .collect()
    }

    /// The plain menu of `s01-editor-menu-live`, minus the three submenu
    /// rows nobody has opened: the two link rows, one separator where
    /// Format's section was, then the clipboard five.
    #[test]
    fn the_editor_menu_is_references_plain_list_without_the_submenus() {
        assert_eq!(
            drawn(&plain_click()),
            [
                "Add link",
                "Add external link",
                "---",
                "Cut",
                "Copy",
                "Paste",
                "Paste as plain text",
                "Select all",
            ]
        );
    }

    /// "Cut and Copy are drawn disabled with no selection" -- and enabled
    /// with one; Paste, Paste as plain text and Select all never are.
    #[test]
    fn cut_and_copy_are_disabled_only_while_nothing_is_selected() {
        assert_eq!(
            clipboard_rows(&plain_click()),
            [
                ("Cut".to_string(), true),
                ("Copy".to_string(), true),
                ("Paste".to_string(), false),
                ("Paste as plain text".to_string(), false),
                ("Select all".to_string(), false),
            ]
        );
        let live = Target::Editor(EditorClick {
            spot: Spot::Text,
            selection: "paragraph with".into(),
            selected: true,
        });
        assert_eq!(
            clipboard_rows(&live)
                .into_iter()
                .filter(|(_, d)| *d)
                .count(),
            0,
            "with a selection no clipboard row is disabled"
        );
    }

    /// With a selection the `selection` section opens with Obsidian's
    /// quoted Search row.
    #[test]
    fn a_selection_adds_the_quoted_search_row() {
        let target = Target::Editor(EditorClick {
            spot: Spot::Text,
            selection: "paragraph with".into(),
            selected: true,
        });
        assert_eq!(
            drawn(&target),
            [
                "Add link",
                "Add external link",
                "---",
                "Search for \u{201c}paragraph with\u{201d}",
                "---",
                "Cut",
                "Copy",
                "Paste",
                "Paste as plain text",
                "Select all",
            ]
        );
    }

    /// A right click outside a selection: the Search row is built from the
    /// selection as it was, and Cut and Copy from the one that did not
    /// survive -- both at once, which is what the probe read back.
    #[test]
    fn a_click_outside_the_selection_keeps_the_search_row_and_disables_cut() {
        let target = Target::Editor(EditorClick {
            spot: Spot::Text,
            selection: "paragraph with".into(),
            selected: false,
        });
        assert!(drawn(&target).contains(&"Search for \u{201c}paragraph with\u{201d}".to_string()));
        assert_eq!(
            clipboard_rows(&target)[..2],
            [("Cut".to_string(), true), ("Copy".to_string(), true)]
        );
    }

    /// "On a tag the Format row is replaced by Edit tag; the rest is the
    /// plain menu." Edit tag has no command behind it and Format is a
    /// submenu, so both are out and a tag's rows are the plain menu's --
    /// the one place the tag's name still reaches is [`describe`].
    #[test]
    fn a_tag_draws_the_plain_menu_until_edit_tag_has_a_command() {
        let tag = Target::Editor(EditorClick {
            spot: Spot::Tag("project".into()),
            selection: String::new(),
            selected: false,
        });
        assert_eq!(drawn(&tag), drawn(&plain_click()));
        assert_eq!(tag.path(), "", "a tag names no file");
    }

    /// `s01-editor-menu-live`'s `onWikilink`, minus Edit link, Paragraph,
    /// Insert and Start presentation: the three Open rows above the text
    /// rows, then Copy path, the action three and the
    /// system three. Dropping Start presentation empties the `view`
    /// section, so the capture's two consecutive separators are one here.
    #[test]
    fn a_wikilink_wraps_the_text_rows_in_the_file_menu() {
        let target = Target::Editor(EditorClick {
            spot: Spot::Link("Notes/Plan.md".into()),
            selection: String::new(),
            selected: false,
        });
        assert_eq!(
            drawn(&target),
            [
                "Open in new tab",
                "Open to the right",
                "Open in new window",
                "---",
                "Add link",
                "Add external link",
                "---",
                "Cut",
                "Copy",
                "Paste",
                "Paste as plain text",
                "Select all",
                "---",
                "Copy path",
                "---",
                "Rename...",
                "Move file to...",
                "---",
                "Open in default app",
                "Show in system explorer",
                "Reveal file in navigation",
            ]
        );
        assert_eq!(target.path(), "Notes/Plan.md");
    }

    /// The caret rule: a point inside the selection keeps it, a point
    /// outside moves the caret there, and with nothing selected every point
    /// moves it -- which is the right click that placed the caret on the
    /// word it landed on.
    #[test]
    fn a_right_click_keeps_a_selection_it_lands_in_and_moves_the_caret_otherwise() {
        let selection = 6..20;
        assert_eq!(right_click_caret(&selection, 12), None);
        assert_eq!(right_click_caret(&selection, 6), None, "its head");
        assert_eq!(right_click_caret(&selection, 20), None, "its tail");
        assert_eq!(right_click_caret(&selection, 5), Some(5));
        assert_eq!(right_click_caret(&selection, 35), Some(35));
        // No selection: the caret goes to the click wherever that is, the
        // click's own offset included.
        assert_eq!(right_click_caret(&(0..0), 30), Some(30));
        assert_eq!(right_click_caret(&(30..30), 30), Some(30));
    }

    /// Obsidian's file row has no creation items, and a row away from the
    /// Files view none either.
    #[test]
    fn a_file_row_offers_nothing_that_creates_a_file() {
        for target in [
            Target::File("Plan.md".into()),
            Target::Linked("Plan.md".into()),
        ] {
            for item in items(&target) {
                assert!(
                    !item.label.starts_with("New "),
                    "{target:?} offered {}",
                    item.label
                );
            }
        }
    }

    /// Only the Files view can be revealed in, and only its rows can delete
    /// or copy: the picks a view has no handler for are never offered to it.
    #[test]
    fn each_target_offers_only_the_picks_its_view_handles() {
        let picks = |t: &Target| items(t).iter().map(|i| i.pick).collect::<Vec<_>>();
        for target in [
            Target::File("Plan.md".into()),
            Target::Folder("Notes".into()),
            Target::Blank,
        ] {
            assert!(!picks(&target).contains(&Pick::RevealInNavigation));
        }
        let linked = picks(&Target::Linked("Plan.md".into()));
        for pick in [
            Pick::Delete,
            Pick::Duplicate,
            Pick::NewNote,
            Pick::NewFolder,
            Pick::NewCanvas,
            Pick::SearchInFolder,
        ] {
            assert!(!linked.contains(&pick), "a linked row offered {pick:?}");
        }
    }

    /// The path is the target's: the one place every item of a menu reads it
    /// from.
    #[test]
    fn every_target_carries_the_row_it_was_opened_on() {
        assert_eq!(
            Target::File("Notes/Deep/Plan.md".into()).path(),
            "Notes/Deep/Plan.md"
        );
        assert_eq!(Target::Folder("Notes/Deep".into()).path(), "Notes/Deep");
        assert_eq!(
            Target::Linked("Notes/Plan.md".into()).path(),
            "Notes/Plan.md"
        );
    }

    /// A separator where the section changes, two above `danger`, none at the
    /// top, and none between two items of one section.
    #[test]
    fn a_separator_falls_where_the_section_changes() {
        assert_eq!(separators_above(None, Section::Open), 0);
        assert_eq!(separators_above(Some(Section::Open), Section::Open), 0);
        assert_eq!(separators_above(Some(Section::Open), Section::Action), 1);
        assert_eq!(
            separators_above(Some(Section::Action), Section::Unsectioned),
            1
        );
        assert_eq!(separators_above(Some(Section::System), Section::Danger), 2);
        assert_eq!(separators_above(Some(Section::Danger), Section::Danger), 0);
    }

    /// The socket's view of a menu: the kind, the path and the drawn rows.
    #[test]
    fn describe_reports_the_target_and_the_drawn_rows() {
        let menu = Menu {
            position: Point::new(px(10.), px(20.)),
            submenu: false,
            anchor: Anchor::TopLeft,
            target: Target::Linked("Notes/Plan.md".into()),
        };
        let value = describe(&menu);
        assert_eq!(value["target"], "linked");
        assert_eq!(value["path"], "Notes/Plan.md");
        assert_eq!(value["items"][0], "Open in new tab");
        assert_eq!(value["items"][2], "Open in new window");
        assert_eq!(value["items"][3], "---");
    }

    /// The icon of every item in every menu is bundled and is the name
    /// Obsidian leaves in its DOM, or Lucide's current spelling of it where
    /// Obsidian's is an old alias: `lucide-edit` is `square-pen` and
    /// `lucide-edit-3` is `pen-line`, both from Lucide's own alias data.
    #[test]
    fn every_menu_icon_is_bundled() {
        let targets = [
            Target::File("Plan.md".into()),
            Target::Folder("Notes".into()),
            Target::Blank,
            Target::Linked("Plan.md".into()),
            plain_click(),
            spelling_click(&["world"]),
            Target::Editor(EditorClick {
                spot: Spot::Link("Plan.md".into()),
                selection: "some words".into(),
                selected: true,
            }),
        ];
        for target in targets {
            for item in items(&target) {
                let path = format!("icons/{}.svg", item.icon);
                assert!(
                    crate::assets::Assets.load(&path).unwrap().is_some(),
                    "{} wants {path}, which is not bundled",
                    item.label
                );
            }
        }
    }

    /// Spaces, the characters a URL gives meaning to, and non-ASCII bytes
    /// are percent-encoded; letters, digits, `-._~` and the slashes pass.
    #[test]
    fn a_file_url_escapes_what_a_file_manager_would_misread() {
        assert_eq!(
            file_url(Path::new("/home/me/My Vault/Notes/Plan.md")),
            "file:///home/me/My%20Vault/Notes/Plan.md"
        );
        assert_eq!(
            file_url(Path::new("/v/Café #1/100%?.md")),
            "file:///v/Caf%C3%A9%20%231/100%25%3F.md"
        );
        assert_eq!(
            file_url(Path::new("/v/a-b_c.d~e/f")),
            "file:///v/a-b_c.d~e/f"
        );
    }

    /// The D-Bus call is the first thing tried on Linux and `xdg-open` on
    /// the containing folder the last: the ShowItems argument carries the
    /// file's URL as one `array:string:` and an empty startup id.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_asks_file_manager1_first_and_opens_the_folder_last() {
        let commands = reveal_commands(Path::new("/home/me/My Vault/Notes/Plan.md"));
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].program, "dbus-send");
        assert_eq!(
            commands[0].args,
            [
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                "array:string:file:///home/me/My%20Vault/Notes/Plan.md",
                "string:",
            ]
        );
        assert_eq!(commands[1].program, "xdg-open");
        assert_eq!(commands[1].args, ["/home/me/My Vault/Notes"]);
    }

    /// `open -R` on macOS; `explorer /select,<path>` on Windows, the comma
    /// attached because `explorer` splits its one argument on it.
    #[test]
    fn the_other_desktops_get_their_one_command() {
        let path = Path::new("/v/Plan.md");
        let commands = reveal_commands(path);
        if cfg!(target_os = "macos") {
            assert_eq!(
                commands,
                [Reveal {
                    program: "open",
                    args: vec!["-R".into(), "/v/Plan.md".into()]
                }]
            );
        } else if cfg!(target_os = "windows") {
            assert_eq!(commands.len(), 1);
            assert_eq!(commands[0].program, "explorer");
            assert!(commands[0].args[0].starts_with("/select,"));
        } else {
            assert_eq!(commands.last().unwrap().program, "xdg-open");
        }
    }
}
