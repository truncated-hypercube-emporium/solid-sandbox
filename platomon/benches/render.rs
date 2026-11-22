//! Render benchmark: 120×40 cells, default settings (2 spp, 2 bounces).
//! Acceptance target: ≥150 fps ⇒ ≤6.67 ms per frame on a modern multi-core CPU.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use platomon::ascii::{self, Ramp};
use platomon::aspect::Viewport;
use platomon::color::{ColorBy, ColorDepth, ColorMap, Gradient};
use platomon::metrics::Channel;
use platomon::overlay::{LegendParams, LegendPos, OverlayOptions, Overlays};
use platomon::render::{QualityGovernor, RenderSettings, Renderer};
use platomon::scene::ground::{GroundParams, Pattern, Spin};
use platomon::scene::sdf::SolidKind;
use platomon::scene::{AnimParams, CameraParams, Scene, SlotLayout};

fn scene_at(level: f32) -> Scene {
    let anim = AnimParams::default();
    let mut s = Scene::demo(&anim);
    let ch = [Channel { available: true, level, spin_y: level, spin_x: level * 0.3, activity: level }; 5];
    s.update(&ch, &anim, 0.37);
    s
}

/// 16 solids on two rings, one of each of the 15 kinds (+1 repeat).
fn scene_16(level: f32) -> Scene {
    let anim = AnimParams::default();
    let slots: Vec<SlotLayout> =
        (0..16).map(|i| SlotLayout { solid: SolidKind::ALL[i % SolidKind::ALL.len()], ring: None }).collect();
    let mut s = Scene::new(&slots, &anim, &CameraParams::default());
    let ch = vec![Channel { available: true, level, spin_y: level, spin_x: level * 0.3, activity: level }; 16];
    s.update(&ch, &anim, 0.37);
    s
}

fn bench(c: &mut Criterion) {
    let (w, h) = (120usize, 40usize);
    let settings = RenderSettings::default();
    let ramp = Ramp::default();

    for (name, level) in [("render_120x40_idle", 0.0), ("render_120x40_full", 1.0)] {
        let scene = scene_at(level);
        let mut r = Renderer::new();
        c.bench_function(name, |b| b.iter(|| black_box(r.render(black_box(&scene), w, h, &settings)).len()));
    }

    let scene = scene_16(1.0);
    let mut r = Renderer::new();
    c.bench_function("render_120x40_16_solids_full", |b| {
        b.iter(|| black_box(r.render(black_box(&scene), w, h, &settings)).len())
    });

    // WQHD maximum: 640×240 cells (4×6 px console font on 2560×1440), 16 solids,
    // rotating checkerboard, at each adaptive-quality level.
    let (bw, bh) = (640usize, 240usize);
    let bg = GroundParams {
        lum: 5.0,
        pattern: Pattern::Checker,
        contrast: 0.7,
        scale: 1.5,
        rotate: Spin::Cw,
        rotate_period: 30.0,
        ..GroundParams::default()
    };
    let mut big = scene_16(1.0).with_ground(&bg);
    let anim = AnimParams::default();
    let ch = vec![Channel { available: true, level: 1.0, spin_y: 1.0, spin_x: 0.3, activity: 1.0 }; 16];
    big.update(&ch, &anim, 3.0); // 3 s into the rotation
    let wqhd = RenderSettings { char_aspect: 4.0 / 6.0, ..RenderSettings::default() };
    let mut gov = QualityGovernor::default();
    for q in 0..=QualityGovernor::MAX_LEVEL {
        let s = gov.apply(wqhd);
        let mut r = Renderer::new();
        c.bench_function(&format!("wqhd_640x240_16_solids_checker_rot_q{q}_spp{}_b{}", s.spp, s.trace.bounces), |b| {
            b.iter(|| black_box(r.render(black_box(&big), bw, bh, &s)).len())
        });
        gov.force_level(q + 1);
    }
    let mut r = Renderer::new();
    let mut out = String::with_capacity(bw * bh * 2);
    let vp = Viewport { x: 0, y: 0, w: bw, h: bh, fit: None };
    let buf = r.render(&big, bw, bh, &wqhd).to_vec();
    c.bench_function("wqhd_640x240_ascii_compose_only", |b| {
        b.iter(|| {
            let cells = ascii::glyphs(&buf, &ramp, 1.0);
            ascii::compose_terminal(&cells, None, &vp, (bw, bh), None, &mut out);
            black_box(out.len())
        })
    });
    // Wireframe + size-5 legend over that frame (overlay cost only).
    let mut overlays = Overlays::new(&big);
    let labels: Vec<String> = (0..16).map(|i| format!("NIC ETH{i} 12.5M")).collect();
    let legend = LegendParams { size: 5, opacity: 0.85, pos: LegendPos::TopLeft };
    let opt = OverlayOptions {
        wireframe: true,
        legend: Some(&legend),
        labels: &labels,
        char_aspect: 4.0 / 6.0,
        ramp: &ramp,
        exposure: 1.0,
        color: None,
        levels: &[],
    };
    c.bench_function("wqhd_640x240_overlays_wireframe_legend", |b| {
        b.iter(|| {
            let view = r.view().unwrap();
            black_box(overlays.compose(&view, bw, bh, &big, &opt).chars.len())
        })
    });
    // Colour: lum gradient composed to 24-bit escapes.
    let cm = ColorMap { gradient: Gradient::parse("inferno").unwrap(), by: ColorBy::Lum, depth: ColorDepth::True };
    let levels = vec![0.5; 16];
    let copt = OverlayOptions { wireframe: false, legend: None, color: Some(&cm), levels: &levels, ..opt };
    let mut out = String::with_capacity(bw * bh * 8);
    c.bench_function("wqhd_640x240_color_lum_compose", |b| {
        b.iter(|| {
            let view = r.view().unwrap();
            let cells = overlays.compose(&view, bw, bh, &big, &copt);
            ascii::compose_terminal(cells.chars, cells.colors.map(|c| (c, cm.depth)), &vp, (bw, bh), None, &mut out);
            black_box(out.len())
        })
    });

    let scene = scene_at(1.0);
    let mut r = Renderer::new();
    let mut out = String::with_capacity(w * h * 2);
    let vp = Viewport { x: 0, y: 0, w, h, fit: None };
    c.bench_function("frame_120x40_full_with_ascii", |b| {
        b.iter(|| {
            let buf = r.render(&scene, w, h, &settings);
            let cells = ascii::glyphs(buf, &ramp, 1.0);
            ascii::compose_terminal(&cells, None, &vp, (w, h), None, &mut out);
            black_box(out.len())
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
