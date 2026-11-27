use super::dock_controls::DockDrag;
use super::group_layout::DividerDrag;
use super::release_notes;
use super::tab_drag::{DragPart, DropZone, NoDragPreview, TabDrag, measure_for_drag};
use super::window_frame::WindowFrame;
use super::{
    MainView, Modal, ViewMode, Workspace, client_frame, debug_status, window_controls,
    window_handle,
};
use crate::canvas::CanvasView;
use crate::editor::Editor;
use crate::pdf::view::PdfView;
use crate::reading::ReadingView;
use crate::theme::{HEADER_HEIGHT, Theme};
use gpui::{
    App, Context, DragMoveEvent, Entity, MouseButton, SharedString, Window, div, prelude::*, px,
    relative,
};
use tft_app::{Layout, SplitDirection, Tab};
use tft_formats::FileKind;

/// An Outline row as drawn, with the offset the pane scrolls the note to.
pub(crate) fn context_names(window: &Window) -> Vec<String> {
    window
        .context_stack()
        .iter()
        .filter_map(|c| c.primary().map(|e| e.key.to_string()))
        .collect()
}

/// A file the main view shows as the embed `![[file]]` would draw it, in
/// every mode: an image, the audio player, TFT's video card. None has a
/// source to edit.
fn shown_as_embed(kind: FileKind) -> bool {
    matches!(kind, FileKind::Image | FileKind::Audio | FileKind::Video)
}

/// What the status bar reads on an empty tab; it stays and reads as zero.
pub(super) const EMPTY_TAB_STATUS: [&str; 1] = ["0 properties"];

/// What one group's rendering reads: live fields for the active group, stashed for others.
pub(super) struct GroupView {
    pub(super) id: u64,
    pub(super) active: bool,
    pub(super) tabs: Vec<Tab>,
    pub(super) active_tab: usize,
    pub(super) current: Option<String>,
    pub(super) mode: ViewMode,
    pub(super) history_len: usize,
    pub(super) history_ix: usize,
    detached: Option<SharedString>,
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    canvas: Entity<CanvasView>,
    pdf: Entity<PdfView>,
}

impl Workspace {
    /// The items [`Workspace::status_items`] gives the open leaf.
    pub fn status_bar_items(&self, cx: &App) -> Vec<String> {
        if self.active_group_state.mode == ViewMode::Search {
            return Vec::new();
        }
        let caret = if self.active_group_state.mode.is_editing() {
            let editor = self.active_editor().read(cx);
            let cursor = editor.cursor();
            let cursors = match editor.cursor_count() {
                1 => String::new(),
                n => format!("{n} cursors  ·  "),
            };
            let timings = if debug_status() {
                format!(
                    "parse {:.1} ms, {} spans  ·  shape {:.2} ms  ·  ",
                    editor.stats.parse_ms,
                    editor.stats.spans,
                    editor.prepaint_ms()
                )
            } else {
                String::new()
            };
            format!(
                "Ln {}, Col {}  ·  {cursors}{timings}",
                cursor.line + 1,
                cursor.col + 1
            )
        } else {
            String::new()
        };
        Self::status_items(self.active_group_state.current.as_deref(), &caret)
    }

    /// The same items as one string, for the socket's state dump: the
    /// separator is the one the bar itself puts between them.
    pub fn status_bar_text(&self, cx: &App) -> Option<String> {
        let items = self.status_bar_items(cx);
        (!items.is_empty()).then(|| items.join("  ·  "))
    }

    /// The group whose area the one graph view paints in: the active one while it shows
    /// the graph, else the first that does.
    fn graph_group(&self) -> Option<u64> {
        if self.active_group_state.mode == ViewMode::Graph {
            return Some(self.active_group_id());
        }
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.into_iter().find(|id| {
            self.group_ix(*id).is_some_and(|ix| {
                let g = &self.groups[ix];
                g.state
                    .tabs
                    .get(g.state.active_tab)
                    .is_some_and(|t| t.mode == ViewMode::Graph)
            })
        })
    }

    /// The SearchPane entity is shared, so only one active group paints it.
    fn search_group(&self) -> Option<u64> {
        if self.active_group_state.mode == ViewMode::Search {
            return Some(self.active_group_id());
        }
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.into_iter().find(|id| {
            self.group_ix(*id).is_some_and(|ix| {
                let group = &self.groups[ix];
                group
                    .state
                    .tabs
                    .get(group.state.active_tab)
                    .is_some_and(|tab| tab.mode == ViewMode::Search)
            })
        })
    }

