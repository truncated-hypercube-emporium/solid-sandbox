//! The app without its platform: settings and palettes on disk, the
//! sandbox, the cell grid and the UI, fed taps, time and accelerometer
//! readings, drawing into a pixel buffer. The Android glue (`native`) and the
//! iOS glue (`ios`) both drive one of these.

use std::path::PathBuf;

use crate::font::{CELL_H, CELL_W, Font};
use crate::palette::Palettes;
use crate::sandbox::{self, Sandbox};
use crate::settings::Settings;
use crate::ui::{Canvas, Tap, Ui};
use crate::{Grid, char_scale, draw_row, scale_for};
use platomon::render::RenderSettings;

/// A fresh seed from the clock.
fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |t| t.as_nanos() as u64)
}

pub struct App {
    pub sb: Sandbox,
    pub ui: Ui,
    grid: Grid,
    palettes: Palettes,
    settings_file: Option<PathBuf>,
    palette_file: Option<PathBuf>,
    rs: RenderSettings,
    /// Measured render cost, seconds per cell, and the cell scale it gives
    /// before the "Character scale" setting.
    spc: f32,
    base_scale: i32,
    font: Font,
    log: fn(&str),
}

impl App {
    /// A `w × h` pixel app, keeping `settings.txt` and `palettes.txt` in
    /// `data_dir` (nothing is saved without one); `log` gets one line per event.
    pub fn new(w: i32, h: i32, data_dir: Option<PathBuf>, log: fn(&str)) -> Self {
        let settings_file = data_dir.as_ref().map(|d| d.join("settings.txt"));
        let settings = settings_file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map_or_else(Settings::default, |t| Settings::from_text(&t));
        // the palettes last between sessions: made once, then only when the
        // palette button is pressed
        let palette_file = data_dir.map(|d| d.join("palettes.txt"));
        let loaded = palette_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| Palettes::from_text(&t));
        let fresh = loaded.is_none();
        let palettes = loaded.unwrap_or_else(|| Palettes::generate(seed()));
        let mut sb = Sandbox::with_settings(seed(), &settings);
        sb.set_live(&settings, Some(&palettes));
        let rs = sandbox::render_settings(CELL_W as f32 / CELL_H as f32);
        let spc = sandbox::seconds_per_cell(&mut sb, &rs);
        let base_scale = scale_for(w, h, spc);
        let scale = char_scale(base_scale, settings.char_scale());
        log(&format!("{w}x{h}, {} solids in a {:?}, {:.2} us/cell -> scale {scale}", sb.world.bodies.len(), sb.world.shape, spc * 1e6));
        let app = Self {
            sb,
            ui: Ui::new(w, h, settings),
            grid: Grid::new(w, h, scale),
            palettes,
            settings_file,
            palette_file,
            rs,
            spc,
            base_scale,
            font: Font::default(),
            log,
        };
        if fresh {
            app.save_palettes();
            log("made new palettes");
        }
        app
    }

    fn save_palettes(&self) {
        if let Some(f) = &self.palette_file
            && let Err(e) = std::fs::write(f, self.palettes.to_text())
        {
            (self.log)(&format!("saving palettes failed: {e}"));
        }
    }

    /// The frame-rate cap in force, frames per second.
    pub fn fps(&self) -> f32 {
        self.ui.current.fps()
    }

    /// Whether gravity is on, so the accelerometer is worth reading.
    pub fn wants_accel(&self) -> bool {
        self.ui.current.gravity() > 0.0
    }

    /// The pixel size the app is laid out for.
    pub fn size(&self) -> (i32, i32) {
        (self.grid.w, self.grid.h)
    }

    /// Re-fit the cells and the UI to a `w × h` window (no-op if unchanged).
    pub fn resize(&mut self, w: i32, h: i32) {
        if (w, h) == self.size() {
            return;
        }
        self.base_scale = scale_for(w, h, self.spc);
        let scale = char_scale(self.base_scale, self.ui.current.char_scale());
        (self.log)(&format!("window {w}x{h} -> scale {scale}"));
        self.grid = Grid::new(w, h, scale);
        self.ui.resize(w, h);
    }

    /// Back key (or gesture): closes a dialog; `false` if none was open, so
    /// the platform may leave the app.
    pub fn back(&mut self) -> bool {
        self.ui.back()
    }

    /// A finger going down at window pixel (`x`, `y`).
    pub fn tap(&mut self, x: f32, y: f32) {
        let before = self.ui.current;
        match self.ui.tap(x, y) {
            Tap::Scene => {
                let (gx, gy) = self.grid.to_grid(x, y);
                let hit = self.sb.tap(gx, gy);
                (self.log)(&format!("tap {x:.0},{y:.0} -> {}", hit.map_or("miss".to_string(), |i| format!("solid {i}"))));
            }
            Tap::Ui => (self.log)(&format!("tap {x:.0},{y:.0} -> ui")),
            Tap::NewPalettes => {
                self.palettes = Palettes::generate(seed());
                self.save_palettes();
                // the scene picks up the new colours straight away
                self.sb.set_live(&self.ui.current, Some(&self.palettes));
                (self.log)("new palettes");
            }
            Tap::Apply(s) => {
                if let Some(f) = &self.settings_file
                    && let Err(e) = std::fs::write(f, s.to_text())
                {
                    (self.log)(&format!("saving settings failed: {e}"));
                }
                if !s.same_scene(&before) {
                    self.sb = Sandbox::with_settings(seed(), &s);
                }
                // colours, explosions and performance apply to the running scene
                self.sb.set_live(&s, Some(&self.palettes));
                let scale = char_scale(self.base_scale, s.char_scale());
                if scale != self.grid.scale {
                    self.grid = Grid::new(self.grid.w, self.grid.h, scale);
                    (self.log)(&format!("character scale {} -> scale {scale}", s.char_scale()));
                }
                (self.log)(&format!("applied: {}", s.to_text().replace('\n', " ")));
            }
        }
    }

    /// Advance by `dt` real seconds with this accelerometer reading (raw,
    /// m/s² in Android's device axes, and the display rotation 0..3), then
    /// render the cells.
    pub fn step(&mut self, dt: f32, accel: Option<([f32; 3], i32)>) {
        self.ui.tick(dt);
        self.sb.set_accel(accel);
        self.sb.step(dt);
        self.sb.render(self.grid.cols, self.grid.rows, &self.rs);
    }

    /// Draw the last rendered frame and the UI into a `buf_w × buf_h`
    /// RGBX_8888 buffer (red in the low byte) with `stride` pixels per row.
    pub fn draw(&self, px: &mut [u32], stride: usize, buf_w: i32, buf_h: i32) {
        px.fill(0xFF00_0000);
        let g = &self.grid;
        if buf_w >= g.w && buf_h >= g.h && self.sb.chars.len() == g.cols * g.rows {
            for row in 0..g.rows {
                let r = row * g.cols..(row + 1) * g.cols;
                draw_row(px, stride, g, &self.font, row, &self.sb.chars[r.clone()], &self.sb.colors[r]);
            }
        }
        let (w, h) = (g.w.min(buf_w), g.h.min(buf_h));
        self.ui.draw(&mut Canvas { px, stride, w, h }, &self.font, &self.sb.stats());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_taps_and_draws_without_a_platform() {
        let dir = std::env::temp_dir().join(format!("solid-sandbox-app-{}", seed()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (800, 480);
        let mut app = App::new(w, h, Some(dir.clone()), |_| {});
        assert!(dir.join("palettes.txt").exists(), "palettes saved on first run");
        for _ in 0..3 {
            app.step(1.0 / 30.0, None);
        }
        let mut px = vec![0u32; w as usize * h as usize];
        app.draw(&mut px, w as usize, w, h);
        assert!(px.iter().any(|&p| p != 0xFF00_0000), "something drawn");
        // open the menu (the first tap only shows the buttons), step a row, apply
        let b = app.ui.layout.buttons[0];
        let centre = |r: crate::ui::Rect| ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
        let (bx, by) = centre(b);
        app.tap(bx, by);
        app.tap(bx, by);
        assert!(app.ui.is_open());
        let page = app.ui.page().clone();
        let (nx, ny) = centre(page.rows[2].next);
        app.tap(nx, ny);
        let (ax, ay) = centre(page.apply);
        app.tap(ax, ay);
        assert!(!app.ui.is_open() && !app.ui.current.wireframe());
        let saved = std::fs::read_to_string(dir.join("settings.txt")).unwrap();
        assert!(saved.contains("wireframe=Off"));
        // a new app reads them back
        let again = App::new(w, h, Some(dir.clone()), |_| {});
        assert!(!again.ui.current.wireframe());
        // resizing re-fits; back closes nothing when nothing is open
        app.resize(1280, 720);
        assert_eq!(app.size(), (1280, 720));
        assert!(!app.back());
        let _ = std::fs::remove_dir_all(dir);
    }
}
