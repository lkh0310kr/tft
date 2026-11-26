//! Reading-view model: an owned render tree built from the shared parser's
//! [`Document`], mirroring `render_html`'s tree, footnote numbering and embed
//! rules as data a GPUI element tree can be built from.

use std::ops::Range;
use std::path::PathBuf;
use tft_formats::properties::Property;
use tft_formats::FileKind;
use tft_markdown::{
    canonical_callout_type, html::has_scheme, html::percent_decode, parse, plain_text, Alignment,
    Block, BlockKind, Document, Fold, HighlightColor, Inline, InlineKind, LinkKind, ListItem,
    ListKind, Subpath,
};

/// Embedded notes nest at most this deep; Obsidian stops at a similar depth.
pub const MAX_EMBED_DEPTH: usize = 4;

/// What the model builder needs from the vault to resolve embeds.
pub trait EmbedSource {
    /// Resolves a link body written in `source_path` to a vault path.
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String>;
    /// Reads a markdown file's text.
    fn read(&self, path: &str) -> Option<String>;
    /// Absolute path of a vault file, for images.
    fn absolute(&self, path: &str) -> PathBuf;
}

/// Formatting that applies to a run of text; several may apply at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub highlight: bool,
    /// One of the six colours a `==🔴word==` highlight names, or none for `==word==`.
    pub highlight_color: Option<HighlightColor>,
    pub code: bool,
    pub link: bool,
    pub tag: bool,
    pub math: bool,
    pub footnote: bool,
    /// Raw HTML the model cannot map onto a style: drawn as muted source.
    pub html: bool,
    /// `<u>`: underlined in the text colour (a link in the link colour).
    pub underline: bool,
    /// `<kbd>`: drawn the way inline code is.
    pub kbd: bool,
    /// `<sup>`/`<sub>`: five sixths of the size, raised a third plus a pixel or
    /// lowered a fifth plus a pixel ([`InlineStyle::script`]); drawn over a
    /// placeholder, since a gpui text run cannot change size or leave the baseline.
    pub sup: bool,
    pub sub: bool,
    /// `<small>` (five sixths) and `<big>` (six fifths), on the baseline.
    pub small: bool,
    pub big: bool,
    /// An inline `style`/`<font color>` colour and a `background-color`, as `0xRRGGBBAA`.
    pub color: Option<u32>,
    pub background: Option<u32>,
}

/// How a run set at another size than its paragraph sits: the size as a fraction
/// of the paragraph's, and how far its baseline rises (fraction of the paragraph's
/// size plus whole pixels; negative lowers). Chromium's `vertical-align: super`
/// raises a third plus one pixel, `sub` lowers a fifth plus one; `smaller` is five
/// sixths, `larger` six fifths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Script {
    pub scale: f32,
    pub rise_em: f32,
    pub rise_px: f32,
}

impl Script {
    /// The size and the rise, in pixels, over a paragraph set at `size`.
    pub fn at(&self, size: f32) -> (f32, f32) {
        let rise = if self.rise_em == 0. {
            0.
        } else {
            size * self.rise_em + self.rise_px
        };
        (size * self.scale, rise)
    }
}

impl InlineStyle {
    pub fn is_plain(&self) -> bool {
        *self == InlineStyle::default()
    }

