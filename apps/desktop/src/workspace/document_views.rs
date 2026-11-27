//! Loading files into Desktop views and restoring document/heading/footnote positions.

use super::{VaultState, Workspace, debug_status};
use crate::canvas::CanvasView;
use crate::reading::{self, LinkTarget};
use gpui::{ClipboardItem, Context};
use std::sync::Arc;
use std::time::Instant;
use tft_app::HistoryEntry;
use tft_formats::FileKind;
use tft_markdown::Subpath;

/// Removes `path`, and anything beneath it when it is a folder, from a
/// navigation history, answering where the one-based cursor lands. Entries
/// before the cursor carry it back with them, so back and forward still
/// reach the same notes they did.
pub(super) fn prune_history(history: &mut Vec<HistoryEntry>, ix: usize, path: &str) -> usize {
    let prefix = format!("{path}/");
    let mut cursor = ix;
    let mut seen = 0;
    history.retain(|entry| {
        let keep = entry.path != path && !entry.path.starts_with(&prefix);
        if !keep && seen < ix {
            cursor -= 1;
        }
        seen += 1;
        keep
    });
    cursor.min(history.len())
}

impl Workspace {
    pub(super) fn show_file(
        &mut self,
        path: &str,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        let previous = self.active_group_state.current.replace(path.to_string());
        if let Some(prev) = previous.clone() {
            if prev != path {
                self.save_folds(&prev, cx);
            }
        }
        if let Some(gone) = self.active_group_state.detached.take() {
            // The kept detached buffer's edits exist nowhere else; name what is dropped
            // rather than dropping it silently.
            let lost = {
                let e = self.active_editor().read(cx);
                e.is_dirty() && e.name.as_ref() != path
            };
            if lost {
                self.report(
                    format!(
                        "Discarded the unsaved edits in {gone}; nothing was saving that buffer"
                    ),
                    cx,
                );
            }
        }
        self.explorer.update(cx, |e, cx| e.follow(path, cx));
        self.graph
            .update(cx, |g, cx| g.set_current(Some(path.to_string()), cx));
        let kind = FileKind::from_path(path);
        if kind != FileKind::Canvas {
            self.release_canvas(cx);
        }
        if kind != FileKind::Pdf {
            self.release_pdf(cx);
        }
        match kind {
            FileKind::Canvas => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    // Nothing of a note may show through: the reading view is emptied, the editor let go.
                    self.active_reading().update(cx, |r, cx| {
                        r.set_note(
                            None,
                            reading::build_with("", "", state.as_ref(), state.build_options()),
                            cx,
                        )
                    });
                    self.release_buffer(cx);
                    // The view keeps its scene when the file is what it last read or wrote,
                    // keeps unsaved edits, and reloads otherwise.
                    let (same, matches, dirty) = {
                        let c = self.active_canvas().read(cx);
                        (c.path() == Some(path), c.matches_file(&text), c.is_dirty())
                    };
                    if same && matches {
                        // Own write or an unrelated event.
                    } else if same && dirty {
                        self.report(format!("{path} changed on disk; unsaved edits kept"), cx);
                    } else if same {
                        self.active_canvas().update(cx, |c, cx| c.reload(&text, cx));
                    } else {
                        // Where the tab last looked, when restored from the layout for this file.
                        let view_state = self
                            .active_group_state
                            .tabs
                            .get(self.active_group_state.active_tab)
                            .filter(|t| t.path.as_deref() == Some(path))
                            .and_then(|t| CanvasView::viewport_from_json(t.view_state.as_ref()));
                        let state = state.clone();
                        self.active_canvas()
                            .update(cx, |c, cx| c.load(path, &text, view_state, state, cx));
                    }
                    self.feedback.set_status(path.to_string());
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            FileKind::Markdown => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    // In an editing mode the reading model is built lazily -- on a switch
                    // to reading, a fold sync or a state dump -- not on every open.
                    let build_ms = if self.active_group_state.mode.is_editing() && !debug_status() {
                        let (path, text, state) = (path.to_string(), text.clone(), state.clone());
                        self.active_reading().update(cx, |r, cx| {
                            let build_options = state.build_options();
                            let state = state.clone();
                            r.set_note_later(
                                path,
                                text,
                                move |path, text| {
                                    crate::reading::build_with(
                                        path,
                                        text,
                                        state.as_ref(),
                                        build_options,
                                    )
                                },
                                cx,
                            )
                        });
                        0.0
                    } else {
                        let started = Instant::now();
                        let model =
                            reading::build_with(path, &text, state.as_ref(), state.build_options());
                        let build_ms = started.elapsed().as_secs_f64() * 1000.0;
                        self.active_reading()
                            .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
                        build_ms
                    };
                    // The editor keeps its buffer when the file is what it last saved (or
                    // still holds unsaved edits), and keeps its place when the note changed.
                    let (same, matches, dirty) = {
                        let e = self.active_editor().read(cx);
                        // A save's echo can arrive after the next edit: the buffer is dirty
                        // but the file is what it last wrote, so nothing changed underneath
                        // it (seen 2026-11-10).
                        let echo = e.is_dirty() && e.disk_unchanged(&text);
                        (
                            e.name.as_ref() == path,
                            e.matches_file(&text) || echo,
                            e.is_dirty(),
                        )
                    };
                    if same && matches {
                        // Own write or an unrelated event.
                    } else if same && dirty {
                        self.report(format!("{path} changed on disk; unsaved edits kept"), cx);
                        cx.notify();
                        return;
                    } else if same {
                        self.active_editor().update(cx, |e, cx| e.reload(text, cx));
                    } else {
                        self.active_editor()
                            .update(cx, |e, cx| e.set_text(path.to_string(), text.clone(), cx));
                        self.restore_folds(path, &text, state, cx);
                    }
                    // Build times stay off the status bar unless `TFT_DEBUG_STATUS` is set.
                    self.feedback.set_status(if debug_status() {
                        format!("{path}  ·  reading view built in {build_ms:.1} ms")
                    } else {
                        path.to_string()
                    });
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            // A PDF is drawn by its own view and never writes; a tab switch back to the
            // open file keeps its page and zoom, and the view remembers a file it showed
            // earlier this session.
            FileKind::Pdf => {
                // Nothing of a note may show through: reading emptied, editor let go.
                self.active_reading().update(cx, |r, cx| {
                    r.set_note(
                        None,
                        reading::build_with("", "", state.as_ref(), state.build_options()),
                        cx,
                    )
                });
                self.release_buffer(cx);
                if self.active_pdf().read(cx).path() != Some(path) {
                    let absolute = state.vault().absolute(path);
                    self.active_pdf()
                        .update(cx, |p, cx| p.load(path, absolute, cx));
                }
                self.feedback.set_status(path.to_string());
            }
            // An image, audio or video file is drawn as its own embed.
            FileKind::Image | FileKind::Audio | FileKind::Video => {
                let model = reading::build_with(
                    path,
                    &format!("![[{path}]]"),
                    state.as_ref(),
                    state.build_options(),
                );
                self.active_reading()
                    .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
                self.release_buffer(cx);
                self.feedback.set_status(path.to_string());
            }
            other => {
                // Nothing of the previous note may survive into a file no view can draw,
                // and the file's bytes must not reach the editor.
                self.active_reading().update(cx, |r, cx| {
                    r.set_note(
                        None,
                        reading::build_with("", "", state.as_ref(), state.build_options()),
                        cx,
                    )
                });
                self.release_buffer(cx);
                self.feedback
                    .set_status(format!("{path}: {other:?} files are not supported yet"));
            }
        }
        cx.notify();
    }

