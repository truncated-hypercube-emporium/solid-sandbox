//! Configuration: defaults, TOML loading, sanitising and slot resolution.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::aspect::AspectMode;
use crate::clock::FpsCurve;
use crate::color::{ColorBy, ColorDepth, ColorMap, Gradient};
use crate::scene::Layout;
use crate::metrics::{MetricKind, MetricSpec};
use crate::overlay::{LEGEND_SIZE_MAX, LEGEND_SIZE_MIN, LegendParams, LegendPos};
use crate::scene::camera::{CameraMode, OrbitDir};
use crate::scene::ground::{self, GroundParams, Pattern, Spin};
use crate::scene::sdf::SolidKind;

/// Most solids shown at once.
pub const MAX_SLOTS: usize = 16;

/// Per-metric-kind weights for the activity score (applies to every instance).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Weights {
    pub cpu: f32,
    pub ram: f32,
    pub disk: f32,
    pub net: f32,
    pub gpu: f32,
    pub nic: f32,
    pub disk_dev: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Self { cpu: 1.0, ram: 1.0, disk: 1.0, net: 1.0, gpu: 1.0, nic: 1.0, disk_dev: 1.0 }
    }
}

impl Weights {
    pub fn get(&self, k: MetricKind) -> f32 {
        match k {
            MetricKind::Cpu => self.cpu,
            MetricKind::Ram => self.ram,
            MetricKind::Disk => self.disk,
            MetricKind::Net => self.net,
            MetricKind::Gpu => self.gpu,
            MetricKind::Nic => self.nic,
            MetricKind::DiskDev => self.disk_dev,
        }
    }
}

