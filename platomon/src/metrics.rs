//! Metric selection, normalisation, smoothing, rolling peaks and the activity score.
//!
//! Each displayed solid is a *slot* bound to a [`MetricSpec`] (`cpu`, `nic:Ethernet`, ...).
//! Raw samples arrive at ~4 Hz ([`MetricsState::ingest`]); the renderer calls
//! [`MetricsState::tick`] every frame so the solids move smoothly in between.

use crate::collector::{IoSample, RawMetrics};
use crate::config::{Config, Weights};
use crate::scene::sdf::SolidKind;

/// Fraction of the primary spin applied to the secondary axis for metrics
/// that only have one activity signal.
const TILT: f32 = 0.35;
const MB: f64 = 1_000_000.0;
/// Auto-selected instances switch only when another is this much busier.
const AUTO_HYSTERESIS: f64 = 1.5;

// ---------------------------------------------------------------- metric ids

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    Cpu,
    Ram,
    Disk,
    Net,
    Gpu,
    /// One network adapter.
    Nic,
    /// One disk / volume.
    DiskDev,
}

impl MetricKind {
    pub const ALL: [MetricKind; 7] = [
        MetricKind::Cpu,
        MetricKind::Ram,
        MetricKind::Disk,
        MetricKind::Net,
        MetricKind::Gpu,
        MetricKind::Nic,
        MetricKind::DiskDev,
    ];

    pub fn id(self) -> &'static str {
        match self {
            MetricKind::Cpu => "cpu",
            MetricKind::Ram => "ram",
            MetricKind::Disk => "disk",
            MetricKind::Net => "net",
            MetricKind::Gpu => "gpu",
            MetricKind::Nic => "nic",
            MetricKind::DiskDev => "disk-dev",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace('_', "-");
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Takes an optional `:INSTANCE` suffix.
    pub fn has_instance(self) -> bool {
        matches!(self, MetricKind::Nic | MetricKind::DiskDev)
    }

    pub fn default_solid(self) -> SolidKind {
        match self {
            MetricKind::Cpu => SolidKind::Icosahedron,
            MetricKind::Ram => SolidKind::Cube,
            MetricKind::Disk => SolidKind::Octahedron,
            MetricKind::Net => SolidKind::Tetrahedron,
            MetricKind::Gpu => SolidKind::Dodecahedron,
            MetricKind::Nic => SolidKind::RhombicDodecahedron,
            MetricKind::DiskDev => SolidKind::TruncatedCube,
        }
    }

    /// (scale, spin) description for `--list metrics`.
    pub fn describe(self) -> (&'static str, &'static str) {
        match self {
            MetricKind::Cpu => ("utilisation (minus platomon)", "utilisation"),
            MetricKind::Ram => ("used / total", "rate of change of used memory"),
            MetricKind::Disk => ("all disks, read+write", "Y = read, X = write"),
            MetricKind::Net => ("all adapters, rx+tx", "Y = rx, X = tx"),
            MetricKind::Gpu => ("utilisation (busiest GPU)", "memory-controller util, else utilisation"),
            MetricKind::Nic => ("one adapter, rx+tx (default: busiest)", "Y = rx, X = tx"),
            MetricKind::DiskDev => ("one disk, read+write (default: busiest)", "Y = read, X = write"),
        }
    }
}

/// A metric plus optional instance: `cpu`, `nic`, `nic:Ethernet`, `disk-dev:C:\`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetricSpec {
    pub kind: MetricKind,
    /// `None` = automatic (busiest).
    pub instance: Option<String>,
}

impl MetricSpec {
    pub fn new(kind: MetricKind) -> Self {
        Self { kind, instance: None }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (k, inst) = match s.split_once(':') {
            Some((k, i)) => (k, Some(i.trim())),
            None => (s, None),
        };
        let kind = MetricKind::from_id(k).ok_or_else(|| format!("unknown metric '{k}' (see --list metrics)"))?;
        let instance = match inst {
            Some(i) if !kind.has_instance() => {
                return Err(format!("metric '{}' takes no ':{i}' instance", kind.id()));
            }
            None | Some("") => None,
            Some(i) if i.eq_ignore_ascii_case("auto") => None,
            Some(i) => Some(i.to_string()),
        };
        Ok(Self { kind, instance })
    }