    /// A file `show_file` could not read. `current` must not name it while the editor
    /// holds another note, or edits get silently dropped; go back to the real note, or
    /// let the buffer go when there is none.
    pub(super) fn unreadable(
        &mut self,
        path: &str,
        previous: Option<String>,
        err: &std::io::Error,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        self.report(format!("Could not read {path}: {err}"), cx);
        self.active_group_state.current = previous.filter(|p| p.as_str() != path);
        match self.active_group_state.current.clone() {
            Some(back) => {
                self.explorer.update(cx, |e, cx| e.follow(&back, cx));
                self.graph.update(cx, |g, cx| g.set_current(Some(back), cx));
            }
            None => self.detach_current(true, state, cx),
        }
    }

    /// Lets go of the canvas on screen; its edits were flushed already, and otherwise
    /// `main_view` would keep painting a file that is no longer shown.
    pub(super) fn release_canvas(&mut self, cx: &mut Context<Self>) {
        if self.active_canvas().read(cx).path().is_some() {
            self.active_canvas().update(cx, |c, cx| c.release(cx));
        }
    }

    /// Lets go of the PDF on screen, if any, the same way. Nothing is
    /// flushed first: the view never writes.
    pub(super) fn release_pdf(&mut self, cx: &mut Context<Self>) {
        if self.active_pdf().read(cx).path().is_some() {
            self.active_pdf().update(cx, |p, cx| p.release(cx));
        }
    }