    /// The status bar's items, left to right, for what a leaf holds.
    ///
    /// An **empty** tab is not a blank status bar: with one active
    /// Obsidian's reads `0 properties` (see [[Specification#The empty tab]]),
    /// so the item stays and reads as zero. A canvas tab shows nothing. The
    /// caret position is TFT's own; the parse and shape numbers behind it
    /// are diagnostics and appear only with `TFT_DEBUG_STATUS` set.
    pub(super) fn status_items(current: Option<&str>, caret: &str) -> Vec<String> {
        let Some(path) = current else {
            return EMPTY_TAB_STATUS
                .iter()
                .map(|item| item.to_string())
                .collect();
        };
        if FileKind::from_path(path) == FileKind::Canvas {
            return Vec::new();
        }
        if caret.is_empty() {
            Vec::new()
        } else {
            vec![caret.to_string()]
        }
    }

    pub(super) fn main_view(&self, cx: &App) -> MainView {
        if self.active_release_notes().is_some() {
            return MainView::ReleaseNotes;
        }
        let has_note = self.active_reading().read(cx).path.is_some();
        let image = self
            .active_group_state
            .current
            .as_deref()
            .is_some_and(|p| shown_as_embed(FileKind::from_path(p)));
        let has_buffer = !self.active_editor().read(cx).name.is_empty();
        let canvas = self.active_group_state.current.is_some()
            && self.active_canvas().read(cx).path() == self.active_group_state.current.as_deref();
        let pdf = self.active_group_state.current.is_some()
            && self.active_pdf().read(cx).path() == self.active_group_state.current.as_deref();
        Self::main_view_for(
            self.active_group_state.mode,
            has_note,
            image,
            has_buffer,
            canvas,
            pdf,
        )
    }

    /// The rule over what the views have: `has_note`, `image`, `has_buffer`, `canvas`,
    /// `pdf`. A canvas or PDF has no reading/source mode, so it paints in every mode
    /// but the graph's.
    pub(super) fn main_view_for(
        mode: ViewMode,
        has_note: bool,
        image: bool,
        has_buffer: bool,
        canvas: bool,
        pdf: bool,
    ) -> MainView {
        match mode {
            ViewMode::Search => MainView::Search,
            ViewMode::Graph => MainView::Graph,
            _ if canvas => MainView::Canvas,
            _ if pdf => MainView::Pdf,
            ViewMode::Reading if has_note => MainView::Reading,
            // An image has no source to edit, so it reads the same in every mode.
            ViewMode::LivePreview | ViewMode::Source if image && has_note => MainView::Reading,
            ViewMode::LivePreview | ViewMode::Source if has_buffer => MainView::Editor,
            _ => MainView::Empty,
        }
    }

    /// What one group's rendering reads: live fields for the active group, stashed for others.
    pub(super) fn group_view(&self, ix: usize) -> GroupView {
        let g = &self.groups[ix];
        if ix == self.active_group {
            return GroupView {
                id: g.state.id,
                active: true,
                tabs: self.active_group_state.tab_snapshots(),
                active_tab: self.active_group_state.active_tab,
                current: self.active_group_state.current.clone(),
                mode: self.active_group_state.mode,
                history_len: self.active_group_state.history.len(),
                history_ix: self.active_group_state.history_ix,
                detached: self.active_group_state.detached.clone(),
                editor: self.active_editor().clone(),
                reading: self.active_reading().clone(),
                canvas: self.active_canvas().clone(),
                pdf: self.active_pdf().clone(),
            };
        }
        let tab = g.state.tabs.get(g.state.active_tab);
        GroupView {
            id: g.state.id,
            active: false,
            tabs: g.state.tabs.clone(),
            active_tab: g.state.active_tab,
            current: tab.and_then(|t| t.path.clone()),
            mode: tab.map(|t| t.mode).unwrap_or(ViewMode::Reading),
            history_len: tab.map(|t| t.history.len()).unwrap_or(0),
            history_ix: tab.map(|t| t.history_ix).unwrap_or(0),
            detached: g.state.detached.clone().map(Into::into),
            editor: g.editor.clone(),
            reading: g.reading.clone(),
            canvas: g.canvas.clone(),
            pdf: g.pdf.clone(),
        }
    }

