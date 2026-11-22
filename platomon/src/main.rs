use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use crossterm::{cursor, event, execute, terminal};

use platomon::ascii::{self, Ramp, StatsLine};
use platomon::aspect::{self, AspectMode, PRESETS};
use platomon::clock::{self, FrameClock, SPIN_MARGIN};
use platomon::color::{self, ColorBy, ColorDepth, ColorMap, Gradient};
use platomon::collector::{Collector, gpu::GpuSampler, system::SystemSampler};
use platomon::config::{Config, SlotSpec, split_assignment};
use platomon::metrics::{Channel, MetricKind, MetricsState, Reading, activity_score};
use platomon::overlay::{LegendPos, OverlayOptions, Overlays};
use platomon::render::{QualityGovernor, RenderSettings, Renderer, TraceSettings};
use platomon::scene::camera::{CameraMode, CameraParams, OrbitDir};
use platomon::scene::ground::{Pattern, Spin, parse_vec2};
use platomon::scene::sdf::{Polyhedron, SolidKind};
use platomon::scene::{AnimParams, Layout, Scene, SlotLayout};
use platomon::ui::{self, Action};

/// Longest sleep between input polls while waiting for a frame deadline.
const INPUT_SLICE: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ListWhat {
    Solids,
    Metrics,
    Aspects,
    Sensors,
    Patterns,
    Gradients,
}

/// Platonic-solid ASCII system monitor.
///
/// One path-traced solid per metric, on concentric rings. Keys: s = stats bar,
/// l = legend, w = wireframe, q / Esc / Ctrl-C = quit.
#[derive(Parser, Debug)]
#[command(name = "platomon", version, about, long_about = None)]
struct Cli {
    /// Config file (TOML). Default: <config dir>/platomon/config.toml if present.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Print solids, metrics, aspect presets or this machine's disks/adapters, then exit.
    #[arg(long, value_name = "WHAT")]
    list: Option<ListWhat>,
    /// Print the effective configuration as TOML and exit.
    #[arg(long)]
    print_config: bool,
    /// Render one frame to stdout (e.g. 120x40) and exit.
    #[arg(long, value_name = "COLSxROWS")]
    snapshot: Option<String>,
    /// Seconds of animation (camera, background motion) to simulate before the snapshot.
    #[arg(long, value_name = "SECS", default_value_t = 0.3)]
    snapshot_time: f32,
    /// Append one CSV line per second (fps, quality, render time, grid) to PATH while running.
    #[arg(long, value_name = "PATH")]
    perf_log: Option<PathBuf>,
    /// Show the stats bar at start.
    #[arg(long)]
    stats: bool,

    // ---- metrics & solids
    /// Metrics to show, one solid each (max 16): cpu,ram,disk,net,gpu,nic[:NAME],disk-dev[:NAME].
    #[arg(long, value_delimiter = ',', value_name = "METRIC[,METRIC...]")]
    show: Option<Vec<String>>,
    /// Solid for a metric (repeatable). `nic=...` applies to every nic slot.
    #[arg(long, value_name = "METRIC=SOLID")]
    solid: Vec<String>,
    /// Ring for a metric, 0 = innermost (repeatable).
    #[arg(long, value_name = "METRIC=N")]
    ring: Vec<String>,

    // ---- rendering
    /// Samples per pixel.
    #[arg(long, value_name = "N")]
    spp: Option<u32>,
    /// Indirect bounces.
    #[arg(long, value_name = "N")]
    bounces: Option<u32>,
    /// Frame-rate floor (idle).
    #[arg(long, value_name = "FPS")]
    fps_min: Option<f32>,
    /// Frame-rate ceiling (full load).
    #[arg(long, value_name = "FPS")]
    fps_max: Option<f32>,
    /// Invert the load/fps relationship: idle runs at --fps-max, full load at --fps-min.
    #[arg(long)]
    fps_invert: bool,
    /// Glyph ramp, darkest first.
    #[arg(long, value_name = "STR")]
    ramp: Option<String>,
    /// Terminal cell width / height.
    #[arg(long, value_name = "FLOAT")]
    char_aspect: Option<f32>,