    /// Canonical key used by `--solid`, `--ring` and the config tables.
    pub fn key(&self) -> String {
        match &self.instance {
            Some(i) => format!("{}:{}", self.kind.id(), i),
            None => self.kind.id().to_string(),
        }
    }
}

// ---------------------------------------------------------------- primitives

/// Exponential moving average with a time constant (frame-rate independent).
#[derive(Debug, Clone, Copy)]
pub struct Ema {
    value: f32,
    tau: f32,
}

impl Ema {
    pub fn new(tau: f32) -> Self {
        Self { value: 0.0, tau: tau.max(1e-4) }
    }

    pub fn alpha(dt: f32, tau: f32) -> f32 {
        if dt <= 0.0 { 0.0 } else { 1.0 - (-dt / tau.max(1e-4)).exp() }
    }

    pub fn update(&mut self, target: f32, dt: f32) -> f32 {
        self.value += (target - self.value) * Self::alpha(dt, self.tau);
        self.value
    }

    pub fn value(&self) -> f32 {
        self.value
    }
}

/// A peak that decays by `decay_per_sec` (fraction) every second, never below `floor`.
#[derive(Debug, Clone, Copy)]
pub struct RollingPeak {
    peak: f64,
    floor: f64,
    decay_per_sec: f64,
}

impl RollingPeak {
    pub fn new(floor: f64, decay_per_sec: f64) -> Self {
        Self { peak: floor, floor, decay_per_sec: decay_per_sec.clamp(0.0, 0.999) }
    }

    pub fn update(&mut self, x: f64, dt: f64) -> f64 {
        let decayed = self.peak * (1.0 - self.decay_per_sec).powf(dt.max(0.0));
        let x = if x.is_finite() { x } else { 0.0 };
        self.peak = decayed.max(x).max(self.floor);
        self.peak
    }

    pub fn peak(&self) -> f64 {
        self.peak
    }
}

/// `log1p`-compressed rate in 0..=1 relative to `peak`, with `unit` as the
/// compression knee (the rolling-peak floor).
pub fn normalize_rate(x: f64, peak: f64, unit: f64) -> f32 {
    if !(x.is_finite() && x > 0.0) {
        return 0.0;
    }
    let unit = unit.max(1e-9);
    let denom = (peak.max(unit) / unit).ln_1p();
    ((x / unit).ln_1p() / denom).clamp(0.0, 1.0) as f32
}

/// sysinfo reports a process's CPU as % of one core; convert to the
/// system-wide 0..=100 basis used by global CPU usage.
pub fn self_cpu_system_basis(process_pct: f32, ncpus: usize) -> f32 {
    if !process_pct.is_finite() {
        return 0.0;
    }
    (process_pct / ncpus.max(1) as f32).clamp(0.0, 100.0)
}

/// Feedback-loop guard: system CPU minus our own share, clamped to 0..=100.
pub fn subtract_self_cpu(total_pct: f32, self_pct: f32) -> f32 {
    let t = if total_pct.is_finite() { total_pct } else { 0.0 };
    let s = if self_pct.is_finite() { self_pct.max(0.0) } else { 0.0 };
    (t - s).clamp(0.0, 100.0)
}

/// Weighted mean of activity over available channels. Unavailable channels'
/// weights are excluded entirely.
pub fn activity_score(channels: &[Channel], weights: &[f32]) -> f32 {
    let (mut num, mut den) = (0.0f32, 0.0f32);
    for (c, &w) in channels.iter().zip(weights) {
        if c.available && w > 0.0 {
            num += w * c.activity.clamp(0.0, 1.0);
            den += w;
        }
    }
    if den > 0.0 { (num / den).clamp(0.0, 1.0) } else { 0.0 }
}

