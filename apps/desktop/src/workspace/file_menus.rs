//! Desktop context-menu routing, file actions, and menu rendering.

use super::Workspace;
use crate::file_menu::{self, Pick};
use crate::reading::LinkTarget;
use crate::theme::Theme;
use gpui::{Anchor, ClipboardItem, Context, Pixels, Window};
use tft_app::SplitDirection;
use tft_text::LinkSyntax;

/// One file menu's target, keyboard handoff, and owning window.
pub(super) struct FileMenuState {
    pub(super) menu: Option<file_menu::Menu>,
    pub(super) focus: file_menu::MenuFocus,
    pub(super) window: Option<u64>,
}

impl FileMenuState {
    pub(super) fn new(cx: &mut Context<Workspace>) -> Self {
        Self {
            menu: None,
            focus: file_menu::MenuFocus::new(cx),
            window: None,
        }
    }
}

impl Workspace {
    /// A right click on something that stands for a file away from the
    /// Files view -- an Outgoing links row (a Links row, a
    /// mentions group, or a match, which belongs to its group's file), or a
    /// link in the reading view: the file menu, in the list Obsidian gives
    /// rows away from the Files view ([`file_menu::Target::Linked`]).
    pub(super) fn open_file_menu(
        &mut self,
        path: String,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.file_menu_state.menu = Some(file_menu::Menu {
            position,
            submenu: false,
            anchor: Anchor::TopLeft,
            target: file_menu::Target::Linked(path),
        });
        cx.notify();
    }

    pub(super) fn close_file_menu(&mut self, cx: &mut Context<Self>) {
        if self.file_menu_state.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The vault file a wikilink's text resolves to, from the note it was
    /// written in; `None` for a dangling link.
    pub(super) fn link_path(&self, body: &str) -> Option<String> {
        let state = self.vault_session.state.as_ref()?;
        let source = self.active_group_state.current.clone().unwrap_or_default();
        state.cache().resolve(body, &source).map(str::to_string)
    }

    /// A right click in the editor, which has placed the caret already: the
    /// editor's context menu, at the click. What it landed in decides the
    /// rows — a wikilink that names a file wraps the file menu around them,
    /// a tag reads as a tag, and everything else (plain text, an external
    /// link, a dangling wikilink) is the plain menu.
    pub(super) fn open_editor_menu(
        &mut self,
        position: gpui::Point<Pixels>,
        selection: String,
        selected: bool,
        link: Option<LinkTarget>,
        cx: &mut Context<Self>,
    ) {
        let spot = match link {
            // A link to a note that is not there yet names no file for
            // Rename, Move or Copy path to act on, so it reads as text.
            Some(LinkTarget::Internal(body)) => match self.link_path(&body) {
                Some(path) => file_menu::Spot::Link(path),
                None => file_menu::Spot::Text,
            },
            Some(LinkTarget::Tag(tag)) => file_menu::Spot::Tag(tag),
            // An external link raised the plain menu in the probe, and a
            // footnote reference was not probed; both read as text.
            _ => file_menu::Spot::Text,
        };
        self.file_menu_state.menu = Some(file_menu::Menu {
            position,
            submenu: false,
            anchor: Anchor::TopLeft,
            target: file_menu::Target::Editor(file_menu::EditorClick {
                spot,
                selection,
                selected,
            }),
        });
        cx.notify();
    }

    /// A right click on a link in the reading view: the file menu, the one
    /// menu that view raises. A link to a note that is not there yet names
    /// no file, so it raises nothing rather than a menu of rows that would
    /// act on nothing.
    pub(super) fn open_reading_menu(
        &mut self,
        target: LinkTarget,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let LinkTarget::Internal(body) = target else {
            return;
        };
        let Some(path) = self.link_path(&body) else {
            return;
        };
        self.open_file_menu(path, position, cx);
    }

    /// What a pick on that menu does: the handlers the explorer's events
    /// already route to, plus the two rows the explorer's menu has no twin
    /// of, plus the editor menu's own rows.
    pub(super) fn file_menu_pick(
        &mut self,
        pick: Pick,
        target: &file_menu::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = target.path().to_string();
        match pick {
            Pick::CloseTab
            | Pick::CloseOthers
            | Pick::CloseAfter
            | Pick::CloseAll
            | Pick::TogglePin
            | Pick::MoveToNewWindow => {
                if let file_menu::Target::Tab { group, ix, .. } = target {
                    self.tab_menu_pick(pick, *group, *ix, window, cx);
                }
            }
            Pick::AddFileProperty => self.run_command("markdown:add-metadata-property", window, cx),
            Pick::Find => self.run_command("editor:open-search", window, cx),
            Pick::Replace => self.run_command("editor:open-search-replace", window, cx),
            Pick::OpenInNewTab => self.open_link_in_new_tab(path, None, cx),
            Pick::OpenToRight => self.open_to_right(path, window, cx),
            // The view header's "Open in new window" copies the leaf, as
            // `workspace:open-in-new-window` does; a row's opens its file.
            Pick::OpenInNewWindow => match target {
                file_menu::Target::View(_) | file_menu::Target::Tab { .. } => {
                    self.open_in_new_window(window, cx)
                }
                _ => self.open_path_in_new_window(path, window, cx),
            },
            // The view header's splits act on the group the header belongs
            // to, which the button made active before the menu opened.
            Pick::SplitRight => self.split(SplitDirection::Vertical, window, cx),
            Pick::SplitDown => self.split(SplitDirection::Horizontal, window, cx),
            // "Delete file" on the view header's menu: the same prompt the
            // Files view's Delete raises. The explorer's own menu is not
            // drawn here, so this is the one path that reaches it.
            Pick::Delete => {
                if !path.is_empty() {
                    self.start_delete(&path, window, cx)
                }
            }
            Pick::Rename => self.start_rename(&path, window, cx),
            Pick::Move => self.start_move(&path, window, cx),
            Pick::CopyPath => self.copy_path(&path, cx),
            Pick::CopyAbsolutePath => self.copy_absolute_path(&path, cx),
            Pick::OpenInDefaultApp => self.open_in_default_app(&path, cx),
            Pick::ShowInSystemExplorer => self.show_in_system_explorer(&path, cx),
            Pick::RevealInNavigation => self.reveal_in_navigation(&path, window, cx),
            // The editor menu's rows, which act on the editor rather than
            // on a path. The menu only offers them where a right click in
            // the text raised it, so the editor they reach is the one that
            // was clicked.
            Pick::AddLink => self
                .active_editor()
                .update(cx, |e, cx| e.insert_link(LinkSyntax::Wikilink, window, cx)),
            Pick::AddExternalLink => self
                .active_editor()
                .update(cx, |e, cx| e.insert_link(LinkSyntax::Markdown, window, cx)),
            // The text the row was labelled with, which is the selection as
            // it stood at the click -- not whatever is selected now, which
            // the click may have taken away.
            Pick::SearchForSelection => {
                if let file_menu::Target::Editor(click) = target {
                    let query = click.selection.clone();
                    self.show_search(&query, Some(window), cx);
                }
            }
            Pick::Cut => self
                .active_editor()
                .update(cx, |e, cx| e.cut_selection(window, cx)),
            Pick::Copy => self
                .active_editor()
                .update(cx, |e, cx| e.copy_selection(cx)),
            Pick::Paste => self
                .active_editor()
                .update(cx, |e, cx| e.paste_clipboard(window, cx)),
            // The clipboard's plain text only: no HTML conversion, no files.
            Pick::PastePlain => self
                .active_editor()
                .update(cx, |e, cx| e.paste_plain(window, cx)),
            Pick::SelectAll => self
                .active_editor()
                .update(cx, |e, cx| e.select_all_text(cx)),
            // The Files view's own rows, which no menu drawn here lists.
            Pick::Duplicate
            | Pick::NewNote
            | Pick::NewFolder
            | Pick::NewCanvas
            | Pick::SearchInFolder => {}
        }
    }

    /// The open file menu, drawn over everything else at the click. It is
    /// the editor's and the reading view's menu as well as the panes', so
    /// it is drawn at the workspace's root rather than inside a dock.
    pub(super) fn render_file_menu(
        &mut self,
        here: Option<u64>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.overlay_home() != here {
            return None;
        }
        let menu = self.file_menu_state.menu.as_ref()?;
        let target = menu.target.clone();
        let focus = self.file_menu_state.focus.handle.clone();
        Some(file_menu::render(
            menu,
            theme,
            &focus,
            window.viewport_size().width,
            cx,
            |this, cx| this.close_file_menu(cx),
            move |this, pick, window, cx| this.file_menu_pick(pick, &target, window, cx),
            |this, open, cx| {
                if let Some(menu) = this.file_menu_state.menu.as_mut() {
                    if menu.submenu != open {
                        menu.submenu = open;
                        cx.notify();
                    }
                }
            },
        ))
    }
    /// The explorer menu's "Copy path": the vault-relative path, the one the app
    /// speaks. Silent, as Obsidian shows no notice for it.
    pub(super) fn copy_path(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
    }

    /// The Files tree's "Copy Path": the absolute filesystem path.
    pub(super) fn copy_absolute_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.as_ref() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        cx.write_to_clipboard(ClipboardItem::new_string(super::commands::full_path_text(
            absolute,
        )));
    }

