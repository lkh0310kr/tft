//! GPUI rendering of a reading-view [`Model`].
//!
//! Top-level blocks go through a lazily measured `list`; inline content is
//! one `InteractiveText` per run, with styled and clickable ranges.

use super::model::{
    Embed, EmbedKind, HtmlTableExtras, InlineStyle, LinkTarget, Model, RBlock, RItem, RichText,
    Script, Segment, TaskMarker,
};
use crate::assets::{callout_icon, icon};
use crate::fold::{self, FoldKey};
use crate::keymap;
use crate::math::{self, FormulaElement, Inline, Overlay, ScriptText, SpaceWidths};
use crate::properties::{Field, Part, Target};
use crate::properties::{PROPERTIES_FOLD_ID, PROPERTY_ID_BASE};
use crate::theme::Theme;
use gpui::{
    actions, canvas, deferred, div, img, list, prelude::*, px, AnyElement, App, Bounds,
    ClipboardItem, Context, CursorStyle, DispatchPhase, DragMoveEvent, Element, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, GlobalElementId,
    HighlightStyle, Hitbox, HitboxBehavior, Hsla, InspectorElementId, InteractiveText, KeyBinding,
    LayoutId, ListAlignment, ListOffset, ListState, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ObjectFit, Pixels, Point, SharedString, SharedUri, StrikethroughStyle, StyledText,
    Task, TextLayout, TextRun, UnderlineStyle, Window, WindowTextSystem,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tft_formats::config::PropertyTypes;
use tft_formats::properties::{PropertiesInDocument, Property, PropertyType};
use tft_formats::FileKind;
use tft_highlight::{highlight, Paint};
use tft_markdown::Alignment;

actions!(
    reading,
    [
        ScrollPageDown,
        ScrollPageUp,
        ScrollLineDown,
        ScrollLineUp,
        ScrollToTop,
        ScrollToBottom,
        SelectAll,
        Copy,
    ]
);

/// Binds the reading view's keys in its own context; call once at startup.
/// Obsidian has no reading-view commands, so these are a browser scroller's.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("ReadingView");
    cx.bind_keys([
        KeyBinding::new("pagedown", ScrollPageDown, ctx),
        KeyBinding::new("pageup", ScrollPageUp, ctx),
        KeyBinding::new("space", ScrollPageDown, ctx),
        KeyBinding::new("shift-space", ScrollPageUp, ctx),
        KeyBinding::new("down", ScrollLineDown, ctx),
        KeyBinding::new("up", ScrollLineUp, ctx),
        KeyBinding::new("home", ScrollToTop, ctx),
        KeyBinding::new("end", ScrollToBottom, ctx),
    ]);
    // Mod: Cmd on macOS, Ctrl elsewhere (see `keymap`).
    keymap::bind_mod(cx, ctx, "a", SelectAll);
    keymap::bind_mod(cx, ctx, "c", Copy);
    // Either end of the note: Cmd+↑/↓ on macOS, Ctrl+Home/End elsewhere.
    #[cfg(any(target_os = "macos", target_arch = "wasm32"))]
    cx.bind_keys([
        KeyBinding::new("cmd-up", ScrollToTop, ctx),
        KeyBinding::new("cmd-down", ScrollToBottom, ctx),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-home", ScrollToTop, ctx),
        KeyBinding::new("ctrl-end", ScrollToBottom, ctx),
    ]);
}

/// The line height as a multiple of the base font size, and so the line an
/// arrow key scrolls.
pub const LINE_HEIGHT: f32 = 1.6;

/// Pixels a page key scrolls: the viewport less an eighth, capped at 40 px, as
/// a browser does. A viewport of zero is unpainted and scrolls nothing.
pub(crate) fn page_distance(viewport: f32) -> f32 {
    if viewport <= 0. {
        return 0.;
    }
    viewport - (viewport / 8.).min(40.)
}

/// Pixels an arrow key scrolls: one rendered line.
pub(crate) fn line_distance(base_font_size: f32) -> f32 {
    base_font_size * LINE_HEIGHT
}

/// Whether `next` reloads the note shown, keeping place and folds; another note, or none, drops both.
pub(crate) fn same_note(current: Option<&str>, next: Option<&str>) -> bool {
    next.is_some() && current == next
}

/// The pane body's top padding, which holds the inline title when one shows.
pub(crate) const HEADER_TOP: f32 = 24.;

/// The inline title above a markdown note: its basename without extension. An embedded image shows none.
pub(crate) fn inline_title(path: &str) -> Option<&str> {
    if !FileKind::from_path(path).is_markdown() {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    Some(name.rsplit_once('.').map_or(name, |(stem, _)| stem))
}

/// The title the header shows: none unless `showInlineTitle` is on and a note is open.
pub(crate) fn shown_title(show: bool, path: Option<&str>) -> Option<&str> {
    if !show {
        return None;
    }
    path.and_then(inline_title)
}

/// Height of the header row: the top padding plus the inline title's h1
/// (Obsidian's 1.618em at 700 on a 1.2 line, 0.5em below). The editor reserves
/// the same, so a mode switch does not move the text.
pub(crate) fn header_height(base: f32, title: bool) -> f32 {
    let (size, _, line_height) = crate::theme::heading_style(1);
    let h1 = base * size;
    HEADER_TOP
        + if title {
            h1 * line_height + h1 * 0.5
        } else {
            0.
        }
}

// ----- selection ---------------------------------------------------------------

/// A place in the reading view: a block, a [`RichText`] and a byte offset — the
/// triple a DOM selection's node and offset map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadingPos {
    /// Index into [`Model::blocks`].
    pub block: usize,
    /// [`RichText::id`], unique within one model.
    pub rich: usize,
    /// Byte offset into that run's text.
    pub offset: usize,
}

/// One [`RichText`] of a note in document order, which a selection can cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run<'a> {
    pub block: usize,
    pub rich: usize,
    pub text: &'a str,
    /// What precedes this run when joined: a newline, or a tab between table cells.
    pub before: &'static str,
}

/// Every [`RichText`] of `model` in document order with its block, walking the
/// model as the renderer draws it. Blocks with no inline model — code, raw HTML,
/// display math, properties — carry nothing and cannot be selected.
pub(crate) fn runs(model: &Model) -> Vec<Run<'_>> {
    let mut out = Vec::new();
    for (ix, block) in model.blocks.iter().enumerate() {
        collect_runs(std::slice::from_ref(block), ix, &mut out);
    }
    out
}

fn collect_runs<'a>(blocks: &'a [RBlock], block: usize, out: &mut Vec<Run<'a>>) {
    let push = |text: &'a RichText, before: &'static str, out: &mut Vec<Run<'a>>| {
        out.push(Run {
            block,
            rich: text.id,
            text: &text.text,
            before,
        });
    };
    for b in blocks {
        match b {
            // Segments are laid out in a column, so each text segment starts its own line.
            RBlock::Paragraph(segments) => {
                for segment in segments {
                    match segment {
                        Segment::Text(text) => push(text, "\n", out),
                        Segment::Embed(embed) => {
                            if let EmbedKind::Note { blocks, .. } = &embed.kind {
                                collect_runs(blocks, block, out);
                            }
                        }
                        Segment::Math(_) => {}
                    }
                }
            }
            RBlock::Heading { text, .. } => push(text, "\n", out),
            RBlock::Quote(children) => collect_runs(children, block, out),
            RBlock::Callout {
                title, children, ..
            } => {
                push(title, "\n", out);
                collect_runs(children, block, out);
            }
            RBlock::List { items, .. } => {
                for item in items {
                    collect_runs(&item.children, block, out);
                }
            }
            RBlock::Table { header, rows, .. } => {
                for row in std::iter::once(header).chain(rows.iter()) {
                    for (at, cell) in row.iter().enumerate() {
                        push(cell, if at == 0 { "\n" } else { "\t" }, out);
                    }
                }
            }
            RBlock::Footnotes(notes) => {
                for (_, body) in notes {
                    collect_runs(body, block, out);
                }
            }
            RBlock::Details { summary, body, .. } => {
                push(summary, "\n", out);
                collect_runs(body, block, out);
            }
            RBlock::Properties(_)
            | RBlock::ThematicBreak
            | RBlock::Code { .. }
            | RBlock::Html(_)
            | RBlock::Math(_) => {}
        }
    }
}

/// Where `pos` sits in `runs`, by block and run id.
fn run_index(runs: &[Run<'_>], pos: ReadingPos) -> Option<usize> {
    runs.iter()
        .position(|r| r.block == pos.block && r.rich == pos.rich)
}

/// The two ends in document order; `None` if either names a run this model lacks.
pub(crate) fn ordered(
    runs: &[Run<'_>],
    anchor: ReadingPos,
    head: ReadingPos,
) -> Option<(ReadingPos, ReadingPos)> {
    let a = run_index(runs, anchor)?;
    let b = run_index(runs, head)?;
    Some(if (a, anchor.offset) <= (b, head.offset) {
        (anchor, head)
    } else {
        (head, anchor)
    })
}

/// The selected range of each covered run, by index into `runs`; empty runs are left out.
pub(crate) fn selected_ranges(
    runs: &[Run<'_>],
    anchor: ReadingPos,
    head: ReadingPos,
) -> Vec<(usize, Range<usize>)> {
    let Some((start, end)) = ordered(runs, anchor, head) else {
        return Vec::new();
    };
    let (first, last) = match (run_index(runs, start), run_index(runs, end)) {
        (Some(a), Some(b)) => (a, b),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for (at, run) in runs.iter().enumerate().take(last + 1).skip(first) {
        let text = run.text;
        let from = if at == first {
            floor_boundary(text, start.offset)
        } else {
            0
        };
        let to = if at == last {
            floor_boundary(text, end.offset)
        } else {
            text.len()
        };
        if from < to {
            out.push((at, from..to));
        }
    }
    out
}

/// The plain text a selection yields, joined as the reader sees it: what a
/// browser copy gives, with no source markers.
pub(crate) fn selected_text(runs: &[Run<'_>], anchor: ReadingPos, head: ReadingPos) -> String {
    let mut out = String::new();
    for (n, (at, range)) in selected_ranges(runs, anchor, head).into_iter().enumerate() {
        if n > 0 {
            out.push_str(runs[at].before);
        }
        out.push_str(&runs[at].text[range]);
    }
    out
}

/// Anchor and head covering every run, for Ctrl+A; `None` when nothing is selectable.
pub(crate) fn whole_note(runs: &[Run<'_>]) -> Option<(ReadingPos, ReadingPos)> {
    let first = runs.first()?;
    let last = runs.last()?;
    Some((
        ReadingPos {
            block: first.block,
            rich: first.rich,
            offset: 0,
        },
        ReadingPos {
            block: last.block,
            rich: last.rich,
            offset: last.text.len(),
        },
    ))
}

/// The range a double-click at `offset` takes: a word, or the whitespace or punctuation there.
pub(crate) fn word_at(text: &str, offset: usize) -> Range<usize> {
    let at = floor_boundary(text, offset);
    if text.is_empty() {
        return 0..0;
    }
    let wordish = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    // A press at the very end of the text takes the word before it.
    let probe = if at == text.len() {
        text[..at].chars().next_back()
    } else {
        text[at..].chars().next()
    };
    let word = probe.is_some_and(wordish);
    let like = |c: char| wordish(c) == word;
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| like(*c))
        .map(|(i, _)| i)
        .last()
        .unwrap_or(at);
    let end = text[at..]
        .char_indices()
        .take_while(|(_, c)| like(*c))
        .map(|(i, c)| at + i + c.len_utf8())
        .last()
        .unwrap_or(at);
    start..end
}

/// The largest char boundary at or below `at`, so a hit-test offset cannot slice a run.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// One end of a selection, for the control socket's state dump.
fn describe_pos(pos: ReadingPos) -> Value {
    json!({"block": pos.block, "rich": pos.rich, "offset": pos.offset})
}

/// What the reader did over a run of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectGesture {
    /// Left button down: the anchor and the head go here.
    Press,
    /// Moved with the left button down: the head follows.
    Drag,
    /// Double-click: the word under the pointer.
    Word,
    /// Triple-click: the whole run, which is Obsidian's paragraph.
    Line,
}

/// Told which run a gesture landed on and the byte offset in it.
pub type SelectHandler = Rc<dyn Fn(SelectGesture, usize, usize, &mut Window, &mut App)>;

/// Lets the drawn text be selected. Live Preview passes none: its selection covers the widgets.
pub(crate) struct SelectHost {
    /// The selected range of each [`RichText`], by id, painted behind the text.
    pub ranges: Rc<HashMap<usize, Range<usize>>>,
    pub color: Hsla,
    pub on_select: SelectHandler,
}

/// Maps a paragraph's bytes to its laid-out string, where formulas and scripted
/// runs stand as placeholders; each edit is a range and its new length.
#[derive(Debug, Clone, Default)]
pub(crate) struct Remap {
    edits: Vec<(Range<usize>, usize)>,
}

impl Remap {
    /// A paragraph offset in the laid-out string; inside a placeholder, its start.
    pub(crate) fn to_shaped(&self, offset: usize) -> usize {
        let mut shift = 0isize;
        for (range, len) in &self.edits {
            if offset <= range.start {
                break;
            }
            if offset < range.end {
                return (range.start as isize + shift) as usize;
            }
            shift += *len as isize - range.len() as isize;
        }
        (offset as isize + shift) as usize
    }

    /// A laid-out offset back in the paragraph; inside a placeholder, its nearer end.
    pub(crate) fn to_model(&self, shaped: usize) -> usize {
        let mut shift = 0isize;
        for (range, len) in &self.edits {
            let start = (range.start as isize + shift) as usize;
            if shaped <= start {
                break;
            }
            if shaped < start + len {
                return if (shaped - start) * 2 < *len {
                    range.start
                } else {
                    range.end
                };
            }
            shift += *len as isize - range.len() as isize;
        }
        (shaped as isize - shift).max(0) as usize
    }
}

/// The byte offset a point fell on in a laid-out run: exact inside the text, else nearest.
fn index_at(layout: &gpui::TextLayout, position: gpui::Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(at) | Err(at) => at,
    }
}

/// The wash behind selected text: the accent at the editor selection's alpha.
pub(crate) fn selection_color(theme: &Theme) -> Hsla {
    let mut color = theme.tokens.interactive_accent;
    color.a = if theme.dark { 0.35 } else { 0.25 };
    color
}

/// Whether the first paragraph's first strong character is right to left, as `dir="auto"` does.
fn blocks_rtl(blocks: &[RBlock]) -> bool {
    blocks.iter().find_map(|b| match b {
        RBlock::Paragraph(segments) => segments.iter().find_map(|s| match s {
            Segment::Text(t) => tft_text::bidi::strong_direction(&t.text),
            _ => None,
        }),
        _ => None,
    }) == Some(tft_text::bidi::Direction::Rtl)
}

/// Merges a selection's wash into a text's style runs, splitting runs at the
/// ends to keep the list sorted and non-overlapping for `with_highlights`.
pub(crate) fn with_selection(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    text: &str,
    selection: Range<usize>,
    color: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let len = text.len();
    // Hit tests may land inside a placeholder, so both ends walk back to a char boundary.
    let selection = floor_boundary(text, selection.start)..floor_boundary(text, selection.end);
    if selection.is_empty() {
        return highlights;
    }
    let mut cuts: Vec<usize> = vec![0, len, selection.start, selection.end];
    for (range, _) in &highlights {
        cuts.push(range.start.min(len));
        cuts.push(range.end.min(len));
    }
    cuts.retain(|at| *at <= len);
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::with_capacity(cuts.len());
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let base = highlights
            .iter()
            .find(|(range, _)| range.start <= from && to <= range.end)
            .map(|(_, style)| *style);
        let inside = selection.start <= from && to <= selection.end;
        let Some(mut style) = base.or(if inside {
            Some(HighlightStyle::default())
        } else {
            None
        }) else {
            continue;
        };
        if inside {
            style.background_color = Some(color);
        }
        out.push((from..to, style));
    }
    out
}

/// Something the reader did to a property row. Live Preview edits the note; the
/// reading view raises only [`PropertyAction::RemoveItem`].
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyAction {
    /// Start typing in a row's key or value.
    Edit {
        target: Target,
        part: Part,
    },
    /// Start typing in one item; `index` past the last is the empty input after the pills.
    EditItem {
        key: String,
        index: usize,
    },
    /// A pill's own remove control: drop that item and leave the rest.
    RemoveItem {
        key: String,
        index: usize,
    },
    /// Flip a checkbox row.
    Toggle {
        key: String,
    },
    Remove {
        key: String,
    },
    Retype {
        key: String,
        ty: PropertyType,
    },
    /// Open an unnamed row at the end of the block.
    AddRow,
    /// Show the type menu for a row.
    OpenTypeMenu {
        key: String,
    },
}

/// Lets the properties block be edited: Live Preview passes the open field and a
/// sink, the reading view only the pills' remove control.
pub struct PropertiesHost {
    /// The field being typed into, when it belongs to this block.
    pub field: Option<Field>,
    /// The pill a Backspace selected: the first of two removing presses focuses
    /// it, the second removes it.
    pub selected_pill: Option<usize>,
    /// `propertiesInDocument`: `Hidden` draws nothing at all.
    pub mode: PropertiesInDocument,
    /// The key whose type menu is open, if any.
    pub menu: Option<String>,
    /// Whether the foldable "Properties" heading is drawn; the File pane drops it.
    pub heading: bool,
    pub on_action: PropertyHandler,
}

/// What a property row's affordances do when clicked.
pub type PropertyHandler = Rc<dyn Fn(PropertyAction, &mut Window, &mut App)>;

/// Something the reader did to a table cell; only Live Preview turns these into edits.
#[derive(Debug, Clone, PartialEq)]
pub enum TableAction {
    /// Start typing in the cell standing for these bytes.
    Edit { span: Range<usize> },
    /// A press on a handle selects that line; `table` is named by its first cell's start.
    Select {
        table: usize,
        axis: TableAxis,
        index: usize,
    },
    /// A held line would land before `index`, or after it when `after` is set.
    DragOver {
        table: usize,
        axis: TableAxis,
        from: usize,
        index: usize,
        after: bool,
    },
    /// A click on the "+" bar under (`Row`) or beside (`Col`) the table appends an empty line.
    Append { table: usize, axis: TableAxis },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAxis {
    Row,
    Col,
}

/// The line a handle holds, carried by gpui's drag; the document keeps where it lands.
#[derive(Clone)]
pub struct TableLineDrag {
    pub table: usize,
    pub axis: TableAxis,
    pub from: usize,
}

/// A dragged handle's ghost: Obsidian draws none, so this is empty.
pub struct TableGhost;

impl Render for TableGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}

/// The row or column a handle press selected, drawn outlined and tinted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSelect {
    pub table: usize,
    pub axis: TableAxis,
    pub index: usize,
}

/// A handle's line being dragged and the gap it is held over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableDragView {
    pub table: usize,
    pub axis: TableAxis,
    pub from: usize,
    pub target: Option<(usize, bool)>,
}

