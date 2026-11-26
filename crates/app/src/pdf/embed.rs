//! An embedded PDF (`![[x.pdf]]`) and a canvas file card: pdf.js's toolbar
//! over pages at one zoom. Two custom elements ask the `documents` cache for
//! sizes/pages; only visible pages render. Nothing reads a file on the UI
//! thread.

use crate::pdf::view::{step_in, step_out, CSS_UNITS};
use crate::reading::model::{each_embed, EmbedKind, RBlock};
use crate::theme::{Theme, FONT_UI_SMALL, ICON_SIZE};
use gpui::{
    div, img, point, prelude::*, px, relative, size, white, AnyElement, App, AvailableSpace,
    Bounds, Div, DivFrameState, Element, ElementId, GlobalElementId, Hitbox, InspectorElementId,
    LayoutId, Pixels, ScrollHandle, SharedString, Style, Window,
};
use serde_json::{json, Value as Json};
use std::cell::Cell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tft_pdf::PageSize;

use super::{documents, page_source, DocState, Loaded};

/// How tall an embed is before its file has loaded, when the link names no
/// height: observed as the box's `height: 800px` on every unforced embed
/// before pdf.js sizes it to the page.
pub const DEFAULT_HEIGHT: f32 = 800.0;

/// The toolbar's height, less its bottom hairline: the first page starts
/// 39.37 px under the box's top in the embed and the canvas card alike,
/// which is the 0.69 px box border, this, and the toolbar's own 0.69 px
/// border (drawn 1 px here, so a page sits 40 px down).
pub const TOOLBAR_HEIGHT: f32 = 38.0;

/// What the frame adds around the content: its two borders, the toolbar
/// and the toolbar's border.
pub const CHROME_HEIGHT: f32 = 2.0 + TOOLBAR_HEIGHT + 1.0;

/// Between two pages: observed as 10 px (page 2 starts 10 px under page 1's
/// bottom; the content ends with the last page).
pub const PAGE_GAP: f32 = 10.0;

/// What Obsidian's styled scrollbar takes off the right of the box: the
/// client width was 591 inside a 604.43 box (less two 0.69 borders) and 587
/// inside a 600 card. gpui draws no scrollbar, so this strip stays empty and
/// the pages are the width Obsidian's are.
pub const SCROLLBAR_WIDTH: f32 = 12.0;

/// Pages this far outside the viewport are rendered too, so a wheel notch
/// lands on a page that is already there rather than an empty box.
const PREFETCH: f32 = 200.0;

/// The toolbar's metrics, observed on the open view's toolbar (`s03-toolbar`):
/// a button is 28 × 24 with its 16 px icon centred, 4 px apart, the first
/// 12 px in; a spacer is 12 px; the divider is a 1 × 16 hairline; the page
/// box is 52 × 30.
const BUTTON_WIDTH: f32 = 28.0;
const BUTTON_HEIGHT: f32 = 24.0;
const TOOLBAR_GAP: f32 = 4.0;
const TOOLBAR_PADDING: f32 = 12.0;
const SPACER: f32 = 12.0;
const PAGE_BOX_WIDTH: f32 = 52.0;
const PAGE_BOX_HEIGHT: f32 = 30.0;

/// What a click inside an embed opens, by vault path. The reading view hands
/// this over as an internal link, which is how every other embed opens its
/// target.
pub type OpenPath = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// What a toolbar button does.
type Click = Rc<dyn Fn(&mut Window, &mut App)>;

/// A `.pdf` embedded in a note: what the model carries. Nothing here has
/// read the file; the element resolves it through the document cache when
/// it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfEmbed {
    /// Unique within one model, for the element id the scroll offset is kept
    /// under: two embeds of the same file in one note scroll apart.
    pub id: usize,
    /// The file's vault path, which is what "Open" opens and what a change
    /// to the file invalidates by.
    pub path: String,
    pub absolute: PathBuf,
    /// The zero-based page `#page=` named, if any.
    pub page: Option<usize>,
    /// The height the link asked for, if any; the opened page's otherwise.
    pub height: Option<f32>,
}

