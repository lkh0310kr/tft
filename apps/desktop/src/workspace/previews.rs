//! Desktop hover-preview lifecycle, nested link routing, and popover rendering.

use super::{LinkTarget, Workspace};
use crate::editor::{Editor, EditorEvent};
use crate::theme::Theme;
use gpui::{AppContext, Bounds, Context, Entity, Focusable, Pixels, Window};
use std::rc::Rc;
use std::time::Duration;
use tft_formats::properties::PropertiesInDocument;

/// Hover popovers and the debounce task for edits made inside them.
#[derive(Default)]
pub(super) struct PreviewState {
    pub(super) model: crate::hover_preview::HoverPreview,
    save_task: Option<gpui::Task<()>>,
}

impl Workspace {
    pub(super) fn view_origin(&self, view: gpui::EntityId) -> Option<(Option<u64>, String)> {
        let ix = self
            .groups
            .iter()
            .position(|g| g.reading.entity_id() == view || g.editor.entity_id() == view)?;
        let group = &self.groups[ix];
        let path = if ix == self.active_group {
            self.active_group_state
                .tabs
                .get(self.active_group_state.active_tab)?
                .path
                .clone()
        } else {
            group.state.tabs.get(group.state.active_tab)?.path.clone()
        }?;
        Some((self.window_of_group(group.state.id), path))
    }

    /// The pointer came onto or left a link. A link that may preview arms the show
    /// timer; leaving arms the hide timer, defused by reaching the popover.
    pub(super) fn link_hover(
        &mut self,
        source: crate::hover_preview::Source,
        view: gpui::EntityId,
        hover: Option<crate::reading::LinkHover>,
        cx: &mut Context<Self>,
    ) {
        let Some((window, from)) = self.view_origin(view) else {
            return;
        };
        self.nested_link_hover(source, window, from, 0, hover, cx);
    }

    /// [`Self::link_hover`] for a link `depth` popovers deep; leaving clears it only
    /// if it is still one at this depth, since arrival can precede departure.
    pub(super) fn nested_link_hover(
        &mut self,
        source: crate::hover_preview::Source,
        window: Option<u64>,
        from: String,
        depth: usize,
        hover: Option<crate::reading::LinkHover>,
        cx: &mut Context<Self>,
    ) {
        if hover.is_none()
            && self
                .preview_state
                .model
                .pending
                .as_ref()
                .is_some_and(|p| p.depth != depth)
        {
            return;
        }
        self.preview_state.model.generation += 1;
        let pending = hover.and_then(|h| {
            self.preview_state.model.modifiers = h.modifiers;
            let LinkTarget::Internal(target) = h.target else {
                return None;
            };
            Some(crate::hover_preview::Pending {
                window,
                source,
                target,
                from,
                anchor: h.anchor,
                depth,
            })
        });
        self.set_preview_pending(pending, cx);
    }

    pub(super) fn set_preview_pending(
        &mut self,
        pending: Option<crate::hover_preview::Pending>,
        cx: &mut Context<Self>,
    ) {
        self.preview_state.model.pending = pending;
        if self.preview_state.model.pending.is_some() {
            self.arm_preview_show(cx);
        }
        // Either way something may have been left: a link for nothing, or
        // for another link whose popover goes ahead of this one's.
        self.arm_preview_hide(cx);
    }

