//! Terminal rendering module.
//!
//! This module provides [`TerminalRenderer`], which handles efficient rendering of
//! terminal content using GPUI's text and drawing systems.
//!
//! # Rendering Pipeline
//!
//! The renderer processes the terminal grid in several stages:
//!
//! ```text
//! Terminal Grid → Layout Phase → Paint Phase
//!                      │              │
//!                      ├─ Collect backgrounds
//!                      ├─ Batch text runs
//!                      │              │
//!                      │              ├─ Paint default background
//!                      │              ├─ Paint non-default backgrounds
//!                      │              ├─ Paint text characters
//!                      │              └─ Paint cursor
//! ```
//!
//! # Optimizations
//!
//! The renderer includes several optimizations to minimize draw calls:
//!
//! 1. **Background Merging**: Adjacent cells with the same background color are
//!    merged into single rectangles, reducing the number of quads to paint.
//!
//! 2. **Text Batching**: Adjacent cells with identical styling (color, bold, italic)
//!    are grouped into [`BatchedTextRun`]s for efficient text shaping.
//!
//! 3. **Default Background Skip**: Cells with the default background color don't
//!    generate separate background rectangles.
//!
//! 4. **Cell Measurement**: Font metrics are measured once using the 'M' character
//!    and cached for consistent cell dimensions.
//!
//! # Cell Dimensions
//!
//! Cell size is calculated from actual font metrics:
//!
//! - **Width**: Measured from shaped 'M' character (typically widest in monospace)
//! - **Height**: `(ascent + descent) × line_height_multiplier`
//!
//! The `line_height_multiplier` (default 1.2) adds extra vertical space to
//! accommodate tall glyphs from nerd fonts and other icon fonts.
//!
//! # Example
//!
//! ```ignore
//! use gpui::px;
//! use gpui_terminal::{ColorPalette, TerminalRenderer};
//!
//! let renderer = TerminalRenderer::new(
//!     "JetBrains Mono".to_string(),
//!     px(14.0),
//!     1.2,  // line height multiplier
//!     ColorPalette::default(),
//! );
//! ```

use crate::colors::ColorPalette;
use crate::event::GpuiEventProxy;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape};
use gpui::{
    App, Bounds, Edges, Font, FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Point,
    SharedString, Size, TextAlign, TextRun, UnderlineStyle, Window, px, quad, transparent_black,
};

/// One search hit on the grid: a span of cells on a content line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub line: i32,
    pub start_col: usize,
    pub end_col: usize,
}

/// A batched run of text with consistent styling.
///
/// This struct groups adjacent terminal cells with identical visual attributes
/// to reduce the number of text rendering calls.
#[derive(Debug, Clone)]
pub struct BatchedTextRun {
    /// The text content to render
    pub text: String,

    /// Starting column position
    pub start_col: usize,

    /// Row position
    pub row: usize,

    /// Foreground color
    pub fg_color: Hsla,

    /// Background color
    pub bg_color: Hsla,

    /// Bold flag
    pub bold: bool,

    /// Italic flag
    pub italic: bool,

    /// Underline flag
    pub underline: bool,
}

/// Background rectangle to paint.
///
/// Represents a rectangular region with a solid color background.
#[derive(Debug, Clone)]
pub struct BackgroundRect {
    /// Starting column position
    pub start_col: usize,

    /// Ending column position (exclusive)
    pub end_col: usize,

    /// Row position
    pub row: usize,

    /// Background color
    pub color: Hsla,
}

impl BackgroundRect {
    /// Check if this rectangle can be merged with another.
    ///
    /// Two rectangles can be merged if they:
    /// - Are on the same row
    /// - Have the same color
    /// - Are horizontally adjacent
    fn can_merge_with(&self, other: &Self) -> bool {
        self.row == other.row && self.color == other.color && self.end_col == other.start_col
    }
}