    /// The run's size and baseline when it is not the paragraph's: `<sup>`, `<sub>`
    /// (and a footnote reference, an Obsidian `<sup>`), `<small>` and `<big>`. One
    /// level only: a `<sup>` inside a `<sup>` is drawn as one.
    pub fn script(&self) -> Option<Script> {
        let scale = if self.big && !(self.small || self.sup || self.sub) {
            6. / 5.
        } else {
            5. / 6.
        };
        if self.sup || self.footnote {
            Some(Script {
                scale,
                rise_em: 1. / 3.,
                rise_px: 1.,
            })
        } else if self.sub {
            Some(Script {
                scale,
                rise_em: -1. / 5.,
                rise_px: -1.,
            })
        } else if self.small || self.big {
            Some(Script {
                scale,
                rise_em: 0.,
                rise_px: 0.,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A wikilink body or vault-relative markdown link destination, resolved by the workspace.
    Internal(String),
    External(String),
    Tag(String),
    /// Jump to footnote number `n` at the bottom of the note.
    Footnote(usize),
}

/// One line-wrapped run of inline content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichText {
    /// Unique within one model, for GPUI element ids.
    pub id: usize,
    pub text: String,
    /// Non-overlapping styled byte ranges in order; gaps are plain.
    pub runs: Vec<(Range<usize>, InlineStyle)>,
    pub links: Vec<(Range<usize>, LinkTarget)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Text(RichText),
    Embed(Embed),
    /// `$$x$$` in a paragraph: display math, drawn as its own centred block.
    Math(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Embed {
    pub kind: EmbedKind,
    /// The link body as written, for opening the target.
    pub link: String,
    pub alt: String,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmbedKind {
    Image(PathBuf),
    Note {
        path: String,
        /// `Note`, `Note > Heading`, or `Note > ^id`.
        title: String,
        blocks: Vec<RBlock>,
    },
    /// A `.pdf`: path and subpath, nothing read; pages come from the document cache.
    Pdf(crate::pdf::embed::PdfEmbed),
    /// A `.canvas`: its minimap under its name, read at build time; not a live canvas.
    Canvas(crate::canvas::minimap::CanvasEmbed),
    /// An audio file: the player (`media`), nothing read here.
    Audio(crate::media::MediaEmbed),
    /// A video file: the card TFT draws in place of a player (`media`).
    Video(crate::media::MediaEmbed),
    /// `![alt](https://...)`: an `http:`/`https:` image fetched with no prompt,
    /// its alt shown on failure. The URL as written; a wikilink naming a URL is a
    /// missing file, not this ([[Specification#Observed remote image behavior]]).
    Remote(String),
    /// The target file does not exist.
    Missing,
    /// The target exists but TFT cannot render it inline yet.
    Unsupported(FileKind),
    TooDeep,
}

/// Calls `f` on every embed in `blocks`, in document order, descending into
/// embedded notes, quotes, callouts, lists and footnotes.
pub fn each_embed<'a>(blocks: &'a [RBlock], f: &mut dyn FnMut(&'a Embed)) {
    for block in blocks {
        match block {
            RBlock::Paragraph(segments) => {
                for segment in segments {
                    if let Segment::Embed(embed) = segment {
                        f(embed);
                        if let EmbedKind::Note { blocks, .. } = &embed.kind {
                            each_embed(blocks, f);
                        }
                    }
                }
            }
            RBlock::Quote(children) | RBlock::Callout { children, .. } => each_embed(children, f),
            RBlock::List { items, .. } => {
                for item in items {
                    each_embed(&item.children, f);
                }
            }
            RBlock::Footnotes(notes) => {
                for (_, body) in notes {
                    each_embed(body, f);
                }
            }
            _ => {}
        }
    }
}

/// Where a task's `[ ]` marker sits: zero-based line and byte column. The buffer
/// drops a BOM and rewrites `\r\n`, so a file byte offset is not a buffer offset,
/// but every file line is still a buffer line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskMarker {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RItem {
    pub task: Option<char>,
    /// Where the three bytes `[`, status, `]` are for a task of this note; an embedded
    /// item has none, its bytes belonging to another file.
    pub marker: Option<TaskMarker>,
    pub number: Option<u64>,
    pub children: Vec<RBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RBlock {
    /// The properties block: parsed frontmatter entries with byte spans. The renderer
    /// resolves each key's type against `types.json` and builds the rows, read-only
    /// here and editable in Live Preview.
    Properties(Vec<Property>),
    Paragraph(Vec<Segment>),
    Heading {
        level: u8,
        text: RichText,
        plain: String,
    },
    ThematicBreak,
    Code {
        /// Unique within one model, for GPUI element ids.
        id: usize,
        language: Option<String>,
        text: String,
        /// What the copy button writes: the body with `\n` endings and one trailing newline removed.
        copy: String,
        /// A raw `<pre>` wraps its lines (`white-space: pre-wrap`); a fence keeps its columns.
        wrap: bool,
        /// Whether the block has a copy button: a fence always, a raw `<pre>` only with a `<code>`.
        copyable: bool,
    },
    /// A raw `<details>`: its `<summary>` folds the rest, open when the tag says `open`.
    Details {
        /// The summary's own text id, which is also the fold's.
        id: usize,
        open: bool,
        summary: RichText,
        body: Vec<RBlock>,
    },
    Html(String),
    Quote(Vec<RBlock>),
    Callout {
        /// Canonical type, one of Obsidian's built-ins.
        kind: &'static str,
        fold: Option<bool>,
        title: RichText,
        children: Vec<RBlock>,
    },
    List {
        tight: bool,
        items: Vec<RItem>,
    },
    Table {
        alignments: Vec<Alignment>,
        header: Vec<RichText>,
        rows: Vec<Vec<RichText>>,
        /// The bytes each cell stands for, header first, so Live Preview can edit in place.
        spans: Vec<Vec<Range<usize>>>,
        /// What only a raw `<table>` has: a `<caption>` and cell spans; `None` for a Markdown table.
        html: Option<Box<HtmlTableExtras>>,
    },
    Math(String),
    /// `(number, body)` in reference order, rendered after a rule.
    Footnotes(Vec<(usize, Vec<RBlock>)>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HtmlTableExtras {
    /// The `<caption>`: centred over the table, as wide as it, wrapping.
    pub caption: Option<RichText>,
    /// `caption-side: bottom` in the caption's `style`.
    pub caption_bottom: bool,
    /// Where each cell sits when a `colspan`/`rowspan` makes the grid other than one
    /// cell per column: header first, then `rows`, one entry per cell. Empty when no
    /// cell spans more than one.
    pub places: Vec<Vec<CellPlace>>,
    /// The number of columns `places` lays the cells out in.
    pub columns: usize,
}

/// One cell of a raw table with spans: its first column, its column and row counts
/// (resolved: `colspan="0"` is one, `rowspan="0"` and a span past the end run to the
/// section's end, a `colspan` past the last column stops there), and its `align`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPlace {
    pub col: usize,
    pub colspan: usize,
    pub rowspan: usize,
    pub align: Alignment,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub blocks: Vec<RBlock>,
    /// Source byte offset where each top-level block starts, for mapping cache
    /// positions to list items. One entry per block, built from the same list, so a
    /// dropped comment or footnote definition shifts neither.
    pub starts: Vec<usize>,
    /// Source byte offset of footnote `n`'s definition, in Footnotes order. An inline
    /// footnote carries its reference's offset instead.
    pub footnotes: Vec<usize>,
}

impl Model {
    /// Index of the top-level block containing byte `offset`.
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        if self.starts.is_empty() {
            return None;
        }
        Some(match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        })
    }
}

/// Rendering settings that come from `app.json`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// Obsidian's "Strict line breaks": a newline in a paragraph is a space, not a break.
    pub strict_line_breaks: bool,
}

/// Builds the model for `text`, the content of `source_path`, with the default settings.
#[cfg(test)]
pub fn build(source_path: &str, text: &str, vault: &dyn EmbedSource) -> Model {
    build_with(source_path, text, vault, BuildOptions::default())
}

/// Builds the model for `text`, the content of `source_path`.
pub fn build_with(
    source_path: &str,
    text: &str,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Model {
    let doc = parse(text);
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth: 0,
        next_id: 0,
        footnotes: FootnoteState::default(),
        line_starts: None,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let (mut blocks, mut starts) = b.blocks_with_starts(&doc.blocks);
    let mut footnotes = Vec::new();
    if let Some((section, offsets)) = b.footnote_section(&doc) {
        // The appended section needs a start of its own, or a definition offset overshoots.
        starts.push(offsets.iter().copied().min().unwrap_or(doc.source.len()));
        blocks.push(section);
        footnotes = offsets;
    }
    Model {
        blocks,
        starts,
        footnotes,
    }
}

/// Builds one part of `text`: the section under a heading path (`#A#B`) or the block
/// with an id (`#^id`), `None` if absent. A canvas card's `subpath` shows this, built
/// at depth 0 so its markers are the note's own and its embeds nest as the note's do.
pub fn build_section(
    source_path: &str,
    text: &str,
    subpath: &Subpath<'_>,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Option<Model> {
    let doc = parse(text);
    let selected: Vec<Block<'_>> = match subpath {
        Subpath::Headings(names) => section(&doc.blocks, names)?.to_vec(),
        Subpath::Block(id) => vec![block_by_id(&doc.blocks, id)?.clone()],
    };
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth: 0,
        next_id: 0,
        footnotes: FootnoteState::default(),
        line_starts: None,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let (mut blocks, mut starts) = b.blocks_with_starts(&selected);
    let mut footnotes = Vec::new();
    if let Some((section, offsets)) = b.footnote_section(&doc) {
        starts.push(offsets.iter().copied().min().unwrap_or(doc.source.len()));
        blocks.push(section);
        footnotes = offsets;
    }
    Some(Model {
        blocks,
        starts,
        footnotes,
    })
}

/// A top-level block Live Preview renders in place of its source lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    /// Byte span of the block in the note (newline after it excluded).
    pub span: Range<usize>,
    pub block: RBlock,
}

/// The blocks of `doc` that Live Preview shows as widgets, ids unique among them.
pub fn build_widgets(
    source_path: &str,
    doc: &Document<'_>,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Vec<Widget> {
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth: 0,
        next_id: 0,
        footnotes: FootnoteState::default(),
        line_starts: None,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let mut out = Vec::new();
    for block in &doc.blocks {
        let widget = match &block.kind {
            // The properties block is a widget so Live Preview can draw it as an editable table.
            BlockKind::Yaml { .. } => true,
            BlockKind::Table { .. } | BlockKind::Math { .. } => true,
            // A ```mermaid fence is the one code block Live Preview draws rendered
            // (Obsidian's `cm-preview-code-block cm-embed-block cm-lang-mermaid`).
            BlockKind::CodeBlock { info, .. } => {
                tft_mermaid::is_mermaid(info.as_deref().and_then(|i| i.split_whitespace().next()))
            }
            // A callout is the reading view's tinted block; a cursor inside restores source lines.
            BlockKind::Callout { .. } => true,
            // Only raw HTML the renderer can draw earns a widget; anything else would
            // fold its source away to show the same muted text.
            BlockKind::HtmlBlock { content } => {
                html_images(content).is_some() || html_block_renders(content)
            }
            BlockKind::Paragraph { content } => is_lone_embed(content),
            // An image alone in a list item or quote is drawn on the marker's line, beside it.
            BlockKind::List { .. } | BlockKind::BlockQuote { .. } => {
                nested_widgets(&mut b, block, &mut out);
                false
            }
            _ => false,
        };
        if !widget {
            continue;
        }
        if let Some(built) = b.block(block, false) {
            out.push(Widget {
                span: block.span.start..block.span.end,
                block: built,
            });
        }
    }
    out
}

/// A paragraph that is one embed and nothing else, which Live Preview draws as the embed.
fn is_lone_embed(content: &[Inline<'_>]) -> bool {
    match content {
        [one] => match &one.kind {
            InlineKind::Wikilink { embed, .. } => *embed,
            InlineKind::Image {
                kind: LinkKind::Inline,
                ..
            } => true,
            InlineKind::Image {
                kind: LinkKind::Reference,
                destination,
                ..
            } => crate::remote::is_remote(destination),
            _ => false,
        },
        _ => false,
    }
}

/// The widgets inside a list or a quote: paragraphs that are one embed, at any depth.
/// The span starts at the embed, so the marker bytes stay with the editor.
fn nested_widgets(b: &mut Builder<'_>, block: &Block<'_>, out: &mut Vec<Widget>) {
    let children: Vec<&Block<'_>> = match &block.kind {
        BlockKind::List { items, .. } => items.iter().flat_map(|i| i.children.iter()).collect(),
        BlockKind::BlockQuote { children } => children.iter().collect(),
        _ => return,
    };
    for child in children {
        match &child.kind {
            BlockKind::Paragraph { content } if is_lone_embed(content) => {
                if let Some(built) = b.block(child, false) {
                    out.push(Widget {
                        span: child.span.start..child.span.end,
                        block: built,
                    });
                }
            }
            BlockKind::List { .. } | BlockKind::BlockQuote { .. } => nested_widgets(b, child, out),
            _ => {}
        }
    }
}

/// Builds an embedded note or section's blocks at `depth`, sharing `next_id`.
#[allow(clippy::too_many_arguments)]
fn build_embedded(
    source_path: &str,
    doc: &Document<'_>,
    blocks: &[Block<'_>],
    depth: usize,
    next_id: &mut usize,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Vec<RBlock> {
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth,
        next_id: *next_id,
        footnotes: FootnoteState::default(),
        line_starts: None,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let mut out = b.blocks(blocks, false);
    // An embedded section is not jumped into, so its footnote offsets are unused.
    if let Some((section, _)) = b.footnote_section(doc) {
        out.push(section);
    }
    *next_id = b.next_id;
    out
}

#[derive(Default)]
struct FootnoteState {
    /// Labels present in the document (first definition wins).
    definitions: Vec<String>,
    /// Labels in order of first reference; index + 1 is the number.
    numbers: Vec<String>,
    /// Inline footnotes keyed by generated label.
    inline: Vec<(String, RichText)>,
}

fn collect_definitions(blocks: &[Block<'_>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    tft_markdown::walk_blocks(blocks, &mut |b| {
        if let BlockKind::FootnoteDefinition { label, .. } = &b.kind {
            if !out.iter().any(|l| l == label) {
                out.push(label.to_string());
            }
        }
    });
    out
}

struct Builder<'v> {
    vault: &'v dyn EmbedSource,
    source_path: &'v str,
    /// The whole note, so the properties block can carry file byte spans.
    text: &'v str,
    opts: BuildOptions,
    depth: usize,
    next_id: usize,
    footnotes: FootnoteState,
    /// Where each line of `text` starts, built when a task marker first asks.
    line_starts: Option<Vec<usize>>,
}

impl<'v> Builder<'v> {
    /// The line and column of a byte offset in the note.
    fn marker_at(&mut self, offset: usize) -> TaskMarker {
        let text = self.text;
        let starts = self.line_starts.get_or_insert_with(|| {
            std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .collect()
        });
        let line = match starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        TaskMarker {
            line,
            col: offset - starts[line],
        }
    }

    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    fn blocks(&mut self, blocks: &[Block<'_>], tight: bool) -> Vec<RBlock> {
        blocks.iter().filter_map(|b| self.block(b, tight)).collect()
    }

    /// The top level, where each kept block keeps its offset: `block` drops comments
    /// and footnote definitions, so the parsed list's offsets would drift.
    fn blocks_with_starts(&mut self, blocks: &[Block<'_>]) -> (Vec<RBlock>, Vec<usize>) {
        let mut out = Vec::new();
        let mut starts = Vec::new();
        for block in blocks {
            if let Some(built) = self.block(block, false) {
                out.push(built);
                starts.push(block.span.start);
            }
        }
        (out, starts)
    }

    fn block(&mut self, block: &Block<'_>, _tight: bool) -> Option<RBlock> {
        Some(match &block.kind {
            BlockKind::Yaml { .. } => RBlock::Properties(
                tft_cache::note_properties(self.text)
                    .map(|p| p.entries)
                    .unwrap_or_default(),
            ),
            BlockKind::Paragraph { content } => RBlock::Paragraph(self.segments(content)),
            BlockKind::Heading { level, content, .. } => RBlock::Heading {
                level: *level,
                text: self.rich(content),
                plain: plain_text(content).trim().to_string(),
            },
            BlockKind::ThematicBreak => RBlock::ThematicBreak,
            BlockKind::CodeBlock { info, content, .. } => {
                // Obsidian renders and copies with `\n` endings whatever the file
                // holds, and its copy drops exactly one trailing newline.
                let body = if content.contains('\r') {
                    content.replace("\r\n", "\n")
                } else {
                    content.to_string()
                };
                RBlock::Code {
                    id: self.id(),
                    language: info
                        .as_deref()
                        .and_then(|i| i.split_whitespace().next())
                        .map(str::to_string),
                    text: body.trim_end_matches('\n').to_string(),
                    copy: body.strip_suffix('\n').unwrap_or(&body).to_string(),
                    wrap: false,
                    copyable: true,
                }
            }
            // A block of nothing but comments and scripts draws nothing:
            // Obsidian's sanitizer takes a `<script>` out whole and keeps what
            // follows, and no DOM paints a comment ([[Specification#Sanitizer]]).
            BlockKind::HtmlBlock { content } if html_block_is_invisible(content) => return None,
            BlockKind::HtmlBlock { content } => match html_images(content) {
                // Obsidian resolves a raw `<img src>` like a wikilink and renders a
                // run of them as one block, through the same embeds a markdown image uses.
                Some(imgs) => {
                    let mut segments = Vec::with_capacity(imgs.len());
                    for img in imgs {
                        let embed = if crate::remote::is_remote(&img.src) {
                            remote_embed(&img.src, img.alt, img.width, img.height)
                        } else {
                            self.embed(&img.src, img.alt, img.width, img.height)
                        };
                        segments.push(Segment::Embed(embed));
                    }
                    RBlock::Paragraph(segments)
                }
                None => self.html_block(content),
            },
            BlockKind::BlockQuote { children } => RBlock::Quote(self.blocks(children, false)),
            BlockKind::Callout {
                callout_type,
                fold,
                title,
                children,
            } => {
                let title = if title.is_empty() {
                    // Shared with the reading view's HTML and Live Preview, so a
                    // `[!bad-type]` callout reads the same everywhere.
                    RichText {
                        id: self.id(),
                        text: tft_markdown::callout_default_title(callout_type),
                        ..RichText::default()
                    }
                } else {
                    // The title is block-parsed but this view paints one run of rich
                    // text, so flatten its inlines; block styling in a title is dropped.
                    let flat: Vec<Inline<'_>> = tft_markdown::inline_runs(title)
                        .into_iter()
                        .flat_map(|run| run.iter().cloned())
                        .collect();
                    self.rich(&flat)
                };
                RBlock::Callout {
                    kind: canonical_callout_type(callout_type),
                    fold: fold.map(|f| f == Fold::Collapsed),
                    title,
                    children: self.blocks(children, false),
                }
            }
            BlockKind::List { kind, tight, items } => RBlock::List {
                tight: *tight,
                items: self.items(kind, items, *tight),
            },
            BlockKind::Table {
                alignments,
                header,
                rows,
            } => RBlock::Table {
                alignments: alignments.clone(),
                header: header.iter().map(|c| self.rich(&c.content)).collect(),
                rows: rows
                    .iter()
                    .map(|r| r.iter().map(|c| self.rich(&c.content)).collect())
                    .collect(),
                spans: std::iter::once(header)
                    .chain(rows.iter())
                    .map(|r| r.iter().map(|c| c.span.start..c.span.end).collect())
                    .collect(),
                html: None,
            },
            BlockKind::Math { content } => RBlock::Math(content.trim_matches('\n').to_string()),
            BlockKind::Comment { .. } | BlockKind::FootnoteDefinition { .. } => return None,
        })
    }

    fn items(&mut self, kind: &ListKind, items: &[ListItem<'_>], tight: bool) -> Vec<RItem> {
        let mut number = match kind {
            ListKind::Ordered { start, .. } => Some(*start),
            ListKind::Bullet { .. } => None,
        };
        items
            .iter()
            .map(|item| {
                let n = number;
                if let Some(n) = number.as_mut() {
                    *n += 1;
                }
                let marker = item
                    .task_span
                    .filter(|_| self.depth == 0)
                    .map(|s| self.marker_at(s.start));
                RItem {
                    task: item.task,
                    marker,
                    number: n,
                    children: self.blocks(&item.children, tight),
                }
            })
            .collect()
    }

    /// A paragraph's content split into text runs and block-level embeds.
    fn segments(&mut self, content: &[Inline<'_>]) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut acc = RichBuilder::default();
        let is_display_math =
            |inline: &Inline<'_>| matches!(inline.kind, InlineKind::Math { display: true, .. });
        for (ix, inline) in content.iter().enumerate() {
            match &inline.kind {
                // Space on either side of a display block is its margin, not text:
                // an HTML renderer collapses it at the block's edge.
                InlineKind::Text(t)
                    if ix
                        .checked_sub(1)
                        .is_some_and(|p| is_display_math(&content[p]))
                        || content.get(ix + 1).is_some_and(is_display_math) =>
                {
                    let mut t: &str = t;
                    if ix
                        .checked_sub(1)
                        .is_some_and(|p| is_display_math(&content[p]))
                    {
                        t = t.trim_start();
                    }
                    if content.get(ix + 1).is_some_and(is_display_math) {
                        t = t.trim_end();
                    }
                    acc.push(t);
                }
                InlineKind::Wikilink { body, embed: true } => {
                    self.flush(&mut acc, &mut out);
                    let embed = self.wikilink_embed(body);
                    out.push(Segment::Embed(embed));
                }
                // `![alt|200x100](https://...)`: fetched and drawn, the size read off
                // the alt as for a vault image; a reference image works the same way.
                InlineKind::Image {
                    destination,
                    kind: LinkKind::Inline | LinkKind::Reference,
                    children,
                    ..
                } if crate::remote::is_remote(destination) => {
                    self.flush(&mut acc, &mut out);
                    let (alt, width, height) = alias_parts(&plain_text(children));
                    out.push(Segment::Embed(remote_embed(
                        destination,
                        alt.unwrap_or_default(),
                        width,
                        height,
                    )));
                }
                InlineKind::Image {
                    destination,
                    kind: LinkKind::Inline,
                    children,
                    ..
                } if !has_scheme(destination) => {
                    self.flush(&mut acc, &mut out);
                    // `![alt|100](pic.png)` sizes the picture the way
                    // `![[pic.png|100]]` does: the alt is read as an alias.
                    let (alt, width, height) = alias_parts(&plain_text(children));
                    let dest = percent_decode(destination).into_owned();
                    let embed = self.embed(&dest, alt.unwrap_or_default(), width, height);
                    out.push(Segment::Embed(embed));
                }
                InlineKind::Math {
                    content,
                    display: true,
                } => {
                    self.flush(&mut acc, &mut out);
                    out.push(Segment::Math(content.to_string()));
                }
                _ => self.inline(inline, &mut acc),
            }
        }
        self.flush(&mut acc, &mut out);
        out
    }

    fn flush(&mut self, acc: &mut RichBuilder, out: &mut Vec<Segment>) {
        let text = std::mem::take(acc).finish(self.id());
        // Whitespace between two embeds would render as an empty line.
        if !text.text.trim().is_empty() {
            out.push(Segment::Text(text));
        }
    }

    fn rich(&mut self, content: &[Inline<'_>]) -> RichText {
        let mut acc = RichBuilder::default();
        for inline in content {
            self.inline(inline, &mut acc);
        }
        acc.finish(self.id())
    }

    /// Rich text from raw HTML: tags through the inline overlay, text between them
    /// entity-decoded with newlines read as spaces.
    fn rich_html(&mut self, html: &str) -> RichText {
        let id = self.id();
        rich_from_html(html, id)
    }

    /// A raw HTML block that is not a run of images: a `<pre>` as a wrapped code
    /// block, `<details>` as a fold, a container as a paragraph, everything else as
    /// muted source ([[Specification#Observed raw HTML behavior]]).
    fn html_block(&mut self, content: &str) -> RBlock {
        match classify_html_block(content) {
            HtmlBlockKind::Pre { text, code } => RBlock::Code {
                id: self.id(),
                language: None,
                copy: text.clone(),
                text,
                wrap: true,
                copyable: code,
            },
            HtmlBlockKind::Details {
                open,
                summary,
                body,
            } => {
                let summary = self.rich_html(&summary);
                let body = if body.trim().is_empty() {
                    Vec::new()
                } else {
                    let rich = self.rich_html(&body);
                    vec![RBlock::Paragraph(vec![Segment::Text(rich)])]
                };
                RBlock::Details {
                    id: summary.id,
                    open,
                    summary,
                    body,
                }
            }
            HtmlBlockKind::Container(inner) => {
                let rich = self.rich_html(&inner);
                RBlock::Paragraph(vec![Segment::Text(rich)])
            }
            HtmlBlockKind::Table(table) => self.html_table(table),
            HtmlBlockKind::Source => RBlock::Html(content.trim_end().to_string()),
        }
    }

    /// A raw `<table>` drawn like a Markdown table (Obsidian gives both the same
    /// `div.el-table` cells): a body-row `<th>` is bold and ragged rows pad to the
    /// widest; with no cell spans Live Preview never edits it in place.
    fn html_table(&mut self, table: HtmlTable) -> RBlock {
        let places = place_cells(&table);
        let spanned = places
            .iter()
            .flatten()
            .any(|p| p.colspan != 1 || p.rowspan != 1);
        let columns = if spanned {
            0
        } else {
            table
                .header
                .len()
                .max(table.rows.iter().map(Vec::len).max().unwrap_or(0))
        };
        let caption = table.caption.as_ref().map(|(html, _)| self.rich_html(html));
        let caption_bottom = table.caption.as_ref().is_some_and(|(_, b)| *b);
        let mut row_of = |cells: &[HtmlCell]| -> Vec<RichText> {
            let mut row: Vec<RichText> = cells
                .iter()
                .map(|c| {
                    let mut rich = self.rich_html(&c.html);
                    if c.head {
                        let len = rich.text.len();
                        rich.runs = embolden(std::mem::take(&mut rich.runs), len);
                    }
                    rich
                })
                .collect();
            while row.len() < columns {
                let id = self.id();
                row.push(RichText {
                    id,
                    ..RichText::default()
                });
            }
            row
        };
        let header = if table.header.is_empty() {
            Vec::new()
        } else {
            row_of(&table.header)
        };
        let rows: Vec<Vec<RichText>> = table.rows.iter().map(|r| row_of(r)).collect();
        let grid_columns = places
            .iter()
            .flatten()
            .map(|p| p.col + p.colspan)
            .max()
            .unwrap_or(0);
        let mut alignments = table.alignments;
        alignments.resize(columns.max(grid_columns), Alignment::None);
        let html = (spanned || caption.is_some()).then(|| {
            Box::new(HtmlTableExtras {
                caption,
                caption_bottom,
                places: if spanned { places } else { Vec::new() },
                columns: grid_columns,
            })
        });
        RBlock::Table {
            alignments,
            header,
            rows,
            spans: Vec::new(),
            html,
        }
    }

    fn inline(&mut self, inline: &Inline<'_>, acc: &mut RichBuilder) {
        match &inline.kind {
            InlineKind::Text(t) => acc.push(t),
            InlineKind::SoftBreak if self.opts.strict_line_breaks => acc.push(" "),
            InlineKind::SoftBreak | InlineKind::HardBreak => acc.push("\n"),
            InlineKind::Code(c) => acc.styled(|s| s.code = true, |a| a.push(c)),
            InlineKind::Html(h) => acc.html(h),
            InlineKind::Emphasis(c) => self.wrap(acc, c, |s| s.italic = true),
            InlineKind::Strong(c) => self.wrap(acc, c, |s| s.bold = true),
            InlineKind::Strikethrough(c) => self.wrap(acc, c, |s| s.strike = true),
            InlineKind::Highlight { color, children } => {
                let saved = acc.style;
                acc.style.highlight = true;
                acc.style.highlight_color = *color;
                for c in children {
                    self.inline(c, acc);
                }
                acc.style = saved;
            }
            InlineKind::Link {
                destination,
                kind,
                children,
                ..
            } => {
                let internal = matches!(kind, LinkKind::Inline | LinkKind::Reference)
                    && tft_markdown::html::is_vault_destination(destination);
                let target = if internal {
                    LinkTarget::Internal(percent_decode(destination).into_owned())
                } else {
                    LinkTarget::External(destination.to_string())
                };
                let start = acc.len();
                if *kind == LinkKind::Autolink {
                    let shown = plain_text(children);
                    let shown = shown.strip_prefix("mailto:").unwrap_or(&shown).to_string();
                    acc.styled(|s| s.link = true, |a| a.push(&shown));
                    acc.link(start, target);
                } else {
                    // The HTML renderer's rule: Obsidian ends a link at the first child
                    // that is a link of its own, so text after it is not clickable through.
                    let split = children
                        .iter()
                        .position(tft_markdown::html::renders_as_anchor);
                    let head = split.unwrap_or(children.len());
                    self.wrap(acc, &children[..head], |s| s.link = true);
                    acc.link(start, target);
                    if let Some(at) = split {
                        for child in &children[at..] {
                            self.inline(child, acc);
                        }
                    }
                }
            }
            InlineKind::Image {
                destination,
                children,
                ..
            } => {
                // External or reference-style images: a link until remote images load.
                let start = acc.len();
                let alt = plain_text(children);
                let shown = if alt.is_empty() {
                    destination.to_string()
                } else {
                    alt
                };
                acc.styled(|s| s.link = true, |a| a.push(&shown));
                acc.link(start, LinkTarget::External(destination.to_string()));
            }
            // An embed reaching this walk has no block room (a heading, table cell or
            // callout title; `segments()` takes the paragraph case). The link is the
            // clickable stand-in until `RichText` can carry an inline embed.
            InlineKind::Wikilink { body, embed } => {
                let link = tft_markdown::parse_linktext(body);
                let target = tft_markdown::link_target(body).to_string();
                let shown = match link.alias {
                    // `![[pic.png|200]]`: the size hint is not display text.
                    Some(a) if *embed => {
                        alias_parts(a).0.unwrap_or_else(|| wikilink_display(&link))
                    }
                    Some(a) => a.to_string(),
                    None => wikilink_display(&link),
                };
                let start = acc.len();
                acc.styled(|s| s.link = true, |a| a.push(&shown));
                acc.link(start, LinkTarget::Internal(target));
            }
            InlineKind::Tag(name) => {
                let start = acc.len();
                acc.styled(
                    |s| s.tag = true,
                    |a| {
                        a.push("#");
                        a.push(name);
                    },
                );
                acc.link(start, LinkTarget::Tag(name.to_string()));
            }
            InlineKind::Math { content, .. } => {
                acc.styled(|s| s.math = true, |a| a.push(content));
            }
            InlineKind::Comment(_) => {}
            InlineKind::FootnoteReference(label) => {
                if self
                    .footnotes
                    .definitions
                    .iter()
                    .any(|l| l == label.as_ref())
                {
                    let n = self.footnote_number(label);
                    self.footnote_ref(acc, n);
                } else {
                    acc.push(label);
                }
            }
            InlineKind::InlineFootnote(children) => {
                let label = format!("inline-{}", self.footnotes.inline.len() + 1);
                let body = self.rich(children);
                self.footnotes.inline.push((label.clone(), body));
                let n = self.footnote_number(&label);
                self.footnote_ref(acc, n);
            }
        }
    }

    fn wrap(&mut self, acc: &mut RichBuilder, children: &[Inline<'_>], f: fn(&mut InlineStyle)) {
        let saved = acc.style;
        f(&mut acc.style);
        for c in children {
            self.inline(c, acc);
        }
        acc.style = saved;
    }

    fn footnote_number(&mut self, label: &str) -> usize {
        if let Some(i) = self.footnotes.numbers.iter().position(|l| l == label) {
            return i + 1;
        }
        self.footnotes.numbers.push(label.to_string());
        self.footnotes.numbers.len()
    }

    fn footnote_ref(&mut self, acc: &mut RichBuilder, n: usize) {
        let start = acc.len();
        acc.styled(|s| s.footnote = true, |a| a.push(&format!("[{n}]")));
        acc.link(start, LinkTarget::Footnote(n));
    }

    /// The Footnotes section and where each definition is written; a click reveals that offset.
    fn footnote_section(&mut self, doc: &Document<'_>) -> Option<(RBlock, Vec<usize>)> {
        if self.footnotes.numbers.is_empty() {
            return None;
        }
        let labels = self.footnotes.numbers.clone();
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for (i, label) in labels.iter().enumerate() {
            let n = i + 1;
            let mut at = None;
            let body =
                if let Some((_, text)) = self.footnotes.inline.iter().find(|(l, _)| l == label) {
                    vec![RBlock::Paragraph(vec![Segment::Text(text.clone())])]
                } else {
                    let mut found = None;
                    tft_markdown::walk_blocks(&doc.blocks, &mut |b| {
                        if found.is_none() {
                            if let BlockKind::FootnoteDefinition { label: l, children } = &b.kind {
                                if l == label {
                                    found = Some(children);
                                    at = Some(b.span.start);
                                }
                            }
                        }
                    });
                    match found {
                        Some(children) => self.blocks(children, false),
                        None => Vec::new(),
                    }
                };
            // An inline footnote is written where referenced; an undefined label points nowhere else.
            offsets.push(at.unwrap_or(0));
            out.push((n, body));
        }
        Some((RBlock::Footnotes(out), offsets))
    }

    // ----- embeds --------------------------------------------------------------

    fn wikilink_embed(&mut self, body: &str) -> Embed {
        let link = tft_markdown::parse_linktext(body);
        let target = tft_markdown::link_target(body);
        let (alt, width, height) = alias_parts(link.alias.unwrap_or(""));
        let alt = alt.unwrap_or_else(|| wikilink_display(&link));
        self.embed(target, alt, width, height)
    }

    fn embed(&mut self, link: &str, alt: String, width: Option<f32>, height: Option<f32>) -> Embed {
        let parsed = tft_markdown::parse_linktext(link);
        let resolved = self.vault.resolve(link, self.source_path);
        let kind = match resolved {
            None => EmbedKind::Missing,
            Some(path) => match FileKind::from_path(&path) {
                FileKind::Image => EmbedKind::Image(self.vault.absolute(&path)),
                FileKind::Markdown => {
                    if self.depth + 1 >= MAX_EMBED_DEPTH {
                        EmbedKind::TooDeep
                    } else {
                        self.note_embed(&path, parsed.subpath.as_ref(), &wikilink_display(&parsed))
                    }
                }
                // A PDF is resolved only; the document cache parses it off the UI thread.
                // The subpath (`#page=3&height=400`) is the link's own, read from the raw text.
                FileKind::Pdf => {
                    let id = self.next_id;
                    self.next_id += 1;
                    EmbedKind::Pdf(crate::pdf::embed::PdfEmbed::new(
                        id,
                        path.clone(),
                        self.vault.absolute(&path),
                        link,
                        height,
                    ))
                }
                // A canvas is its minimap: read and reduced here from the file's cards
                // and edges, as a note embed is read; it nests no further.
                FileKind::Canvas => EmbedKind::Canvas(crate::canvas::minimap::CanvasEmbed::new(
                    &path,
                    self.vault.read(&path).as_deref(),
                )),
                FileKind::Audio | FileKind::Video => {
                    let id = self.next_id;
                    self.next_id += 1;
                    let media = crate::media::MediaEmbed {
                        id,
                        path: path.clone(),
                        absolute: self.vault.absolute(&path),
                    };
                    if FileKind::from_path(&path) == FileKind::Audio {
                        EmbedKind::Audio(media)
                    } else {
                        EmbedKind::Video(media)
                    }
                }
                other => EmbedKind::Unsupported(other),
            },
        };
        Embed {
            kind,
            link: link.to_string(),
            alt,
            width,
            height,
        }
    }

    fn note_embed(&mut self, path: &str, subpath: Option<&Subpath<'_>>, title: &str) -> EmbedKind {
        let Some(text) = self.vault.read(path) else {
            return EmbedKind::Missing;
        };
        let doc = parse(&text);
        let selected: Vec<Block<'_>> = match subpath {
            None => doc
                .blocks
                .iter()
                .filter(|b| !matches!(b.kind, BlockKind::Yaml { .. }))
                .cloned()
                .collect(),
            Some(Subpath::Headings(names)) => match section(&doc.blocks, names) {
                Some(s) => s.to_vec(),
                None => return EmbedKind::Missing,
            },
            Some(Subpath::Block(id)) => match block_by_id(&doc.blocks, id) {
                Some(b) => vec![b.clone()],
                None => return EmbedKind::Missing,
            },
        };
        let mut next_id = self.next_id;
        let blocks = build_embedded(
            path,
            &doc,
            &selected,
            self.depth + 1,
            &mut next_id,
            self.vault,
            self.opts,
        );
        self.next_id = next_id;
        let title = if title.is_empty() {
            path.trim_end_matches(".md").to_string()
        } else {
            title.to_string()
        };
        EmbedKind::Note {
            path: path.to_string(),
            title,
            blocks,
        }
    }
}

/// Rich text from raw HTML: tags through the inline overlay, text between them
/// entity-decoded; Markdown between the tags stays literal.
fn rich_from_html(html: &str, id: usize) -> RichText {
    let mut acc = RichBuilder::default();
    let mut rest = html.trim();
    while !rest.is_empty() {
        match rest.find('<') {
            Some(0) => {
                let end = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
                acc.html(&rest[..end]);
                rest = &rest[end..];
            }
            Some(at) => {
                acc.push(&html_text(&rest[..at]));
                rest = &rest[at..];
            }
            None => {
                acc.push(&html_text(rest));
                rest = "";
            }
        }
    }
    acc.finish(id)
}

/// What Live Preview draws for one inline HTML element while no cursor touches it:
/// the element rendered with its content literal (`<b>**md**</b>` shows `**md**`
/// bold), where the reading view parses it ([[Specification#Inline HTML in Live
/// Preview]]). `None` keeps the source: nothing rendered, un-drawable, or a line break.
pub fn html_inline_widget(element: &str) -> Option<RichText> {
    let rich = rich_from_html(element, 0);
    let drawable = !rich.text.trim().is_empty()
        && !rich.text.contains('\n')
        && !rich.runs.iter().any(|(_, s)| s.html);
    drawable.then_some(rich)
}

/// Accumulates text with a style stack into one [`RichText`].
#[derive(Default)]
struct RichBuilder {
    text: String,
    /// The style the Markdown nesting gives the next text; `wrap`/`styled` save and restore it.
    style: InlineStyle,
    /// The raw HTML tags open here, innermost last; kept apart from `style` because a
    /// tag's extent is not a Markdown node's (`*a <b>b* c</b>`), so neither restores.
    html: Vec<HtmlOpen>,
    runs: Vec<(Range<usize>, InlineStyle)>,
    links: Vec<(Range<usize>, LinkTarget)>,
}

struct HtmlOpen {
    name: HtmlTag,
    /// What the tag's `style` attribute (or `<font color>`) lays over it.
    css: InlineCss,
    /// For `<a href>`: where its text started and what it points at, recorded on close.
    link: Option<(usize, LinkTarget)>,
}

/// The inline HTML tags mapped onto the model's own styles, the part of the spec's
/// raw-HTML subset a flat `RichText` can carry; anything else stays muted source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HtmlTag {
    Bold,
    Italic,
    Strike,
    Highlight,
    Underline,
    Kbd,
    Sup,
    Sub,
    Small,
    Big,
    /// `<code>` written as HTML: inline code's look (probe of 2026-11-15).
    Code,
    Anchor,
    /// `<span>`, `<div>`, `<font>`, ...: dropped, content kept.
    Transparent,
}

impl HtmlTag {
    fn from_name(name: &str) -> Option<HtmlTag> {
        Some(match name.to_ascii_lowercase().as_str() {
            "b" | "strong" => HtmlTag::Bold,
            "i" | "em" => HtmlTag::Italic,
            "s" | "del" | "strike" => HtmlTag::Strike,
            "mark" => HtmlTag::Highlight,
            "u" => HtmlTag::Underline,
            "kbd" => HtmlTag::Kbd,
            "sup" => HtmlTag::Sup,
            "sub" => HtmlTag::Sub,
            "small" => HtmlTag::Small,
            "big" => HtmlTag::Big,
            "code" => HtmlTag::Code,
            "a" => HtmlTag::Anchor,
            "span" | "div" | "font" | "center" | "p" | "section" | "article" | "ins" | "samp"
            | "var" | "tt" => HtmlTag::Transparent,
            _ => return None,
        })
    }

    fn apply(self, s: &mut InlineStyle) {
        match self {
            HtmlTag::Bold => s.bold = true,
            HtmlTag::Italic => s.italic = true,
            HtmlTag::Strike => s.strike = true,
            HtmlTag::Highlight => s.highlight = true,
            HtmlTag::Underline => s.underline = true,
            HtmlTag::Kbd => s.kbd = true,
            HtmlTag::Sup => s.sup = true,
            HtmlTag::Sub => s.sub = true,
            HtmlTag::Small => s.small = true,
            HtmlTag::Big => s.big = true,
            HtmlTag::Code => s.code = true,
            // The link style belongs to an anchor with somewhere to go, so `open_tag` sets it.
            HtmlTag::Anchor | HtmlTag::Transparent => {}
        }
    }
}

/// The part of an inline `style` the reading model draws: text colour, background,
/// weight, slant and `text-decoration`. A size, font, box, `display` or `var(--…)`
/// colour is dropped; `<font color>` is the same colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct InlineCss {
    color: Option<u32>,
    background: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
}

impl InlineCss {
    fn from_attrs(attrs: &str, font: bool) -> InlineCss {
        let mut css = InlineCss::default();
        let Some(attrs) = html_attrs(attrs) else {
            return css;
        };
        for (name, value) in attrs {
            match name.as_str() {
                "style" => css.declarations(&value),
                "color" if font => {
                    if let Some(c) = css_color(&value) {
                        css.color = Some(c);
                    }
                }
                _ => {}
            }
        }
        css
    }

    /// `prop: value; …`, later declarations winning, as in CSS.
    fn declarations(&mut self, style: &str) {
        for decl in style.split(';') {
            let Some((prop, value)) = decl.split_once(':') else {
                continue;
            };
            let prop = prop.trim().to_ascii_lowercase();
            let value = value
                .trim()
                .trim_end_matches("!important")
                .trim()
                .to_ascii_lowercase();
            match prop.as_str() {
                "color" => {
                    if let Some(c) = css_color(&value) {
                        self.color = Some(c);
                    }
                }
                // The shorthand's colour, when it is the whole value.
                "background-color" | "background" => {
                    if let Some(c) = css_color(&value) {
                        self.background = Some(c);
                    }
                }
                "font-weight" => {
                    self.bold = match value.as_str() {
                        "bold" | "bolder" => Some(true),
                        "normal" | "lighter" => Some(false),
                        n => n.parse::<u32>().ok().map(|w| w >= 600),
                    }
                    .or(self.bold);
                }
                "font-style" => {
                    self.italic = match value.as_str() {
                        "italic" | "oblique" => Some(true),
                        "normal" => Some(false),
                        _ => self.italic,
                    };
                }
                "text-decoration" | "text-decoration-line" => {
                    let words: Vec<&str> = value.split_whitespace().collect();
                    if words.contains(&"none") {
                        self.underline = Some(false);
                        self.strike = Some(false);
                    } else {
                        if words.contains(&"underline") {
                            self.underline = Some(true);
                        }
                        if words.contains(&"line-through") {
                            self.strike = Some(true);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn apply(&self, s: &mut InlineStyle) {
        if let Some(c) = self.color {
            s.color = Some(c);
        }
        if let Some(c) = self.background {
            s.background = Some(c);
        }
        if let Some(b) = self.bold {
            s.bold = b;
        }
        if let Some(i) = self.italic {
            s.italic = i;
        }
        if let Some(u) = self.underline {
            s.underline = u;
        }
        if let Some(x) = self.strike {
            s.strike = x;
        }
    }
}

/// A CSS colour as `0xRRGGBBAA` (hex, named, `rgb()`, `hsl()`) through the theme
/// loader's grammar; `None` for `var()`, `currentcolor` and the like.
fn css_color(value: &str) -> Option<u32> {
    crate::theme::color::parse_color(value)
}

/// One inline HTML token: a whole tag, or a comment, declaration or CDATA section.
enum HtmlToken<'a> {
    Open {
        name: &'a str,
        attrs: &'a str,
    },
    Close(&'a str),
    /// `<!-- -->`, `<!DOCTYPE>`, `<![CDATA[ ]]>`, `<?php ?>`: nothing shown.
    Comment,
}

fn html_token(raw: &str) -> Option<HtmlToken<'_>> {
    let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
    if inner.starts_with(['!', '?']) {
        return Some(HtmlToken::Comment);
    }
    let (closing, rest) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let name_len = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric())
        .count();
    if name_len == 0 {
        return None;
    }
    let (name, attrs) = rest.split_at(name_len);
    Some(if closing {
        HtmlToken::Close(name)
    } else {
        HtmlToken::Open {
            name,
            attrs: attrs.trim().trim_end_matches('/'),
        }
    })
}

impl RichBuilder {
    fn len(&self) -> usize {
        self.text.len()
    }

    /// The style the next text takes: the Markdown style overlaid with every open HTML tag.
    fn effective(&self) -> InlineStyle {
        let mut s = self.style;
        for open in &self.html {
            open.name.apply(&mut s);
            open.css.apply(&mut s);
            if open.link.is_some() {
                s.link = true;
            }
        }
        s
    }

    fn push(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let end = self.text.len();
        let style = self.effective();
        if style.is_plain() {
            return;
        }
        if let Some((range, last)) = self.runs.last_mut() {
            if *last == style && range.end == start {
                range.end = end;
                return;
            }
        }
        self.runs.push((start..end, style));
    }

    /// One inline HTML token: understood tags become style, a break or a link; a
    /// comment vanishes; a closer with no opener is ignored; anything else stays
    /// muted source.
    fn html(&mut self, raw: &str) {
        match html_token(raw) {
            Some(HtmlToken::Comment) => {}
            Some(HtmlToken::Open { name, .. }) if name.eq_ignore_ascii_case("br") => {
                self.push("\n")
            }
            Some(HtmlToken::Open { name, attrs }) => match HtmlTag::from_name(name) {
                Some(tag) => self.open_tag(tag, attrs, name.eq_ignore_ascii_case("font")),
                None => self.styled(|s| s.html = true, |a| a.push(raw)),
            },
            Some(HtmlToken::Close(name)) => match HtmlTag::from_name(name) {
                Some(tag) => self.close_tag(tag),
                None => self.styled(|s| s.html = true, |a| a.push(raw)),
            },
            None => self.styled(|s| s.html = true, |a| a.push(raw)),
        }
    }

    fn open_tag(&mut self, name: HtmlTag, attrs: &str, font: bool) {
        let mut link = None;
        if name == HtmlTag::Anchor {
            // Obsidian keeps a raw anchor's href as written (no vault resolution, so
            // `Note.md` is external) and drops a `javascript:` one, leaving plain text.
            let href = html_attrs(attrs)
                .into_iter()
                .flatten()
                .find(|(n, _)| n == "href")
                .map(|(_, v)| v)
                .filter(|v| {
                    !v.is_empty()
                        && !v
                            .trim_start()
                            .to_ascii_lowercase()
                            .starts_with("javascript:")
                });
            link = href.map(|href| (self.text.len(), LinkTarget::External(href)));
        }
        let css = InlineCss::from_attrs(attrs, font);
        self.html.push(HtmlOpen { name, css, link });
    }

    /// Closes the innermost open tag of this name and anything opened inside it.
    fn close_tag(&mut self, name: HtmlTag) {
        let Some(at) = self.html.iter().rposition(|o| o.name == name) else {
            return;
        };
        let closed: Vec<HtmlOpen> = self.html.drain(at..).collect();
        for open in closed {
            if let Some((start, target)) = open.link {
                self.link(start, target);
            }
        }
    }

    /// The end of the text closes every tag still open, as Obsidian does.
    fn close_all(&mut self) {
        let open: Vec<HtmlOpen> = std::mem::take(&mut self.html);
        for open in open {
            if let Some((start, target)) = open.link {
                self.link(start, target);
            }
        }
    }

    fn styled(&mut self, f: impl FnOnce(&mut InlineStyle), body: impl FnOnce(&mut RichBuilder)) {
        let saved = self.style;
        f(&mut self.style);
        body(self);
        self.style = saved;
    }

    fn link(&mut self, start: usize, target: LinkTarget) {
        let end = self.text.len();
        if end > start {
            self.links.push((start..end, target));
        }
    }

    fn finish(mut self, id: usize) -> RichText {
        self.close_all();
        RichText {
            id,
            text: self.text,
            runs: self.runs,
            links: self.links,
        }
    }
}

/// A remote image's embed: the URL as written, nothing resolved.
fn remote_embed(url: &str, alt: String, width: Option<f32>, height: Option<f32>) -> Embed {
    let url = url.trim();
    Embed {
        kind: EmbedKind::Remote(url.to_string()),
        link: url.to_string(),
        alt,
        width,
        height,
    }
}

/// The `|`-separated parts of an embed alias (a wikilink's alias or a markdown
/// image's alt): each `W`/`WxH` part claims the size (last wins), the rest are the
/// alt, `|`-joined.
fn alias_parts(alias: &str) -> (Option<String>, Option<f32>, Option<f32>) {
    let mut width = None;
    let mut height = None;
    let mut alt: Vec<&str> = Vec::new();
    for part in alias.split('|').map(str::trim) {
        match size_hint(part) {
            Some((w, h)) => {
                width = Some(w);
                height = h;
            }
            None if !part.is_empty() => alt.push(part),
            None => {}
        }
    }
    let alt = (!alt.is_empty()).then(|| alt.join("|"));
    (alt, width, height)
}

/// `200` or `200x100` → `(width, Some(height))`.
fn size_hint(s: &str) -> Option<(f32, Option<f32>)> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'x') {
        return None;
    }
    let mut parts = s.split('x');
    let w: f32 = parts.next()?.parse().ok()?;
    let h = match parts.next() {
        Some(h) => Some(h.parse().ok()?),
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((w, h))
}

/// Whether an HTML block paints nothing: comments and `<script>` elements plus
/// their whitespace. An unterminated one swallows the rest of the block.
/// What a raw HTML block draws as.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HtmlBlockKind {
    /// `<pre>…</pre>`: the literal text and whether a `<code>` child earns the copy button.
    Pre { text: String, code: bool },
    /// `<details [open]>`: the summary's HTML, the rest's HTML.
    Details {
        open: bool,
        summary: String,
        body: String,
    },
    /// A container the sanitizer keeps with its content: the inner HTML.
    Container(String),
    /// `<table>`: its cells, read as far as a flat grid can carry them.
    Table(HtmlTable),
    /// Anything else, drawn as source.
    Source,
}

/// A raw `<table>` as rows of cells' inner HTML.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct HtmlTable {
    /// The `<thead>` row, or a first row of nothing but `<th>`; empty when neither.
    header: Vec<HtmlCell>,
    rows: Vec<Vec<HtmlCell>>,
    /// Per column, from the `align` attribute of the first row's cells.
    alignments: Vec<Alignment>,
    /// The `<caption>`'s inner HTML, and whether its `style` puts it below.
    caption: Option<(String, bool)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HtmlCell {
    html: String,
    /// A `<th>` outside the header row, drawn bold.
    head: bool,
    /// `colspan` and `rowspan` as written (1 when absent or unreadable; 0 is kept).
    colspan: usize,
    rowspan: usize,
    align: Alignment,
}

/// Whether a raw HTML block renders as a Live Preview widget: everything the
/// reading view renders but a `<table>` (left as highlighted source), where
/// `<div>`, `<pre>`, `<details>`, `<center>` and `<p>` become `cm-html-embed`.
fn html_block_renders(content: &str) -> bool {
    !matches!(
        classify_html_block(content),
        HtmlBlockKind::Source | HtmlBlockKind::Table(_)
    )
}

/// `runs` over `len` bytes with bold laid over every byte, gaps included.
fn embolden(
    runs: Vec<(Range<usize>, InlineStyle)>,
    len: usize,
) -> Vec<(Range<usize>, InlineStyle)> {
    let bold = InlineStyle {
        bold: true,
        ..InlineStyle::default()
    };
    let mut out = Vec::new();
    let mut at = 0;
    for (range, mut style) in runs {
        if range.start > at {
            out.push((at..range.start, bold));
        }
        style.bold = true;
        at = range.end;
        out.push((range, style));
    }
    if at < len {
        out.push((at..len, bold));
    }
    out
}

/// Where each cell of `table` sits, header first: a cell takes the first column its
/// row has free; `rowspan="0"` and a past-end span stop at the section's end, and a
/// `colspan` stops at the table's last column.
fn place_cells(table: &HtmlTable) -> Vec<Vec<CellPlace>> {
    let mut sections: Vec<(usize, usize)> = Vec::new();
    let head = usize::from(!table.header.is_empty());
    if head == 1 {
        sections.push((0, 1));
    }
    sections.push((head, head + table.rows.len()));
    let all: Vec<&Vec<HtmlCell>> = (!table.header.is_empty())
        .then_some(&table.header)
        .into_iter()
        .chain(table.rows.iter())
        .collect();
    // `taken[r]` holds the columns row `r` has lost to a span from above.
    let mut taken: Vec<Vec<bool>> = vec![Vec::new(); all.len()];
    let mut out: Vec<Vec<CellPlace>> = Vec::with_capacity(all.len());
    for (r, cells) in all.iter().enumerate() {
        let end = sections
            .iter()
            .find(|(a, b)| (*a..*b).contains(&r))
            .map(|(_, b)| *b)
            .unwrap_or(all.len());
        let mut col = 0;
        let mut row = Vec::with_capacity(cells.len());
        for cell in cells.iter() {
            while taken[r].get(col).copied().unwrap_or(false) {
                col += 1;
            }
            let colspan = cell.colspan.max(1);
            let rowspan = match cell.rowspan {
                0 => end - r,
                n => n.min(end - r),
            };
            for line in taken.iter_mut().skip(r).take(rowspan) {
                if line.len() < col + colspan {
                    line.resize(col + colspan, false);
                }
                for slot in &mut line[col..col + colspan] {
                    *slot = true;
                }
            }
            row.push(CellPlace {
                col,
                colspan,
                rowspan,
                align: cell.align,
            });
            col += colspan;
        }
        out.push(row);
    }
    let columns = out.iter().flatten().map(|p| p.col + 1).max().unwrap_or(0);
    for p in out.iter_mut().flatten() {
        p.colspan = p.colspan.min(columns - p.col);
    }
    out
}

/// The cells of a raw `<table>` block, or `None` for one this grid cannot hold.
fn parse_html_table(html: &str) -> Option<HtmlTable> {
    let mut table = HtmlTable::default();
    let mut row: Option<(Vec<HtmlCell>, bool)> = None;
    let mut cell: Option<HtmlCell> = None;
    let mut in_head = false;
    let mut in_caption = false;
    let mut first_alignments: Option<Vec<Alignment>> = None;
    let mut depth = 0usize;
    let finish_cell = |cell: &mut Option<HtmlCell>,
                       row: &mut Option<(Vec<HtmlCell>, bool)>,
                       aligns: &mut Vec<Alignment>| {
        if let Some(c) = cell.take() {
            let (cells, _) = row.get_or_insert_with(|| (Vec::new(), false));
            aligns.push(c.align);
            cells.push(c);
        }
    };
    let mut row_aligns: Vec<Alignment> = Vec::new();
    let mut finish_row = |row: &mut Option<(Vec<HtmlCell>, bool)>,
                          row_aligns: &mut Vec<Alignment>,
                          table: &mut HtmlTable| {
        let aligns = std::mem::take(row_aligns);
        let Some((cells, head_section)) = row.take() else {
            return;
        };
        if cells.is_empty() {
            return;
        }
        if first_alignments.is_none() {
            first_alignments = Some(aligns);
        }
        let all_th = cells.iter().all(|c| c.head);
        if table.header.is_empty() && table.rows.is_empty() && (head_section || all_th) {
            table.header = cells
                .into_iter()
                .map(|c| HtmlCell { head: false, ..c })
                .collect();
        } else {
            table.rows.push(cells);
        }
    };
    let mut rest = html.trim();
    while !rest.is_empty() {
        let (token, text) = match rest.find('<') {
            Some(0) => {
                let end = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
                let token = &rest[..end];
                rest = &rest[end..];
                (Some(token), "")
            }
            Some(at) => {
                let text = &rest[..at];
                rest = &rest[at..];
                (None, text)
            }
            None => {
                let text = rest;
                rest = "";
                (None, text)
            }
        };
        let Some(token) = token else {
            if in_caption {
                if let Some((inner, _)) = table.caption.as_mut() {
                    inner.push_str(text);
                }
            } else if let Some(c) = cell.as_mut() {
                c.html.push_str(text);
            }
            continue;
        };
        let (name, closing, attrs) = match html_token(token) {
            Some(HtmlToken::Open { name, attrs }) => (name.to_ascii_lowercase(), false, attrs),
            Some(HtmlToken::Close(name)) => (name.to_ascii_lowercase(), true, ""),
            _ => (String::new(), false, ""),
        };
        if in_caption && !(name == "caption" && closing) && name != "table" {
            if let Some((inner, _)) = table.caption.as_mut() {
                inner.push_str(token);
            }
            continue;
        }
        match (name.as_str(), closing) {
            ("caption", false) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut table);
                in_caption = true;
                // Only the first caption counts, as in a browser.
                if table.caption.is_none() {
                    let bottom = html_attrs(attrs)
                        .into_iter()
                        .flatten()
                        .find(|(n, _)| n == "style")
                        .is_some_and(|(_, v)| {
                            v.split(';').any(|d| {
                                d.split_once(':').is_some_and(|(p, v)| {
                                    p.trim().eq_ignore_ascii_case("caption-side")
                                        && v.trim().eq_ignore_ascii_case("bottom")
                                })
                            })
                        });
                    table.caption = Some((String::new(), bottom));
                }
            }
            ("caption", true) => in_caption = false,
            ("table", false) => {
                if depth > 0 {
                    return None;
                }
                depth += 1;
            }
            ("table", true) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut table);
                break;
            }
            ("thead", open_or_close) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut table);
                in_head = !open_or_close;
            }
            ("tbody" | "tfoot", _) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut table);
                in_head = false;
            }
            ("tr", false) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut table);
                row = Some((Vec::new(), in_head));
            }
            ("tr", true) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut table);
            }
            ("td" | "th", false) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                if row.is_none() {
                    row = Some((Vec::new(), in_head));
                }
                let attrs = html_attrs(attrs).unwrap_or_default();
                let attr = |key: &str| attrs.iter().find(|(n, _)| n == key).map(|(_, v)| v);
                let align = attr("align")
                    .map(|v| match v.to_ascii_lowercase().as_str() {
                        "left" => Alignment::Left,
                        "center" => Alignment::Center,
                        "right" => Alignment::Right,
                        _ => Alignment::None,
                    })
                    .unwrap_or(Alignment::None);
                // A non-numeric span is one; HTML caps them at 1000 and 65534.
                let span = |key: &str, cap: usize| -> usize {
                    attr(key)
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(cap)
                };
                cell = Some(HtmlCell {
                    html: String::new(),
                    head: name == "th",
                    colspan: span("colspan", 1000),
                    rowspan: span("rowspan", 65534),
                    align,
                });
            }
            ("td" | "th", true) => finish_cell(&mut cell, &mut row, &mut row_aligns),
            _ => {
                if let Some(c) = cell.as_mut() {
                    c.html.push_str(token);
                }
            }
        }
    }
    finish_cell(&mut cell, &mut row, &mut row_aligns);
    finish_row(&mut row, &mut row_aligns, &mut table);
    if table.header.is_empty() && table.rows.is_empty() {
        return None;
    }
    table.alignments = first_alignments.unwrap_or_default();
    Some(table)
}