    /// The main area: the layout tree drawn as nested rows and columns, with draggable dividers.
    pub(super) fn render_layout(
        &self,
        node: &Layout,
        path: &mut Vec<usize>,
        window: Option<u64>,
        frame: WindowFrame,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match node {
            Layout::Group(id) => match self.group_ix(*id) {
                Some(ix) => self.render_group(ix, window, frame, theme, cx),
                None => div().into_any_element(),
            },
            Layout::Split {
                direction,
                children,
                ..
            } => {
                let row = *direction == SplitDirection::Vertical;
                // A split whose children lack a `dimension` shares its room equally, as
                // Obsidian draws one.
                let dims: Option<Vec<f64>> =
                    children.iter().map(|c| self.dimension_of(c)).collect();
                let split_path = path.clone();
                let drag_path = split_path.clone();
                let mut container = div()
                    .id(SharedString::from(format!("split-{split_path:?}")))
                    .flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .when(row, |d| d.flex_row())
                    .when(!row, |d| d.flex_col())
                    .on_drag_move(cx.listener(
                        move |this, e: &DragMoveEvent<DividerDrag>, _, cx| {
                            let drag = e.drag(cx).clone();
                            if drag.path == drag_path {
                                this.drag_divider(&drag, e.bounds, e.event.position, cx);
                            }
                        },
                    ));
                for (ix, child) in children.iter().enumerate() {
                    path.push(ix);
                    let body = self.render_layout(child, path, window, frame, theme, cx);
                    path.pop();
                    let grow = dims.as_ref().map(|d| d[ix] as f32).unwrap_or(1.0);
                    let mut cell = div()
                        .relative()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex_basis(px(0.))
                        .child(body);
                    cell.style().flex_grow = Some(grow.max(0.01));
                    if ix > 0 {
                        let drag = DividerDrag {
                            window,
                            path: split_path.clone(),
                            before: ix - 1,
                        };
                        let accent = theme.tokens.interactive_accent;
                        let dragging = self.divider_drag_state.active.as_ref().is_some_and(
                            |(w, p, before)| {
                                *w == window && p == &split_path && *before == ix - 1
                            },
                        );
                        let handle = div()
                            .id(("divider", ix as u64))
                            .absolute()
                            .when(row, |d| {
                                d.left(px(0.))
                                    .top_0()
                                    .h_full()
                                    .w(px(3.))
                                    .cursor_col_resize()
                            })
                            .when(!row, |d| {
                                d.top(px(0.))
                                    .left_0()
                                    .w_full()
                                    .h(px(3.))
                                    .cursor_row_resize()
                            })
                            .occlude()
                            // The split's edge takes the primary colour under the
                            // pointer and for as long as it is dragged, like the
                            // sidebar's handle.
                            .hover(move |s| s.bg(accent))
                            .active(move |s| s.bg(accent))
                            .when(dragging, move |d| d.bg(accent))
                            .on_drag(drag, |_, _, _, cx| cx.new(|_| NoDragPreview));
                        cell = cell
                            .when(row, |d| d.border_l_1())
                            .when(!row, |d| d.border_t_1())
                            .border_color(theme.tokens.background_modifier_border)
                            .child(handle);
                    }
                    container = container.child(cell);
                }
                container.into_any_element()
            }
        }
    }

    /// One group: its tab strip, view header and view. A mouse down in a non-active
    /// group makes it active before the view handles the click.
    fn render_group(
        &self,
        ix: usize,
        window: Option<u64>,
        frame: WindowFrame,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let view = self.group_view(ix);
        let id = view.id;
        // The tree being drawn: the main area's, or the popout's whose window is rendering.
        let tree = self.tree(window);
        let mut order = Vec::new();
        tree.groups(&mut order);
        // Top left is the first group however the splits run; top right needs the tree's shape.
        let first = order.first() == Some(&id);
        let last = tree.top_right() == Some(id);
        div()
            .id(("group", id))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(!view.active, |d| {
                d.capture_any_mouse_down(
                    cx.listener(move |this, _, _, cx| this.activate_group(id, cx)),
                )
            })
            .child(self.render_tab_strip(&view, window, first, last, frame, theme, cx))
            // Every tab draws its view header, in the main area or a popout.
            .when(view.mode != ViewMode::Search, |d| {
                d.child(self.render_header(&view, theme, cx))
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_group_main(&view, theme, cx))
                    .when(self.tab_drag_state.active, |d| {
                        d.child(self.render_drop_zones(id, theme, cx))
                    }),
            )
            .into_any_element()
    }