/// Terminal renderer with font settings and cell dimensions.
///
/// This struct manages the rendering of terminal content, including text,
/// backgrounds, and cursor. It maintains font metrics and provides the
/// [`paint`](Self::paint) method for drawing the terminal grid.
///
/// # Font Metrics
///
/// Cell dimensions are calculated from actual font measurements via
/// [`measure_cell`](Self::measure_cell). This ensures accurate character
/// positioning regardless of the font used.
///
/// # Usage
///
/// The renderer is typically used internally by [`TerminalView`](crate::TerminalView),
/// but can also be used directly for custom rendering:
///
/// ```ignore
/// // Measure cell dimensions (call once per font change)
/// renderer.measure_cell(window);
///
/// // Paint the terminal grid
/// renderer.paint(bounds, padding, &term, window, cx);
/// ```
///
/// # Performance
///
/// For optimal performance:
/// - Call `measure_cell` only when font settings change
/// - The `paint` method is designed to be called every frame
/// - Background and text batching minimize GPU draw calls
#[derive(Clone)]
pub struct TerminalRenderer {
    /// Font family name (e.g., "Fira Code", "Menlo")
    pub font_family: String,

    /// Font size in pixels
    pub font_size: Pixels,

    /// Width of a single character cell
    pub cell_width: Pixels,

    /// Height of a single character cell (line height)
    pub cell_height: Pixels,

    /// Multiplier for line height to accommodate tall glyphs
    pub line_height_multiplier: f32,

    /// Color palette for resolving terminal colors
    pub palette: ColorPalette,
}

/// What a paint pass draws besides the grid itself: search hits, the
/// hovered link, the cursor, and the scrollbar.
pub struct PaintOptions<'a> {
    pub search: Option<(&'a [SearchMatch], usize)>,
    pub hover: Option<SearchMatch>,
    pub cursor_visible: bool,
    pub scrollbar_color: Hsla,
}

impl Default for PaintOptions<'_> {
    fn default() -> Self {
        Self {
            search: None,
            hover: None,
            cursor_visible: true,
            scrollbar_color: transparent_black(),
        }
    }
}

impl TerminalRenderer {
    /// Creates a new terminal renderer with the given font settings and color palette.
    ///
    /// # Arguments
    ///
    /// * `font_family` - The name of the font family to use
    /// * `font_size` - The font size in pixels
    /// * `line_height_multiplier` - Multiplier for line height (e.g., 1.2 for 20% extra)
    /// * `palette` - The color palette to use for terminal colors
    ///
    /// # Returns
    ///
    /// A new `TerminalRenderer` instance with default cell dimensions.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::px;
    /// use gpui_terminal::render::TerminalRenderer;
    /// use gpui_terminal::ColorPalette;
    ///
    /// let renderer = TerminalRenderer::new("Fira Code".to_string(), px(14.0), 1.2, ColorPalette::default());
    /// ```
    pub fn new(
        font_family: String,
        font_size: Pixels,
        line_height_multiplier: f32,
        palette: ColorPalette,
    ) -> Self {
        // Default cell dimensions - will be measured on first paint
        // Using 0.6 as approximate em-width ratio for monospace fonts
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.4; // Line height with some spacing

        Self {
            font_family,
            font_size,
            cell_width,
            cell_height,
            line_height_multiplier,
            palette,
        }
    }