fn classify_html_block(content: &str) -> HtmlBlockKind {
    // A comment before the first tag says nothing; the inline overlay drops it.
    let mut trimmed = content.trim();
    while let Some(after) = trimmed.strip_prefix("<!--") {
        trimmed = match after.find("-->") {
            Some(end) => after[end + 3..].trim_start(),
            None => return HtmlBlockKind::Source,
        };
    }
    let Some(end) = trimmed.find('>') else {
        return HtmlBlockKind::Source;
    };
    let Some(HtmlToken::Open { name, attrs }) = html_token(&trimmed[..=end]) else {
        return HtmlBlockKind::Source;
    };
    let name = name.to_ascii_lowercase();
    let after = &trimmed[end + 1..];
    let inner = |closing: &str| -> &str {
        match find_ignore_ascii_case(after, closing) {
            Some(at) => &after[..at],
            None => after,
        }
    };
    match name.as_str() {
        "pre" => {
            let mut body = inner("</pre").trim_matches('\n').to_string();
            let mut code = false;
            if let Some(stripped) = strip_tag(&body, "code") {
                body = stripped;
                code = true;
            }
            HtmlBlockKind::Pre {
                text: decode_entities(&strip_tags(&body))
                    .trim_end_matches('\n')
                    .to_string(),
                code,
            }
        }
        "details" => {
            let open = attrs.split_whitespace().any(|a| {
                a.eq_ignore_ascii_case("open") || a.to_ascii_lowercase().starts_with("open=")
            });
            let inner = inner("</details").trim();
            let (summary, body) = match find_ignore_ascii_case(inner, "<summary") {
                Some(at) => {
                    let after_tag = inner[at..]
                        .find('>')
                        .map(|i| at + i + 1)
                        .unwrap_or(inner.len());
                    let rest = &inner[after_tag..];
                    match find_ignore_ascii_case(rest, "</summary") {
                        Some(close) => {
                            let tail = rest[close..]
                                .find('>')
                                .map(|i| close + i + 1)
                                .unwrap_or(rest.len());
                            (rest[..close].to_string(), rest[tail..].to_string())
                        }
                        None => (rest.to_string(), String::new()),
                    }
                }
                None => (String::from("Details"), inner.to_string()),
            };
            HtmlBlockKind::Details {
                open,
                summary,
                body,
            }
        }
        "div" | "p" | "center" | "span" | "font" | "small" | "big" | "section" | "article"
        | "ins" | "samp" | "var" | "tt" | "kbd" | "sup" | "sub" | "b" | "strong" | "i" | "em"
        | "u" | "mark" | "s" | "del" | "strike" | "a" => {
            HtmlBlockKind::Container(trimmed.to_string())
        }
        "table" => match parse_html_table(trimmed) {
            Some(table) => HtmlBlockKind::Table(table),
            None => HtmlBlockKind::Source,
        },
        _ => HtmlBlockKind::Source,
    }
}