    // ---- overlays
    /// Show the legend: a wireframe of each solid with its metric (key l).
    #[arg(long)]
    legend: bool,
    /// Legend font size 1..5: 1-2 terminal text, 3-5 block letters (1-3 cells per pixel). Implies --legend.
    #[arg(long, value_name = "1..5")]
    legend_size: Option<u8>,
    /// Legend background opacity 0..1 (0 = see-through, 1 = solid). Implies --legend.
    #[arg(long, value_name = "0..1")]
    legend_opacity: Option<f32>,
    /// Legend corner: tl, tr, bl, br (or top-left, ...). Implies --legend.
    #[arg(long, value_name = "CORNER")]
    legend_pos: Option<LegendPos>,
    /// Overlay each solid's wireframe in inverted luminosity (key w).
    #[arg(long)]
    wireframe: bool,

    // ---- colour
    /// Colour gradient: off, a preset (see --list gradients) or hex stops like "#000:#f80:#fff".
    #[arg(long, value_name = "GRADIENT")]
    color: Option<String>,
    /// What picks the colour: lum (brightness) or load (each solid by its metric). Implies a default gradient.
    #[arg(long, value_name = "lum|load")]
    color_by: Option<ColorBy>,
    /// Colour codes: 24bit or 256.
    #[arg(long, value_name = "24bit|256")]
    color_depth: Option<ColorDepth>,
    /// Run the gradient backwards.
    #[arg(long)]
    color_reverse: bool,

    // ---- layout
    /// rings (on the ground) or sphere (concentric spherical shells; --ring picks the shell).
    #[arg(long, value_name = "rings|sphere")]
    layout: Option<Layout>,

    // ---- display ratio
    /// Picture ratio: fill, a preset (vga, wxga, fhd, uwqhd, ... see --list aspects) or W:H.
    #[arg(long, value_name = "PRESET|W:H")]
    aspect: Option<String>,
    /// letterbox (blank margins) or crop (fill the terminal, cut overflow).
    #[arg(long, value_name = "MODE")]
    aspect_mode: Option<AspectMode>,

    // ---- camera
    /// static, orbit (circle the centroid of the solids) or focus (glide toward the busiest solid).
    #[arg(long, value_name = "MODE")]
    camera: Option<CameraMode>,
    /// Focus camera: 0 = only turn toward the busiest solid, 1 = close-up.
    #[arg(long, value_name = "0..1")]
    focus_zoom: Option<f32>,
    /// Focus camera: glide time constant, seconds.
    #[arg(long, value_name = "SECS")]
    focus_tau: Option<f32>,
    /// Seconds per orbit.
    #[arg(long, value_name = "SECS")]
    orbit_period: Option<f32>,
    /// random, cw or ccw (seen from above).
    #[arg(long, value_name = "DIR")]
    orbit_dir: Option<OrbitDir>,
    /// Orbit radius in world units (default: fit the layout).
    #[arg(long, value_name = "FLOAT")]
    orbit_radius: Option<f32>,
    /// Camera height (default: fit the layout).
    #[arg(long, value_name = "FLOAT")]
    orbit_height: Option<f32>,
    /// Vertical drift while orbiting; 0 = level circle.
    #[arg(long, value_name = "FLOAT")]
    orbit_bob: Option<f32>,
    /// Seed for random choices (orbit direction).
    #[arg(long, value_name = "U64")]
    seed: Option<u64>,

    // ---- background (ground plane)
    /// Background luminosity 0..10: 5 = original, 0 = no background surface.
    #[arg(long, value_name = "0..10")]
    bg_lum: Option<f32>,
    /// Background pattern: solid, checker, diamond, brick, triangles, hex, radial.
    #[arg(long, value_name = "PATTERN")]
    bg_pattern: Option<Pattern>,
    /// Pattern tone spread around the luminosity, 0..1.
    #[arg(long, value_name = "0..1")]
    bg_contrast: Option<f32>,
    /// Tile size in world units (a full-size solid is 2 across).
    #[arg(long, value_name = "FLOAT")]
    bg_scale: Option<f32>,
    /// Constant pattern translation, world units/s, e.g. 0.5:0 or -0.2:0.3.
    #[arg(long, value_name = "DX:DZ", allow_hyphen_values = true)]
    bg_scroll: Option<String>,
    /// Radius of a circular back-and-forth translation; 0 = off.
    #[arg(long, value_name = "FLOAT")]
    bg_sway: Option<f32>,
    /// Seconds per sway cycle.
    #[arg(long, value_name = "SECS")]
    bg_sway_period: Option<f32>,
    /// Rotate the background about the camera's focal point: off, cw, ccw.
    #[arg(long, value_name = "DIR")]
    bg_rotate: Option<Spin>,
    /// Seconds per background revolution.
    #[arg(long, value_name = "SECS")]
    bg_rotate_period: Option<f32>,
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("platomon: {msg}");
    std::process::exit(2);
}