    /// Plain text of a cached heading (markup stripped).
    pub(super) fn heading_text(raw: &str) -> String {
        let doc = tft_markdown::parse(raw);
        match doc.blocks.first().map(|b| &b.kind) {
            Some(tft_markdown::BlockKind::Paragraph { content }) => {
                tft_markdown::plain_text(content)
            }
            _ => raw.to_string(),
        }
        .trim()
        .to_string()
    }

    /// Lets go of a buffer for a file the editor cannot save. A dirty buffer stays on
    /// screen as belonging to no file; a clean one goes.
    pub(super) fn release_buffer(&mut self, cx: &mut Context<Self>) {
        let (dirty, name) = {
            let editor = self.active_editor().read(cx);
            (editor.is_dirty(), editor.name.clone())
        };
        if dirty && !name.is_empty() {
            self.document_save.cancel();
            self.active_group_state.detached = Some(name);
            return;
        }
        self.clear_buffer(cx);
    }

    /// Empties the editor; keeping a canvas's or PDF's bytes would let the autosave
    /// write them through the markdown writer.
    pub(super) fn clear_buffer(&mut self, cx: &mut Context<Self>) {
        self.document_save.cancel();
        self.active_group_state.detached = None;
        self.active_editor()
            .update(cx, |e, cx| e.set_text("", String::new(), cx));
    }

    /// Lets go of the open note: no pane, graph or history may keep describing it.
    /// `keep_buffer` keeps a buffer whose edits exist nowhere else.
    pub(super) fn detach_current(
        &mut self,
        keep_buffer: bool,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        self.document_save.cancel();
        if keep_buffer {
            let name = self.active_editor().read(cx).name.clone();
            self.active_group_state.detached = (!name.is_empty()).then_some(name);
        } else {
            self.clear_buffer(cx);
        }
        self.release_canvas(cx);
        self.release_pdf(cx);
        self.active_reading().update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.active_group_state.current = None;
        // Nothing is open, so no row is the open file's; otherwise the next `set_tree`
        // would paint a path the vault no longer has.
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
    }

    /// Drops every row that describes the open note; panes are not refreshed on the way
    /// out, so a gone note's outline, backlinks and outgoing links would stay clickable.
    pub(super) fn forget_history(&mut self, path: &str) {
        self.active_group_state.history_ix = prune_history(
            &mut self.active_group_state.history,
            self.active_group_state.history_ix,
            path,
        );
        for tab in self.stashed_tabs_mut() {
            tab.history_ix = prune_history(&mut tab.history, tab.history_ix, path);
        }
    }