/// What a table cell does when clicked.
pub type TableHandler = Rc<dyn Fn(TableAction, &mut Window, &mut App)>;

/// Lets a table's cells be edited: Live Preview passes the open cell and a sink.
pub struct TableHost {
    /// The open cell's bytes and the text in hand, not yet written to the document.
    pub cell: Option<(Range<usize>, Field)>,
    /// The row or column a handle press selected, and the drag in progress.
    pub select: Option<TableSelect>,
    pub drag: Option<TableDragView>,
    pub on_action: TableHandler,
}

/// What a rendered link does when clicked.
pub type LinkHandler = Rc<dyn Fn(LinkTarget, &mut Window, &mut App)>;
/// What a rendered link does when right-clicked, at the click's position.
pub type MenuHandler = Rc<dyn Fn(LinkTarget, Point<Pixels>, &mut Window, &mut App)>;
/// The pointer on a rendered link for page preview: its first glyph's line box
/// (the popover anchors to the element's left edge and bottom) and the modifiers.
#[derive(Clone, Debug)]
pub struct LinkHover {
    pub target: LinkTarget,
    pub anchor: Bounds<Pixels>,
    pub modifiers: gpui::Modifiers,
}
/// Told when the pointer enters a rendered link (`Some`) or leaves it (`None`).
pub type HoverHandler = Rc<dyn Fn(Option<LinkHover>, &mut Window, &mut App)>;
/// A host's hover reporting: the handler and the `(RichText::id, link index)` on
/// the pointer, so only the text that reported a link reports leaving it.
#[derive(Clone)]
pub struct HoverHost {
    pub on_hover: HoverHandler,
    pub current: Rc<std::cell::Cell<Option<(usize, usize)>>>,
}
/// What a host draws under a note at the width given (the readable line or the
/// view less padding): the backlinks block of
/// `backlink:toggle-backlinks-in-document`, in `.cm-sizer` or `.mod-footer`.
/// Built during layout, so it reads the host as it is that frame.
pub type FooterBuilder = Rc<dyn Fn(f32, &mut Window, &mut App) -> Option<AnyElement>>;
/// What a foldable callout's title does when clicked, by title id.
pub type ToggleHandler = Rc<dyn Fn(usize, &mut App)>;
/// What a task checkbox does when clicked.
pub type TaskHandler = Rc<dyn Fn(TaskMarker, &mut App)>;
/// Told which code block's copy button was pressed, so its check can show.
pub type CopyHandler = Rc<dyn Fn(usize, &mut App)>;

/// The settings and affordances only the reading view has.
pub struct ReadingHost {
    /// `propertiesInDocument`: `Hidden` draws no properties block.
    pub properties: PropertiesInDocument,
    /// The code block whose copy button is showing its check.
    pub copied: Option<usize>,
    pub on_copy: CopyHandler,
    pub on_task: TaskHandler,
    pub on_link: LinkHandler,
    /// Pressed link shared across paint frames so a fast browser click can
    /// resolve its release without waiting for another animation frame.
    pub link_pressed: Rc<std::cell::Cell<Option<(usize, usize)>>>,
    /// A pill's remove control, handed [`PropertyAction::RemoveItem`] only; the
    /// rest stays read-only. `None` for a host that cannot write the note.
    pub on_remove_item: Option<PropertyHandler>,
    /// What a right click on a rendered link raises: the file menu here, nothing elsewhere.
    pub on_menu: Option<MenuHandler>,
    /// Page preview's hover; `None` reports nothing (a canvas card, the popover).
    pub hover: Option<HoverHost>,
}

pub enum ReadingEvent {
    OpenLink(LinkTarget),
    /// A Ctrl+click on a link: it opens in a new tab.
    OpenLinkInNewTab(LinkTarget),
    /// A right click on a rendered link: the file menu, at the click — what
    /// Obsidian's reading view raises and nothing else
    /// ([[Specification#Observed editor context menu and view header actions]]).
    ContextMenu {
        target: LinkTarget,
        position: Point<Pixels>,
    },
    /// The pointer came onto a rendered link, or left it (`None`).
    Hover(Option<LinkHover>),
}

/// A reader clicked a task checkbox; the workspace makes the edit Live Preview would.
pub struct TaskToggled {
    /// The note the view showed, so an event that outlived it writes nothing.
    pub path: String,
    /// Where the three bytes `[`, status, `]` are in it.
    pub marker: TaskMarker,
}

/// A reader clicked a list pill's remove control; as with [`TaskToggled`], the
/// workspace makes the edit.
pub struct PropertyItemRemoved {
    /// The note the view showed, so an event that outlived it writes nothing.
    pub path: String,
    /// The property the pill belongs to.
    pub key: String,
    /// The pill's index among the list's items.
    pub index: usize,
}

pub struct ReadingView {
    focus_handle: FocusHandle,
    pub path: Option<String>,
    model: Arc<Model>,
    list: ListState,
    /// What the host draws under the last block, the list's one item past the blocks.
    footer: Option<FooterBuilder>,
    /// Callouts whose fold state the reader flipped, by title id.
    toggled: HashSet<usize>,
    /// Heading and list folds by key, shared with the editor; only headings collapse here.
    folds: Vec<FoldKey>,
    /// `foldHeading`: with it off the indicator is not drawn.
    fold_headings: bool,
    /// `showIndentGuide` and `rightToLeft`, for the renderer.
    chrome: crate::gutter::Chrome,
    /// Per top-level block: inside a collapsed heading's section.
    hidden: Vec<bool>,
    readable_line_length: bool,
    /// `showInlineTitle`: the note's name at h1 size above its first block.
    show_inline_title: bool,
    properties_in_document: PropertiesInDocument,
    /// The vault's `types.json`, for the type each property row shows.
    property_types: PropertyTypes,
    /// The code block showing its check, and the timer that takes it away.
    copied: Option<usize>,
    flash: Option<Task<()>>,
    /// Width of the view as last painted; tables lay their columns out for it.
    width: Pixels,
    /// The selection's anchor and head; dropped when the note changes.
    selection: Option<(ReadingPos, ReadingPos)>,
    /// Whether a press landed on text and the button is still down.
    dragging: bool,
    /// The link page preview last heard of, shared with every text (see [`HoverHost`]).
    hover_current: Rc<std::cell::Cell<Option<(usize, usize)>>>,
    link_pressed: Rc<std::cell::Cell<Option<(usize, usize)>>>,
    /// Each run's selected range, by [`RichText::id`], derived from `selection` on every change.
    selected: Rc<HashMap<usize, Range<usize>>>,
    /// A note handed over in an editing mode, built only when something reads it
    /// (first render, fold sync, control socket).
    pending: Option<(String, String, Box<dyn FnOnce(&str, &str) -> Model>)>,
    /// The note `model` was built from, which a reload compares against.
    built_path: Option<String>,
}

impl EventEmitter<ReadingEvent> for ReadingView {}
impl EventEmitter<TaskToggled> for ReadingView {}
impl EventEmitter<PropertyItemRemoved> for ReadingView {}

impl Focusable for ReadingView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// A fenced block as one styled run per token: `highlight` ports Prism, whose
/// flat runs come in painting order (nested after parent), so applying them in
/// sequence leaves the innermost showing. No grammar means a plain block.
fn code_text(text: &str, language: Option<&str>, theme: &Theme) -> AnyElement {
    let runs = highlight(text, language);
    if runs.is_empty() {
        return SharedString::from(text.to_string()).into_any_element();
    }
    let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = runs
        .into_iter()
        .map(|run| {
            (
                run.start..run.end,
                HighlightStyle {
                    color: Some(code_colour(run.style.paint, theme)),
                    font_weight: run.style.bold.then_some(FontWeight::BOLD),
                    font_style: run.style.italic.then_some(FontStyle::Italic),
                    ..Default::default()
                },
            )
        })
        .collect();
    StyledText::new(SharedString::from(text.to_string()))
        .with_highlights(highlights)
        .into_any_element()
}

/// The registry token a highlighted run is painted from.
fn code_colour(paint: Paint, theme: &Theme) -> Hsla {
    let t = &theme.tokens;
    match paint {
        Paint::Normal => t.code_normal,
        Paint::Comment => t.code_comment,
        Paint::Keyword => t.code_keyword,
        Paint::String => t.code_string,
        Paint::Function => t.code_function,
        Paint::Value => t.code_value,
        Paint::Operator => t.code_operator,
        Paint::Punctuation => t.code_punctuation,
        Paint::Property => t.code_property,
        Paint::Tag => t.code_tag,
        Paint::Important => t.code_important,
    }
}

impl ReadingView {
    pub fn new(cx: &mut Context<Self>) -> ReadingView {
        ReadingView {
            focus_handle: cx.focus_handle(),
            path: None,
            model: Arc::new(Model::default()),
            list: ListState::new(0, ListAlignment::Top, px(1200.)),
            footer: None,
            toggled: HashSet::new(),
            folds: Vec::new(),
            fold_headings: true,
            chrome: crate::gutter::Chrome::default(),
            hidden: Vec::new(),
            readable_line_length: true,
            show_inline_title: true,
            properties_in_document: PropertiesInDocument::default(),
            property_types: PropertyTypes::default(),
            copied: None,
            flash: None,
            width: px(900.),
            selection: None,
            dragging: false,
            hover_current: Rc::default(),
            link_pressed: Rc::default(),
            selected: Rc::new(HashMap::new()),
            pending: None,
            built_path: None,
        }
    }

    /// Hands the view a note without building it: built on first read. The host supplies
    /// a builder so Vault-specific IO and configuration stay outside the shared view.
    pub fn set_note_later(
        &mut self,
        path: String,
        text: String,
        build: impl FnOnce(&str, &str) -> Model + 'static,
        cx: &mut Context<Self>,
    ) {
        if !same_note(self.built_path.as_deref(), Some(&path)) {
            self.model = Arc::new(Model::default());
            self.list.reset(0);
            self.built_path = None;
        }
        self.path = Some(path.clone());
        self.pending = Some((path, text, Box::new(build)));
        cx.notify();
    }

    /// Builds the note handed over by [`ReadingView::set_note_later`], if any.
    pub fn ensure_built(&mut self, cx: &mut Context<Self>) {
        if let Some((path, text, build)) = self.pending.take() {
            let model = build(&path, &text);
            self.set_note(Some(path), model, cx);
        }
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Forgets the link the pointer was last reported on (page preview).
    pub fn clear_hover(&mut self) {
        self.hover_current.set(None);
    }

    pub fn set_readable_line_length(&mut self, on: bool) {
        self.readable_line_length = on;
    }

    pub fn set_show_inline_title(&mut self, on: bool) {
        self.show_inline_title = on;
    }

    pub fn set_properties_in_document(&mut self, mode: PropertiesInDocument) {
        self.properties_in_document = mode;
    }

    /// The inline title over the blocks, if the setting and the file allow one.
    pub fn inline_title(&self) -> Option<&str> {
        shown_title(self.show_inline_title, self.path.as_deref())
    }

    pub fn set_property_types(&mut self, types: PropertyTypes) {
        self.property_types = types;
    }

    pub fn set_note(&mut self, path: Option<String>, model: Model, cx: &mut Context<Self>) {
        let same = same_note(self.built_path.as_deref(), path.as_deref());
        let top = self.list.logical_scroll_top();
        self.pending = None;
        self.built_path = path.clone();
        self.path = path;
        self.list
            .reset(model.blocks.len() + usize::from(self.footer.is_some()));
        if same {
            // A reload after a change on disk keeps the reader's place.
            self.list.scroll_to(top);
        }
        self.model = Arc::new(model);
        if !same {
            // Folds go too: the watcher re-enters this on every autosave, which
            // once collapsed open callouts after each save.
            self.toggled.clear();
            self.folds.clear();
        }
        self.recompute_hidden();
        self.copied = None;
        // A selection names old-model runs, so it goes even on a reload.
        self.selection = None;
        self.dragging = false;
        self.selected = Rc::new(HashMap::new());
        cx.notify();
    }

    // ----- selection ------------------------------------------------------------

    /// Stores a selection and derives the per-run ranges the blocks paint.
    fn set_selection(
        &mut self,
        selection: Option<(ReadingPos, ReadingPos)>,
        cx: &mut Context<Self>,
    ) {
        let model = self.model.clone();
        let runs = runs(&model);
        let mut ranges = HashMap::new();
        let selection = selection.filter(|(a, h)| ordered(&runs, *a, *h).is_some());
        if let Some((anchor, head)) = selection {
            for (at, range) in selected_ranges(&runs, anchor, head) {
                ranges.insert(runs[at].rich, range);
            }
        }
        self.selection = selection;
        self.selected = Rc::new(ranges);
        cx.notify();
    }

    /// Records what the reader did over a run of text.
    fn gesture(&mut self, gesture: SelectGesture, pos: ReadingPos, cx: &mut Context<Self>) {
        match gesture {
            SelectGesture::Press => {
                self.dragging = true;
                self.set_selection(Some((pos, pos)), cx);
            }
            SelectGesture::Drag => {
                // Most drag events land on the byte the last one did.
                if self.dragging {
                    if let Some((anchor, head)) = self.selection {
                        if head != pos {
                            self.set_selection(Some((anchor, pos)), cx);
                        }
                    }
                }
            }
            SelectGesture::Word | SelectGesture::Line => {
                let model = self.model.clone();
                let runs = runs(&model);
                let Some(at) = runs
                    .iter()
                    .position(|r| r.block == pos.block && r.rich == pos.rich)
                else {
                    return;
                };
                let range = if gesture == SelectGesture::Word {
                    word_at(runs[at].text, pos.offset)
                } else {
                    0..runs[at].text.len()
                };
                self.dragging = false;
                self.set_selection(
                    Some((
                        ReadingPos {
                            offset: range.start,
                            ..pos
                        },
                        ReadingPos {
                            offset: range.end,
                            ..pos
                        },
                    )),
                    cx,
                );
            }
        }
    }

    /// A press that landed on no text drops the selection, as a browser does.
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_some() {
            self.set_selection(None, cx);
        }
        self.dragging = false;
    }

    pub fn selected_text(&self) -> String {
        let Some((anchor, head)) = self.selection else {
            return String::new();
        };
        selected_text(&runs(&self.model), anchor, head)
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let whole = whole_note(&runs(&model));
        self.dragging = false;
        self.set_selection(whole, cx);
    }

    /// Ctrl+C: the selection as plain text; with nothing selected the clipboard is left alone.
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    // ----- heading folds --------------------------------------------------------

