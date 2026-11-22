//! Overlays drawn over the shaded picture:
//!
//! * **wireframe** — each solid's visible edges, drawn into the picture with
//!   inverted luminosity so neighbouring solids separate clearly;
//! * **legend** — a panel in one corner with a small wireframe of every solid
//!   (spinning with it) next to the metric it shows.

use glam::Vec3A;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::ascii::{self, Ramp};
use crate::color::{self, ColorBy, ColorMap};
use crate::render::FrameView;
use crate::scene::Scene;
use crate::scene::sdf::SolidKind;

// ------------------------------------------------------------------ params

/// Legend corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LegendPos {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl FromStr for LegendPos {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "tl" | "top-left" => Ok(Self::TopLeft),
            "tr" | "top-right" => Ok(Self::TopRight),
            "bl" | "bottom-left" => Ok(Self::BottomLeft),
            "br" | "bottom-right" => Ok(Self::BottomRight),
            _ => Err(format!("unknown legend position '{s}' (tl|tr|bl|br)")),
        }
    }
}

/// Smallest and largest legend size.
pub const LEGEND_SIZE_MIN: u8 = 1;
pub const LEGEND_SIZE_MAX: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegendParams {
    /// 1..=5. 1–2: terminal text; 3–5: block letters, 1–3 cells per font pixel.
    /// Icons grow with the size.
    pub size: u8,
    /// Panel background: 0 = see-through, 1 = solid blank.
    pub opacity: f32,
    pub pos: LegendPos,
}

impl Default for LegendParams {
    fn default() -> Self {
        Self { size: 2, opacity: 0.85, pos: LegendPos::TopLeft }
    }
}

// ------------------------------------------------------------------ wire models

/// Unit-circumradius edge model of one solid.
#[derive(Debug, Clone)]
struct Wire {
    verts: Vec<Vec3A>,
    /// Face normals and offsets.
    faces: Vec<(Vec3A, f32)>,
    edges: Vec<(usize, usize, usize, usize)>,
}

impl Wire {
    fn new(kind: SolidKind) -> Self {
        let p = crate::scene::sdf::Polyhedron::new(kind);
        Self { verts: p.vertices(), faces: p.face_planes(), edges: p.edges() }
    }
}

/// Inverted luminosity, pushed at least `MIN_SEP` away from the original so
/// edges stay visible on mid-grey too.
#[inline]
pub fn invert(l: f32) -> f32 {
    const MIN_SEP: f32 = 0.4;
    let l = l.clamp(0.0, 1.0);
    let i = 1.0 - l;
    if (i - l).abs() >= MIN_SEP {
        i
    } else if l < 0.5 {
        l + MIN_SEP
    } else {
        l - MIN_SEP
    }
}

// ------------------------------------------------------------------ block font

/// 3×5 pixel font, one byte per row, bit 2 = left column.
fn font_glyph(c: char) -> [u8; 5] {
    match c.to_ascii_uppercase() {
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [6, 5, 5, 5, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 6, 3],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [6, 1, 2, 4, 7],
        '3' => [6, 1, 2, 1, 6],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 6, 1, 6],
        '6' => [3, 4, 7, 5, 7],
        '7' => [7, 1, 2, 2, 2],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 6],
        '%' => [5, 1, 2, 4, 5],
        '.' => [0, 0, 0, 0, 2],
        ':' => [0, 2, 0, 2, 0],
        '-' => [0, 0, 7, 0, 0],
        '+' => [0, 2, 7, 2, 0],
        '/' => [1, 1, 2, 4, 4],
        '\\' => [4, 4, 2, 1, 1],
        '_' => [0, 0, 0, 0, 7],
        '(' => [2, 4, 4, 4, 2],
        ')' => [2, 1, 1, 1, 2],
        ' ' => [0; 5],
        _ => [7, 1, 2, 0, 2], // '?'
    }
}

/// Label typography for a legend size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Font {
    /// 0 = terminal text, else cells per font pixel (vertically).
    scale: usize,
    /// Columns per font pixel.
    px_w: usize,
}

