//! Inline HTML colours, as `0xRRGGBBAA`.

/// A colour from an inline `style` or `<font color>`, or `None` when the
/// value is not one of the supported shapes.
pub fn parse_color(src: &str) -> Option<u32> {
    let value = src.trim().to_ascii_lowercase();
    if value.is_empty() || value.contains("var(") || value.contains("currentcolor") {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return hex_color(hex);
    }
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
    {
        return rgb_color(inner);
    }
    if let Some(inner) = value
        .strip_prefix("hsla(")
        .or_else(|| value.strip_prefix("hsl("))
    {
        return hsl_color(inner);
    }
    named_color(&value)
}

fn args(inner: &str) -> Option<Vec<&str>> {
    let inner = inner.strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    (parts.len() >= 3).then_some(parts)
}

fn alpha(value: Option<&&str>) -> f32 {
    match value {
        Some(v) if v.ends_with('%') => v[..v.len() - 1].parse::<f32>().unwrap_or(100.) / 100.,
        Some(v) => v.parse::<f32>().unwrap_or(1.),
        None => 1.,
    }
    .clamp(0., 1.)
}

fn byte(v: f32) -> u32 {
    (v.clamp(0., 1.) * 255.).round() as u32
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> u32 {
    byte(r) << 24 | byte(g) << 16 | byte(b) << 8 | byte(a)
}

fn rgb_color(inner: &str) -> Option<u32> {
    let parts = args(inner)?;
    let channel = |v: &str| {
        if let Some(p) = v.strip_suffix('%') {
            p.parse::<f32>().ok().map(|p| p / 100.)
        } else {
            v.parse::<f32>().ok().map(|n| n / 255.)
        }
    };
    Some(rgba(
        channel(parts[0])?,
        channel(parts[1])?,
        channel(parts[2])?,
        alpha(parts.get(3)),
    ))
}

fn hsl_color(inner: &str) -> Option<u32> {
    let parts = args(inner)?;
    let h = parts[0]
        .trim_end_matches("deg")
        .parse::<f32>()
        .ok()?
        .rem_euclid(360.)
        / 360.;
    let percent = |v: &str| {
        v.strip_suffix('%')
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| v / 100.)
            .or_else(|| v.parse::<f32>().ok().map(|v| v / 100.))
    };
    let s = percent(parts[1])?.clamp(0., 1.);
    let l = percent(parts[2])?.clamp(0., 1.);
    let a = alpha(parts.get(3));
    let q = if l < 0.5 { l * (1. + s) } else { l + s - l * s };
    let p = 2. * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.);
        if t < 1. / 6. {
            p + (q - p) * 6. * t
        } else if t < 0.5 {
            q
        } else if t < 2. / 3. {
            p + (q - p) * (2. / 3. - t) * 6.
        } else {
            p
        }
    };
    Some(rgba(
        channel(h + 1. / 3.),
        channel(h),
        channel(h - 1. / 3.),
        a,
    ))
}

fn hex_color(hex: &str) -> Option<u32> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(match hex.len() {
        3 => {
            let (r, g, b) = (((v >> 8) & 0xf) * 17, ((v >> 4) & 0xf) * 17, (v & 0xf) * 17);
            r << 24 | g << 16 | b << 8 | 0xff
        }
        4 => {
            let (r, g, b, a) = (
                ((v >> 12) & 0xf) * 17,
                ((v >> 8) & 0xf) * 17,
                ((v >> 4) & 0xf) * 17,
                (v & 0xf) * 17,
            );
            r << 24 | g << 16 | b << 8 | a
        }
        6 => v << 8 | 0xff,
        8 => v,
        _ => return None,
    })
}

fn named_color(name: &str) -> Option<u32> {
    NAMED_COLORS
        .binary_search_by_key(&name, |&(n, _)| n)
        .ok()
        .map(|i| NAMED_COLORS[i].1 << 8 | 0xff)
}

const NAMED_COLORS: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_rgb_hsl_and_names_land_on_rgba() {
        assert_eq!(parse_color("#fff"), Some(0xffffffff));
        assert_eq!(parse_color("#ff000080"), Some(0xff000080));
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(0xff0000ff));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some(0xff0000ff));
        assert_eq!(parse_color("red"), Some(0xff0000ff));
        assert_eq!(parse_color("var(--x)"), None);
    }
}
