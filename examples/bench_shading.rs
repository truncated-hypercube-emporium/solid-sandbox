//! What each shader and ambient-light option costs per frame: the raymarch
//! (path-traced) shader with and without glow and with 1 to 3 ambient-light
//! rays per cell, and the optimized (rasterized, dithered) shader with and
//! without ambient light.
//!
//!   cargo run --release --example bench_shading
//!   $env:RAYON_NUM_THREADS = 4; cargo run --release --example bench_shading   # fewer cores, like a phone

use std::time::Instant;

use solid_sandbox::sandbox::{Sandbox, render_settings};
use solid_sandbox::settings::{Glow, Settings, Shader};

/// The grid a 2009×1080 phone gets at pixel scale 2 (12×16-pixel cells).
const COLS: usize = 167;
const ROWS: usize = 67;
const FRAMES: usize = 40;

/// Sixteen solids in a cube, kicked and left to collide for a moment, so
/// they glow and crowd each other.
fn scene() -> Sandbox {
    let mut s = Settings::default();
    (s.pick[0], s.pick[1]) = (1, 13);
    let mut sb = Sandbox::with_settings(42, &s);
    sb.world.finish_growth();
    let mut rng = fastrand::Rng::with_seed(9);
    for i in 0..sb.world.bodies.len() {
        sb.world.kick(i, &mut rng);
    }
    for _ in 0..45 {
        sb.step(1.0 / 30.0);
    }
    sb
}

/// Mean milliseconds per frame, rendering the same moment `FRAMES` times.
fn time(sb: &mut Sandbox) -> f64 {
    let rs = render_settings(0.75);
    for _ in 0..3 {
        sb.render(COLS, ROWS, &rs);
    }
    let t0 = Instant::now();
    for _ in 0..FRAMES {
        sb.render(COLS, ROWS, &rs);
    }
    t0.elapsed().as_secs_f64() * 1e3 / FRAMES as f64
}

fn main() {
    let threads = std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "all".into());
    println!("{COLS}x{ROWS} cells, 16 solids, {FRAMES} frames each, {threads} threads\n");
    let cases: [(&str, Shader, Glow, bool, usize); 8] = [
        ("raymarch", Shader::Raymarch, Glow::Off, false, 0),
        ("raymarch + glow", Shader::Raymarch, Glow::Low, false, 0),
        ("raymarch + glow + ambient, 1 ray", Shader::Raymarch, Glow::Low, true, 1),
        ("raymarch + glow + ambient, 2 rays", Shader::Raymarch, Glow::Low, true, 2),
        ("raymarch + glow + ambient, 3 rays", Shader::Raymarch, Glow::Low, true, 3),
        ("optimized", Shader::Optimized, Glow::Off, false, 0),
        ("optimized + ambient", Shader::Optimized, Glow::Off, true, 0),
        ("optimized + glow + ambient", Shader::Optimized, Glow::Low, true, 0),
    ];
    let mut base = None;
    let mut glow_base = None;
    for (name, shader, glow, ambient, rays) in cases {
        let mut sb = scene();
        (sb.shader, sb.glow, sb.ambient, sb.ao_rays) = (shader, glow, ambient, rays.max(1));
        let ms = time(&mut sb);
        let base = *base.get_or_insert(ms);
        if name == "raymarch + glow" {
            glow_base = Some(ms);
        }
        let over_glow = match (shader, ambient, glow_base) {
            (Shader::Raymarch, true, Some(g)) => format!("  (+{:.0}% over raymarch + glow)", (ms / g - 1.0) * 100.0),
            _ => String::new(),
        };
        println!("{name:<36} {ms:7.2} ms/frame  {:6.3} us/cell  {:5.2}x raymarch{over_glow}", ms * 1e3 / (COLS * ROWS) as f64, ms / base);
    }
}