impl Font {
    fn for_size(size: u8, char_aspect: f32) -> Self {
        if size < 3 {
            return Self { scale: 0, px_w: 1 };
        }
        let scale = (size - 2) as usize;
        let px_w = ((scale as f32 * 0.5 / char_aspect.max(0.1)).round() as usize).max(1);
        Self { scale, px_w }
    }

    fn height(&self) -> usize {
        if self.scale == 0 { 1 } else { 5 * self.scale }
    }

    fn width(&self, text: &str) -> usize {
        let n = text.chars().count();
        if self.scale == 0 { n } else { (n * 4).saturating_sub(1) * self.px_w }
    }
}

// ------------------------------------------------------------------ layout

#[derive(Debug, Clone, PartialEq)]
struct Panel {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    icon_w: usize,
    icon_h: usize,
    label_w: usize,
    font: Font,
    /// (slot, x, y) of each entry's icon.
    entries: Vec<(usize, usize, usize)>,
}

/// Rows of an icon for a legend size (at least 2 more than the label).
fn icon_rows(size: u8, font: &Font) -> usize {
    (3 + 2 * size as usize).max(font.height() + 2)
}

const PAD: usize = 1;
const GAP_COLS: usize = 2;
const MAX_LABEL: usize = 48;

fn layout(p: &LegendParams, w: usize, h: usize, labels: &[String], char_aspect: f32) -> Option<Panel> {
    let n = labels.len();
    if n == 0 || w < 8 || h < 4 {
        return None;
    }
    let size = p.size.clamp(LEGEND_SIZE_MIN, LEGEND_SIZE_MAX);
    let font = Font::for_size(size, char_aspect);
    let mut icon_h = icon_rows(size, &font).min(h.saturating_sub(2 * PAD));
    if icon_h < 3 {
        return None;
    }
    if icon_h < font.height() {
        icon_h = font.height().min(h);
    }
    let icon_w = ((icon_h as f32 / char_aspect.max(0.1)).round() as usize).max(3);
    let mut label_w = labels.iter().map(|l| font.width(l)).max().unwrap_or(0).min(MAX_LABEL * 4 * font.px_w);
    let entry_h = icon_h + 1;
    let per_col = ((h - 2 * PAD + 1) / entry_h).max(1);
    let mut ncols = n.div_ceil(per_col);
    let panel_w = |ncols: usize, label_w: usize| 2 * PAD + ncols * (icon_w + 1 + label_w) + (ncols - 1) * GAP_COLS;
    while ncols > 1 && panel_w(ncols, label_w) > w {
        ncols -= 1;
    }
    if panel_w(ncols, label_w) > w {
        // One column still too wide: clip the labels.
        label_w = w.checked_sub(2 * PAD + icon_w + 1)?;
        if label_w < 3 {
            return None;
        }
    }
    let shown = n.min(ncols * per_col);
    let rows_used = shown.min(per_col);
    let pw = panel_w(ncols, label_w);
    let ph = (2 * PAD + rows_used * entry_h - 1).min(h);
    let (x, y) = match p.pos {
        LegendPos::TopLeft => (0, 0),
        LegendPos::TopRight => (w - pw, 0),
        LegendPos::BottomLeft => (0, h - ph),
        LegendPos::BottomRight => (w - pw, h - ph),
    };
    let entries = (0..shown)
        .map(|i| {
            let (c, r) = (i / per_col, i % per_col);
            (i, x + PAD + c * (icon_w + 1 + label_w + GAP_COLS), y + PAD + r * entry_h)
        })
        .collect();
    Some(Panel { x, y, w: pw, h: ph, icon_w, icon_h, label_w, font, entries })
}

// ------------------------------------------------------------------ drawing helpers

struct Grid<'a> {
    cells: &'a mut [char],
    /// Per-cell colour, when colour is on; `put` paints `ink`.
    colors: Option<&'a mut [u32]>,
    ink: u32,
    w: usize,
    h: usize,
}

impl<'a> Grid<'a> {
    fn new(cells: &'a mut [char], w: usize, h: usize) -> Self {
        Self { cells, colors: None, ink: color::NONE, w, h }
    }

