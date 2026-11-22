//! Key handling and the stats bar.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::metrics::{MetricKind, Reading};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    ToggleStats,
    ToggleLegend,
    ToggleWireframe,
    Resize(u16, u16),
}

pub fn map_event(ev: &Event) -> Action {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. }) => {
            // Windows reports both press and release; act on press only.
            if *kind == KeyEventKind::Release {
                return Action::None;
            }
            match code {
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Action::Quit,
                KeyCode::Char('s') | KeyCode::Char('S') if *kind == KeyEventKind::Press => Action::ToggleStats,
                KeyCode::Char('l') | KeyCode::Char('L') if *kind == KeyEventKind::Press => Action::ToggleLegend,
                KeyCode::Char('w') | KeyCode::Char('W') if *kind == KeyEventKind::Press => Action::ToggleWireframe,
                _ => Action::None,
            }
        }
        Event::Resize(w, h) => Action::Resize(*w, *h),
        _ => Action::None,
    }
}

fn pct(v: Option<f32>) -> String {
    v.map_or_else(|| "n/a".to_string(), |p| format!("{:.0}%", p))
}

fn rate(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |r| format!("{:.1}", r))
}

/// One stats-bar segment.
pub fn segment(r: &Reading) -> String {
    let name = || r.name.clone().unwrap_or_else(|| "auto".into());
    let pair = |label: String, up: &str, down: &str| match (r.a, r.b) {
        (Some(_), Some(_)) => format!("{label} {up}{} {down}{} MB/s", rate(r.a), rate(r.b)),
        _ => format!("{label} n/a"),
    };
    match r.kind {
        MetricKind::Cpu => format!("CPU {}", pct(r.pct)),
        MetricKind::Ram => format!("RAM {}", pct(r.pct)),
        MetricKind::Gpu => format!("GPU {}", pct(r.pct)),
        MetricKind::Disk => pair("DSK".into(), "r", "w"),
        MetricKind::Net => pair("NET".into(), "↓", "↑"),
        MetricKind::Nic => pair(format!("NIC {}", name()), "↓", "↑"),
        MetricKind::DiskDev => pair(format!("DEV {}", name()), "r", "w"),
    }
}

/// Compact legend label for block letters: `CPU 37%`, `NET 1.5M`,
/// `NIC ETHERNET 0.2M` (rates are read + write, MB/s).
pub fn short_label(r: &Reading) -> String {
    let total = || match (r.a, r.b) {
        (Some(a), Some(b)) => format!("{:.1}M", a + b),
        _ => "n/a".to_string(),
    };
    let name = || r.name.as_deref().unwrap_or("auto").chars().take(10).collect::<String>();
    match r.kind {
        MetricKind::Cpu => format!("CPU {}", pct(r.pct)),
        MetricKind::Ram => format!("RAM {}", pct(r.pct)),
        MetricKind::Gpu => format!("GPU {}", pct(r.pct)),
        MetricKind::Disk => format!("DSK {}", total()),
        MetricKind::Net => format!("NET {}", total()),
        MetricKind::Nic => format!("NIC {} {}", name(), total()),
        MetricKind::DiskDev => format!("DEV {} {}", name(), total()),
    }
}

/// `CPU 37% | RAM 62% | DSK r12.4 w3.1 MB/s | NET ↓1.2 ↑0.3 MB/s | GPU 8% | fps 48/52`
pub fn stats_line(readings: &[&Reading], achieved_fps: f32, target_fps: f32) -> String {
    stats_line_fit(readings, achieved_fps, target_fps, "", usize::MAX)
}