/// Every tunable, with defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    // --- what to show ---
    /// Metrics, one solid each: `cpu`, `ram`, `disk`, `net`, `gpu`, `nic[:NAME]`, `disk-dev[:NAME]`.
    pub show: Vec<String>,

    // --- frame rate ---
    pub fps_min: f32,
    pub fps_max: f32,
    pub fps_curve: FpsCurve,
    /// Maximum change of the target fps, in fps per second.
    pub fps_slew: f32,
    /// Invert the load → fps mapping: idle runs at `fps_max`, full load at `fps_min`.
    pub fps_invert: bool,

    // --- renderer ---
    pub spp: u32,
    pub bounces: u32,
    pub max_steps: u32,
    pub max_dist: f32,
    /// Drop spp / bounces when the frame budget is missed.
    pub adaptive_quality: bool,

    // --- output ---
    pub ramp: String,
    /// Width / height of one terminal cell.
    pub char_aspect: f32,
    pub exposure: f32,
    pub show_stats: bool,
    /// `fill`, a preset (`vga`, `wxga`, `fhd`, `uwqhd`, ...) or `W:H`.
    pub aspect: String,
    pub aspect_mode: AspectMode,

    // --- overlays ---
    /// Legend of solid wireframes and their metrics (key `l`).
    pub legend: bool,
    /// 1..=5: 1–2 terminal text, 3–5 block letters; icons grow with it.
    pub legend_size: u8,
    /// Legend background: 0 = see-through, 1 = solid.
    pub legend_opacity: f32,
    /// top-left, top-right, bottom-left, bottom-right.
    pub legend_pos: LegendPos,
    /// Inverted-luminosity wireframe over every solid (key `w`).
    pub wireframe: bool,

    // --- colour ---
    /// `off`, a gradient preset (`inferno`, `viridis`, `traffic`, ...) or hex
    /// stops like `"#000:#f80:#fff"`.
    pub color: String,
    /// `lum`: colour by brightness; `load`: each solid by its metric's load.
    pub color_by: ColorBy,
    /// `24bit` or `256`.
    pub color_depth: ColorDepth,
    pub color_reverse: bool,

    // --- layout ---
    /// `rings` (on the ground) or `sphere` (concentric spherical shells).
    pub layout: Layout,

    // --- camera ---
    pub camera: CameraMode,
    /// Seconds per revolution.
    pub orbit_period: f32,
    pub orbit_dir: OrbitDir,
    /// World units; unset = fit the layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orbit_radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orbit_height: Option<f32>,
    /// Vertical drift amplitude while orbiting.
    pub orbit_bob: f32,
    /// Focus camera: 0 = only turn toward the busiest solid, 1 = close-up.
    pub focus_zoom: f32,
    /// Focus camera: glide time constant, seconds.
    pub focus_tau: f32,
    /// RNG seed for random choices (orbit direction); unset = random.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,

    // --- background (ground plane) ---
    /// 0 = no background surface, 5 = original, 10 = brightest.
    pub bg_lum: f32,
    pub bg_pattern: Pattern,
    /// Tile tone spread around `bg_lum`, 0..=1.
    pub bg_contrast: f32,
    /// Tile size, world units.
    pub bg_scale: f32,
    /// Constant pattern translation [x, z], world units/s.
    pub bg_scroll: [f32; 2],
    /// Radius of a circular back-and-forth translation; 0 = none.
    pub bg_sway: f32,
    pub bg_sway_period: f32,
    /// Rotation about the camera's focal point: off, cw, ccw.
    pub bg_rotate: Spin,
    /// Seconds per revolution.
    pub bg_rotate_period: f32,

    // --- animation ---
    pub min_scale: f32,
    pub max_scale: f32,
    /// rad/s at full activity.
    pub max_spin: f32,
    /// rad/s at zero activity.
    pub idle_spin: f32,
    /// EMA time constant for all smoothed signals, seconds.
    pub smoothing_tau: f32,

    // --- collector / metrics ---
    pub poll_hz: f32,
    /// Floor for rolling peaks of rate metrics, bytes/s.
    pub rate_floor_bytes: f64,
    /// Fraction of the rolling peak lost per second.
    pub peak_decay_per_sec: f64,

    // --- tables (must stay last for TOML) ---
    /// metric key → solid id, e.g. `cpu = "truncated-icosahedron"`.
    pub solids: BTreeMap<String, String>,
    /// metric key → ring index (0 = innermost).
    pub rings: BTreeMap<String, u32>,
    pub weights: Weights,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show: ["cpu", "ram", "disk", "net", "gpu"].map(String::from).to_vec(),
            fps_min: 5.0,
            fps_max: 150.0,
            fps_curve: FpsCurve::Sqrt,
            fps_slew: 60.0,
            fps_invert: false,
            spp: 2,
            bounces: 2,
            max_steps: 64,
            max_dist: 50.0,
            adaptive_quality: true,
            ramp: crate::ascii::DEFAULT_RAMP.to_string(),
            char_aspect: 0.5,
            exposure: 1.0,
            show_stats: false,
            aspect: "fill".to_string(),
            aspect_mode: AspectMode::Letterbox,
            legend: false,
            legend_size: LegendParams::default().size,
            legend_opacity: LegendParams::default().opacity,
            legend_pos: LegendPos::TopLeft,
            wireframe: false,
            color: "off".to_string(),
            color_by: ColorBy::Lum,
            color_depth: ColorDepth::True,
            color_reverse: false,
            layout: Layout::Rings,
            focus_zoom: 0.6,
            focus_tau: 1.5,
            camera: CameraMode::Static,
            orbit_period: 60.0,
            orbit_dir: OrbitDir::Random,
            orbit_radius: None,
            orbit_height: None,
            orbit_bob: 0.3,
            seed: None,
            bg_lum: ground::LUM_REF,
            bg_pattern: Pattern::Solid,
            bg_contrast: 0.5,
            bg_scale: 1.0,
            bg_scroll: [0.0, 0.0],
            bg_sway: 0.0,
            bg_sway_period: 10.0,
            bg_rotate: Spin::Off,
            bg_rotate_period: 60.0,
            min_scale: 0.45,
            max_scale: 1.0,
            max_spin: 3.0,
            idle_spin: 0.15,
            smoothing_tau: 0.35,
            poll_hz: 4.0,
            rate_floor_bytes: 1_000_000.0,
            peak_decay_per_sec: 0.01,
            solids: BTreeMap::new(),
            rings: BTreeMap::new(),
            weights: Weights::default(),
        }
    }
}