    /// Every top-level heading: its block index, level and key.
    fn headings(&self) -> Vec<(usize, u8, FoldKey)> {
        let heads: Vec<(usize, u8, &str)> = self
            .model
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(ix, b)| match b {
                RBlock::Heading { level, plain, .. } => Some((ix, *level, plain.as_str())),
                _ => None,
            })
            .collect();
        let keys = fold::heading_keys(heads.iter().map(|(_, l, t)| (*l, *t)));
        heads
            .into_iter()
            .zip(keys)
            .map(|((ix, level, _), key)| (ix, level, key))
            .collect()
    }

    /// Marks the blocks a collapsed heading hides, up to the next of the same or
    /// higher level; absent, as they are from Obsidian's DOM.
    fn recompute_hidden(&mut self) {
        let heads = self.headings();
        let mut hidden = vec![false; self.model.blocks.len()];
        for (i, (ix, level, key)) in heads.iter().enumerate() {
            if !self.folds.contains(key) {
                continue;
            }
            let end = heads[i + 1..]
                .iter()
                .find(|(_, l, _)| l <= level)
                .map(|(next, _, _)| *next)
                .unwrap_or(hidden.len());
            for h in hidden.iter_mut().take(end).skip(ix + 1) {
                *h = true;
            }
        }
        self.hidden = hidden;
    }

    fn toggle_heading(&mut self, key: FoldKey, cx: &mut Context<Self>) {
        match self.folds.iter().position(|k| *k == key) {
            Some(at) => {
                self.folds.remove(at);
            }
            None => self.folds.push(key),
        }
        self.recompute_hidden();
        cx.notify();
    }

    /// The folds by key, for the editor to take over on a mode switch.
    pub fn fold_keys(&self) -> Vec<FoldKey> {
        self.folds.clone()
    }

    pub fn set_fold_keys(&mut self, keys: Vec<FoldKey>, cx: &mut Context<Self>) {
        self.ensure_built(cx);
        self.folds = keys;
        self.recompute_hidden();
        cx.notify();
    }

    pub fn set_chrome(&mut self, chrome: crate::gutter::Chrome) {
        self.chrome = chrome;
    }

    pub fn set_fold_headings(&mut self, on: bool) {
        self.fold_headings = on;
    }

    pub fn properties_folded(&self) -> bool {
        self.toggled.contains(&PROPERTIES_FOLD_ID)
    }

    pub fn set_properties_folded(&mut self, on: bool, cx: &mut Context<Self>) {
        if on != self.properties_folded() {
            self.toggle_callout(PROPERTIES_FOLD_ID, cx);
        }
    }

    /// `editor:toggle-fold-properties` from a preview leaf, which Obsidian honours.
    pub fn toggle_properties_fold(&mut self, cx: &mut Context<Self>) {
        self.toggle_callout(PROPERTIES_FOLD_ID, cx);
    }

    /// The note on screen, as built.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Scrolls so the top-level block `ix` is at the top of the view.
    pub fn scroll_to_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        // A jump into a collapsed section expands it, as a link into one does.
        if self.hidden.get(ix).copied().unwrap_or(false) {
            let heads = self.headings();
            let hiding: Vec<FoldKey> = heads
                .iter()
                .enumerate()
                .filter(|(i, (at, level, _))| {
                    let end = heads[i + 1..]
                        .iter()
                        .find(|(_, l, _)| l <= level)
                        .map(|(next, _, _)| *next)
                        .unwrap_or(self.model.blocks.len());
                    *at < ix && ix < end
                })
                .map(|(_, (_, _, key))| key.clone())
                .collect();
            self.folds.retain(|k| !hiding.contains(k));
            self.recompute_hidden();
        }
        if ix < self.model.blocks.len() {
            self.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }

    pub fn scroll_to_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(ix) = self.model.block_at(offset) {
            self.scroll_to_block(ix, cx);
        }
    }

    /// Byte offset of the top block, which the Outline marks in reading mode.
    pub fn top_offset(&self) -> Option<usize> {
        let top = self.list.logical_scroll_top();
        self.model.starts.get(top.item_ix).copied()
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.list.scroll_to_end();
        cx.notify();
    }

    /// Scrolls by `distance` pixels, as the wheel does. A distance past either end
    /// is left to the list, which walks the scroll top back until the view fills.
    fn scroll_by(&mut self, distance: f32, cx: &mut Context<Self>) {
        self.list.scroll_by(px(distance));
        cx.notify();
    }

    /// One page of the painted viewport, down or up.
    fn page(&mut self, down: bool, cx: &mut Context<Self>) {
        let height = f32::from(self.list.viewport_bounds().size.height);
        let dy = page_distance(height);
        self.scroll_by(if down { dy } else { -dy }, cx);
    }

    /// One rendered line, down or up.
    fn line(&mut self, down: bool, cx: &mut Context<Self>) {
        let dy = line_distance(cx.global::<Theme>().base_font_size);
        self.scroll_by(if down { dy } else { -dy }, cx);
    }

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(true, cx);
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(false, cx);
    }

    fn scroll_line_down(&mut self, _: &ScrollLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.line(true, cx);
    }

    fn scroll_line_up(&mut self, _: &ScrollLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.line(false, cx);
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_to_block(0, cx);
    }

    fn scroll_to_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_to_end(cx);
    }

    /// What the reader is looking at, for the control socket: the top block and
    /// how far into it the view starts, which [`ReadingView::scroll_to_block`]
    /// takes back.
    pub fn describe(&self, _cx: &gpui::App) -> Value {
        let top = self.list.logical_scroll_top();
        json!({
            "path": self.path,
            "scroll": {
                "item": top.item_ix,
                "offset": f32::from(top.offset_in_item),
            },
            // As `editor.selection` reports: the two ends in order and the text, `null` when none.
            "selection": self.selection.and_then(|(anchor, head)| {
                let runs = runs(&self.model);
                let (start, end) = ordered(&runs, anchor, head)?;
                Some(json!({
                    "anchor": describe_pos(start),
                    "head": describe_pos(end),
                    "runs": selected_ranges(&runs, start, end).len(),
                    "text": selected_text(&runs, start, end),
                }))
            }),
            // A note handed over in an editing mode and not built yet.
            "pending": self.is_pending(),
            // An embedded PDF's pages are images with no title, so the dump names it.
            "pdfs": crate::pdf::embed::describe_all(&self.model.blocks),
        })
    }

    /// Shows the check as Obsidian does: `lucide-check` through 900 ms, `lucide-copy` at 1000.
    fn flash_copied(&mut self, id: usize, cx: &mut Context<Self>) {
        self.copied = Some(id);
        self.flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1000))
                .await;
            this.update(cx, |view, cx| {
                view.copied = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn toggle_callout(&mut self, id: usize, cx: &mut Context<Self>) {
        if !self.toggled.remove(&id) {
            self.toggled.insert(id);
        }
        cx.notify();
    }

    /// The footer item: the host's block 48 px under the last (Obsidian's `.mod-footer`).
    fn render_footer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(build) = self.footer.clone() else {
            return div().into_any_element();
        };
        let mut available = f32::from(self.width) - 64.;
        if self.readable_line_length {
            available = available.min(700.);
        }
        let Some(element) = build(available.max(100.), window, cx) else {
            return div().pb(px(120.)).into_any_element();
        };
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .px(px(32.))
            .pt(px(48.))
            .pb(px(120.))
            .child(
                div()
                    .w_full()
                    .when(self.readable_line_length, |d| d.max_w(px(700.)))
                    .child(element),
            )
            .into_any_element()
    }

    fn render_block(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let model = self.model.clone();
        let Some(block) = model.blocks.get(ix) else {
            return self.render_footer(window, cx);
        };
        if self.hidden.get(ix).copied().unwrap_or(false) {
            return div().into_any_element();
        }
        // The block's inner width, capped at the readable line length.
        let mut available = f32::from(self.width) - 64.;
        if self.readable_line_length {
            available = available.min(700.);
        }
        let view = cx.entity();
        let on_link: LinkHandler = Rc::new({
            let view = view.clone();
            move |target, window, cx| {
                let modifiers = window.modifiers();
                let new_tab = modifiers.control || modifiers.platform;
                view.update(cx, |_, cx| {
                    cx.emit(if new_tab {
                        ReadingEvent::OpenLinkInNewTab(target)
                    } else {
                        ReadingEvent::OpenLink(target)
                    })
                });
            }
        });
        let on_toggle: ToggleHandler = Rc::new({
            let view = view.clone();
            move |id, cx| {
                view.update(cx, |v, cx| v.toggle_callout(id, cx));
            }
        });
        let on_copy: CopyHandler = Rc::new({
            let view = view.clone();
            move |id, cx| {
                view.update(cx, |v, cx| v.flash_copied(id, cx));
            }
        });
        let on_task: TaskHandler = Rc::new({
            let view = view.clone();
            move |marker, cx| {
                view.update(cx, |v, cx| {
                    if let Some(path) = v.path.clone() {
                        cx.emit(TaskToggled { path, marker });
                    }
                });
            }
        });
        let on_remove_item: PropertyHandler = Rc::new({
            let view = view.clone();
            move |action, _window, cx| {
                // The block is read-only but for this gesture; anything else is ignored.
                if let PropertyAction::RemoveItem { key, index } = action {
                    view.update(cx, |v, cx| {
                        if let Some(path) = v.path.clone() {
                            cx.emit(PropertyItemRemoved { path, key, index });
                        }
                    });
                }
            }
        });
        let on_menu: MenuHandler = Rc::new({
            let view = view.clone();
            move |target, position, _window, cx| {
                view.update(cx, |_, cx| {
                    cx.emit(ReadingEvent::ContextMenu { target, position })
                });
            }
        });
        let host = ReadingHost {
            properties: self.properties_in_document,
            copied: self.copied,
            on_copy,
            on_task,
            on_link: on_link.clone(),
            link_pressed: self.link_pressed.clone(),
            on_remove_item: Some(on_remove_item),
            on_menu: Some(on_menu),
            hover: Some(HoverHost {
                on_hover: Rc::new({
                    let view = view.clone();
                    move |hover, _window, cx| {
                        view.update(cx, |_, cx| cx.emit(ReadingEvent::Hover(hover)));
                    }
                }),
                current: self.hover_current.clone(),
            }),
        };
        // The runs take the mouse, so a press lands on the text it hit; selected ranges ride in.
        let select = SelectHost {
            ranges: self.selected.clone(),
            color: selection_color(&theme),
            on_select: Rc::new({
                let view = view.clone();
                move |gesture, rich, offset, _window, cx| {
                    view.update(cx, |v, cx| {
                        v.gesture(
                            gesture,
                            ReadingPos {
                                block: ix,
                                rich,
                                offset,
                            },
                            cx,
                        )
                    });
                }
            }),
        };
        let r = Renderer::new(
            &theme,
            window.text_system().clone(),
            available,
            &self.toggled,
            &self.property_types,
            // Read-only: the reading view edits neither properties nor cells.
            None,
            None,
            on_link,
            on_toggle,
        )
        .with_reading_host(&host)
        .with_selection(&select)
        .with_chrome(self.chrome);
        let mut inner = r.block(block, false);
        // Obsidian's `heading-collapse-indicator`: a chevron in the left margin,
        // shown on hover or while collapsed, that only its own click toggles.
        // `foldHeading` off hides it; drawn here so margins sit outside its box.
        if let RBlock::Heading { level, text, .. } = block {
            if self.fold_headings {
                if let Some((_, _, key)) = self.headings().into_iter().find(|(at, _, _)| *at == ix)
                {
                    let collapsed = self.folds.contains(&key);
                    let base = theme.base_font_size;
                    let (size, weight, line_height) = crate::theme::heading_style(*level);
                    let top = match level {
                        1 => 1.0,
                        2 => 2.5,
                        _ => 1.5,
                    };
                    let line_px = base * size * line_height;
                    let view = view.clone();
                    // Reaching 28 px into the margin makes the chevron count as the heading.
                    inner = div()
                        .mt(px(base * top))
                        .mb(px(base))
                        .ml(px(-28.))
                        .pl(px(28.))
                        .relative()
                        .group("heading")
                        .child(
                            div()
                                .text_size(px(base * size))
                                .line_height(px(line_px))
                                .font_weight(FontWeight(weight))
                                .child(r.rich_at(text, base * size)),
                        )
                        .child(
                            div()
                                .id(("heading-fold", ix))
                                .absolute()
                                .left(px(0.))
                                .top(px((line_px - 16.) / 2.))
                                .size(px(16.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .when(!collapsed, |d| d.invisible())
                                .group_hover("heading", |s| s.visible())
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    let key = key.clone();
                                    view.update(cx, |v, cx| v.toggle_heading(key, cx));
                                })
                                .child(icon(
                                    if collapsed {
                                        "chevron-right"
                                    } else {
                                        "chevron-down"
                                    },
                                    16.,
                                    theme.tokens.text_muted,
                                )),
                        )
                        .into_any_element();
                }
            }
        }
        // The view's bottom padding goes under the footer when there is one.
        let last = ix + 1 == model.blocks.len() && self.footer.is_none();
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .px(px(32.))
            .when(last, |d| d.pb(px(120.)))
            .child(
                div()
                    .w_full()
                    .when(self.readable_line_length, |d| d.max_w(px(700.)))
                    .child(inner),
            )
            .into_any_element()
    }
}

