//! GPUI event subscriptions connecting live view entities to Workspace actions.

use super::{ChooserPurpose, Modal, Workspace};
use crate::canvas::{CanvasEvent, CanvasView};
use crate::chooser::{Chooser, ChooserEvent};
use crate::copy_results::{CopyResults, CopyResultsEvent};
use crate::editor::{Editor, EditorEvent};
use crate::explorer::{ExplorerEvent, FileExplorer};
use crate::export_modals::{ImageExport, ImageExportEvent, PdfExport, PdfExportEvent};
use crate::file_menu;
use crate::graph::{GraphEvent, GraphView};
use crate::pdf::view::{PdfEvent, PdfView};
use crate::prompt::{Prompt, PromptEvent};
use crate::reading::{PropertyItemRemoved, ReadingEvent, ReadingView, TaskToggled};
use crate::search::{SearchEvent, SearchPane};
use crate::settings::{SettingsEvent, SettingsView};
use crate::switcher::{QuickSwitcher, SwitcherEvent};
use gpui::{Anchor, ClipboardItem, Context, Entity, Subscription, Window};
use tft_app::command_palette::{CommandPalette, PaletteEvent};
use tft_app::FileOperationRequest;

/// Connects every Workspace-owned view and panel entity to the Workspace event handlers.
pub(super) fn workspace_subscriptions(
    explorer: &Entity<FileExplorer>,
    search: &Entity<SearchPane>,
    switcher: &Entity<QuickSwitcher>,
    command_palette: &Entity<CommandPalette>,
    chooser: &Entity<Chooser>,
    settings: &Entity<SettingsView>,
    prompt: &Entity<Prompt>,
    copy_results: &Entity<CopyResults>,
    pdf_export: &Entity<PdfExport>,
    image_export: &Entity<ImageExport>,
    release_notes: &Entity<ReadingView>,
    editor: &Entity<Editor>,
    reading: &Entity<ReadingView>,
    canvas: &Entity<CanvasView>,
    pdf: &Entity<PdfView>,
    graph: &Entity<GraphView>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Vec<Subscription> {
    let mut subscriptions = vec![
        cx.subscribe_in(graph, window, |this, _, event: &GraphEvent, window, cx| {
            match event {
                GraphEvent::Open(path) => {
                    let back = this.active_group_state.mode_before_graph;
                    this.set_mode(back, window, cx);
                    this.open_file(path.clone(), None, cx);
                }
                // A graph node previews its file on Mod by default (observed).
                GraphEvent::Hover(Some(hover)) => {
                    let where_ = this.window_of_group(this.groups[this.active_group].state.id);
                    this.row_hover(
                        crate::hover_preview::Source::Graph,
                        where_,
                        crate::hover_preview::exact(&hover.path),
                        hover.path.clone(),
                        true,
                        hover.anchor,
                        hover.modifiers,
                        cx,
                    )
                }
                GraphEvent::Hover(None) => {
                    this.source_left(crate::hover_preview::Source::Graph, cx)
                }
            }
        }),
        cx.subscribe_in(
            explorer,
            window,
            |this, _, event: &ExplorerEvent, window, cx| match event {
                ExplorerEvent::SortChanged(sort) => {
                    this.layout_persistence.set_file_tree_sort(*sort);
                    this.save_layout(cx);
                }
                ExplorerEvent::FoldersExpanded(folders) => {
                    this.layout_persistence.set_file_tree_expanded(folders);
                    this.save_layout(cx);
                }
                ExplorerEvent::Open(path) => {
                    // A row's click or Enter hands the keyboard to the note it opened.
                    this.open_file(path.clone(), None, cx);
                    this.focus_main(window, cx);
                }
                ExplorerEvent::OpenInNewTab(path) => {
                    this.open_link_in_new_tab(path.clone(), None, cx)
                }
                ExplorerEvent::NewNote { folder } => this.dispatch_file_operation(
                    FileOperationRequest::CreateNote {
                        parent: folder.clone(),
                    },
                    window,
                    cx,
                ),
                ExplorerEvent::NewFolder { parent } => this.start_new_folder(parent, window, cx),
                // Obsidian renames a tree row on the row, not in a
                // dialog, and F2 and the menu's "Rename..." both do it
                // (the 2026-10-25 pass, steps `s09` and `s11`).
                ExplorerEvent::Rename(path) => {
                    let path = path.clone();
                    this.explorer.update(cx, |e, cx| e.start_rename(&path, cx));
                }
                ExplorerEvent::RenameTo { path, name } => {
                    this.commit_row_rename(path.clone(), name, window, cx)
                }
                ExplorerEvent::Move(path) => this.start_move(path, window, cx),
                ExplorerEvent::MoveInto { path, folder } => {
                    this.drop_into_folder(path, folder, window, cx)
                }
                ExplorerEvent::Duplicate(path) => this.duplicate(path, cx),
                ExplorerEvent::Delete(path) => this.start_delete(path, window, cx),
                ExplorerEvent::NewCanvas { folder } => this.new_canvas(folder, cx),
                ExplorerEvent::OpenToRight(path) => this.open_to_right(path.clone(), window, cx),
                ExplorerEvent::OpenInNewWindow(path) => {
                    this.open_path_in_new_window(path.clone(), window, cx)
                }
                ExplorerEvent::CopyPath(path) => this.copy_path(path, cx),
                ExplorerEvent::CopyAbsolutePath(path) => this.copy_absolute_path(path, cx),
                ExplorerEvent::OpenInDefaultApp(path) => this.open_in_default_app(path, cx),
                ExplorerEvent::ShowInSystemExplorer(path) => this.show_in_system_explorer(path, cx),
                ExplorerEvent::SearchInFolder(folder) => this.search_in_folder(folder, window, cx),
                ExplorerEvent::LeaveFocus => this.focus_main(window, cx),
                ExplorerEvent::Hover {
                    path,
                    over,
                    anchor,
                    modifiers,
                } => this.row_hover(
                    crate::hover_preview::Source::Files,
                    None,
                    crate::hover_preview::exact(path),
                    path.clone(),
                    *over,
                    *anchor,
                    *modifiers,
                    cx,
                ),
            },
        ),
        cx.subscribe_in(
            prompt,
            window,
            |this, _, event: &PromptEvent, window, cx| match event {
                PromptEvent::Submit(value) => this.prompt_submitted(value, window, cx),
                PromptEvent::Secondary => this.prompt_secondary(window, cx),
                PromptEvent::Tertiary => this.prompt_tertiary(window, cx),
                PromptEvent::Cancel => this.prompt_cancelled(window, cx),
            },
        ),
        cx.subscribe_in(
            copy_results,
            window,
            |this, _, event: &CopyResultsEvent, window, cx| match event {
                CopyResultsEvent::Copy(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    this.report("Results copied to your clipboard.", cx);
                }
                CopyResultsEvent::Close => this.close_modal(window, cx),
            },
        ),
        cx.subscribe_in(
            pdf_export,
            window,
            |this, _, event: &PdfExportEvent, window, cx| {
                this.on_pdf_export_event(event, window, cx)
            },
        ),
        cx.subscribe_in(
            image_export,
            window,
            |this, _, event: &ImageExportEvent, window, cx| {
                this.on_image_export_event(event, window, cx)
            },
        ),
        // The release notes' links: only ones to the web, which open
        // in the browser as a note's do.
        cx.subscribe(
            release_notes,
            |this, _, event: &ReadingEvent, cx| match event {
                ReadingEvent::OpenLink(target) => this.follow_link(target.clone(), false, cx),
                ReadingEvent::OpenLinkInNewTab(target) => {
                    this.follow_link(target.clone(), true, cx)
                }
                // Nothing there links to a note, so nothing previews.
                ReadingEvent::ContextMenu { .. } | ReadingEvent::Hover(_) => {}
            },
        ),
        cx.subscribe_in(
            search,
            window,
            |this, _, event: &SearchEvent, window, cx| match event {
                SearchEvent::Open(path, offset) => {
                    let existing = this.active_group_state.tabs.iter().position(|tab| {
                        tab.path
                            .as_deref()
                            .is_some_and(|open| open.eq_ignore_ascii_case(path))
                    });
                    if let Some(ix) = existing {
                        this.activate_tab(ix, cx);
                    } else {
                        this.open_in_new_tab(path.clone(), None, cx);
                    }
                    if let Some(offset) = *offset {
                        this.reveal_offset(offset, cx);
                    }
                }
                SearchEvent::CopyResults(entries) => {
                    this.open_copy_results(entries.clone(), window, cx);
                }
                SearchEvent::Dismiss => {
                    if this.active_group_state.mode == tft_app::ViewMode::Search {
                        let ix = this.active_group_state.active_tab;
                        this.close_tab(ix, cx);
                    } else {
                        this.focus_main(window, cx);
                    }
                }
                SearchEvent::StateChanged => {}
                SearchEvent::ContextMenu {
                    path,
                    file_row,
                    position,
                } => {
                    this.file_menu_state.menu = Some(file_menu::Menu {
                        position: *position,
                        submenu: false,
                        anchor: Anchor::TopLeft,
                        target: file_menu::Target::Search {
                            path: path.clone(),
                            file_row: *file_row,
                        },
                    });
                    cx.notify();
                }
            },
        ),
        cx.subscribe_in(
            switcher,
            window,
            |this, _, event: &SwitcherEvent, window, cx| match event {
                SwitcherEvent::Open(path) => {
                    this.close_modal(window, cx);
                    this.open_file(path.clone(), None, cx);
                    // The switcher can open a file inside a folded folder with
                    // the sidebar hidden. Show it there too: the row is
                    // selected and scrolled into view, while the keyboard
                    // stays on the note that just opened.
                    this.left_dock.set_visible(true);
                    this.explorer.update(cx, |e, cx| e.reveal(&path, cx));
                    cx.notify();
                }
                SwitcherEvent::Close => this.close_modal(window, cx),
            },
        ),
        cx.subscribe_in(
            command_palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| match event {
                // Closing first puts the keyboard back where the command
                // expects it; the command then runs as if its hotkey had.
                PaletteEvent::Run(id) => {
                    let id = id.clone();
                    this.close_modal(window, cx);
                    this.run_command(&id, window, cx);
                }
                PaletteEvent::Close => this.close_modal(window, cx),
            },
        ),
        cx.subscribe_in(
            settings,
            window,
            |this, _, event: &SettingsEvent, _window, cx| match event {
                SettingsEvent::Close => this.close_settings(cx),
                SettingsEvent::Changed(change) => this.change_setting(change.as_ref().clone(), cx),
            },
        ),
        cx.subscribe_in(
            chooser,
            window,
            |this, _, event: &ChooserEvent, window, cx| match event {
                ChooserEvent::Chosen(value) => {
                    let purpose = match this.overlay.modal {
                        Modal::Chooser(purpose) => purpose,
                        _ => return,
                    };
                    this.overlay.modal = Modal::None;
                    match purpose {
                        ChooserPurpose::CanvasFile => this
                            .active_canvas()
                            .update(cx, |c, cx| c.add_file_card_at_center(value, cx)),
                    }
                    this.focus_main(window, cx);
                    cx.notify();
                }
                ChooserEvent::Close => {
                    this.overlay.modal = Modal::None;
                    // Whatever the main area paints, not the editor:
                    // its handle is outside the dispatch tree in
                    // reading view and would swallow every key.
                    this.focus_main(window, cx);
                    cx.notify();
                }
            },
        ),
    ];
    // The desktop's focus moving to this window takes the active group
    // with it, as it does for a popout (see `window_activated`).
    subscriptions.push(cx.observe_window_activation(window, |ws, window, cx| {
        if window.is_window_active() {
            ws.window_activated(None, cx);
        }
    }));
    subscriptions.extend(view_subscriptions(editor, reading, canvas, pdf, cx));
    subscriptions
}

/// What the workspace listens to on a group's views; a click makes its group active
/// before the view handles it, so the views that send events are the live ones.
pub(super) fn view_subscriptions(
    editor: &Entity<Editor>,
    reading: &Entity<ReadingView>,
    canvas: &Entity<CanvasView>,
    pdf: &Entity<PdfView>,
    cx: &mut Context<Workspace>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe(pdf, |this, pdf, event: &PdfEvent, cx| match event {
            PdfEvent::Report(message) => this.report(message.clone(), cx),
            PdfEvent::CopyLink {
                subpath,
                alias,
                quote,
            } => {
                let Some(path) = pdf.read(cx).path().map(str::to_string) else {
                    return;
                };
                this.copy_pdf_link(&path, subpath, alias, quote.as_deref(), cx);
            }
        }),
        cx.subscribe(
            canvas,
            |this, canvas, event: &CanvasEvent, cx| match event {
                CanvasEvent::Edited => this.schedule_save(cx),
                CanvasEvent::OpenLink(target) => this.follow_link(target.clone(), false, cx),
                CanvasEvent::OpenLinkInNewTab(target) => this.follow_link(target.clone(), true, cx),
                CanvasEvent::Viewport(view_state) => {
                    this.record_canvas_view_state(&canvas, view_state.clone(), cx)
                }
                // The chooser needs a window to take focus, which this subscription
                // lacks; the next render opens it.
                CanvasEvent::PickFile(kind) => {
                    this.render_requests.request_canvas_pick(*kind);
                    cx.notify();
                }
            },
        ),
        cx.subscribe(
            reading,
            |this, reading, event: &ReadingEvent, cx| match event {
                ReadingEvent::Hover(hover) => this.link_hover(
                    crate::hover_preview::Source::Reading,
                    reading.entity_id(),
                    hover.clone(),
                    cx,
                ),
                ReadingEvent::OpenLink(target) => this.follow_link(target.clone(), false, cx),
                ReadingEvent::OpenLinkInNewTab(target) => {
                    this.follow_link(target.clone(), true, cx)
                }
                ReadingEvent::ContextMenu { target, position } => {
                    this.open_reading_menu(target.clone(), *position, cx)
                }
            },
        ),
        cx.subscribe(reading, |this, _, event: &TaskToggled, cx| {
            this.toggle_task(&event.path, event.marker, cx)
        }),
        cx.subscribe(reading, |this, _, event: &PropertyItemRemoved, cx| {
            this.remove_property_item(&event.path, &event.key, event.index, cx)
        }),
        cx.subscribe(
            editor,
            |this, editor, event: &EditorEvent, cx| match event {
                EditorEvent::Hover(hover) => this.link_hover(
                    crate::hover_preview::Source::Editor,
                    editor.entity_id(),
                    hover.clone(),
                    cx,
                ),
                EditorEvent::Edited => {
                    this.schedule_save(cx);
                }
                EditorEvent::OpenLink(target) => this.follow_link(target.clone(), false, cx),
                EditorEvent::OpenLinkInNewTab(target) => this.follow_link(target.clone(), true, cx),
                EditorEvent::PropertyTypeAssigned { key, ty } => {
                    this.assign_property_type(key.clone(), *ty, cx)
                }
                EditorEvent::ContextMenu {
                    position,
                    selection,
                    selected,
                    link,
                } => {
                    this.open_editor_menu(*position, selection.clone(), *selected, link.clone(), cx)
                }
                EditorEvent::RenameTo(value) => {
                    this.render_requests.request_title_rename(value.clone());
                    cx.notify();
                }
                EditorEvent::Reported(message) => this.report(message.clone(), cx),
                EditorEvent::SlashCommands => {
                    this.render_requests.request_slash(editor.clone(), None);
                    cx.notify();
                }
                EditorEvent::RunCommand(id) => {
                    this.render_requests
                        .request_slash(editor.clone(), Some(id.clone()));
                    cx.notify();
                }
                EditorEvent::Attachments(incoming) => {
                    this.render_requests
                        .request_attachments(editor.clone(), incoming.clone());
                    cx.notify();
                }
            },
        ),
    ]
}