impl PdfEmbed {
    /// The embed `link` (the link body without its alias, `x.pdf#page=3`)
    /// resolved to `path`, with `alias_height` the `|WxH` of the alias when
    /// one was written.
    ///
    /// The subpath is the text after the link's first `#`, read by
    /// `tft_pdf::parse_subpath` rather than the markdown parser, which
    /// takes `#page=3` for a heading. The height is `#height=` when the link
    /// has one, else the alias's explicit height, else nothing: `#height=`
    /// is the way Obsidian documents sizing a PDF embed, and the `|WxH`
    /// hint is the image form, honoured here so `![[x.pdf|300x200]]` is
    /// not simply ignored. OBSERVE: whether Obsidian reads the alias size on
    /// a PDF at all, and which wins when both are written (the probe wrote
    /// `|alias text` and `#page=3|alias`, never a size).
    pub fn new(
        id: usize,
        path: String,
        absolute: PathBuf,
        link: &str,
        alias_height: Option<f32>,
    ) -> PdfEmbed {
        let subpath = link.split_once('#').map(|(_, sub)| sub).unwrap_or("");
        let parsed = tft_pdf::parse_subpath(subpath);
        PdfEmbed {
            id,
            path,
            absolute,
            page: parsed.page,
            height: parsed.height.or(alias_height),
        }
    }

    /// The height of the embed's frame: the toolbar over the content, where
    /// the content is the height the link asked for, else the opened page
    /// fitted to `inner_width` (the frame's width less its borders) once the
    /// file's `sizes` are known, else nothing yet — and before the file has
    /// loaded, with no height asked for, the 800 px the box starts at.
    pub fn frame_height(&self, sizes: Option<&[PageSize]>, inner_width: f32) -> f32 {
        match (self.height, sizes) {
            (Some(h), _) => CHROME_HEIGHT + h.max(1.0),
            (None, Some(sizes)) => unforced_height(sizes, self.page, inner_width),
            (None, None) => DEFAULT_HEIGHT,
        }
    }

    /// The frame's height before the file has loaded.
    pub fn box_height(&self) -> f32 {
        self.frame_height(None, 0.0)
    }
}

/// Draws an embedded PDF inside a note: the embed's frame, at the embed's
/// height, with the toolbar and the pages inside it.
///
/// `available` is the width the note gives a block, which is what the frame
/// fills: an unforced height is the opened page's at that width, and has to
/// be decided before the frame is laid out. An embed nested in a quote or a
/// list is a little narrower than that, so its page ends a little above the
/// frame's bottom; the fit itself is to the real width.
pub fn render(embed: &PdfEmbed, theme: &Theme, available: f32, open: OpenPath) -> AnyElement {
    Frame {
        id: ElementId::from(("pdf-embed", embed.id)),
        embed: embed.clone(),
        theme: theme.clone(),
        available,
        open,
        div: None,
    }
    .into_any_element()
}

/// Draws a `.pdf` file card on a canvas: the toolbar and the pages filling
/// the card, fitted to its width, opened at `page`. The canvas draws the
/// frame; `scale` is the zoom, which the toolbar follows (the pages follow it
/// through the width).
pub fn render_card(
    absolute: &Path,
    path: &str,
    page: Option<usize>,
    node_id: &str,
    scale: f32,
    theme: &Theme,
    open: OpenPath,
) -> AnyElement {
    viewer(
        ElementId::Name(format!("card-pdf-{node_id}").into()),
        absolute,
        path,
        page,
        scale,
        theme,
        open,
    )
}

/// The viewer every drawing of a PDF shares: pdf.js's toolbar over the
/// pages, filling whatever bounds its parent gives it, opened at `page`,
/// keeping its scroll offset and zoom across frames under `id`.
///
/// It needs nothing from its caller but the file: the document comes from
/// the cache when it is painted, so it can be dropped into any element tree
/// — a note's paragraph flow, a canvas card, or a leaf. The open view draws
/// its own toolbar and pages today; this is the one to adopt when the two
/// are unified, and the module note records what of it is observed. `scale`
/// multiplies the toolbar's metrics (a canvas card's zoom; `1.0` elsewhere);
/// `open` is what "Open" does with the vault `path`.
pub fn viewer(
    id: ElementId,
    absolute: &Path,
    path: &str,
    page: Option<usize>,
    scale: f32,
    theme: &Theme,
    open: OpenPath,
) -> AnyElement {
    Pages {
        id,
        absolute: absolute.to_path_buf(),
        path: path.to_string(),
        page,
        theme: theme.clone(),
        scale,
        open,
        child: None,
    }
    .into_any_element()
}

