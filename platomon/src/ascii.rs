//! Radiance → luminance → glyph, and frame-buffer → string.

use std::fmt::Write as _;

use crate::aspect::Viewport;
use crate::color::{self, ColorDepth};

/// Default ramp: 16 luminance levels, ordered by glyph ink coverage.
pub const DEFAULT_RAMP: &str = " .,:;-~=+*ox#%&@";
/// The original 10-level ramp (`--ramp " .:-=+*#%@"`).
pub const CLASSIC_RAMP: &str = " .:-=+*#%@";

/// Glyph ramp, darkest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Ramp {
    chars: Vec<char>,
}

impl Ramp {
    pub fn new(s: &str) -> Self {
        let chars: Vec<char> = s.chars().filter(|c| !c.is_control()).collect();
        if chars.len() < 2 { Self::default() } else { Self { chars } }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// `lum` in 0..=1 → bucket index; bucket `i` covers `[i/n, (i+1)/n)`.
    #[inline(always)]
    pub fn level(&self, lum: f32) -> usize {
        let n = self.chars.len();
        let l = if lum.is_finite() { lum.clamp(0.0, 1.0) } else { 0.0 };
        ((l * n as f32) as usize).min(n - 1)
    }

    /// `lum` in 0..=1 → glyph.
    #[inline(always)]
    pub fn glyph(&self, lum: f32) -> char {
        self.chars[self.level(lum)]
    }
}

impl Default for Ramp {
    fn default() -> Self {
        Self { chars: DEFAULT_RAMP.chars().collect() }
    }
}

const INV_GAMMA: f32 = 1.0 / 2.2;

/// Reinhard tone map + gamma 2.2.
#[inline(always)]
pub fn tone_map(radiance: f32, exposure: f32) -> f32 {
    let x = (radiance * exposure).max(0.0);
    if !x.is_finite() {
        return if x.is_infinite() { 1.0 } else { 0.0 };
    }
    (x / (1.0 + x)).powf(INV_GAMMA)
}

/// Radiance buffer → display luminance 0..=1.
pub fn tone_buffer(buf: &[f32], exposure: f32, out: &mut Vec<f32>) {
    out.clear();
    out.extend(buf.iter().map(|&v| tone_map(v, exposure)));
}

/// Display luminance → glyphs.
pub fn shade(lum: &[f32], ramp: &Ramp, out: &mut Vec<char>) {
    out.clear();
    out.extend(lum.iter().map(|&l| ramp.glyph(l)));
}

/// Radiance buffer straight to glyphs.
pub fn glyphs(buf: &[f32], ramp: &Ramp, exposure: f32) -> Vec<char> {
    buf.iter().map(|&v| ramp.glyph(tone_map(v, exposure))).collect()
}

/// Per-cell colours (`0xRRGGBB` or [`color::NONE`]) and the escape depth.
pub type Colors<'a> = Option<(&'a [u32], ColorDepth)>;

/// Push one row of glyphs, switching colour only where it changes. Leaves
/// the default colour set.
fn push_row(out: &mut String, cells: &[char], colors: Option<(&[u32], ColorDepth)>) {
    let Some((cols, depth)) = colors else {
        out.extend(cells);
        return;
    };
    let mut cur = color::NONE;
    for (i, &c) in cells.iter().enumerate() {
        let col = cols.get(i).copied().unwrap_or(color::NONE);
        if col != cur && c != ' ' {
            color::push_sgr(out, col, depth);
            cur = col;
        }
        out.push(c);
    }
    if cur != color::NONE {
        color::push_sgr(out, color::NONE, depth);
    }
}

/// Rows of the `w × h` glyph grid joined by `'\n'` (no trailing newline).
pub fn compose_plain(cells: &[char], colors: Colors<'_>, w: usize, h: usize, out: &mut String) {
    out.clear();
    for y in 0..h {
        if y > 0 {
            out.push('\n');
        }
        let lo = (y * w).min(cells.len());
        let hi = ((y + 1) * w).min(cells.len());
        let row_colors = colors.map(|(c, d)| (&c[lo.min(c.len())..hi.min(c.len())], d));
        push_row(out, &cells[lo..hi], row_colors);
        out.extend(std::iter::repeat_n(' ', w - (hi - lo)));
    }
}

/// The stats line: text, 0-based terminal row, terminal width.
pub struct StatsLine<'a> {
    pub text: &'a str,
    pub row: usize,
    pub width: usize,
}

