//! Colour gradient mapping: glyph luminance or per-solid load → RGB, sent as
//! 24-bit or 256-colour ANSI foreground codes.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::str::FromStr;

/// "No colour": the terminal's default foreground.
pub const NONE: u32 = u32::MAX;

/// What the gradient position comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorBy {
    /// Every cell by its brightness (dark → first stop, bright → last stop).
    #[default]
    Lum,
    /// Each solid by its metric's load (idle → first stop, full → last
    /// stop), shaded by brightness; background stays uncoloured.
    Load,
}

impl FromStr for ColorBy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "lum" | "luminance" | "brightness" => Ok(Self::Lum),
            "load" | "metric" => Ok(Self::Load),
            _ => Err(format!("unknown colour source '{s}' (lum|load)")),
        }
    }
}

/// Colour escape codes to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorDepth {
    /// 24-bit `ESC[38;2;R;G;Bm`.
    #[default]
    #[serde(rename = "24bit")]
    True,
    /// xterm 256-colour palette `ESC[38;5;Nm`, for older terminals.
    #[serde(rename = "256")]
    Ansi256,
}

impl FromStr for ColorDepth {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "24bit" | "24" | "true" | "truecolor" => Ok(Self::True),
            "256" | "8bit" => Ok(Self::Ansi256),
            _ => Err(format!("unknown colour depth '{s}' (24bit|256)")),
        }
    }
}

/// Built-in gradients, first stop = dark / idle.
pub const PRESETS: [(&str, &str, &[u32]); 12] = [
    ("inferno", "black, purple, red, orange, pale yellow", &[0x000004, 0x420a68, 0x932667, 0xdd513a, 0xfca50a, 0xfcffa4]),
    ("viridis", "purple, blue, teal, green, yellow", &[0x440154, 0x3b528b, 0x21918c, 0x5ec962, 0xfde725]),
    ("plasma", "indigo, magenta, orange, yellow", &[0x0d0887, 0x7e03a8, 0xcc4778, 0xf89540, 0xf0f921]),
    ("fire", "black, deep red, orange, gold, white", &[0x000000, 0x5a0000, 0xc81e00, 0xff8c00, 0xffd700, 0xffffe0]),
    ("ice", "navy, blue, cyan, white", &[0x000010, 0x002050, 0x0070c0, 0x40c0ff, 0xe0ffff]),
    ("ocean", "deep blue to aqua", &[0x000814, 0x001d3d, 0x003566, 0x0096c7, 0x90e0ef]),
    ("sunset", "violet, magenta, coral, gold", &[0x2d0b59, 0x8a1c7c, 0xda4167, 0xf78154, 0xfcd581]),
    ("matrix", "phosphor green", &[0x001000, 0x003b00, 0x008f11, 0x00ff41, 0xc8ffc8]),
    ("amber", "amber monochrome monitor", &[0x100800, 0x6b3a00, 0xffb000, 0xffe0a0]),
    ("grey", "neutral grey ramp", &[0x000000, 0xffffff]),
    ("traffic", "green, yellow, red (good for --color-by load)", &[0x00c040, 0xa0e000, 0xffd000, 0xff7000, 0xff2020]),
    ("rainbow", "red through violet", &[0xff0000, 0xff8000, 0xffff00, 0x00ff00, 0x00ffff, 0x0040ff, 0x8000ff]),
];

/// Default gradient when a colour source is chosen without one.
pub fn default_for(by: ColorBy) -> &'static str {
    match by {
        ColorBy::Lum => "inferno",
        ColorBy::Load => "traffic",
    }
}

/// Evenly spaced RGB stops.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    stops: Vec<[f32; 3]>,
}

fn rgb_f(c: u32) -> [f32; 3] {
    [((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32]
}

fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    match h.len() {
        6 => u32::from_str_radix(h, 16).ok(),
        3 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
        }
        _ => None,
    }
}

impl Gradient {
    pub fn from_stops(stops: &[u32]) -> Self {
        Self { stops: stops.iter().map(|&c| rgb_f(c)).collect() }
    }