/// Where each page sits in the scroll box at one zoom, in logical pixels
/// from the top-left of the content.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLayout {
    /// Logical pixels per point.
    pub scale: f32,
    pub pages: Vec<Bounds<f32>>,
    /// The content: at least the client width, so narrower pages centre,
    /// and as tall as the last page's bottom.
    pub content: gpui::Size<f32>,
}

impl PageLayout {
    /// pdf.js's `page-width` on page `target` of `sizes` in a client
    /// `width` wide, as logical pixels per point: the opened page fills the
    /// width, whatever the others measure.
    pub fn fit_scale(sizes: &[PageSize], target: Option<usize>, width: f32) -> f32 {
        let page = target
            .and_then(|p| sizes.get(p))
            .or_else(|| sizes.first())
            .copied()
            .unwrap_or(PageSize {
                width: 612.0,
                height: 792.0,
            });
        width.max(1.0) / page.width.max(1.0)
    }

    /// Lays `sizes` out at `scale` in a client `width` wide: each page at
    /// its own size, left-flush when it is the width and centred when it is
    /// narrower, [`PAGE_GAP`] between pages, nothing above the first or
    /// below the last.
    pub fn new(sizes: &[PageSize], scale: f32, width: f32) -> PageLayout {
        let widest = sizes.iter().map(|s| s.width * scale).fold(0.0, f32::max);
        let content_width = width.max(widest).max(1.0);
        let mut y = 0.0;
        let pages = sizes
            .iter()
            .enumerate()
            .map(|(ix, s)| {
                if ix > 0 {
                    y += PAGE_GAP;
                }
                let w = page_px(s.width, scale);
                let h = page_px(s.height, scale);
                let bounds = Bounds::new(
                    gpui::Point::new(((content_width - w) / 2.0).max(0.0), y),
                    gpui::Size::new(w, h),
                );
                y += h;
                bounds
            })
            .collect();
        PageLayout {
            scale,
            pages,
            content: gpui::Size::new(content_width, y),
        }
    }

    /// The scroll offset (distance from the top of the content) that puts
    /// page `page` flush under the toolbar; `None` for a page the file does
    /// not have.
    pub fn offset_of(&self, page: usize) -> Option<f32> {
        self.pages.get(page).map(|p| p.top().max(0.0))
    }

    /// The pages intersecting a viewport `height` tall scrolled `offset`
    /// down the content, widened by [`PREFETCH`] on both sides.
    pub fn visible(&self, offset: f32, height: f32) -> Range<usize> {
        let start = offset - PREFETCH;
        let end = offset + height + PREFETCH;
        let first = self
            .pages
            .iter()
            .position(|p| p.bottom() > start)
            .unwrap_or(self.pages.len());
        let last = self.pages[first..]
            .iter()
            .position(|p| p.top() >= end)
            .map_or(self.pages.len(), |n| first + n);
        first..last
    }

    /// The page the toolbar names: the one under the viewport's vertical
    /// midpoint, or the first below that when the midpoint is in a gap, or
    /// the last. The open view counts the same way.
    pub fn page_at(&self, offset: f32, height: f32) -> usize {
        let mid = offset + height / 2.0;
        self.pages
            .iter()
            .position(|p| p.bottom() >= mid)
            .unwrap_or(self.pages.len().saturating_sub(1))
    }
}

/// A page's drawn side: points times the scale, rounded down to 2 px the
/// way pdf.js's `--scale-round` does (observed: 590 × 416 for a page that
/// scales to 591 × 417.9).
fn page_px(points: f32, scale: f32) -> f32 {
    ((points * scale / 2.0).floor() * 2.0).max(1.0)
}

/// The height an unforced embed takes once its file is loaded: the frame's
/// two borders, the toolbar and its border, and the opened page at the
/// `page-width` scale of a client `inner_width` wide less the scrollbar.
/// Observed as 455.88 for a 416 px page and 873.84 for an 834 px one.
pub fn unforced_height(sizes: &[PageSize], target: Option<usize>, inner_width: f32) -> f32 {
    let client = (inner_width - SCROLLBAR_WIDTH).max(1.0);
    let scale = PageLayout::fit_scale(sizes, target, client);
    let page = target
        .and_then(|p| sizes.get(p))
        .or_else(|| sizes.first())
        .map_or(0.0, |s| page_px(s.height, scale));
    (CHROME_HEIGHT + page).max(1.0)
}

