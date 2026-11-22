//! The screen buttons (menu, cogwheel, colour), their three dialogs and the
//! stats readout, drawn straight into the window's pixels with the bitmap
//! font. Layout and tap handling are plain Rust, tested on the host.

use crate::font::{CELL_H, CELL_W, Font, GLYPH_W};
use crate::rgbx;
use crate::settings::{COLOUR_FIELDS, FIELDS, PALETTE_FIELD, SETTINGS_FIELDS, SIMULATION_FIELDS, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && y >= self.y as f32 && x < (self.x + self.w) as f32 && y < (self.y + self.h) as f32
    }
}

/// One setting's row: where its label goes and its two tap halves.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    pub label: (i32, i32),
    /// The value box; its left half steps back, its right half forward.
    pub value: Rect,
    pub prev: Rect,
    pub next: Rect,
    /// The square button beside the value (the palette row's "new palettes").
    pub button: Option<Rect>,
}

/// Inset of the palette and colour buttons' four colour squares from their
/// edge, pixels.
pub const SWATCH_MARGIN: i32 = 6;
/// The four squares as first drawn: top left, top right, bottom left,
/// bottom right.
pub const SWATCHES: [u32; 4] = [0xff_00_00, 0x00_00_ff, 0x00_ff_00, 0xff_ff_00];
/// Side of the white square in the middle of the palette button, pixels.
pub const SWATCH_CENTRE: i32 = 8;
/// Radius asked for the white circle in the middle of the colour button,
/// pixels; it is drawn as big as fits inside the colour squares.
pub const COLOUR_DOT_RADIUS: i32 = 128;
/// Real seconds the buttons stay up after a tap (they start hidden).
pub const BUTTONS_SHOW: f32 = 4.0;

/// The four squares turned a quarter clockwise.
pub fn rotate_swatches([tl, tr, bl, br]: [u32; 4]) -> [u32; 4] {
    [bl, tl, br, tr]
}

/// The four squares mirrored left to right.
pub fn mirror_swatches([tl, tr, bl, br]: [u32; 4]) -> [u32; 4] {
    [tr, tl, br, bl]
}

/// Columns for the labels ("Mass variability" and a gap) and the values
/// ("< Dodecahedron >").
const LABEL_COLS: i32 = 17;
const VALUE_COLS: i32 = 16;

/// Which dialog is showing, each opened by its own screen button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The menu button's.
    Settings,
    /// The cogwheel's.
    Simulation,
    /// The colour button's.
    Colour,
}

impl View {
    pub const ALL: [View; 3] = [View::Settings, View::Simulation, View::Colour];