    #[inline]
    fn put(&mut self, x: isize, y: isize, c: char) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let i = y as usize * self.w + x as usize;
            self.cells[i] = c;
            if let Some(cols) = self.colors.as_deref_mut() {
                cols[i] = self.ink;
            }
        }
    }

    #[inline]
    fn get(&self, x: isize, y: isize) -> Option<char> {
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
            .then(|| self.cells[y as usize * self.w + x as usize])
    }
}

/// Glyph for a line segment's direction (cell units, y down).
fn slope_char(dx: f32, dy: f32, char_aspect: f32) -> char {
    let (px, py) = (dx * char_aspect, dy);
    let a = py.atan2(px).to_degrees().rem_euclid(180.0);
    if !(22.5..157.5).contains(&a) {
        '-'
    } else if (67.5..112.5).contains(&a) {
        '|'
    } else if a < 90.0 {
        '\\' // going right and down
    } else {
        '/'
    }
}

/// Draw a line between continuous cell positions, clipped to `clip` (x0, y0, x1, y1).
fn draw_line(
    g: &mut Grid<'_>,
    (x0, y0): (f32, f32),
    (x1, y1): (f32, f32),
    clip: (isize, isize, isize, isize),
    ch: impl Fn(char) -> Option<char>,
) {
    let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
    for s in 0..=n {
        let t = s as f32 / n as f32;
        let x = (x0 + (x1 - x0) * t).floor() as isize;
        let y = (y0 + (y1 - y0) * t).floor() as isize;
        if x < clip.0 || y < clip.1 || x >= clip.2 || y >= clip.3 {
            continue;
        }
        if let Some(old) = g.get(x, y)
            && let Some(c) = ch(old)
        {
            g.put(x, y, c);
        }
    }
}