fn apply_cli(cfg: &mut Config, cli: &Cli) -> Result<(), String> {
    macro_rules! set {
        ($($f:ident),*) => { $( if let Some(v) = cli.$f.clone() { cfg.$f = v; } )* };
    }
    set!(spp, bounces, fps_min, fps_max, ramp, char_aspect, aspect, aspect_mode, camera, orbit_period, orbit_dir, orbit_bob);
    set!(bg_lum, bg_pattern, bg_contrast, bg_scale, bg_sway, bg_sway_period, bg_rotate, bg_rotate_period);
    set!(legend_size, legend_opacity, legend_pos);
    set!(color, color_by, color_depth, layout, focus_zoom, focus_tau);
    if cli.color_reverse {
        cfg.color_reverse = true;
    }
    // Choosing a colour source (or reversing) without a gradient picks a default one.
    if (cli.color_by.is_some() || cli.color_reverse) && cli.color.is_none() && cfg.color.trim() == "off" {
        cfg.color = platomon::color::default_for(cfg.color_by).to_string();
    }
    if cli.legend || cli.legend_size.is_some() || cli.legend_opacity.is_some() || cli.legend_pos.is_some() {
        cfg.legend = true;
    }
    if cli.wireframe {
        cfg.wireframe = true;
    }
    if cli.fps_invert {
        cfg.fps_invert = true;
    }
    if let Some(s) = &cli.bg_scroll {
        cfg.bg_scroll = parse_vec2(s).map_err(|e| format!("--bg-scroll: {e}"))?;
    }
    if cli.orbit_radius.is_some() {
        cfg.orbit_radius = cli.orbit_radius;
    }
    if cli.orbit_height.is_some() {
        cfg.orbit_height = cli.orbit_height;
    }
    if cli.seed.is_some() {
        cfg.seed = cli.seed;
    }
    if let Some(show) = &cli.show {
        cfg.show = show.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    for a in &cli.solid {
        let (k, v) = split_assignment(a).map_err(|e| format!("--solid: {e}"))?;
        cfg.solids.insert(k, v);
    }
    for a in &cli.ring {
        let (k, v) = split_assignment(a).map_err(|e| format!("--ring: {e}"))?;
        let n: u32 = v.parse().map_err(|_| format!("--ring {a}: ring must be a number 0..=15"))?;
        if n > 15 {
            return Err(format!("--ring {a}: ring must be 0..=15"));
        }
        cfg.rings.insert(k, n);
    }
    if cli.stats {
        cfg.show_stats = true;
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let mut cfg = Config::load(cli.config.as_deref()).unwrap_or_else(|e| fail(e));
    apply_cli(&mut cfg, &cli).unwrap_or_else(|e| fail(e));
    cfg.sanitize();

    if let Some(what) = cli.list {
        list(what);
        return;
    }
    if cli.print_config {
        print!("{}", cfg.to_toml());
        return;
    }
    let slots = cfg.resolve_slots().unwrap_or_else(|e| fail(e));
    let ratio = aspect::parse_aspect(&cfg.aspect).unwrap_or_else(|e| fail(e));

    if let Some(spec) = &cli.snapshot {
        let Some((w, h)) = spec.split_once(['x', 'X']).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        else {
            fail("--snapshot expects COLSxROWS, e.g. 120x40");
        };
        println!("{}", snapshot(&cfg, &slots, ratio, (w, h), cli.snapshot_time));
        return;
    }
    let cmap = cfg.color_map().unwrap_or_else(|e| fail(format!("--color: {e}")));

    let perf = match cli.perf_log.as_deref().map(PerfLog::create).transpose() {
        Ok(p) => p,
        Err(e) => fail(format!("--perf-log: {e}")),
    };
    if let Err(e) = run(&cfg, &slots, ratio, perf, cmap.as_ref()) {
        restore_terminal();
        eprintln!("platomon: {e}");
        std::process::exit(1);
    }
}

// ------------------------------------------------------------------ --list

fn list(what: ListWhat) {
    match what {
        ListWhat::Solids => {
            println!("{:<26} {:>5} {:>8}  family", "SOLID", "FACES", "VERTICES");
            for (i, k) in SolidKind::ALL.iter().enumerate() {
                let p = Polyhedron::new(*k);
                let family = match i {
                    0..=4 => "Platonic",
                    13 | 14 => "Catalan",
                    _ => "Archimedean",
                };
                println!("{:<26} {:>5} {:>8}  {family}", k.id(), p.face_planes().len(), p.vertices().len());
            }
        }
        ListWhat::Metrics => {
            println!("{:<18} {:<24} {:<42} SPIN", "METRIC", "DEFAULT SOLID", "SCALE");
            for k in MetricKind::ALL {
                let (scale, spin) = k.describe();
                let id = if k.has_instance() { format!("{}[:NAME]", k.id()) } else { k.id().to_string() };
                println!("{:<18} {:<24} {:<42} {spin}", id, k.default_solid().id(), scale);
            }
            println!("\nNAME: see --list sensors. Without NAME the busiest instance is used.");
        }
        ListWhat::Patterns => {
            println!("{:<10} DESCRIPTION", "PATTERN");
            for p in Pattern::ALL {
                println!("{:<10} {}", p.id(), p.describe());
            }
            println!("\nTiles alternate around --bg-lum by --bg-contrast; --bg-scale sets tile size.");
        }
        ListWhat::Gradients => {
            println!("{:<9} {:<32} DESCRIPTION", "GRADIENT", "SWATCH (24-bit)");
            for (id, desc, stops) in color::PRESETS {
                let g = Gradient::from_stops(stops);
                let mut bar = String::new();
                for i in 0..32 {
                    color::push_sgr(&mut bar, g.sample(i as f32 / 31.0, 1.0), ColorDepth::True);
                    bar.push('█');
                }
                color::push_sgr(&mut bar, color::NONE, ColorDepth::True);
                println!("{id:<9} {bar} {desc}");
            }
            println!("\nOr 2+ hex stops, dark/idle first: --color \"#000:#f80:#fff\". --color-reverse flips any gradient.");
        }
        ListWhat::Aspects => {
            println!("{:<8} {:>11}  RATIO", "PRESET", "RESOLUTION");
            for (id, w, h) in PRESETS {
                let res = if id == "square" { "-".to_string() } else { format!("{w}x{h}") };
                println!("{id:<8} {res:>11}  {:.3}", w as f32 / h as f32);
            }
            println!("\nAlso: fill (default) or any W:H, e.g. --aspect 21:9");
        }
        ListWhat::Sensors => {
            let mut sys = SystemSampler::new();
            sys.sample(0.25); // prime counters
            std::thread::sleep(Duration::from_millis(1000));
            let m = sys.sample(1.0);
            let mut gpu = GpuSampler::new();
            let g = gpu.sample();
            println!("GPU backend: {} ({})", gpu.backend_name(), g.map_or("n/a".into(), |g| format!("{:.0}%", g.util)));
            println!("CPU: {}", m.cpu_total.map_or("n/a".into(), |c| format!("{c:.0}%")));
            println!("\nDisks (use with disk-dev:NAME):");
            if m.disks.is_empty() {
                println!("  none with I/O counters (disk-dev will show n/a)");
            }
            for d in &m.disks {
                let alt = d.alt.as_deref().map(|a| format!("  [{a}]")).unwrap_or_default();
                println!("  {:<24}{alt:<14} r{:>8.2} w{:>8.2} MB/s", d.name, d.a_bps / 1e6, d.b_bps / 1e6);
            }
            println!("\nNetwork adapters (use with nic:NAME):");
            if m.nics.is_empty() {
                println!("  none");
            }
            for n in &m.nics {
                println!("  {:<38} ↓{:>8.2} ↑{:>8.2} MB/s", n.name, n.a_bps / 1e6, n.b_bps / 1e6);
            }
        }
    }
}

// ------------------------------------------------------------------ setup helpers

fn anim_params(cfg: &Config) -> AnimParams {
    AnimParams { min_scale: cfg.min_scale, max_scale: cfg.max_scale, idle_spin: cfg.idle_spin, max_spin: cfg.max_spin }
}

fn camera_params(cfg: &Config) -> CameraParams {
    CameraParams {
        mode: cfg.camera,
        period: cfg.orbit_period,
        dir: cfg.orbit_dir,
        radius: cfg.orbit_radius,
        height: cfg.orbit_height,
        bob: cfg.orbit_bob,
        seed: cfg.seed,
        focus_zoom: cfg.focus_zoom,
        focus_tau: cfg.focus_tau,
    }
}

fn build_scene(cfg: &Config, slots: &[SlotSpec]) -> Scene {
    Scene::with_layout(&layouts(slots), &anim_params(cfg), &camera_params(cfg), cfg.layout)
        .with_ground(&cfg.ground_params())
}

fn render_settings(cfg: &Config, ratio: Option<f32>) -> RenderSettings {
    RenderSettings {
        spp: cfg.spp,
        trace: TraceSettings { bounces: cfg.bounces, max_steps: cfg.max_steps, max_dist: cfg.max_dist },
        char_aspect: cfg.char_aspect,
        fit_aspect: ratio,
    }
}

fn layouts(slots: &[SlotSpec]) -> Vec<SlotLayout> {
    slots.iter().map(|s| SlotLayout { solid: s.solid, ring: s.ring }).collect()
}

/// One frame with every slot at a fixed demo level.
fn snapshot(cfg: &Config, slots: &[SlotSpec], ratio: Option<f32>, (w, h): (usize, usize), secs: f32) -> String {
    let anim = anim_params(cfg);
    let cmap = cfg.color_map().unwrap_or_else(|e| fail(format!("--color: {e}")));
    let mut scene = build_scene(cfg, slots);
    let levels = [0.7, 0.5, 0.3, 0.9, 0.0, 0.6, 0.4, 0.8];
    let ch: Vec<Channel> = (0..slots.len())
        .map(|i| {
            let l = levels[i % levels.len()];
            Channel { available: true, level: l, spin_y: l, spin_x: 0.2, activity: l }
        })
        .collect();
    // Step in ≤0.1 s increments, the way the live loop would.
    let secs = if secs.is_finite() { secs.clamp(0.0, 86_400.0) } else { 0.3 };
    let steps = (secs / 0.1).ceil().max(1.0) as usize;
    for _ in 0..steps {
        scene.update(&ch, &anim, secs / steps as f32);
    }
    let vp = aspect::viewport(w, h, ratio, cfg.aspect_mode, cfg.char_aspect);
    let mut r = Renderer::new();
    r.render(&scene, vp.w, vp.h, &render_settings(cfg, ratio));
    let labels: Vec<String> =
        slots.iter().zip(&ch).map(|(s, c)| format!("{} {:.0}%", s.metric.key().to_uppercase(), c.level * 100.0)).collect();
    let legend = cfg.legend_params();
    let opt = OverlayOptions {
        wireframe: cfg.wireframe,
        legend: cfg.legend.then_some(&legend),
        labels: &labels,
        char_aspect: cfg.char_aspect,
        ramp: &Ramp::new(&cfg.ramp),
        exposure: cfg.exposure,
        color: cmap.as_ref(),
        levels: &ch.iter().map(|c| c.level).collect::<Vec<_>>(),
    };
    let mut overlays = Overlays::new(&scene);
    let mut s = String::new();
    if let Some(view) = r.view() {
        let cells = overlays.compose(&view, vp.w, vp.h, &scene, &opt);
        let colors = cells.colors.zip(cmap.as_ref().map(|m| m.depth));
        ascii::compose_plain(cells.chars, colors, vp.w, vp.h, &mut s);
    }
    s
}

/// Legend labels from live readings (compact for block letters).
fn legend_labels(readings: &[&Reading], size: u8, out: &mut Vec<String>) {
    out.clear();
    out.extend(readings.iter().map(|r| if size >= 3 { ui::short_label(r) } else { ui::segment(r) }));
}

// ------------------------------------------------------------------ terminal

fn restore_terminal() {
    let mut out = io::stdout();
    let _ = execute!(
        out,
        crossterm::style::ResetColor,
        terminal::EnableLineWrap,
        cursor::Show,
        terminal::LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self; // restores even if the next line fails
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::DisableLineWrap,
            terminal::Clear(terminal::ClearType::All)
        )?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Restore the terminal before the panic message is printed. The collector
/// thread catches its own panics, so it is left alone; anything else
/// (main, rayon workers) is fatal.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("platomon-collector") {
            return;
        }
        restore_terminal();
        default(info);
    }));
}