/// Pick an instance by name, or (auto) the busiest with hysteresis.
fn pick<'a>(list: &'a [IoSample], want: Option<&str>, chosen: &mut Option<String>) -> Option<&'a IoSample> {
    if let Some(w) = want {
        return list.iter().find(|s| s.matches(w));
    }
    let total = |s: &IoSample| s.a_bps + s.b_bps;
    let busiest = list.iter().max_by(|a, b| total(a).total_cmp(&total(b)))?;
    let current = chosen.as_deref().and_then(|c| list.iter().find(|s| s.name == c));
    let p = match current {
        Some(c) if total(busiest) <= AUTO_HYSTERESIS * total(c) + 1e3 => c,
        _ => busiest,
    };
    *chosen = Some(p.name.clone());
    Some(p)
}

// ---------------------------------------------------------------- state

/// A slot's normalised signals, all 0..=1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Channel {
    pub available: bool,
    /// Drives scale.
    pub level: f32,
    pub spin_y: f32,
    pub spin_x: f32,
    /// Contribution to the fps activity score. Equals `level` except for RAM,
    /// whose level (used/total) is a fill state, not activity — there the
    /// rate of change is used so an idle machine can settle near fps_min.
    pub activity: f32,
}

/// Human-readable values of one slot for the stats bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub kind: MetricKind,
    /// Resolved instance name (for `nic` / `disk-dev`).
    pub name: Option<String>,
    pub pct: Option<f32>,
    /// MB/s: read / rx.
    pub a: Option<f64>,
    /// MB/s: write / tx.
    pub b: Option<f64>,
}

impl Reading {
    fn empty(kind: MetricKind) -> Self {
        Self { kind, name: None, pct: None, a: None, b: None }
    }
}

#[derive(Debug, Clone, Copy)]
struct Smoothed {
    level: Ema,
    spin_y: Ema,
    spin_x: Ema,
    activity: Ema,
}

struct Slot {
    spec: MetricSpec,
    target: Channel,
    smooth: Smoothed,
    peak: RollingPeak,
    last_mem: Option<u64>,
    chosen: Option<String>,
    reading: Reading,
}

pub struct MetricsState {
    slots: Vec<Slot>,
    unit: f64,
    last_seq: u64,
}