/// Like [`stats_line`] (plus `extra` after the fps), fitted to `width`
/// characters: when too long, the metric part is cut (marked `~`) so the
/// fps reading always stays visible.
pub fn stats_line_fit(readings: &[&Reading], achieved_fps: f32, target_fps: f32, extra: &str, width: usize) -> String {
    let metrics = readings.iter().map(|r| segment(r)).collect::<Vec<_>>().join(" | ");
    let fps = format!("fps {achieved_fps:.0}/{target_fps:.0}{extra}");
    let full = if metrics.is_empty() { fps.clone() } else { format!("{metrics} | {fps}") };
    let len = |s: &str| s.chars().count();
    if len(&full) <= width {
        return full;
    }
    let tail = format!("~ | {fps}");
    if len(&tail) >= width {
        return fps.chars().take(width).collect();
    }
    let head: String = metrics.chars().take(width - len(&tail)).collect();
    format!("{head}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode, m: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent { code, modifiers: m, kind, state: KeyEventState::NONE })
    }

    #[test]
    fn keys() {
        let p = KeyEventKind::Press;
        assert_eq!(map_event(&key(KeyCode::Char('q'), KeyModifiers::NONE, p)), Action::Quit);
        assert_eq!(map_event(&key(KeyCode::Esc, KeyModifiers::NONE, p)), Action::Quit);
        assert_eq!(map_event(&key(KeyCode::Char('c'), KeyModifiers::CONTROL, p)), Action::Quit);
        assert_eq!(map_event(&key(KeyCode::Char('c'), KeyModifiers::NONE, p)), Action::None);
        assert_eq!(map_event(&key(KeyCode::Char('s'), KeyModifiers::NONE, p)), Action::ToggleStats);
        assert_eq!(
            map_event(&key(KeyCode::Char('s'), KeyModifiers::NONE, KeyEventKind::Release)),
            Action::None
        );
        assert_eq!(
            map_event(&key(KeyCode::Char('s'), KeyModifiers::NONE, KeyEventKind::Repeat)),
            Action::None
        );
        assert_eq!(map_event(&Event::Resize(80, 24)), Action::Resize(80, 24));
        assert_eq!(map_event(&key(KeyCode::Char('l'), KeyModifiers::NONE, p)), Action::ToggleLegend);
        assert_eq!(map_event(&key(KeyCode::Char('W'), KeyModifiers::SHIFT, p)), Action::ToggleWireframe);
    }

    #[test]
    fn short_labels() {
        use MetricKind::*;
        assert_eq!(short_label(&rd(Cpu, Some(37.4), None, None, None)), "CPU 37%");
        assert_eq!(short_label(&rd(Net, None, Some(1.0), Some(0.5), None)), "NET 1.5M");
        assert_eq!(short_label(&rd(Nic, None, None, None, Some("Ethernet 2 long"))), "NIC Ethernet 2 n/a");
    }

    fn rd(kind: MetricKind, pct: Option<f32>, a: Option<f64>, b: Option<f64>, name: Option<&str>) -> Reading {
        Reading { kind, name: name.map(String::from), pct, a, b }
    }

    #[test]
    fn stats_format() {
        use MetricKind::*;
        let rs = [
            rd(Cpu, Some(37.2), None, None, None),
            rd(Ram, Some(62.0), None, None, None),
            rd(Disk, None, Some(12.44), Some(3.1), None),
            rd(Net, None, Some(1.2), Some(0.3), None),
            rd(Gpu, Some(8.0), None, None, None),
        ];
        let refs: Vec<&Reading> = rs.iter().collect();
        assert_eq!(
            stats_line(&refs, 48.0, 52.0),
            "CPU 37% | RAM 62% | DSK r12.4 w3.1 MB/s | NET ↓1.2 ↑0.3 MB/s | GPU 8% | fps 48/52"
        );
        let na: Vec<Reading> = [Cpu, Ram, Disk, Net, Gpu].into_iter().map(|k| rd(k, None, None, None, None)).collect();
        let refs: Vec<&Reading> = na.iter().collect();
        assert_eq!(stats_line(&refs, 5.0, 5.0), "CPU n/a | RAM n/a | DSK n/a | NET n/a | GPU n/a | fps 5/5");

        let extra = [
            rd(Nic, None, Some(1.0), Some(0.5), Some("Wi-Fi")),
            rd(Nic, None, None, None, None),
            rd(DiskDev, None, Some(0.0), Some(2.31), Some(r"C:\")),
        ];
        let refs: Vec<&Reading> = extra.iter().collect();
        assert_eq!(stats_line(&refs, 9.0, 10.0), r"NIC Wi-Fi ↓1.0 ↑0.5 MB/s | NIC auto n/a | DEV C:\ r0.0 w2.3 MB/s | fps 9/10");
    }

    #[test]
    fn fitted_line_keeps_fps() {
        let rs = [rd(MetricKind::Cpu, Some(37.0), None, None, None), rd(MetricKind::Ram, Some(62.0), None, None, None)];
        let refs: Vec<&Reading> = rs.iter().collect();
        assert_eq!(stats_line_fit(&refs, 48.0, 52.0, "", 80), "CPU 37% | RAM 62% | fps 48/52");
        let cut = stats_line_fit(&refs, 48.0, 52.0, " | q-1", 24);
        assert_eq!(cut, "CPU 3~ | fps 48/52 | q-1");
        assert_eq!(cut.chars().count(), 24);
        assert_eq!(stats_line_fit(&refs, 48.0, 52.0, "", 5), "fps 4");
    }
}