/// The scroll offset, the zoom and the one-time jump, kept across frames
/// under the element's id. Cloning shares all three: the handle is an `Rc`
/// inside, and so are the cells.
#[derive(Clone, Default)]
struct State {
    scroll: ScrollHandle,
    /// Whether the embed has been scrolled to its `#page=` once. The jump
    /// waits for the document (the page's top is unknown before it), and
    /// happens only once so the reader's own scrolling is not undone.
    jumped: Rc<Cell<bool>>,
    /// The zoom the toolbar's buttons set, in pdf.js's units (`1.0` is 100
    /// %); `None` is `page-width`, the zoom every embed opens at.
    zoom: Rc<Cell<Option<f32>>>,
}

/// The embed's frame: a `div` with the embed's border and height around
/// [`Pages`]. Its one job is deciding the height, which needs the document,
/// which needs an `App`, which `request_layout` has.
struct Frame {
    id: ElementId,
    embed: PdfEmbed,
    theme: Theme,
    available: f32,
    open: OpenPath,
    /// Built in `request_layout`, drawn through the rest of the frame.
    div: Option<Div>,
}

impl IntoElement for Frame {
    type Element = Frame;

    fn into_element(self) -> Frame {
        self
    }
}

impl Element for Frame {
    type RequestLayoutState = DivFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, DivFrameState) {
        // Before the file loads, and for a file that never will, the sizes
        // are unknown and the frame is the 800 px the box starts at.
        let sizes = match documents(cx).document(&self.embed.absolute, cx) {
            DocState::Ready(loaded) => Some(loaded),
            _ => None,
        };
        let height = self
            .embed
            .frame_height(sizes.as_ref().map(|l| &l.sizes[..]), self.available - 2.0);
        // Un-keyed: the id gpui hands this element is the one the div's
        // state is kept under, the way `Scaled` passes it through.
        let mut frame = div()
            .w_full()
            .h(px(height))
            .border_1()
            .border_color(self.theme.tokens.background_modifier_border)
            .rounded(px(4.))
            .overflow_hidden()
            .child(Pages {
                id: ElementId::from(("pdf-pages", self.embed.id)),
                absolute: self.embed.absolute.clone(),
                path: self.embed.path.clone(),
                page: self.embed.page,
                theme: self.theme.clone(),
                scale: 1.0,
                open: self.open.clone(),
                child: None,
            });
        let layout = frame.request_layout(id, inspector_id, window, cx);
        self.div = Some(frame);
        layout
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut DivFrameState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        self.div
            .as_mut()
            .expect("laid out before prepaint")
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut DivFrameState,
        prepaint: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.div.as_mut().expect("laid out before paint").paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        )
    }
}

/// The toolbar and the pages of one file, filling whatever bounds the
/// element is given. See the module note for why this is a custom element.
struct Pages {
    id: ElementId,
    absolute: PathBuf,
    /// The vault path, which is what "Open" opens.
    path: String,
    page: Option<usize>,
    theme: Theme,
    /// The canvas's zoom for a card, `1.0` in a note: the toolbar is drawn
    /// at this scale, the way Obsidian's CSS transform scales a card's.
    scale: f32,
    open: OpenPath,
    /// Built in `prepaint`, painted in `paint`.
    child: Option<AnyElement>,
}

/// What the toolbar shows, decided before it is built.
struct ToolbarState {
    /// The page the viewport is on, one-based, or 0 before the file loads.
    current: usize,
    count: usize,
    /// The zoom the next step starts from, in pdf.js's units.
    zoom: f32,
}