    /// The drop target over a group's content while a tab is dragged: it lights the
    /// zone the tab would land in and takes the drop, covering the view.
    fn render_drop_zones(
        &self,
        id: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let zone = self
            .tab_drag_state
            .drop
            .filter(|(g, _)| *g == id)
            .map(|(_, z)| z);
        let wash = theme.tokens.interactive_accent.opacity(0.15);
        let edge = theme.tokens.interactive_accent.opacity(0.6);
        div()
            .id(("drop-zones", id))
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.tab_drag_moved(id, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                this.drop_tab_on_group(drag, id, window, cx)
            }))
            .child(measure_for_drag(
                &self.tab_drag_state.bounds,
                DragPart::Content(id),
            ))
            .when_some(zone, |d, zone| {
                let half = relative(0.5);
                d.child(
                    div()
                        .absolute()
                        .bg(wash)
                        .border_2()
                        .border_color(edge)
                        .rounded(px(6.))
                        .map(|d| match zone {
                            DropZone::Center => d.top_0().left_0().right_0().bottom_0(),
                            DropZone::Left => d.top_0().left_0().bottom_0().w(half),
                            DropZone::Right => d.top_0().right_0().bottom_0().w(half),
                            DropZone::Top => d.top_0().left_0().right_0().h(half),
                            DropZone::Bottom => d.bottom_0().left_0().right_0().h(half),
                        }),
                )
            })
    }

    /// The file group `id` shows, wherever that group's fields live.
    fn group_current(&self, id: u64) -> Option<&str> {
        let ix = self.group_ix(id)?;
        if ix == self.active_group {
            return self.active_group_state.current.as_deref();
        }
        let group = &self.groups[ix];
        group
            .state
            .tabs
            .get(group.state.active_tab)?
            .path
            .as_deref()
    }

    /// What a group's area paints, by [`Workspace::main_view`]'s rule. There is one
    /// graph view, so a second group says where it is instead.
    fn render_group_main(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_note = view.reading.read(cx).path.is_some();
        let image = view
            .current
            .as_deref()
            .is_some_and(|p| shown_as_embed(FileKind::from_path(p)));
        let has_buffer = !view.editor.read(cx).name.is_empty();
        let canvas =
            view.current.is_some() && view.canvas.read(cx).path() == view.current.as_deref();
        let pdf = view.current.is_some() && view.pdf.read(cx).path() == view.current.as_deref();
        if let Some(version) = view
            .tabs
            .get(view.active_tab)
            .and_then(release_notes::version_of)
        {
            return self.release_notes_view(&version, cx).into_any_element();
        }
        match Self::main_view_for(view.mode, has_note, image, has_buffer, canvas, pdf) {
            MainView::Search if self.search_group() == Some(view.id) => {
                self.search.clone().into_any_element()
            }
            MainView::Search => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.tokens.text_muted)
                .child("Search is open in another pane")
                .into_any_element(),
            MainView::Canvas => view.canvas.clone().into_any_element(),
            MainView::Pdf => view.pdf.clone().into_any_element(),
            MainView::Graph if self.graph_group() == Some(view.id) => {
                if self.radial_tree.active {
                    self.radial_tree
                        .view
                        .as_ref()
                        .map(|tree| tree.clone().into_any_element())
                        .unwrap_or_else(|| self.graph.clone().into_any_element())
                } else {
                    self.graph.clone().into_any_element()
                }
            }
            MainView::Graph => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.tokens.text_muted)
                .child("The graph is open in another pane")
                .into_any_element(),
            MainView::Reading | MainView::ReleaseNotes => view.reading.clone().into_any_element(),
            MainView::Editor => Self::render_editor(&view.editor, view.detached.clone(), theme),
            MainView::Empty => match view.current.as_deref() {
                Some(path) => Self::render_unsupported_file(path, theme),
                None => self.render_empty_tab(theme, cx),
            },
        }
    }

    /// The editor, under a line naming the lost file if its buffer is no longer backed
    /// by one; the alternative is unreported typing.
    fn render_editor(
        editor: &Entity<Editor>,
        detached: Option<SharedString>,
        theme: &Theme,
    ) -> gpui::AnyElement {
        let Some(name) = detached else {
            return editor.clone().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .bg(theme.tokens.background_modifier_hover)
                    .text_color(theme.tokens.text_muted)
                    .border_b_1()
                    .border_color(theme.tokens.background_modifier_border)
                    .child(format!(
                        "{name} — this buffer no longer belongs to a file in the vault, \
                         and nothing saves it"
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(editor.clone().into_any_element()),
            )
            .into_any_element()
    }

    /// A file whose kind no view can draw: an error message on its own.
    fn render_unsupported_file(path: &str, theme: &Theme) -> gpui::AnyElement {
        let message: SharedString = format!(
            "{:?} files are not supported yet",
            FileKind::from_path(path)
        )
        .into();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.tokens.text_muted)
            .child(message)
            .into_any_element()
    }

    // ----- tabs ------------------------------------------------------------------
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = WindowFrame::of(window);
        if let Some(value) = self.render_requests.take_title_rename() {
            cx.defer_in(window, move |this, window, cx| {
                this.rename_from_inline_title(&value, window, cx)
            });
        }
        for (editor, run) in self.render_requests.take_slash() {
            cx.defer_in(window, move |this, window, cx| match run {
                None => {
                    let commands = this.slash_commands(window, cx);
                    editor.update(cx, |e, cx| e.open_slash(commands, cx));
                }
                Some(id) => this.run_slash_command(&editor, &id, window, cx),
            });
        }
        for (editor, incoming) in self.render_requests.take_attachments() {
            cx.defer_in(window, move |this, window, cx| {
                this.save_incoming(&editor, incoming, window, cx)
            });
        }
        self.left_dock.keyboard = self.left_dock.focus.is_focused(window);
        let corners = frame.corners;
        // The desktop's scheme: a vault on `theme: "system"` follows it.
        let system_dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        if self.theme_appearance.set_system_dark(system_dark) {
            self.reload_theme(cx);
        }
        // An open menu holds the keyboard, so Escape reaches it first; a
        // closed one hands the keyboard back where it was.
        self.sync_menu_focus(None, window, cx);
        if let Some(kind) = self.render_requests.take_canvas_pick() {
            self.open_canvas_file_chooser(kind, window, cx);
        }
        // A view's focus handle only joins the dispatch tree when the frame
        // that paints it is prepainted, so this — the start of that frame —
        // is the first moment a focus asked for elsewhere can land. A modal
        // owns the keyboard while it is up, and closing one focuses the main
        // area anyway, so the request is dropped rather than queued.
        // A popout's render does the same for an active group of its own
        // (see `render_popout`), so this window only acts on its own.
        if !self.active_group_in_popout()
            && std::mem::take(&mut self.focus.refocus_main)
            && self.overlay.modal == Modal::None
            && self.keyboard_is_parked(window, cx)
        {
            self.focus_main(window, cx);
        }
        // A title edit belongs to the file whose header it opened on: if
        // that leaf moved on -- another tab, another file, the rename
        // itself -- the field goes with it rather than committing a name to
        // whatever is on screen now.
        if let Some(edit) = &self.title_editing.editing {
            if self.group_current(edit.group) != Some(edit.path.as_str()) {
                self.title_editing.editing = None;
                if self.title_editing.focus.is_focused(window) {
                    self.focus_main(window, cx);
                }
            }
        }
        self.tab_drag_frame(window, cx);
        let theme = cx.global::<Theme>().clone();
        // The active leaf's name, which is the window title's first part:
        // "New tab" with nothing open, as Obsidian's reads
        // (`New tab - <vault> - Obsidian 1.14.5`).
        let title = self.window_leaf_title(&self.layout_tree);

        let main_body: gpui::AnyElement = if let Some(loading) = &self.vault_session.loading {
            div()
                .p_4()
                .text_color(theme.tokens.text_muted)
                .child(loading.clone())
                .into_any_element()
        } else {
            let tree = self.layout_tree.clone();
            self.render_layout(&tree, &mut Vec::new(), None, frame, &theme, cx)
        };

        let wanted_title: SharedString =
            format!("{title} - {} - TFT", self.vault_session.name).into();
        if self.main_window.set_title(wanted_title.clone()) {
            window.set_window_title(&wanted_title);
        }

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(main_body);

        let frameless = frame.frameless;
        let mac = frame.mac_frame();
        // A pane whose plugin is off has no tab, so it cannot be the one
        // showing.
        let left_body: gpui::AnyElement = self.explorer.clone().into_any_element();
        let left = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.left_dock.width as f32))
            .h_full()
            .bg(theme.tokens.tab_container_background)
            .map(|d| corners.left().apply(d))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    // Past the traffic lights on macOS, where this header
                    // is the window's top-left corner.
                    .pl(px(8. + frame.left_space()))
                    .pr(px(8.))
                    .bg(theme.tokens.sidebar_tab_container_background)
                    .gap(px(3.))
                    .border_b_1()
                    .border_color(theme.tokens.tab_outline_color)
                    .child(window_handle(
                        div().id("left-dock-space").flex_1().h_full(),
                        frameless,
                    ))
                    // The toggle sits at the right end, against the dock's
                    // edge.
                    .child(self.sidebar_toggle(&theme, cx)),
            )
            .child(div().flex_1().overflow_hidden().child(left_body))
            .child(self.render_vault_profile(&theme, window, cx))
            .child(self.dock_handle(true, &theme));

        // The modal and the file menu draw in the window they came up in:
        // this one, unless that was a popout.
        let overlay = self.render_modal_overlay(None);
        // The notices that went up while this window was the active one.
        let notices = self.render_notices(None, px(30.), &theme, cx);
        let preview = self.render_hover_preview(None, &theme, window, cx);

        let left_visible = self.left_dock.visible;
        // A modal owns the keyboard while it is up, the way an Obsidian
        // modal's own keymap scope keeps app hotkeys from firing behind it.
        // gpui's key contexts nest rather than replace — the overlay is a
        // child of this div — so the workspace stands its handlers down
        // instead: without them a hotkey can no longer focus a pane behind
        // the modal, or swap the modal out from under its own Escape.
        let hotkeys_live = self.overlay.modal == Modal::None;
        // macOS has the desktop's traffic lights instead.
        let controls = (frameless && !mac)
            .then(|| window_controls(&theme, window, corners, |w, _| w.remove_window()));

        let content = div()
            .key_context("Workspace")
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, _, cx| {
                this.preview_modifiers(e.modifiers, cx)
            }))
            .track_focus(&self.focus.handle)
            // A tab dragged out of this window: the moves and the
            // release reach this window only, so this is where the
            // other windows' zones are found and dropped on.
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<TabDrag>, window, cx| {
                this.cross_window_drag_moved(e.bounds, e.event.position, window, cx)
            }))
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.cross_window_drop(window, cx)),
            )
            .when(hotkeys_live, |d| {
                d.on_action(cx.listener(Self::toggle_mode))
                    .on_action(cx.listener(Self::go_back))
                    .on_action(cx.listener(Self::go_forward))
                    .on_action(cx.listener(Self::open_search_action))
                    .on_action(cx.listener(Self::open_graph_action))
                    .on_action(cx.listener(Self::open_command_palette_action))
                    .on_action(cx.listener(Self::focus_explorer_action))
                    .on_action(cx.listener(Self::run_command_action))
            })
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.background_primary)
            // This box paints the whole content square, over the frame's
            // rounding and under its children's; a corner left square here
            // is a notch of `theme.tokens.background_primary` outside every rounded child.
            .map(|d| corners.apply(d))
            .font_family(theme.ui_font.clone())
            .text_size(px(crate::theme::FONT_UI_MEDIUM))
            .text_color(theme.tokens.text_normal)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DockDrag>, _, cx| {
                        let left = e.drag(cx).left;
                        this.drag_dock(left, e.bounds, e.event.position, cx);
                    }))
                    .on_drop(cx.listener(|this, _: &DockDrag, _, cx| {
                        this.left_dock.dragging = false;
                        cx.notify();
                    }))
                    .on_drop(cx.listener(|this, _: &DividerDrag, _, cx| {
                        this.divider_drag_state.active = None;
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.left_dock.dragging
                                || this.divider_drag_state.active.is_some()
                            {
                                this.left_dock.dragging = false;
                                this.divider_drag_state.active = None;
                                cx.notify();
                            }
                        }),
                    )
                    .when(left_visible, |d| d.child(left))
                    .child(main),
            )
            .children(controls)
            .children(overlay)
            // One file menu for every view that raises it -- the Backlinks
            // and Outgoing links rows, the editor's text, a link in the
            // reading view -- positioned at the click and drawn over
            // everything else, so no dock's clipping can cut it off.
            .children(self.render_file_menu(None, &theme, window, cx))
            .children(notices)
            .children(preview);
        client_frame(content.into_any_element(), &theme, window)
    }
}