impl Render for ReadingView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_built(cx);
        let theme = cx.global::<Theme>().clone();
        let base = theme.base_font_size;
        let view = cx.entity().downgrade();
        // Obsidian's `div.mod-header`: the inline title above the properties,
        // a sibling of the list so block indices and folds keep their meaning.
        let title = self
            .inline_title()
            .map(|t| SharedString::from(t.to_string()));
        let (size, weight, line_height) = crate::theme::heading_style(1);
        let h1 = base * size;
        let header = div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .px(px(32.))
            .pt(px(HEADER_TOP))
            .child(
                div()
                    .w_full()
                    .when(self.readable_line_length, |d| d.max_w(px(700.)))
                    .children(title.map(|title| {
                        div()
                            .text_size(px(h1))
                            .line_height(px(h1 * line_height))
                            .font_weight(FontWeight(weight))
                            .mb(px(h1 * 0.5))
                            .child(title)
                    })),
            );
        div()
            .key_context("ReadingView")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_line_down))
            .on_action(cx.listener(Self::scroll_line_up))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            // A left press drops the selection; a run's handler puts one back,
            // bubbling after this capture listener. A right press leaves it.
            .capture_any_mouse_down(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left {
                    this.clear_selection(cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .size_full()
            .font_family(theme.text_font.clone())
            .text_size(px(base))
            .line_height(px(base * LINE_HEIGHT))
            .text_color(theme.tokens.text_normal)
            .relative()
            .flex()
            .flex_col()
            .child(header)
            .child(
                // The footer follows host state the list cannot see, so it is remeasured each frame.
                {
                    if self.footer.is_some() {
                        let n = self.model.blocks.len();
                        if self.list.item_count() == n + 1 {
                            self.list.remeasure_items(n..n + 1);
                        }
                    }
                    list(self.list.clone(), move |ix, window, cx| {
                        view.update(cx, |this, cx| this.render_block(ix, window, cx))
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                },
            )
            .child(
                // Records the painted width; a change lays the blocks out again.
                canvas(
                    {
                        let view = cx.entity();
                        move |bounds, _, cx| {
                            view.update(cx, |this, cx| {
                                if this.width != bounds.size.width {
                                    this.width = bounds.size.width;
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

/// The block metrics text is set in, as multiples of the base font size; anything
/// not named here (lists, quotes, tables, callouts) scales with that base.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BlockStyle {
    /// `(size, weight, line height)` of h1 to h6.
    pub headings: [(f32, f32, f32); 6],
    /// The margin above h1 to h6.
    pub heading_top: [f32; 6],
    /// The margin below every heading.
    pub heading_bottom: f32,
    /// The margin below a paragraph or a code block.
    pub gap: f32,
    /// A code block's text size.
    pub code_size: f32,
}

impl BlockStyle {
    /// Obsidian's reading view: h1 1.618em at 700 down to h6 1em at 600 on 1.2 lines.
    pub(crate) const READING: BlockStyle = BlockStyle {
        headings: crate::theme::HEADINGS,
        heading_top: [1.0, 2.5, 1.5, 1.5, 1.5, 1.5],
        heading_bottom: 1.0,
        gap: 0.9,
        code_size: 0.875,
    };

    fn heading(&self, level: u8) -> (f32, f32, f32) {
        self.headings[(level.clamp(1, 6) - 1) as usize]
    }
}

/// Builds GPUI elements for model blocks, shared by the reading view and Live Preview.
pub struct Renderer<'a> {
    /// The metrics blocks are set in.
    style: BlockStyle,
    theme: &'a Theme,
    toggled: &'a HashSet<usize>,
    /// The vault's `types.json`, so a row draws its key's assigned type.
    property_types: &'a PropertyTypes,
    /// Set by Live Preview to make the properties block editable.
    properties_host: Option<&'a PropertiesHost>,
    /// Set by Live Preview to make a table's cells editable.
    table_host: Option<&'a TableHost>,
    /// Set by the reading view for the affordances only it draws.
    reading_host: Option<&'a ReadingHost>,
    /// Set by the reading view so its text can be selected and painted.
    select: Option<&'a SelectHost>,
    on_link: LinkHandler,
    on_toggle: ToggleHandler,
    text_system: Arc<WindowTextSystem>,
    available: f32,
    /// `showIndentGuide` (a nested list's guide) and `rightToLeft`.
    chrome: crate::gutter::Chrome,
}

impl<'a> Renderer<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme: &'a Theme,
        text_system: Arc<WindowTextSystem>,
        available: f32,
        toggled: &'a HashSet<usize>,
        property_types: &'a PropertyTypes,
        properties_host: Option<&'a PropertiesHost>,
        table_host: Option<&'a TableHost>,
        on_link: LinkHandler,
        on_toggle: ToggleHandler,
    ) -> Renderer<'a> {
        Renderer {
            style: BlockStyle::READING,
            theme,
            toggled,
            property_types,
            properties_host,
            table_host,
            reading_host: None,
            select: None,
            on_link,
            on_toggle,
            text_system,
            available: available.max(100.),
            chrome: crate::gutter::Chrome::default(),
        }
    }

    /// The Editor tab's `showIndentGuide` and `rightToLeft`.
    pub(crate) fn with_chrome(mut self, chrome: crate::gutter::Chrome) -> Renderer<'a> {
        self.chrome = chrome;
        self
    }

    /// Makes the text selectable: each run takes the mouse and paints what is selected.
    pub(crate) fn with_selection(mut self, host: &'a SelectHost) -> Renderer<'a> {
        self.select = Some(host);
        self
    }

    /// The reading view's own settings and handlers.
    pub fn with_reading_host(mut self, host: &'a ReadingHost) -> Renderer<'a> {
        self.reading_host = Some(host);
        self
    }

    fn measure(&self, text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
        let t = self.theme;
        let font = Font {
            family: if mono {
                t.mono_font.clone()
            } else {
                t.text_font.clone()
            },
            features: Default::default(),
            fallbacks: None,
            weight: if bold {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            },
            style: FontStyle::Normal,
        };
        let run = TextRun {
            len: text.len(),
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = self.text_system.shape_line(
            SharedString::from(text.to_string()),
            px(font_size),
            &[run],
            None,
        );
        f32::from(line.width)
    }

    /// Column widths as an HTML table's automatic layout sizes them: each wants
    /// its longest cell (max-content) and needs its longest word (min-content),
    /// slack shared by range and overflowing below the minimum. `editing` grows.
    fn column_widths(
        &self,
        header: &[RichText],
        rows: &[Vec<RichText>],
        font_size: f32,
        editing: Option<(usize, &str)>,
    ) -> Vec<f32> {
        let count = header
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));
        let mut max = vec![0f32; count];
        let mut min = vec![0f32; count];
        let mut fold = |ix: usize, cell: &RichText, bold: bool| {
            let (mx, mn) = self.cell_extent(cell, font_size, bold);
            max[ix] = max[ix].max(mx);
            min[ix] = min[ix].max(mn);
        };
        for (ix, cell) in header.iter().enumerate() {
            fold(ix, cell, true);
        }
        for row in rows {
            for (ix, cell) in row.iter().enumerate() {
                fold(ix, cell, false);
            }
        }
        if let Some((ix, text)) = editing.filter(|(ix, _)| *ix < count) {
            let width = self.measure(text, font_size, false, false);
            max[ix] = max[ix].max(width);
            min[ix] = min[ix].max(width);
        }
        self.fit_columns(&max, &min)
    }

    /// A cell's longest line (max-content) and longest word (min-content).
    fn cell_extent(&self, cell: &RichText, font_size: f32, bold: bool) -> (f32, f32) {
        let mono = cell.runs.iter().any(|(_, s)| s.code || s.kbd);
        let longest_line = cell
            .text
            .lines()
            .max_by_key(|l| l.chars().count())
            .unwrap_or("");
        let longest_word = cell
            .text
            .split_whitespace()
            .max_by_key(|w| w.chars().count())
            .unwrap_or("");
        (
            self.measure(longest_line, font_size, bold, mono),
            self.measure(longest_word, font_size, bold, mono),
        )
    }

    /// Column widths from content widths: padding added, slack shared when the note is narrow.
    fn fit_columns(&self, max: &[f32], min: &[f32]) -> Vec<f32> {
        // Two pixels of slack: the wrapper measures advances slightly differently.
        let max: Vec<f32> = max
            .iter()
            .map(|m| m.ceil() + 2. + TABLE_CELL_PADDING)
            .collect();
        let min: Vec<f32> = min
            .iter()
            .map(|m| m.max(8.).ceil() + 2. + TABLE_CELL_PADDING)
            .collect();
        let sum_max: f32 = max.iter().sum();
        let sum_min: f32 = min.iter().sum();
        if sum_max <= self.available {
            max
        } else if sum_min >= self.available {
            min
        } else {
            let share = (self.available - sum_min) / (sum_max - sum_min);
            max.iter()
                .zip(&min)
                .map(|(mx, mn)| (mn + (mx - mn) * share).floor())
                .collect()
        }
    }

    /// A raw table's `<caption>`: centred across the table (or its longest word),
    /// above it or below under `caption-side: bottom` (Obsidian 1.15.0).
    fn captioned(
        &self,
        table: AnyElement,
        caption: &RichText,
        bottom: bool,
        table_width: f32,
        gap: Pixels,
    ) -> AnyElement {
        let (_, word) = self.cell_extent(caption, self.theme.base_font_size, false);
        let width = table_width.max(word.ceil() + 2.);
        let caption = div()
            .w(px(width))
            .flex_shrink_0()
            .text_center()
            .child(self.rich(caption))
            .into_any_element();
        let (first, second) = if bottom {
            (table, caption)
        } else {
            (caption, table)
        };
        div()
            .mb(gap)
            .flex()
            .flex_col()
            .items_start()
            .child(first)
            .child(second)
            .into_any_element()
    }

    /// A raw table with spanning cells: a grid placed by the table model
    /// ([`super::model::CellPlace`]), columns sized as `column_widths` sizes
    /// them with spanning needs shared, rows as tall as their content. Never
    /// edited in place: a raw table is source in Live Preview.
    fn spanned_table(
        &self,
        header: &[RichText],
        rows: &[Vec<RichText>],
        extras: &HtmlTableExtras,
        font_size: f32,
        gap: Pixels,
    ) -> AnyElement {
        let t = self.theme;
        let columns = extras.columns.max(1);
        let has_header = !header.is_empty();
        let mut lines: Vec<&[RichText]> = Vec::with_capacity(rows.len() + 1);
        if has_header {
            lines.push(header);
        }
        lines.extend(rows.iter().map(Vec::as_slice));
        let row_count = lines.len();
        let bold_at = |r: usize| has_header && r == 0;
        // One-column cells first, then each spanning cell's shortfall shared evenly.
        let mut max = vec![0f32; columns];
        let mut min = vec![0f32; columns];
        let mut spanning = Vec::new();
        for (r, line) in lines.iter().enumerate() {
            let places = extras.places.get(r).map(Vec::as_slice).unwrap_or(&[]);
            for (cell, place) in line.iter().zip(places) {
                let (mx, mn) = self.cell_extent(cell, font_size, bold_at(r));
                if place.colspan == 1 {
                    max[place.col] = max[place.col].max(mx);
                    min[place.col] = min[place.col].max(mn);
                } else {
                    spanning.push((*place, mx, mn));
                }
            }
        }
        let between = 2. + TABLE_CELL_PADDING;
        for (place, mx, mn) in spanning {
            let cols = place.col..place.col + place.colspan;
            let n = place.colspan as f32;
            for (need, have) in [(mx, &mut max), (mn, &mut min)] {
                let room: f32 = have[cols.clone()].iter().sum::<f32>() + (n - 1.) * between;
                if need > room {
                    for w in &mut have[cols.clone()] {
                        *w += (need - room) / n;
                    }
                }
            }
        }
        let widths = self.fit_columns(&max, &min);
        let table_width: f32 = widths.iter().sum();
        let mut cells: Vec<AnyElement> = Vec::new();
        for (r, line) in lines.iter().enumerate() {
            let places = extras.places.get(r).map(Vec::as_slice).unwrap_or(&[]);
            for (cell, place) in line.iter().zip(places) {
                let width: f32 = widths[place.col..place.col + place.colspan].iter().sum();
                let last_col = place.col + place.colspan >= columns;
                let last_row = r + place.rowspan >= row_count;
                let head = bold_at(r);
                cells.push(
                    div()
                        .col_start(place.col as i16 + 1)
                        .col_end((place.col + place.colspan) as i16 + 1)
                        .row_start(r as i16 + 1)
                        .row_end((r + place.rowspan) as i16 + 1)
                        .w(px(width))
                        .px_2()
                        .py_1()
                        .when(!last_col, |d| d.border_r_1())
                        .when(!last_row, |d| d.border_b_1())
                        .border_color(if head && place.rowspan == 1 {
                            t.tokens.table_header_border_color
                        } else {
                            t.tokens.table_border_color
                        })
                        .when(head, |d| {
                            d.bg(t.tokens.table_header_background)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.tokens.table_header_color)
                        })
                        .map(|d| match place.align {
                            Alignment::Center => d.text_center(),
                            Alignment::Right => d.text_right(),
                            _ => d,
                        })
                        .child(self.rich(cell))
                        .into_any_element(),
                );
            }
        }
        // A zero-height row pinning every column's width, so the grid never guesses one.
        for (ix, w) in widths.iter().enumerate() {
            cells.push(
                div()
                    .col_start(ix as i16 + 1)
                    .col_end(ix as i16 + 2)
                    .row_start(row_count as i16 + 1)
                    .row_end(row_count as i16 + 2)
                    .w(px(*w))
                    .h(px(0.))
                    .into_any_element(),
            );
        }
        let table = div()
            .w(px(table_width + 2.))
            .flex_shrink_0()
            .grid()
            .grid_cols_max_content(columns as u16)
            .border_1()
            .border_color(t.tokens.table_border_color)
            .rounded_sm()
            .text_size(px(font_size))
            .children(cells)
            .into_any_element();
        match &extras.caption {
            Some(caption) => {
                self.captioned(table, caption, extras.caption_bottom, table_width + 2., gap)
            }
            None => div().mb(gap).flex().child(table).into_any_element(),
        }
    }

    fn blocks(&self, blocks: &[RBlock], tight: bool) -> Vec<AnyElement> {
        blocks.iter().map(|b| self.block(b, tight)).collect()
    }

    pub fn block(&self, block: &RBlock, tight: bool) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let style = self.style;
        let gap = if tight { px(0.) } else { px(base * style.gap) };
        match block {
            RBlock::Properties(entries) => self.properties(entries),
            RBlock::Paragraph(segments) => div()
                .mb(gap)
                .flex()
                .flex_col()
                .children(segments.iter().map(|s| self.segment(s)))
                .into_any_element(),
            RBlock::Heading { level, text, .. } => {
                let (size, weight, line_height) = style.heading(*level);
                let top = style.heading_top[(level.clamp(&1, &6) - 1) as usize];
                div()
                    .mt(px(base * top))
                    .mb(px(base * style.heading_bottom))
                    .text_size(px(base * size))
                    .line_height(px(base * size * line_height))
                    .font_weight(FontWeight(weight))
                    .child(self.rich_at(text, base * size))
                    .into_any_element()
            }
            // `hr`: `--hr-color`.
            RBlock::ThematicBreak => div()
                .my(px(base * 1.5))
                .h(px(1.))
                .w_full()
                .bg(t.tokens.hr_color)
                .into_any_element(),
            RBlock::Code {
                id,
                language,
                text,
                copy,
                wrap,
                copyable,
            } => {
                // A ```mermaid fence is the diagram: no band, no copy button, the block's width.
                if tft_mermaid::is_mermaid(language.as_deref()) {
                    return div()
                        .mb(gap)
                        .w_full()
                        .pb(px(crate::mermaid::BOTTOM_GAP))
                        .child(crate::mermaid::block(text, t))
                        .into_any_element();
                }
                let id = *id;
                let (wrap, copyable) = (*wrap, *copyable);
                let payload = copy.clone();
                let on_copy = self.reading_host.map(|h| h.on_copy.clone());
                let copied = self.reading_host.is_some_and(|h| h.copied == Some(id));
                div()
                    .mb(gap)
                    .relative()
                    .group("code-block")
                    .bg(t.tokens.code_background)
                    .rounded_md()
                    .py_2()
                    .font_family(t.mono_font.clone())
                    .text_size(px(base * style.code_size))
                    .line_height(px(base * style.code_size * 1.5))
                    .child(
                        // Long lines scroll sideways, except a raw `<pre>`, which wraps.
                        div()
                            .id(("code", id))
                            .when(!wrap, |d| d.overflow_x_scroll())
                            .px_3()
                            .child(
                                div()
                                    .when(!wrap, |d| d.whitespace_nowrap())
                                    .child(code_text(text, language.as_deref(), t)),
                            ),
                    )
                    .when(copyable, |d| {
                        d.child(
                            // No language name: only a copy button, while hovered.
                            div()
                                .id(("copy-code", id))
                                .absolute()
                                .top_0()
                                .right_0()
                                .m(px(6.))
                                .px_2()
                                .py(px(6.))
                                .rounded_sm()
                                .invisible()
                                .group_hover("code-block", |s| s.visible())
                                .cursor_pointer()
                                .hover(|s| s.bg(t.tokens.background_secondary))
                                // `lucide-copy`, or `lucide-check` while it shows: a 16 px icon.
                                .child(icon(
                                    if copied { "check" } else { "copy" },
                                    16.,
                                    t.tokens.text_muted,
                                ))
                                .on_click(move |_, _, cx| {
                                    // An empty fence writes nothing: in Obsidian the
                                    // clipboard kept what it had, and no check flashes.
                                    if payload.is_empty() {
                                        return;
                                    }
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        payload.clone(),
                                    ));
                                    if let Some(on_copy) = &on_copy {
                                        on_copy(id, cx);
                                    }
                                }),
                        )
                    })
                    .into_any_element()
            }
            // A raw `<details>`: the summary folds the body; `open` starts it open, a click flips it.
            RBlock::Details {
                id,
                open,
                summary,
                body,
            } => {
                let id = *id;
                let flipped = self.toggled.contains(&id);
                let collapsed = *open == flipped;
                let on_toggle = self.on_toggle.clone();
                div()
                    .mb(gap)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("details", id))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .block_mouse_except_scroll()
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                on_toggle(id, cx)
                            })
                            .child(icon(
                                if collapsed {
                                    "chevron-right"
                                } else {
                                    "chevron-down"
                                },
                                14.,
                                t.tokens.text_muted,
                            ))
                            .child(div().flex_1().child(self.rich(summary))),
                    )
                    .when(!collapsed, |d| {
                        d.child(
                            div()
                                .pl(px(22.))
                                .flex()
                                .flex_col()
                                .children(self.blocks(body, false)),
                        )
                    })
                    .into_any_element()
            }
            RBlock::Html(text) => div()
                .mb(gap)
                .font_family(t.mono_font.clone())
                .text_size(px(base * 0.875))
                .text_color(t.tokens.text_muted)
                .child(SharedString::from(text.clone()))
                .into_any_element(),
            // A right-to-left quote keeps its bar on the right (`dir="auto"`).
            RBlock::Quote(children) if blocks_rtl(children) => div()
                .mb(gap)
                .border_r_2()
                .border_color(t.tokens.interactive_accent)
                .pr_4()
                .flex()
                .flex_col()
                .children(self.blocks(children, false))
                .into_any_element(),
            RBlock::Quote(children) => div()
                .mb(gap)
                .border_l_2()
                .border_color(t.tokens.interactive_accent)
                .pl_4()
                .flex()
                .flex_col()
                .children(self.blocks(children, false))
                .into_any_element(),
            RBlock::Callout {
                kind,
                fold,
                title,
                children,
            } => {
                let color = t.callout_color(kind);
                let foldable = fold.is_some();
                let flipped = self.toggled.contains(&title.id);
                let collapsed = fold.unwrap_or(false) != flipped;
                let id = title.id;
                let on_toggle = self.on_toggle.clone();
                let mut bg = color;
                bg.a = if t.dark { 0.12 } else { 0.08 };
                div()
                    .mb(gap)
                    .bg(bg)
                    .border_l_4()
                    .border_color(color)
                    .rounded_md()
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("callout", id))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            // Mouse-down, blocked: in Live Preview the editor's window
                            // listener runs ahead and would reveal the title line's
                            // source before the click's button-up. Scroll falls through.
                            .when(foldable, |d| {
                                d.cursor_pointer()
                                    .block_mouse_except_scroll()
                                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                        on_toggle(id, cx)
                                    })
                            })
                            // The type's Lucide icon in the callout's colour (see [`callout_icon`]).
                            .child(icon(callout_icon(kind), 16., color))
                            .child(
                                div().flex_1().child(
                                    // A link in the title wins: Obsidian navigates and
                                    // leaves the fold alone. The rest of the row still folds.
                                    match foldable && !title.links.is_empty() {
                                        true => div()
                                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .child(self.rich(title))
                                            .into_any_element(),
                                        false => self.rich(title).into_any_element(),
                                    },
                                ),
                            )
                            .when(foldable, |d| {
                                // `.callout-fold` holds `lucide-chevron-down`
                                // either way, turned a quarter when collapsed;
                                // `chevron-right` is the same drawing turned.
                                d.child(icon(
                                    if collapsed {
                                        "chevron-right"
                                    } else {
                                        "chevron-down"
                                    },
                                    16.,
                                    t.tokens.text_muted,
                                ))
                            }),
                    )
                    .when(!collapsed && !children.is_empty(), |d| {
                        d.child(
                            div()
                                .mt_2()
                                .flex()
                                .flex_col()
                                .children(self.blocks(children, false)),
                        )
                    })
                    .into_any_element()
            }
            RBlock::List { tight, items } => div()
                .mb(gap)
                .flex()
                .flex_col()
                .children(items.iter().map(|item| self.item(item, *tight)))
                .into_any_element(),
            RBlock::Table {
                alignments,
                header,
                rows,
                spans,
                html,
            } => {
                let font_size = base * 0.9375;
                if let Some(extras) = html.as_deref().filter(|e| !e.places.is_empty()) {
                    return self.spanned_table(header, rows, extras, font_size, gap);
                }
                let caption = html
                    .as_deref()
                    .and_then(|e| e.caption.as_ref().map(|c| (c, e.caption_bottom)));
                let host = self.table_host;
                // The open cell's column measures the text in hand, so it grows as it is typed.
                let editing = host.and_then(|h| h.cell.as_ref()).and_then(|(at, f)| {
                    spans
                        .iter()
                        .find_map(|row| row.iter().position(|s| s == at))
                        .map(|ix| (ix, f.text.as_str()))
                });
                let columns = self.column_widths(header, rows, font_size, editing);
                let table_width: f32 = columns.iter().sum();
                // A table is named by its first cell's start, which the handle actions carry.
                let table_id = spans.first().and_then(|r| r.first()).map(|s| s.start);
                let handles = host.is_some() && table_id.is_some() && !header.is_empty();
                let last_row = rows.len();
                let last_col = columns.len().saturating_sub(1);
                let accent = t.tokens.interactive_accent;
                let selected = host
                    .and_then(|h| h.select)
                    .filter(|s| Some(s.table) == table_id);
                let dragging = host
                    .and_then(|h| h.drag)
                    .filter(|d| Some(d.table) == table_id);
                // `row` is 0 for the header; each cell's span is the model's.
                let cell = |text: &RichText, row: usize, ix: usize, head: bool| -> AnyElement {
                    let align = alignments.get(ix).copied().unwrap_or(Alignment::None);
                    let width = columns.get(ix).copied().unwrap_or(80.);
                    let span = spans.get(row).and_then(|r| r.get(ix)).cloned();
                    // The open cell shows the text in hand with a caret, as a property field does.
                    let editing = host
                        .and_then(|h| h.cell.as_ref())
                        .filter(|(at, _)| Some(at) == span.as_ref())
                        .map(|(_, f)| f);
                    // `th`/`td`: `--table-border-color` lines, the header in
                    // `--table-header-color` at 600 on a transparent ground.
                    let mut drawn = div()
                        .id((
                            "table-cell",
                            span.as_ref().map(|s| s.start).unwrap_or(row * 4096 + ix),
                        ))
                        .relative()
                        .w(px(width))
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        // A line between columns only: the table's border is the last column's edge.
                        .when(ix != last_col, |d| d.border_r_1())
                        .border_color(t.tokens.table_border_color)
                        .when(head, |d| {
                            d.font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.tokens.table_header_color)
                        })
                        .map(|d| match align {
                            Alignment::Center => d.text_center(),
                            Alignment::Right => d.text_right(),
                            _ => d,
                        })
                        .map(|d| match editing {
                            Some(field) => d.child(self.property_field(field, false)),
                            None => d.child(self.rich(text)),
                        });
                    if handles {
                        let id = table_id.unwrap_or(0);
                        let host = host.expect("handles are only drawn with a host");
                        // The selected row or column: an accent outline and tint per cell.
                        if let Some(sel) = selected {
                            let hit = match sel.axis {
                                TableAxis::Row => sel.index == row,
                                TableAxis::Col => sel.index == ix,
                            };
                            if hit {
                                let mut tint = accent;
                                tint.a = 0.15;
                                let mut over = div()
                                    .absolute()
                                    .top(px(-1.))
                                    .left(px(-1.))
                                    .right(px(-1.))
                                    .bottom(px(-1.))
                                    .bg(tint)
                                    .border_color(accent);
                                over = match sel.axis {
                                    TableAxis::Row => over
                                        .border_t_2()
                                        .border_b_2()
                                        .when(ix == 0, |d| d.border_l_2())
                                        .when(ix == last_col, |d| d.border_r_2()),
                                    TableAxis::Col => over
                                        .border_l_2()
                                        .border_r_2()
                                        .when(row == 0, |d| d.border_t_2())
                                        .when(row == last_row, |d| d.border_b_2()),
                                };
                                drawn = drawn.child(over);
                            }
                        }
                        // The gap a held line would land in: a thick accent bar on the pointer's side.
                        if let Some(drag) = dragging {
                            if let Some((at, after)) = drag.target {
                                let on_it = match drag.axis {
                                    TableAxis::Row => at == row,
                                    TableAxis::Col => at == ix,
                                };
                                if on_it {
                                    let bar = div().absolute().bg(accent);
                                    drawn = drawn.child(match (drag.axis, after) {
                                        (TableAxis::Row, false) => {
                                            bar.top(px(-2.)).left(px(0.)).right(px(0.)).h(px(4.))
                                        }
                                        (TableAxis::Row, true) => {
                                            bar.bottom(px(-2.)).left(px(0.)).right(px(0.)).h(px(4.))
                                        }
                                        (TableAxis::Col, false) => {
                                            bar.left(px(-2.)).top(px(0.)).bottom(px(0.)).w(px(4.))
                                        }
                                        (TableAxis::Col, true) => {
                                            bar.right(px(-2.)).top(px(0.)).bottom(px(0.)).w(px(4.))
                                        }
                                    });
                                }
                            }
                        }
                        // One cell per line answers for a held handle (a column's
                        // header cell, a row's first cell). Past the table's end
                        // the nearest edge takes it, so the bar follows it out.
                        let over = host.on_action.clone();
                        drawn = drawn.on_drag_move(
                            move |event: &DragMoveEvent<TableLineDrag>, window, cx| {
                                let drag = event.drag(cx).clone();
                                if drag.table != id {
                                    return;
                                }
                                let at = event.event.position;
                                let b = event.bounds;
                                let target = match drag.axis {
                                    TableAxis::Row if ix == 0 => {
                                        if row == 0 && at.y < b.top() {
                                            Some((0, false))
                                        } else if row == last_row && at.y >= b.bottom() {
                                            Some((last_row, true))
                                        } else if at.y >= b.top() && at.y < b.bottom() {
                                            Some((row, at.y > b.center().y))
                                        } else {
                                            None
                                        }
                                    }
                                    TableAxis::Col if row == 0 => {
                                        if ix == 0 && at.x < b.left() {
                                            Some((0, false))
                                        } else if ix == last_col && at.x >= b.right() {
                                            Some((last_col, true))
                                        } else if at.x >= b.left() && at.x < b.right() {
                                            Some((ix, at.x > b.center().x))
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                };
                                if let Some((index, after)) = target {
                                    over(
                                        TableAction::DragOver {
                                            table: id,
                                            axis: drag.axis,
                                            from: drag.from,
                                            index,
                                            after,
                                        },
                                        window,
                                        cx,
                                    );
                                }
                            },
                        );
                        // A grip above each column and left of each row: invisible
                        // until hovered, pressed to select the line, dragged to move
                        // it. The header's row grip joins the first column's.
                        let make = |axis: TableAxis, index: usize| -> AnyElement {
                            let held = dragging.is_some_and(|d| d.axis == axis && d.from == index);
                            let chosen =
                                selected.is_some_and(|s| s.axis == axis && s.index == index);
                            let act = host.on_action.clone();
                            let drag = TableLineDrag {
                                table: id,
                                axis,
                                from: index,
                            };
                            let base = div()
                                .id(match axis {
                                    TableAxis::Col => ("table-col-handle", id * 64 + index),
                                    TableAxis::Row => ("table-row-handle", id * 64 + index),
                                })
                                .absolute()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor(if held {
                                    CursorStyle::ClosedHand
                                } else {
                                    CursorStyle::OpenHand
                                })
                                .opacity(if held || chosen { 1. } else { 0. })
                                .hover(|s| s.opacity(1.))
                                .when(held, |d| d.bg(accent))
                                .child(icon(
                                    match axis {
                                        TableAxis::Col => "grip-horizontal",
                                        TableAxis::Row => "grip-vertical",
                                    },
                                    14.,
                                    if held {
                                        gpui::white()
                                    } else {
                                        t.tokens.text_faint
                                    },
                                ));
                            let base = match axis {
                                TableAxis::Col => base
                                    .top(px(-14.))
                                    .left(px(0.))
                                    .w_full()
                                    .h(px(14.))
                                    .rounded_tl(px(4.))
                                    .rounded_tr(px(4.)),
                                TableAxis::Row => base
                                    .left(px(-14.))
                                    .top(px(0.))
                                    .h_full()
                                    .w(px(14.))
                                    .rounded_tl(px(4.))
                                    .rounded_bl(px(4.)),
                            };
                            base.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                act(
                                    TableAction::Select {
                                        table: id,
                                        axis,
                                        index,
                                    },
                                    window,
                                    cx,
                                );
                            })
                            .on_drag(drag, |_, _, _, cx: &mut App| -> Entity<TableGhost> {
                                cx.new(|_| TableGhost)
                            })
                            .into_any_element()
                        };
                        if head {
                            drawn = drawn.child(make(TableAxis::Col, ix));
                        }
                        if ix == 0 {
                            drawn = drawn.child(make(TableAxis::Row, row));
                        }
                    }
                    match (host, span) {
                        (Some(host), Some(span)) => {
                            let on_action = host.on_action.clone();
                            drawn
                                .cursor_text()
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    on_action(TableAction::Edit { span: span.clone() }, window, cx);
                                })
                                .into_any_element()
                        }
                        _ => drawn.into_any_element(),
                    }
                };
                // The "+" bars: "Add row below" as wide as the table, "Add column
                // to the right" as tall, each 16 px with a `plus` until hovered; a
                // click appends an empty padded line. They sit in the table's margin.
                let plus = |axis: TableAxis| -> Option<AnyElement> {
                    let host = host.filter(|_| handles)?;
                    let id = table_id?;
                    let act = host.on_action.clone();
                    let base = div()
                        .id(match axis {
                            TableAxis::Row => ("table-row-btn", id),
                            TableAxis::Col => ("table-col-btn", id),
                        })
                        .absolute()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.)
                        .hover(|s| s.opacity(1.))
                        .child(icon("plus", 16., t.tokens.text_faint));
                    let base = match axis {
                        TableAxis::Row => base
                            .left(px(0.))
                            .bottom(px(-16.))
                            .w(px(table_width))
                            .h(px(16.))
                            .cursor(CursorStyle::ResizeDown),
                        TableAxis::Col => base
                            .top(px(0.))
                            .left(px(table_width))
                            .h_full()
                            .w(px(16.))
                            .cursor(CursorStyle::ResizeRight),
                    };
                    Some(
                        base.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            act(TableAction::Append { table: id, axis }, window, cx);
                        })
                        .into_any_element(),
                    )
                };
                let table = div()
                    .when(caption.is_none(), |d| d.mb(gap))
                    .relative()
                    .flex()
                    .flex_row()
                    .children(plus(TableAxis::Row))
                    .children(plus(TableAxis::Col))
                    .child({
                        let grid = div()
                            .w(px(table_width))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .border_1()
                            .border_color(t.tokens.table_border_color)
                            .rounded_sm()
                            .text_size(px(font_size))
                            // Neither `<thead>` nor `<th>` means no header row.
                            .when(!header.is_empty(), |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .bg(t.tokens.table_header_background)
                                        .border_b_1()
                                        .border_color(t.tokens.table_header_border_color)
                                        .children(
                                            header
                                                .iter()
                                                .enumerate()
                                                .map(|(i, c)| cell(c, 0, i, true)),
                                        ),
                                )
                            })
                            .children(rows.iter().enumerate().map(|(ri, row)| {
                                div()
                                    .flex()
                                    .flex_row()
                                    .when(ri + 1 < rows.len(), |d| {
                                        d.border_b_1().border_color(t.tokens.table_border_color)
                                    })
                                    .children(
                                        row.iter()
                                            .enumerate()
                                            .map(|(i, c)| cell(c, ri + 1, i, false)),
                                    )
                            }));
                        // The editor sees the release, wherever it is.
                        grid.into_any_element()
                    })
                    .into_any_element();
                match caption {
                    Some((caption, bottom)) => {
                        self.captioned(table, caption, bottom, table_width, gap)
                    }
                    None => table,
                }
            }
            RBlock::Math(text) => self.math_block(text, gap),
            RBlock::Footnotes(notes) => div()
                .mt(px(base * 2.))
                .pt(px(base))
                .border_t_1()
                .border_color(t.tokens.background_modifier_border)
                .text_size(px(base * 0.875))
                .flex()
                .flex_col()
                .children(notes.iter().map(|(n, blocks)| {
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(
                            div()
                                .w(px(28.))
                                .flex_shrink_0()
                                .text_color(t.tokens.text_muted)
                                .child(SharedString::from(format!("{n}."))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .children(self.blocks(blocks, true)),
                        )
                }))
                .into_any_element(),
        }
    }

    fn item(&self, item: &RItem, tight: bool) -> AnyElement {
        let t = self.theme;
        let k = &t.tokens;
        let base = t.base_font_size;
        let marker: AnyElement = match (item.task, item.number) {
            (Some(c), _) => {
                let checked = c != ' ';
                // Obsidian's checkbox: `--checkbox-border-color` until checked, then
                // `--checkbox-color` with a `--checkbox-marker-color` mark.
                let check = div()
                    .w(px(16.))
                    .h(px(16.))
                    .mt(px(base * 0.3))
                    .rounded_sm()
                    .border_1()
                    .border_color(if checked {
                        k.checkbox_color
                    } else {
                        k.checkbox_border_color
                    })
                    .when(checked, |d| d.bg(k.checkbox_color))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(k.checkbox_marker_color)
                    .when(checked, |d| d.child("✓"));
                // Clicking rewrites the line as in Live Preview. An embed's item has no
                // marker and stays inert: its bytes are another file's.
                match (self.reading_host, item.marker) {
                    (Some(host), Some(marker)) => {
                        let on_task = host.on_task.clone();
                        check
                            .id(("task", marker.line))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| on_task(marker, cx))
                            .into_any_element()
                    }
                    _ => check.into_any_element(),
                }
            }
            // `li::marker`: `--list-marker-color`, the faint text colour.
            (None, Some(n)) => div()
                .text_color(k.list_marker_color)
                .child(SharedString::from(format!("{n}.")))
                .into_any_element(),
            (None, None) => div()
                .text_color(k.list_marker_color)
                .child("•")
                .into_any_element(),
        };
        let done = item.task.is_some_and(|c| c != ' ');
        // A right-to-left item keeps its marker on the right (`li` is `dir="auto"`).
        let rtl = blocks_rtl(&item.children);
        // `showIndentGuide`: a 1 px `--indentation-guide-color` line from the item's
        // first line to its end (Obsidian 1.15.0 draws it on the nested `ul`).
        let guide = (self.chrome.indent_guides
            && item
                .children
                .iter()
                .any(|b| matches!(b, RBlock::List { .. })))
        .then(|| {
            let line = div()
                .absolute()
                .top(px(base * 1.5))
                .bottom_0()
                .w(px(1.))
                .bg(k.indentation_guide_color);
            // The marker's middle: 3 px in from the column's edge.
            if rtl {
                line.right(px(-11.))
            } else {
                line.left(px(-11.))
            }
        });
        div()
            .flex()
            .when(rtl, |d| d.flex_row_reverse())
            .when(!rtl, |d| d.flex_row())
            .gap_2()
            .when(!tight, |d| d.mb(px(base * 0.4)))
            .child(
                div()
                    .min_w(px(22.))
                    .flex_shrink_0()
                    .flex()
                    .when(rtl, |d| d.justify_start())
                    .when(!rtl, |d| d.justify_end())
                    .child(marker),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    // A done task is dimmed and struck through.
                    .when(done, |d| {
                        d.text_color(k.checklist_done_color).line_through()
                    })
                    .relative()
                    .children(guide)
                    .children(self.blocks(&item.children, tight)),
            )
            .into_any_element()
    }

    fn segment(&self, segment: &Segment) -> AnyElement {
        match segment {
            Segment::Text(text) => self.rich(text),
            Segment::Embed(embed) => self.embed(embed),
            // Display math in a paragraph is its own block; the gap follows the paragraph.
            Segment::Math(tex) => self.math_block(tex, px(0.)),
        }
    }

    /// `$$...$$`: display-style, centred, at body size, scrolling sideways as
    /// MathJax does. An unparsable formula shows its source in the error red.
    fn math_block(&self, tex: &str, gap: Pixels) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let tex = tex.trim();
        match math::typeset(tex, true) {
            Ok(formula) => div()
                .id(ElementId::Name(format!("math-block:{tex}").into()))
                .mb(gap)
                .py(px(base * 0.5))
                .w_full()
                .overflow_x_scroll()
                .flex()
                .flex_row()
                .child(div().mx_auto().flex_shrink_0().child(FormulaElement::new(
                    formula,
                    px(base),
                    t.tokens.text_normal,
                )))
                .into_any_element(),
            Err(_) => div()
                .mb(gap)
                .py(px(base * 0.5))
                .text_color(math::error_color())
                .text_center()
                .child(SharedString::from(tex.to_string()))
                .into_any_element(),
        }
    }

    fn embed(&self, embed: &Embed) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        match &embed.kind {
            EmbedKind::Image(path) => {
                let mut image = img(path.clone())
                    .max_w_full()
                    .object_fit(ObjectFit::Contain);
                if let Some(w) = embed.width {
                    image = image.w(px(w));
                }
                if let Some(h) = embed.height {
                    image = image.h(px(h));
                }
                div().my_1().child(image).into_any_element()
            }
            // `![alt|60x30](https://...)`: fetched through `remote.rs` and drawn at
            // its own size or the `|60x30` hint, as the `<img>` attributes do. A
            // failed load is the alt text at one line, like Chromium's broken image.
            EmbedKind::Remote(url) => {
                let alt = SharedString::from(embed.alt.clone());
                let mut image = img(SharedUri::from(url.clone()))
                    .max_w_full()
                    .object_fit(ObjectFit::Fill)
                    .with_fallback(move || div().child(alt.clone()).into_any_element());
                if let Some(w) = embed.width {
                    image = image.w(px(w));
                }
                if let Some(h) = embed.height {
                    image = image.h(px(h));
                }
                div().my_1().child(image).into_any_element()
            }
            EmbedKind::Note {
                path,
                title,
                blocks,
            } => {
                let on_link = self.on_link.clone();
                let link = embed.link.clone();
                div()
                    .my_2()
                    .border_1()
                    .border_color(t.tokens.background_modifier_border)
                    .rounded_md()
                    .px_4()
                    .py_3()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("embed-title", path.len() + title.len()))
                            .mb_2()
                            .text_size(px(base * 0.75))
                            .text_color(t.tokens.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.tokens.link_color))
                            .on_click(move |_, window, cx| {
                                on_link(LinkTarget::Internal(link.clone()), window, cx)
                            })
                            .child(SharedString::from(format!("↗ {title}"))),
                    )
                    .children(self.blocks(blocks, false))
                    .into_any_element()
            }
            // `![[x.pdf]]`: `pdf::embed` draws the pages, asking the document cache
            // when painted; the model held only path and subpath.
            EmbedKind::Pdf(pdf) => {
                let on_link = self.on_link.clone();
                let open: crate::pdf::embed::OpenPath =
                    Rc::new(move |path: &str, window: &mut Window, cx: &mut App| {
                        on_link(LinkTarget::Internal(path.to_string()), window, cx)
                    });
                crate::pdf::embed::render(pdf, t, self.available, open)
            }
            // `![[x.canvas]]`: a minimap under the name; a click opens the canvas.
            EmbedKind::Canvas(canvas) => {
                let on_link = self.on_link.clone();
                let open: crate::canvas::minimap::OpenPath =
                    Rc::new(move |path: &str, window: &mut Window, cx: &mut App| {
                        on_link(LinkTarget::Internal(path.to_string()), window, cx)
                    });
                crate::canvas::minimap::render(canvas, t, self.available, open)
            }
            // `![[x.mp3]]`: the player; `![[x.mp4]]`: TFT's card (`media`).
            EmbedKind::Audio(media) => crate::media::audio(media, t),
            EmbedKind::Video(media) => crate::media::video(media, t),
            // A missing canvas is offered for creation: `“Missing.canvas” is not created yet. Click to create.`.
            EmbedKind::Missing
                if FileKind::from_path(tft_markdown::parse_linktext(&embed.link).path)
                    == FileKind::Canvas =>
            {
                let on_link = self.on_link.clone();
                let link = embed.link.clone();
                div()
                    .id(SharedString::from(format!("embed-missing:{}", embed.link)))
                    .my_1()
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(t.tokens.background_modifier_hover)
                    .text_size(px(14.))
                    .text_color(t.tokens.text_muted)
                    .text_center()
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        on_link(LinkTarget::Internal(link.clone()), window, cx)
                    })
                    .child(SharedString::from(format!(
                        "\u{201c}{}\u{201d} is not created yet. Click to create.",
                        embed.link
                    )))
                    .into_any_element()
            }
            // A missing attachment is Obsidian's `file-embed
            // mod-empty-attachment` box: 8 px padding and radius, 14 px muted text
            // reading `“missing.pdf” could not be found.` A missing note keeps its own line.
            EmbedKind::Missing
                if FileKind::from_path(tft_markdown::parse_linktext(&embed.link).path)
                    != FileKind::Markdown =>
            {
                div()
                    .my_1()
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(t.tokens.background_modifier_hover)
                    .text_size(px(14.))
                    .text_color(t.tokens.text_muted)
                    .child(SharedString::from(format!(
                        "\u{201c}{}\u{201d} could not be found.",
                        embed.link
                    )))
                    .into_any_element()
            }
            EmbedKind::Missing => div()
                .my_1()
                .text_color(t.tokens.text_muted)
                .italic()
                .child(SharedString::from(format!("{}: file not found", embed.alt)))
                .into_any_element(),
            EmbedKind::Unsupported(kind) => {
                let on_link = self.on_link.clone();
                let link = embed.link.clone();
                div()
                    .id(("embed-file", embed.link.len()))
                    .my_1()
                    .text_color(t.tokens.link_color)
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        on_link(LinkTarget::Internal(link.clone()), window, cx)
                    })
                    .child(SharedString::from(format!("{} ({kind:?})", embed.alt)))
                    .into_any_element()
            }
            EmbedKind::TooDeep => div()
                .my_1()
                .text_color(t.tokens.text_muted)
                .italic()
                .child(SharedString::from(format!(
                    "{}: embed nested too deep",
                    embed.alt
                )))
                .into_any_element(),
        }
    }

    /// The properties block: a foldable heading and one row per property with its
    /// type glyph, key and value. Without a [`PropertiesHost`] the rows are
    /// read-only except a pill's remove control ([`ReadingHost::on_remove_item`]).
    pub(crate) fn properties(&self, entries: &[Property]) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let host = self.properties_host;
        let hidden = host.is_some_and(|h| h.mode == PropertiesInDocument::Hidden)
            || self
                .reading_host
                .is_some_and(|h| h.properties == PropertiesInDocument::Hidden);
        if hidden {
            // "Hide": the YAML stays in the file, shown by neither view.
            return div().into_any_element();
        }
        let heading = host.is_none_or(|h| h.heading);
        let folded = heading && self.toggled.contains(&PROPERTIES_FOLD_ID);
        let mut next_id = PROPERTY_ID_BASE;
        let rows = crate::properties::rows(
            entries,
            |key| tft_formats::properties::type_of(key, entries, self.property_types),
            &mut next_id,
        );
        let on_toggle = self.on_toggle.clone();
        let add = host.map(|h| h.on_action.clone());
        // `.metadata-container`: `--text-muted` in a `--metadata-border-color` line.
        div()
            .mb(px(base * 1.2))
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(t.tokens.metadata_border_color)
            .pb(px(base * 0.6))
            .text_size(px(base * 0.875))
            .when(!heading, |d| d.mb_0().border_b_0())
            .when(heading, |d| {
                d.child(
                    div()
                        .id("properties-heading")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .py_1()
                        .cursor_pointer()
                        .text_color(t.tokens.text_normal)
                        .font_weight(FontWeight::MEDIUM)
                        .child(SharedString::from(if folded {
                            "\u{25b8}"
                        } else {
                            "\u{25be}"
                        }))
                        .child(SharedString::from("Properties"))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            on_toggle(PROPERTIES_FOLD_ID, cx);
                        }),
                )
            })
            .when(!folded, |d| {
                d.children(rows.into_iter().map(|row| self.property_row(row)))
            })
            .when_some(
                host.and_then(|h| h.field.clone())
                    .filter(|f| f.target == Target::New && !folded),
                |d, field| {
                    // The row "Add file property" opened: a key field, nothing written yet.
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .py_1()
                            .child(
                                div()
                                    .w(px(base))
                                    .flex_shrink_0()
                                    .text_color(t.tokens.text_faint)
                                    .child(SharedString::from("T")),
                            )
                            .child(self.property_field(&field, false)),
                    )
                },
            )
            .when_some(
                add.filter(|_| {
                    !folded
                        && !host.is_some_and(|h| {
                            h.field.as_ref().is_some_and(|f| f.target == Target::New)
                        })
                }),
                |d, on_action| {
                    // Obsidian's "Add file property" row; nothing is written until named.
                    d.child(
                        div()
                            .id("properties-add")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .py_1()
                            .cursor_pointer()
                            .text_color(t.tokens.text_faint)
                            .child(SharedString::from("+"))
                            .child(SharedString::from("Add property"))
                            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                on_action(PropertyAction::AddRow, window, cx);
                            }),
                    )
                },
            )
            .into_any_element()
    }

    /// The caret-bearing field a row shows while it is being typed into.
    fn property_field(&self, field: &Field, wide: bool) -> AnyElement {
        let t = self.theme;
        let before = &field.text[..field.cursor];
        let after = &field.text[field.cursor..];
        div()
            .flex()
            .flex_row()
            .items_center()
            .when(wide, |d| d.flex_1())
            .min_w_0()
            .px_1()
            .rounded_sm()
            .bg(t.tokens.background_modifier_hover)
            .child(SharedString::from(before.to_string()))
            .child(
                div()
                    .w(px(1.))
                    .h(px(t.base_font_size))
                    .bg(t.tokens.interactive_accent)
                    .flex_shrink_0(),
            )
            .child(SharedString::from(after.to_string()))
            .into_any_element()
    }

    fn property_row(&self, row: crate::properties::Row) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        let host = self.properties_host;
        // The type menu opens under the row, so a narrow column keeps the value's width.
        let menu = host
            .filter(|h| h.menu.as_deref() == Some(row.key.as_str()))
            .map(|h| self.property_type_menu(&row, h));
        let line = div()
            .flex()
            .flex_row()
            .items_start()
            .gap_2()
            .py_1()
            .child(
                // The type glyph and key share the left column.
                div()
                    .w(px(160.))
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .text_color(t.tokens.text_muted)
                    .child({
                        let glyph = div()
                            .w(px(base))
                            .flex_shrink_0()
                            .text_color(t.tokens.text_faint)
                            .child(SharedString::from(crate::properties::type_glyph(row.ty)));
                        match host.map(|h| h.on_action.clone()) {
                            Some(on_action) => {
                                let key = row.key.clone();
                                glyph
                                    .id(("property-type", row.id))
                                    .cursor_pointer()
                                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                        on_action(
                                            PropertyAction::OpenTypeMenu { key: key.clone() },
                                            window,
                                            cx,
                                        );
                                    })
                                    .into_any_element()
                            }
                            None => glyph.into_any_element(),
                        }
                    })
                    .child(match self.open_field(&row, Part::Key) {
                        Some(field) => self.property_field(field, true),
                        None => {
                            let cell = div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .overflow_hidden()
                                .child(SharedString::from(row.key.clone()));
                            match host.map(|h| h.on_action.clone()) {
                                Some(on_action) => {
                                    let key = row.key.clone();
                                    cell.id(("property-key", row.id))
                                        .cursor_pointer()
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            move |_, window, cx| {
                                                on_action(
                                                    PropertyAction::Edit {
                                                        target: Target::Existing(key.clone()),
                                                        part: Part::Key,
                                                    },
                                                    window,
                                                    cx,
                                                );
                                            },
                                        )
                                        .into_any_element()
                                }
                                None => cell.into_any_element(),
                            }
                        }
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(match self.open_field(&row, Part::Value) {
                        Some(field) => self.property_field(field, true),
                        None => self.property_value_cell(&row, host),
                    })
                    .when(row.mismatched, |d| {
                        // A value that does not fit its type is marked, its text kept.
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_color(t.tokens.text_faint)
                                .child(SharedString::from("\u{26a0}")),
                        )
                    })
                    .when_some(host.map(|h| h.on_action.clone()), |d, on_action| {
                        let key = row.key.clone();
                        d.child(
                            div()
                                .id(("property-remove", row.id))
                                .flex_shrink_0()
                                .px_1()
                                .cursor_pointer()
                                .text_color(t.tokens.text_faint)
                                .child(SharedString::from("\u{00d7}"))
                                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                    on_action(
                                        PropertyAction::Remove { key: key.clone() },
                                        window,
                                        cx,
                                    );
                                }),
                        )
                    }),
            );
        div()
            .relative()
            .flex()
            .flex_col()
            .child(line)
            // A popover, not a row: it floats over the rows below, so opening
            // it moves nothing in the block.
            .children(menu.map(deferred))
            .into_any_element()
    }

    /// The type menu from a row's icon; `tags` and `aliases` are reserved and not offered.
    fn property_type_menu(
        &self,
        row: &crate::properties::Row,
        host: &PropertiesHost,
    ) -> AnyElement {
        let t = self.theme;
        let base = t.base_font_size;
        div()
            // Anchored under the type glyph: the menu is out of flow and
            // paints after the rows (deferred by the caller).
            .absolute()
            .top(px(base * 0.875 * LINE_HEIGHT + 8.))
            .left(px(24.))
            .min_w(px(180.))
            .flex()
            .flex_col()
            .border_1()
            .border_color(t.tokens.background_modifier_border)
            .rounded_md()
            .shadow_md()
            .bg(t.tokens.background_secondary)
            .py_1()
            .children(PropertyType::selectable().iter().copied().map(|ty| {
                let on_action = host.on_action.clone();
                let key = row.key.clone();
                div()
                    .id(("property-type-option", row.id * 16 + ty as usize))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_0p5()
                    .cursor_pointer()
                    .when(ty == row.ty, |d| d.text_color(t.tokens.interactive_accent))
                    .hover(|d| d.bg(t.tokens.background_modifier_hover))
                    .child(SharedString::from(crate::properties::type_glyph(ty)))
                    .child(SharedString::from(ty.label()))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        on_action(
                            PropertyAction::Retype {
                                key: key.clone(),
                                ty,
                            },
                            window,
                            cx,
                        );
                    })
            }))
            .into_any_element()
    }

    /// The open field when it belongs to this row and part; an item's field is drawn among pills.
    fn open_field(&self, row: &crate::properties::Row, part: Part) -> Option<&Field> {
        let field = self.properties_host?.field.as_ref()?;
        (field.part == part
            && field.key() == Some(row.key.as_str())
            && field.item_index().is_none())
        .then_some(field)
    }

    /// Which item the open field holds; past the last is the empty input after the pills.
    fn open_item(&self, row: &crate::properties::Row) -> Option<(usize, &Field)> {
        let field = self.properties_host?.field.as_ref()?;
        Some((field.item_of(row)?, field))
    }

    /// The value cell when no field is open: read-only here; in Live Preview a
    /// checkbox flips and anything else opens a field.
    fn property_value_cell(
        &self,
        row: &crate::properties::Row,
        host: Option<&PropertiesHost>,
    ) -> AnyElement {
        let Some(on_action) = host.map(|h| h.on_action.clone()) else {
            return self.property_value(row);
        };
        if let crate::properties::Display::Checkbox(checked) = row.value {
            let key = row.key.clone();
            let on_toggle = on_action.clone();
            return div()
                .id(("property-checkbox", row.id))
                .flex_1()
                .min_w_0()
                .cursor_pointer()
                .child(self.property_value(row))
                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    let _ = checked;
                    on_toggle(PropertyAction::Toggle { key: key.clone() }, window, cx);
                })
                .into_any_element();
        }
        let key = row.key.clone();
        // A list never opens whole: a click lands in the empty input after the pills.
        let append = row.is_list_valued().then_some(row.items.len());
        div()
            .id(("property-value", row.id))
            .flex_1()
            .min_w_0()
            .cursor_text()
            .child(self.property_value(row))
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                let action = match append {
                    Some(index) => PropertyAction::EditItem {
                        key: key.clone(),
                        index,
                    },
                    None => PropertyAction::Edit {
                        target: Target::Existing(key.clone()),
                        part: Part::Value,
                    },
                };
                on_action(action, window, cx);
            })
            .into_any_element()
    }

    fn property_value(&self, row: &crate::properties::Row) -> AnyElement {
        let t = self.theme;
        let open = self.open_item(row);
        match &row.value {
            crate::properties::Display::Scalar(text) => div()
                .flex_1()
                .min_w_0()
                .child(self.rich(text))
                .into_any_element(),
            crate::properties::Display::Pills(items) if !row.is_blank() => {
                self.property_pills(row, items, open)
            }
            // An empty value still takes a new item; its field is all the cell draws.
            crate::properties::Display::Empty | crate::properties::Display::Pills(_)
                if open.is_some() =>
            {
                self.property_pills(row, &[], open)
            }
            // The box keeps its size; the cell takes the width, keeping affordances at the edge.
            crate::properties::Display::Checkbox(checked) => div()
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .w(px(t.base_font_size))
                        .h(px(t.base_font_size))
                        .border_1()
                        .border_color(t.tokens.background_modifier_border)
                        .rounded_sm()
                        .when(*checked, |d| {
                            d.bg(t.tokens.interactive_accent)
                                .text_color(t.tokens.background_primary)
                                .child(SharedString::from("\u{2713}"))
                        }),
                )
                .into_any_element(),
            // A blank cell (`aliases: []`, an empty key) must draw something or it has no click.
            crate::properties::Display::Empty | crate::properties::Display::Pills(_) => div()
                .text_color(t.tokens.text_faint)
                .child(SharedString::from("Empty"))
                .into_any_element(),
        }
    }

    /// A list value: one pill per item, the open field standing in for its item,
    /// so editing one never touches the others; a field past the last draws last.
    ///
    /// Live Preview's pill opens that item and drops it from its remove button;
    /// the reading view only draws the remove button, routed to the workspace
    /// through [`ReadingHost::on_remove_item`].
    fn property_pills(
        &self,
        row: &crate::properties::Row,
        items: &[RichText],
        open: Option<(usize, &Field)>,
    ) -> AnyElement {
        let t = self.theme;
        let on_action = self.properties_host.map(|h| h.on_action.clone());
        // Drawn for whichever host can take the item away: either sink.
        let remove = on_action
            .clone()
            .or_else(|| self.reading_host.and_then(|h| h.on_remove_item.clone()));
        // The pill a Backspace selected, drawn focused until the next press removes it.
        let selected = open.and_then(|_| self.properties_host.and_then(|h| h.selected_pill));
        // A click in the open field is not one on the cell behind.
        let item_field = |field: &Field| {
            div()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(self.property_field(field, false))
                .into_any_element()
        };
        let mut cells: Vec<AnyElement> = Vec::new();
        for (ix, item) in items.iter().enumerate() {
            if let Some((at, field)) = open {
                if at == ix {
                    cells.push(item_field(field));
                    continue;
                }
            }
            cells.push(
                div()
                    .id(("property-pill", item.id))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_1p5()
                    .rounded_sm()
                    // `.multi-select-pill`: `--pill-color` on `--pill-background` in a
                    // `--pill-border-color` line, brightening to the focus ring when selected.
                    .bg(t.tokens.pill_background)
                    .text_color(t.tokens.pill_color)
                    .border_1()
                    .border_color(if selected == Some(ix) {
                        t.tokens.background_modifier_border_focus
                    } else {
                        t.tokens.pill_border_color
                    })
                    .child(self.rich(item))
                    .when_some(on_action.clone(), |d, on_action| {
                        let key = row.key.clone();
                        d.cursor_text().on_mouse_down(
                            gpui::MouseButton::Left,
                            move |_, window, cx| {
                                // The cell behind opens the append input; the pill opens itself.
                                cx.stop_propagation();
                                on_action(
                                    PropertyAction::EditItem {
                                        key: key.clone(),
                                        index: ix,
                                    },
                                    window,
                                    cx,
                                );
                            },
                        )
                    })
                    .when_some(remove.clone(), |d, remove| {
                        let key = row.key.clone();
                        d.child(
                            div()
                                .id(("property-pill-remove", item.id))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                // `.multi-select-pill-remove-button`: a 14 px `lucide-x`, always drawn.
                                .child(crate::assets::icon(
                                    "x",
                                    t.base_font_size * 0.875,
                                    t.tokens.text_faint,
                                ))
                                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                    cx.stop_propagation();
                                    remove(
                                        PropertyAction::RemoveItem {
                                            key: key.clone(),
                                            index: ix,
                                        },
                                        window,
                                        cx,
                                    );
                                }),
                        )
                    })
                    .into_any_element(),
            );
        }
        if let Some((_, field)) = open.filter(|(at, _)| *at >= items.len()) {
            // A selected pill holds the focus, so the empty input shows no caret.
            if selected.is_none() {
                cells.push(item_field(field));
            }
        }
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_1()
            .children(cells)
            .into_any_element()
    }

    fn rich(&self, text: &RichText) -> AnyElement {
        self.rich_at(text, self.theme.base_font_size)
    }

    /// Inline content at `size`: one `InteractiveText`, with formulas and
    /// differently sized runs (`<sup>`, `<sub>`, `<small>`, footnote references)
    /// typeset over placeholders.
    pub(crate) fn rich_at(&self, text: &RichText, size: f32) -> AnyElement {
        // Each `$x$` run in text style; an unparsable one stays its source in error red.
        let formulas: Vec<(Range<usize>, Arc<tft_math::Formula>)> = text
            .runs
            .iter()
            .filter(|(_, s)| s.math)
            .filter_map(|(r, _)| {
                let tex = text.text.get(r.clone())?;
                math::typeset(tex, false).ok().map(|f| (r.clone(), f))
            })
            .collect();
        // A right-to-left paragraph is laid out by TFT, since gpui cannot wrap it (`crate::rtl`).
        if formulas.is_empty() && crate::rtl::is_rtl(&text.text) {
            return self.rtl_text(text);
        }
        let scripts: Vec<(Range<usize>, InlineStyle, Script)> = text
            .runs
            .iter()
            .filter(|(r, s)| !s.math && !r.is_empty() && !text.text[r.clone()].contains('\n'))
            .filter_map(|(r, s)| s.script().map(|sc| (r.clone(), *s, sc)))
            .collect();
        if formulas.is_empty() && scripts.is_empty() {
            let (interactive, layout) =
                self.interactive_with_layout(text, &text.text, &text.runs, &text.links, None);
            let element = self.selectable(interactive.into_any_element(), text, &layout, None);
            return self.with_link_menu(element, layout, text.id, text.links.clone());
        }
        let t = self.theme;
        let em = px(size);
        let font = Font {
            family: t.text_font.clone(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let spaces = SpaceWidths::measure(&self.text_system, &font, em);
        enum Stand {
            Formula(Arc<tft_math::Formula>),
            Script(InlineStyle, Script),
        }
        let mut stands: Vec<(Range<usize>, Stand)> = formulas
            .into_iter()
            .map(|(r, f)| (r, Stand::Formula(f)))
            .chain(
                scripts
                    .into_iter()
                    .map(|(r, s, sc)| (r, Stand::Script(s, sc))),
            )
            .collect();
        stands.sort_by_key(|(r, _)| r.start);
        // Rebuild with a placeholder per formula or scripted run, carrying runs and links across.
        let mut edits: Vec<(Range<usize>, usize)> = Vec::new();
        let mut shaped = String::with_capacity(text.text.len());
        let mut inlines = Vec::new();
        let mut texts = Vec::new();
        let mut at = 0;
        for (range, stand) in &stands {
            if range.start < at {
                continue;
            }
            shaped.push_str(&text.text[at..range.start]);
            let start = shaped.len();
            match stand {
                Stand::Formula(formula) => {
                    let (placeholder, slot) = spaces.placeholder(math::width_px(formula, em));
                    shaped.push_str(&placeholder);
                    inlines.push(Inline {
                        range: start..shaped.len(),
                        formula: formula.clone(),
                        slot,
                    });
                }
                Stand::Script(style, script) => {
                    let own = &text.text[range.clone()];
                    let (scaled, _) = script.at(size);
                    let width = self.measure_styled(own, scaled, *style);
                    let (placeholder, _) = spaces.nearest(px(width));
                    shaped.push_str(&placeholder);
                    texts.push(ScriptText {
                        range: start..shaped.len(),
                        text: SharedString::from(own.to_string()),
                        highlight: self.highlight(*style),
                        family: (style.code || style.kbd || style.html)
                            .then(|| t.mono_font.clone()),
                        script: *script,
                    });
                }
            }
            edits.push((range.clone(), shaped.len() - start));
            at = range.end;
        }
        shaped.push_str(&text.text[at..]);
        let remap = Rc::new(Remap { edits });
        let runs: Vec<(Range<usize>, InlineStyle)> = text
            .runs
            .iter()
            .filter(|(r, _)| !stands.iter().any(|(f, _)| f == r))
            .map(|(r, s)| (remap.to_shaped(r.start)..remap.to_shaped(r.end), *s))
            .collect();
        let links: Vec<(Range<usize>, LinkTarget)> = text
            .links
            .iter()
            .map(|(r, l)| (remap.to_shaped(r.start)..remap.to_shaped(r.end), l.clone()))
            .collect();
        let (interactive, layout) =
            self.interactive_with_layout(text, &shaped, &runs, &links, Some(&remap));
        let overlay = Overlay::new(
            interactive.into_any_element(),
            layout.clone(),
            inlines,
            em,
            t.tokens.text_normal,
        )
        .with_texts(texts)
        .into_any_element();
        let element = self.selectable(overlay, text, &layout, Some(remap));
        self.with_link_menu(element, layout, text.id, links)
    }

    fn measure_styled(&self, text: &str, size: f32, s: InlineStyle) -> f32 {
        let t = self.theme;
        let font = Font {
            family: if s.code || s.kbd || s.html {
                t.mono_font.clone()
            } else {
                t.text_font.clone()
            },
            features: Default::default(),
            fallbacks: None,
            weight: if s.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if s.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
        };
        let run = TextRun {
            len: text.len(),
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = self.text_system.shape_line(
            SharedString::from(text.to_string()),
            px(size),
            &[run],
            None,
        );
        f32::from(line.width)
    }

    /// The text element, with a right click on a link raising the host's menu;
    /// only the reading view does. It also reports the pointer entering and
    /// leaving a link where the host previews links.
    fn with_link_menu(
        &self,
        child: AnyElement,
        layout: TextLayout,
        text_id: usize,
        links: Vec<(Range<usize>, LinkTarget)>,
    ) -> AnyElement {
        let on_menu = self.reading_host.and_then(|h| h.on_menu.clone());
        let on_link = self.reading_host.map(|h| h.on_link.clone());
        let link_pressed = self.reading_host.map(|h| h.link_pressed.clone());
        let hover = self.reading_host.and_then(|h| h.hover.clone());
        if (on_menu.is_none() && on_link.is_none() && hover.is_none()) || links.is_empty() {
            return child;
        }
        LinkMenu {
            child,
            layout,
            text_id,
            links,
            on_menu,
            on_link,
            link_pressed,
            hover,
        }
        .into_any_element()
    }

    /// Wraps a run so a press, drag or multiple click reaches the view with its
    /// byte offset. A layout answers hit tests only once laid out *and*
    /// prepainted, and gpui registers listeners while painting, by when it is both.
    fn selectable(
        &self,
        element: AnyElement,
        text: &RichText,
        layout: &gpui::TextLayout,
        remap: Option<Rc<Remap>>,
    ) -> AnyElement {
        let Some(host) = self.select else {
            return element;
        };
        let id = text.id;
        // A hit in the laid-out string, as an offset of the paragraph's own.
        let at = move |layout: &gpui::TextLayout, position| {
            let shaped = index_at(layout, position);
            remap.as_ref().map_or(shaped, |r| r.to_model(shaped))
        };
        let at2 = at.clone();
        div()
            .child(element)
            .on_mouse_down(MouseButton::Left, {
                let (on_select, layout) = (host.on_select.clone(), layout.clone());
                move |event: &gpui::MouseDownEvent, window, cx| {
                    let gesture = match event.click_count {
                        0 | 1 => SelectGesture::Press,
                        2 => SelectGesture::Word,
                        _ => SelectGesture::Line,
                    };
                    on_select(gesture, id, at(&layout, event.position), window, cx);
                }
            })
            .on_mouse_move({
                let (on_select, layout) = (host.on_select.clone(), layout.clone());
                move |event: &gpui::MouseMoveEvent, window, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        on_select(
                            SelectGesture::Drag,
                            id,
                            at2(&layout, event.position),
                            window,
                            cx,
                        );
                    }
                }
            })
            .into_any_element()
    }

    /// The `InteractiveText` of `string` styled by `runs`, with its layout handle.
    fn interactive_with_layout(
        &self,
        text: &RichText,
        string: &str,
        runs: &[(Range<usize>, InlineStyle)],
        links: &[(Range<usize>, LinkTarget)],
        remap: Option<&Remap>,
    ) -> (InteractiveText, gpui::TextLayout) {
        let t = self.theme;
        let highlights: Vec<_> = runs
            .iter()
            .map(|(range, style)| (range.clone(), self.highlight(*style)))
            .collect();
        let mono: Vec<_> = runs
            .iter()
            .filter(|(_, s)| s.code || s.kbd || s.html)
            .map(|(range, _)| (range.clone(), t.mono_font.clone()))
            .collect();
        // The selection merges into the highlight list, so it sits behind the runs' styles.
        let highlights = match self
            .select
            .and_then(|h| h.ranges.get(&text.id).map(|range| (range.clone(), h.color)))
            .map(|(range, color)| match remap {
                Some(r) => (r.to_shaped(range.start)..r.to_shaped(range.end), color),
                None => (range, color),
            }) {
            Some((range, color)) => with_selection(highlights, string, range, color),
            None => highlights,
        };
        let mut styled =
            StyledText::new(SharedString::from(string.to_string())).with_highlights(highlights);
        if !mono.is_empty() {
            styled = styled.with_font_family_overrides(mono);
        }
        let layout = styled.layout().clone();
        let ranges: Vec<_> = links.iter().map(|(r, _)| r.clone()).collect();
        let targets: Vec<LinkTarget> = links.iter().map(|(_, l)| l.clone()).collect();
        let on_link = self.on_link.clone();
        let interactive = InteractiveText::new(("rich", text.id), styled).on_click(
            ranges,
            move |ix, window, cx| {
                if let Some(target) = targets.get(ix).cloned() {
                    on_link(target, window, cx);
                }
            },
        );
        (interactive, layout)
    }

    /// Right-to-left inline content, right-aligned in rows of the block's width.
    fn rtl_text(&self, text: &RichText) -> AnyElement {
        let highlights: Vec<_> = text
            .runs
            .iter()
            .map(|(range, style)| (range.clone(), self.highlight(*style)))
            .collect();
        let mono: Vec<_> = text
            .runs
            .iter()
            .filter(|(_, s)| s.code || s.kbd || s.html)
            .map(|(range, _)| (range.clone(), self.theme.mono_font.clone()))
            .collect();
        crate::rtl::RtlText::new(SharedString::from(text.text.clone()), highlights, mono)
            .into_any_element()
    }

    /// The inline runs, by Obsidian's tokens: `<del>` struck through in its own
    /// colour, `==mark==` on `--highlight-background`, inline code on
    /// `--code-background`, tags in `--tag-color`, links in `--link-color`.
    fn highlight(&self, s: InlineStyle) -> HighlightStyle {
        let t = self.theme;
        let k = &t.tokens;
        let mut h = HighlightStyle::default();
        if s.bold {
            h.font_weight = Some(FontWeight::BOLD);
        }
        if s.italic {
            h.font_style = Some(FontStyle::Italic);
        }
        if s.strike {
            h.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if s.highlight {
            h.background_color = Some(match s.highlight_color {
                Some(c) => t.highlight_color(c),
                None => k.highlight_background,
            });
        }
        // `<kbd>` wears inline code's look, the box Obsidian themes it with.
        if s.code || s.kbd {
            h.background_color = Some(k.code_background);
            h.color = Some(k.code_normal);
        }
        // `<u>`: underlined in the text colour; a link's underline takes over.
        if s.underline {
            h.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            });
        }
        // An inline `style` colour and background (or `<font color>`): after the
        // tags, as an author style wins, and before a link's colour.
        if let Some(c) = s.color {
            h.color = Some(rgba_hsla(c));
        }
        if let Some(c) = s.background {
            h.background_color = Some(rgba_hsla(c));
        }
        if s.math {
            // Only a formula that failed to typeset is still text here.
            h.color = Some(math::error_color());
        }
        if s.html {
            h.color = Some(k.text_muted);
        }
        if s.tag {
            h.color = Some(k.tag_color);
        }
        if s.footnote {
            h.color = Some(k.link_color);
        }
        if s.link {
            h.color = Some(k.link_color);
            h.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(k.link_color),
                wavy: false,
            });
        }
        h
    }
}

