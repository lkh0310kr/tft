//! GPUI's view of the shared design tokens.

use gpui::Hsla;

pub type Tokens = crate::design::Tokens<Hsla>;
pub type ShadowLayer = crate::design::ShadowLayer<Hsla>;

fn color(c: crate::design::Color) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a: c.a,
    }
}

pub fn dark() -> Tokens {
    crate::design::dark().map(color)
}

pub fn light() -> Tokens {
    crate::design::light().map(color)
}