/// Terminal frame from the picture's glyph grid (`vp.w × vp.h`): each row is
/// prefixed with an absolute cursor move, so no line wrapping or scrolling
/// can occur. Every cell of the `cols × rows` area is written each frame — the
/// picture at `vp.x, vp.y` plus blank margins — so the screen never depends on
/// a terminal honouring clear-screen codes. Everything ends up in one string
/// for a single `write_all`.
pub fn compose_terminal(
    cells: &[char],
    colors: Colors<'_>,
    vp: &Viewport,
    (cols, rows): (usize, usize),
    stats: Option<StatsLine<'_>>,
    out: &mut String,
) {
    out.clear();
    let w = vp.w;
    let right = cols.saturating_sub(vp.x + vp.w);
    for ty in 0..rows {
        let _ = write!(out, "\x1b[{};1H", ty + 1);
        if ty >= vp.y && ty < vp.y + vp.h && w > 0 {
            let y = ty - vp.y;
            out.extend(std::iter::repeat_n(' ', vp.x));
            let row = y * w..(y + 1) * w;
            push_row(out, &cells[row.clone()], colors.map(|(c, d)| (&c[row], d)));
            out.extend(std::iter::repeat_n(' ', right));
        } else {
            out.extend(std::iter::repeat_n(' ', cols));
        }
    }
    if let Some(StatsLine { text: s, row, width: w }) = stats {
        let _ = write!(out, "\x1b[{};1H", row + 1);
        let mut n = 0;
        for c in s.chars().take(w) {
            out.push(c);
            n += 1;
        }
        out.extend(std::iter::repeat_n(' ', w.saturating_sub(n)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_boundaries() {
        // 16 levels, bucket i covers [i/16, (i+1)/16)
        let r = Ramp::default();
        assert_eq!(r.len(), 16);
        let expected: Vec<char> = DEFAULT_RAMP.chars().collect();
        for (i, &c) in expected.iter().enumerate() {
            let lo = i as f32 / 16.0;
            assert_eq!(r.glyph(lo), c, "start of bucket {i}");
            assert_eq!(r.glyph(lo + 1.0 / 16.0 - 1e-4), c, "end of bucket {i}");
        }
        assert_eq!(r.glyph(0.0), ' ');
        assert_eq!(r.glyph(0.0624), ' ');
        assert_eq!(r.glyph(0.0625), '.');
        assert_eq!(r.glyph(0.5), '+');
        assert_eq!(r.glyph(0.9375), '@');
        assert_eq!(r.glyph(1.0), '@');
        assert_eq!(r.glyph(5.0), '@');
        assert_eq!(r.glyph(-1.0), ' ');
        assert_eq!(r.glyph(f32::NAN), ' ');

        // the classic 10-level ramp still works
        let c = Ramp::new(CLASSIC_RAMP);
        assert_eq!(c.len(), 10);
        assert_eq!(c.glyph(0.0999), ' ');
        assert_eq!(c.glyph(0.1), '.');
        assert_eq!(c.glyph(0.7), '#');
        assert_eq!(c.glyph(0.9), '@');
    }

    #[test]
    fn custom_ramp() {
        let r = Ramp::new("ab");
        assert_eq!(r.glyph(0.49), 'a');
        assert_eq!(r.glyph(0.5), 'b');
        assert_eq!(Ramp::new("x"), Ramp::default());
        let u = Ramp::new(" ░▒▓█");
        assert_eq!(u.len(), 5);
        assert_eq!(u.glyph(1.0), '█');
    }

    #[test]
    fn tone_map_range() {
        assert_eq!(tone_map(0.0, 1.0), 0.0);
        assert_eq!(tone_map(-3.0, 1.0), 0.0);
        assert!(tone_map(1e9, 1.0) > 0.999);
        assert_eq!(tone_map(f32::INFINITY, 1.0), 1.0);
        assert_eq!(tone_map(f32::NAN, 1.0), 0.0);
        let mid = tone_map(1.0, 1.0); // 0.5^(1/2.2)
        assert!((mid - 0.5f32.powf(1.0 / 2.2)).abs() < 1e-6);
        assert!(tone_map(0.5, 1.0) < tone_map(0.6, 1.0));
    }

    #[test]
    fn plain_frame_length() {
        let (w, h) = (17, 5);
        let buf = vec![0.5; w * h];
        let mut s = String::new();
        compose_plain(&glyphs(&buf, &Ramp::default(), 1.0), None, w, h, &mut s);
        assert_eq!(s.chars().count(), w * h + (h - 1));
        assert_eq!(s.lines().count(), h);
        assert!(s.lines().all(|l| l.chars().count() == w));
    }

    fn stats(text: &str, row: usize, width: usize) -> Option<StatsLine<'_>> {
        Some(StatsLine { text, row, width })
    }

    #[test]
    fn terminal_frame_contents() {
        let (w, h) = (8, 3);
        let vp = Viewport { x: 0, y: 0, w, h, fit: None };
        let buf = vec![' '; w * h];
        let mut s = String::new();
        compose_terminal(&buf, None, &vp, (w, h), stats("CPU 1% | a very long stats line", 3, 8), &mut s);
        assert!(s.starts_with("\x1b[1;1H"));
        assert!(s.ends_with("\x1b[4;1HCPU 1% |"), "stats truncated to width: {s:?}");
        let glyphs: usize =
            s.split("\x1b[").skip(1).take(h).map(|seg| seg.split_once('H').unwrap().1.chars().count()).sum();
        assert_eq!(glyphs, w * h);

        compose_terminal(&buf, None, &vp, (w, h), stats("ab", 3, 8), &mut s);
        assert!(s.ends_with("\x1b[4;1Hab      "), "stats padded to width");
    }

    #[test]
    fn every_cell_written_including_margins() {
        // 7×4 area, 3×2 picture at (2,1)
        let vp = Viewport { x: 2, y: 1, w: 3, h: 2, fit: Some(1.0) };
        let mut s = String::new();
        compose_terminal(&glyphs(&[1e9; 6], &Ramp::default(), 1.0), None, &vp, (7, 4), None, &mut s);
        assert_eq!(s, "\x1b[1;1H       \x1b[2;1H  @@@  \x1b[3;1H  @@@  \x1b[4;1H       ");
    }

    #[test]
    fn colours_switch_only_on_change_and_reset_each_row() {
        let vp = Viewport { x: 1, y: 0, w: 4, h: 1, fit: None };
        let cells = ['a', 'b', ' ', 'c'];
        let cols = [0xff0000, 0xff0000, 0x00ff00, color::NONE];
        let mut s = String::new();
        compose_terminal(&cells, Some((&cols, ColorDepth::True)), &vp, (6, 1), None, &mut s);
        // one code for the red run, none for the blank (no visible ink), reset for 'c'
        assert_eq!(s, "\x1b[1;1H \x1b[38;2;255;0;0mab \x1b[39mc ");
        compose_plain(&cells, Some((&cols, ColorDepth::Ansi256)), 4, 1, &mut s);
        assert_eq!(s, "\x1b[38;5;196mab \x1b[39mc");
    }
}