    pub fn fields(self) -> &'static [usize] {
        match self {
            View::Settings => &SETTINGS_FIELDS,
            View::Simulation => &SIMULATION_FIELDS,
            View::Colour => &COLOUR_FIELDS,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            View::Settings => "Settings",
            View::Simulation => "Simulation Parameters",
            View::Colour => "Colour",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One dialog: a panel with a title, a row per field, and Cancel and Apply.
#[derive(Debug, Clone)]
pub struct Page {
    /// The fields shown, as indices into [`FIELDS`].
    pub fields: &'static [usize],
    pub panel: Rect,
    pub title_y: i32,
    pub rows: Vec<Row>,
    pub cancel: Rect,
    pub apply: Rect,
}

impl Page {
    /// The panel's height for `n` rows at this spacing.
    fn height(n: i32, pad: i32, row_h: i32) -> i32 {
        pad * 2 + row_h * (n + 2) + row_h / 2
    }

    fn new(w: i32, h: i32, (cw, ch): (i32, i32), (pad, row_h): (i32, i32), fields: &'static [usize]) -> Self {
        let n = fields.len() as i32;
        let pw = pad * 2 + (LABEL_COLS + VALUE_COLS) * cw;
        let ph = Self::height(n, pad, row_h);
        let panel = Rect { x: (w - pw) / 2, y: (h - ph) / 2, w: pw, h: ph };
        let rows = fields
            .iter()
            .enumerate()
            .map(|(i, &f)| {
                let y = panel.y + pad + row_h * (i as i32 + 1);
                let mut value = Rect { x: panel.x + pad + LABEL_COLS * cw, y, w: VALUE_COLS * cw, h: row_h };
                // the palette row gives up the left end of its value for a
                // square button, a small gap before the cycler
                let button = (f == PALETTE_FIELD).then(|| {
                    let gap = (cw / 2).max(2);
                    let b = Rect { x: value.x, y, w: row_h, h: row_h };
                    value.x += row_h + gap;
                    value.w -= row_h + gap;
                    b
                });
                let half = value.w / 2;
                Row {
                    label: (panel.x + pad, y + (row_h - ch) / 2),
                    value,
                    prev: Rect { w: half, ..value },
                    next: Rect { x: value.x + half, w: value.w - half, ..value },
                    button,
                }
            })
            .collect();
        let by = panel.y + ph - pad - row_h;
        let bw = (pw - pad * 3) / 2;
        Self {
            fields,
            panel,
            title_y: panel.y + pad + (row_h - ch) / 2,
            rows,
            cancel: Rect { x: panel.x + pad, y: by, w: bw, h: row_h },
            apply: Rect { x: panel.x + pad * 2 + bw, y: by, w: bw, h: row_h },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Layout {
    /// Pixel scale of the dialogs' font.
    pub ts: i32,
    /// The menu button (top-right corner), the cogwheel left of it, and the
    /// colour button under it.
    pub buttons: [Rect; 3],
    /// The dialogs, in [`View::ALL`] order.
    pub pages: [Page; 3],
    /// Font pixel size of the stats readout's block letters.
    pub stats_ts: i32,
}

impl Layout {
    pub fn new(w: i32, h: i32) -> Self {
        let (cw0, ch0) = (CELL_W as i32, CELL_H as i32);
        let n = View::ALL.iter().map(|v| v.fields().len()).max().unwrap_or(1) as i32;
        // the tallest dialog is two cells of padding round a title, the rows
        // and Cancel/Apply: the biggest font that fits with a line to spare,
        // rows two lines tall, or a little tighter (7/4 of a line) where that
        // keeps the font a size bigger
        let fits = |ts: i32, (num, den): (i32, i32)| Page::height(n, cw0 * 2 * ts, ch0 * ts * num / den) + ch0 * ts <= h;
        let widest = (w / (cw0 * (LABEL_COLS + VALUE_COLS + 6))).max(1);
        let mut pick = None;
        for ts in (1..=widest).rev() {
            if let Some(rows) = [(2, 1), (7, 4)].into_iter().find(|&r| fits(ts, r)) {
                pick = Some((ts, rows));
                break;
            }
        }
        let ts = pick.map_or(1, |p| p.0);
        let (cw, ch) = (cw0 * ts, ch0 * ts);
        // the buttons: a ninth of the short side, at least two lines tall
        let bs = (w.min(h) / 9).max(ch * 2);
        let m = bs / 4;
        // a screen too small even at the smallest font: rows a line and a
        // quarter tall, one cell of padding
        let spacing = match pick {
            Some((_, (num, den))) => (cw * 2, ch * num / den),
            None => (cw, ch * 5 / 4),
        };
        Self {
            ts,
            buttons: [
                Rect { x: w - m - bs, y: m, w: bs, h: bs },
                Rect { x: w - 2 * (m + bs), y: m, w: bs, h: bs },
                Rect { x: w - m - bs, y: 2 * m + bs, w: bs, h: bs },
            ],
            pages: View::ALL.map(|v| Page::new(w, h, (cw, ch), spacing, v.fields())),
            stats_ts: (h.min(w) / 270).max(1),
        }
    }

    pub fn page(&self, v: View) -> &Page {
        &self.pages[v.index()]
    }

    pub fn button(&self, v: View) -> Rect {
        self.buttons[v.index()]
    }
}

/// What a tap did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tap {
    /// Not on the UI: the tap goes to the solids.
    Scene,
    /// Handled by a button or a dialog.
    Ui,
    /// "Apply" pressed: restart with these settings.
    Apply(Settings),
    /// The palette button pressed: make new palettes.
    NewPalettes,
}

pub struct Ui {
    pub layout: Layout,
    /// Settings in force.
    pub current: Settings,
    /// The edited copy while a dialog is open, shared by all three: switching
    /// dialogs keeps the edits, and Apply applies them all.
    pub draft: Option<Settings>,
    /// The dialog showing while one is open.
    pub view: View,
    /// The palette (and colour) buttons' four squares, and how often the
    /// palette button has been pressed: each press turns them a quarter, then
    /// mirrors them, alternately.
    pub swatches: [u32; 4],
    pub swatch_presses: u32,
    /// Real seconds the buttons stay up (they hide at 0 unless a dialog is open).
    pub shown: f32,
}

impl Ui {
    pub fn new(w: i32, h: i32, current: Settings) -> Self {
        Self { layout: Layout::new(w, h), current, draft: None, view: View::Settings, swatches: SWATCHES, swatch_presses: 0, shown: 0.0 }
    }

    /// The dialog showing (or that would show).
    pub fn page(&self) -> &Page {
        self.layout.page(self.view)
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        self.layout = Layout::new(w, h);
    }

    pub fn is_open(&self) -> bool {
        self.draft.is_some()
    }

    /// Whether the buttons are showing: while a dialog is open, and for
    /// [`BUTTONS_SHOW`] seconds after a tap.
    pub fn buttons_visible(&self) -> bool {
        self.is_open() || self.shown > 0.0
    }

    /// Count down the buttons' time on screen by `dt` real seconds.
    pub fn tick(&mut self, dt: f32) {
        if !self.is_open() {
            self.shown = (self.shown - dt).max(0.0);
        }
    }

    /// Whether dialog `v` holds edits not yet applied.
    fn edited(&self, v: View) -> bool {
        self.draft.is_some_and(|d| v.fields().iter().any(|&f| d.pick[f] != self.current.pick[f]))
    }

    /// Route a tap at screen pixel (`x`, `y`).
    pub fn tap(&mut self, x: f32, y: f32) -> Tap {
        // hidden buttons: a tap brings them back; on a button it does only that
        let hidden = !self.buttons_visible();
        self.shown = BUTTONS_SHOW;
        if hidden {
            let on_button = self.layout.buttons.iter().any(|b| b.contains(x, y));
            return if on_button { Tap::Ui } else { Tap::Scene };
        }
        // a screen button opens its dialog, switches to it from another one
        // (keeping the edits), or closes it when it is the one showing
        if let Some(v) = View::ALL.into_iter().find(|&v| self.layout.button(v).contains(x, y)) {
            if self.is_open() && self.view == v {
                self.draft = None;
            } else {
                self.draft.get_or_insert(self.current);
                self.view = v;
            }
            return Tap::Ui;
        }
        let page = self.layout.page(self.view);
        let Some(d) = &mut self.draft else { return Tap::Scene };
        if page.apply.contains(x, y) {
            let s = *d;
            self.current = s;
            self.draft = None;
            return Tap::Apply(s);
        }
        for (row, &f) in page.rows.iter().zip(page.fields) {
            if row.button.is_some_and(|b| b.contains(x, y)) {
                self.swatches = if self.swatch_presses % 2 == 0 { rotate_swatches(self.swatches) } else { mirror_swatches(self.swatches) };
                self.swatch_presses += 1;
                return Tap::NewPalettes;
            }
            if row.prev.contains(x, y) {
                d.step(f, -1);
                return Tap::Ui;
            }
            if row.next.contains(x, y) {
                d.step(f, 1);
                return Tap::Ui;
            }
        }
        // Cancel, or anywhere outside the panel, closes without applying
        if page.cancel.contains(x, y) || !page.panel.contains(x, y) {
            self.draft = None;
        }
        Tap::Ui
    }

    /// Back key: closes the dialog if one is open (returns whether it
    /// handled the key).
    pub fn back(&mut self) -> bool {
        self.draft.take().is_some()
    }

    /// Draw the stats readout (`stats`, lines in the bottom-right corner),
    /// the open dialog and the three buttons.
    pub fn draw(&self, c: &mut Canvas, font: &Font, stats: &[String]) {
        let l = &self.layout;
        let ts = l.ts;
        let s = l.stats_ts;
        let line_h = CELL_H as i32 * s + s;
        let m = l.buttons[0].h / 4;
        for (k, line) in stats.iter().rev().enumerate() {
            let w = line.chars().count() as i32 * CELL_W as i32 * s - s;
            c.blocks(font, c.w - m - w, c.h - m - (k as i32 + 1) * line_h, s, line, 0xe0_e8_f4);
        }
        if let Some(d) = &self.draft {
            let page = self.page();
            c.dim(0x60);
            c.fill(page.panel, 0x10_16_20);
            c.outline(page.panel, ts.max(1), 0x5a_6a_88);
            c.text_centered(font, page.panel.x + page.panel.w / 2, page.title_y, ts, self.view.title(), 0xff_ff_ff);
            for (row, &f) in page.rows.iter().zip(page.fields) {
                c.text(font, row.label.0, row.label.1, ts, FIELDS[f].label, 0xa0_ae_c4);
                c.fill(row.value, 0x1c_24_32);
                let ty = row.label.1;
                let arrow = 0x7f_90_b0;
                c.text(font, row.value.x + CELL_W as i32 * ts / 2, ty, ts, "<", arrow);
                c.text(font, row.value.x + row.value.w - CELL_W as i32 * ts * 3 / 2, ty, ts, ">", arrow);
                // a value that differs from the one in force is highlighted
                let ink = if d.pick[f] != self.current.pick[f] { 0xff_d8_60 } else { 0xff_ff_ff };
                c.text_centered(font, row.value.x + row.value.w / 2, ty, ts, d.value(f), ink);
                if let Some(b) = row.button {
                    draw_swatch_button(c, b, ts, self.swatches, Centre::Square);
                }
            }
            for (r, label, bg) in [(page.cancel, "Cancel", 0x2a_32_42), (page.apply, "Apply", 0x2c_5a_9e)] {
                c.fill(r, bg);
                c.text_centered(font, r.x + r.w / 2, r.y + (r.h - CELL_H as i32 * ts) / 2, ts, label, 0xff_ff_ff);
            }
        }
        for v in View::ALL.into_iter().filter(|_| self.buttons_visible()) {
            let b = l.button(v);
            let showing = self.is_open() && self.view == v;
            // yellow edge: this dialog has edits waiting for Apply
            let edge = if self.edited(v) { 0xff_d8_60 } else { 0x7f_90_b0 };
            match v {
                View::Colour => draw_swatch_button(c, b, (b.w / 24).max(1), self.swatches, Centre::Circle),
                _ => c.fill(b, if showing { 0x2c_5a_9e } else { 0x1c_24_32 }),
            }
            c.outline(b, (b.w / 24).max(1), if showing && v == View::Colour { 0x2c_5a_9e } else { edge });
            match v {
                View::Settings => {
                    // three bars
                    let (bar_w, bar_h) = (b.w * 11 / 20, (b.h / 12).max(1));
                    for k in 0..3 {
                        let y = b.y + b.h * (7 + 3 * k) / 20 - bar_h / 2 + b.h / 40;
                        c.fill(Rect { x: b.x + (b.w - bar_w) / 2, y, w: bar_w, h: bar_h }, 0xe0_e8_f4);
                    }
                }
                View::Simulation => draw_cog(c, b, if showing { 0x2c_5a_9e } else { 0x1c_24_32 }),
                View::Colour => {}
            }
        }
    }
}

/// Teeth on the cogwheel.
const COG_TEETH: f32 = 8.0;

/// A cogwheel filling the middle of button `b`, its hole in `bg`.
fn draw_cog(c: &mut Canvas, b: Rect, bg: u32) {
    let (cx, cy) = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
    let s = b.w as f32;
    let (outer, inner, hole) = (s * 0.36, s * 0.27, s * 0.11);
    for y in b.y..b.y + b.h {
        for x in b.x..b.x + b.w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let tooth = (dy.atan2(dx) / std::f32::consts::TAU * COG_TEETH).rem_euclid(1.0) < 0.5;
            if r < hole {
                c.put(x, y, bg);
            } else if r <= inner || (r <= outer && tooth) {
                c.put(x, y, 0xe0_e8_f4);
            }
        }
    }
}

/// What sits in the middle of a swatch button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Centre {
    /// The palette button's [`SWATCH_CENTRE`]-pixel white square.
    Square,
    /// The colour button's [`COLOUR_DOT`]-pixel white circle.
    Circle,
}

/// The palette and colour buttons: a square with fully saturated red, blue,
/// green and yellow squares (in `swatches` order) filling it but for a
/// [`SWATCH_MARGIN`]-pixel margin, and a white square or circle in the middle.
fn draw_swatch_button(c: &mut Canvas, b: Rect, edge: i32, swatches: [u32; 4], centre: Centre) {
    c.fill(b, 0x1c_24_32);
    c.outline(b, edge.max(1), 0x5a_6a_88);
    let inner = Rect { x: b.x + SWATCH_MARGIN, y: b.y + SWATCH_MARGIN, w: b.w - 2 * SWATCH_MARGIN, h: b.h - 2 * SWATCH_MARGIN };
    let (hw, hh) = (inner.w / 2, inner.h / 2);
    for (k, &color) in swatches.iter().enumerate() {
        let (col, row) = ((k % 2) as i32, (k / 2) as i32);
        let w = if col == 0 { hw } else { inner.w - hw };
        let h = if row == 0 { hh } else { inner.h - hh };
        c.fill(Rect { x: inner.x + col * hw, y: inner.y + row * hh, w, h }, color);
    }
    let (mx, my) = (inner.x + hw, inner.y + hh);
    match centre {
        Centre::Square => {
            let s = SWATCH_CENTRE;
            c.fill(Rect { x: mx - s / 2, y: my - s / 2, w: s, h: s }, 0xff_ff_ff);
        }
        Centre::Circle => {
            let r = COLOUR_DOT_RADIUS.min(inner.w.min(inner.h) / 2);
            for dy in -r..r {
                for dx in -r..r {
                    let (fx, fy) = (dx as f32 + 0.5, dy as f32 + 0.5);
                    if fx * fx + fy * fy <= (r * r) as f32 {
                        c.put(mx + dx, my + dy, 0xff_ff_ff);
                    }
                }
            }
        }
    }
}

/// A locked RGBX_8888 window buffer (or a test buffer).
pub struct Canvas<'a> {
    pub px: &'a mut [u32],
    pub stride: usize,
    pub w: i32,
    pub h: i32,
}

impl Canvas<'_> {
    fn clip(&self, r: Rect) -> Option<(usize, usize, usize, usize)> {
        let (x0, y0) = (r.x.max(0), r.y.max(0));
        let (x1, y1) = ((r.x + r.w).min(self.w), (r.y + r.h).min(self.h));
        (x0 < x1 && y0 < y1).then_some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }

    pub fn fill(&mut self, r: Rect, color: u32) {
        let Some((x0, y0, x1, y1)) = self.clip(r) else { return };
        let ink = rgbx(color);
        for y in y0..y1 {
            if let Some(span) = self.px.get_mut(y * self.stride + x0..y * self.stride + x1) {
                span.fill(ink);
            }
        }
    }

    /// One pixel.
    pub fn put(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h
            && let Some(p) = self.px.get_mut(y as usize * self.stride + x as usize)
        {
            *p = rgbx(color);
        }
    }

    pub fn outline(&mut self, r: Rect, t: i32, color: u32) {
        self.fill(Rect { h: t, ..r }, color);
        self.fill(Rect { y: r.y + r.h - t, h: t, ..r }, color);
        self.fill(Rect { w: t, ..r }, color);
        self.fill(Rect { x: r.x + r.w - t, w: t, ..r }, color);
    }

    /// Scale every pixel's brightness by `keep / 256`.
    pub fn dim(&mut self, keep: u32) {
        let Some((x0, y0, x1, y1)) = self.clip(Rect { x: 0, y: 0, w: self.w, h: self.h }) else { return };
        for y in y0..y1 {
            for p in self.px.iter_mut().skip(y * self.stride + x0).take(x1 - x0) {
                let ch = |s: u32| (((*p >> s) & 0xff) * keep >> 8) << s;
                *p = 0xFF00_0000 | ch(16) | ch(8) | ch(0);
            }
        }
    }