    pub(super) fn reveal_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.active_reading()
            .update(cx, |r, cx| r.scroll_to_offset(offset, cx));
        self.active_editor()
            .update(cx, |e, cx| e.reveal_offset(offset, cx));
        // An outline row or backlink moved the caret; the keyboard goes where the caret went.
        self.request_focus_main(cx);
    }

    pub(super) fn scroll_to_subpath(
        &mut self,
        path: &str,
        subpath: &Subpath<'_>,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        // `[[doc.pdf#page=3]]`: the link parser reads `page=3` as a heading, the PDF
        // view as a page; every `#` part is put back and parsed as pdf.js's hash.
        if FileKind::from_path(path) == FileKind::Pdf {
            if let Subpath::Headings(names) = subpath {
                let raw = names.join("#");
                // `#page=99`, `#page=0` and `#page=abc` leave the view put;
                // `selection=` marks the text and brings it near the top.
                let parsed = tft_pdf::parse_subpath(&raw);
                if let Some(page) = parsed.page {
                    if self.active_pdf().read(cx).path() == Some(path) {
                        self.active_pdf()
                            .update(cx, |p, cx| match parsed.selection {
                                Some(selection) => p.go_to_selection(page, selection, cx),
                                None => p.go_to_page(page, false, cx),
                            });
                    }
                }
            }
            return;
        }
        let offset = Self::subpath_offset(state, path, subpath);
        if let Some(offset) = offset {
            self.reveal_offset(offset, cx);
        }
    }

    /// Copy link/quote in a PDF: a wikilink to the file at the vault's link format,
    /// subpath and alias after it; the quote is `> text`, a blank line, the link.
    pub(super) fn copy_pdf_link(
        &mut self,
        path: &str,
        subpath: &str,
        alias: &str,
        quote: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.vault_session.state.clone() else {
            return;
        };
        let target =
            tft_cache::links::link_path(state.cache().resolver(), path, path, state.link_format());
        let link = format!("[[{target}#{subpath}|{alias}]]");
        let text = match quote {
            Some(text) => crate::pdf::selection::quote(text, &link),
            None => link,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Where a `#Heading` or `#^block` subpath starts in a note, by the cache.
    pub(super) fn subpath_offset(
        state: &VaultState,
        path: &str,
        subpath: &Subpath<'_>,
    ) -> Option<usize> {
        let cache = state.cache();
        let meta = cache.file(path)?;
        match subpath {
            Subpath::Headings(names) => {
                let last = names.last()?;
                meta.headings
                    .iter()
                    .find(|h| Self::heading_text(&h.heading).eq_ignore_ascii_case(last.trim()))
                    .map(|h| h.position.start.offset)
            }
            Subpath::Block(id) => meta
                .blocks
                .get(&id.to_lowercase())
                .or_else(|| meta.blocks.get(*id))
                .map(|b| b.position.start.offset),
        }
    }

    // ----- page preview ------------------------------------------------------------

    /// The window and note of the group whose reading view or editor is `view`.
    pub(super) fn follow_link(
        &mut self,
        target: LinkTarget,
        new_tab: bool,
        cx: &mut Context<Self>,
    ) {
        match target {
            LinkTarget::Internal(body) => {
                let Some(state) = self.vault_session.state.clone() else {
                    return;
                };
                let source = self.active_group_state.current.clone().unwrap_or_default();
                let parsed = tft_markdown::parse_linktext(&body);
                let resolved = state.cache().resolve(&body, &source).map(str::to_string);
                match resolved {
                    Some(path) if new_tab => self.open_link_in_new_tab(path, parsed.subpath, cx),
                    Some(path) => {
                        if path == source {
                            if let Some(sub) = parsed.subpath {
                                self.scroll_to_subpath(&path, &sub, &state, cx);
                            }
                        } else {
                            self.open_file(path, parsed.subpath, cx);
                        }
                    }
                    None => {
                        // Obsidian creates the note a dangling link points at.
                        if new_tab {
                            self.new_tab(cx);
                        }
                        let name = parsed.path.to_string();
                        self.create_note_for(&name, None, cx);
                    }
                }
            }
            LinkTarget::External(url) => cx.open_url(&url),
            LinkTarget::Tag(tag) => self.show_search(&format!("tag:#{tag}"), None, cx),
            LinkTarget::Footnote(n) => {
                // Obsidian's reference anchors to that one definition, not the note's
                // end; reveal it the same way a heading jump does.
                let at = self
                    .active_reading()
                    .read(cx)
                    .model()
                    .footnotes
                    .get(n.saturating_sub(1))
                    .copied();
                match at {
                    Some(offset) => self.reveal_offset(offset, cx),
                    None => self
                        .active_reading()
                        .update(cx, |r, cx| r.scroll_to_end(cx)),
                }
            }
        }
    }
}
