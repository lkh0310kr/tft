//! Inline editing of the active view header's document title.

use super::render::GroupView;
use super::{ViewMode, Workspace, inline_field, leaf_title};
use crate::theme::{FONT_UI_SMALL, HEADER_HEIGHT, Theme};
use gpui::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, Hsla, MouseButton,
    MouseDownEvent, Pixels, SharedString, UTF16Selection, Window, canvas, div, prelude::*, px,
};
use std::collections::HashMap;
use std::ops::Range;

/// The view-header title being renamed in place. Obsidian's `.view-header-title`
/// is editable at rest, so a click starts typing and only Enter renames the file.
/// The text is the stem, shared with the rename prompt's validator.
pub(super) struct TitleEdit {
    /// The group whose header holds the field.
    pub(super) group: u64,
    /// The file the field opened on; the edit is dropped if that leaf moves on.
    pub(super) path: String,
    /// The typed text and the input method's composition, as the file tree's row rename holds.
    pub(super) editing: inline_field::Editing,
}

/// Live inline-title editor state shared with focus routing and group cleanup.
pub(super) struct TitleEditingState {
    pub(super) focus: FocusHandle,
    pub(super) editing: Option<TitleEdit>,
    pub(super) layouts: HashMap<u64, inline_field::Layout>,
}

impl TitleEditingState {
    pub(super) fn new(focus: FocusHandle) -> Self {
        Self {
            focus,
            editing: None,
            layouts: HashMap::new(),
        }
    }
}