// ------------------------------------------------------------------ main loop

/// One frame's timings for [`PerfLog`].
struct PerfSample {
    /// Render + ASCII compose, seconds.
    render: f32,
    /// Write + flush to the terminal, seconds.
    write: f32,
    bytes: usize,
    /// Time spent waiting for the frame deadline (sleep + spin + input), seconds.
    wait: f32,
    /// How far past the deadline the wait actually ended, seconds.
    late: f32,
    achieved: f32,
    target: f32,
    quality: u8,
}

/// Once-per-second CSV of what the frame loop is doing.
struct PerfLog {
    out: io::BufWriter<std::fs::File>,
    start: Instant,
    window: Instant,
    frames: u32,
    render_sum: f32,
    render_max: f32,
    write_sum: f32,
    bytes_sum: usize,
    wait_sum: f32,
    late_sum: f32,
    late_max: f32,
}

impl PerfLog {
    fn create(path: &std::path::Path) -> io::Result<Self> {
        let mut out = io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(
            out,
            "t_s,frames,achieved_fps,target_fps,quality,render_ms_avg,render_ms_max,write_ms_avg,wait_ms_avg,late_ms_avg,late_ms_max,kb_per_frame,cols,rows,solids"
        )?;
        out.flush()?;
        let now = Instant::now();
        Ok(Self {
            out,
            start: now,
            window: now,
            frames: 0,
            render_sum: 0.0,
            render_max: 0.0,
            write_sum: 0.0,
            bytes_sum: 0,
            wait_sum: 0.0,
            late_sum: 0.0,
            late_max: 0.0,
        })
    }