/// One displayed solid.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotSpec {
    pub metric: MetricSpec,
    pub solid: SolidKind,
    /// Requested ring; `None` = automatic.
    pub ring: Option<u32>,
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// Find the table entry for `spec`: an exact key wins over a kind-wide key
/// (`nic` applies to every `nic:*` slot).
fn lookup<'a, V>(table: &'a [(MetricSpec, V)], spec: &MetricSpec) -> Option<&'a V> {
    table
        .iter()
        .find(|(k, _)| k == spec)
        .or_else(|| table.iter().find(|(k, _)| k.instance.is_none() && k.kind == spec.kind))
        .map(|(_, v)| v)
}

impl Config {
    /// `<platform config dir>/platomon/config.toml`
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("platomon").join("config.toml"))
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Load from an explicit path (must exist), else the default path if it
    /// exists, else defaults.
    pub fn load(explicit: Option<&Path>) -> Result<Self, String> {
        let path = match explicit {
            Some(p) => p.to_path_buf(),
            None => match Self::default_path() {
                Some(p) if p.is_file() => p,
                _ => return Ok(Self::default()),
            },
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
        Self::from_toml(&text).map_err(|e| format!("invalid config {}: {e}", path.display()))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_else(|e| format!("# cannot serialise config: {e}\n"))
    }

    /// Clamp everything into a usable range. Never fails.
    pub fn sanitize(&mut self) {
        let d = Self::default();
        self.fps_min = finite_or(self.fps_min, d.fps_min).max(1.0);
        self.fps_max = finite_or(self.fps_max, d.fps_max).max(self.fps_min);
        self.fps_slew = finite_or(self.fps_slew, d.fps_slew).max(0.1);

        self.spp = self.spp.clamp(1, 64);
        self.bounces = self.bounces.min(8);
        self.max_steps = self.max_steps.clamp(8, 1024);
        self.max_dist = finite_or(self.max_dist, d.max_dist).clamp(5.0, 1000.0);

        if self.ramp.chars().count() < 2 {
            self.ramp = d.ramp.clone();
        }
        self.char_aspect = finite_or(self.char_aspect, d.char_aspect).clamp(0.1, 4.0);
        self.exposure = finite_or(self.exposure, d.exposure).clamp(0.01, 100.0);

        self.legend_size = self.legend_size.clamp(LEGEND_SIZE_MIN, LEGEND_SIZE_MAX);
        self.legend_opacity = finite_or(self.legend_opacity, d.legend_opacity).clamp(0.0, 1.0);

        self.focus_zoom = finite_or(self.focus_zoom, d.focus_zoom).clamp(0.0, 1.0);
        self.focus_tau = finite_or(self.focus_tau, d.focus_tau).clamp(0.05, 60.0);
        if self.color.trim().is_empty() {
            self.color = d.color.clone();
        }

        self.orbit_period = finite_or(self.orbit_period, d.orbit_period).clamp(2.0, 86_400.0);
        self.orbit_bob = finite_or(self.orbit_bob, d.orbit_bob).clamp(0.0, 10.0);

        self.bg_lum = finite_or(self.bg_lum, d.bg_lum).clamp(0.0, ground::LUM_MAX);
        self.bg_contrast = finite_or(self.bg_contrast, d.bg_contrast).clamp(0.0, 1.0);
        self.bg_scale = finite_or(self.bg_scale, d.bg_scale).clamp(0.05, 100.0);
        for v in &mut self.bg_scroll {
            *v = finite_or(*v, 0.0).clamp(-100.0, 100.0);
        }
        self.bg_sway = finite_or(self.bg_sway, d.bg_sway).clamp(0.0, 100.0);
        self.bg_sway_period = finite_or(self.bg_sway_period, d.bg_sway_period).clamp(0.5, 86_400.0);
        self.bg_rotate_period = finite_or(self.bg_rotate_period, d.bg_rotate_period).clamp(0.5, 86_400.0);
        for v in [&mut self.orbit_radius, &mut self.orbit_height] {
            *v = v.filter(|x| x.is_finite()).map(|x| x.clamp(0.5, 500.0));
        }

        self.min_scale = finite_or(self.min_scale, d.min_scale).clamp(0.05, 1.0);
        self.max_scale = finite_or(self.max_scale, d.max_scale).clamp(self.min_scale, 1.0);
        self.idle_spin = finite_or(self.idle_spin, d.idle_spin).max(0.0);
        self.max_spin = finite_or(self.max_spin, d.max_spin).max(self.idle_spin);
        self.smoothing_tau = finite_or(self.smoothing_tau, d.smoothing_tau).clamp(0.01, 10.0);

        self.poll_hz = finite_or(self.poll_hz, d.poll_hz).clamp(0.5, 20.0);
        if !self.rate_floor_bytes.is_finite() || self.rate_floor_bytes < 1.0 {
            self.rate_floor_bytes = d.rate_floor_bytes;
        }
        if !self.peak_decay_per_sec.is_finite() {
            self.peak_decay_per_sec = d.peak_decay_per_sec;
        }
        self.peak_decay_per_sec = self.peak_decay_per_sec.clamp(0.0, 0.99);

        let w = &mut self.weights;
        for v in [&mut w.cpu, &mut w.ram, &mut w.disk, &mut w.net, &mut w.gpu, &mut w.nic, &mut w.disk_dev] {
            *v = finite_or(*v, 1.0).max(0.0);
        }
    }

    pub fn ground_params(&self) -> GroundParams {
        GroundParams {
            lum: self.bg_lum,
            pattern: self.bg_pattern,
            contrast: self.bg_contrast,
            scale: self.bg_scale,
            scroll: self.bg_scroll,
            sway: self.bg_sway,
            sway_period: self.bg_sway_period,
            rotate: self.bg_rotate,
            rotate_period: self.bg_rotate_period,
        }
    }

    /// The colour mapping, or `None` for monochrome (`color = "off"`).
    pub fn color_map(&self) -> Result<Option<ColorMap>, String> {
        if matches!(self.color.trim().to_ascii_lowercase().as_str(), "off" | "none" | "mono") {
            return Ok(None);
        }
        let g = Gradient::parse(&self.color)?;
        let gradient = if self.color_reverse { g.reversed() } else { g };
        Ok(Some(ColorMap { gradient, by: self.color_by, depth: self.color_depth }))
    }

    pub fn legend_params(&self) -> LegendParams {
        LegendParams { size: self.legend_size, opacity: self.legend_opacity, pos: self.legend_pos }
    }

    /// Resolve `show`, `solids` and `rings` into slots. Errors are meant for the user.
    pub fn resolve_slots(&self) -> Result<Vec<SlotSpec>, String> {
        let mut specs: Vec<MetricSpec> = Vec::new();
        for s in &self.show {
            let spec = MetricSpec::parse(s)?;
            if specs.contains(&spec) {
                return Err(format!("metric '{}' listed twice in show", spec.key()));
            }
            specs.push(spec);
        }
        if specs.is_empty() {
            return Err("nothing to show: --show needs at least one metric".into());
        }
        if specs.len() > MAX_SLOTS {
            return Err(format!("at most {MAX_SLOTS} metrics can be shown (got {})", specs.len()));
        }

        let parse_keys = |keys: Vec<&String>, what: &str| -> Result<Vec<MetricSpec>, String> {
            keys.into_iter()
                .map(|k| {
                    let spec = MetricSpec::parse(k).map_err(|e| format!("{what} '{k}': {e}"))?;
                    let used = specs.iter().any(|s| *s == spec || (spec.instance.is_none() && s.kind == spec.kind));
                    if used { Ok(spec) } else { Err(format!("{what} '{k}': metric is not in show")) }
                })
                .collect()
        };

        let solid_keys = parse_keys(self.solids.keys().collect(), "solid for")?;
        let mut solids = Vec::new();
        for (spec, id) in solid_keys.into_iter().zip(self.solids.values()) {
            let kind = SolidKind::from_id(id).ok_or_else(|| format!("unknown solid '{id}' (see --list solids)"))?;
            solids.push((spec, kind));
        }
        let ring_keys = parse_keys(self.rings.keys().collect(), "ring for")?;
        let rings: Vec<(MetricSpec, u32)> = ring_keys.into_iter().zip(self.rings.values().copied()).collect();

        Ok(specs
            .into_iter()
            .map(|m| SlotSpec {
                solid: lookup(&solids, &m).copied().unwrap_or(m.kind.default_solid()),
                ring: lookup(&rings, &m).copied(),
                metric: m,
            })
            .collect())
    }
}