fn draw_text(g: &mut Grid<'_>, x: usize, y: usize, max_w: usize, text: &str, font: &Font, ink: char) {
    if font.scale == 0 {
        for (i, c) in text.chars().filter(|c| !c.is_control()).take(max_w).enumerate() {
            g.put((x + i) as isize, y as isize, c);
        }
        return;
    }
    let (s, pw) = (font.scale, font.px_w);
    for (i, c) in text.chars().enumerate() {
        let cx = x + i * 4 * pw;
        if cx + 3 * pw > x + max_w {
            break;
        }
        for (row, bits) in font_glyph(c).iter().enumerate() {
            for col in 0..3 {
                if bits & (4 >> col) == 0 {
                    continue;
                }
                for dy in 0..s {
                    for dx in 0..pw {
                        g.put((cx + col * pw + dx) as isize, (y + row * s + dy) as isize, ink);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ the layers

/// Wire models per slot plus scratch buffers, reused across frames.
pub struct Overlays {
    wires: Vec<Wire>,
    mask: Vec<bool>,
    lum: Vec<f32>,
    cells: Vec<char>,
    colors: Vec<u32>,
}

/// What to draw this frame.
pub struct OverlayOptions<'a> {
    pub wireframe: bool,
    pub legend: Option<&'a LegendParams>,
    /// One label per slot.
    pub labels: &'a [String],
    pub char_aspect: f32,
    pub ramp: &'a Ramp,
    pub exposure: f32,
    /// Colour gradient mapping; `None` = monochrome.
    pub color: Option<&'a ColorMap>,
    /// Each slot's load 0..=1 (for `--color-by load`).
    pub levels: &'a [f32],
}

/// The composed picture: glyphs and, when colour is on, per-cell colours.
pub struct Cells<'a> {
    pub chars: &'a [char],
    pub colors: Option<&'a [u32]>,
}

/// Brightness of `--color-by load` colours for glyph bucket `b` of `n`:
/// never fully black, so the hue stays readable on dark faces.
fn load_shade(b: usize, n: usize) -> f32 {
    0.35 + 0.65 * (b as f32 + 0.5) / n as f32
}

impl Overlays {
    pub fn new(scene: &Scene) -> Self {
        let wires = scene.solids.iter().map(|s| Wire::new(s.poly.kind())).collect();
        Self { wires, mask: Vec::new(), lum: Vec::new(), cells: Vec::new(), colors: Vec::new() }
    }

    /// Fill `self.colors` from the final luminances.
    fn colorize(&mut self, cm: &ColorMap, frame: &FrameView<'_>, ramp: &Ramp, levels: &[f32]) {
        let n = ramp.len();
        self.colors.clear();
        match cm.by {
            ColorBy::Lum => {
                let table: Vec<u32> = (0..n).map(|b| cm.gradient.sample((b as f32 + 0.5) / n as f32, 1.0)).collect();
                self.colors.extend(self.lum.iter().map(|&l| table[ramp.level(l)]));
            }
            ColorBy::Load => {
                // slot × bucket table
                let table: Vec<Vec<u32>> = levels
                    .iter()
                    .map(|&lv| (0..n).map(|b| cm.gradient.sample(lv, load_shade(b, n))).collect())
                    .collect();
                self.colors.extend(self.lum.iter().enumerate().map(|(i, &l)| {
                    let id = frame.ids.get(i).copied().unwrap_or(crate::render::NO_HIT) as usize;
                    table.get(id).map_or(color::NONE, |t| t[ramp.level(l)])
                }));
            }
        }
    }

    /// Tone-map the frame, apply the overlays and return the `w × h` grid.
    pub fn compose(
        &mut self,
        frame: &FrameView<'_>,
        w: usize,
        h: usize,
        scene: &Scene,
        opt: &OverlayOptions<'_>,
    ) -> Cells<'_> {
        let ramp = opt.ramp;
        ascii::tone_buffer(frame.radiance, opt.exposure, &mut self.lum);
        self.lum.resize(w * h, 0.0);
        if opt.wireframe {
            self.invert_edges(frame, w, h, scene);
        }
        let panel = opt.legend.and_then(|p| layout(p, w, h, opt.labels, opt.char_aspect).map(|panel| (p, panel)));
        if let Some((p, panel)) = &panel {
            let keep = 1.0 - p.opacity.clamp(0.0, 1.0);
            for y in panel.y..panel.y + panel.h {
                for l in &mut self.lum[y * w + panel.x..y * w + panel.x + panel.w] {
                    *l *= keep;
                }
            }
        }
        ascii::shade(&self.lum, ramp, &mut self.cells);
        if let Some(cm) = opt.color {
            self.colorize(cm, frame, ramp, opt.levels);
            if let Some((_, panel)) = &panel {
                // Panel background stays uncoloured so labels read clearly.
                for y in panel.y..panel.y + panel.h {
                    self.colors[y * w + panel.x..y * w + panel.x + panel.w].fill(color::NONE);
                }
            }
        }
        if let Some((_, panel)) = &panel {
            let mut g = Grid::new(&mut self.cells, w, h);
            if opt.color.is_some() {
                g.colors = Some(&mut self.colors);
            }
            let icon_ink = |slot: usize| match opt.color {
                Some(cm) if cm.by == ColorBy::Load => {
                    cm.gradient.sample(opt.levels.get(slot).copied().unwrap_or(0.0), 1.0)
                }
                Some(cm) => cm.gradient.sample(1.0, 1.0),
                None => color::NONE,
            };
            draw_legend(&mut g, panel, &self.wires, scene, opt, &icon_ink);
        }
        Cells { chars: &self.cells, colors: opt.color.is_some().then_some(&self.colors[..]) }
    }

    /// Mark every visible edge cell of every solid and invert its luminosity.
    fn invert_edges(&mut self, frame: &FrameView<'_>, w: usize, h: usize, scene: &Scene) {
        self.mask.clear();
        self.mask.resize(w * h, false);
        if frame.depth.len() < w * h || frame.ids.len() < w * h {
            return;
        }
        let proj = frame.proj;
        let eye = proj.cam.pos;
        for (i, (inst, wire)) in scene.solids.iter().zip(&self.wires).enumerate() {
            let rot = inst.inv_rot.transpose();
            let front: Vec<bool> = wire
                .faces
                .iter()
                .map(|&(n, o)| {
                    let nw = rot * n;
                    nw.dot(eye - (inst.center + nw * (o * inst.scale))) > 0.0
                })
                .collect();
            let world: Vec<Vec3A> = wire.verts.iter().map(|&v| inst.center + rot * (v * inst.scale)).collect();
            // Depth slack: an edge is hidden only by something clearly in front of it.
            let tol = 0.2 * inst.scale + 0.02;
            for &(a, b, f1, f2) in &wire.edges {
                if !(front[f1] || front[f2]) {
                    continue; // back edge: the solid itself hides it
                }
                let (pa, pb) = (world[a], world[b]);
                let (Some(sa), Some(sb)) = (proj.project(pa), proj.project(pb)) else { continue };
                let n = (2.0 * (sb.0 - sa.0).abs().max((sb.1 - sa.1).abs())).ceil().max(1.0) as usize;
                for s in 0..=n {
                    let p = pa.lerp(pb, s as f32 / n as f32);
                    let Some((x, y)) = proj.project(p) else { continue };
                    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                        continue;
                    }
                    let c = y as usize * w + x as usize;
                    if self.mask[c] {
                        continue;
                    }
                    if frame.ids[c] as usize == i || frame.depth[c] >= (p - eye).length() - tol {
                        self.mask[c] = true;
                    }
                }
            }
        }
        for (l, &m) in self.lum.iter_mut().zip(&self.mask) {
            if m {
                *l = invert(*l);
            }
        }
    }
}

fn draw_legend(
    g: &mut Grid<'_>,
    panel: &Panel,
    wires: &[Wire],
    scene: &Scene,
    opt: &OverlayOptions<'_>,
    icon_ink: &dyn Fn(usize) -> u32,
) {
    let (labels, ramp, char_aspect) = (opt.labels, opt.ramp, opt.char_aspect);
    let cam = scene.camera;
    let toward = -cam.forward;
    let ink = ramp.glyph(1.0);
    let hidden = '.';
    let (iw, ih) = (panel.icon_w, panel.icon_h);
    // Unit circumradius → icon cells, matching physical proportions.
    let r_rows = (ih as f32 - 1.0) * 0.5;
    let r_cols = (r_rows / char_aspect.max(0.1)).min((iw as f32 - 1.0) * 0.5);
    let r_rows = r_cols * char_aspect.max(0.1);

    for &(slot, ex, ey) in &panel.entries {
        let (Some(inst), Some(wire)) = (scene.solids.get(slot), wires.get(slot)) else { continue };
        let rot = inst.inv_rot.transpose();
        let (cx, cy) = (ex as f32 + iw as f32 * 0.5, ey as f32 + ih as f32 * 0.5);
        let screen: Vec<(f32, f32)> = wire
            .verts
            .iter()
            .map(|&v| {
                let q = rot * v;
                (cx + q.dot(cam.right) * r_cols, cy - q.dot(cam.up) * r_rows)
            })
            .collect();
        let front: Vec<bool> = wire.faces.iter().map(|&(n, _)| (rot * n).dot(toward) > 0.0).collect();
        let clip = (ex as isize, ey as isize, (ex + iw) as isize, (ey + ih) as isize);
        g.ink = icon_ink(slot);
        // Hidden edges first (dotted, only on blank cells), then visible ones on top.
        for pass in 0..2 {
            for &(a, b, f1, f2) in &wire.edges {
                let visible = front[f1] || front[f2];
                if visible != (pass == 1) {
                    continue;
                }
                let (pa, pb) = (screen[a], screen[b]);
                if visible {
                    let c = slope_char(pb.0 - pa.0, pb.1 - pa.1, char_aspect);
                    draw_line(g, pa, pb, clip, |_| Some(c));
                } else {
                    draw_line(g, pa, pb, clip, |old| (old == ' ').then_some(hidden));
                }
            }
        }
        // Label, vertically centred on the icon.
        let label = labels.get(slot).map(String::as_str).unwrap_or("");
        g.ink = color::NONE;
        let lx = ex + iw + 1;
        let ly = ey + (ih.saturating_sub(panel.font.height())) / 2;
        draw_text(g, lx, ly, panel.label_w, label, &panel.font, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{RenderSettings, Renderer};
    use crate::scene::AnimParams;

    #[test]
    fn invert_keeps_separation() {
        for i in 0..=100 {
            let l = i as f32 / 100.0;
            let v = invert(l);
            assert!((0.0..=1.0).contains(&v));
            assert!((v - l).abs() >= 0.4 - 1e-6, "{l} -> {v}");
        }
        assert_eq!(invert(0.0), 1.0);
        assert_eq!(invert(1.0), 0.0);
    }

    #[test]
    fn positions_parse() {
        assert_eq!("tl".parse::<LegendPos>().unwrap(), LegendPos::TopLeft);
        assert_eq!("Bottom_Right".parse::<LegendPos>().unwrap(), LegendPos::BottomRight);
        assert!("middle".parse::<LegendPos>().is_err());
    }

    fn labels(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("CPU {i}%")).collect()
    }

    #[test]
    fn legend_fits_and_sits_in_its_corner() {
        let (w, h) = (120, 40);
        for pos in [LegendPos::TopLeft, LegendPos::TopRight, LegendPos::BottomLeft, LegendPos::BottomRight] {
            for size in LEGEND_SIZE_MIN..=LEGEND_SIZE_MAX {
                for n in [1, 5, 16] {
                    let p = LegendParams { size, opacity: 1.0, pos };
                    let Some(pl) = layout(&p, w, h, &labels(n), 0.5) else { continue };
                    assert!(pl.x + pl.w <= w && pl.y + pl.h <= h, "{pos:?} {size} {n}: {pl:?}");
                    let right = matches!(pos, LegendPos::TopRight | LegendPos::BottomRight);
                    let bottom = matches!(pos, LegendPos::BottomLeft | LegendPos::BottomRight);
                    assert_eq!(pl.x == w - pl.w, right || pl.w == w);
                    assert_eq!(pl.y == h - pl.h, bottom || pl.h == h);
                    for &(_, ex, ey) in &pl.entries {
                        assert!(ex + pl.icon_w <= pl.x + pl.w && ey + pl.icon_h <= pl.y + pl.h);
                    }
                }
            }
        }
        // 16 entries at size 2 need more than one column at 40 rows
        let pl = layout(&LegendParams::default(), w, h, &labels(16), 0.5).unwrap();
        assert_eq!(pl.entries.len(), 16);
        assert!(pl.entries.iter().any(|e| e.1 != pl.entries[0].1));
    }

    #[test]
    fn block_font_metrics() {
        let f = Font::for_size(3, 0.5);
        assert_eq!((f.scale, f.px_w, f.height(), f.width("AB")), (1, 1, 5, 7));
        let f = Font::for_size(5, 0.5);
        assert_eq!((f.scale, f.px_w, f.height()), (3, 3, 15));
        assert_eq!(Font::for_size(2, 0.5).width("CPU 5%"), 6);
        let mut cells = vec![' '; 8 * 5];
        draw_text(&mut Grid::new(&mut cells, 8, 5), 0, 0, 8, "T1", &Font::for_size(3, 0.5), '#');
        let rows: Vec<String> = cells.chunks(8).map(|r| r.iter().collect()).collect();
        assert_eq!(rows, ["###  #  ", " #  ##  ", " #   #  ", " #   #  ", " #  ### "]);
    }

    const W: usize = 100;
    const H: usize = 34;

    /// (plain glyphs, overlaid glyphs, colours, ids)
    fn frame_full(
        wireframe: bool,
        legend: Option<&LegendParams>,
        color: Option<&ColorMap>,
    ) -> (Vec<char>, Vec<char>, Option<Vec<u32>>, Vec<u8>) {
        let scene = Scene::demo(&AnimParams::default());
        let mut r = Renderer::new();
        let rs = RenderSettings { spp: 1, ..RenderSettings::default() };
        r.render(&scene, W, H, &rs);
        let view = r.view().unwrap();
        let ramp = Ramp::default();
        let plain = ascii::glyphs(view.radiance, &ramp, 1.0);
        let mut o = Overlays::new(&scene);
        let labels = labels(scene.solids.len());
        let levels = [0.0, 0.25, 0.5, 0.75, 1.0];
        let opt = OverlayOptions {
            wireframe,
            legend,
            labels: &labels,
            char_aspect: 0.5,
            ramp: &ramp,
            exposure: 1.0,
            color,
            levels: &levels,
        };
        let out = o.compose(&view, W, H, &scene, &opt);
        (plain, out.chars.to_vec(), out.colors.map(<[u32]>::to_vec), view.ids.to_vec())
    }

    fn frame_with(wireframe: bool, legend: Option<&LegendParams>) -> (Vec<char>, Vec<char>) {
        let (p, o, _, _) = frame_full(wireframe, legend, None);
        (p, o)
    }

    #[test]
    fn lum_colours_follow_glyph_brightness() {
        let cm = ColorMap {
            gradient: crate::color::Gradient::parse("grey").unwrap(),
            by: ColorBy::Lum,
            depth: crate::color::ColorDepth::True,
        };
        let (_, chars, cols, _) = frame_full(false, None, Some(&cm));
        let cols = cols.expect("colours on");
        let ramp: Vec<char> = crate::ascii::DEFAULT_RAMP.chars().collect();
        for (c, col) in chars.iter().zip(&cols) {
            let b = ramp.iter().position(|r| r == c).unwrap();
            assert_eq!(*col & 0xff, ((b as f32 + 0.5) / 16.0 * 255.0).round() as u32, "glyph {c:?}");
        }
        let (_, _, none, _) = frame_full(false, None, None);
        assert!(none.is_none(), "monochrome has no colour buffer");
    }

    #[test]
    fn load_colours_solids_only_and_legend_is_plain() {
        let cm = ColorMap {
            gradient: crate::color::Gradient::parse("#000000:#ff0000").unwrap(),
            by: ColorBy::Load,
            depth: crate::color::ColorDepth::True,
        };
        let (_, _, cols, ids) = frame_full(false, None, Some(&cm));
        let cols = cols.unwrap();
        for (i, (&col, &id)) in cols.iter().zip(&ids).enumerate() {
            if (id as usize) < 5 {
                assert_eq!(col & 0x00ffff, 0, "pure red hue at {i}");
                if id == 0 {
                    assert_eq!(col, 0, "slot 0 is idle → first stop (black)");
                }
            } else {
                assert_eq!(col, color::NONE, "background uncoloured at {i}");
            }
        }
        assert!(cols.iter().any(|&c| c != color::NONE && c >> 16 > 0x80), "busy solids are bright red");

        // opaque legend: panel is uncoloured except icon strokes in the slot colour
        let p = LegendParams { size: 2, opacity: 1.0, pos: LegendPos::TopLeft };
        let (_, chars, cols, _) = frame_full(false, Some(&p), Some(&cm));
        let cols = cols.unwrap();
        let text: String = chars.chunks(W).map(|r| r.iter().collect::<String>() + "\n").collect();
        let row = text.lines().position(|l| l.contains("CPU 4%")).unwrap();
        let col = text.lines().nth(row).unwrap().find("CPU 4%").unwrap();
        assert_eq!(cols[row * W + col], color::NONE, "label text in default colour");
        assert!(cols.contains(&0xff0000), "slot 4 (full load) icon drawn in the last stop");
    }

    #[test]
    fn no_overlays_is_plain_shading() {
        let (plain, out) = frame_with(false, None);
        assert_eq!(plain, out);
    }

    #[test]
    fn wireframe_changes_only_solid_cells() {
        let (plain, out) = frame_with(true, None);
        let changed = plain.iter().zip(&out).filter(|(a, b)| a != b).count();
        assert!(changed > 50, "edges drawn: {changed}");
        assert!(changed < plain.len() / 3, "not the whole picture: {changed}");
    }

    #[test]
    fn opaque_legend_blanks_its_panel_and_draws_labels() {
        let p = LegendParams { size: 2, opacity: 1.0, pos: LegendPos::BottomRight };
        let (_, out) = frame_with(false, Some(&p));
        let text: String = out.chunks(100).map(|r| r.iter().collect::<String>() + "\n").collect();
        for i in 0..5 {
            assert!(text.contains(&format!("CPU {i}%")), "label {i}:\n{text}");
        }
        assert!(text.contains('/') || text.contains('\\') || text.contains('|'), "icons drawn");
    }
}