    fn frame(&mut self, s: &PerfSample, cols: u16, rows: u16, solids: usize) {
        self.frames += 1;
        self.render_sum += s.render;
        self.render_max = self.render_max.max(s.render);
        self.write_sum += s.write;
        self.bytes_sum += s.bytes;
        self.wait_sum += s.wait;
        self.late_sum += s.late;
        self.late_max = self.late_max.max(s.late);
        if self.window.elapsed() >= Duration::from_secs(1) {
            let n = self.frames as f32;
            let _ = writeln!(
                self.out,
                "{:.1},{},{:.1},{:.1},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.1},{},{},{}",
                self.start.elapsed().as_secs_f32(),
                self.frames,
                s.achieved,
                s.target,
                s.quality,
                1000.0 * self.render_sum / n,
                1000.0 * self.render_max,
                1000.0 * self.write_sum / n,
                1000.0 * self.wait_sum / n,
                1000.0 * self.late_sum / n,
                1000.0 * self.late_max,
                self.bytes_sum as f32 / n / 1024.0,
                cols,
                rows,
                solids
            );
            let _ = self.out.flush();
            self.window = Instant::now();
            self.frames = 0;
            self.render_sum = 0.0;
            self.render_max = 0.0;
            self.write_sum = 0.0;
            self.bytes_sum = 0;
            self.wait_sum = 0.0;
            self.late_sum = 0.0;
            self.late_max = 0.0;
        }
    }
}