/// Split `KEY=VALUE` at the last `=` (keys like `disk-dev:C:\` contain no `=`).
pub fn split_assignment(s: &str) -> Result<(String, String), String> {
    let (k, v) = s.rsplit_once('=').ok_or_else(|| format!("expected METRIC=VALUE, got '{s}'"))?;
    let (k, v) = (k.trim(), v.trim());
    if k.is_empty() || v.is_empty() {
        return Err(format!("expected METRIC=VALUE, got '{s}'"));
    }
    Ok((k.to_string(), v.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_through_toml() {
        let mut c = Config::default();
        c.solids.insert("cpu".into(), "cube".into());
        c.rings.insert("ram".into(), 1);
        c.orbit_radius = Some(12.0);
        let back = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let c = Config::from_toml("spp = 4\ncamera = \"orbit\"\n[weights]\ngpu = 0.0\n[solids]\n\"nic:eth0\" = \"cube\"\n")
            .unwrap();
        assert_eq!(c.spp, 4);
        assert_eq!(c.camera, CameraMode::Orbit);
        assert_eq!(c.weights.gpu, 0.0);
        assert_eq!(c.weights.cpu, 1.0);
        assert_eq!(c.solids["nic:eth0"], "cube");
        assert_eq!(c.show.len(), 5);
    }

    #[test]
    fn unknown_keys_rejected() {
        assert!(Config::from_toml("nope = 1").is_err());
        assert!(Config::from_toml("camera = \"spin\"").is_err());
    }

    #[test]
    fn fps_bounds_clamped() {
        let mut c = Config { fps_min: 0.0, fps_max: -3.0, ..Config::default() };
        c.sanitize();
        assert_eq!((c.fps_min, c.fps_max), (1.0, 1.0));
        let mut c = Config { fps_min: 200.0, fps_max: 100.0, ..Config::default() };
        c.sanitize();
        assert!(c.fps_min == 200.0 && c.fps_min <= c.fps_max);
    }

    #[test]
    fn bad_values_replaced() {
        let mut c = Config { ramp: "x".into(), spp: 0, orbit_radius: Some(f32::NAN), ..Config::default() };
        c.sanitize();
        assert_eq!(c.ramp, Config::default().ramp);
        assert_eq!(c.spp, 1);
        assert_eq!(c.orbit_radius, None);
    }

    #[test]
    fn default_slots() {
        let s = Config::default().resolve_slots().unwrap();
        let solids: Vec<_> = s.iter().map(|x| x.solid).collect();
        assert_eq!(
            solids,
            [
                SolidKind::Icosahedron,
                SolidKind::Cube,
                SolidKind::Octahedron,
                SolidKind::Tetrahedron,
                SolidKind::Dodecahedron
            ]
        );
        assert!(s.iter().all(|x| x.ring.is_none()));
    }

    #[test]
    fn slot_overrides_and_errors() {
        let mut c = Config { show: vec!["cpu".into(), "nic:eth0".into(), "nic:wlan0".into()], ..Config::default() };
        c.solids.insert("nic".into(), "cube".into());
        c.solids.insert("nic:wlan0".into(), "rhombic-triacontahedron".into());
        c.rings.insert("NIC".into(), 2);
        let s = c.resolve_slots().unwrap();
        assert_eq!(s[1].solid, SolidKind::Cube, "kind-wide key");
        assert_eq!(s[2].solid, SolidKind::RhombicTriacontahedron, "exact key wins");
        assert_eq!((s[0].ring, s[1].ring, s[2].ring), (None, Some(2), Some(2)));

        let bad = |f: &dyn Fn(&mut Config)| {
            let mut c = Config::default();
            f(&mut c);
            c.resolve_slots().unwrap_err()
        };
        assert!(bad(&|c| c.show = vec!["cpu".into(), "cpu".into()]).contains("twice"));
        assert!(bad(&|c| c.show.clear()).contains("nothing"));
        assert!(bad(&|c| c.show = vec!["cpu-temp".into()]).contains("unknown metric"));
        assert!(bad(&|c| c.show = (0..17).map(|i| format!("nic:n{i}")).collect()).contains("at most"));
        assert!(bad(&|c| _ = c.solids.insert("cpu".into(), "sphere".into())).contains("unknown solid"));
        assert!(bad(&|c| _ = c.solids.insert("nic".into(), "cube".into())).contains("not in show"));
        assert!(bad(&|c| _ = c.rings.insert("gpu:0".into(), 1)).contains("takes no"));
    }

    #[test]
    fn background_settings() {
        let c = Config::from_toml(
            "bg_lum = 0.0\nbg_pattern = \"hex\"\nbg_scroll = [0.5, -1.0]\nbg_rotate = \"cw\"\n",
        )
        .unwrap();
        let g = c.ground_params();
        assert_eq!((g.lum, g.pattern, g.scroll, g.rotate), (0.0, Pattern::Hex, [0.5, -1.0], Spin::Cw));
        assert!(Config::from_toml("bg_pattern = \"plaid\"").is_err());
        let mut c = Config { bg_lum: 50.0, bg_contrast: -1.0, bg_scroll: [f32::NAN, 3.0], ..Config::default() };
        c.sanitize();
        assert_eq!((c.bg_lum, c.bg_contrast, c.bg_scroll), (10.0, 0.0, [0.0, 3.0]));
        assert_eq!(Config::default().bg_lum, 5.0, "default reproduces the original ground");
    }

    #[test]
    fn colour_layout_and_focus_settings() {
        assert_eq!(Config::default().color_map().unwrap(), None);
        let c = Config::from_toml(
            "color = \"viridis\"\ncolor_by = \"load\"\ncolor_depth = \"256\"\ncolor_reverse = true\nlayout = \"sphere\"\ncamera = \"focus\"\nfocus_zoom = 0.9\n",
        )
        .unwrap();
        let m = c.color_map().unwrap().unwrap();
        assert_eq!((m.by, m.depth), (ColorBy::Load, ColorDepth::Ansi256));
        assert_eq!(m.gradient.sample(0.0, 1.0), 0xfde725, "reversed: starts at viridis' yellow");
        assert_eq!((c.layout, c.camera, c.focus_zoom), (Layout::Sphere, CameraMode::Focus, 0.9));
        assert!(Config { color: "plaid".into(), ..Config::default() }.color_map().is_err());
        assert!(Config::from_toml("layout = \"cube\"").is_err());
        let back = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn overlay_settings() {
        let c = Config::from_toml("legend = true\nlegend_pos = \"bottom-right\"\nlegend_size = 4\nwireframe = true\n")
            .unwrap();
        let p = c.legend_params();
        assert!(c.legend && c.wireframe);
        assert_eq!((p.pos, p.size), (LegendPos::BottomRight, 4));
        let mut c = Config { legend_size: 9, legend_opacity: f32::NAN, ..Config::default() };
        c.sanitize();
        assert_eq!((c.legend_size, c.legend_opacity), (LEGEND_SIZE_MAX, 0.85));
        assert!(Config::from_toml("legend_pos = \"middle\"").is_err());
    }

    #[test]
    fn assignment_split() {
        assert_eq!(split_assignment("cpu=cube").unwrap(), ("cpu".into(), "cube".into()));
        assert_eq!(split_assignment(r"disk-dev:C:\=cube").unwrap(), (r"disk-dev:C:\".into(), "cube".into()));
        assert!(split_assignment("cpu").is_err());
        assert!(split_assignment("cpu=").is_err());
    }
}
