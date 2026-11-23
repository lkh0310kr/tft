//! GPUI's view of the shared design tokens.

use gpui::Hsla;

pub type Tokens = tft_core::design::Tokens<Hsla>;
pub type ShadowLayer = tft_core::design::ShadowLayer<Hsla>;

fn color(c: tft_core::design::Color) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a: c.a,
    }
}

pub fn dark() -> Tokens {
    tft_core::design::dark().map(color)
}

pub fn light() -> Tokens {
    tft_core::design::light().map(color)
}