    /// Measure cell dimensions based on actual font metrics.
    ///
    /// This method measures the actual width and height of characters
    /// using the GPUI text system.
    ///
    /// # Arguments
    ///
    /// * `window` - The GPUI window for text system access
    pub fn measure_cell(&mut self, window: &mut Window) {
        // Measure using a reference character (M is typically the widest)
        let font = Font {
            family: self.font_family.clone().into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };

        let text_run = TextRun {
            len: 1,
            font,
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        // Shape a single 'M' character to get its metrics
        let shaped = window
            .text_system()
            .shape_line("M".into(), self.font_size, &[text_run], None);

        // Get the width from the shaped line (accessed via Deref to LineLayout)
        if shaped.width > px(0.0) {
            self.cell_width = shaped.width;
        }

        // Calculate height from ascent + descent with multiplier for tall glyphs (nerd fonts, etc.)
        let line_height = shaped.ascent + shaped.descent;
        if line_height > px(0.0) {
            self.cell_height = line_height * self.line_height_multiplier;
        }
    }

    /// A cell's foreground and background with reverse video applied.
    fn cell_colors(&self, cell: &Cell, colors: &Colors) -> (Hsla, Hsla) {
        let mut fg = self.palette.resolve(cell.fg, colors);
        let mut bg = self.palette.resolve(cell.bg, colors);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        (fg, bg)
    }

    /// Layout cells into batched text runs and background rects for a single row.
    ///
    /// This method processes a row of terminal cells and groups adjacent cells
    /// with identical styling into batched runs. It also collects background
    /// rectangles that need to be painted.
    ///
    /// # Arguments
    ///
    /// * `row` - The row number
    /// * `cells` - Iterator over (column, Cell) pairs
    /// * `colors` - Terminal color configuration
    ///
    /// # Returns
    ///
    /// A tuple of `(backgrounds, text_runs)` where:
    ///
    /// - `backgrounds` is a vector of merged background rectangles
    /// - `text_runs` is a vector of batched text runs
    pub fn layout_row(
        &self,
        row: usize,
        cells: impl Iterator<Item = (usize, Cell)>,
        colors: &Colors,
    ) -> (Vec<BackgroundRect>, Vec<BatchedTextRun>) {
        let mut backgrounds = Vec::new();
        let mut text_runs = Vec::new();

        let mut current_run: Option<BatchedTextRun> = None;
        let mut current_bg: Option<BackgroundRect> = None;

        for (col, cell) in cells {
            let (fg_color, bg_color) = self.cell_colors(&cell, colors);

            // A wide character's second cell is a spacer with no text, but
            // it still carries half of the character's background. Skipping
            // it leaves a one-cell hole after every CJK glyph.
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                match current_bg.as_mut() {
                    Some(bg_rect) if bg_rect.color == bg_color && bg_rect.end_col == col => {
                        bg_rect.end_col = col + 1;
                    }
                    _ => {
                        if let Some(bg_rect) = current_bg.take() {
                            backgrounds.push(bg_rect);
                        }
                        current_bg = Some(BackgroundRect {
                            start_col: col,
                            end_col: col + 1,
                            row,
                            color: bg_color,
                        });
                    }
                }
                continue;
            }
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let underline = cell.flags.contains(Flags::UNDERLINE);

            // Get the character (or space if empty)
            let ch = if cell.c == ' ' || cell.c == '\0' {
                ' '
            } else {
                cell.c
            };

            // Handle background rectangles
            if let Some(ref mut bg_rect) = current_bg {
                if bg_rect.color == bg_color && bg_rect.end_col == col {
                    // Extend current background
                    bg_rect.end_col = col + 1;
                } else {
                    // Save current background and start new one
                    backgrounds.push(bg_rect.clone());
                    current_bg = Some(BackgroundRect {
                        start_col: col,
                        end_col: col + 1,
                        row,
                        color: bg_color,
                    });
                }
            } else {
                // Start new background
                current_bg = Some(BackgroundRect {
                    start_col: col,
                    end_col: col + 1,
                    row,
                    color: bg_color,
                });
            }

            // Handle text runs
            if let Some(ref mut run) = current_run {
                if run.fg_color == fg_color
                    && run.bg_color == bg_color
                    && run.bold == bold
                    && run.italic == italic
                    && run.underline == underline
                {
                    // Extend current run
                    run.text.push(ch);
                } else {
                    // Save current run and start new one
                    text_runs.push(run.clone());
                    current_run = Some(BatchedTextRun {
                        text: ch.to_string(),
                        start_col: col,
                        row,
                        fg_color,
                        bg_color,
                        bold,
                        italic,
                        underline,
                    });
                }
            } else {
                // Start new run
                current_run = Some(BatchedTextRun {
                    text: ch.to_string(),
                    start_col: col,
                    row,
                    fg_color,
                    bg_color,
                    bold,
                    italic,
                    underline,
                });
            }
        }

        // Push final run and background
        if let Some(run) = current_run {
            text_runs.push(run);
        }
        if let Some(bg) = current_bg {
            backgrounds.push(bg);
        }

        // Merge adjacent backgrounds with same color
        let merged_backgrounds = self.merge_backgrounds(backgrounds);

        (merged_backgrounds, text_runs)
    }

