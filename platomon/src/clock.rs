//! Frame-rate controller: activity → target fps, slew limiting, pacing,
//! and achieved-fps tracking.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FpsCurve {
    Linear,
    #[default]
    Sqrt,
}

/// Map activity 0..=1 onto `min..=max` fps.
pub fn activity_to_fps(activity: f32, min: f32, max: f32, curve: FpsCurve) -> f32 {
    let a = if activity.is_finite() { activity.clamp(0.0, 1.0) } else { 0.0 };
    let s = match curve {
        FpsCurve::Linear => a,
        FpsCurve::Sqrt => a.sqrt(),
    };
    (min + (max - min) * s).clamp(min, max.max(min))
}

/// Move `current` toward `target` by at most `max_rate * dt`.
pub fn slew_limit(current: f32, target: f32, max_rate: f32, dt: f32) -> f32 {
    let step = (max_rate * dt).max(0.0);
    current + (target - current).clamp(-step, step)
}

/// Time before the deadline that we stop sleeping and spin. OS timers are
/// coarser on Windows, so leave more slack there.
pub const SPIN_MARGIN: Duration =
    if cfg!(windows) { Duration::from_micros(2000) } else { Duration::from_micros(1000) };

/// Busy-wait (yielding) until `deadline`.
pub fn spin_until(deadline: Instant) {
    while Instant::now() < deadline {
        std::thread::yield_now();
    }
}

/// Sleep most of the way to `deadline`, then spin the rest.
pub fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now + SPIN_MARGIN {
        std::thread::sleep(deadline - now - SPIN_MARGIN);
    }
    spin_until(deadline);
}

pub struct FrameClock {
    min: f32,
    max: f32,
    curve: FpsCurve,
    slew: f32,
    /// Idle runs fast, load runs slow.
    invert: bool,
    target: f32,
    /// EMA of frame duration, seconds.
    avg_dt: f32,
    last_frame: Instant,
}

/// Time constant for the achieved-fps average, seconds.
const ACHIEVED_TAU: f32 = 0.5;
/// Longest dt handed to the animation (e.g. after the process was suspended).
const MAX_ANIM_DT: f32 = 0.25;

impl FrameClock {
    pub fn new(min: f32, max: f32, curve: FpsCurve, slew: f32) -> Self {
        let min = min.max(1.0);
        let max = max.max(min);
        Self { min, max, curve, slew, invert: false, target: min, avg_dt: 1.0 / min, last_frame: Instant::now() }
    }

    /// Invert the mapping: idle → `max` fps, full load → `min` fps.
    pub fn inverted(mut self, invert: bool) -> Self {
        self.invert = invert;
        self
    }

    /// Update the target from the activity score, slew-limited over `dt`.
    pub fn update_target(&mut self, activity: f32, dt: f32) {
        let a = if activity.is_finite() { activity.clamp(0.0, 1.0) } else { 0.0 };
        let a = if self.invert { 1.0 - a } else { a };
        let want = activity_to_fps(a, self.min, self.max, self.curve);
        self.target = slew_limit(self.target, want, self.slew, dt).clamp(self.min, self.max);
    }

    pub fn target_fps(&self) -> f32 {
        self.target
    }

    pub fn achieved_fps(&self) -> f32 {
        1.0 / self.avg_dt.max(1e-6)
    }