    /// The pointer came onto a previewing row (a pane's row, a tab) or left it.
    /// Leaving clears the link only if it is still this row's; the next arrival can precede departure.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row_hover(
        &mut self,
        source: crate::hover_preview::Source,
        window: Option<u64>,
        target: String,
        from: String,
        over: bool,
        anchor: Bounds<Pixels>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        if !over {
            let ours =
                self.preview_state.model.pending.as_ref().is_some_and(|p| {
                    p.source == source && p.anchor == anchor && p.target == target
                });
            if !ours {
                return;
            }
        }
        self.preview_state.model.generation += 1;
        self.preview_state.model.modifiers = modifiers;
        let pending = over.then_some(crate::hover_preview::Pending {
            window,
            source,
            target,
            from,
            anchor,
            depth: 0,
        });
        self.set_preview_pending(pending, cx);
    }

    /// The pointer left whatever `source` last reported, for sources that do not say which.
    pub(super) fn source_left(
        &mut self,
        source: crate::hover_preview::Source,
        cx: &mut Context<Self>,
    ) {
        if self
            .preview_state
            .model
            .pending
            .as_ref()
            .is_some_and(|p| p.source == source && p.depth == 0)
        {
            self.preview_state.model.generation += 1;
            self.set_preview_pending(None, cx);
        }
    }

    /// Mod down with the pointer on a link raises its popover after ~300 ms; Mod up leaves one up.
    pub(super) fn preview_modifiers(&mut self, modifiers: gpui::Modifiers, cx: &mut Context<Self>) {
        let was = crate::hover_preview::mod_held(&self.preview_state.model.modifiers);
        self.preview_state.model.modifiers = modifiers;
        if !was
            && crate::hover_preview::mod_held(&modifiers)
            && self.preview_state.model.pending.is_some()
        {
            self.preview_state.model.generation += 1;
            self.arm_preview_show(cx);
        }
    }

    /// Starts the show timer if the link may preview: the plugin on and Mod held
    /// where its source needs it.
    pub(super) fn arm_preview_show(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let Some(pending) = &self.preview_state.model.pending else {
            return;
        };
        // Mod-click in the Files tree opens a tab. Do not let the Files
        // source's usual Mod-to-preview gesture race with that action.
        if pending.source == crate::hover_preview::Source::Files
            && crate::hover_preview::mod_held(&self.preview_state.model.modifiers)
        {
            return;
        }
        if !state.core_plugin_enabled("page-preview") {
            return;
        }
        if pending.source.requires_mod(&state.page_preview())
            && !crate::hover_preview::mod_held(&self.preview_state.model.modifiers)
        {
            return;
        }
        let generation = self.preview_state.model.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::hover_preview::SHOW_DELAY)
                .await;
            this.update(cx, |this, cx| {
                if this.preview_state.model.generation == generation {
                    this.show_preview(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Starts a hide timer, if a popover is up; [`HoverPreview::kept`] reads where the
    /// pointer is when it fires, so no generation is needed.
    pub(super) fn arm_preview_hide(&mut self, cx: &mut Context<Self>) {
        if self.preview_state.model.shown.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::hover_preview::HIDE_DELAY)
                .await;
            this.update(cx, |this, cx| {
                let kept = this.preview_state.model.kept();
                this.truncate_previews(kept, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn show_preview(&mut self, cx: &mut Context<Self>) {
        let (Some(state), Some(pending)) = (
            self.vault_session.state.clone(),
            self.preview_state.model.pending.clone(),
        ) else {
            return;
        };
        let (content, from) = crate::hover_preview::content(
            state.as_ref(),
            state.build_options(),
            &pending.target,
            &pending.from,
        );
        // An editing popover goes only on Escape or a press outside, so a link under it raises nothing.
        if self
            .preview_state
            .model
            .shown
            .iter()
            .skip(pending.depth)
            .any(|s| s.editing.is_some())
        {
            return;
        }
        let note =
            crate::hover_preview::editable_note(state.as_ref(), &pending.target, &pending.from);
        // The new popover replaces whatever was up above the link's own.
        self.truncate_previews(pending.depth, cx);
        // The properties block starts folded, as Obsidian's popover had it.
        self.preview_state
            .model
            .shown
            .push(crate::hover_preview::Shown::new(
                pending.window,
                pending.anchor,
                pending.target.clone(),
                from,
                content,
                note,
                [crate::properties::PROPERTIES_FOLD_ID]
                    .into_iter()
                    .collect(),
            ));
        cx.notify();
    }

    /// Keeps the first `keep` popovers and takes down the rest, writing an editing one's edits first.
    pub(super) fn truncate_previews(&mut self, keep: usize, cx: &mut Context<Self>) {
        if self.preview_state.model.shown.len() <= keep {
            return;
        }
        self.flush_preview_save(cx);
        self.preview_state.model.shown.truncate(keep);
        // A view that is gone never reports the pointer leaving its link.
        if self
            .preview_state
            .model
            .pending
            .as_ref()
            .is_some_and(|p| p.depth > keep)
        {
            self.preview_state.model.pending = None;
        }
        cx.notify();
    }

    pub(super) fn hide_preview(&mut self, cx: &mut Context<Self>) {
        self.truncate_previews(0, cx);
    }

    /// Drops the link under the pointer and the non-editing popovers, and makes every
    /// view forget its link: called when the screen changes under the pointer.
    pub(super) fn reset_preview(&mut self, cx: &mut Context<Self>) {
        let hp = &mut self.preview_state.model;
        if hp.pending.is_none() && hp.shown.is_empty() {
            return;
        }
        hp.generation += 1;
        hp.pending = None;
        let kept = hp
            .shown
            .iter()
            .rposition(|s| s.editing.is_some())
            .map_or(0, |ix| ix + 1);
        self.truncate_previews(kept, cx);
        for group in &self.groups {
            group.reading.update(cx, |r, _| r.clear_hover());
            group.editor.update(cx, |e, _| e.clear_hover());
        }
    }

    /// The pointer came onto the `ix`th popover or left it.
    pub(super) fn popover_hover(&mut self, ix: usize, over: bool, cx: &mut Context<Self>) {
        let Some(shown) = self.preview_state.model.shown.get_mut(ix) else {
            return;
        };
        shown.over = over;
        if !over {
            self.preview_state.model.generation += 1;
            self.arm_preview_hide(cx);
        }
    }

    /// A link clicked inside the `ix`th popover opens against the shown note; the popovers go.
    pub(super) fn preview_link(
        &mut self,
        ix: usize,
        target: LinkTarget,
        new_tab: bool,
        cx: &mut Context<Self>,
    ) {
        let from = self
            .preview_state
            .model
            .shown
            .get(ix)
            .map(|s| s.from.clone());
        self.hide_preview(cx);
        let target = match (target, from, self.vault_session.state.clone()) {
            (LinkTarget::Internal(body), Some(from), Some(state)) => {
                match state.cache().resolve(&body, &from) {
                    Some(path) => {
                        let sub = body.find('#').map(|i| &body[i..]).unwrap_or("");
                        LinkTarget::Internal(format!("{path}{sub}"))
                    }
                    None => LinkTarget::Internal(body),
                }
            }
            (target, _, _) => target,
        };
        self.follow_link(target, new_tab, cx);
    }

    /// A click on the `ix`th popover's text makes it a Live Preview editor in the same
    /// box, caret where clicked, until Escape or a press outside.
    pub(super) fn edit_preview(
        &mut self,
        ix: usize,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        // "Click to create": the note is made, empty, and opens in the
        // active tab in its editing mode with the keyboard in it; the
        // popover goes (observed, `s11` `missingClick`).
        if let Some(shown) = self.preview_state.model.shown.get(ix) {
            if matches!(shown.content, crate::hover_preview::Content::Missing(_)) {
                let name = tft_markdown::parse_linktext(&shown.target).path.to_string();
                self.hide_preview(cx);
                self.create_from_link(&name, window, cx);
                self.request_focus_main(cx);
                return;
            }
        }
        let Some((path, subpath)) = self
            .preview_state
            .model
            .shown
            .get(ix)
            .filter(|s| s.editing.is_none())
            .and_then(|s| s.note.clone())
        else {
            return;
        };
        // The popovers over this one go; the open note's edits are written first so the
        // popover starts from what is on disk.
        self.truncate_previews(ix + 1, cx);
        self.flush_save(cx);
        let Ok(text) = std::fs::read_to_string(state.vault().absolute(&path)) else {
            return;
        };
        let editor = cx.new(|cx| crate::new_editor(window, cx));
        Self::give_editor_the_vault(&editor, &state, cx);
        Self::configure_editor(&editor, &state, cx);
        editor.update(cx, |e, cx| {
            e.set_readable_line_length(false);
            e.set_show_inline_title(false);
            e.set_live_preview(true, cx);
            e.set_text(path.clone(), text, cx);
        });
        let subscription = cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
            this.preview_editor_event(&editor, event, cx)
        });
        if let Some(shown) = self.preview_state.model.shown.get_mut(ix) {
            shown.editing = Some(editor.clone());
            shown.editing_subscription = Some(subscription);
            shown.hover_current.set(None);
        }
        window.focus(&editor.focus_handle(cx), cx);
        // The caret at the click once a frame maps the point; for a section, at its start.
        let section = (!subpath.is_empty())
            .then(|| tft_markdown::parse_linktext(&subpath).subpath)
            .flatten()
            .and_then(|sub| Self::subpath_offset(&state, &path, &sub));
        match section {
            Some(offset) => editor.update(cx, |e, cx| {
                e.set_selection_range(offset, offset, window, cx);
                e.reveal_offset(offset, cx);
            }),
            None => {
                let editor = editor.downgrade();
                Self::caret_when_laid_out(editor, position, 4, window);
            }
        }
        cx.notify();
    }

    /// Puts an editor's caret at a window point once it has a layout, trying for `tries` more frames.
    pub(super) fn caret_when_laid_out(
        editor: gpui::WeakEntity<Editor>,
        position: gpui::Point<Pixels>,
        tries: usize,
        window: &mut Window,
    ) {
        window.on_next_frame(move |window, cx| {
            let placed = editor
                .update(cx, |e, cx| e.caret_at_point(position, window, cx))
                .unwrap_or(true);
            if !placed && tries > 0 {
                Self::caret_when_laid_out(editor, position, tries - 1, window);
            }
        });
    }

    /// What an editing popover's editor reports.
    pub(super) fn preview_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.preview_state.model.shown.iter().position(|s| {
            s.editing
                .as_ref()
                .is_some_and(|e| e.entity_id() == editor.entity_id())
        }) else {
            return;
        };
        match event {
            EditorEvent::Edited => self.schedule_preview_save(cx),
            EditorEvent::OpenLink(target) => {
                let target = target.clone();
                self.preview_link(ix, target, false, cx)
            }
            EditorEvent::OpenLinkInNewTab(target) => {
                let target = target.clone();
                self.preview_link(ix, target, true, cx)
            }
            // Its links raise popovers on the editor's rules (Mod).
            EditorEvent::Hover(hover) => {
                let shown = &self.preview_state.model.shown[ix];
                let (window, from) = (shown.window, shown.from.clone());
                self.nested_link_hover(
                    crate::hover_preview::Source::Editor,
                    window,
                    from,
                    ix + 1,
                    hover.clone(),
                    cx,
                )
            }
            EditorEvent::Reported(message) => self.report(message.clone(), cx),
            _ => {}
        }
    }

    /// An editing popover's edits are written two seconds after the last one, as the open note's are.
    pub(super) fn schedule_preview_save(&mut self, cx: &mut Context<Self>) {
        self.preview_state.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |ws, cx| ws.flush_preview_save(cx)).ok();
        }));
    }

    /// Writes every editing popover's unsaved edits now.
    pub(super) fn flush_preview_save(&mut self, cx: &mut Context<Self>) {
        self.preview_state.save_task = None;
        let editors: Vec<Entity<Editor>> = self
            .preview_state
            .model
            .shown
            .iter()
            .filter_map(|s| s.editing.clone())
            .collect();
        for editor in editors {
            if editor.read(cx).is_dirty() {
                self.write_buffer(&editor, cx);
            }
        }
    }

    /// Escape or a press outside the `ix`th editing popover: it goes with those over
    /// it. Escape returns the keyboard to the main view; a press keeps what it hit.
    pub(super) fn close_preview(
        &mut self,
        ix: usize,
        escape: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .preview_state
            .model
            .shown
            .get(ix)
            .is_some_and(|s| s.editing.is_some())
        {
            self.preview_state.model.generation += 1;
            self.truncate_previews(ix, cx);
            // The ones under it go too, unless the pointer is over them.
            self.arm_preview_hide(cx);
            if escape {
                self.focus_main(window, cx);
            }
        }
    }

    /// The popovers up in the window being drawn.
    pub(super) fn render_hover_preview(
        &mut self,
        here: Option<u64>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let Some(state) = self.vault_session.state.clone() else {
            return Vec::new();
        };
        let properties = PropertiesInDocument::default();
        let types = state.property_types().clone();
        let mut out = Vec::new();
        for (ix, shown) in self.preview_state.model.shown.iter().enumerate() {
            if shown.window != here {
                continue;
            }
            let this = cx.entity().downgrade();
            let (window_id, from) = (shown.window, shown.from.clone());
            let handlers = crate::hover_preview::Handlers {
                on_link: Rc::new({
                    let this = this.clone();
                    move |target, _window, cx| {
                        this.update(cx, |this, cx| this.preview_link(ix, target, false, cx))
                            .ok();
                    }
                }),
                // A link in a popover previews on the reading view's rules.
                on_hover_link: Rc::new({
                    let this = this.clone();
                    move |hover, _window, cx| {
                        let from = from.clone();
                        this.update(cx, |this, cx| {
                            this.nested_link_hover(
                                crate::hover_preview::Source::Reading,
                                window_id,
                                from,
                                ix + 1,
                                hover,
                                cx,
                            )
                        })
                        .ok();
                    }
                }),
                on_hover: Rc::new({
                    let this = this.clone();
                    move |over, _window, cx| {
                        this.update(cx, |this, cx| this.popover_hover(ix, over, cx))
                            .ok();
                    }
                }),
                on_edit: Rc::new({
                    let this = this.clone();
                    move |position, window, cx| {
                        this.update(cx, |this, cx| this.edit_preview(ix, position, window, cx))
                            .ok();
                    }
                }),
                on_close: Rc::new(move |escape, window, cx| {
                    this.update(cx, |this, cx| this.close_preview(ix, escape, window, cx))
                        .ok();
                }),
            };
            out.push(crate::hover_preview::render(
                shown, ix, theme, &types, properties, handlers, window,
            ));
        }
        out
    }
}