    /// The explorer menu's "Open in default app": one command per platform, no crate.
    /// The child is spawned and let go; only a spawn failure is reported.
    pub(super) fn open_in_default_app(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        let mut command = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else if cfg!(target_os = "windows") {
            // `start` is a shell builtin, and its first quoted argument is the window
            // title, not the file.
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        } else {
            std::process::Command::new("xdg-open")
        };
        if let Err(err) = command
            .arg(&absolute)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            self.report(format!("Could not open {path}: {err}"), cx);
        }
    }

    /// The file menu's "Show in system explorer", through [`file_menu::reveal_commands`].
    /// The D-Bus call waits, so the whole run is off the UI thread.
    pub(super) fn show_in_system_explorer(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { file_menu::show_in_system_explorer(&absolute) })
                .await;
            if let Err(err) = result {
                this.update(cx, |ws, cx| {
                    ws.report(format!("Could not show {path}: {err}"), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// The file menu's "Reveal file in navigation", and `file-explorer:reveal-active-file`:
    /// show Files expanded to `path`, ring its row and give it the keyboard.
    pub(super) fn reveal_in_navigation(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_search_field(window, cx);
        self.left_dock.set_visible(true);
        self.explorer
            .update(cx, |e, cx| e.reveal_and_focus(path, window, cx));
        cx.notify();
    }

    /// The explorer menu's "Search in folder": scoped with `path:`; the trailing slash
    /// keeps `Notes/` from matching `Notes archive/`.
    pub(super) fn search_in_folder(
        &mut self,
        folder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = match folder.is_empty() {
            true => String::new(),
            false => format!("path:\"{folder}/\""),
        };
        self.show_search(&query, Some(window), cx);
    }
}