    pub fn frame_period(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.target)
    }

    /// `last_frame + 1/target_fps`
    pub fn next_deadline(&self) -> Instant {
        self.last_frame + self.frame_period()
    }

    /// Mark the start of a new frame; returns wall-clock dt since the
    /// previous frame (capped for the animation).
    pub fn begin_frame(&mut self) -> f32 {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if dt > 0.0 && dt < 2.0 {
            // Time-weighted EMA of frame duration, so fps averages over ~tau seconds.
            let a = 1.0 - (-dt / ACHIEVED_TAU).exp();
            self.avg_dt += (dt - self.avg_dt) * a;
        }
        dt.min(MAX_ANIM_DT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_endpoints() {
        for c in [FpsCurve::Linear, FpsCurve::Sqrt] {
            assert_eq!(activity_to_fps(0.0, 5.0, 150.0, c), 5.0);
            assert_eq!(activity_to_fps(1.0, 5.0, 150.0, c), 150.0);
            assert_eq!(activity_to_fps(-1.0, 5.0, 150.0, c), 5.0);
            assert_eq!(activity_to_fps(7.0, 5.0, 150.0, c), 150.0);
            assert_eq!(activity_to_fps(f32::NAN, 5.0, 150.0, c), 5.0);
        }
        assert_eq!(activity_to_fps(0.25, 5.0, 150.0, FpsCurve::Sqrt), 5.0 + 145.0 * 0.5);
        assert_eq!(activity_to_fps(0.25, 5.0, 150.0, FpsCurve::Linear), 5.0 + 145.0 * 0.25);
    }

    #[test]
    fn slew_limits_both_directions() {
        assert_eq!(slew_limit(5.0, 150.0, 60.0, 0.5), 35.0);
        assert_eq!(slew_limit(150.0, 5.0, 60.0, 0.5), 120.0);
        assert_eq!(slew_limit(10.0, 12.0, 60.0, 0.5), 12.0);
        assert_eq!(slew_limit(10.0, 12.0, 60.0, -1.0), 10.0);
    }

    #[test]
    fn clock_clamps_and_slews() {
        let mut c = FrameClock::new(0.0, -5.0, FpsCurve::Sqrt, 60.0);
        assert_eq!(c.target_fps(), 1.0);
        c.update_target(1.0, 10.0);
        assert_eq!(c.target_fps(), 1.0, "max clamped up to min");

        let mut c = FrameClock::new(5.0, 150.0, FpsCurve::Sqrt, 60.0);
        assert_eq!(c.target_fps(), 5.0);
        c.update_target(1.0, 1.0);
        assert_eq!(c.target_fps(), 65.0);
        for _ in 0..10 {
            c.update_target(1.0, 1.0);
        }
        assert_eq!(c.target_fps(), 150.0);
        for _ in 0..10 {
            c.update_target(0.0, 1.0);
        }
        assert_eq!(c.target_fps(), 5.0);
    }

    #[test]
    fn inverted_clock_runs_fast_when_idle() {
        let mut c = FrameClock::new(5.0, 150.0, FpsCurve::Linear, 1000.0).inverted(true);
        c.update_target(0.0, 1.0);
        assert_eq!(c.target_fps(), 150.0, "idle → max");
        c.update_target(1.0, 1.0);
        assert_eq!(c.target_fps(), 5.0, "full load → min");
        c.update_target(0.25, 1.0);
        assert_eq!(c.target_fps(), 5.0 + 145.0 * 0.75);
        c.update_target(f32::NAN, 1.0);
        assert_eq!(c.target_fps(), 150.0, "no reading counts as idle");
        // slew still applies
        let mut c = FrameClock::new(5.0, 150.0, FpsCurve::Sqrt, 60.0).inverted(true);
        c.update_target(0.0, 1.0);
        assert_eq!(c.target_fps(), 65.0);
    }

    /// Timing-sensitive, so it is kept out of the parallel test run: the
    /// render tests saturate every core (rayon) and a 3-core CI runner then
    /// wakes sleepers 20+ ms late. CI runs it alone:
    /// `cargo test -- --ignored --test-threads=1`.
    #[test]
    #[ignore = "timing-sensitive: run alone with --ignored --test-threads=1"]
    fn pacing_hits_deadline() {
        // Never early, every time. Lateness is judged on the best of several
        // tries with a loose bound: shared CI runners (notably macOS) can
        // deschedule a thread for 5–10 ms, which says nothing about pacing.
        let mut best = Duration::MAX;
        for _ in 0..10 {
            let deadline = Instant::now() + Duration::from_millis(15);
            sleep_until(deadline);
            let now = Instant::now();
            assert!(now >= deadline, "woke before the deadline");
            best = best.min(now.duration_since(deadline));
        }
        assert!(best < Duration::from_millis(20), "best overshoot {best:?}");
    }
}