impl Workspace {
    /// The view header's title, editable where it stands, so a click starts an edit
    /// (see [[Specification#The view-header title is an inline rename field]]). Painted
    /// as a shaped line so a click lands the caret on a glyph; a leaf with no file is not renamed.
    pub(super) fn render_title(
        &self,
        view: &GroupView,
        title: SharedString,
        color: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = view.id;
        let editing = self
            .title_editing
            .editing
            .as_ref()
            .is_some_and(|e| e.group == id);
        let read_entity = cx.entity();
        let paint_entity = cx.entity();
        let read_focus = self.title_editing.focus.clone();
        let paint_focus = self.title_editing.focus.clone();
        let mut selection_color = theme.tokens.interactive_accent;
        selection_color.a = if theme.dark { 0.35 } else { 0.25 };
        let caret_color = theme.tokens.text_normal;
        let body = canvas(
            move |bounds, window, cx| {
                let ws = read_entity.read(cx);
                let edit = ws.title_editing.editing.as_ref().filter(|e| e.group == id);
                let text: SharedString = match edit {
                    Some(edit) => edit.editing.field.text().to_string().into(),
                    None => title.clone(),
                };
                let scroll_x = ws
                    .title_editing
                    .layouts
                    .get(&id)
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
                // The input method is pointed at the field only while it is open: the
                // title is not a tab stop.
                if editing {
                    window.handle_input(
                        &paint_focus,
                        ElementInputHandler::new(bounds, paint_entity.clone()),
                        cx,
                    );
                }
                let layout =
                    inline_field::paint(prepaint, bounds, selection_color, caret_color, window, cx);
                paint_entity.update(cx, |ws, _| {
                    ws.title_editing.layouts.insert(id, layout);
                });
            },
        )
        .size_full();
        let mut host = div()
            .id(("view-title", id))
            .flex_1()
            .min_w_0()
            // The canvas has no size of its own, so the row's height is set here.
            .h(px(HEADER_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(FONT_UI_SMALL))
            .text_color(color)
            .child(body);
        if let Some(path) = view.current.clone() {
            host = host.cursor_text().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.title_clicked(id, path.clone(), event, window, cx)
                }),
            );
        }
        if !editing {
            return host.into_any_element();
        }
        // The keys here are only a field's, in the crate's own field key context.
        let host = host
            .track_focus(&self.title_editing.focus)
            .key_context("TextInput");
        inline_field::bind_keys(host, cx).into_any_element()
    }

    /// A click on a view-header title focuses the field and puts the caret where the
    /// pointer landed; a double click takes the word. A click past the text changes nothing.
    fn title_clicked(
        &mut self,
        group: u64,
        path: String,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.title_editing.layouts.get(&group).cloned() else {
            return;
        };
        // A click in another group's header makes that group active first.
        self.activate_group(group, cx);
        // The field holds the name without its extension, the same stem the rename prompt boxes.
        let name = leaf_title(Some(&path), ViewMode::Reading).to_string();
        let mut edit = match self.title_editing.editing.take() {
            Some(edit) if edit.group == group && edit.path == path => edit,
            _ => TitleEdit {
                group,
                path,
                editing: inline_field::Editing::new(name),
            },
        };
        let Some(at) = layout.offset_for(event.position, edit.editing.field.text()) else {
            self.title_editing.editing = None;
            return;
        };
        edit.editing
            .field
            .click(at, event.click_count, event.modifiers.shift);
        self.title_editing.editing = Some(edit);
        // The field's focus handle joins the dispatch tree only with the frame that
        // paints it; a click landing before that frame takes the keys next frame.
        let focus = self.title_editing.focus.clone();
        window.focus(&focus, cx);
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    /// Enter in the view-header title: the same validator and link rewrite as the
    /// rename prompt. A refused name leaves the field open and says why.
    fn commit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.title_editing.editing.take() else {
            return;
        };
        let value = edit.editing.field.text().trim().to_string();
        let to = match self.renamed_path(&edit.path, &value) {
            Ok(to) => to,
            Err(err) => {
                self.title_editing.editing = Some(edit);
                self.report(err, cx);
                return;
            }
        };
        // The keyboard goes back to the view before the rename, which may raise the
        // "Update links?" prompt and want it.
        self.focus_main(window, cx);
        self.rename_with_links(&edit.path, &to, window, cx);
        cx.notify();
    }

    /// Enter in the note's inline title: the same path, validator and link rewrite as
    /// the view-header title.
    pub(super) fn rename_from_inline_title(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.active_group_state.current.clone() else {
            return;
        };
        match self.renamed_path(&from, value.trim()) {
            Ok(to) => self.rename_with_links(&from, &to, window, cx),
            Err(err) => self.report(err, cx),
        }
    }

    /// Escape in the view-header title: the edit is abandoned, as Obsidian's does.
    fn cancel_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_editing.editing.take().is_none() {
            return;
        }
        self.focus_main(window, cx);
        cx.notify();
    }
}

impl inline_field::Host for Workspace {
    fn editing(&mut self) -> Option<&mut inline_field::Editing> {
        self.title_editing
            .editing
            .as_mut()
            .map(|edit| &mut edit.editing)
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_title(window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_title(window, cx);
    }
}

/// Typed, composed and pasted text while the view header's title is being
/// renamed in place; [`inline_field`] is the half the file tree's row
/// rename shares. The handler is registered by the title's own element
/// against the title's own focus handle (see [`Workspace::render_title`]),
/// so it only ever sees text meant for that field; with no field open every
/// method answers as an empty one would.
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        inline_field::text_for_range(
            &self.title_editing.editing.as_ref()?.editing,
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
            &self.title_editing.editing.as_ref()?.editing,
        ))
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        inline_field::marked_text_range(&self.title_editing.editing.as_ref()?.editing)
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(edit) = self.title_editing.editing.as_mut() {
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
        let Some(edit) = self.title_editing.editing.as_mut() else {
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
        let Some(edit) = self.title_editing.editing.as_mut() else {
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
        let edit = self.title_editing.editing.as_ref()?;
        let layout = self.title_editing.layouts.get(&edit.group)?;
        Some(inline_field::bounds_for_range(
            &edit.editing,
            layout,
            range_utf16,
            element_bounds,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
