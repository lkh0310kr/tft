//! Colours, metrics and component measurements shared by every TFT UI.

pub mod color;
pub mod tokens;

pub use color::parse_color;
pub use tokens::{dark, light, Color, ShadowLayer, Tokens};

pub const FONT_UI_SMALLER: f32 = 12.;
pub const FONT_UI_SMALL: f32 = 13.;
pub const FONT_UI_MEDIUM: f32 = 15.;
pub const HEADER_HEIGHT: f32 = 40.;
pub const TAB_CURVE: f32 = 10.;
pub const RIBBON_WIDTH: f32 = 44.;
pub const ICON_SIZE: f32 = 16.;
pub const SIDEBAR_WIDTH: f32 = 300.;

pub const DEFAULT_MONO_FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(target_os = "windows") {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};

pub const HEADINGS: [(f32, f32, f32); 6] = [
    (1.618, 700., 1.2),
    (1.462, 680., 1.2),
    (1.318, 660., 1.3),
    (1.188, 640., 1.3),
    (1.076, 620., 1.3),
    (1.0, 600., 1.3),
];

pub fn heading_style(level: u8) -> (f32, f32, f32) {
    match level {
        1..=6 => HEADINGS[level as usize - 1],
        _ => HEADINGS[5],
    }
}

#[derive(Debug, Clone)]
pub struct CanvasColors<C = Color> {
    pub background: C,
    pub card_border: C,
    pub dot: C,
    pub label: C,
    pub border_alpha: f32,
    pub tint_alpha: f32,
    pub wash_alpha: f32,
    pub on_color: C,
    pub control_group: C,
    pub control_border: C,
    pub control_bg: C,
    pub control_fg: C,
    pub control_bg_hover: C,
    pub control_fg_hover: C,
    pub card_menu_bg: C,
    pub presets: [C; 6],
}

impl<C: Copy> CanvasColors<C> {
    pub fn from_tokens(dark: bool, t: &Tokens<C>, on_color: C) -> Self {
        CanvasColors {
            background: t.canvas_background,
            card_border: t.canvas_color,
            dot: t.canvas_dot_pattern,
            label: t.canvas_card_label_color,
            border_alpha: 0.7,
            tint_alpha: 0.07,
            wash_alpha: 0.1,
            on_color,
            control_group: t.interactive_normal,
            control_border: t.background_modifier_border,
            control_bg: t.background_primary,
            control_fg: t.text_muted,
            control_bg_hover: t.interactive_hover,
            control_fg_hover: t.text_normal,
            card_menu_bg: if dark {
                t.background_secondary
            } else {
                t.background_primary
            },
            presets: [
                t.canvas_color_1,
                t.canvas_color_2,
                t.canvas_color_3,
                t.canvas_color_4,
                t.canvas_color_5,
                t.canvas_color_6,
            ],
        }
    }

    pub fn preset(&self, n: u8) -> Option<C> {
        (1..=6)
            .contains(&n)
            .then(|| self.presets[usize::from(n) - 1])
    }
}