impl Pages {
    /// A toolbar button: pdf.js's `clickable-icon`, 28 × 24 with its 16 px
    /// icon centred, a wash on hover when it does something.
    fn button(
        &self,
        id: &'static str,
        name: &str,
        tip: &'static str,
        enabled: bool,
        on_click: Option<Click>,
    ) -> AnyElement {
        let s = self.scale;
        crate::ui::toolbar_button(
            id,
            name,
            tip,
            enabled,
            false,
            BUTTON_WIDTH * s,
            BUTTON_HEIGHT * s,
            ICON_SIZE * s,
            4. * s,
            &self.theme,
        )
        .when_some(on_click.filter(|_| enabled), |d, on_click| {
            d.on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_click(window, cx)
            })
        })
        .into_any_element()
    }

    /// pdf.js's toolbar as the embed carries it: sidebar toggle, sidebar
    /// options, spacer, zoom out, divider, zoom in, display options, spacer,
    /// the page box and "of N" on the left; "Open" on the right.
    ///
    /// OBSERVE: the sidebar and the two option menus are drawn and inert
    /// (what they open inside an embed was not probed), and the page box is
    /// drawn and not editable.
    fn toolbar(&self, state: &State, bar: ToolbarState) -> AnyElement {
        let s = self.scale;
        let theme = &self.theme;
        let ready = bar.count > 0;
        let spacer = || div().w(px(SPACER * s)).flex_shrink_0();
        let zoom_out: Click = {
            let zoom = state.zoom.clone();
            let from = bar.zoom;
            Rc::new(move |_, cx| {
                zoom.set(Some(step_out(from)));
                cx.refresh_windows();
            })
        };
        let zoom_in: Click = {
            let zoom = state.zoom.clone();
            let from = bar.zoom;
            Rc::new(move |_, cx| {
                zoom.set(Some(step_in(from)));
                cx.refresh_windows();
            })
        };
        let open: Click = {
            let open = self.open.clone();
            let path = self.path.clone();
            Rc::new(move |window, cx| open(&path, window, cx))
        };
        div()
            .id("pdf-embed-toolbar")
            .w_full()
            .h(px(TOOLBAR_HEIGHT * s))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(TOOLBAR_GAP * s))
            .px(px(TOOLBAR_PADDING * s))
            .bg(theme.tokens.background_primary)
            .border_b_1()
            .border_color(theme.tokens.background_modifier_border)
            .font_family(theme.ui_font.clone())
            .text_size(px(FONT_UI_SMALL * s))
            .text_color(theme.tokens.text_muted)
            .child(self.button(
                "pdf-embed-sidebar",
                "layout-list",
                "Toggle sidebar",
                false,
                None,
            ))
            .child(self.button(
                "pdf-embed-sidebar-options",
                "chevron-down",
                "Sidebar options",
                false,
                None,
            ))
            .child(spacer())
            .child(self.button(
                "pdf-embed-zoom-out",
                "zoom-out",
                "Zoom out",
                ready,
                Some(zoom_out),
            ))
            .child(
                div()
                    .w(px(1.))
                    .h(px(ICON_SIZE * s))
                    .flex_shrink_0()
                    .bg(theme.tokens.background_modifier_border),
            )
            .child(self.button(
                "pdf-embed-zoom-in",
                "zoom-in",
                "Zoom in",
                ready,
                Some(zoom_in),
            ))
            .child(self.button(
                "pdf-embed-display-options",
                "chevron-down",
                "Display options",
                false,
                None,
            ))
            .child(spacer())
            .child(
                div()
                    .w(px(PAGE_BOX_WIDTH * s))
                    .h(px(PAGE_BOX_HEIGHT * s))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px(px(8. * s))
                    .rounded(px(4. * s))
                    // `input.pdf-page-input`: a form field.
                    .border_1()
                    .border_color(theme.tokens.background_modifier_border)
                    .bg(theme.tokens.background_modifier_form_field)
                    .text_color(theme.tokens.text_normal)
                    .child(SharedString::from(bar.current.to_string())),
            )
            .child(
                div()
                    .whitespace_nowrap()
                    .child(SharedString::from(format!("of {}", bar.count))),
            )
            .child(div().flex_1())
            .child(self.button(
                "pdf-embed-open",
                "ellipsis-vertical",
                "Open",
                true,
                Some(open),
            ))
            .into_any_element()
    }

    /// The scroll box with every page in it, the visible ones as images,
    /// and what the toolbar should say about it.
    fn pages(
        &self,
        loaded: &Loaded,
        bounds: Bounds<Pixels>,
        state: &State,
        window: &Window,
        theming: Option<tft_pdf::Theming>,
    ) -> (AnyElement, ToolbarState) {
        let client_width = (f32::from(bounds.size.width) - SCROLLBAR_WIDTH * self.scale).max(1.0);
        let viewport_height =
            (f32::from(bounds.size.height) - (TOOLBAR_HEIGHT + 1.0) * self.scale).max(1.0);
        let fit = PageLayout::fit_scale(&loaded.sizes, self.page, client_width);
        // The zoom in pdf.js's units, so the buttons step the way its do.
        let zoom = state.zoom.get().unwrap_or(fit / CSS_UNITS);
        let layout = PageLayout::new(&loaded.sizes, zoom * CSS_UNITS, client_width);
        if !state.jumped.get() {
            if let Some(offset) = self.page.and_then(|p| layout.offset_of(p)) {
                // The div clamps it to the content once that is laid out.
                state.scroll.set_offset(point(px(0.), px(-offset)));
            }
            state.jumped.set(true);
        }
        let offset = -f32::from(state.scroll.offset().y);
        let visible = layout.visible(offset, viewport_height);
        let current = if layout.pages.is_empty() {
            0
        } else {
            layout.page_at(offset, viewport_height) + 1
        };
        let factor = window.scale_factor();
        let absolute = self.absolute.clone();
        // Every page is framed by `--pdf-shadow`, in an embed as in the view.
        let frame = crate::theme::box_shadows(&self.theme.tokens.pdf_shadow);
        let column = div()
            .relative()
            .w(px(layout.content.width))
            .h(px(layout.content.height))
            .flex_shrink_0()
            .children(layout.pages.iter().enumerate().map(|(ix, page)| {
                // A page is opaque white under its ink, so the box is white
                // before the render lands and the image covers it after.
                let page_box = div()
                    .absolute()
                    .left(px(page.origin.x))
                    .top(px(page.origin.y))
                    .w(px(page.size.width))
                    .h(px(page.size.height))
                    .bg(match theming {
                        Some(t) => gpui::rgb(
                            (u32::from(t.background[0]) << 16)
                                | (u32::from(t.background[1]) << 8)
                                | u32::from(t.background[2]),
                        )
                        .into(),
                        None => white(),
                    })
                    .shadow(frame.clone());
                if visible.contains(&ix) {
                    page_box.child(
                        img(page_source(
                            absolute.clone(),
                            ix,
                            layout.scale * factor,
                            theming,
                        ))
                        .w(px(page.size.width))
                        .h(px(page.size.height)),
                    )
                } else {
                    page_box
                }
            }));
        let element = div()
            .id("pdf-embed-scroll")
            .track_scroll(&state.scroll)
            .overflow_scroll()
            // The wheel scrolls the pages and nothing behind them: this
            // listener runs after the div's own has moved it (gpui runs the
            // bubble listeners innermost first, and the div's own is
            // registered after this one).
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .flex_1()
            .min_h_0()
            .w_full()
            // `.pdf-container`: `--pdf-background` (probe of 2026-11-04).
            .bg(self.theme.tokens.pdf_background)
            .child(column)
            .into_any_element();
        (
            element,
            ToolbarState {
                current,
                count: layout.pages.len(),
                zoom,
            },
        )
    }
}