fn run(
    cfg: &Config,
    slots: &[SlotSpec],
    ratio: Option<f32>,
    mut perf: Option<PerfLog>,
    cmap: Option<&ColorMap>,
) -> io::Result<()> {
    install_panic_hook();
    let collector = Collector::spawn(cfg.poll_hz);
    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout().lock();

    let (mut cols, mut rows) = terminal::size()?;
    let mut show_stats = cfg.show_stats;
    let mut need_clear = true;

    let anim = anim_params(cfg);
    let base = render_settings(cfg, ratio);
    let ramp = Ramp::new(&cfg.ramp);
    let specs: Vec<_> = slots.iter().map(|s| s.metric.clone()).collect();

    let mut scene = build_scene(cfg, slots);
    let mut metrics = MetricsState::new(cfg, &specs);
    let mut levels: Vec<f32> = Vec::new();
    let weights = metrics.weights(&cfg.weights);
    let mut fps = FrameClock::new(cfg.fps_min, cfg.fps_max, cfg.fps_curve, cfg.fps_slew).inverted(cfg.fps_invert);
    let mut renderer = Renderer::new();
    let mut governor = QualityGovernor::default();
    let mut frame = String::with_capacity(cols as usize * rows as usize * 2 + 1024);
    let mut overlays = Overlays::new(&scene);
    let legend = cfg.legend_params();
    let mut show_legend = cfg.legend;
    let mut wireframe = cfg.wireframe;
    let mut labels: Vec<String> = Vec::new();

    loop {
        // Wait for the frame deadline, staying responsive to input. Input is
        // polled without blocking: a blocking `event::poll(timeout)` rounds up
        // to the OS timer tick (15.6 ms on Windows), capping fps at ~64, while
        // `thread::sleep` uses a high-resolution timer.
        let deadline = fps.next_deadline();
        let wait_start = Instant::now();
        loop {
            while event::poll(Duration::ZERO)? {
                match ui::map_event(&event::read()?) {
                    Action::Quit => return Ok(()),
                    Action::ToggleStats => {
                        show_stats = !show_stats;
                        need_clear = true;
                    }
                    Action::ToggleLegend => show_legend = !show_legend,
                    Action::ToggleWireframe => wireframe = !wireframe,
                    Action::Resize(w, h) => {
                        (cols, rows) = (w, h);
                        need_clear = true;
                    }
                    Action::None => {}
                }
            }
            let now = Instant::now();
            if now + SPIN_MARGIN >= deadline {
                break;
            }
            std::thread::sleep((deadline - SPIN_MARGIN - now).min(INPUT_SLICE));
        }
        clock::spin_until(deadline);
        let woke = Instant::now();
        let wait_secs = woke.duration_since(wait_start).as_secs_f32();
        let late_secs = woke.saturating_duration_since(deadline).as_secs_f32();

        // Resize events are not delivered by every terminal (e.g. some ConPTY
        // hosts), so also poll the size; a change triggers a full clear.
        if let Ok(size) = terminal::size()
            && size != (cols, rows)
        {
            (cols, rows) = size;
            need_clear = true;
        }

        let dt = fps.begin_frame();
        if let Some(raw) = collector.latest() {
            metrics.ingest(&raw);
        }
        metrics.tick(dt);
        let channels = metrics.channels();
        fps.update_target(activity_score(&channels, &weights), dt);
        scene.update(&channels, &anim, dt);
        levels.clear();
        levels.extend(channels.iter().map(|c| if c.available { c.level } else { 0.0 }));

        let area_h = (rows as usize).saturating_sub(show_stats as usize);
        let vp = aspect::viewport(cols as usize, area_h, ratio, cfg.aspect_mode, cfg.char_aspect);
        let settings = if cfg.adaptive_quality { governor.apply(base) } else { base };

        let t0 = Instant::now();
        renderer.render(&scene, vp.w, vp.h, &settings);
        let readings = metrics.readings();
        let stats_text = show_stats.then(|| {
            let extra = if governor.level() > 0 { format!(" | q-{}", governor.level()) } else { String::new() };
            ui::stats_line_fit(&readings, fps.achieved_fps(), fps.target_fps(), &extra, cols as usize)
        });
        let stats = stats_text.as_deref().map(|text| StatsLine { text, row: area_h, width: cols as usize });
        if show_legend {
            legend_labels(&readings, legend.size, &mut labels);
        }
        let opt = OverlayOptions {
            wireframe,
            legend: show_legend.then_some(&legend),
            labels: &labels,
            char_aspect: cfg.char_aspect,
            ramp: &ramp,
            exposure: cfg.exposure,
            color: cmap,
            levels: &levels,
        };
        if let Some(view) = renderer.view() {
            let cells = overlays.compose(&view, vp.w, vp.h, &scene, &opt);
            let colors = cells.colors.zip(cmap.map(|m| m.depth));
            ascii::compose_terminal(cells.chars, colors, &vp, (cols as usize, area_h), stats, &mut frame);
        }
        let render_secs = t0.elapsed().as_secs_f32();

        if need_clear {
            // Belt and braces: the frame already overwrites every cell.
            frame.insert_str(0, "\x1b[2J");
            need_clear = false;
        }
        let tw = Instant::now();
        out.write_all(frame.as_bytes())?;
        out.flush()?;
        let write_secs = tw.elapsed().as_secs_f32();

        governor.update(render_secs, fps.target_fps(), dt);
        if let Some(p) = perf.as_mut() {
            let s = PerfSample {
                render: render_secs,
                write: write_secs,
                bytes: frame.len(),
                wait: wait_secs,
                late: late_secs,
                achieved: fps.achieved_fps(),
                target: fps.target_fps(),
                quality: governor.level(),
            };
            p.frame(&s, cols, rows, slots.len());
        }
    }
}
