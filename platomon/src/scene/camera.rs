//! Camera, and the rig that places it: static, or orbiting the layout's centroid.

use glam::Vec3A;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CameraMode {
    #[default]
    Static,
    Orbit,
    /// Glide toward the busiest solid.
    Focus,
}

impl FromStr for CameraMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "static" => Ok(Self::Static),
            "orbit" => Ok(Self::Orbit),
            "focus" => Ok(Self::Focus),
            _ => Err(format!("unknown camera '{s}' (static|orbit|focus)")),
        }
    }
}

/// Orbit direction, seen from above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OrbitDir {
    #[default]
    Random,
    Cw,
    Ccw,
}

impl FromStr for OrbitDir {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "random" => Ok(Self::Random),
            "cw" => Ok(Self::Cw),
            "ccw" => Ok(Self::Ccw),
            _ => Err(format!("unknown orbit direction '{s}' (random|cw|ccw)")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub pos: Vec3A,
    pub forward: Vec3A,
    pub right: Vec3A,
    pub up: Vec3A,
    /// Minimum horizontal / vertical field of view, radians (fit whichever is tighter).
    pub hfov: f32,
    pub vfov: f32,
}

impl Camera {
    pub fn look_at(pos: Vec3A, target: Vec3A, hfov_deg: f32, vfov_deg: f32) -> Self {
        let forward = (target - pos).normalize();
        let right = forward.cross(Vec3A::Y).normalize();
        let up = right.cross(forward);
        Self { pos, forward, right, up, hfov: hfov_deg.to_radians(), vfov: vfov_deg.to_radians() }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CameraParams {
    pub mode: CameraMode,
    pub period: f32,
    pub dir: OrbitDir,
    pub radius: Option<f32>,
    pub height: Option<f32>,
    pub bob: f32,
    pub seed: Option<u64>,
    /// Focus mode: 0 = only turn toward the busiest solid, 1 = close-up.
    pub focus_zoom: f32,
    /// Focus mode: glide time constant, seconds.
    pub focus_tau: f32,
}

impl Default for CameraParams {
    fn default() -> Self {
        Self {
            mode: CameraMode::Static,
            period: 60.0,
            dir: OrbitDir::Random,
            radius: None,
            height: None,
            bob: 0.3,
            seed: None,
            focus_zoom: 0.6,
            focus_tau: 1.5,
        }
    }
}

/// Focus mode close-up distance from the solid's centre, world units
/// (a full-size solid then fills most of the 30° vertical view).
pub const FOCUS_CLOSE: f32 = 5.0;

/// Distance and height per unit of layout extent (tuned on the 5-solid ring).
const RADIUS_PER_EXTENT: f32 = 2.32;
const HEIGHT_PER_EXTENT: f32 = 1.4;
const TARGET_Y: f32 = 0.55;
const HFOV_DEG: f32 = 50.0;
const VFOV_DEG: f32 = 30.0;

#[derive(Debug, Clone)]
pub struct CameraRig {
    mode: CameraMode,
    /// +1 = counter-clockwise from above, −1 = clockwise.
    dir: f32,
    omega: f32,
    radius: f32,
    height: f32,
    bob: f32,
    angle: f32,
    centroid: Vec3A,
    focus_zoom: f32,
    focus_tau: f32,
    /// Focus mode: smoothed eye position and look-at point.
    eye: Vec3A,
    look: Vec3A,
}

impl CameraRig {
    /// `centroid`: mean solid position; `extent`: layout radius around it.
    pub fn new(p: &CameraParams, centroid: Vec3A, extent: f32) -> Self {
        let dir = match p.dir {
            OrbitDir::Ccw => 1.0,
            OrbitDir::Cw => -1.0,
            OrbitDir::Random => {
                let mut rng = p.seed.map_or_else(fastrand::Rng::new, fastrand::Rng::with_seed);
                if rng.bool() { 1.0 } else { -1.0 }
            }
        };
        let mut rig = Self {
            mode: p.mode,
            dir,
            omega: TAU / p.period.max(0.1),
            radius: p.radius.unwrap_or(RADIUS_PER_EXTENT * extent),
            height: p.height.unwrap_or(HEIGHT_PER_EXTENT * extent),
            bob: p.bob,
            angle: 0.0,
            centroid,
            focus_zoom: if p.focus_zoom.is_finite() { p.focus_zoom.clamp(0.0, 1.0) } else { 0.6 },
            focus_tau: if p.focus_tau.is_finite() { p.focus_tau.max(0.05) } else { 1.5 },
            eye: Vec3A::ZERO,
            look: Vec3A::ZERO,
        };
        (rig.eye, rig.look) = rig.overview();
        rig
    }

    pub fn direction(&self) -> f32 {
        self.dir
    }

    pub fn angle(&self) -> f32 {
        self.angle
    }

    /// Eye and look-at point of the overview (static / orbit) shot.
    fn overview(&self) -> (Vec3A, Vec3A) {
        let (s, c) = self.angle.sin_cos();
        let bob = if self.mode == CameraMode::Orbit { self.bob * (2.0 * self.angle).sin() } else { 0.0 };
        let pos = self.centroid + Vec3A::new(self.radius * s, self.height + bob, self.radius * c);
        (pos, self.centroid + Vec3A::new(0.0, TARGET_Y, 0.0))
    }

    /// Where focus mode wants to be for a solid centred at `p`: on the line
    /// from the solid toward the overview eye, `focus_zoom` of the way from
    /// the overview distance to a close-up.
    pub fn focus_shot(&self, p: Vec3A) -> (Vec3A, Vec3A) {
        let (base, _) = self.overview();
        let to_base = base - p;
        let far = to_base.length().max(1e-3);
        let dist = far + (FOCUS_CLOSE.min(far) - far) * self.focus_zoom;
        (p + to_base / far * dist, p)
    }

    /// Advance by wall-clock `dt`. `focus`: centre of the solid focus mode
    /// should move toward (`None` = overview).
    pub fn advance(&mut self, dt: f32, focus: Option<Vec3A>) {
        if self.mode == CameraMode::Orbit {
            self.angle = (self.angle + self.dir * self.omega * dt).rem_euclid(TAU);
        }
        if self.mode == CameraMode::Focus {
            let (eye, look) = focus.map_or_else(|| self.overview(), |p| self.focus_shot(p));
            let a = 1.0 - (-dt.max(0.0) / self.focus_tau).exp();
            self.eye += (eye - self.eye) * a;
            self.look += (look - self.look) * a;
        }
    }

    pub fn camera(&self) -> Camera {
        let (pos, target) = if self.mode == CameraMode::Focus { (self.eye, self.look) } else { self.overview() };
        Camera::look_at(pos, target, HFOV_DEG, VFOV_DEG)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(mode: CameraMode, dir: OrbitDir) -> CameraParams {
        CameraParams { mode, dir, period: 10.0, ..CameraParams::default() }
    }

    #[test]
    fn static_never_moves() {
        let mut r = CameraRig::new(&params(CameraMode::Static, OrbitDir::Ccw), Vec3A::ZERO, 3.7);
        let before = r.camera().pos;
        r.advance(5.0, Some(Vec3A::X));
        assert_eq!(r.camera().pos, before);
        assert!(before.z > 0.0 && before.x.abs() < 1e-6, "in front, on +z");
    }

    #[test]
    fn orbit_circles_centroid_at_period() {
        let c = Vec3A::new(1.0, 0.0, -2.0);
        let mut r = CameraRig::new(&CameraParams { bob: 0.0, ..params(CameraMode::Orbit, OrbitDir::Ccw) }, c, 3.7);
        let d0 = (r.camera().pos - c) * Vec3A::new(1.0, 0.0, 1.0);
        r.advance(2.5, None); // quarter turn
        let p = r.camera().pos - c;
        assert!(((p * Vec3A::new(1.0, 0.0, 1.0)).length() - d0.length()).abs() < 1e-4, "constant radius");
        assert!(p.x > 0.0 && p.z.abs() < 1e-3, "ccw from +z goes to +x: {p:?}");
        r.advance(7.5, None);
        assert!(r.angle() < 1e-4 || (TAU - r.angle()) < 1e-4, "full turn after one period");
        // looks at the centroid
        let cam = r.camera();
        let to = (c + Vec3A::new(0.0, TARGET_Y, 0.0) - cam.pos).normalize();
        assert!(cam.forward.dot(to) > 0.9999);
    }

    #[test]
    fn directions() {
        let cw = CameraRig::new(&params(CameraMode::Orbit, OrbitDir::Cw), Vec3A::ZERO, 1.0);
        assert_eq!(cw.direction(), -1.0);
        let a = CameraRig::new(&CameraParams { seed: Some(42), ..params(CameraMode::Orbit, OrbitDir::Random) }, Vec3A::ZERO, 1.0);
        let b = CameraRig::new(&CameraParams { seed: Some(42), ..params(CameraMode::Orbit, OrbitDir::Random) }, Vec3A::ZERO, 1.0);
        assert_eq!(a.direction(), b.direction(), "seeded random is repeatable");
        let seen: std::collections::HashSet<i32> = (0..64)
            .map(|s| CameraRig::new(&CameraParams { seed: Some(s), ..params(CameraMode::Orbit, OrbitDir::Random) }, Vec3A::ZERO, 1.0).direction() as i32)
            .collect();
        assert_eq!(seen.len(), 2, "random yields both directions");
    }

    #[test]
    fn focus_glides_toward_the_solid_and_back() {
        let p = CameraParams { focus_zoom: 1.0, focus_tau: 0.5, ..params(CameraMode::Focus, OrbitDir::Cw) };
        let mut r = CameraRig::new(&p, Vec3A::ZERO, 3.7);
        let start = r.camera().pos;
        let solid = Vec3A::new(2.7, 1.0, 0.0);
        let (want, _) = r.focus_shot(solid);
        assert!(((want - solid).length() - FOCUS_CLOSE).abs() < 1e-3, "zoom 1 = close-up distance");
        r.advance(0.5, Some(solid));
        let mid = r.camera().pos;
        assert!((mid - want).length() < (start - want).length(), "moving toward it");
        assert!((mid - want).length() > 0.1, "but smoothly, not a jump");
        for _ in 0..100 {
            r.advance(0.1, Some(solid));
        }
        let cam = r.camera();
        assert!((cam.pos - want).length() < 1e-2);
        assert!(cam.forward.dot((solid - cam.pos).normalize()) > 0.9999, "looks at the solid");
        for _ in 0..100 {
            r.advance(0.1, None);
        }
        assert!((r.camera().pos - start).length() < 1e-2, "back to the overview");
        // zoom 0: same distance as the overview, only turns
        let r0 = CameraRig::new(&CameraParams { focus_zoom: 0.0, ..p }, Vec3A::ZERO, 3.7);
        let (eye0, _) = r0.focus_shot(solid);
        assert!(((eye0 - solid).length() - (start - solid).length()).abs() < 1e-3);
    }

    #[test]
    fn parse_enums() {
        assert_eq!("focus".parse::<CameraMode>().unwrap(), CameraMode::Focus);
        assert_eq!("ORBIT".parse::<CameraMode>().unwrap(), CameraMode::Orbit);
        assert_eq!("cw".parse::<OrbitDir>().unwrap(), OrbitDir::Cw);
        assert!("sideways".parse::<OrbitDir>().is_err());
    }
}