    /// Merge adjacent background rects with same color.
    ///
    /// This optimization reduces the number of rectangles to paint by
    /// combining horizontally adjacent rectangles that share the same color.
    ///
    /// # Arguments
    ///
    /// * `rects` - Vector of background rectangles to merge
    ///
    /// # Returns
    ///
    /// A new vector with merged rectangles
    fn merge_backgrounds(&self, mut rects: Vec<BackgroundRect>) -> Vec<BackgroundRect> {
        if rects.is_empty() {
            return rects;
        }

        let mut merged = Vec::new();
        let mut current = rects.remove(0);

        for rect in rects {
            if current.can_merge_with(&rect) {
                current.end_col = rect.end_col;
            } else {
                merged.push(current);
                current = rect;
            }
        }

        merged.push(current);
        merged
    }

    /// Paint terminal content to the window.
    ///
    /// This is the main rendering method that draws the terminal grid,
    /// including backgrounds, text, and cursor.
    ///
    /// # Arguments
    ///
    /// * `bounds` - The bounding box to render within
    /// * `padding` - Padding around the terminal content
    /// * `term` - The terminal state
    /// * `window` - The GPUI window
    /// * `cx` - The application context
    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        term: &Term<GpuiEventProxy>,
        options: PaintOptions<'_>,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let PaintOptions {
            search,
            hover,
            cursor_visible,
            scrollbar_color,
        } = options;
        // Get terminal dimensions
        let grid = term.grid();
        let display_offset = grid.display_offset() as i32;
        let selection = term.selection.as_ref().and_then(|s| s.to_range(term));
        let num_lines = grid.screen_lines();
        let num_cols = grid.columns();
        let colors = term.colors();

        // Calculate default background color
        let default_bg = self.palette.resolve(
            Color::Named(alacritty_terminal::vte::ansi::NamedColor::Background),
            colors,
        );

        // Paint default background (covers full bounds including padding)
        window.paint_quad(quad(
            bounds,
            px(0.0),
            default_bg,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));

        // Calculate origin offset (content starts after padding)
        let origin = Point {
            x: bounds.origin.x + padding.left,
            y: bounds.origin.y + padding.top,
        };

        // Iterate over visible lines
        for line_idx in 0..num_lines {
            // A scrolled viewport shows history lines above row 0.
            let line = Line(line_idx as i32 - display_offset);

            // Collect cells for this line
            let cells: Vec<(usize, Cell)> = (0..num_cols)
                .map(|col_idx| {
                    let col = Column(col_idx);
                    let point = AlacPoint::new(line, col);
                    let cell = grid[point].clone();
                    (col_idx, cell)
                })
                .collect();

            // Layout the row for backgrounds
            let (backgrounds, _) = self.layout_row(line_idx, cells.iter().cloned(), colors);

            // Paint backgrounds
            for bg_rect in backgrounds {
                // Skip if it's the default background color
                if bg_rect.color == default_bg {
                    continue;
                }

                let x = origin.x + self.cell_width * (bg_rect.start_col as f32);
                let y = origin.y + self.cell_height * (bg_rect.row as f32);
                let width = self.cell_width * ((bg_rect.end_col - bg_rect.start_col) as f32);
                let height = self.cell_height;

                let rect_bounds = Bounds {
                    origin: Point { x, y },
                    size: Size { width, height },
                };

                window.paint_quad(quad(
                    rect_bounds,
                    px(0.0),
                    bg_rect.color,
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            }

            // The selection range, as a wash over the cells it covers.
            if let Some(selection) = &selection
                && line >= selection.start.line
                && line <= selection.end.line
            {
                let start_col = if line == selection.start.line {
                    selection.start.column.0
                } else {
                    0
                };
                let end_col = if line == selection.end.line {
                    selection.end.column.0
                } else {
                    num_cols.saturating_sub(1)
                };
                if end_col >= start_col {
                    let x = origin.x + self.cell_width * (start_col as f32);
                    let y = origin.y + self.cell_height * (line_idx as f32);
                    let width = self.cell_width * ((end_col - start_col + 1) as f32);
                    window.paint_quad(quad(
                        Bounds {
                            origin: Point { x, y },
                            size: Size {
                                width,
                                height: self.cell_height,
                            },
                        },
                        px(0.0),
                        self.palette.cursor().opacity(0.3),
                        Edges::<Pixels>::default(),
                        transparent_black(),
                        Default::default(),
                    ));
                }
            }

            // Search hits: a yellow wash, the current hit brighter.
            if let Some((matches, current)) = search {
                for (ix, hit) in matches.iter().enumerate() {
                    if hit.line != line.0 {
                        continue;
                    }
                    let color = if ix == current {
                        Hsla {
                            h: 0.13,
                            s: 1.0,
                            l: 0.55,
                            a: 0.55,
                        }
                    } else {
                        Hsla {
                            h: 0.13,
                            s: 1.0,
                            l: 0.55,
                            a: 0.28,
                        }
                    };
                    let x = origin.x + self.cell_width * (hit.start_col as f32);
                    let y = origin.y + self.cell_height * (line_idx as f32);
                    let width = self.cell_width * ((hit.end_col - hit.start_col) as f32);
                    window.paint_quad(quad(
                        Bounds {
                            origin: Point { x, y },
                            size: Size {
                                width,
                                height: self.cell_height,
                            },
                        },
                        px(0.0),
                        color,
                        Edges::<Pixels>::default(),
                        transparent_black(),
                        Default::default(),
                    ));
                }
            }

            // The hovered link gets an underline in the accent colour, the
            // pointer's sign that the token opens.
            if let Some(hit) = hover
                && hit.line == line.0
            {
                let x = origin.x + self.cell_width * (hit.start_col as f32);
                let y = origin.y + self.cell_height * (line_idx as f32 + 1.0) - px(1.5);
                let width = self.cell_width * ((hit.end_col - hit.start_col) as f32);
                window.paint_quad(quad(
                    Bounds {
                        origin: Point { x, y },
                        size: Size {
                            width,
                            height: px(1.5),
                        },
                    },
                    px(0.0),
                    self.palette.cursor(),
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            }

            // Calculate vertical offset to center text in cell
            // The multiplier adds extra height; we want to distribute it evenly top/bottom
            let base_height = self.cell_height / self.line_height_multiplier;
            let vertical_offset = (self.cell_height - base_height) / 2.0;

            // Paint each character individually at exact cell positions
            // This ensures perfect alignment for terminal emulation
            for (col_idx, cell) in cells {
                let ch = cell.c;

                let flags = cell.flags;

                // The second cell of a wide character is a placeholder.
                if flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
                    || flags
                        .contains(alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }

                // Skip empty cells (space or null)
                if ch == ' ' || ch == '\0' {
                    continue;
                }

                let x = origin.x + self.cell_width * (col_idx as f32);
                let y = origin.y + self.cell_height * (line_idx as f32) + vertical_offset;

                // Get cell colours (reverse video swaps the two).
                let (fg_color, _) = self.cell_colors(&cell, colors);

                let bold = flags.contains(alacritty_terminal::term::cell::Flags::BOLD);
                let italic = flags.contains(alacritty_terminal::term::cell::Flags::ITALIC);
                let underline = flags.contains(alacritty_terminal::term::cell::Flags::UNDERLINE);

                // Create font with styling
                let font = Font {
                    family: self.font_family.clone().into(),
                    features: FontFeatures::default(),
                    fallbacks: None,
                    weight: if bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    style: if italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    },
                };

                // Create text run for this single character
                let char_str = ch.to_string();
                let text_run = TextRun {
                    len: char_str.len(),
                    font,
                    color: fg_color,
                    background_color: None,
                    underline: if underline {
                        Some(UnderlineStyle {
                            thickness: px(1.0),
                            color: Some(fg_color),
                            wavy: false,
                        })
                    } else {
                        None
                    },
                    strikethrough: None,
                };

                // Shape and paint the character at its cell. Zed renders the
                // font's own glyph at the configured size; a wide (CJK or
                // emoji) glyph keeps its natural advance rather than being
                // enlarged, so Korean text is the same size as Latin text.
                let text: SharedString = char_str.into();
                let shaped_line = window.text_system().shape_line(
                    text,
                    self.font_size,
                    std::slice::from_ref(&text_run),
                    None,
                );
                let _ = shaped_line.paint(
                    Point { x, y },
                    self.cell_height,
                    TextAlign::Left,
                    None,
                    window,
                    _cx,
                );
            }
        }

        // Paint cursor. Scrolled into history, the cursor's line is not on
        // screen; it comes back when the viewport returns to the bottom.
        if display_offset == 0 && cursor_visible && term.mode().contains(TermMode::SHOW_CURSOR) {
            let cursor_point = grid.cursor.point;
            let cursor_x = origin.x + self.cell_width * (cursor_point.column.0 as f32);
            let cursor_y = origin.y + self.cell_height * (cursor_point.line.0 as f32);

            let cursor_color = self.palette.resolve(
                Color::Named(alacritty_terminal::vte::ansi::NamedColor::Cursor),
                colors,
            );

            let full = Bounds {
                origin: Point {
                    x: cursor_x,
                    y: cursor_y,
                },
                size: Size {
                    width: self.cell_width,
                    height: self.cell_height,
                },
            };
            // The shape the application chose with DECSCUSR: a block, a
            // beam at the left edge, or an underline; a hollow block gets
            // four thin edges instead of a filled cell.
            let filled: Option<(Bounds<Pixels>, Pixels)> = match term.cursor_style().shape {
                CursorShape::Hidden => None,
                CursorShape::Beam => Some((
                    Bounds {
                        origin: full.origin,
                        size: Size {
                            width: px(2.),
                            height: full.size.height,
                        },
                    },
                    px(0.),
                )),
                CursorShape::Underline => Some((
                    Bounds {
                        origin: Point {
                            x: full.origin.x,
                            y: full.origin.y + full.size.height - px(2.),
                        },
                        size: Size {
                            width: full.size.width,
                            height: px(2.),
                        },
                    },
                    px(0.),
                )),
                CursorShape::HollowBlock => {
                    let edge = px(1.);
                    for bounds in [
                        Bounds {
                            origin: full.origin,
                            size: Size {
                                width: full.size.width,
                                height: edge,
                            },
                        },
                        Bounds {
                            origin: Point {
                                x: full.origin.x,
                                y: full.origin.y + full.size.height - edge,
                            },
                            size: Size {
                                width: full.size.width,
                                height: edge,
                            },
                        },
                        Bounds {
                            origin: full.origin,
                            size: Size {
                                width: edge,
                                height: full.size.height,
                            },
                        },
                        Bounds {
                            origin: Point {
                                x: full.origin.x + full.size.width - edge,
                                y: full.origin.y,
                            },
                            size: Size {
                                width: edge,
                                height: full.size.height,
                            },
                        },
                    ] {
                        window.paint_quad(quad(
                            bounds,
                            px(0.0),
                            cursor_color,
                            Edges::<Pixels>::default(),
                            transparent_black(),
                            Default::default(),
                        ));
                    }
                    None
                }
                CursorShape::Block => Some((full, px(0.))),
            };

            if let Some((cursor_bounds, radius)) = filled {
                window.paint_quad(quad(
                    cursor_bounds,
                    radius,
                    cursor_color,
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            }
        }

        // Scrollbar: the visible screen as a thumb over the whole buffer.
        let history = grid.history_size();
        if history > 0 && num_lines > 0 {
            let track = Bounds {
                origin: Point {
                    x: bounds.origin.x + bounds.size.width - px(6.),
                    y: origin.y,
                },
                size: Size {
                    width: px(4.),
                    height: bounds.size.height - padding.top - padding.bottom,
                },
            };
            let total = (history + num_lines) as f32;
            let thumb_height = (track.size.height * (num_lines as f32 / total)).max(px(24.));
            let travel = track.size.height - thumb_height;
            let top = 1.0 - display_offset as f32 / history as f32;
            window.paint_quad(quad(
                Bounds {
                    origin: Point {
                        x: track.origin.x,
                        y: track.origin.y + travel * top,
                    },
                    size: Size {
                        width: track.size.width,
                        height: thumb_height,
                    },
                },
                px(2.),
                scrollbar_color,
                Edges::<Pixels>::default(),
                transparent_black(),
                Default::default(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renderer_creation() {
        let renderer = TerminalRenderer::new(
            "Fira Code".to_string(),
            px(14.0),
            1.2,
            ColorPalette::default(),
        );
        assert_eq!(renderer.font_family, "Fira Code");
        assert_eq!(renderer.font_size, px(14.0));
        assert_eq!(renderer.line_height_multiplier, 1.2);
    }

    #[test]
    fn test_background_rect_merge() {
        let black = Hsla::black();

        let rect1 = BackgroundRect {
            start_col: 0,
            end_col: 5,
            row: 0,
            color: black,
        };

        let rect2 = BackgroundRect {
            start_col: 5,
            end_col: 10,
            row: 0,
            color: black,
        };

        assert!(rect1.can_merge_with(&rect2));

        let rect3 = BackgroundRect {
            start_col: 5,
            end_col: 10,
            row: 1,
            color: black,
        };

        assert!(!rect1.can_merge_with(&rect3));
    }

    #[test]
    fn test_merge_backgrounds() {
        let renderer = TerminalRenderer::new(
            "monospace".to_string(),
            px(14.0),
            1.2,
            ColorPalette::default(),
        );
        let black = Hsla::black();

        let rects = vec![
            BackgroundRect {
                start_col: 0,
                end_col: 5,
                row: 0,
                color: black,
            },
            BackgroundRect {
                start_col: 5,
                end_col: 10,
                row: 0,
                color: black,
            },
        ];

        let merged = renderer.merge_backgrounds(rects);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start_col, 0);
        assert_eq!(merged[0].end_col, 10);
    }

    #[test]
    fn a_wide_glyph_background_covers_both_cells() {
        use alacritty_terminal::vte::ansi::NamedColor;
        let renderer =
            TerminalRenderer::new("Menlo".to_string(), px(14.0), 1.2, ColorPalette::default());
        let wide = Cell {
            c: '한',
            flags: Flags::WIDE_CHAR,
            bg: Color::Named(NamedColor::Red),
            ..Cell::default()
        };
        let spacer = Cell {
            flags: Flags::WIDE_CHAR_SPACER,
            bg: Color::Named(NamedColor::Red),
            ..Cell::default()
        };
        let cells = vec![(0usize, wide), (1, spacer)];
        let colors = Colors::default();
        let (backgrounds, _) = renderer.layout_row(0, cells.into_iter(), &colors);
        let red = backgrounds
            .iter()
            .find(|b| b.start_col == 0)
            .expect("the wide glyph's background");
        assert_eq!(red.end_col, 2, "the spacer contributes its second cell");
    }

    #[test]
    fn reverse_video_swaps_the_background_and_foreground() {
        use alacritty_terminal::vte::ansi::NamedColor;
        let palette = ColorPalette::default();
        let renderer = TerminalRenderer::new("Menlo".to_string(), px(14.0), 1.2, palette.clone());
        let cell = Cell {
            fg: Color::Named(NamedColor::Red),
            bg: Color::Named(NamedColor::Background),
            flags: Flags::INVERSE,
            ..Cell::default()
        };
        let colors = Colors::default();
        let (backgrounds, _) = renderer.layout_row(0, std::iter::once((0usize, cell)), &colors);
        let bg = backgrounds
            .iter()
            .find(|b| b.start_col == 0)
            .expect("a background");
        assert_eq!(bg.color, palette.ansi_colors()[1]);
    }
}
