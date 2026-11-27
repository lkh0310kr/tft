//! The shell's GPUI theme: `appearance.json` chooses dark, light or system.
//! Token values come from `crate::design`; this module converts them to
//! GPUI colours. Paint sites read the surface's own token.

pub mod color;
pub mod registry;

use std::sync::OnceLock;

use gpui::{rgb, Global, Hsla, SharedString};
use tft_formats::config::AppearanceConfig;

pub use crate::design::{
    heading_style, DEFAULT_MONO_FONT, FONT_UI_MEDIUM, FONT_UI_SMALL, FONT_UI_SMALLER,
    HEADER_HEIGHT, HEADINGS, ICON_SIZE, RIBBON_WIDTH, SIDEBAR_WIDTH,
};
pub use registry::Tokens;

#[derive(Debug, Clone)]
pub struct Theme {
    pub dark: bool,
    pub ui_font: SharedString,
    pub text_font: SharedString,
    pub mono_font: SharedString,
    pub base_font_size: f32,
    /// The canvas: card background and the border of a card with no colour
    /// of its own, the dot grid, the six preset colours a card or edge can
    /// carry (`"1"` red through `"6"` purple), the stroke of an edge with no
    /// colour, the label text above a card and inside a group, and the wash
    /// of a group's body and a marquee.
    pub canvas: CanvasColors,
    /// The registry: every token by Obsidian's name.
    pub tokens: Tokens,
}

pub type CanvasColors = crate::design::CanvasColors<Hsla>;

impl Global for Theme {}

/// The fixed tokens of one base theme, including TFT's primary accent
/// (`#0083f0`).
fn base_tokens(dark: bool) -> &'static Tokens {
    static DARK: OnceLock<Tokens> = OnceLock::new();
    static LIGHT: OnceLock<Tokens> = OnceLock::new();
    let cell = if dark { &DARK } else { &LIGHT };
    cell.get_or_init(|| {
        if dark {
            registry::dark()
        } else {
            registry::light()
        }
    })
}

impl Theme {
    pub fn dark() -> Theme {
        Theme::from_tokens(true, base_tokens(true).clone())
    }

    /// Obsidian's light theme with TFT's accent. The app's windows reach
    /// the light theme through `from_appearance`; Export to PDF prints in
    /// this one, as Obsidian prints in its light theme whatever is on
    /// screen, and the tests reach it here.
    pub fn light() -> Theme {
        Theme::from_tokens(false, base_tokens(false).clone())
    }

    /// A theme over a token set.
    pub fn from_tokens(dark: bool, tokens: Tokens) -> Theme {
        Theme {
            dark,
            ui_font: crate::assets::FONT_FAMILY.into(),
            text_font: crate::assets::FONT_FAMILY.into(),
            mono_font: DEFAULT_MONO_FONT.into(),
            base_font_size: 16.0,
            canvas: CanvasColors::from_tokens(
                dark,
                &tokens,
                if dark { gpui::black() } else { gpui::white() },
            ),
            tokens,
        }
    }

    /// The theme `appearance.json` asks for: the base color scheme only.
    /// `system` (and an absent key) means the desktop's own scheme, which
    /// only a window can answer; `system_dark` is that answer.
    pub fn from_appearance(config: &AppearanceConfig, system_dark: bool) -> Theme {
        let dark = match config.theme.as_deref() {
            Some("light") => false,
            Some("dark") => true,
            _ => system_dark,
        };
        Theme::from_tokens(dark, base_tokens(dark).clone())
    }

    /// The wash behind a `==🔴word==` highlight: Obsidian paints the
    /// theme's `--color-<name>` at 0.3 alpha — red was read back as
    /// `oklch(0.659737 0.216875 23.8105 / 0.3)` on the dark theme, which is
    /// its `#fb464c`; the other five are the same tokens the canvas presets
    /// use, at the same alpha.
    pub fn highlight_color(&self, color: tft_markdown::HighlightColor) -> Hsla {
        use tft_markdown::HighlightColor as C;
        let t = &self.tokens;
        let mut c = match color {
            C::Red => t.color_red,
            C::Orange => t.color_orange,
            C::Yellow => t.color_yellow,
            C::Green => t.color_green,
            C::Blue => t.color_blue,
            C::Purple => t.color_purple,
        };
        c.a = 0.3;
        c
    }

    /// Obsidian's default callout colors by canonical type. `--callout-color`
    /// is a `--color-<name>` token, the same six the canvas presets and the
    /// highlight washes draw from, so the hue is picked here and the theme's
    /// own value comes from the registry — which is what makes the dark
    /// theme's callouts the dark theme's colours. The table of observed
    /// values is in `docs/Specification.md` under "The lucide icon
    /// and colour of every built-in callout type"; it was read in the
    /// default dark theme, and the light column is the same tokens' light
    /// values.
    pub fn callout_color(&self, kind: &str) -> Hsla {
        let t = &self.tokens;
        match kind {
            "abstract" | "tip" => t.color_cyan,
            "success" => t.color_green,
            "question" | "warning" => t.color_orange,
            "failure" | "danger" | "bug" => t.color_red,
            "example" => t.color_purple,
            // Obsidian's grey is a flat value in both themes.
            "quote" => rgb(0x9e9e9e).into(),
            // `note`, `info`, `todo` and anything Obsidian does not know.
            _ => t.color_blue,
        }
    }
}

/// A shadow token as gpui draws it: one [`gpui::BoxShadow`] per outer
/// layer. gpui has no inset shadow, so an inset layer is left out.
pub fn box_shadows(layers: &[registry::ShadowLayer]) -> Vec<gpui::BoxShadow> {
    layers
        .iter()
        .filter(|l| !l.inset)
        .map(|l| gpui::BoxShadow {
            color: l.color,
            offset: gpui::point(gpui::px(l.x), gpui::px(l.y)),
            blur_radius: gpui::px(l.blur),
            spread_radius: gpui::px(l.spread),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_palettes_are_the_probes_own() {
        let dark = Theme::dark();
        let light = Theme::light();
        assert!(dark.dark && !light.dark);
        assert_eq!(
            dark.tokens.background_primary,
            registry::dark().background_primary
        );
        assert_eq!(
            light.tokens.background_primary,
            registry::light().background_primary
        );
        assert_ne!(
            dark.tokens.background_primary,
            light.tokens.background_primary
        );
        assert_eq!(dark.canvas.background, registry::dark().canvas_background);
        assert_eq!(light.canvas.background, registry::light().canvas_background);
    }

    #[test]
    fn appearance_picks_dark_light_or_the_system() {
        let config = |theme: Option<&str>| AppearanceConfig {
            theme: theme.map(str::to_string),
            ..AppearanceConfig::default()
        };
        assert!(Theme::from_appearance(&config(Some("dark")), false).dark);
        assert!(!Theme::from_appearance(&config(Some("light")), true).dark);
        assert!(Theme::from_appearance(&config(Some("system")), true).dark);
        assert!(!Theme::from_appearance(&config(None), false).dark);
    }

    #[test]
    fn a_shadow_token_drops_its_inset_layers() {
        let shadow = registry::ShadowLayer {
            x: 0.,
            y: 1.,
            blur: 2.,
            spread: 3.,
            color: gpui::black(),
            inset: false,
        };
        let inset = registry::ShadowLayer {
            inset: true,
            ..shadow.clone()
        };
        assert_eq!(box_shadows(&[shadow, inset]).len(), 1);
    }
}