    /// A preset name, or two or more hex colours separated by `,` `:` `;`
    /// `/` or spaces, e.g. `#000:#f80:#fff` or `001020,ff8800`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let name = s.trim().to_ascii_lowercase();
        if let Some((_, _, stops)) = PRESETS.iter().find(|(id, _, _)| *id == name) {
            return Ok(Self::from_stops(stops));
        }
        let parts: Vec<&str> = s.split([',', ':', ';', '/', ' ']).filter(|p| !p.trim().is_empty()).collect();
        let stops: Option<Vec<u32>> = parts.iter().map(|p| parse_hex(p)).collect();
        match stops {
            Some(v) if v.len() >= 2 => Ok(Self::from_stops(&v)),
            _ => Err(format!(
                "unknown gradient '{s}': use a preset (see --list gradients) or 2+ hex colours like \"#000:#f80:#fff\""
            )),
        }
    }

    pub fn reversed(mut self) -> Self {
        self.stops.reverse();
        self
    }

    /// Colour at `t` in 0..=1 as `0xRRGGBB`, scaled by `bright` (0..=1).
    pub fn sample(&self, t: f32, bright: f32) -> u32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let b = if bright.is_finite() { bright.clamp(0.0, 1.0) } else { 0.0 };
        let n = self.stops.len();
        let f = t * (n - 1) as f32;
        let i = (f as usize).min(n - 2);
        let u = f - i as f32;
        let (a, c) = (self.stops[i], self.stops[i + 1]);
        let ch = |k: usize| ((a[k] + (c[k] - a[k]) * u) * b).round().clamp(0.0, 255.0) as u32;
        (ch(0) << 16) | (ch(1) << 8) | ch(2)
    }
}

/// Nearest xterm-256 palette index (6×6×6 cube or 24-step grey ramp).
pub fn ansi256(c: u32) -> u8 {
    let (r, g, b) = (((c >> 16) & 0xff) as i32, ((c >> 8) & 0xff) as i32, (c & 0xff) as i32);
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let q = |v: i32| LEVELS.iter().enumerate().min_by_key(|(_, l)| (v - **l).abs()).map_or(0, |(i, _)| i);
    let (qr, qg, qb) = (q(r), q(g), q(b));
    let cube = (LEVELS[qr], LEVELS[qg], LEVELS[qb]);
    let grey_i = (((r + g + b) / 3 - 8).max(0) / 10).min(23);
    let gv = 8 + grey_i * 10;
    let d = |x: (i32, i32, i32)| (x.0 - r).pow(2) + (x.1 - g).pow(2) + (x.2 - b).pow(2);
    if d((gv, gv, gv)) < d(cube) { 232 + grey_i as u8 } else { 16 + (36 * qr + 6 * qg + qb) as u8 }
}

/// Append the SGR code that sets the foreground to `c` ([`NONE`] = default).
pub fn push_sgr(out: &mut String, c: u32, depth: ColorDepth) {
    if c == NONE {
        out.push_str("\x1b[39m");
        return;
    }
    match depth {
        ColorDepth::True => {
            let _ = write!(out, "\x1b[38;2;{};{};{}m", (c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff);
        }
        ColorDepth::Ansi256 => {
            let _ = write!(out, "\x1b[38;5;{}m", ansi256(c));
        }
    }
}

/// Colouring for one run: gradient, source and output depth.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorMap {
    pub gradient: Gradient,
    pub by: ColorBy,
    pub depth: ColorDepth,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_custom_parse() {
        for (id, _, stops) in PRESETS {
            let g = Gradient::parse(id).unwrap();
            assert_eq!(g.sample(0.0, 1.0), stops[0], "{id} start");
            assert_eq!(g.sample(1.0, 1.0), *stops.last().unwrap(), "{id} end");
        }
        let g = Gradient::parse("#000:#f80:#fff").unwrap();
        assert_eq!(g.sample(0.5, 1.0), 0xff8800);
        assert_eq!(g.sample(0.25, 1.0), 0x804400);
        assert_eq!(Gradient::parse("000000, ffffff").unwrap().sample(1.0, 0.5), 0x808080);
        assert_eq!(Gradient::parse("grey").unwrap().reversed().sample(0.0, 1.0), 0xffffff);
        assert!(Gradient::parse("#fff").is_err(), "one stop");
        assert!(Gradient::parse("plaid").is_err());
        assert!(Gradient::parse("#12345g:#000").is_err());
        assert_eq!(Gradient::parse("fire").unwrap().sample(f32::NAN, 1.0), 0x000000);
    }

    #[test]
    fn palette_256() {
        assert_eq!(ansi256(0x000000), 16);
        assert_eq!(ansi256(0xffffff), 231);
        assert_eq!(ansi256(0xff0000), 196);
        assert_eq!(ansi256(0x00ff00), 46);
        assert_eq!(ansi256(0x808080), 244);
        assert_eq!(ansi256(0x121212), 233);
    }

    #[test]
    fn sgr_codes() {
        let mut s = String::new();
        push_sgr(&mut s, 0x0a141e, ColorDepth::True);
        push_sgr(&mut s, 0xff0000, ColorDepth::Ansi256);
        push_sgr(&mut s, NONE, ColorDepth::True);
        assert_eq!(s, "\x1b[38;2;10;20;30m\x1b[38;5;196m\x1b[39m");
        assert_eq!("load".parse::<ColorBy>().unwrap(), ColorBy::Load);
        assert_eq!("256".parse::<ColorDepth>().unwrap(), ColorDepth::Ansi256);
        assert!("cmyk".parse::<ColorDepth>().is_err());
    }
}