impl MetricsState {
    pub fn new(cfg: &Config, specs: &[MetricSpec]) -> Self {
        let e = Ema::new(cfg.smoothing_tau);
        let smooth = Smoothed { level: e, spin_y: e, spin_x: e, activity: e };
        let peak = RollingPeak::new(cfg.rate_floor_bytes, cfg.peak_decay_per_sec);
        let slots = specs
            .iter()
            .map(|s| Slot {
                spec: s.clone(),
                target: Channel::default(),
                smooth,
                peak,
                last_mem: None,
                chosen: None,
                reading: Reading::empty(s.kind),
            })
            .collect();
        Self { slots, unit: cfg.rate_floor_bytes, last_seq: 0 }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Feed a raw snapshot. Repeated snapshots (same `seq`) are ignored.
    pub fn ingest(&mut self, raw: &RawMetrics) {
        if raw.seq == self.last_seq {
            return;
        }
        self.last_seq = raw.seq;
        let dt = raw.elapsed.max(1e-3);
        let unit = self.unit;

        for slot in &mut self.slots {
            let kind = slot.spec.kind;
            let mut r = Reading::empty(kind);
            let rate_pair = |peak: &mut RollingPeak, a: f64, b: f64| {
                let (a, b) = (a.max(0.0), b.max(0.0));
                let p = peak.update(a + b, dt);
                let level = normalize_rate(a + b, p, unit);
                Channel {
                    available: true,
                    level,
                    activity: level,
                    spin_y: normalize_rate(a, p, unit),
                    spin_x: normalize_rate(b, p, unit),
                }
            };

            slot.target = match kind {
                MetricKind::Cpu => match raw.cpu_total {
                    Some(total) => {
                        let c = subtract_self_cpu(total, raw.cpu_self.unwrap_or(0.0));
                        r.pct = Some(c);
                        let u = c / 100.0;
                        Channel { available: true, level: u, spin_y: u, spin_x: u * TILT, activity: u }
                    }
                    None => Channel::default(),
                },
                MetricKind::Ram => match (raw.mem_used, raw.mem_total) {
                    (Some(used), Some(total)) if total > 0 => {
                        let level = (used as f64 / total as f64).clamp(0.0, 1.0) as f32;
                        r.pct = Some(level * 100.0);
                        let rate = slot.last_mem.map_or(0.0, |prev| (used as f64 - prev as f64).abs() / dt);
                        slot.last_mem = Some(used);
                        let p = slot.peak.update(rate, dt);
                        let s = normalize_rate(rate, p, unit);
                        Channel { available: true, level, spin_y: s, spin_x: s * TILT, activity: s }
                    }
                    _ => {
                        slot.last_mem = None;
                        Channel::default()
                    }
                },
                MetricKind::Disk | MetricKind::Net => {
                    let (a, b) = if kind == MetricKind::Disk {
                        (raw.disk_read_bps, raw.disk_write_bps)
                    } else {
                        (raw.net_rx_bps, raw.net_tx_bps)
                    };
                    match (a, b) {
                        (Some(a), Some(b)) => {
                            r.a = Some(a / MB);
                            r.b = Some(b / MB);
                            rate_pair(&mut slot.peak, a, b)
                        }
                        _ => Channel::default(),
                    }
                }
                MetricKind::Nic | MetricKind::DiskDev => {
                    let list = if kind == MetricKind::Nic { &raw.nics } else { &raw.disks };
                    match pick(list, slot.spec.instance.as_deref(), &mut slot.chosen) {
                        Some(s) => {
                            r.name = Some(s.name.clone());
                            r.a = Some(s.a_bps / MB);
                            r.b = Some(s.b_bps / MB);
                            rate_pair(&mut slot.peak, s.a_bps, s.b_bps)
                        }
                        None => {
                            r.name = slot.spec.instance.clone();
                            Channel::default()
                        }
                    }
                }
                MetricKind::Gpu => match raw.gpu_util {
                    Some(g) => {
                        let u = (g / 100.0).clamp(0.0, 1.0);
                        r.pct = Some(u * 100.0);
                        let spin = raw.gpu_mem_util.map_or(u, |m| (m / 100.0).clamp(0.0, 1.0).max(u));
                        Channel { available: true, level: u, spin_y: spin, spin_x: u * TILT, activity: u }
                    }
                    None => Channel::default(),
                },
            };
            slot.reading = r;
        }
    }

    /// Advance smoothing by `dt` seconds of wall-clock time.
    pub fn tick(&mut self, dt: f32) {
        for s in &mut self.slots {
            let t = s.target;
            s.smooth.level.update(t.level, dt);
            s.smooth.spin_y.update(t.spin_y, dt);
            s.smooth.spin_x.update(t.spin_x, dt);
            s.smooth.activity.update(t.activity, dt);
        }
    }

    pub fn channel(&self, i: usize) -> Channel {
        let s = &self.slots[i];
        Channel {
            available: s.target.available,
            level: s.smooth.level.value().clamp(0.0, 1.0),
            spin_y: s.smooth.spin_y.value().clamp(0.0, 1.0),
            spin_x: s.smooth.spin_x.value().clamp(0.0, 1.0),
            activity: s.smooth.activity.value().clamp(0.0, 1.0),
        }
    }

    pub fn channels(&self) -> Vec<Channel> {
        (0..self.slots.len()).map(|i| self.channel(i)).collect()
    }

    pub fn weights(&self, w: &Weights) -> Vec<f32> {
        self.slots.iter().map(|s| w.get(s.spec.kind)).collect()
    }

    pub fn activity(&self, w: &Weights) -> f32 {
        activity_score(&self.channels(), &self.weights(w))
    }

    pub fn readings(&self) -> Vec<&Reading> {
        self.slots.iter().map(|s| &s.reading).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn ema_converges_and_is_rate_independent() {
        let mut a = Ema::new(0.5);
        let mut b = Ema::new(0.5);
        for _ in 0..10 {
            a.update(1.0, 0.1);
        }
        for _ in 0..100 {
            b.update(1.0, 0.01);
        }
        assert!((a.value() - b.value()).abs() < 1e-4);
        assert!(approx(a.value(), 1.0 - (-2.0f32).exp()));
        for _ in 0..1000 {
            a.update(1.0, 0.1);
        }
        assert!(approx(a.value(), 1.0));
        assert_eq!(Ema::alpha(0.0, 1.0), 0.0);
    }

    #[test]
    fn rolling_peak_decays_to_floor() {
        let mut p = RollingPeak::new(1.0, 0.01);
        assert_eq!(p.update(100.0, 0.25), 100.0);
        let after = p.update(0.0, 1.0);
        assert!((after - 99.0).abs() < 1e-9, "decays 1%/s: {after}");
        for _ in 0..100_000 {
            p.update(0.0, 1.0);
        }
        assert_eq!(p.peak(), 1.0);
        assert_eq!(p.update(f64::NAN, 1.0), 1.0);
    }

    #[test]
    fn normalize_rate_bounds() {
        assert_eq!(normalize_rate(0.0, 10.0, 1.0), 0.0);
        assert_eq!(normalize_rate(-5.0, 10.0, 1.0), 0.0);
        assert_eq!(normalize_rate(f64::NAN, 10.0, 1.0), 0.0);
        assert!(approx(normalize_rate(10.0, 10.0, 1.0), 1.0));
        assert_eq!(normalize_rate(1e12, 10.0, 1.0), 1.0);
        assert!(approx(normalize_rate(3.0, 15.0, 1.0), 0.5));
        assert!(normalize_rate(10.0, 100.0, 1.0) > 0.5);
    }

    #[test]
    fn self_cpu_subtraction() {
        assert_eq!(subtract_self_cpu(50.0, 10.0), 40.0);
        assert_eq!(subtract_self_cpu(5.0, 10.0), 0.0);
        assert_eq!(subtract_self_cpu(100.0, 0.0), 100.0);
        assert_eq!(subtract_self_cpu(150.0, 0.0), 100.0);
        assert_eq!(subtract_self_cpu(50.0, f32::NAN), 50.0);
        assert_eq!(subtract_self_cpu(50.0, -5.0), 50.0);
        let s = self_cpu_system_basis(200.0, 8);
        assert_eq!(s, 25.0);
        assert_eq!(subtract_self_cpu(30.0, s), 5.0);
        assert_eq!(self_cpu_system_basis(50.0, 0), 50.0);
    }

    #[test]
    fn activity_excludes_unavailable() {
        let on = |a| Channel { available: true, activity: a, ..Default::default() };
        let off = Channel { available: false, activity: 1.0, ..Default::default() };
        let w = [1.0; 5];
        assert_eq!(activity_score(&[on(1.0), on(0.0), off, off, off], &w), 0.5);
        assert_eq!(activity_score(&[off; 5], &w), 0.0);
        assert_eq!(activity_score(&[on(1.0), on(0.0)], &[3.0, 1.0]), 0.75);
        assert_eq!(activity_score(&[on(1.0), on(0.0)], &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn spec_parsing() {
        assert_eq!(MetricSpec::parse("cpu").unwrap(), MetricSpec::new(MetricKind::Cpu));
        assert_eq!(MetricSpec::parse(" NIC ").unwrap(), MetricSpec::new(MetricKind::Nic));
        assert_eq!(MetricSpec::parse("nic:auto").unwrap(), MetricSpec::new(MetricKind::Nic));
        let d = MetricSpec::parse(r"disk-dev:C:\").unwrap();
        assert_eq!(d.kind, MetricKind::DiskDev);
        assert_eq!(d.instance.as_deref(), Some(r"C:\"));
        assert_eq!(d.key(), r"disk-dev:C:\");
        assert!(MetricSpec::parse("cpu:0").is_err());
        assert!(MetricSpec::parse("cpu-temp").is_err());
        assert_eq!(MetricSpec::parse("disk_dev").unwrap().kind, MetricKind::DiskDev);
    }

    fn io(name: &str, a: f64, b: f64) -> IoSample {
        IoSample { name: name.into(), alt: None, a_bps: a, b_bps: b }
    }

    #[test]
    fn auto_pick_has_hysteresis() {
        let mut chosen = None;
        let l1 = [io("eth0", 100.0, 0.0), io("wlan0", 5000.0, 0.0)];
        assert_eq!(pick(&l1, None, &mut chosen).unwrap().name, "wlan0");
        let l2 = [io("eth0", 6000.0, 0.0), io("wlan0", 5000.0, 0.0)];
        assert_eq!(pick(&l2, None, &mut chosen).unwrap().name, "wlan0", "sticky within 1.5x");
        let l3 = [io("eth0", 20000.0, 0.0), io("wlan0", 5000.0, 0.0)];
        assert_eq!(pick(&l3, None, &mut chosen).unwrap().name, "eth0");
        assert_eq!(pick(&l3, Some("WLAN0"), &mut chosen).unwrap().name, "wlan0");
        assert!(pick(&l3, Some("nope"), &mut chosen).is_none());
    }

    fn raw(seq: u64) -> RawMetrics {
        RawMetrics {
            seq,
            elapsed: 0.25,
            cpu_total: Some(60.0),
            cpu_self: Some(20.0),
            mem_used: Some(4),
            mem_total: Some(8),
            disk_read_bps: Some(0.0),
            disk_write_bps: Some(0.0),
            nics: vec![io("eth0", 2e6, 1e6)],
            ..RawMetrics::default()
        }
    }

    fn specs(s: &[&str]) -> Vec<MetricSpec> {
        s.iter().map(|x| MetricSpec::parse(x).unwrap()).collect()
    }

    #[test]
    fn state_applies_guard_and_marks_na() {
        let cfg = Config::default();
        let mut m = MetricsState::new(&cfg, &specs(&["cpu", "ram", "disk", "net", "gpu", "nic", "disk-dev:X"]));
        m.ingest(&raw(1));
        for _ in 0..200 {
            m.tick(0.05);
        }
        let cpu = m.channel(0);
        assert!(cpu.available && approx(cpu.level, 0.4), "{cpu:?}");
        assert!(approx(m.channel(1).level, 0.5));
        assert!(!m.channel(3).available, "net n/a");
        assert!(!m.channel(4).available, "gpu n/a");
        assert!(m.channel(5).available, "nic auto-picked");
        assert!(!m.channel(6).available, "named disk missing");
        let r = m.readings();
        assert_eq!(r[0].pct, Some(40.0));
        assert_eq!(r[3].a, None);
        assert_eq!(r[5].name.as_deref(), Some("eth0"));
        assert_eq!(r[5].a, Some(2.0));
        assert_eq!(r[6].name.as_deref(), Some("X"));
        // available: cpu 0.4, ram 0 (no change), disk 0, nic = level at its own peak = 1.0
        assert!(approx(m.activity(&Weights::default()), (0.4 + 1.0) / 4.0));
    }

    #[test]
    fn same_seq_ingested_once() {
        let cfg = Config::default();
        let mut m = MetricsState::new(&cfg, &specs(&["cpu"]));
        let mut r = raw(1);
        m.ingest(&r);
        r.cpu_total = Some(100.0);
        m.ingest(&r);
        assert_eq!(m.readings()[0].pct, Some(40.0));
        r.seq = 2;
        m.ingest(&r);
        assert_eq!(m.readings()[0].pct, Some(80.0));
    }
}