/// `body` with one outer `<tag …>…</tag>` pair removed, when it starts with the opener.
fn strip_tag(body: &str, tag: &str) -> Option<String> {
    let trimmed = body.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with(&format!("<{tag}")) {
        return None;
    }
    let open_end = trimmed.find('>')? + 1;
    let inner = &trimmed[open_end..];
    let close = format!("</{tag}");
    let cut = match find_ignore_ascii_case(inner, &close) {
        Some(at) => &inner[..at],
        None => inner,
    };
    Some(cut.trim_start_matches('\n').to_string())
}

/// Every tag removed, its content kept: what a `<pre>` shows for the markup inside it.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        match rest[at..].find('>') {
            Some(end) => rest = &rest[at + end + 1..],
            None => {
                out.push_str(&rest[at..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text between tags: entities decoded, and the newlines HTML reads as spaces read as spaces.
fn html_text(raw: &str) -> String {
    let decoded = decode_entities(raw);
    // HTML collapses whitespace to one space and keeps a chunk's edge: `The ` stays `The `.
    let mut out = String::with_capacity(decoded.len());
    let mut in_space = false;
    for ch in decoded.chars() {
        if ch.is_whitespace() && ch != '\u{a0}' {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out
}

/// The entities a hand-written note uses, decoded; everything else as it was, newlines included.
fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let entity = tail.find(';').map(|i| &tail[..=i]);
        let decoded = match entity {
            Some("&amp;") => Some('&'),
            Some("&lt;") => Some('<'),
            Some("&gt;") => Some('>'),
            Some("&quot;") => Some('"'),
            Some("&apos;") | Some("&#39;") => Some('\''),
            Some("&nbsp;") => Some('\u{a0}'),
            Some(e) if e.starts_with("&#") => {
                let num = &e[2..e.len() - 1];
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse::<u32>().ok(),
                };
                code.and_then(char::from_u32)
            }
            _ => None,
        };
        match (decoded, entity) {
            (Some(ch), Some(e)) => {
                out.push(ch);
                rest = &tail[e.len()..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn html_block_is_invisible(content: &str) -> bool {
    let mut rest = content.trim();
    let mut seen = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = match after.find("-->") {
                Some(end) => &after[end + 3..],
                // An unclosed comment runs to the end of the block.
                None => "",
            };
        } else if let Some(after) = script_open(rest) {
            rest = match find_ignore_ascii_case(after, "</script") {
                // Past the closing tag's `>`, which may carry whitespace before it (`</script >`).
                Some(end) => match after[end..].find('>') {
                    Some(gt) => &after[end + gt + 1..],
                    None => "",
                },
                None => "",
            };
        } else {
            return false;
        }
        seen = true;
        rest = rest.trim_start();
    }
    seen
}

/// The text after `<script ...>` when `rest` opens a script element.
fn script_open(rest: &str) -> Option<&str> {
    let after = rest
        .get(..7)
        .filter(|t| t.eq_ignore_ascii_case("<script"))?;
    let after = &rest[after.len()..];
    if !after.starts_with([' ', '\t', '\n', '\r', '/', '>']) {
        return None;
    }
    // Past the open tag, whose attributes may hold a `>` only inside quotes.
    split_tag(after).map(|(_, tail)| tail)
}

/// `haystack.find(needle)` ignoring ASCII case, for a lower-case `needle`.
fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// A raw `<img>` of an HTML block, with the attributes Obsidian honours.
struct HtmlImage {
    src: String,
    alt: String,
    width: Option<f32>,
    height: Option<f32>,
}

/// The images of a block holding nothing but `<img>` tags, the shape Obsidian renders
/// as one `div.el-img`; any other content, or an out-of-vault image, gives `None`.
fn html_images(content: &str) -> Option<Vec<HtmlImage>> {
    let mut out = Vec::new();
    let mut rest = content.trim();
    while !rest.is_empty() {
        let after = match rest.get(..4) {
            // Only `<img` itself, not `<image>` or some longer tag name.
            Some(tag) if tag.eq_ignore_ascii_case("<img") => &rest[4..],
            _ => return None,
        };
        if !after.starts_with([' ', '\t', '\n', '\r', '/', '>']) {
            return None;
        }
        let (attrs, tail) = split_tag(after)?;
        out.push(html_image(attrs)?);
        rest = tail.trim_start();
    }
    (!out.is_empty()).then_some(out)
}

/// Splits a tag's attributes at the `>` that closes it, skipping any inside a quoted value.
fn split_tag(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let mut quote = None;
    for (i, &c) in b.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return Some((&s[..i], &s[i + 1..])),
            None => {}
        }
    }
    None
}

/// One `<img>`, from the text between `<img` and its `>`.
fn html_image(attrs: &str) -> Option<HtmlImage> {
    let mut src = None;
    let mut alt = String::new();
    let mut width = None;
    let mut height = None;
    for (name, value) in html_attrs(attrs.trim_end().trim_end_matches('/'))? {
        match name.as_str() {
            "src" => src = Some(value),
            "alt" => alt = value,
            // A raw dimension is CSS pixels; a `%` or `calc()` keeps the block as source.
            "width" => width = Some(value.parse().ok()?),
            "height" => height = Some(value.parse().ok()?),
            _ => {}
        }
    }
    let src = src?;
    // A remote image is fetched; any other scheme leaves the block as source.
    if src.is_empty() || (has_scheme(&src) && !crate::remote::is_remote(&src)) {
        return None;
    }
    // A URL is requested as written; a vault path is a file name.
    let src = if crate::remote::is_remote(&src) {
        src
    } else {
        percent_decode(&src).into_owned()
    };
    Some(HtmlImage {
        src,
        alt,
        width,
        height,
    })
}

/// A tag's attributes as lowercase name/value pairs; a malformed run gives `None`.
fn html_attrs(s: &str) -> Option<Vec<(String, String)>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        if i == start {
            return None;
        }
        let name = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            match b.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    i += 1;
                    let from = i;
                    while i < b.len() && b[i] != q {
                        i += 1;
                    }
                    if i >= b.len() {
                        return None;
                    }
                    value = s[from..i].to_string();
                    i += 1;
                }
                _ => {
                    let from = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    value = s[from..i].to_string();
                }
            }
        }
        out.push((name, value));
    }
    Some(out)
}