impl IntoElement for Pages {
    type Element = Pages;

    fn into_element(self) -> Pages {
        self
    }
}

impl Element for Pages {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        // The box is whatever its parent gives it; the pages are laid out
        // inside those bounds in `prepaint`, once they are known.
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let id = id.expect("`Pages` has an element id");
        let state: State = window.with_element_state(id, |state, _| {
            let state: State = state.unwrap_or_default();
            (state.clone(), state)
        });
        let theming = super::theming(cx);
        let (body, bar) = match documents(cx).document(&self.absolute, cx) {
            DocState::Ready(loaded) => self.pages(&loaded, bounds, &state, window, theming),
            // Observed on `broken.pdf`: the toolbar says "0 of 0" over an
            // empty box, and nothing else. OBSERVE: whether a loading file
            // looks the same; it does here.
            DocState::Loading | DocState::Failed(_) | DocState::Locked { .. } => (
                div().flex_1().min_h_0().w_full().into_any_element(),
                ToolbarState {
                    current: 0,
                    count: 0,
                    zoom: 1.0,
                },
            ),
        };
        let mut child = div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.toolbar(&state, bar))
            .child(body)
            .into_any_element();
        child.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        child.prepaint_at(bounds.origin, window, cx);
        self.child = Some(child);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = self.child.as_mut() {
            child.paint(window, cx);
        }
    }
}