    /// Text with each font pixel `ts` across, leaving `gap` pixels of each
    /// one's right and bottom edge unpainted.
    fn glyphs(&mut self, font: &Font, x: i32, y: i32, ts: i32, gap: i32, s: &str, color: u32) {
        for (i, c) in s.chars().enumerate() {
            let cx = x + i as i32 * CELL_W as i32 * ts;
            for (gy, bits) in font.get(c).iter().enumerate() {
                for gx in 0..GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - gx)) != 0 {
                        let r = Rect { x: cx + gx as i32 * ts, y: y + gy as i32 * ts, w: ts - gap, h: ts - gap };
                        self.fill(r, color);
                    }
                }
            }
        }
    }

    pub fn text(&mut self, font: &Font, x: i32, y: i32, ts: i32, s: &str, color: u32) {
        self.glyphs(font, x, y, ts, 0, s, color);
    }

    /// Block letters: text whose font pixels are separate squares, `ts`
    /// pixels apart, with a dark border so they read over the scene.
    pub fn blocks(&mut self, font: &Font, x: i32, y: i32, ts: i32, s: &str, color: u32) {
        let gap = if ts >= 3 { (ts / 4).max(1) } else { 0 };
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            self.glyphs(font, x + dx, y + dy, ts, gap, s, 0x00_00_00);
        }
        self.glyphs(font, x, y, ts, gap, s, color);
    }

    /// Text whose middle is at `cx`.
    pub fn text_centered(&mut self, font: &Font, cx: i32, y: i32, ts: i32, s: &str, color: u32) {
        let w = s.chars().count() as i32 * CELL_W as i32 * ts - ts;
        self.text(font, cx - w / 2, y, ts, s, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centre(r: Rect) -> (f32, f32) {
        ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32)
    }

    /// A UI with its buttons up, as after a first tap.
    fn shown(w: i32, h: i32, s: Settings) -> Ui {
        let mut ui = Ui::new(w, h, s);
        ui.shown = BUTTONS_SHOW;
        ui
    }

    #[test]
    fn buttons_start_hidden_and_a_tap_brings_them_back() {
        let mut ui = Ui::new(2009, 1080, Settings::default());
        assert!(!ui.buttons_visible(), "hidden at launch");
        let menu = ui.layout.buttons[0];
        assert_eq!(tap(&mut ui, menu), Tap::Ui, "a tap where a hidden button is only shows them");
        assert!(ui.buttons_visible() && !ui.is_open());
        ui.tick(BUTTONS_SHOW - 0.1);
        assert!(ui.buttons_visible());
        ui.tick(0.2);
        assert!(!ui.buttons_visible(), "gone again when left alone");
        assert_eq!(ui.tap(500.0, 500.0), Tap::Scene, "a scene tap still reaches the solids");
        assert!(ui.buttons_visible(), "and shows the buttons");
        tap(&mut ui, menu);
        ui.tick(60.0);
        assert!(ui.is_open() && ui.buttons_visible(), "they stay up while a dialog is open");
        // nothing drawn while hidden
        let (w, h, stride) = (2009, 1080, 2009);
        let mut px = vec![0u32; stride * h as usize];
        let ui = Ui::new(w, h, Settings::default());
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &Font::default(), &[]);
        assert!(px.iter().all(|&p| p == 0));
    }

    fn tap(ui: &mut Ui, r: Rect) -> Tap {
        ui.tap(centre(r).0, centre(r).1)
    }

    #[test]
    fn layout_fits_phone_screens() {
        for (w, h) in [(2009, 1080), (1280, 720), (800, 480), (1080, 2009), (320, 240)] {
            let l = Layout::new(w, h);
            let inside = |r: Rect| r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h;
            let [menu, cog, colour] = l.buttons;
            // the menu button in the top-right corner, the cogwheel just left
            // of it, the colour button just below it
            assert!(inside(menu) && menu.x > w / 2 && menu.y < h / 4, "{w}x{h}");
            assert!(inside(cog) && cog.y == menu.y && cog.x + cog.w < menu.x && menu.x - (cog.x + cog.w) <= menu.w / 2, "{w}x{h}");
            assert!(inside(colour) && colour.x == menu.x && colour.y > menu.y + menu.h && colour.y - (menu.y + menu.h) <= menu.h / 2, "{w}x{h}");
            for p in &l.pages {
                assert!(inside(p.panel), "{w}x{h}: {:?}", p.panel);
                assert_eq!(p.rows.len(), p.fields.len());
                let mut bottom = p.title_y;
                for r in &p.rows {
                    assert!(r.value.x + r.value.w <= p.panel.x + p.panel.w);
                    assert!(r.value.y >= bottom);
                    bottom = r.value.y + r.value.h;
                }
                assert!(p.apply.y >= bottom && p.apply.y + p.apply.h <= p.panel.y + p.panel.h, "{w}x{h}");
            }
            let longest = FIELDS.iter().flat_map(|f| f.choices).map(|c| c.len()).max().unwrap() as i32;
            assert!(longest + 4 <= VALUE_COLS, "values fit between the arrows");
            let title = View::ALL.iter().map(|v| v.title().len()).max().unwrap() as i32;
            assert!(title <= LABEL_COLS + VALUE_COLS, "titles fit");
        }
        // big enough to hit with a thumb on a 1080p phone
        assert!(Layout::new(2009, 1080).buttons.iter().all(|b| b.w >= 100));
    }

    #[test]
    fn dialogs_hold_the_requested_options() {
        let labels = |v: View| v.fields().iter().map(|&f| FIELDS[f].label).collect::<Vec<_>>();
        assert_eq!(
            labels(View::Simulation),
            ["Sim speed", "Framerate", "Sim stepping", "Character scale", "Collisions", "Elasticity", "Damping", "Gravity", "Mass variability"]
        );
        assert_eq!(labels(View::Colour), ["Colour mode", "Palette", "Saturation", "Wire colour", "Glow colour"]);
        assert_eq!(labels(View::Settings), ["Container", "Solids", "Wireframe", "Tap speed", "Glow", "Explosions", "Charge", "Spawn", "Dynamic camera"]);
        assert_eq!(View::Simulation.title(), "Simulation Parameters");
    }

    #[test]
    fn button_opens_dialog_and_apply_returns_the_edits() {
        let mut ui = shown(2009, 1080, Settings::default());
        assert_eq!(ui.tap(500.0, 500.0), Tap::Scene, "a tap on the scene");
        let menu = ui.layout.buttons[0];
        assert_eq!(tap(&mut ui, menu), Tap::Ui);
        assert!(ui.is_open() && ui.view == View::Settings);
        // while open, nothing reaches the scene
        let p = ui.layout.page(View::Settings).clone();
        assert_eq!(ui.tap(centre(p.panel).0, p.title_y as f32), Tap::Ui);
        assert!(ui.is_open() && ui.draft == Some(ui.current), "title tap changes nothing");
        tap(&mut ui, p.rows[0].next);
        tap(&mut ui, p.rows[0].next);
        tap(&mut ui, p.rows[2].prev);
        let Tap::Apply(s) = tap(&mut ui, p.apply) else { panic!("apply") };
        assert_eq!(s.value(0), "Octahedron");
        assert!(!s.wireframe());
        assert_eq!(ui.current, s);
        assert!(!ui.is_open());
    }

    #[test]
    fn cancel_outside_and_back_discard_edits() {
        let mut ui = shown(1280, 720, Settings::default());
        let open = |ui: &mut Ui| {
            let b = ui.layout.buttons[0];
            tap(ui, b);
            let r = ui.layout.pages[0].rows[1];
            tap(ui, r.next);
        };
        open(&mut ui);
        let c = ui.layout.pages[0].cancel;
        tap(&mut ui, c);
        assert!(!ui.is_open());
        open(&mut ui);
        ui.tap(2.0, 700.0);
        assert!(!ui.is_open(), "tap outside the panel");
        open(&mut ui);
        assert!(ui.back());
        assert!(!ui.back(), "back with nothing open exits instead");
        assert_eq!(ui.current, Settings::default());
    }

    #[test]
    fn each_button_opens_its_own_dialog_and_edits_carry_across() {
        let mut ui = shown(2009, 1080, Settings::default());
        let [menu, cog, colour] = ui.layout.buttons;
        assert_eq!(tap(&mut ui, cog), Tap::Ui);
        assert_eq!(ui.view, View::Simulation);
        // rows step the performance fields: 2x speed, 60 fps, 960 Hz, scale 2
        let rows = ui.layout.page(View::Simulation).rows.clone();
        for _ in 0..2 {
            tap(&mut ui, rows[0].next);
            tap(&mut ui, rows[1].next);
        }
        tap(&mut ui, rows[2].prev);
        tap(&mut ui, rows[2].prev);
        tap(&mut ui, rows[3].next);
        let d = ui.draft.unwrap();
        assert_eq!((d.sim_speed(), d.fps(), d.step_hz(), d.char_scale()), (2.0, 60.0, 960.0, 2.0));
        assert!(d.same_scene(&ui.current), "only performance changed");
        // the colour button switches dialogs and keeps the edits
        tap(&mut ui, colour);
        assert_eq!((ui.view, ui.draft), (View::Colour, Some(d)));
        assert!(ui.edited(View::Simulation) && !ui.edited(View::Colour));
        let c = ui.layout.page(View::Colour).rows[0];
        tap(&mut ui, c.next);
        // the showing dialog's button closes it; the others switch
        tap(&mut ui, menu);
        assert_eq!(ui.view, View::Settings);
        tap(&mut ui, menu);
        assert!(!ui.is_open());
        tap(&mut ui, cog);
        assert_eq!(ui.draft, Some(ui.current), "closing dropped the edits");
        // Apply from any dialog applies everything edited
        tap(&mut ui, rows[3].prev);
        tap(&mut ui, colour);
        tap(&mut ui, c.next);
        let apply = ui.layout.page(View::Colour).apply;
        let Tap::Apply(s) = tap(&mut ui, apply) else { panic!("apply") };
        assert_eq!((s.char_scale(), s.colour_by()), (0.5, crate::settings::ColourBy::Spin));
    }

    #[test]
    fn palette_button_sits_beside_the_palette_cycler() {
        let mut ui = shown(2009, 1080, Settings::default());
        let colour = ui.layout.buttons[2];
        tap(&mut ui, colour);
        let rows = ui.layout.page(View::Colour).rows.clone();
        let i = COLOUR_FIELDS.iter().position(|&f| f == PALETTE_FIELD).unwrap();
        let (row, b) = (rows[i], rows[i].button.unwrap());
        assert!(rows.iter().enumerate().all(|(k, r)| r.button.is_some() == (k == i)), "only the palette row has one");
        assert!(ui.layout.pages.iter().flat_map(|p| &p.rows).filter(|r| r.button.is_some()).count() == 1);
        assert_eq!((b.w, b.h), (row.value.h, row.value.h), "square, as tall as the row");
        assert!(b.x + b.w <= row.value.x && b.y == row.value.y, "left of the cycler");
        let label_end = row.label.0 + (FIELDS[PALETTE_FIELD].label.len() as i32) * CELL_W as i32 * ui.layout.ts;
        assert!(b.x > label_end, "clear of the label");
        assert!(row.value.x + row.value.w == rows[0].value.x + rows[0].value.w, "the cycler still ends with the others");
        let before = ui.draft;
        assert_eq!(tap(&mut ui, b), Tap::NewPalettes);
        assert_eq!(ui.draft, before, "it changes no setting");
        assert!(ui.is_open(), "and keeps the dialog open");
        // presses alternate: a quarter turn, a mirror, a quarter turn, ...
        let [r, bl, g, y] = SWATCHES;
        assert_eq!(ui.swatches, [g, r, y, bl], "turned a quarter clockwise");
        tap(&mut ui, b);
        assert_eq!(ui.swatches, [r, g, bl, y], "then mirrored");
        tap(&mut ui, b);
        assert_eq!(ui.swatches, rotate_swatches([r, g, bl, y]));
        tap(&mut ui, b);
        assert_eq!(ui.swatches, mirror_swatches(rotate_swatches([r, g, bl, y])));
        assert_eq!(ui.swatch_presses, 4);
        // the cycler still steps the palette
        tap(&mut ui, row.next);
        assert_eq!(ui.draft.unwrap().palette(), Some(0));
    }

    /// White pixels in the 24×24 square round (`cx`, `cy`).
    fn whites(px: &[u32], stride: usize, cx: i32, cy: i32) -> usize {
        (cy - 12..cy + 12).flat_map(|y| (cx - 12..cx + 12).map(move |x| (x, y))).filter(|&(x, y)| px[y as usize * stride + x as usize] == rgbx(0xffffff)).count()
    }

    #[test]
    fn swatch_button_draws_four_squares_inside_a_6px_margin() {
        let (w, h, stride) = (2009, 1080, 2009);
        let mut px = vec![0u32; stride * h as usize];
        let font = Font::default();
        let mut ui = shown(w, h, Settings::default());
        ui.draft = Some(Settings::default());
        ui.view = View::Colour;
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &font, &[]);
        let i = COLOUR_FIELDS.iter().position(|&f| f == PALETTE_FIELD).unwrap();
        let b = ui.layout.page(View::Colour).rows[i].button.unwrap();
        let at = |px: &[u32], x: i32, y: i32| px[y as usize * stride + x as usize];
        let m = SWATCH_MARGIN;
        let (l, r, t, bt) = (b.x + m, b.x + b.w - m - 1, b.y + m, b.y + b.h - m - 1);
        assert_eq!([at(&px, l, t), at(&px, r, t), at(&px, l, bt), at(&px, r, bt)], SWATCHES.map(rgbx), "red, blue / green, yellow");
        // the white 8x8 square in the middle
        let (cx, cy) = (l + (b.w - 2 * m) / 2, t + (b.h - 2 * m) / 2);
        assert_eq!(whites(&px, stride, cx, cy), (SWATCH_CENTRE * SWATCH_CENTRE) as usize, "an 8x8 white square");
        assert_eq!(at(&px, cx, cy), rgbx(0xffffff));
        // drawn in the button's current order
        ui.swatches = rotate_swatches(SWATCHES);
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &font, &[]);
        assert_eq!([at(&px, l, t), at(&px, r, t), at(&px, l, bt), at(&px, r, bt)], rotate_swatches(SWATCHES).map(rgbx));
        // the margin itself is button, not swatch
        for (x, y) in [(l - 1, t), (l, t - 1), (r + 1, bt), (r, bt + 1)] {
            assert!(!SWATCHES.map(rgbx).contains(&at(&px, x, y)), "margin at {x},{y}");
        }
    }

    #[test]
    fn colour_button_is_the_swatch_button_with_a_big_white_circle() {
        let (w, h, stride) = (2009, 1080, 2009);
        let mut px = vec![0u32; stride * h as usize];
        let font = Font::default();
        let ui = shown(w, h, Settings::default());
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &font, &[]);
        let b = ui.layout.buttons[2];
        let at = |x: i32, y: i32| px[y as usize * stride + x as usize];
        let m = SWATCH_MARGIN;
        let (l, r, t, bt) = (b.x + m, b.x + b.w - m - 1, b.y + m, b.y + b.h - m - 1);
        // the colours still show in the corners
        assert_eq!([at(l, t), at(r, t), at(l, bt), at(r, bt)], SWATCHES.map(rgbx));
        let cy = t + (b.h - 2 * m) / 2;
        // the circle is as wide as the squares (128 px radius doesn't fit)
        let rad = COLOUR_DOT_RADIUS.min((b.w - 2 * m) / 2);
        let row = (l..=r).filter(|&x| at(x, cy) == rgbx(0xffffff)).count() as i32;
        assert!((row - 2 * rad).abs() <= 1, "{row} across, radius {rad}");
        assert!(rad >= 50, "big: {rad}");
        let white = (t..=bt).flat_map(|y| (l..=r).map(move |x| (x, y))).filter(|&(x, y)| at(x, y) == rgbx(0xffffff)).count() as f32;
        let area = std::f32::consts::PI * (rad * rad) as f32;
        assert!((white - area).abs() < area * 0.03, "a circle's area: {white} vs {area}");
    }
    #[test]
    fn cogwheel_has_teeth_and_a_hole() {
        let (w, h, stride) = (2009, 1080, 2009);
        let mut px = vec![0u32; stride * h as usize];
        let ui = shown(w, h, Settings::default());
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &Font::default(), &[]);
        let b = ui.layout.buttons[1];
        let ink = rgbx(0xe0_e8_f4);
        let (cx, cy) = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
        let at = |r: f32, a: f32| px[(cy + r * a.sin()) as usize * stride + (cx + r * a.cos()) as usize];
        assert_ne!(at(0.0, 0.0), ink, "hole");
        assert_eq!(at(b.w as f32 * 0.2, 1.0), ink, "body");
        // round the rim, ink and gaps alternate eight times
        let rim = b.w as f32 * 0.32;
        let flips = (0..360).filter(|&d| (at(rim, (d as f32).to_radians()) == ink) != (at(rim, ((d + 1) as f32).to_radians()) == ink)).count();
        assert_eq!(flips, 16, "eight teeth");
    }

    #[test]
    fn stats_sit_in_the_bottom_right_in_block_letters() {
        let (w, h, stride) = (2009, 1080, 2009);
        let mut px = vec![0u32; stride * h as usize];
        let ui = shown(w, h, Settings::default());
        let stats = ["COLLISIONS 42".to_string(), "DESTROYED 3".to_string(), "AVG SPEED 1.50".to_string()];
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &Font::default(), &stats);
        let ink: Vec<(i32, i32)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| px[y as usize * stride + x as usize] == rgbx(0xe0_e8_f4))
            .filter(|&(x, y)| !ui.layout.buttons.iter().any(|b| b.contains(x as f32, y as f32)))
            .collect();
        assert!(!ink.is_empty());
        assert!(ink.iter().all(|&(x, y)| x > w / 2 && y > h * 3 / 4), "bottom-right quarter");
        let s = ui.layout.stats_ts;
        assert!(s >= 3, "big enough to read: {s}");
        // the blocks have gaps between them
        let (x, y) = ink[0];
        assert_ne!(px[(y + s - 1) as usize * stride + x as usize], rgbx(0xe0_e8_f4), "a gap under each block");
    }

    #[test]
    fn draws_buttons_and_dialog_inside_the_buffer() {
        let (w, h, stride) = (800, 480, 810);
        let mut px = vec![0xFF80_8080u32; stride * h as usize];
        let font = Font::default();
        let mut ui = shown(w, h, Settings::default());
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &font, &[]);
        let b = ui.layout.buttons[0];
        assert_eq!(px[(b.y as usize + 1) * stride + b.x as usize + 1], rgbx(0x7f_90_b0), "button outline");
        assert_eq!(px[stride * 10 + 10], 0xFF80_8080, "scene untouched when closed");
        ui.draft = Some(Settings::default());
        ui.draw(&mut Canvas { px: &mut px, stride, w, h }, &font, &[]);
        assert_eq!(px[stride * 10 + 10], 0xFF30_3030, "scene dimmed when open");
        for y in 0..h as usize {
            assert!(px[y * stride + w as usize..(y + 1) * stride].iter().all(|&p| p == 0xFF80_8080), "row {y} padding");
        }
        let panel_ink = px.iter().filter(|&&p| p == rgbx(0xff_ff_ff)).count();
        assert!(panel_ink > 200, "text drawn: {panel_ink}");
    }
}