/// A table cell's horizontal padding, both sides, plus its border.
const TABLE_CELL_PADDING: f32 = 17.;

/// A `0xRRGGBBAA` colour as gpui's.
pub(crate) fn rgba_hsla(c: u32) -> Hsla {
    gpui::rgba(c).into()
}

/// Shaped text whose links answer a right click: wraps the text element, reads
/// the click's byte from the layout, and raises the host's menu only inside a
/// link's range.
///
/// It wraps rather than sitting at the view's root (the reading view has no text
/// menu elsewhere), and takes the mouse down so the `InteractiveText` underneath
/// does not treat a right-button down as a click.
struct LinkMenu {
    child: AnyElement,
    layout: TextLayout,
    text_id: usize,
    links: Vec<(Range<usize>, LinkTarget)>,
    on_menu: Option<MenuHandler>,
    on_link: Option<LinkHandler>,
    link_pressed: Option<Rc<std::cell::Cell<Option<(usize, usize)>>>>,
    hover: Option<HoverHost>,
}

impl IntoElement for LinkMenu {
    type Element = LinkMenu;

    fn into_element(self) -> LinkMenu {
        self
    }
}

impl Element for LinkMenu {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.child.prepaint(window, cx);
        // `Normal`: reports hover and occludes nothing, so left clicks still work.
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        let hitbox = hitbox.clone();
        let layout = self.layout.clone();
        let links = Rc::new(std::mem::take(&mut self.links));
        if let Some(host) = self.hover.clone() {
            let (hitbox, layout, links, id) =
                (hitbox.clone(), layout.clone(), links.clone(), self.text_id);
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let over = (event.pressed_button.is_none() && hitbox.is_hovered(window))
                    .then(|| layout.index_for_position(event.position).ok())
                    .flatten()
                    .and_then(|at| links.iter().position(|(range, _)| range.contains(&at)));
                let now = host.current.get();
                match over {
                    Some(ix) if now != Some((id, ix)) => {
                        host.current.set(Some((id, ix)));
                        let (range, target) = &links[ix];
                        let Some(origin) = layout.position_for_index(range.start) else {
                            return;
                        };
                        let anchor = Bounds::new(origin, gpui::size(px(1.), layout.line_height()));
                        (host.on_hover)(
                            Some(LinkHover {
                                target: target.clone(),
                                anchor,
                                modifiers: event.modifiers,
                            }),
                            window,
                            cx,
                        );
                    }
                    None if now.is_some_and(|(t, _)| t == id) => {
                        host.current.set(None);
                        (host.on_hover)(None, window, cx);
                    }
                    _ => {}
                }
            });
        }
        if let (Some(on_link), Some(pressed)) = (self.on_link.clone(), self.link_pressed.clone()) {
            let (hitbox, layout, links, id, pressed) = (
                hitbox.clone(),
                layout.clone(),
                links.clone(),
                self.text_id,
                pressed,
            );
            window.on_mouse_event({
                let hitbox = hitbox.clone();
                let layout = layout.clone();
                let links = links.clone();
                let pressed = pressed.clone();
                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let Some(index) = layout
                        .index_for_position(event.position)
                        .ok()
                        .and_then(|at| links.iter().position(|(range, _)| range.contains(&at)))
                    else {
                        return;
                    };
                    pressed.set(Some((id, index)));
                    // A link press belongs to navigation, not text selection.
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                let Some((pressed_id, pressed_index)) = pressed.get() else {
                    return;
                };
                if pressed_id != id {
                    return;
                }
                pressed.set(None);
                cx.stop_propagation();
                if !hitbox.is_hovered(window) {
                    return;
                }
                let Some(released_index) = layout
                    .index_for_position(event.position)
                    .ok()
                    .and_then(|at| links.iter().position(|(range, _)| range.contains(&at)))
                else {
                    return;
                };
                if pressed_index == released_index {
                    on_link(links[released_index].1.clone(), window, cx);
                }
            });
        }
        let Some(on_menu) = self.on_menu.clone() else {
            return;
        };
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Right
                || !hitbox.is_hovered(window)
            {
                return;
            }
            // Past a row's end `index_for_position` errs, so a click beside the text raises nothing.
            let Ok(at) = layout.index_for_position(event.position) else {
                return;
            };
            let Some((_, target)) = links.iter().find(|(range, _)| range.contains(&at)) else {
                return;
            };
            cx.stop_propagation();
            on_menu(target.clone(), event.position, window, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        header_height, inline_title, line_distance, ordered, page_distance, runs, same_note,
        selected_ranges, selected_text, shown_title, whole_note, with_selection, word_at,
        ReadingPos, Remap, HEADER_TOP, LINE_HEIGHT,
    };

    /// A scripted run or formula is a placeholder: offsets map both ways and a hit
    /// inside lands on the nearer end, so a selection copies the paragraph's bytes.
    #[test]
    fn offsets_map_across_placeholders_both_ways() {
        // "ab[cd]ef[g]h": `cd` (2..4) became 5 bytes, `g` (6..7) 2 bytes.
        let r = Remap {
            edits: vec![(2..4, 5), (6..7, 2)],
        };
        assert_eq!(r.to_shaped(0), 0);
        assert_eq!(r.to_shaped(2), 2);
        assert_eq!(r.to_shaped(3), 2);
        assert_eq!(r.to_shaped(4), 7);
        assert_eq!(r.to_shaped(6), 9);
        assert_eq!(r.to_shaped(7), 11);
        assert_eq!(r.to_shaped(8), 12);
        for model in [0, 1, 2, 4, 5, 6, 7, 8] {
            assert_eq!(r.to_model(r.to_shaped(model)), model, "{model}");
        }
        assert_eq!(r.to_model(3), 2);
        assert_eq!(r.to_model(6), 4);
        assert_eq!(r.to_model(10), 7);
    }
    use crate::reading::model::{build, EmbedSource, Model};
    use gpui::{hsla, HighlightStyle};
    use std::ops::Range;
    use std::path::PathBuf;

    #[test]
    fn a_page_is_the_viewport_less_a_strip_of_overlap() {
        // Tall viewport: the overlap is the 40 px cap.
        assert_eq!(page_distance(800.), 760.);
        // Short viewport: the overlap is an eighth, so the keys move most of a view.
        assert_eq!(page_distance(160.), 140.);
        // The cap and the eighth meet at 320 px.
        assert_eq!(page_distance(320.), 280.);
    }

    #[test]
    fn a_page_never_scrolls_backwards_and_an_unpainted_view_not_at_all() {
        assert_eq!(page_distance(0.), 0.);
        assert_eq!(page_distance(-1.), 0.);
        for viewport in [1., 10., 41., 320., 1000., 4000.] {
            let d = page_distance(viewport);
            assert!(d > 0., "{viewport} scrolled {d}");
            assert!(d <= viewport, "{viewport} scrolled {d}, past a page");
            assert!(
                d >= viewport * 0.875,
                "{viewport} scrolled only {d}, less than seven eighths"
            );
        }
    }

    #[test]
    fn a_line_is_the_rendered_line_height() {
        assert_eq!(line_distance(16.), 16. * LINE_HEIGHT);
        assert_eq!(line_distance(20.), 32.);
        // An arrow key moves less than a page key at any window's viewport.
        assert!(line_distance(16.) < page_distance(200.));
    }

    #[test]
    fn only_a_reload_of_the_same_note_is_the_same_note() {
        // The reload the watcher drives on every autosave: the reader's
        // place and their folds are kept.
        assert!(same_note(Some("Notes/A.md"), Some("Notes/A.md")));
        // Another note, or no note: both are dropped.
        assert!(!same_note(Some("Notes/A.md"), Some("Notes/B.md")));
        assert!(!same_note(Some("Notes/A.md"), None));
        assert!(!same_note(None, Some("Notes/A.md")));
        // Clearing the view twice is not a reload of a note.
        assert!(!same_note(None, None));
    }

    #[test]
    fn the_inline_title_is_a_notes_basename_without_its_extension() {
        assert_eq!(inline_title("Folder/Note.md"), Some("Note"));
        assert_eq!(inline_title("Note.md"), Some("Note"));
        // Only the extension comes off: a dotted name keeps its dots.
        assert_eq!(inline_title("Notes/v1.2 release.md"), Some("v1.2 release"));
        assert_eq!(inline_title("Notes/README.MD"), Some("README"));
        // An image's synthetic model and a file no view draws are no note, so
        // neither gets a title.
        assert_eq!(inline_title("Attachments/photo.png"), None);
        assert_eq!(inline_title("Board.canvas"), None);
        assert_eq!(inline_title("Paper.pdf"), None);
    }

    #[test]
    fn the_title_shows_only_with_the_setting_on_and_a_note_open() {
        // What `set_note` + `showInlineTitle` come to.
        assert_eq!(shown_title(true, Some("Folder/Note.md")), Some("Note"));
        assert_eq!(shown_title(true, None), None);
        assert_eq!(shown_title(false, Some("Folder/Note.md")), None);
        assert_eq!(shown_title(true, Some("Folder/photo.jpg")), None);
    }

    #[test]
    fn the_header_is_the_top_padding_plus_an_h1_line_and_its_margin() {
        // Without a title the row is only the padding the first block used
        // to carry.
        assert_eq!(header_height(16., false), HEADER_TOP);
        // With one: Obsidian's 25.888 px h1 on a 31.07 px line with a
        // 12.944 px margin below, at the 16 px default.
        let with = header_height(16., true);
        assert!(
            (with - (HEADER_TOP + 31.0656 + 12.944)).abs() < 0.01,
            "{with}"
        );
        // It scales with the body font, as an em-sized heading does.
        assert!(header_height(20., true) > with);
    }

    // ----- selection ------------------------------------------------------------

    /// A vault with nothing in it: the builder wants a source, and these notes
    /// have no embeds.
    struct NoVault;

    impl EmbedSource for NoVault {
        fn resolve(&self, _: &str, _: &str) -> Option<String> {
            None
        }
        fn read(&self, _: &str) -> Option<String> {
            None
        }
        fn absolute(&self, path: &str) -> PathBuf {
            PathBuf::from(path)
        }
    }

    fn model(text: &str) -> Model {
        build("Note.md", text, &NoVault)
    }

    /// The position `offset` bytes into the `n`th run of the note.
    fn at(model: &Model, n: usize, offset: usize) -> ReadingPos {
        let runs = runs(model);
        let run = &runs[n];
        ReadingPos {
            block: run.block,
            rich: run.rich,
            offset,
        }
    }

    fn texts(model: &Model) -> Vec<String> {
        runs(model).iter().map(|r| r.text.to_string()).collect()
    }

    #[test]
    fn the_runs_of_a_note_are_its_inline_text_in_document_order() {
        let m = model("# Title\n\nfirst para\n\n- one\n- two\n\n```\ncode\n```\n\n> quoted\n");
        assert_eq!(
            texts(&m),
            ["Title", "first para", "one", "two", "quoted"],
            "a code block carries no inline model, so it cannot be selected"
        );
        // Every run knows its top-level block, and blocks only move forwards.
        let blocks: Vec<usize> = runs(&m).iter().map(|r| r.block).collect();
        assert!(blocks.windows(2).all(|p| p[0] <= p[1]), "{blocks:?}");
        assert_eq!(blocks[0], 0);
        // The two list items share their block and have ids of their own.
        assert_eq!(blocks[2], blocks[3]);
        assert_ne!(runs(&m)[2].rich, runs(&m)[3].rich);
    }

    #[test]
    fn a_callouts_title_comes_before_its_body() {
        let m = model("> [!note] Heads up\n> the body\n");
        assert_eq!(texts(&m), ["Heads up", "the body"]);
    }

    #[test]
    fn an_anchor_below_the_head_is_ordered_for_painting() {
        let m = model("one\n\ntwo\n");
        let r = runs(&m);
        let (first, second) = (at(&m, 0, 2), at(&m, 1, 1));
        assert_eq!(ordered(&r, second, first), Some((first, second)));
        assert_eq!(ordered(&r, first, second), Some((first, second)));
        // Inside one run it is the offsets that order the pair.
        let (low, high) = (at(&m, 0, 1), at(&m, 0, 3));
        assert_eq!(ordered(&r, high, low), Some((low, high)));
        // An end naming a missing run is no selection, as a leftover anchor is.
        let stray = ReadingPos {
            block: 0,
            rich: usize::MAX,
            offset: 0,
        };
        assert_eq!(ordered(&r, stray, first), None);
        assert_eq!(ordered(&r, first, stray), None);
    }

    #[test]
    fn a_selection_across_blocks_copies_a_line_for_each() {
        let m = model("# Heading\n\nfirst para\n\nsecond para\n");
        let r = runs(&m);
        let (from, to) = (at(&m, 0, 3), at(&m, 2, 6));
        assert_eq!(selected_text(&r, from, to), "ding\nfirst para\nsecond");
        // The same selection dragged the other way copies the same text.
        assert_eq!(selected_text(&r, to, from), "ding\nfirst para\nsecond");
        // Three runs touched: first and last partly, the middle whole.
        assert_eq!(
            selected_ranges(&r, from, to),
            vec![(0, 3..7), (1, 0..10), (2, 0..6)]
        );
    }

    #[test]
    fn a_tables_cells_copy_tab_separated_and_its_rows_by_line() {
        let m = model("| a | b |\n| - | - |\n| c | d |\n");
        let r = runs(&m);
        let (from, to) = whole_note(&r).unwrap();
        assert_eq!(selected_text(&r, from, to), "a\tb\nc\td");
    }

    #[test]
    fn select_all_takes_the_whole_note_and_a_note_with_no_text_nothing() {
        let m = model("alpha\n\nbeta\n");
        let r = runs(&m);
        let (from, to) = whole_note(&r).unwrap();
        assert_eq!(from, at(&m, 0, 0));
        assert_eq!(to, at(&m, 1, 4));
        assert_eq!(selected_text(&r, from, to), "alpha\nbeta");
        // A note of only a code block has no selectable run, so Ctrl+A selects
        // nothing rather than panicking on an end.
        let bare = model("```\ncode\n```\n");
        assert!(runs(&bare).is_empty());
        assert_eq!(whole_note(&runs(&bare)), None);
    }

    #[test]
    fn an_empty_selection_covers_no_runs_and_copies_nothing() {
        let m = model("alpha\n\nbeta\n");
        let r = runs(&m);
        let caret = at(&m, 0, 2);
        assert_eq!(selected_ranges(&r, caret, caret), Vec::new());
        assert_eq!(selected_text(&r, caret, caret), "");
        // Two ends in different runs covering no bytes: nothing is copied, and
        // no stray newline with it.
        assert_eq!(selected_text(&r, at(&m, 0, 5), at(&m, 1, 0)), "");
    }

    #[test]
    fn an_offset_past_a_runs_text_is_clamped_rather_than_slicing_it() {
        let m = model("héllo\n");
        let r = runs(&m);
        // Past the end.
        assert_eq!(selected_text(&r, at(&m, 0, 0), at(&m, 0, 99)), "héllo");
        // Inside the two bytes of the é: the cut walks back below it.
        assert_eq!(selected_text(&r, at(&m, 0, 0), at(&m, 0, 2)), "h");
    }

    #[test]
    fn a_double_click_takes_the_word_under_it() {
        assert_eq!(word_at("the quick brown", 4), 4..9);
        assert_eq!(word_at("the quick brown", 7), 4..9);
        // A press at the very end takes the word before it.
        assert_eq!(word_at("the quick brown", 15), 10..15);
        // Spaces and punctuation are a run of their own, so a press takes
        // something.
        assert_eq!(word_at("a, b", 1), 1..3);
        assert_eq!(word_at("", 0), 0..0);
        // An apostrophe is part of the word, as a browser's word select
        // treats one.
        assert_eq!(word_at("don't stop", 2), 0..5);
    }

    #[test]
    fn the_selection_wash_splits_the_style_runs_it_overlaps() {
        let bold = HighlightStyle {
            font_weight: Some(gpui::FontWeight::BOLD),
            ..Default::default()
        };
        let wash = hsla(0.5, 0.5, 0.5, 0.3);
        // "plain bold plain", the bold at 6..10, selected from inside it into
        // the plain tail.
        let out = with_selection(vec![(6..10, bold)], "plain bold plain", 8..14, wash);
        let ranges: Vec<Range<usize>> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![6..8, 8..10, 10..14]);
        assert_eq!(out[0].1.background_color, None, "outside the selection");
        assert_eq!(out[1].1.background_color, Some(wash));
        assert_eq!(
            out[1].1.font_weight, bold.font_weight,
            "the wash is behind the style the model gave the run"
        );
        assert_eq!(out[2].1.background_color, Some(wash));
        // Sorted and non-overlapping, as `with_highlights` needs.
        assert!(out.windows(2).all(|p| p[0].0.end <= p[1].0.start));
        // An empty selection leaves the runs exactly as they were.
        assert_eq!(
            with_selection(vec![(6..10, bold)], "plain bold plain", 3..3, wash),
            vec![(6..10, bold)]
        );
    }
}