/// Every embedded PDF in a built note, in document order, as the control
/// socket reports them: an embed's pages are images and its box has no
/// title, so this is how `![[x.pdf]]` is checked from outside the app.
pub fn describe_all(blocks: &[RBlock]) -> Vec<Json> {
    let mut out = Vec::new();
    each_embed(blocks, &mut |embed| {
        if let EmbedKind::Pdf(pdf) = &embed.kind {
            out.push(json!({
                "path": pdf.path,
                "page": pdf.page,
                "height": pdf.height,
                "box_height": pdf.box_height(),
                "link": embed.link,
            }));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(n: usize) -> Vec<PageSize> {
        vec![
            PageSize {
                width: 612.0,
                height: 792.0
            };
            n
        ]
    }

    /// The probe's `doc.pdf`: A4, landscape first (rotated), page 3
    /// portrait.
    fn mixed() -> Vec<PageSize> {
        let landscape = PageSize {
            width: 841.8898,
            height: 595.2756,
        };
        let portrait = PageSize {
            width: 595.2756,
            height: 841.8898,
        };
        vec![landscape, landscape, portrait, landscape, landscape]
    }

    #[test]
    fn the_subpath_is_read_as_a_pdf_subpath_not_a_heading() {
        let e = PdfEmbed::new(
            0,
            "Field.pdf".into(),
            "/v/Field.pdf".into(),
            "Field.pdf#page=3&height=400",
            None,
        );
        assert_eq!(e.page, Some(2));
        assert_eq!(e.height, Some(400.0));
        // Observed: `#height=300` drew 338 px, the toolbar over the content.
        assert_eq!(e.box_height(), CHROME_HEIGHT + 400.0);
        assert_eq!(
            e.frame_height(Some(&letter(3)), 600.0),
            CHROME_HEIGHT + 400.0
        );
        let plain = PdfEmbed::new(
            0,
            "Field.pdf".into(),
            "/v/Field.pdf".into(),
            "Field.pdf",
            None,
        );
        assert_eq!((plain.page, plain.height), (None, None));
        assert_eq!(plain.box_height(), DEFAULT_HEIGHT);
    }

    #[test]
    fn the_link_height_wins_over_the_alias_and_the_alias_over_the_default() {
        let both = PdfEmbed::new(
            0,
            "F.pdf".into(),
            "/v/F.pdf".into(),
            "F.pdf#height=200",
            Some(300.0),
        );
        assert_eq!(both.box_height(), CHROME_HEIGHT + 200.0);
        let alias = PdfEmbed::new(0, "F.pdf".into(), "/v/F.pdf".into(), "F.pdf", Some(300.0));
        assert_eq!(alias.box_height(), CHROME_HEIGHT + 300.0);
    }

    /// `s08-embeds`, preview 9: `![[doc.pdf]]` in a 604.43 px box, client
    /// 591, scale `page-width` 0.5265 on the landscape first page.
    #[test]
    fn page_width_is_measured_on_the_opened_page() {
        let scale = PageLayout::fit_scale(&mixed(), None, 591.0) / CSS_UNITS;
        assert!((scale - 0.526494).abs() < 1e-4, "{scale}");
        // Preview 10: `#page=3` fits the portrait page instead, 0.7446.
        let scale = PageLayout::fit_scale(&mixed(), Some(2), 591.0) / CSS_UNITS;
        assert!((scale - 0.744613).abs() < 1e-4, "{scale}");
        // A page the file does not have fits the first.
        assert_eq!(
            PageLayout::fit_scale(&mixed(), Some(9), 591.0),
            PageLayout::fit_scale(&mixed(), None, 591.0)
        );
    }

    /// The same preview: page 1 at the top, page 2 ten pixels under it,
    /// page 3 (narrower at this scale) centred, the content ending at the
    /// last page's bottom — 2321 px in Obsidian.
    #[test]
    fn pages_stack_flush_at_the_top_with_the_gap_between_them() {
        let sizes = mixed();
        let scale = PageLayout::fit_scale(&sizes, None, 591.0);
        let layout = PageLayout::new(&sizes, scale, 591.0);
        assert_eq!(layout.pages.len(), 5);
        assert_eq!(layout.pages[0].top(), 0.0);
        // Half a pixel in: a 590 px page centred in 591 (Obsidian's sat 0.66
        // from the border).
        assert_eq!(layout.pages[0].left(), 0.5);
        // 591 × 417.9 at the scale, drawn 590 × 416 (rounded down to 2 px).
        assert_eq!(layout.pages[0].size.width, 590.0);
        let first_h = layout.pages[0].size.height;
        assert_eq!(first_h, 416.0);
        assert_eq!(layout.pages[1].top(), first_h + PAGE_GAP);
        let third = layout.pages[2];
        assert!(third.size.width < 591.0);
        assert!((third.left() - (591.0 - third.size.width) / 2.0).abs() < 1e-3);
        assert_eq!(layout.content.width, 591.0);
        // 416 + 10 + 416 + 10 + 590 + 10 + 416 + 10 + 416; Obsidian measured
        // 2321, its rects carrying pdf.js's page borders.
        assert_eq!(layout.content.height, 2294.0);
        assert_eq!(layout.content.height, layout.pages[4].bottom());
    }

    #[test]
    fn a_zoomed_file_is_wider_than_the_box_and_still_left_flush() {
        let sizes = letter(2);
        let layout = PageLayout::new(&sizes, 2.0, 400.0);
        assert_eq!(layout.content.width, 1224.0);
        assert_eq!(layout.pages[0].left(), 0.0);
        assert_eq!(layout.pages[1].top(), 1584.0 + PAGE_GAP);
    }

    #[test]
    fn the_page_offset_puts_the_page_flush_under_the_toolbar() {
        let layout = PageLayout::new(&letter(3), 1.0, 612.0);
        assert_eq!(layout.offset_of(0), Some(0.0));
        assert_eq!(layout.offset_of(1), Some(792.0 + PAGE_GAP));
        assert_eq!(layout.offset_of(3), None);
    }

    #[test]
    fn only_the_pages_near_the_viewport_are_visible() {
        let layout = PageLayout::new(&letter(10), 1.0, 612.0);
        // At the top, a 400 px viewport sees page 0 and prefetches nothing
        // past the first page that starts beyond it plus the margin.
        assert_eq!(layout.visible(0.0, 400.0), 0..1);
        // Scrolled to page 5, with the prefetch reaching the page before.
        let at_five = layout.offset_of(5).unwrap();
        assert_eq!(layout.visible(at_five, 400.0), 4..6);
        // Past the end, nothing.
        assert_eq!(
            layout.visible(layout.content.height + 1000.0, 400.0),
            10..10
        );
        // An empty file has no pages to show.
        assert_eq!(PageLayout::new(&[], 1.0, 300.0).visible(0.0, 400.0), 0..0);
    }

    #[test]
    fn the_toolbar_names_the_page_under_the_viewports_midpoint() {
        let layout = PageLayout::new(&letter(3), 1.0, 612.0);
        assert_eq!(layout.page_at(0.0, 400.0), 0);
        assert_eq!(layout.page_at(700.0, 400.0), 1);
        assert_eq!(layout.page_at(5000.0, 400.0), 2);
    }

    /// `s08-embeds`: 455.88 px for the landscape first page in a 604.43 px
    /// box, 873.84 for `#page=3`; the toolbar's hairline is drawn as a whole
    /// pixel here, so each is a fraction over.
    #[test]
    fn an_unforced_embed_is_as_tall_as_its_opened_page_under_the_toolbar() {
        let inner = 604.43 - 1.38;
        let first = unforced_height(&mixed(), None, inner);
        assert!((first - 455.88).abs() < 1.5, "{first}");
        let third = unforced_height(&mixed(), Some(2), inner);
        assert!((third - 873.84).abs() < 1.5, "{third}");
        // An empty file is the toolbar alone.
        assert_eq!(unforced_height(&[], None, inner), CHROME_HEIGHT);
        // An unforced embed is 800 px until the sizes are known, then the
        // page's; through the embed itself.
        let e = PdfEmbed::new(
            0,
            "doc.pdf".into(),
            "/v/doc.pdf".into(),
            "doc.pdf#page=3",
            None,
        );
        assert_eq!(e.box_height(), DEFAULT_HEIGHT);
        assert_eq!(e.frame_height(Some(&mixed()), inner), third);
    }

    #[test]
    fn a_zero_width_box_still_lays_out() {
        let layout = PageLayout::new(
            &letter(1),
            PageLayout::fit_scale(&letter(1), None, 0.0),
            0.0,
        );
        assert!(layout.pages[0].size.width >= 1.0);
        assert!(layout.pages[0].size.height >= 1.0);
    }
}