/// Display text of a wikilink without an alias: `Note`, `Note > Heading`, `Note > ^id`.
fn wikilink_display(link: &tft_markdown::LinkText<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !link.path.is_empty() {
        parts.push(link.path.to_string());
    }
    match &link.subpath {
        Some(Subpath::Headings(hs)) => parts.extend(hs.iter().map(|h| h.to_string())),
        Some(Subpath::Block(b)) => parts.push(format!("^{b}")),
        None => {}
    }
    parts.join(" > ")
}

fn heading_matches(block: &Block<'_>, name: &str) -> Option<u8> {
    if let BlockKind::Heading { level, content, .. } = &block.kind {
        if plain_text(content).trim().eq_ignore_ascii_case(name.trim()) {
            return Some(*level);
        }
    }
    None
}

/// The blocks under a heading path (`#A#B`), to the next heading of the same or higher level.
fn section<'b, 'a>(blocks: &'b [Block<'a>], names: &[&str]) -> Option<&'b [Block<'a>]> {
    let mut range: &[Block<'a>] = blocks;
    for name in names {
        let start = range
            .iter()
            .position(|b| heading_matches(b, name).is_some())?;
        let level = heading_matches(&range[start], name)?;
        let end = range[start + 1..]
            .iter()
            .position(|b| matches!(&b.kind, BlockKind::Heading { level: l, .. } if *l <= level))
            .map(|i| start + 1 + i)
            .unwrap_or(range.len());
        range = &range[start..end];
    }
    Some(range)
}

fn block_by_id<'b, 'a>(blocks: &'b [Block<'a>], id: &str) -> Option<&'b Block<'a>> {
    let mut found = None;
    tft_markdown::walk_blocks(blocks, &mut |b| {
        if found.is_none() && b.id.is_some_and(|i| i.id.eq_ignore_ascii_case(id)) {
            found = Some(b);
        }
    });
    if found.is_some() {
        return found;
    }
    // A list item id lives on its first paragraph in the cache; in the tree the
    // paragraph carries it, and `walk_blocks` visits it.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tft_formats::properties::Value as PropertyValue;

    struct Stub {
        files: HashMap<&'static str, &'static str>,
    }

    impl EmbedSource for Stub {
        fn resolve(&self, linktext: &str, _source: &str) -> Option<String> {
            let target = tft_markdown::parse_linktext(linktext).path;
            let target = if target.is_empty() { "Self" } else { target };
            self.files
                .keys()
                .find(|k| {
                    k.trim_end_matches(".md")
                        .eq_ignore_ascii_case(target.trim_end_matches(".md"))
                })
                .map(|k| k.to_string())
        }
        fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).map(|s| s.to_string())
        }
        fn absolute(&self, path: &str) -> PathBuf {
            PathBuf::from("/vault").join(path)
        }
    }

    fn stub() -> Stub {
        Stub {
            files: HashMap::from([
                ("Self.md", "root"),
                (
                    "Other.md",
                    "---\ntitle: x\n---\n# A\nfirst\n## B\nsecond ^blk\n# C\nthird",
                ),
                ("Loop.md", "![[Loop]]"),
                ("pic.png", ""),
                ("Field.pdf", ""),
                ("Nester.md", "nested\n"),
            ]),
        }
    }

    fn first_embed(block: &RBlock) -> &Embed {
        match block {
            RBlock::Paragraph(segs) => match &segs[0] {
                Segment::Embed(embed) => embed,
                other => panic!("not an embed: {other:?}"),
            },
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    fn para_text(block: &RBlock) -> &RichText {
        match block {
            RBlock::Paragraph(segs) => match &segs[0] {
                Segment::Text(t) => t,
                other => panic!("not text: {other:?}"),
            },
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    #[test]
    fn runs_merge_and_links_cover_their_text() {
        let m = build(
            "Self.md",
            "a **b *c*** [[Note|n]] #tag `x` https://e.com",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "a b c n #tag x https://e.com");
        let bold = InlineStyle {
            bold: true,
            ..InlineStyle::default()
        };
        let bold_italic = InlineStyle {
            bold: true,
            italic: true,
            ..InlineStyle::default()
        };
        assert_eq!(t.runs[0], (2..4, bold));
        assert_eq!(t.runs[1], (4..5, bold_italic));
        assert!(t
            .runs
            .iter()
            .any(|(r, s)| s.link && &t.text[r.clone()] == "n"));
        assert!(t
            .runs
            .iter()
            .any(|(r, s)| s.code && &t.text[r.clone()] == "x"));
        assert_eq!(
            t.links,
            vec![
                (6..7, LinkTarget::Internal("Note".into())),
                (8..12, LinkTarget::Tag("tag".into())),
                (15..28, LinkTarget::External("https://e.com".into())),
            ]
        );
    }

    #[test]
    fn soft_breaks_become_newlines_and_comments_vanish() {
        let m = build("Self.md", "one\ntwo %%hidden%% three", &stub());
        assert_eq!(para_text(&m.blocks[0]).text, "one\ntwo  three");
        let strict = BuildOptions {
            strict_line_breaks: true,
        };
        let m = build_with("Self.md", "one\ntwo  \nthree", &stub(), strict);
        assert_eq!(para_text(&m.blocks[0]).text, "one two\nthree");
    }

    #[test]
    fn embeds_split_paragraphs_and_resolve_recursively() {
        let m = build(
            "Self.md",
            "before ![[pic.png|200x100]] ![[Other#B]] ![[Other#^blk]] ![[Nope]] after",
            &stub(),
        );
        let RBlock::Paragraph(segs) = &m.blocks[0] else {
            panic!()
        };
        assert_eq!(segs.len(), 6);
        match &segs[1] {
            Segment::Embed(Embed {
                kind: EmbedKind::Image(p),
                width,
                height,
                ..
            }) => {
                assert_eq!(p, &PathBuf::from("/vault/pic.png"));
                assert_eq!((*width, *height), (Some(200.0), Some(100.0)));
            }
            other => panic!("{other:?}"),
        }
        match &segs[2] {
            Segment::Embed(Embed {
                kind: EmbedKind::Note { title, blocks, .. },
                ..
            }) => {
                assert_eq!(title, "Other > B");
                assert!(matches!(&blocks[0], RBlock::Heading { plain, .. } if plain == "B"));
                assert_eq!(para_text(&blocks[1]).text, "second");
                assert_eq!(blocks.len(), 2, "section stops before # C");
            }
            other => panic!("{other:?}"),
        }
        match &segs[3] {
            Segment::Embed(Embed {
                kind: EmbedKind::Note { blocks, .. },
                ..
            }) => assert_eq!(para_text(&blocks[0]).text, "second"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &segs[4],
            Segment::Embed(Embed {
                kind: EmbedKind::Missing,
                ..
            })
        ));
        assert_eq!(para_text(&segs[5].as_text()).text, " after");
    }

    impl Segment {
        fn as_text(&self) -> RBlock {
            match self {
                Segment::Text(t) => RBlock::Paragraph(vec![Segment::Text(t.clone())]),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn recursive_embeds_stop_at_the_depth_limit() {
        let m = build("Loop.md", "![[Loop]]", &stub());
        let mut depth = 0;
        let mut blocks = &m.blocks;
        loop {
            let RBlock::Paragraph(segs) = &blocks[0] else {
                panic!()
            };
            match &segs[0] {
                Segment::Embed(Embed {
                    kind: EmbedKind::Note { blocks: inner, .. },
                    ..
                }) => {
                    depth += 1;
                    blocks = inner;
                }
                Segment::Embed(Embed {
                    kind: EmbedKind::TooDeep,
                    ..
                }) => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(depth, MAX_EMBED_DEPTH - 1);
    }

    #[test]
    fn a_pdf_embed_carries_its_subpath_and_reads_nothing() {
        use crate::pdf::embed::DEFAULT_HEIGHT;
        let m = build(
            "Self.md",
            "![[Field.pdf]]\n\n![[Field.pdf#page=2]]\n\n![[Field.pdf#height=200|300x150]]\n\n\
             ![](Field.pdf#page=3)\n\n![[Nope.pdf]]",
            &stub(),
        );
        let pdf = |ix: usize| match &first_embed(&m.blocks[ix]).kind {
            EmbedKind::Pdf(pdf) => pdf.clone(),
            other => panic!("{other:?}"),
        };
        // The stub never read `Field.pdf`, so the model carries only the path and subpath.
        let plain = pdf(0);
        assert_eq!(plain.path, "Field.pdf");
        assert_eq!(plain.absolute, PathBuf::from("/vault/Field.pdf"));
        assert_eq!((plain.page, plain.height), (None, None));
        assert_eq!(plain.box_height(), DEFAULT_HEIGHT);
        // `#page=2` is the link's own subpath, not a heading: zero-based.
        assert_eq!(pdf(1).page, Some(1));
        // `#height=` wins over the alias size; the alias is still the alt.
        let sized = pdf(2);
        assert_eq!(sized.height, Some(200.0));
        assert_eq!(first_embed(&m.blocks[2]).width, Some(300.0));
        // The markdown image form reads the same subpath.
        assert_eq!(pdf(3).page, Some(2));
        // Every embed has an id of its own, so two of one file scroll apart.
        assert_ne!(pdf(0).id, pdf(1).id);
        assert!(matches!(first_embed(&m.blocks[4]).kind, EmbedKind::Missing));
    }

    #[test]
    fn a_canvas_embed_is_its_minimap_and_media_embeds_read_nothing() {
        let mut s = stub();
        s.files.insert(
            "Board.canvas",
            r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":250,"height":60,"text":"A"}],"edges":[]}"#,
        );
        s.files.insert("Empty.canvas", "");
        s.files.insert("tone.mp3", "");
        s.files.insert("clip.mp4", "");
        let m = build(
            "Self.md",
            "![[Board.canvas]]\n\n![[Empty.canvas|400]]\n\n![[tone.mp3]]\n\n![[clip.mp4]]\n\n\
             ![[tone.mp3|300]]\n\n![[Gone.canvas]]",
            &s,
        );
        match &first_embed(&m.blocks[0]).kind {
            EmbedKind::Canvas(c) => {
                assert_eq!(
                    (c.path.as_str(), c.name.as_str()),
                    ("Board.canvas", "Board")
                );
                let map = c
                    .minimap
                    .as_ref()
                    .expect("a canvas with a card has a picture");
                assert_eq!(map.cards.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        // An empty file is the title alone; the size hint changes nothing.
        match &first_embed(&m.blocks[1]).kind {
            EmbedKind::Canvas(c) => assert!(c.minimap.is_none()),
            other => panic!("{other:?}"),
        }
        let media = |ix: usize| match &first_embed(&m.blocks[ix]).kind {
            EmbedKind::Audio(a) => ("audio", a.clone()),
            EmbedKind::Video(v) => ("video", v.clone()),
            other => panic!("{other:?}"),
        };
        let (kind, audio) = media(2);
        assert_eq!(kind, "audio");
        assert_eq!(audio.absolute, PathBuf::from("/vault/tone.mp3"));
        assert_eq!(media(3).0, "video");
        // Two embeds of one file are two elements.
        assert_ne!(media(4).1.id, audio.id);
        assert!(matches!(first_embed(&m.blocks[5]).kind, EmbedKind::Missing));
    }

    #[test]
    fn each_embed_walks_into_embedded_notes() {
        let m = build("Self.md", "![[Nester]]\n\n> ![[Field.pdf]]", &stub());
        let mut seen = Vec::new();
        each_embed(&m.blocks, &mut |embed| seen.push(embed.link.clone()));
        assert_eq!(seen, vec!["Nester", "Field.pdf"]);
    }

    /// `blocks` drops a comment block; `starts` drops exactly the same one.
    #[test]
    fn a_block_comment_shifts_neither_list() {
        let plain = build("Self.md", "# One\n\nbody\n\n# Two\n", &stub());
        assert_eq!(plain.blocks.len(), plain.starts.len());
        let two = plain.starts[2];
        assert!(matches!(&plain.blocks[2], RBlock::Heading { plain, .. } if plain == "Two"));

        // The same note behind a block comment: one more parsed block, the same built blocks.
        let text = "%%\na block comment\n%%\n\n# One\n\nbody\n\n# Two\n";
        let with_comment = build("Self.md", text, &stub());
        assert_eq!(
            with_comment.blocks.len(),
            with_comment.starts.len(),
            "{:?}",
            with_comment.blocks
        );
        assert_eq!(with_comment.blocks.len(), plain.blocks.len());
        assert!(matches!(&with_comment.blocks[2], RBlock::Heading { plain, .. } if plain == "Two"));
        // The offsets moved by the comment's length; `block_at` follows them, not the parsed index.
        assert!(with_comment.starts[2] > two);
        assert_eq!(
            with_comment.block_at(text.find("# Two").unwrap()),
            Some(2),
            "a jump to Two has to land on Two"
        );
    }

    /// An offset inside a footnote definition belongs to the appended Footnotes block.
    #[test]
    fn an_offset_in_a_definition_resolves_to_the_footnotes_block() {
        let text = "a[^1] b\n\n# H\n\n[^1]: the note\n";
        let m = build("Self.md", text, &stub());
        assert_eq!(m.blocks.len(), m.starts.len());
        let last = m.blocks.len() - 1;
        assert!(matches!(m.blocks[last], RBlock::Footnotes(_)));
        let definition = text.find("[^1]: ").unwrap();
        assert_eq!(m.footnotes, vec![definition]);
        assert_eq!(m.block_at(definition), Some(last));
    }

    /// Clicking reference `n` reveals definition `n`; an inline footnote points at itself.
    #[test]
    fn each_footnote_knows_where_its_definition_is() {
        let text = "x[^b] y^[inline] z[^a]\n\n[^a]: A text\n[^b]: B text\n";
        let m = build("Self.md", text, &stub());
        assert_eq!(
            m.footnotes,
            vec![
                text.find("[^b]: ").unwrap(),
                0,
                text.find("[^a]: ").unwrap()
            ],
            "b is 1, the inline one is 2, a is 3"
        );
    }

    #[test]
    fn footnotes_number_by_first_reference_and_collect_at_the_end() {
        let m = build(
            "Self.md",
            "x[^b] y^[inline] z[^a] again[^b] none[^zz]\n\n[^a]: A text\n[^b]: B text",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "x[1] y[2] z[3] again[1] nonezz");
        assert_eq!(
            t.links.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>(),
            vec![
                LinkTarget::Footnote(1),
                LinkTarget::Footnote(2),
                LinkTarget::Footnote(3),
                LinkTarget::Footnote(1)
            ]
        );
        let RBlock::Footnotes(notes) = m.blocks.last().unwrap() else {
            panic!("{:?}", m.blocks.last())
        };
        assert_eq!(notes.len(), 3);
        assert_eq!(para_text(&notes[0].1[0]).text, "B text");
        assert_eq!(para_text(&notes[1].1[0]).text, "inline");
        assert_eq!(para_text(&notes[2].1[0]).text, "A text");
    }

    #[test]
    fn properties_lists_and_callouts() {
        let m = build(
            "Self.md",
            "---\ntitle: T\ntags: [a, b]\n---\n> [!TIP]- Fold me\n> body\n\n1. one\n2. [x] two\n\n- [ ] task",
            &stub(),
        );
        let RBlock::Properties(entries) = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0])
        };
        assert_eq!(
            entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            vec!["title", "tags"]
        );
        assert_eq!(entries[0].value, PropertyValue::Text("T".into()));
        assert_eq!(
            entries[1].value,
            PropertyValue::List(vec!["a".into(), "b".into()])
        );
        match &m.blocks[1] {
            RBlock::Callout {
                kind, fold, title, ..
            } => {
                assert_eq!(*kind, "tip");
                assert_eq!(*fold, Some(true));
                assert_eq!(title.text, "Fold me");
            }
            other => panic!("{other:?}"),
        }
        match &m.blocks[2] {
            RBlock::List { items, .. } => {
                assert_eq!(items[0].number, Some(1));
                assert_eq!(items[1].number, Some(2));
                assert_eq!(items[1].task, Some('x'));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.starts.len(), 4);
        assert_eq!(m.block_at(0), Some(0));
        assert_eq!(m.block_at(m.starts[2] + 3), Some(2));
    }

    #[test]
    fn task_markers_carry_the_line_and_stop_at_an_embed() {
        let vault = Stub {
            files: HashMap::from([("Tasks.md", "- [ ] inner\n")]),
        };
        let m = build(
            "Self.md",
            "---\na: 1\n---\n\n- [ ] alpha\n  - [x] beta\n\n![[Tasks]]\n",
            &vault,
        );
        let RBlock::List { items, .. } = &m.blocks[1] else {
            panic!("{:?}", m.blocks[1])
        };
        assert_eq!(items[0].marker, Some(TaskMarker { line: 4, col: 2 }));
        let RBlock::List { items: nested, .. } = &items[0].children[1] else {
            panic!("{:?}", items[0].children)
        };
        assert_eq!(nested[0].marker, Some(TaskMarker { line: 5, col: 4 }));
        // The embedded item's bytes are another file's, so it carries no marker.
        let RBlock::Paragraph(segments) = m.blocks.last().unwrap() else {
            panic!("{:?}", m.blocks.last())
        };
        let Segment::Embed(embed) = &segments[0] else {
            panic!("{:?}", segments[0])
        };
        let EmbedKind::Note { blocks, .. } = &embed.kind else {
            panic!("{:?}", embed.kind)
        };
        let RBlock::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(items[0].task, Some(' '));
        assert_eq!(items[0].marker, None);
    }

    #[test]
    fn a_code_block_copies_lf_text_with_one_newline_removed() {
        let m = build("Self.md", "```js\nlet x = 1;\n\n\n```\n", &stub());
        match &m.blocks[0] {
            RBlock::Code {
                language,
                text,
                copy,
                ..
            } => {
                assert_eq!(language.as_deref(), Some("js"));
                assert_eq!(text, "let x = 1;");
                assert_eq!(copy, "let x = 1;\n\n");
            }
            other => panic!("{other:?}"),
        }
        // A CRLF file renders and copies as LF, as Obsidian does, keeping its own endings.
        let m = build("Self.md", "```\r\none\r\ntwo\r\n```\r\n", &stub());
        match &m.blocks[0] {
            RBlock::Code { text, copy, .. } => {
                assert_eq!(text, "one\ntwo");
                assert_eq!(copy, "one\ntwo");
            }
            other => panic!("{other:?}"),
        }
    }

    fn widgets(src: &str, vault: &dyn EmbedSource) -> Vec<Widget> {
        let doc = parse(src);
        build_widgets("Self.md", &doc, vault, BuildOptions::default())
    }

    #[test]
    fn live_preview_makes_a_mermaid_fence_a_widget_and_no_other_code() {
        let src = "```mermaid\nflowchart LR\n  A --> B\n```\n\n```rust\nfn x() {}\n```\n";
        let w = widgets(src, &stub());
        assert_eq!(w.len(), 1, "{w:?}");
        let RBlock::Code { language, text, .. } = &w[0].block else {
            panic!("{:?}", w[0].block)
        };
        assert_eq!(language.as_deref(), Some("mermaid"));
        assert_eq!(text, "flowchart LR\n  A --> B");
        assert_eq!(
            &src[w[0].span.clone()],
            "```mermaid\nflowchart LR\n  A --> B\n```"
        );
    }

    #[test]
    fn live_preview_makes_a_callout_a_widget() {
        let src = "top\n\n> [!warning]- Careful\n> body\n> more\n\n> plain quote\n";
        let w = widgets(src, &stub());
        assert_eq!(w.len(), 1, "the plain quote is not a widget: {w:?}");
        let RBlock::Callout {
            kind,
            fold,
            title,
            children,
        } = &w[0].block
        else {
            panic!("{:?}", w[0].block)
        };
        assert_eq!(*kind, "warning");
        assert_eq!(*fold, Some(true), "`-` opens collapsed, as in Obsidian");
        assert_eq!(title.text, "Careful");
        assert_eq!(children.len(), 1);
        // The span covers every line, so Live Preview folds all three behind the widget.
        assert_eq!(
            &src[w[0].span.clone()],
            "> [!warning]- Careful\n> body\n> more"
        );
    }

    #[test]
    fn an_image_html_block_becomes_an_image_widget() {
        let w = widgets("<img src=\"pic.png\" width=\"100\" alt=\"A\">\n", &stub());
        assert_eq!(w.len(), 1);
        let RBlock::Paragraph(segs) = &w[0].block else {
            panic!("{:?}", w[0].block)
        };
        match &segs[0] {
            Segment::Embed(Embed {
                kind: EmbedKind::Image(path),
                alt,
                width,
                height,
                ..
            }) => {
                assert_eq!(path, &PathBuf::from("/vault/pic.png"));
                assert_eq!(alt, "A");
                assert_eq!((*width, *height), (Some(100.0), None));
            }
            other => panic!("{other:?}"),
        }
        // Consecutive raw imgs are one block; a percent-escaped src is the vault path.
        let m = build(
            "Self.md",
            "<img src='pic.png'/>\n<img src=pic%2Epng height=50>\n",
            &stub(),
        );
        let RBlock::Paragraph(segs) = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0])
        };
        assert_eq!(segs.len(), 2);
        assert!(matches!(
            &segs[1],
            Segment::Embed(Embed {
                kind: EmbedKind::Image(_),
                height: Some(h),
                ..
            }) if *h == 50.0
        ));
    }

    /// A remote image reads its size off the alt; a reference image and a raw `<img>`
    /// are fetched the same way, while a wikilink URL and `//` or `file:` are not remote.
    #[test]
    fn http_images_are_remote_embeds() {
        let m = build(
            "Self.md",
            "![alt|200x100](https://e.com/x.png)\n\n\
             ![a|200](http://h/x%20y.png \"title\")\n\n\
             ![ref alt][rr]\n\n\
             [rr]: HTTPS://e.com/r.png \"Ref\"\n\n\
             <img src=\"https://e.com/raw%20x.png\" width=\"40\" alt=\"raw\">\n\n\
             ![[https://e.com/x.png]]\n\n\
             ![p](//e.com/x.png)\n",
            &stub(),
        );
        let remote = |ix: usize| {
            let e = first_embed(&m.blocks[ix]);
            match &e.kind {
                EmbedKind::Remote(url) => (url.as_str(), e.alt.as_str(), e.width, e.height),
                other => panic!("block {ix}: {other:?}"),
            }
        };
        assert_eq!(
            remote(0),
            ("https://e.com/x.png", "alt", Some(200.), Some(100.))
        );
        // The URL is requested as written: no percent-decoding.
        assert_eq!(remote(1), ("http://h/x%20y.png", "a", Some(200.), None));
        assert_eq!(remote(2), ("HTTPS://e.com/r.png", "ref alt", None, None));
        assert_eq!(
            remote(3),
            ("https://e.com/raw%20x.png", "raw", Some(40.), None)
        );
        assert!(matches!(first_embed(&m.blocks[4]).kind, EmbedKind::Missing));
        // `//host` resolves against `app://` and loads nothing: a missing file, never a request.
        assert!(matches!(first_embed(&m.blocks[5]).kind, EmbedKind::Missing));
        // Each stands alone in its paragraph, so Live Preview draws each.
        let src = "![alt](https://e.com/x.png)\n\n![r][rr]\n\n[rr]: https://e.com/r.png\n";
        let w = widgets(src, &stub());
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w
            .iter()
            .all(|w| matches!(&first_embed(&w.block).kind, EmbedKind::Remote(_))));
    }

    /// An image alone in a list item or quote is a widget spanning the embed, not the
    /// marker; one sharing text with its paragraph is not, nor anything inside a callout.
    #[test]
    fn an_image_alone_in_a_list_item_or_quote_is_a_widget() {
        let src = "- ![[pic.png]]\n\
                   - text ![[pic.png]]\n\
                   - item\n\
                   \n  ![[pic.png|60]]\n\
                   \n1. ![x](pic.png)\n   - [ ] ![r](https://e.com/r.png)\n\
                   \n> ![[pic.png]]\n> > ![[pic.png]]\n\
                   \n> [!note]\n> ![[pic.png]]\n";
        let w = widgets(src, &stub());
        let shown: Vec<(&str, &str)> = w
            .iter()
            .map(|w| {
                let line_start = src[..w.span.start].rfind('\n').map_or(0, |i| i + 1);
                (&src[line_start..w.span.start], &src[w.span.clone()])
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                ("- ", "![[pic.png]]"),
                ("  ", "![[pic.png|60]]"),
                ("1. ", "![x](pic.png)"),
                ("   - [ ] ", "![r](https://e.com/r.png)"),
                ("> ", "![[pic.png]]"),
                ("> > ", "![[pic.png]]"),
                ("", "> [!note]\n> ![[pic.png]]"),
            ]
        );
        assert!(matches!(first_embed(&w[1].block).width, Some(60.)));
        assert!(matches!(
            first_embed(&w[3].block).kind,
            EmbedKind::Remote(_)
        ));
    }

    #[test]
    fn other_html_blocks_stay_source() {
        // A mixed block is not something the renderer can draw, so it keeps its raw
        // text and never folds its lines (comments and scripts now draw nothing).
        let src = "<img src=\"pic.png\" width=\"50%\">\n";
        assert!(widgets(src, &stub()).is_empty(), "{src:?} became a widget");
        let m = build("Self.md", src, &stub());
        assert!(
            matches!(&m.blocks[0], RBlock::Html(_)),
            "{src:?} -> {:?}",
            m.blocks[0]
        );
    }

    /// A raw `<table>` is a table in the reading view (first `<th>` row as header,
    /// Markdown literal in a cell) but stays source in Live Preview, which never makes
    /// a raw table a widget.
    #[test]
    fn a_raw_table_is_a_table_in_reading_and_source_in_live_preview() {
        let src = "<table>\n<tr><th>H1</th><th align=\"right\">H2</th></tr>\n<tr><td>a</td><td><b>b</b> **md**</td></tr>\n</table>\n";
        assert!(widgets(src, &stub()).is_empty());
        let m = build("Self.md", src, &stub());
        let RBlock::Table {
            alignments,
            header,
            rows,
            spans,
            ..
        } = &m.blocks[0]
        else {
            panic!("{:?}", m.blocks[0]);
        };
        let texts = |row: &[RichText]| row.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(header), ["H1", "H2"]);
        assert_eq!(rows.len(), 1);
        assert_eq!(texts(&rows[0]), ["a", "b **md**"]);
        let bold = InlineStyle {
            bold: true,
            ..InlineStyle::default()
        };
        assert_eq!(rows[0][1].runs, vec![(0..1, bold)]);
        assert_eq!(alignments, &[Alignment::None, Alignment::Right]);
        assert!(spans.is_empty(), "a raw table is not edited in place");

        // No `<th>` row: no header, and a ragged row is padded to the widest.
        let m = build(
            "Self.md",
            "<table><tr><td>c</td></tr><tr><td>d</td><td>e</td></tr></table>\n",
            &stub(),
        );
        let RBlock::Table { header, rows, .. } = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0]);
        };
        assert!(header.is_empty());
        assert_eq!(texts(&rows[0]), ["c", ""]);
        assert_eq!(texts(&rows[1]), ["d", "e"]);
        let m = build(
            "Self.md",
            "<table border=\"1\"><thead><tr><td>L</td></tr></thead><tbody><tr><th>x</th></tr></tbody></table>\n",
            &stub(),
        );
        let RBlock::Table { header, rows, .. } = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0]);
        };
        assert_eq!(texts(header), ["L"]);
        // A `<th>` in the body is bold, the header's weight.
        assert_eq!(rows[0][0].runs, vec![(0..1, bold)]);

        // A table in a cell is more than the grid holds: source.
        let m = build(
            "Self.md",
            "<table><tr><td><table><tr><td>x</td></tr></table></td></tr></table>\n",
            &stub(),
        );
        assert!(matches!(&m.blocks[0], RBlock::Html(_)), "{:?}", m.blocks[0]);
    }

    /// Live Preview's inline element: its content literal, or nothing for what it cannot draw.
    #[test]
    fn an_inline_element_widget_keeps_its_content_literal() {
        let w = html_inline_widget("<b>**md** inside</b>").unwrap();
        assert_eq!(w.text, "**md** inside");
        assert!(w.runs.iter().all(|(_, s)| s.bold));
        assert_eq!(
            html_inline_widget("<b><i>both</i></b>").unwrap().text,
            "both"
        );
        assert!(html_inline_widget("<script>x</script>").is_none());
        assert!(html_inline_widget("<foo>x</foo>").is_none());
        assert!(html_inline_widget("<b>a<br>b</b>").is_none());
        assert!(html_inline_widget("<span></span>").is_none());
    }

    /// A raw `<pre>` is a wrapped code block (copyable with a `<code>` child);
    /// `<details>` folds its summary over its body; a container keeps its content.
    #[test]
    fn pre_details_and_containers_render_rather_than_showing_source() {
        let m = build(
            "Self.md",
            "<pre>**not bold** &amp; two  spaces\n  indented</pre>\n",
            &stub(),
        );
        match &m.blocks[0] {
            RBlock::Code {
                text,
                wrap,
                copyable,
                language,
                ..
            } => {
                assert_eq!(text, "**not bold** & two  spaces\n  indented");
                assert!(*wrap && !*copyable);
                assert_eq!(*language, None);
            }
            other => panic!("{other:?}"),
        }
        let m = build("Self.md", "<pre><code>let x = 1;\n</code></pre>\n", &stub());
        match &m.blocks[0] {
            RBlock::Code { text, copyable, .. } => {
                assert_eq!(text, "let x = 1;");
                assert!(*copyable);
            }
            other => panic!("{other:?}"),
        }
        let m = build(
            "Self.md",
            "<details open>\n<summary>More <b>here</b></summary>\nThe <i>body</i>.\n</details>\n",
            &stub(),
        );
        match &m.blocks[0] {
            RBlock::Details {
                open,
                summary,
                body,
                ..
            } => {
                assert!(*open);
                assert_eq!(summary.text, "More here");
                assert!(summary
                    .runs
                    .iter()
                    .any(|(r, s)| s.bold && &summary.text[r.clone()] == "here"));
                assert_eq!(body.len(), 1);
                match &body[0] {
                    RBlock::Paragraph(segments) => match &segments[0] {
                        Segment::Text(rich) => assert_eq!(rich.text, "The body."),
                        other => panic!("{other:?}"),
                    },
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        let m = build(
            "Self.md",
            "<div class=\"x\">mixed <b>bold</b> &amp; plain</div>\n",
            &stub(),
        );
        match &m.blocks[0] {
            RBlock::Paragraph(segments) => match &segments[0] {
                Segment::Text(rich) => assert_eq!(rich.text, "mixed bold & plain"),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        // Each of those is a widget in Live Preview; a table stays source.
        assert_eq!(widgets("<pre>x</pre>\n", &stub()).len(), 1);
        assert_eq!(
            widgets("<details><summary>s</summary>b</details>\n", &stub()).len(),
            1
        );
        assert_eq!(widgets("<center>c</center>\n", &stub()).len(), 1);
        assert!(widgets("<table><tr><td>c</td></tr></table>\n", &stub()).is_empty());
    }

    /// Obsidian's sanitizer takes a `<script>` out whole and keeps what follows, and no
    /// DOM paints a comment, so neither is drawn as muted source.
    #[test]
    fn a_comment_or_script_block_draws_nothing() {
        for src in [
            "<!-- a comment -->\n",
            "<!-- one --> <!-- two -->\n",
            "<script>window.x = 1</script>\n",
            "<script type=\"text/javascript\">a > b;</script>\n",
            "<script>a</script>\n<!-- and a comment -->\n",
            // Unterminated: the rest of the block goes with it, as a parser reading to end does.
            "<!-- never closed\n",
            "<script>never closed\n",
            // The closing tag may carry whitespace before its `>`.
            "<script>x</script >\n",
        ] {
            let m = build("Self.md", src, &stub());
            assert!(m.blocks.is_empty(), "{src:?} -> {:?}", m.blocks);
            assert!(widgets(src, &stub()).is_empty(), "{src:?} became a widget");
        }
        // The text after a script is its own block and still renders; offsets stay in step.
        let m = build("Self.md", "<script>x</script>\n\nafter\n", &stub());
        assert_eq!(m.blocks.len(), 1);
        assert!(
            matches!(&m.blocks[0], RBlock::Paragraph(_)),
            "{:?}",
            m.blocks[0]
        );
        assert_eq!(m.starts.len(), 1);
        // A comment beside a container drops out and the container draws its content.
        for src in [
            "<!-- a comment --><div>x</div>\n",
            "<div>x</div>\n<!-- a comment -->\n",
        ] {
            let m = build("Self.md", src, &stub());
            match &m.blocks[0] {
                RBlock::Paragraph(segments) => match &segments[0] {
                    Segment::Text(rich) => assert_eq!(rich.text.trim(), "x", "{src:?}"),
                    other => panic!("{src:?} -> {other:?}"),
                },
                other => panic!("{src:?} -> {other:?}"),
            }
        }
    }

    #[test]
    fn markdown_image_size_hints_match_the_wikilink_form() {
        let m = build(
            "Self.md",
            "![a|100](pic.png)\n\n![a|200x100](pic.png)\n\n![a](pic.png)\n\n\
             ![a|wide](pic.png)\n\n![[pic.png|wide|100]]",
            &stub(),
        );
        let e = |ix: usize| {
            let e = first_embed(&m.blocks[ix]);
            assert!(matches!(e.kind, EmbedKind::Image(_)), "{e:?}");
            (e.alt.as_str(), e.width, e.height)
        };
        assert_eq!(e(0), ("a", Some(100.), None));
        assert_eq!(e(1), ("a", Some(200.), Some(100.)));
        assert_eq!(e(2), ("a", None, None));
        // A part that is not a size stays in the alt rather than vanishing.
        assert_eq!(e(3), ("a|wide", None, None));
        // The wikilink spelling goes through the same split.
        assert_eq!(e(4), ("wide", Some(100.), None));
    }

    #[test]
    fn an_embed_outside_a_paragraph_is_a_link_not_its_brackets() {
        let m = build(
            "Self.md",
            "# See ![[pic.png|200]]\n\n| a |\n| --- |\n| ![[Other]] |",
            &stub(),
        );
        let link = InlineStyle {
            link: true,
            ..InlineStyle::default()
        };
        match &m.blocks[0] {
            RBlock::Heading { text, .. } => {
                assert_eq!(text.text, "See pic.png");
                assert_eq!(text.runs, vec![(4..11, link)]);
                assert_eq!(
                    text.links,
                    vec![(4..11, LinkTarget::Internal("pic.png".into()))]
                );
            }
            other => panic!("{other:?}"),
        }
        match &m.blocks[1] {
            RBlock::Table { rows, .. } => {
                let cell = &rows[0][0];
                assert_eq!(cell.text, "Other");
                assert_eq!(cell.runs, vec![(0..5, link)]);
                assert_eq!(
                    cell.links,
                    vec![(0..5, LinkTarget::Internal("Other".into()))]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// The style of the run holding `word`, or plain when no run does.
    fn style_at(t: &RichText, word: &str) -> InlineStyle {
        let at = t
            .text
            .find(word)
            .unwrap_or_else(|| panic!("{word:?} not in {:?}", t.text));
        t.runs
            .iter()
            .find(|(r, _)| r.contains(&at))
            .map(|(_, s)| *s)
            .unwrap_or_default()
    }

    fn only(f: fn(&mut InlineStyle)) -> InlineStyle {
        let mut s = InlineStyle::default();
        f(&mut s);
        s
    }

    #[test]
    fn inline_html_tags_become_styles() {
        let m = build(
            "Self.md",
            "a <b>b</b> <strong>s</strong> <i>i</i> <em>e</em> <s>x</s> <del>d</del> \
             <STRIKE>k</STRIKE> <mark>m</mark> <u>u</u> <kbd>K</kbd> <sup>2</sup> <sub>0</sub>",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "a b s i e x d k m u K 2 0");
        assert_eq!(style_at(t, "a"), InlineStyle::default());
        assert_eq!(style_at(t, "b"), only(|s| s.bold = true));
        assert_eq!(style_at(t, "s"), only(|s| s.bold = true));
        assert_eq!(style_at(t, "i"), only(|s| s.italic = true));
        assert_eq!(style_at(t, "e"), only(|s| s.italic = true));
        assert_eq!(style_at(t, "x"), only(|s| s.strike = true));
        assert_eq!(style_at(t, "d"), only(|s| s.strike = true));
        assert_eq!(style_at(t, "k"), only(|s| s.strike = true));
        assert_eq!(style_at(t, "m"), only(|s| s.highlight = true));
        assert_eq!(style_at(t, "u"), only(|s| s.underline = true));
        assert_eq!(style_at(t, "K"), only(|s| s.kbd = true));
        assert_eq!(style_at(t, "2"), only(|s| s.sup = true));
        assert_eq!(style_at(t, "0"), only(|s| s.sub = true));
        assert!(t.runs.iter().all(|(_, s)| !s.html), "{:?}", t.runs);
        let m = build("Self.md", "a <code>c</code> <big>B</big>", &stub());
        let t = para_text(&m.blocks[0]);
        assert_eq!(style_at(t, "c"), only(|s| s.code = true));
        assert_eq!(style_at(t, "B"), only(|s| s.big = true));
    }

    #[test]
    fn inline_html_nests_drops_wrappers_and_ignores_a_stray_closer() {
        let m = build(
            "Self.md",
            "<u><b>x</b>y</u></b>z<!-- gone --><span class=\"c\">w</span><div>v</div>\
             <font color=\"red\">f</font><center>c</center><small>s</small><p>p</p>",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "xyzwvfcsp");
        assert_eq!(
            t.runs,
            vec![
                (
                    0..1,
                    only(|s| {
                        s.underline = true;
                        s.bold = true;
                    })
                ),
                (1..2, only(|s| s.underline = true)),
                // `<font color>` is the colour; `<small>` its size.
                (5..6, only(|s| s.color = Some(0xff0000ff))),
                (7..8, only(|s| s.small = true)),
            ]
        );
    }

    /// The five `style` properties TFT draws: named/hex/`rgb()`/`hsl()` colours,
    /// `background`, weight, slant and line; `var()`, a size and a stray `color` are dropped.
    #[test]
    fn an_inline_style_attribute_colours_and_weights_its_text() {
        let m = build(
            "Self.md",
            "<span style=\"color: red\">a</span> <span style=\"color:#00ff00\">b</span> \
             <span style=\"background-color: yellow\">c</span> <span style=\"font-weight: bold\">d</span> \
             <span style=\"font-style: italic\">e</span> <span style=\"text-decoration: underline\">f</span> \
             <span style=\"text-decoration: line-through\">g</span> <span style=\"color: rgb(0, 0, 255)\">h</span> \
             <span style=\"color: hsl(120, 100%, 25%)\">i</span> <span style=\"color: var(--text-accent)\">j</span> \
             <span style=\"font-size: 20px\">k</span> <b style=\"color: red; font-weight: normal\">l</b> \
             <span style=\"COLOR: RebeccaPurple !important; color: nope\">m</span> <span style=\"background: orange\">n</span> \
             <span color=\"red\">o</span> <u style=\"text-decoration: none\">p</u>",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(style_at(t, "a"), only(|s| s.color = Some(0xff0000ff)));
        assert_eq!(style_at(t, "b"), only(|s| s.color = Some(0x00ff00ff)));
        assert_eq!(style_at(t, "c"), only(|s| s.background = Some(0xffff00ff)));
        assert_eq!(style_at(t, "d"), only(|s| s.bold = true));
        assert_eq!(style_at(t, "e"), only(|s| s.italic = true));
        assert_eq!(style_at(t, "f"), only(|s| s.underline = true));
        assert_eq!(style_at(t, "g"), only(|s| s.strike = true));
        assert_eq!(style_at(t, "h"), only(|s| s.color = Some(0x0000ffff)));
        assert_eq!(style_at(t, "i"), only(|s| s.color = Some(0x008000ff)));
        assert_eq!(style_at(t, "j"), InlineStyle::default());
        assert_eq!(style_at(t, "k"), InlineStyle::default());
        // The attribute wins over the tag's own weight.
        assert_eq!(style_at(t, "l"), only(|s| s.color = Some(0xff0000ff)));
        assert_eq!(style_at(t, "m"), only(|s| s.color = Some(0x663399ff)));
        assert_eq!(style_at(t, "n"), only(|s| s.background = Some(0xffa500ff)));
        assert_eq!(style_at(t, "o"), InlineStyle::default());
        assert_eq!(style_at(t, "p"), InlineStyle::default());
    }

    /// Chromium's numbers for `size` and `super`/`sub`, as Obsidian computed them over
    /// a 16 px body and a 23.392 px heading.
    #[test]
    fn scripts_shrink_and_rise_the_way_chromium_does() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.02;
        let sup = only(|s| s.sup = true).script().unwrap();
        let (size, rise) = sup.at(16.);
        assert!(near(size, 13.333) && near(rise, 6.328), "{size} {rise}");
        let (size, rise) = sup.at(23.392);
        assert!(near(size, 19.493) && near(rise, 8.797), "{size} {rise}");
        let (size, rise) = only(|s| s.sub = true).script().unwrap().at(16.);
        assert!(near(size, 13.333) && near(rise, -4.187), "{size} {rise}");
        let (size, rise) = only(|s| s.small = true).script().unwrap().at(16.);
        assert!(near(size, 13.333) && rise == 0., "{size} {rise}");
        let (size, rise) = only(|s| s.big = true).script().unwrap().at(16.);
        assert!(near(size, 19.2) && rise == 0., "{size} {rise}");
        // A footnote reference is a `<sup>` in Obsidian's reading view.
        assert_eq!(only(|s| s.footnote = true).script(), Some(sup));
        assert_eq!(only(|s| s.bold = true).script(), None);
    }

    fn html_table_of(src: &str) -> (Vec<RichText>, Vec<Vec<RichText>>, HtmlTableExtras) {
        let m = build("Self.md", src, &stub());
        let RBlock::Table {
            header, rows, html, ..
        } = &m.blocks[0]
        else {
            panic!("{:?}", m.blocks[0]);
        };
        (
            header.clone(),
            rows.clone(),
            html.as_deref().cloned().unwrap_or_default(),
        )
    }

    fn place(col: usize, colspan: usize, rowspan: usize) -> CellPlace {
        CellPlace {
            col,
            colspan,
            rowspan,
            align: Alignment::None,
        }
    }

    /// Obsidian's grid placement: a `colspan`, a `rowspan` pushing the next row right,
    /// `rowspan="0"` and a past-end span stopping at the last row, a wide `colspan` at the last column.
    #[test]
    fn a_raw_tables_spans_place_its_cells_like_a_browser() {
        let (header, rows, x) = html_table_of(
            "<table>\n<caption>The caption</caption>\n<tr><th>A</th><th>B</th><th>C</th></tr>\n\
             <tr><td colspan=\"2\">ab</td><td>c</td></tr>\n\
             <tr><td rowspan=\"2\">d</td><td>e</td><td>f</td></tr>\n\
             <tr><td>g</td><td>h</td></tr>\n</table>\n",
        );
        assert_eq!(header.len(), 3);
        assert_eq!(rows.iter().map(Vec::len).collect::<Vec<_>>(), [2, 3, 2]);
        assert_eq!(
            x.caption.as_ref().map(|c| c.text.as_str()),
            Some("The caption")
        );
        assert!(!x.caption_bottom);
        assert_eq!(x.columns, 3);
        assert_eq!(
            x.places,
            vec![
                vec![place(0, 1, 1), place(1, 1, 1), place(2, 1, 1)],
                vec![place(0, 2, 1), place(2, 1, 1)],
                vec![place(0, 1, 2), place(1, 1, 1), place(2, 1, 1)],
                vec![place(1, 1, 1), place(2, 1, 1)],
            ]
        );
        let (_, _, x) = html_table_of(
            "<table><tr><td colspan=\"5\">over-wide</td></tr><tr><td>a</td><td>b</td></tr></table>\n",
        );
        assert_eq!(x.columns, 2);
        assert_eq!(x.places[0], vec![place(0, 2, 1)]);
        let (_, _, x) = html_table_of(
            "<table><tr><td rowspan=\"0\">zero</td><td>a</td></tr><tr><td>b</td></tr></table>\n",
        );
        assert_eq!(
            x.places,
            vec![vec![place(0, 1, 2), place(1, 1, 1)], vec![place(1, 1, 1)]]
        );
        let (_, _, x) = html_table_of(
            "<table><tr><td rowspan=\"5\">past</td><td>a</td></tr><tr><td>b</td></tr></table>\n",
        );
        assert_eq!(x.places[0][0], place(0, 1, 2));
        // `colspan="0"` is one and a header span stays in the header, so this spans nothing.
        let (header, rows, x) = html_table_of(
            "<table><thead><tr><th rowspan=\"3\">h</th><th>i</th></tr></thead>\
             <tbody><tr><td colspan=\"0\">c0</td><td rowspan=\"2\">x</td></tr></tbody></table>\n",
        );
        assert_eq!((header.len(), rows[0].len()), (2, 2));
        assert!(x.places.is_empty());
    }

    /// A caption alone keeps the plain grid; `caption-side: bottom` is read from its `style`.
    #[test]
    fn a_raw_tables_caption_is_kept_with_its_side() {
        let (_, rows, x) = html_table_of(
            "<table><caption style=\"caption-side: bottom\"><b>bold</b> caption</caption><tr><td>x</td><td>y</td></tr></table>\n",
        );
        assert_eq!(rows.len(), 1);
        assert!(x.places.is_empty());
        assert!(x.caption_bottom);
        let caption = x.caption.unwrap();
        assert_eq!(caption.text, "bold caption");
        assert_eq!(caption.runs, vec![(0..4, only(|s| s.bold = true))]);
        // No caption and no span: a Markdown table's shape, nothing extra.
        let m = build("Self.md", "<table><tr><td>x</td></tr></table>\n", &stub());
        assert!(matches!(&m.blocks[0], RBlock::Table { html: None, .. }));
    }

    #[test]
    fn a_br_is_a_line_break() {
        let m = build("Self.md", "one<br>two<br/>three<br />four<BR>five", &stub());
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "one\ntwo\nthree\nfour\nfive");
        assert!(t.runs.is_empty(), "{:?}", t.runs);
    }

    #[test]
    fn a_raw_anchor_is_an_external_link_kept_as_written() {
        // A raw href is unresolved (`Other.md` is external); `javascript:` drops to plain text.
        let m = build(
            "Self.md",
            "see <a href=\"Other.md\">the note</a> and <a href='javascript:alert(1)'>no</a> \
             <a>bare</a>",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "see the note and no bare");
        assert_eq!(t.runs, vec![(4..12, only(|s| s.link = true))]);
        assert_eq!(
            t.links,
            vec![(4..12, LinkTarget::External("Other.md".into()))]
        );
    }

    #[test]
    fn an_unclosed_tag_runs_to_the_end_of_its_paragraph() {
        // The spec's `start <b>bold never closed`: closed at the paragraph's end, not leaking.
        let m = build(
            "Self.md",
            "start <b>bold never closed\n\n<a href=\"https://e.com\">open link\n\nplain",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "start bold never closed");
        assert_eq!(t.runs, vec![(6..23, only(|s| s.bold = true))]);
        let t = para_text(&m.blocks[1]);
        assert_eq!(
            t.links,
            vec![(0..9, LinkTarget::External("https://e.com".into()))]
        );
        assert!(para_text(&m.blocks[2]).runs.is_empty());
    }

    #[test]
    fn inline_html_outside_the_subset_stays_muted_source() {
        let m = build("Self.md", "a <script>x</script> b <q>c</q> <b", &stub());
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "a <script>x</script> b <q>c</q> <b");
        let source: Vec<&str> = t
            .runs
            .iter()
            .filter(|(_, s)| s.html)
            .map(|(r, _)| &t.text[r.clone()])
            .collect();
        assert_eq!(source, vec!["<script>", "</script>", "<q>", "</q>"]);
        assert!(t.runs.iter().all(|(_, s)| *s == only(|s| s.html = true)));
    }

    #[test]
    fn html_styles_overlay_markdown_nesting() {
        // The emphasis closes before the bold and the bold after it; neither restores over the other.
        let m = build("Self.md", "*a <b>b* c</b> d", &stub());
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "a b c d");
        assert_eq!(style_at(t, "a"), only(|s| s.italic = true));
        assert_eq!(
            style_at(t, "b"),
            only(|s| {
                s.italic = true;
                s.bold = true;
            })
        );
        assert_eq!(style_at(t, "c"), only(|s| s.bold = true));
        assert_eq!(style_at(t, "d"), InlineStyle::default());
    }

    #[test]
    fn callout_without_title_shows_its_type() {
        let m = build("Self.md", "> [!warning]\n> text", &stub());
        match &m.blocks[0] {
            RBlock::Callout { title, fold, .. } => {
                assert_eq!(title.text, "Warning");
                assert_eq!(*fold, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn display_math_in_a_paragraph_is_its_own_segment() {
        let opts = BuildOptions::default();
        let m = build_with("A.md", "Inline $x$ then $$y$$ and after.", &stub(), opts);
        let RBlock::Paragraph(segments) = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0])
        };
        // The space on either side of the block is its margin, not text.
        let texts: Vec<_> = segments
            .iter()
            .map(|s| match s {
                Segment::Text(t) => format!("text:{}", t.text),
                Segment::Math(tex) => format!("math:{tex}"),
                Segment::Embed(_) => "embed".to_string(),
            })
            .collect();
        assert_eq!(
            texts,
            vec!["text:Inline x then", "math:y", "text:and after."]
        );
        // Inline math is a styled run holding the TeX, for the view to typeset over a placeholder.
        let Segment::Text(first) = &segments[0] else {
            unreachable!()
        };
        assert_eq!(
            first.runs,
            vec![(
                7..8,
                InlineStyle {
                    math: true,
                    ..InlineStyle::default()
                }
            )]
        );
        let m = build_with("A.md", "$$\nz\n$$", &stub(), BuildOptions::default());
        assert_eq!(m.blocks[0], RBlock::Math("z".to_string()));
    }
}
