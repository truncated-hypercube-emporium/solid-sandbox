//! Rigid solids bouncing in a container (cube, octahedron, dodecahedron or
//! icosahedron) with the volume of a cube about 10 solid radii on a side.
//!
//! Two collision models:
//! - spheres (the default): solids collide as spheres sized between their
//!   inradius and circumradius, so pointy ones can graze slightly and blunt
//!   ones barely gap; their spin is for show.
//! - exact ([`World::exact`]): each solid's corners and edge midpoints are
//!   tested against the other solid's faces and the container's walls, and
//!   every contact applies an impulse at that point, so off-centre hits spin
//!   solids up and they come to rest on a face. Orientation and angular
//!   velocity are real rigid-body state.
//!
//! Contacts lose a little energy, so motion slowly settles until a tap sends
//! a solid off again.

use glam::{Quat, Vec3, Vec3A};
use platomon::scene::sdf::{Polyhedron, SolidKind};
use std::f32::consts::TAU;

/// Circumradius of every solid, world units.
pub const RADIUS: f32 = 1.0;
/// Side of the bounding cube: roughly 10 solid radii.
pub const BOX_SIDE: f32 = 10.0 * RADIUS;
/// Height of the cube's floor above the y = 0 plane the renderer marches
/// above, so a corner poking past a solid's collision sphere is never cut.
pub const FLOOR: f32 = 0.4;
/// Speed a tapped solid leaves at, world units per second.
pub const TAP_SPEED: f32 = 9.0;
/// Starting speeds are up to this share of [`TAP_SPEED`].
pub const START_SPEED: f32 = 0.6;
/// Fewest and most solids at launch.
pub const MIN_SOLIDS: usize = 4;
pub const MAX_SOLIDS: usize = 24;
/// Bounce energy kept per contact (normal component).
pub const RESTITUTION: f32 = 0.96;
/// Under gravity: share of the sliding speed lost per sub-step in contact
/// (a single bounce loses 1%; resting on a wall stops sliding within a second).
const WALL_FRICTION: f32 = 0.01;
/// Under gravity: contacts closing slower than this don't bounce.
const REST_SPEED: f32 = 1.5;
/// Under gravity: bounce kept per contact, like dice rather than superballs.
const GRAVITY_RESTITUTION: f32 = 0.6;
/// Gravity strength at 1 g on the accelerometer, world units/s²: a solid
/// falls across the 10-unit container in about a second.
pub const GRAVITY: f32 = 20.0;
/// Standard gravity, m/s², as the accelerometer reports it.
pub const STANDARD_G: f32 = 9.806_65;
/// Physics sub-steps per simulated second unless set ([`World::step_hz`]).
pub const STEP_HZ: f32 = 240.0;
/// Longest simulated time one [`World::step`] call advances, seconds, so a
/// stall doesn't tunnel solids through each other.
pub const MAX_STEP_DT: f32 = 0.25;
/// Spin when still, radians per second, so a resting solid still turns
/// (sphere mode only).
const IDLE_SPIN: f32 = 0.12;
/// Exact mode, under gravity: Coulomb friction coefficient at contacts.
const FRICTION: f32 = 0.4;
/// Exact mode, under gravity: angular damping per second (rolling resistance).
const ANGULAR_DAMPING: f32 = 0.6;
/// Exact mode: random spin added by a tap, radians per second.
const TAP_SPIN: f32 = 3.0;

#[derive(Debug, Clone, Copy)]
pub struct Body {
    pub pos: Vec3A,
    pub vel: Vec3A,
    /// Collision radius (sphere mode).
    pub radius: f32,
    /// (pitch, yaw), radians (sphere mode's spin).
    pub angles: (f32, f32),
    /// Spin per unit speed: (pitch, yaw), signed.
    pub spin: (f32, f32),
    /// Orientation: body frame → world. Follows `angles` in sphere mode.
    pub rot: Quat,
    /// Angular velocity, world frame, radians per second (exact mode).
    pub omega: Vec3A,
    /// Mass relative to the heaviest solid (1 unless masses vary).
    pub mass: f32,
    /// Collision counter, 0..=255 (see [`hit_increment`]), and the fraction
    /// of a count carried towards the next one.
    pub hits: u8,
    pub hit_frac: f32,
    /// False once it has exploded: it no longer collides, moves or draws
    /// until [`World::respawn`].
    pub alive: bool,
    /// Size relative to full grown, [`SPAWN_SIZE`]..=1; `radius` is
    /// `base_radius` × this.
    pub size: f32,
    /// Collision radius at full size.
    pub base_radius: f32,
    /// Growth from `grow_from` (its spawn size) to full size: 0..=1, and 1
    /// once fully grown.
    pub grow_progress: f32,
    pub grow_from: f32,
    /// Collision growth: progress per impact (0 when it pops instead).
    pub grow_step: f32,
    /// Pop growth: progress per simulated second (0 when it grows by impact).
    pub pop_rate: f32,
    /// Collision growth earned but not yet shown, and the seconds until it is
    /// (the charge delay).
    pub grow_pending: f32,
    pub grow_timer: f32,
}

/// How new solids come in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spawn {
    /// At [`SPAWN_SIZE`], growing a step with each of its first
    /// [`GROW_IMPACTS`] impacts.
    Grow,
    /// At this share of full size, growing steadily to full size over a
    /// random [`POP_SECONDS`] seconds.
    Pop(f32),
}

/// Seconds a popped-in solid takes to reach full size (inclusive range).
pub const POP_SECONDS: (f32, f32) = (5.0, 15.0);

/// How an impact feeds collision growth: the delay before the growth shows,
/// simulated seconds, and the size of each step relative to the plain one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Charge {
    pub delay: f32,
    pub boost: f32,
}

impl Charge {
    /// "Medium": each step shows 50 ms after its impact.
    pub const MEDIUM: Self = Self { delay: 0.05, boost: 1.0 };
}

/// A new solid's size, relative to full grown.
pub const SPAWN_SIZE: f32 = 0.05;
/// A new solid reaches full size after this many impacts (inclusive range,
/// picked at random per solid); only then does it start taking damage.
pub const GROW_IMPACTS: (u8, u8) = (8, 32);
/// Smallest tap target radius, world units, so a just-spawned speck can
/// still be tapped.
const PICK_MIN: f32 = 0.35;

impl Body {
    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    /// Mass at its current size: [`mass`](Self::mass) (the density) × size³.
    pub fn m(&self) -> f32 {
        self.mass * self.size * self.size * self.size
    }

    /// Whether it is still growing (and so takes no damage).
    pub fn growing(&self) -> bool {
        self.grow_progress < 1.0
    }

    /// Move growth on to `progress` and resize to match.
    fn set_progress(&mut self, progress: f32) {
        self.grow_progress = progress.min(1.0);
        if !self.growing() {
            (self.grow_step, self.pop_rate, self.grow_pending, self.grow_timer) = (0.0, 0.0, 0.0, 0.0);
        }
        self.resize(self.grow_from + (1.0 - self.grow_from) * self.grow_progress);
    }

    /// Advance timed growth by `dt` simulated seconds: popping, or charged
    /// collision growth coming due.
    fn tick_growth(&mut self, dt: f32) {
        if !self.growing() || !self.alive {
            return;
        }
        if self.pop_rate > 0.0 {
            self.set_progress(self.grow_progress + self.pop_rate * dt);
        } else if self.grow_pending > 0.0 {
            self.grow_timer -= dt;
            if self.grow_timer <= 0.0 {
                let p = self.grow_progress + self.grow_pending;
                self.grow_pending = 0.0;
                self.set_progress(p);
            }
        }
    }

    /// Set its size, keeping the collision radius in step.
    fn resize(&mut self, size: f32) {
        self.size = size.clamp(SPAWN_SIZE, 1.0);
        self.radius = self.base_radius * self.size;
    }

    /// Angular speed, radians per second: the real one in exact mode, the
    /// display spin's rate otherwise.
    pub fn spin_rate(&self, exact: bool) -> f32 {
        if exact {
            self.omega.length()
        } else {
            (self.spin.0 * self.spin.0 + self.spin.1 * self.spin.1).sqrt() * (self.speed() + IDLE_SPIN)
        }
    }
}

/// Impacts closing slower than this don't count towards the collision
/// counter, so resting contact and gentle nudges do no damage.
pub const IMPACT_MIN: f32 = 1.5;

/// Collision counter increase for one impact, in counts of 255: from 0.1%
/// (a glancing touch) to 2.5% (a full-speed hit by a much heavier solid).
///
/// - speed factor `fv = min(closing / tap_speed, 1)`
/// - mass factor `fm = m_other / (m_self + m_other)`, 0.5 for equal masses
///   and for walls (immovable, but a wall hit is like meeting a mirror image)
/// - severity `s = min(2 · fm · fv, 1)`
/// - increase `= 255 × (0.1% + 2.4% × s)`; none below [`IMPACT_MIN`].
pub fn hit_increment(closing: f32, m_self: f32, m_other: Option<f32>, tap_speed: f32) -> f32 {
    if closing < IMPACT_MIN {
        return 0.0;
    }
    let fv = (closing / tap_speed.max(1e-3)).min(1.0);
    let fm = m_other.map_or(0.5, |m| m / (m_self + m).max(1e-6));
    let s = (2.0 * fm * fv).min(1.0);
    255.0 * (0.001 + 0.024 * s)
}

/// Explosion chance added for every full [`CHANCE_STEP`]% of the counter
/// above half.
pub const CHANCE_PER_STEP: f32 = 0.03;
pub const CHANCE_STEP: f32 = 4.0;

/// Chance that an impact which leaves the counter at `hits` makes the solid
/// explode: none up to 50% of 255, then 3% more for every full 4% above
/// that, so 3% from 54%, 18% from 74% and 36% at 255 (the most it gets).
/// Reaching 255 always explodes, twice as violently.
pub fn explosion_chance(hits: u8) -> f32 {
    let pct = hits as f32 / 255.0 * 100.0;
    if pct <= 50.0 {
        return 0.0;
    }
    CHANCE_PER_STEP * ((pct - 50.0) / CHANCE_STEP).floor()
}

/// The orientation the renderer's (pitch, yaw) describe: Ry(yaw) · Rx(pitch).
pub fn rot_from_angles((pitch, yaw): (f32, f32)) -> Quat {
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)
}

/// A solid's convex hull for exact contacts, in its own frame at
/// circumradius [`RADIUS`].
#[derive(Debug, Clone)]
pub struct Hull {
    /// Corners and edge midpoints: the points that can touch a face. The
    /// midpoints catch most edge-on-edge contacts, where no corner goes in.
    pub points: Vec<Vec3A>,
    /// Face planes: outward normal and distance from the centre.
    pub planes: Vec<(Vec3A, f32)>,
    /// Moment of inertia per unit mass. Every solid here has tetrahedral or
    /// higher symmetry, so its inertia tensor is a multiple of the identity.
    pub inertia: f32,
    /// How many of `points` are corners (the rest are edge midpoints).
    pub corners: usize,
}

impl Hull {
    pub fn new(poly: &Polyhedron) -> Self {
        let verts: Vec<Vec3A> = poly.vertices().into_iter().map(|v| v * RADIUS).collect();
        let mut points = verts.clone();
        points.extend(poly.edges().into_iter().map(|(a, b, _, _)| (verts[a] + verts[b]) * 0.5));
        let planes = poly.face_planes().into_iter().map(|(n, d)| (n, d * RADIUS)).collect();
        // a solid ball between the in- and circumsphere: within ~10% of the
        // true value for all fifteen solids
        let r = 0.5 * (poly.inradius() + 1.0) * RADIUS;
        Self { points, planes, inertia: 0.4 * r * r, corners: verts.len() }
    }

    /// [`depth`](Self::depth) for the hull scaled by `size`.
    fn depth_at(&self, q: Vec3A, size: f32) -> (f32, Vec3A) {
        let (s, n) = self.depth(q / size);
        (s * size, n)
    }

    /// Signed distance of local point `q` past the face it is nearest to
    /// leaving through (negative inside), and that face's normal.
    fn depth(&self, q: Vec3A) -> (f32, Vec3A) {
        let mut best = (f32::MIN, Vec3A::Y);
        for &(n, d) in &self.planes {
            let s = q.dot(n) - d;
            if s > best.0 {
                best = (s, n);
            }
        }
        best
    }
}

/// A placeholder body (the immovable wall in exact contacts).
const EMPTY: Body = Body {
    pos: Vec3A::ZERO,
    vel: Vec3A::ZERO,
    radius: 0.0,
    angles: (0.0, 0.0),
    spin: (0.0, 0.0),
    rot: Quat::IDENTITY,
    omega: Vec3A::ZERO,
    mass: 1.0,
    hits: 0,
    hit_frac: 0.0,
    alive: true,
    size: 1.0,
    base_radius: 0.0,
    grow_progress: 1.0,
    grow_from: 1.0,
    grow_step: 0.0,
    pop_rate: 0.0,
    grow_pending: 0.0,
    grow_timer: 0.0,
};

/// How contacts respond: the elasticity and damping settings, and whether
/// gravity is on.
#[derive(Debug, Clone, Copy)]
pub struct Response {
    /// Restitution without gravity: 0 (dead) to [`RESTITUTION`].
    pub elasticity: f32,
    /// Share of the impulse two solids exchange (1, 0.75 or 0.5); walls
    /// ignore it.
    pub damping: f32,
    pub gravity_on: bool,
}

impl Response {
    /// Bounce kept by a contact closing at `closing` speed. Under gravity,
    /// slow contacts are dead, so a pile can come to rest instead of
    /// buzzing, and the rest bounce like dice rather than superballs.
    pub fn bounce(&self, closing: f32) -> f32 {
        match (self.gravity_on, closing < REST_SPEED) {
            (false, _) => self.elasticity,
            (true, true) => 0.0,
            (true, false) => self.elasticity * (GRAVITY_RESTITUTION / RESTITUTION),
        }
    }

    /// Normal impulse in units of (reduced mass × closing speed): `1 + e`
    /// scaled by the damping share, but never so little that the solids
    /// keep closing (at worst they stop dead against each other).
    fn coefficient(&self, closing: f32, damping: f32) -> f32 {
        (damping * (1.0 + self.bounce(closing))).max(1.0)
    }

    /// The same for a wall, which takes no damping.
    fn wall(self) -> Self {
        Self { damping: 1.0, ..self }
    }
}

/// Impulse at world point `w` on body `p` (inverse mass and inertia `ip`)
/// against body `f` (`if_`; both zero for a wall), along normal `n`
/// (pointing from `f` to `p`), with friction under gravity. Returns the
/// closing speed it stopped (0 if they were already separating).
fn contact_impulse(p: &mut Body, ip: (f32, f32), f: &mut Body, if_: (f32, f32), w: Vec3A, n: Vec3A, r: Response) -> f32 {
    let ((mp, jp), (mf, jf)) = (ip, if_);
    let (rp, rf) = (w - p.pos, w - f.pos);
    let rel = |p: &Body, f: &Body| p.vel + p.omega.cross(rp) - f.vel - f.omega.cross(rf);
    let vn = rel(p, f).dot(n);
    if vn >= 0.0 {
        return 0.0; // separating
    }
    let (rpn, rfn) = (rp.cross(n), rf.cross(n));
    let k = mp + mf + rpn.length_squared() * jp + rfn.length_squared() * jf;
    let jn = -r.coefficient(-vn, r.damping) * vn / k;
    p.vel += n * (jn * mp);
    p.omega += rpn * (jn * jp);
    f.vel -= n * (jn * mf);
    f.omega -= rfn * (jn * jf);
    if !r.gravity_on {
        return -vn;
    }
    // Coulomb friction: stop the slide, up to FRICTION × the normal impulse
    let v = rel(p, f);
    let vt = v - n * v.dot(n);
    let len = vt.length();
    if len < 1e-6 {
        return -vn;
    }
    let t = vt / len;
    let (rpt, rft) = (rp.cross(t), rf.cross(t));
    let kt = mp + mf + rpt.length_squared() * jp + rft.length_squared() * jf;
    let jt = (len / kt).min(FRICTION * jn);
    p.vel -= t * (jt * mp);
    p.omega -= rpt * (jt * jp);
    f.vel += t * (jt * mf);
    f.omega += rft * (jt * jf);
    -vn
}

/// Inverse mass and inverse moment of inertia of body `b` with hull `h`.
fn inverse_mass(b: &Body, h: &Hull) -> (f32, f32) {
    // inertia per unit mass scales with size²
    (1.0 / b.m(), 1.0 / (b.m() * h.inertia * b.size * b.size))
}

/// Points of `p` inside `f`: an impulse at each; the deepest one's (depth,
/// normal from `f` to `p`) for pushing them apart, and the fastest closing
/// speed among them.
fn points_into(p: &mut Body, hp: &Hull, f: &mut Body, hf: &Hull, r: Response) -> (Option<(f32, Vec3A)>, f32) {
    let (ip, if_) = (inverse_mass(p, hp), inverse_mass(f, hf));
    let to_f = f.rot.conjugate();
    let mut deepest: Option<(f32, Vec3A)> = None;
    let mut closing = 0.0f32;
    for &q in &hp.points {
        let w = p.pos + p.rot * (q * p.size);
        let (s, nl) = hf.depth_at(to_f * (w - f.pos), f.size);
        if s >= 0.0 {
            continue;
        }
        let n = f.rot * nl;
        if deepest.is_none_or(|(d, _)| -s > d) {
            deepest = Some((-s, n));
        }
        closing = closing.max(contact_impulse(p, ip, f, if_, w, n, r));
    }
    (deepest, closing)
}

/// "Strong" explosions: the shock wave starts at the exploded solid's
/// radius and grows to this many times it (1.5 × 1.5) over this many
/// simulated seconds; spin makes it reach up to [`SPIN_REACH`] times further.
pub const SHOCK_REACH: f32 = 2.25;
pub const SHOCK_TIME: f32 = 0.05;
pub const SPIN_REACH: f32 = 2.0;
/// Rim speed (spin × radius) at which spin counts in full, as a share of
/// tap speed: [`World::spin_factor`] is 1 there and beyond.
pub const SPIN_FULL: f32 = 1.0;

/// The invisible sphere a "Strong" explosion sends out.
#[derive(Debug, Clone)]
pub struct Shock {
    /// The solid that exploded (the wave doesn't touch it).
    pub source: usize,
    pub centre: Vec3A,
    /// Its radius at the blast (the solid's) and at full size.
    pub start: f32,
    pub reach: f32,
    /// The exploded solid's mass, which the wave's blow is worth.
    pub mass: f32,
    /// Simulated seconds since the blast.
    pub age: f32,
    /// Which solids it has already damaged.
    pub struck: Vec<bool>,
}

impl Shock {
    /// Its radius now: growing steadily from [`start`](Self::start) to
    /// [`reach`](Self::reach) at [`SHOCK_TIME`].
    pub fn radius(&self) -> f32 {
        self.start + (self.reach - self.start) * (self.age / SHOCK_TIME).min(1.0)
    }
}

/// Drag from the damping setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// Air drag on speed and on spin, per simulated second (velocity falls
    /// by `e^(-linear·t)`, angular velocity by `e^(-angular·t)`).
    pub linear: f32,
    pub angular: f32,
    /// Share of a solid's spin each counted impact takes away.
    pub impact_spin: f32,
}

impl Drag {
    pub const NONE: Self = Self { linear: 0.0, angular: 0.0, impact_spin: 0.0 };

    /// Slow `b` down by `h` seconds of air drag.
    fn apply(&self, b: &mut Body, h: f32) {
        if self.linear > 0.0 {
            b.vel *= (-self.linear * h).exp();
        }
        if self.angular > 0.0 {
            b.omega *= (-self.angular * h).exp();
        }
    }
}

/// Container shapes, one picked at random per launch.
pub const CONTAINERS: [SolidKind; 4] =
    [SolidKind::Cube, SolidKind::Octahedron, SolidKind::Dodecahedron, SolidKind::Icosahedron];

pub struct World {
    pub bodies: Vec<Body>,
    /// Speed a tapped solid leaves at ([`TAP_SPEED`] unless set).
    pub tap_speed: f32,
    /// Acceleration on every body, world units/s² (zero: no gravity).
    pub gravity: Vec3A,
    /// Exact face/corner contacts instead of spheres (needs [`hulls`](Self::set_hulls)).
    pub exact: bool,
    /// Restitution of every contact without gravity ([`RESTITUTION`] unless set).
    pub elasticity: f32,
    /// Share of the impulse two solids exchange per collision (1 unless set).
    pub damping: f32,
    /// Drag on speed and spin (none unless set).
    pub drag: Drag,
    /// Physics sub-steps per simulated second ([`STEP_HZ`] unless set).
    pub step_hz: f32,
    /// Whether impacts can make solids explode (the counter runs either way).
    pub explosions: bool,
    /// Whether respawned solids start small and grow ([`start_growth`](Self::start_growth)).
    pub growth: bool,
    /// How they start small and grow.
    pub spawn: Spawn,
    /// How impacts feed collision growth.
    pub charge: Charge,
    /// Whether explosions also send out a shock wave ("Strong").
    pub strong: bool,
    /// Shock waves spreading now.
    pub shocks: Vec<Shock>,
    /// Impacts counted so far (closing at [`IMPACT_MIN`] or faster), and
    /// solids destroyed.
    pub collisions: u64,
    pub destroyed: u64,
    /// Bodies that exploded since the caller last drained this, with the
    /// explosion's violence (1, or 2 for one that reached 255).
    pub blasts: Vec<(usize, f32)>,
    /// This sub-step's impacts: body, the body it hit (`None`: a wall), and
    /// the closing speed.
    impacts: Vec<(usize, Option<usize>, f32)>,
    /// Dice for explosions.
    rng: fastrand::Rng,
    /// Per-body hulls for exact mode.
    hulls: Vec<Hull>,
    /// Container shape.
    pub shape: SolidKind,
    /// Container centre.
    center: Vec3A,
    /// Container faces: outward normal and distance from the centre.
    pub planes: Vec<(Vec3A, f32)>,
    /// Container corners (world space) and the edges joining them.
    pub verts: Vec<Vec3A>,
    pub edges: Vec<(usize, usize)>,
    /// Bounding box of the container.
    pub min: Vec3A,
    pub max: Vec3A,
}

/// Volume of the convex polyhedron with these face planes and vertices.
pub fn volume(planes: &[(Vec3A, f32)], verts: &[Vec3A]) -> f32 {
    let mut vol = 0.0;
    for &(n, d) in planes {
        let face: Vec<Vec3A> = verts.iter().copied().filter(|v| (v.dot(n) - d).abs() < 1e-3 * d.abs().max(1.0)).collect();
        if face.len() < 3 {
            continue;
        }
        let m = face.iter().copied().sum::<Vec3A>() / face.len() as f32;
        let u = (face[0] - m).normalize();
        let w = n.cross(u);
        let mut ring: Vec<(f32, Vec3A)> = face.iter().map(|&v| ((v - m).dot(w).atan2((v - m).dot(u)), v)).collect();
        ring.sort_by(|a, b| a.0.total_cmp(&b.0));
        let area: f32 =
            (0..ring.len()).map(|i| (ring[i].1 - m).cross(ring[(i + 1) % ring.len()].1 - m).dot(n) * 0.5).sum();
        vol += area.abs() * d / 3.0;
    }
    vol
}

/// Uniformly random unit vector.
pub fn random_dir(rng: &mut fastrand::Rng) -> Vec3A {
    let z = rng.f32() * 2.0 - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let (s, c) = (TAU * rng.f32()).sin_cos();
    Vec3A::new(r * c, r * s, z)
}

impl World {
    /// Centre of the container.
    pub fn centre(&self) -> Vec3A {
        self.center
    }

    /// Whether a sphere of radius `r` at `p` lies inside the container.
    pub fn contains(&self, p: Vec3A, r: f32) -> bool {
        self.planes.iter().all(|&(n, d)| (p - self.center).dot(n) <= d - r + 1e-3)
    }

    /// Non-overlapping bodies with random positions, speeds and spins in a
    /// container of the given shape, with the volume of a [`BOX_SIDE`] cube;
    /// `inradii` (relative to a unit circumradius) give the count and sizes.
    pub fn new(inradii: &[f32], shape: SolidKind, rng: &mut fastrand::Rng) -> Self {
        let poly = Polyhedron::new(shape);
        let (unit_planes, unit_verts) = (poly.face_planes(), poly.vertices());
        let s = (BOX_SIDE.powi(3) / volume(&unit_planes, &unit_verts)).cbrt();
        let below = unit_verts.iter().map(|v| -v.y).fold(0.0f32, f32::max) * s;
        let center = Vec3A::new(0.0, FLOOR + below, 0.0);
        let verts: Vec<Vec3A> = unit_verts.iter().map(|&v| center + v * s).collect();
        let (min, max) = verts.iter().fold((Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN)), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        });
        let mut w = Self {
            bodies: Vec::with_capacity(inradii.len()),
            tap_speed: TAP_SPEED,
            gravity: Vec3A::ZERO,
            exact: false,
            elasticity: RESTITUTION,
            damping: 1.0,
            drag: Drag::NONE,
            step_hz: STEP_HZ,
            explosions: false,
            growth: false,
            spawn: Spawn::Grow,
            charge: Charge::MEDIUM,
            strong: false,
            shocks: Vec::new(),
            collisions: 0,
            destroyed: 0,
            blasts: Vec::new(),
            impacts: Vec::new(),
            rng: fastrand::Rng::with_seed(0x5eed ^ inradii.len() as u64),
            hulls: Vec::new(),
            shape,
            center,
            planes: unit_planes.iter().map(|&(n, d)| (n, d * s)).collect(),
            verts,
            edges: poly.edges().into_iter().map(|(a, b, _, _)| (a, b)).collect(),
            min,
            max,
        };
        for &ir in inradii {
            let radius = 0.5 * (ir + 1.0) * RADIUS;
            let lo = min + Vec3A::splat(radius);
            let span = max - min - Vec3A::splat(2.0 * radius);
            let mut pos = center;
            for _ in 0..2000 {
                let p = lo + span * Vec3A::new(rng.f32(), rng.f32(), rng.f32());
                if w.contains(p, radius) && w.bodies.iter().all(|b| (b.pos - p).length() > b.radius + radius + 0.05) {
                    pos = p;
                    break;
                }
            }
            let speed = TAP_SPEED * START_SPEED * rng.f32();
            let sign = |rng: &mut fastrand::Rng| if rng.bool() { 1.0 } else { -1.0 };
            let spin = (sign(rng) * (0.15 + 0.35 * rng.f32()), sign(rng) * (0.15 + 0.35 * rng.f32()));
            let angles = (TAU * rng.f32(), TAU * rng.f32());
            let rot = rot_from_angles(angles);
            w.bodies.push(Body {
                pos,
                vel: random_dir(rng) * speed,
                radius,
                angles,
                spin,
                rot,
                // a slow tumble about a body axis, from the spin already drawn
                omega: rot * Vec3A::new(spin.0, spin.1, 0.0) * 2.0,
                mass: 1.0,
                hits: 0,
                hit_frac: 0.0,
                alive: true,
                size: 1.0,
                base_radius: radius,
                grow_progress: 1.0,
                grow_from: 1.0,
                grow_step: 0.0,
                pop_rate: 0.0,
                grow_pending: 0.0,
                grow_timer: 0.0,
            });
        }
        w
    }

    /// Give the bodies different densities: the lightest is `lightest` × the
    /// heaviest, the rest spread evenly (on a log scale) between, in random
    /// order. `1.0` makes them all equal.
    pub fn vary_mass(&mut self, lightest: f32, rng: &mut fastrand::Rng) {
        let n = self.bodies.len();
        let mut order: Vec<usize> = (0..n).collect();
        rng.shuffle(&mut order);
        for (b, k) in self.bodies.iter_mut().zip(order) {
            let u = if n > 1 { k as f32 / (n - 1) as f32 } else { 0.0 };
            b.mass = lightest.clamp(1e-3, 1.0).powf(u);
        }
    }

    /// How contacts respond right now.
    pub fn response(&self) -> Response {
        Response { elasticity: self.elasticity, damping: self.damping, gravity_on: self.gravity != Vec3A::ZERO }
    }

    /// Send body `i` off in a random direction at [`tap_speed`](Self::tap_speed).
    pub fn kick(&mut self, i: usize, rng: &mut fastrand::Rng) {
        let (speed, exact) = (self.tap_speed, self.is_exact());
        if let Some(b) = self.bodies.get_mut(i).filter(|b| b.alive) {
            b.vel = random_dir(rng) * speed;
            if exact {
                b.omega += random_dir(rng) * TAP_SPIN;
            }
        }
    }

    /// Give each body its solid's hull, for exact mode (one per body, in order).
    pub fn set_hulls(&mut self, polys: &[Polyhedron]) {
        self.hulls = polys.iter().map(Hull::new).collect();
    }

    /// Exact contacts in force: switched on and every body has a hull.
    pub fn is_exact(&self) -> bool {
        self.exact && self.hulls.len() == self.bodies.len()
    }

    /// Whether body `i` is wholly inside the container: its hull in exact
    /// mode, its collision sphere otherwise.
    pub fn body_inside(&self, i: usize) -> bool {
        let b = &self.bodies[i];
        if !b.alive {
            true
        } else if self.is_exact() {
            self.hulls[i].points[..self.hulls[i].corners].iter().all(|&q| self.contains(b.pos + b.rot * (q * b.size), 0.0))
        } else {
            self.contains(b.pos, b.radius)
        }
    }

    /// Total kinetic energy ×2, rotation included in exact mode.
    pub fn energy(&self) -> f32 {
        (0..self.bodies.len()).map(|i| self.bodies[i].mass * self.specific_energy(i)).sum()
    }

    /// Body `i`'s kinetic energy per unit mass ×2: v², plus I·ω² in exact
    /// mode (sphere mode's spin is for show).
    pub fn specific_energy(&self, i: usize) -> f32 {
        let b = &self.bodies[i];
        if !b.alive {
            return 0.0;
        }
        let spin = if self.is_exact() { self.hulls[i].inertia * b.size * b.size * b.omega.length_squared() } else { 0.0 };
        b.vel.length_squared() + spin
    }

    /// Distance from world point `p` to body `i`'s surface (negative
    /// inside): its faces when it has a hull, its collision sphere otherwise.
    pub fn surface_distance(&self, i: usize, p: Vec3A) -> f32 {
        let b = &self.bodies[i];
        match self.hulls.get(i) {
            Some(h) if self.hulls.len() == self.bodies.len() => h.depth_at(b.rot.conjugate() * (p - b.pos), b.size).0,
            _ => (p - b.pos).length() - b.radius,
        }
    }

    /// Nearest body whose (slightly enlarged, for fingertips) collision
    /// sphere the ray `ro + t·rd` passes through.
    pub fn pick(&self, ro: Vec3A, rd: Vec3A, slack: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, b) in self.bodies.iter().enumerate().filter(|(_, b)| b.alive) {
            let r = (b.radius * slack).max(PICK_MIN);
            let oc = ro - b.pos;
            let bq = oc.dot(rd);
            let c = oc.length_squared() - r * r;
            let disc = bq * bq - c;
            if disc < 0.0 {
                continue;
            }
            let t = -bq - disc.sqrt();
            if t > 0.0 && best.is_none_or(|(_, bt)| t < bt) {
                best = Some((i, t));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Advance by `dt` simulated seconds (at most [`MAX_STEP_DT`]).
    pub fn step(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, MAX_STEP_DT);
        let n = (dt * self.step_hz.max(1.0)).ceil().max(1.0) as usize;
        let h = dt / n as f32;
        if self.is_exact() {
            for _ in 0..n {
                self.substep_exact(h);
                self.settle_impacts();
                self.spread_shocks(h);
                self.tick_growth(h);
            }
            return;
        }
        for _ in 0..n {
            self.substep(h);
            self.settle_impacts();
            self.spread_shocks(h);
            self.tick_growth(h);
        }
        for b in &mut self.bodies {
            let s = b.speed() + IDLE_SPIN;
            b.angles.0 = (b.angles.0 + b.spin.0 * s * dt).rem_euclid(TAU);
            b.angles.1 = (b.angles.1 + b.spin.1 * s * dt).rem_euclid(TAU);
            b.rot = rot_from_angles(b.angles);
        }
    }

    /// Exact mode: rigid bodies with corner/face contacts.
    fn substep_exact(&mut self, h: f32) {
        let g = self.gravity;
        let r = self.response();
        let damp = if r.gravity_on { (1.0 - ANGULAR_DAMPING * h).max(0.0) } else { 1.0 };
        let drag = self.drag;
        for b in self.bodies.iter_mut().filter(|b| b.alive) {
            drag.apply(b, h);
            b.vel += g * h;
            b.pos += b.vel * h;
            b.omega *= damp;
            b.rot = (Quat::from_scaled_axis(Vec3::from(b.omega * h)) * b.rot).normalize();
        }
        self.walls_exact();
        let n = self.bodies.len();
        for i in 0..n {
            for j in i + 1..n {
                let (mut a, mut b) = (self.bodies[i], self.bodies[j]);
                // circumspheres apart (or one gone): no contact possible
                if !(a.alive && b.alive) || (a.pos - b.pos).length() >= (a.size + b.size) * RADIUS {
                    continue;
                }
                let (ha, hb) = (&self.hulls[i], &self.hulls[j]);
                let (ab, c1) = points_into(&mut a, ha, &mut b, hb, r);
                let (ba, c2) = points_into(&mut b, hb, &mut a, ha, r);
                let ba = ba.map(|(d, n)| (d, -n));
                self.impacts.push((i, Some(j), c1.max(c2)));
                // push apart along the deepest contact, the lighter one further
                if let Some((d, nrm)) = [ab, ba].into_iter().flatten().max_by(|x, y| x.0.total_cmp(&y.0)) {
                    let share = b.m() / (a.m() + b.m());
                    a.pos += nrm * (d * share);
                    b.pos -= nrm * (d * (1.0 - share));
                }
                self.bodies[i] = a;
                self.bodies[j] = b;
            }
        }
        // contacts may have pushed a body back through a wall
        self.walls_exact();
    }

    /// Exact mode: corners and edge midpoints past a container wall get an
    /// impulse each; the body moves back by the deepest one per wall.
    fn walls_exact(&mut self) {
        let c = self.center;
        let r = self.response().wall();
        let mut wall = Body { pos: c, ..EMPTY };
        for (i, (b, hull)) in self.bodies.iter_mut().zip(&self.hulls).enumerate() {
            if !b.alive {
                continue;
            }
            let ip = inverse_mass(b, hull);
            let mut closing = 0.0f32;
            for &(n, d) in &self.planes {
                let mut deepest = 0.0f32;
                for &q in &hull.points {
                    let w = b.pos + b.rot * (q * b.size);
                    let pen = (w - c).dot(n) - d;
                    if pen > 0.0 {
                        deepest = deepest.max(pen);
                        // the wall is immovable: zero inverse mass and inertia;
                        // the normal points from the wall into the container
                        wall.vel = Vec3A::ZERO;
                        wall.omega = Vec3A::ZERO;
                        closing = closing.max(contact_impulse(b, ip, &mut wall, (0.0, 0.0), w, -n, r));
                    }
                }
                b.pos -= n * deepest;
            }
            self.impacts.push((i, None, closing));
        }
    }

    fn substep(&mut self, h: f32) {
        let g = self.gravity;
        let drag = self.drag;
        for b in self.bodies.iter_mut().filter(|b| b.alive) {
            drag.apply(b, h);
            b.vel += g * h;
            b.pos += b.vel * h;
        }
        self.confine();
        // impulses along the line of centres, shared by inverse mass
        let r = self.response();
        let friction = if r.gravity_on { WALL_FRICTION } else { 0.0 };
        let n = self.bodies.len();
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (self.bodies[i], self.bodies[j]);
                let d = b.pos - a.pos;
                let dist = d.length();
                let rsum = a.radius + b.radius;
                if !(a.alive && b.alive) || dist >= rsum || dist < 1e-6 {
                    continue;
                }
                let nrm = d / dist;
                let (ia, ib) = (1.0 / a.m(), 1.0 / b.m());
                let s = ia + ib;
                let over = rsum - dist;
                self.bodies[i].pos -= nrm * (over * ia / s);
                self.bodies[j].pos += nrm * (over * ib / s);
                let closing = (a.vel - b.vel).dot(nrm);
                if closing > 0.0 {
                    self.impacts.push((i, Some(j), closing));
                    let imp = nrm * (closing * r.coefficient(closing, r.damping) / s);
                    self.bodies[i].vel -= imp * ia;
                    self.bodies[j].vel += imp * ib;
                    // the same friction as on the walls, on the relative slide
                    let rel = a.vel - b.vel;
                    let slide = (rel - nrm * rel.dot(nrm)) * (friction / s);
                    self.bodies[i].vel -= slide * ia;
                    self.bodies[j].vel += slide * ib;
                }
            }
        }
        // contacts may have pushed a body back through a wall
        self.confine();
    }

    /// Push bodies back inside the container and bounce them off its walls.
    /// Several passes, since leaving one face in a sharp corner can cross
    /// the next.
    fn confine(&mut self) {
        let c = self.center;
        let r = self.response();
        let still = r.gravity_on;
        let bounce = |vn: f32| r.bounce(vn);
        for (i, b) in self.bodies.iter_mut().enumerate() {
            if !b.alive {
                continue;
            }
            let mut closing = 0.0f32;
            for _ in 0..4 {
                let mut moved = false;
                for &(n, d) in &self.planes {
                    let over = (b.pos - c).dot(n) - (d - b.radius);
                    if over > 0.0 {
                        b.pos -= n * over;
                        moved = true;
                        let vn = b.vel.dot(n);
                        if vn > 0.0 {
                            closing = closing.max(vn);
                            b.vel -= n * (vn * (1.0 + bounce(vn)));
                        }
                        // sliding friction under gravity, so solids pressed on
                        // a wall come to rest instead of gliding forever
                        if still {
                            let vt = b.vel - n * b.vel.dot(n);
                            b.vel -= vt * WALL_FRICTION;
                        }
                    }
                }
                if !moved {
                    break;
                }
            }
            if closing > 0.0 {
                self.impacts.push((i, None, closing));
            }
        }
    }

    /// Count this sub-step's impacts on each solid's collision counter and
    /// roll for explosions.
    fn settle_impacts(&mut self) {
        let impacts = std::mem::take(&mut self.impacts);
        for &(i, other, closing) in &impacts {
            if closing < IMPACT_MIN {
                continue;
            }
            self.collisions += 1;
            // each counted impact also knocks spin off what it hits
            let keep = 1.0 - self.drag.impact_spin;
            self.bodies[i].omega *= keep;
            if let Some(j) = other {
                self.bodies[j].omega *= keep;
            }
            match other {
                None => self.hit(i, None, closing),
                Some(j) => {
                    // both take the blow, each weighted by the other's mass
                    let (mi, mj) = (self.bodies[i].m(), self.bodies[j].m());
                    self.hit(i, Some(mj), closing);
                    self.hit(j, Some(mi), closing);
                }
            }
        }
        self.impacts = impacts;
        self.impacts.clear();
    }

    /// One impact on body `i` from something of mass `other` (`None`: a wall).
    fn hit(&mut self, i: usize, other: Option<f32>, closing: f32) {
        let b = &mut self.bodies[i];
        if !b.alive {
            return;
        }
        if b.growing() {
            // a young solid takes no damage; growing by impact, it earns a
            // step, shown after the charge delay (popping ignores impacts)
            if b.grow_step > 0.0 {
                if self.charge.delay > 0.0 {
                    if b.grow_pending <= 0.0 {
                        b.grow_timer = self.charge.delay;
                    }
                    b.grow_pending += b.grow_step;
                } else {
                    b.set_progress(b.grow_progress + b.grow_step);
                }
            }
            return;
        }
        let total = b.hits as f32 + b.hit_frac + hit_increment(closing, b.m(), other, self.tap_speed);
        if total >= 255.0 {
            // survived every roll on the way up: it goes off twice as hard
            (b.hits, b.hit_frac) = (255, 0.0);
            if self.explosions {
                self.blast(i, 2.0);
            }
            return;
        }
        (b.hits, b.hit_frac) = (total as u8, total.fract());
        let chance = explosion_chance(b.hits);
        if self.explosions && chance > 0.0 && self.rng.f32() < chance {
            self.blast(i, 1.0);
        }
    }

    /// Explode body `i`: its collision mesh ceases to exist (it no longer
    /// moves or collides) and the caller is told, to throw its faces out.
    pub fn blast(&mut self, i: usize, violence: f32) {
        if let Some(b) = self.bodies.get_mut(i).filter(|b| b.alive) {
            b.alive = false;
            self.destroyed += 1;
            self.blasts.push((i, violence));
            if self.strong {
                let n = self.bodies.len();
                let b = self.bodies[i];
                let reach = SHOCK_REACH * b.radius * (1.0 + (SPIN_REACH - 1.0) * self.spin_factor(i));
                self.shocks.push(Shock { source: i, centre: b.pos, start: b.radius, reach, mass: b.m(), age: 0.0, struck: vec![false; n] });
            }
        }
    }

    /// How hard body `i` is spinning, 0..=1: a rough measure of the
    /// centripetal momentum at its rim, its rim speed (spin × radius) as a
    /// share of [`SPIN_FULL`] × tap speed. Real spin in exact mode, the
    /// display spin otherwise.
    pub fn spin_factor(&self, i: usize) -> f32 {
        let b = &self.bodies[i];
        (b.spin_rate(self.is_exact()) * b.base_radius * b.size / (SPIN_FULL * self.tap_speed)).clamp(0.0, 1.0)
    }

    /// Grow the shock waves by `h` simulated seconds: every live solid the
    /// sphere reaches is pushed out to its edge, moving outward at least as
    /// fast as the edge, and takes one blow (the first time only) as if hit
    /// at tap speed by a solid of the exploded one's mass.
    fn spread_shocks(&mut self, h: f32) {
        if self.shocks.is_empty() {
            return;
        }
        let mut blows = Vec::new();
        for s in &mut self.shocks {
            s.age += h;
            let r = s.radius();
            let edge_speed = (s.reach - s.start) / SHOCK_TIME;
            for (j, b) in self.bodies.iter_mut().enumerate() {
                if j == s.source || !b.alive {
                    continue;
                }
                let d = b.pos - s.centre;
                let dist = d.length();
                if dist >= r + b.radius {
                    continue;
                }
                let out = if dist > 1e-5 { d / dist } else { Vec3A::Y };
                b.pos = s.centre + out * (r + b.radius);
                let vn = b.vel.dot(out);
                if vn < edge_speed {
                    b.vel += out * (edge_speed - vn);
                }
                if !s.struck[j] {
                    s.struck[j] = true;
                    blows.push((j, s.mass));
                }
            }
        }
        self.shocks.retain(|s| s.age < SHOCK_TIME);
        let speed = self.tap_speed;
        for (j, mass) in blows {
            self.hit(j, Some(mass), speed);
        }
    }

    /// Bring exploded body `i` back as a fresh solid: at rest, with a clean
    /// counter, somewhere it doesn't overlap a live one (if one can be found).
    pub fn respawn(&mut self, i: usize, rng: &mut fastrand::Rng) {
        // look for room for it at full size, so it can grow where it lands
        let radius = self.bodies[i].base_radius;
        let (lo, span) = (self.min + Vec3A::splat(radius), self.max - self.min - Vec3A::splat(2.0 * radius));
        let mut pos = self.center;
        for _ in 0..2000 {
            let p = lo + span * Vec3A::new(rng.f32(), rng.f32(), rng.f32());
            let clear = self.bodies.iter().enumerate().all(|(j, b)| j == i || !b.alive || (b.pos - p).length() > b.radius + radius + 0.05);
            if self.contains(p, radius) && clear {
                pos = p;
                break;
            }
        }
        let angles = (TAU * rng.f32(), TAU * rng.f32());
        let b = &mut self.bodies[i];
        b.pos = pos;
        b.vel = Vec3A::ZERO;
        b.angles = angles;
        b.rot = rot_from_angles(angles);
        b.omega = Vec3A::ZERO;
        (b.hits, b.hit_frac, b.alive) = (0, 0.0, true);
        if self.growth {
            self.start_growth(i);
        }
    }

    /// Start body `i` small, to grow back to full size the [`spawn`](Self::spawn)
    /// way: by a random [`GROW_IMPACTS`] number of impacts (each step scaled
    /// by the charge boost), or popping up over a random [`POP_SECONDS`].
    pub fn start_growth(&mut self, i: usize) {
        let (from, step, rate) = match self.spawn {
            Spawn::Grow => {
                let impacts = self.rng.u8(GROW_IMPACTS.0..=GROW_IMPACTS.1) as f32;
                (SPAWN_SIZE, self.charge.boost / impacts, 0.0)
            }
            Spawn::Pop(from) => {
                let secs = POP_SECONDS.0 + (POP_SECONDS.1 - POP_SECONDS.0) * self.rng.f32();
                (from, 0.0, 1.0 / secs)
            }
        };
        let b = &mut self.bodies[i];
        (b.grow_from, b.grow_step, b.pop_rate, b.grow_pending, b.grow_timer) = (from, step, rate, 0.0, 0.0);
        b.set_progress(0.0);
    }

    /// Advance timed growth for every body, then move any that grew into a
    /// wall back inside (no bounce: growing isn't an impact).
    fn tick_growth(&mut self, dt: f32) {
        let mut any = false;
        for b in &mut self.bodies {
            any |= b.growing() && b.alive;
            b.tick_growth(dt);
        }
        if !any {
            return;
        }
        let (c, exact) = (self.center, self.is_exact());
        for (i, b) in self.bodies.iter_mut().enumerate().filter(|(_, b)| b.alive) {
            for &(n, d) in &self.planes {
                let past = if exact {
                    let h = &self.hulls[i];
                    h.points.iter().map(|&q| (b.pos + b.rot * (q * b.size) - c).dot(n) - d).fold(0.0f32, f32::max)
                } else {
                    ((b.pos - c).dot(n) - (d - b.radius)).max(0.0)
                };
                b.pos -= n * past;
            }
        }
    }

    /// Bring every body to full size at once.
    pub fn finish_growth(&mut self) {
        for b in &mut self.bodies {
            b.set_progress(1.0);
        }
    }

    /// Seed the explosion dice.
    pub fn reseed(&mut self, seed: u64) {
        self.rng = fastrand::Rng::with_seed(seed);
    }
}

/// HSV (all 0..=1, hue wraps) → `0xRRGGBB`.
pub fn hsv(h: f32, s: f32, v: f32) -> u32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as u32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let c = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u32;
    (c(r) << 16) | (c(g) << 8) | c(b)
}

/// One gradient per solid: slow = deep and saturated, fast = bright and
/// pale, drifting a quarter turn round the colour wheel on the way. Hues are
/// spaced by the golden ratio so no two solids share a gradient.
pub fn gradient_stops(i: usize, hue0: f32) -> [u32; 3] {
    let h = hue0 + i as f32 * 0.618_034;
    [hsv(h, 0.9, 0.55), hsv(h + 0.12, 0.85, 0.85), hsv(h + 0.25, 0.35, 1.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(n: usize, seed: u64) -> World {
        world_in(n, seed, SolidKind::Cube)
    }

    fn world_in(n: usize, seed: u64, shape: SolidKind) -> World {
        let mut rng = fastrand::Rng::with_seed(seed);
        World::new(&vec![0.6; n], shape, &mut rng)
    }

    fn inside(w: &World) -> bool {
        w.bodies.iter().all(|b| w.contains(b.pos, b.radius))
    }

    #[test]
    fn containers_match_the_cube_volume_and_sit_above_the_floor() {
        for shape in CONTAINERS {
            let w = world_in(0, 0, shape);
            let local: Vec<Vec3A> = w.verts.iter().map(|&v| v - w.centre()).collect();
            let vol = volume(&w.planes, &local);
            assert!((vol / BOX_SIDE.powi(3) - 1.0).abs() < 1e-3, "{shape:?}: {vol}");
            assert!((w.min.y - FLOOR).abs() < 1e-4, "{shape:?}");
            let (faces, corners) = shape.counts();
            assert_eq!((w.planes.len(), w.verts.len()), (faces, corners), "{shape:?}");
            assert_eq!(w.edges.len(), faces + corners - 2, "{shape:?}: Euler");
        }
        // the cube is the 10-unit box of earlier versions
        let w = world(0, 0);
        assert!((w.min - Vec3A::new(-5.0, FLOOR, -5.0)).length() < 1e-3, "{:?}", w.min);
    }

    #[test]
    fn every_container_holds_its_solids() {
        for shape in CONTAINERS {
            let mut w = world_in(MAX_SOLIDS, 4, shape);
            assert!(inside(&w), "{shape:?} at launch");
            let mut rng = fastrand::Rng::with_seed(1);
            for i in 0..w.bodies.len() {
                w.kick(i, &mut rng);
            }
            for _ in 0..300 {
                w.step(1.0 / 30.0);
                assert!(inside(&w), "{shape:?}");
            }
        }
    }

    #[test]
    fn starts_inside_without_overlap() {
        for seed in 0..20 {
            let w = world_in(MAX_SOLIDS, seed, CONTAINERS[seed as usize % 4]);
            assert!(inside(&w));
            for i in 0..w.bodies.len() {
                for j in i + 1..w.bodies.len() {
                    let (a, b) = (w.bodies[i], w.bodies[j]);
                    assert!((a.pos - b.pos).length() >= a.radius + b.radius, "seed {seed}: {i} and {j} overlap");
                }
                assert!(w.bodies[i].speed() <= TAP_SPEED * START_SPEED + 1e-4);
            }
        }
    }

    #[test]
    fn stays_inside_and_never_gains_energy() {
        let mut w = world(MAX_SOLIDS, 3);
        let mut rng = fastrand::Rng::with_seed(9);
        for i in 0..w.bodies.len() {
            w.kick(i, &mut rng);
        }
        let energy = |w: &World| w.bodies.iter().map(|b| b.vel.length_squared()).sum::<f32>();
        let mut e = energy(&w);
        for _ in 0..600 {
            w.step(1.0 / 30.0);
            assert!(inside(&w));
            let e2 = energy(&w);
            assert!(e2 <= e * 1.0001 + 1e-4, "energy grew {e} -> {e2}");
            e = e2;
        }
        assert!(e > 0.0);
    }

    #[test]
    fn gravity_settles_solids_against_the_wall_it_points_at() {
        for shape in CONTAINERS {
            for g in [Vec3A::NEG_Y, Vec3A::X, Vec3A::new(0.3, 0.5, -0.8).normalize()] {
                let mut w = world_in(MAX_SOLIDS, 2, shape);
                w.gravity = g * GRAVITY;
                for _ in 0..30 * 12 {
                    w.step(1.0 / 30.0);
                    assert!(inside(&w), "{shape:?}");
                }
                let mean = w.bodies.iter().map(|b| b.pos).sum::<Vec3A>() / w.bodies.len() as f32;
                let drop = (mean - w.centre()).dot(g);
                assert!(drop > 2.0, "{shape:?} {g:?}: piled {drop:.2} towards gravity");
                let ke = w.bodies.iter().map(|b| b.vel.length_squared()).sum::<f32>() / w.bodies.len() as f32;
                assert!(ke < 1.0, "{shape:?} {g:?}: settles, mean v² {ke:.3}");
            }
        }
    }

    /// An exact-mode world of `n` solids cycling through all fifteen kinds.
    fn exact_world(n: usize, seed: u64, shape: SolidKind) -> World {
        let mut rng = fastrand::Rng::with_seed(seed);
        let polys: Vec<Polyhedron> = (0..n).map(|i| Polyhedron::new(SolidKind::ALL[(i + seed as usize) % 15])).collect();
        let inradii: Vec<f32> = polys.iter().map(|p| p.inradius()).collect();
        let mut w = World::new(&inradii, shape, &mut rng);
        w.set_hulls(&polys);
        w.exact = true;
        w
    }

    /// Deepest a corner of one solid sits inside another, or past a wall.
    fn worst_overlap(w: &World) -> f32 {
        let mut worst = 0.0f32;
        for (i, (a, ha)) in w.bodies.iter().zip(&w.hulls).enumerate() {
            for &q in &ha.points[..ha.corners] {
                let p = a.pos + a.rot * q;
                for &(n, d) in &w.planes {
                    worst = worst.max((p - w.centre()).dot(n) - d);
                }
                for (j, (b, hb)) in w.bodies.iter().zip(&w.hulls).enumerate() {
                    if i != j {
                        worst = worst.max(-hb.depth(b.rot.conjugate() * (p - b.pos)).0);
                    }
                }
            }
        }
        worst
    }

    #[test]
    fn hulls_have_corners_edges_and_inertia() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            let h = Hull::new(&p);
            let (faces, corners) = k.counts();
            assert_eq!(h.corners, corners, "{k:?}");
            assert_eq!(h.points.len(), corners + faces + corners - 2, "{k:?}: corners + edges");
            assert_eq!(h.planes.len(), faces);
            assert!(h.inertia > 0.15 && h.inertia < 0.4, "{k:?}: {}", h.inertia);
            for &q in &h.points {
                assert!(h.depth(q).0.abs() < 1e-3, "{k:?}: hull points lie on the surface");
            }
            assert!(h.depth(Vec3A::ZERO).0 < -0.3);
        }
    }

    #[test]
    fn exact_contacts_never_gain_energy_or_pass_through() {
        for (k, shape) in CONTAINERS.into_iter().enumerate() {
            let mut w = exact_world(MAX_SOLIDS, k as u64, shape);
            let mut rng = fastrand::Rng::with_seed(9);
            for i in 0..w.bodies.len() {
                w.kick(i, &mut rng);
            }
            // settle any overlap left by sphere-based placement first
            w.step(0.05);
            let mut e = w.energy();
            let mut worst = 0.0f32;
            for _ in 0..450 {
                w.step(1.0 / 30.0);
                let e2 = w.energy();
                assert!(e2 <= e * 1.0001 + 1e-4, "{shape:?}: energy grew {e} -> {e2}");
                e = e2;
                worst = worst.max(worst_overlap(&w));
            }
            assert!(worst < 0.12, "{shape:?}: a corner went {worst:.3} deep");
            assert!(e > 1.0, "{shape:?}: still moving");
            assert!(w.bodies.iter().all(|b| b.pos.is_finite() && b.omega.is_finite() && b.rot.is_normalized()));
        }
    }

    #[test]
    fn an_off_centre_hit_spins_a_solid() {
        let mut w = exact_world(1, 1, SolidKind::Cube); // seed 1: a cube
        assert_eq!(w.hulls[0].corners, 8);
        let b = &mut w.bodies[0];
        b.pos = w.center;
        b.rot = Quat::from_rotation_z(0.4) * Quat::from_rotation_x(0.3); // corner-first
        b.omega = Vec3A::ZERO;
        b.vel = Vec3A::NEG_Y * 6.0;
        for _ in 0..30 {
            w.step(1.0 / 30.0);
        }
        assert!(w.bodies[0].omega.length() > 0.5, "spun up: {:?}", w.bodies[0].omega);
        // a sphere would bounce back at 0.96 × 6; the corner hit turned some
        // of that into spin (a flat, frictionless wall can't push sideways)
        let b = w.bodies[0];
        assert!(b.speed() < 0.96 * 6.0 - 0.2, "{}", b.speed());
        assert!(b.vel.x.abs() + b.vel.z.abs() < 1e-3);
    }

    #[test]
    fn exact_solids_come_to_rest_on_a_face() {
        for k in [SolidKind::Cube, SolidKind::Tetrahedron, SolidKind::Octahedron, SolidKind::Dodecahedron] {
            let mut rng = fastrand::Rng::with_seed(4);
            let poly = Polyhedron::new(k);
            let mut w = World::new(&[poly.inradius()], SolidKind::Cube, &mut rng);
            w.set_hulls(&[poly]);
            w.exact = true;
            w.gravity = Vec3A::NEG_Y * GRAVITY;
            w.bodies[0].rot = Quat::from_rotation_z(0.7) * Quat::from_rotation_x(0.5);
            for _ in 0..30 * 6 {
                w.step(1.0 / 30.0);
            }
            let b = w.bodies[0];
            assert!(b.speed() < 0.05 && b.omega.length() < 0.05, "{k:?}: at rest, v {:.3} ω {:.3}", b.speed(), b.omega.length());
            let floor = w.min.y;
            let low = w.hulls[0].points[..w.hulls[0].corners]
                .iter()
                .filter(|&&q| (b.pos + b.rot * q).y - floor < 0.02)
                .count();
            assert!(low >= 3, "{k:?}: {low} corners on the floor, so it rests on a face");
        }
    }

    #[test]
    fn exact_piles_settle_under_gravity() {
        for shape in CONTAINERS {
            let mut w = exact_world(MAX_SOLIDS, 3, shape);
            w.gravity = Vec3A::NEG_Y * GRAVITY;
            let mut worst = 0.0f32;
            for _ in 0..30 * 10 {
                w.step(1.0 / 30.0);
                worst = worst.max(worst_overlap(&w));
            }
            let ke = w.energy() / w.bodies.len() as f32;
            assert!(ke < 0.5, "{shape:?}: settles, mean energy {ke:.3}");
            assert!(worst < 0.15, "{shape:?}: a corner went {worst:.3} deep");
        }
    }

    /// `cargo test --release exact_step_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn exact_step_cost() {
        let mut w = exact_world(MAX_SOLIDS, 3, SolidKind::Cube);
        w.gravity = Vec3A::NEG_Y * GRAVITY;
        for _ in 0..150 {
            w.step(1.0 / 30.0);
        }
        let t = std::time::Instant::now();
        for _ in 0..300 {
            w.step(1.0 / 30.0);
        }
        println!("exact, 12 piled solids: {:.2} ms per 1/30 s frame", t.elapsed().as_secs_f64() * 1e3 / 300.0);
    }

    #[test]
    fn kick_sets_fixed_speed_in_random_directions() {
        let mut w = world(2, 1);
        let mut rng = fastrand::Rng::with_seed(5);
        let mut dirs = Vec::new();
        for _ in 0..50 {
            w.kick(0, &mut rng);
            assert!((w.bodies[0].speed() - TAP_SPEED).abs() < 1e-4);
            dirs.push(w.bodies[0].vel / TAP_SPEED);
        }
        let mean = dirs.iter().copied().fold(Vec3A::ZERO, |a, b| a + b) / dirs.len() as f32;
        assert!(mean.length() < 0.35, "directions spread out: {mean:?}");
        w.kick(99, &mut rng); // out of range: ignored
    }

    #[test]
    fn head_on_equal_masses_swap_velocities() {
        let mut w = world(2, 0);
        w.bodies[0].pos = Vec3A::new(-2.0, 5.0, 0.0);
        w.bodies[1].pos = Vec3A::new(2.0, 5.0, 0.0);
        w.bodies[0].vel = Vec3A::new(4.0, 0.0, 0.0);
        w.bodies[1].vel = Vec3A::ZERO;
        for _ in 0..8 {
            w.step(0.1); // contact after ~0.6 s; then 0 travels < 1.1, 1 reaches ~5.9 < wall
        }
        assert!(w.bodies[0].vel.x.abs() < 0.3, "{:?}", w.bodies[0].vel);
        assert!(w.bodies[1].vel.x > 3.5);
    }

    /// Two solids head-on: 0 at 4 units/s towards 1 at rest. Velocities
    /// along x once they have parted.
    fn head_on(w: &mut World) -> (f32, f32) {
        w.bodies.truncate(2);
        w.bodies[0].pos = Vec3A::new(-2.0, 5.0, 0.0);
        w.bodies[1].pos = Vec3A::new(2.0, 5.0, 0.0);
        w.bodies[0].vel = Vec3A::new(4.0, 0.0, 0.0);
        w.bodies[1].vel = Vec3A::ZERO;
        for _ in 0..8 {
            w.step(0.1);
        }
        (w.bodies[0].vel.x, w.bodies[1].vel.x)
    }

    #[test]
    fn mass_spread_runs_from_lightest_to_heaviest() {
        let mut w = world(MAX_SOLIDS, 1);
        let mut rng = fastrand::Rng::with_seed(3);
        w.vary_mass(0.05, &mut rng);
        let m: Vec<f32> = w.bodies.iter().map(|b| b.mass).collect();
        let (lo, hi) = m.iter().fold((f32::MAX, 0.0f32), |(l, h), &x| (l.min(x), h.max(x)));
        assert!((hi - 1.0).abs() < 1e-5 && (lo - 0.05).abs() < 1e-5, "{m:?}");
        w.vary_mass(1.0, &mut rng);
        assert!(w.bodies.iter().all(|b| b.mass == 1.0));
    }

    #[test]
    fn a_heavy_solid_barely_slows_hitting_a_light_one() {
        let mut w = world(2, 0);
        w.bodies[0].mass = 1.0;
        w.bodies[1].mass = 0.05;
        let (a, b) = head_on(&mut w);
        // elastic: a keeps (m1-m2)/(m1+m2) ≈ 0.9 of its speed, b leaves at ~1.9×
        assert!(a > 3.0 && b > 6.5, "{a} {b}");
        // momentum is conserved (no wall reached yet)
        assert!((a + 0.05 * b - 4.0).abs() < 0.05, "{a} {b}");
        // and the other way round: the light one bounces back
        let mut w = world(2, 0);
        w.bodies[0].mass = 0.05;
        let (a, b) = head_on(&mut w);
        assert!(a < -3.0 && b > 0.0 && b < 0.5, "{a} {b}");
    }

    #[test]
    fn elasticity_off_makes_collisions_dead() {
        let mut w = world(2, 0);
        w.elasticity = 0.0;
        let (a, b) = head_on(&mut w);
        assert!((a - 2.0).abs() < 0.1 && (b - 2.0).abs() < 0.1, "they move off together: {a} {b}");
        // and a wall stops a solid instead of bouncing it
        let mut w = world(1, 0);
        w.elasticity = 0.0;
        w.bodies[0].pos = Vec3A::new(0.0, 5.0, 0.0);
        w.bodies[0].vel = Vec3A::new(6.0, 0.0, 0.0);
        for _ in 0..10 {
            w.step(0.1);
        }
        assert!(w.bodies[0].vel.x.abs() < 1e-3, "{:?}", w.bodies[0].vel);
        // low elasticity bounces less than high
        let mut w = world(1, 0);
        w.elasticity = 0.6;
        w.bodies[0].pos = Vec3A::new(0.0, 5.0, 0.0);
        w.bodies[0].vel = Vec3A::new(6.0, 0.0, 0.0);
        for _ in 0..10 {
            w.step(0.1);
        }
        assert!((w.bodies[0].vel.x + 3.6).abs() < 1e-3, "{:?}", w.bodies[0].vel);
    }

    #[test]
    fn damping_shares_the_exchange() {
        for (damping, want) in [(1.0, 4.0 * 0.98), (0.75, 4.0 * 0.75 * 0.98), (0.5, 2.0)] {
            let mut w = world(2, 0);
            w.damping = damping;
            let (a, b) = head_on(&mut w);
            assert!((b - want).abs() < 0.05, "damping {damping}: struck solid leaves at {b}, want {want}");
            assert!((a + b - 4.0).abs() < 0.05, "momentum kept: {a} {b}");
        }
    }

    #[test]
    fn exact_contacts_respect_mass_elasticity_and_damping() {
        for (k, shape) in CONTAINERS.into_iter().enumerate() {
            let mut w = exact_world(MAX_SOLIDS, k as u64, shape);
            let mut rng = fastrand::Rng::with_seed(9);
            w.vary_mass(0.05, &mut rng);
            w.elasticity = 0.6;
            w.damping = 0.5;
            for i in 0..w.bodies.len() {
                w.kick(i, &mut rng);
            }
            w.step(0.05);
            let mut e = w.energy();
            let e0 = e;
            for _ in 0..300 {
                w.step(1.0 / 30.0);
                let e2 = w.energy();
                assert!(e2 <= e * 1.0001 + 1e-4, "{shape:?}: energy grew {e} -> {e2}");
                e = e2;
            }
            assert!(e < 0.5 * e0, "{shape:?}: lossy contacts lose energy, {e0} -> {e}");
            assert!(worst_overlap(&w) < 0.15);
            assert!(w.bodies.iter().all(|b| b.pos.is_finite() && b.omega.is_finite()));
        }
    }

    #[test]
    fn surface_distance_uses_the_hull() {
        let mut w = exact_world(1, 1, SolidKind::Cube); // a cube, inradius 1/√3
        w.bodies[0].rot = Quat::IDENTITY;
        let c = w.bodies[0].pos;
        let ir = 1.0 / 3f32.sqrt();
        assert!((w.surface_distance(0, c + Vec3A::X * (ir + 0.3)) - 0.3).abs() < 1e-4);
        assert!(w.surface_distance(0, c) < -0.5);
    }

    #[test]
    fn damage_runs_from_a_tenth_to_two_and_a_half_percent() {
        let pct = |c, m, o| hit_increment(c, m, o, TAP_SPEED) / 255.0 * 100.0;
        assert_eq!(pct(IMPACT_MIN - 0.01, 1.0, Some(1.0)), 0.0, "gentle contact does no damage");
        assert!((pct(IMPACT_MIN, 1.0, Some(0.001)) - 0.1).abs() < 0.01, "a feather barely counts");
        assert!((pct(TAP_SPEED, 1.0, Some(1.0)) - 2.5).abs() < 1e-4, "full speed, equal masses");
        assert!((pct(TAP_SPEED * 3.0, 0.05, Some(1.0)) - 2.5).abs() < 1e-4, "capped");
        assert!((pct(TAP_SPEED / 2.0, 1.0, None) - (0.1 + 2.4 * 0.5)).abs() < 1e-4, "a wall counts as an equal mass");
        // heavier hitters do more damage, at every speed
        for v in [2.0, 4.0, 8.0] {
            assert!(pct(v, 1.0, Some(0.05)) < pct(v, 1.0, Some(1.0)) && pct(v, 1.0, Some(1.0)) <= pct(v, 0.05, Some(1.0)));
        }
    }

    #[test]
    fn explosion_chance_climbs_three_percent_per_four_above_half() {
        let at = |pct: f32| explosion_chance((pct / 100.0 * 255.0).ceil() as u8);
        assert_eq!(explosion_chance(0), 0.0);
        assert_eq!(explosion_chance(127), 0.0);
        assert_eq!(explosion_chance(137), 0.0, "53.7%");
        assert!((at(54.0) - 0.03).abs() < 1e-5 && (at(58.0) - 0.06).abs() < 1e-5 && (at(86.0) - 0.27).abs() < 1e-5);
        assert!((explosion_chance(255) - 0.36).abs() < 1e-5, "36% at the top");
        let mut last = 0.0;
        for c in 0..=255u8 {
            assert!(explosion_chance(c) >= last);
            last = explosion_chance(c);
        }
    }

    #[test]
    fn impacts_raise_the_counter_but_resting_does_not() {
        // a solid bouncing between two walls at tap speed
        let mut w = world(1, 0);
        w.bodies[0].pos = w.centre();
        w.bodies[0].vel = Vec3A::X * TAP_SPEED;
        for _ in 0..20 {
            w.step(0.1);
        }
        let b = w.bodies[0];
        // ~2 s at 9 u/s across a 8.4-unit gap: 2 wall hits of 2.5% (6.4 counts) each
        assert!(b.hits >= 10 && b.hits <= 16, "{} hits", b.hits);
        // under gravity, a solid resting on the floor stops counting
        let mut w = world(1, 0);
        w.gravity = Vec3A::NEG_Y * GRAVITY;
        for _ in 0..30 {
            w.step(0.1);
        }
        let settled = w.bodies[0].hits;
        for _ in 0..50 {
            w.step(0.1);
        }
        assert_eq!(w.bodies[0].hits, settled, "resting contact is not an impact");
    }

    #[test]
    fn exploded_solids_stop_colliding_and_respawn_fresh() {
        for exact in [false, true] {
            let mut w = if exact { exact_world(2, 1, SolidKind::Cube) } else { world(2, 0) };
            w.explosions = true;
            w.bodies[0].hits = 254;
            w.bodies[0].hit_frac = 0.9;
            head_on(&mut w);
            // the hit pushed 0 to 255: it exploded twice as hard
            assert_eq!(w.blasts, vec![(0, 2.0)], "exact {exact}");
            assert!(!w.bodies[0].alive && w.bodies[1].alive);
            assert_eq!(w.pick(w.bodies[0].pos + Vec3A::Z * 20.0, -Vec3A::Z, 1.0), None, "can't tap it");
            // its mesh is gone: 1 flies straight through where it was
            let c = w.centre();
            w.bodies[0].pos = c;
            w.bodies[1].pos = c - Vec3A::X * 2.0;
            w.bodies[1].vel = Vec3A::X * 4.0;
            w.bodies[1].omega = Vec3A::ZERO;
            w.step(0.25);
            w.step(0.25);
            assert!((w.bodies[1].vel - Vec3A::X * 4.0).length() < 1e-4, "exact {exact}: {:?}", w.bodies[1].vel);
            assert_eq!(w.bodies[0].pos, c, "a dead solid doesn't move");
            let mut rng = fastrand::Rng::with_seed(3);
            w.respawn(0, &mut rng);
            let r = w.bodies[0];
            assert!(r.alive && r.hits == 0 && r.vel == Vec3A::ZERO && w.contains(r.pos, r.radius));
            assert!((r.pos - w.bodies[1].pos).length() > r.radius + w.bodies[1].radius);
        }
        // explosions off: the counter saturates and nothing explodes
        let mut w = world(2, 0);
        w.bodies[0].hits = 254;
        head_on(&mut w);
        assert!(w.blasts.is_empty() && w.bodies[0].alive && w.bodies[0].hits == 255);
    }

    #[test]
    fn explosions_happen_at_random_past_half_the_counter() {
        let mut w = world(MAX_SOLIDS, 5);
        w.explosions = true;
        for b in &mut w.bodies {
            b.hits = 230; // 90%: 30% per impact
        }
        let mut rng = fastrand::Rng::with_seed(1);
        for i in 0..w.bodies.len() {
            w.kick(i, &mut rng);
        }
        for _ in 0..90 {
            w.step(1.0 / 30.0);
        }
        let n = w.blasts.len();
        assert!(n >= 4, "several explode within 3 s: {n}");
        // (one pushed to 255 goes off at 2×)
        assert!(w.blasts.iter().all(|&(i, v)| (v == 1.0 || v == 2.0) && !w.bodies[i].alive));
    }

    #[test]
    fn strong_explosions_push_neighbours_out_and_damage_them() {
        for exact in [false, true] {
            let mut w = if exact { exact_world(3, 4, SolidKind::Cube) } else { world(3, 4) };
            (w.explosions, w.strong) = (true, true);
            let c = w.centre();
            let r0 = w.bodies[0].radius;
            // 1 sits inside the wave's reach, 2 well outside it
            w.bodies[0].pos = c;
            w.bodies[1].pos = c + Vec3A::X * (r0 * 1.2 + w.bodies[1].radius * 0.5);
            w.bodies[2].pos = c - Vec3A::X * (r0 * SHOCK_REACH + w.bodies[2].radius + 1.0);
            for b in &mut w.bodies {
                (b.vel, b.omega) = (Vec3A::ZERO, Vec3A::ZERO);
            }
            let hits_before = w.bodies[1].hits;
            w.blast(0, 1.0);
            assert_eq!((w.shocks.len(), w.destroyed), (1, 1));
            let m0 = w.bodies[0].m();
            // it reaches its full size in 250 ms and is gone after
            // from the solid's own radius to full size in 50 ms, then gone
            let reach = w.shocks[0].reach;
            assert!(reach >= SHOCK_REACH * r0 && w.shocks[0].start == r0, "at least 2.25 radii");
            assert!((w.shocks[0].radius() - r0).abs() < 1e-6);
            w.step(SHOCK_TIME * 0.5);
            assert!(!w.shocks.is_empty() && (w.shocks[0].radius() - (r0 + reach) * 0.5).abs() < 0.05 * r0);
            w.step(SHOCK_TIME * 0.6);
            assert!(w.shocks.is_empty());
            let b1 = w.bodies[1];
            assert!((b1.pos - c).length() >= reach + b1.radius - 1e-3, "pushed out to the wave's edge");
            assert!(b1.vel.dot((b1.pos - c).normalize()) > 0.0, "moving away");
            // one blow, worth a tap-speed hit from a solid of the exploded one's mass
            let expect = hit_increment(w.tap_speed, b1.m(), Some(m0), w.tap_speed);
            assert!((b1.hits as f32 - hits_before as f32 - expect).abs() <= 1.5, "{} -> {} vs +{expect}", hits_before, b1.hits);
            assert_eq!((w.bodies[2].hits, w.bodies[2].vel), (0, Vec3A::ZERO), "out of reach");
            // spin makes the wave reach further: up to twice as far
            let mut s = if exact { exact_world(3, 4, SolidKind::Cube) } else { world(3, 4) };
            (s.explosions, s.strong) = (true, true);
            s.bodies[0].omega = Vec3A::Y * 50.0;
            s.bodies[0].spin = (30.0, 30.0);
            s.bodies[0].vel = Vec3A::X * 2.0;
            assert!(s.spin_factor(0) > 0.99, "fast spin counts in full: {}", s.spin_factor(0));
            s.blast(0, 1.0);
            assert!((s.shocks[0].reach - SHOCK_REACH * SPIN_REACH * s.bodies[0].radius).abs() < 1e-4);
            // a weak explosion sends no wave
            let mut w = world(3, 4);
            w.explosions = true;
            w.blast(0, 1.0);
            assert!(w.shocks.is_empty());
        }
    }

    #[test]
    fn damping_drags_speed_and_spin() {
        for exact in [false, true] {
            let mut w = if exact { exact_world(1, 2, SolidKind::Cube) } else { world(1, 2) };
            w.drag = Drag { linear: 0.2, angular: 1.2, impact_spin: 0.5 };
            let c = w.centre();
            (w.bodies[0].pos, w.bodies[0].vel, w.bodies[0].omega) = (c, Vec3A::X * 0.5, Vec3A::Y * 4.0);
            w.step(0.25);
            w.step(0.25);
            let b = w.bodies[0];
            assert!((b.vel.length() - 0.5 * (-0.2f32 * 0.5).exp()).abs() < 1e-3, "drag on speed: {}", b.vel.length());
            assert!((b.omega.length() - 4.0 * (-1.2f32 * 0.5).exp()).abs() < 0.05, "drag on spin: {}", b.omega.length());
        }
        // a counted impact knocks off half the spin
        let mut w = world(1, 2);
        w.drag = Drag { linear: 0.0, angular: 0.0, impact_spin: 0.5 };
        w.bodies[0].omega = Vec3A::Y * 4.0;
        w.impacts.push((0, None, IMPACT_MIN * 2.0));
        w.settle_impacts();
        assert!((w.bodies[0].omega.length() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn counts_collisions_and_destroyed_solids() {
        let mut w = world(2, 1);
        let c = w.centre();
        w.bodies[0].pos = c - Vec3A::X * 2.0;
        w.bodies[1].pos = c + Vec3A::X * 2.0;
        w.bodies[0].vel = Vec3A::X * 6.0;
        w.bodies[1].vel = -Vec3A::X * 6.0;
        w.step(0.5);
        assert!(w.collisions >= 1, "the head-on hit counts");
        let before = w.collisions;
        for b in &mut w.bodies {
            b.vel = Vec3A::X * 0.1; // too gentle to count
        }
        w.step(0.2);
        assert_eq!(w.collisions, before);
        w.blast(1, 1.0);
        w.blast(1, 1.0);
        assert_eq!(w.destroyed, 1, "once per solid");
    }

    #[test]
    fn new_solids_start_at_5_percent_and_grow_over_8_to_32_impacts() {
        for exact in [false, true] {
            let mut w = if exact { exact_world(1, 1, SolidKind::Cube) } else { world(1, 0) };
            w.charge = Charge { delay: 0.0, boost: 1.0 };
            let full = w.bodies[0].radius;
            let mut impacts = std::collections::BTreeSet::new();
            for _ in 0..300 {
                w.start_growth(0);
                impacts.insert((1.0 / w.bodies[0].grow_step).round() as u8);
            }
            assert!(impacts.iter().all(|t| (GROW_IMPACTS.0..=GROW_IMPACTS.1).contains(t)));
            assert!(impacts.contains(&8) && impacts.contains(&32), "the whole range: {impacts:?}");
            let b = w.bodies[0];
            assert!((b.size - SPAWN_SIZE).abs() < 1e-6 && (b.radius - full * SPAWN_SIZE).abs() < 1e-6);
            assert!((b.m() - b.mass * SPAWN_SIZE.powi(3)).abs() < 1e-9, "mass goes with volume");
            // bounce it between two walls: each wall hit is one growth step, no damage
            let target = (1.0 / b.grow_step).round() as usize;
            w.bodies[0].pos = w.centre();
            w.bodies[0].vel = Vec3A::X * TAP_SPEED;
            let mut sizes = vec![b.size];
            let mut steps = 0;
            while w.bodies[0].growing() && steps < 2000 {
                w.step(0.05);
                steps += 1;
                let s = w.bodies[0].size;
                assert!(s >= *sizes.last().unwrap(), "exact {exact}: only grows");
                if s > *sizes.last().unwrap() {
                    sizes.push(s);
                }
                assert_eq!(w.bodies[0].hits, 0, "no damage while growing");
                assert!(w.body_inside(0), "exact {exact}: stays in as it grows");
            }
            let b = w.bodies[0];
            assert!(!b.growing() && b.size == 1.0 && (b.radius - full).abs() < 1e-6, "exact {exact}: full size");
            assert_eq!(sizes.len() - 1, target, "exact {exact}: one step per impact");
            // then it wears as usual (it has slowed after all those bounces)
            for _ in 0..200 {
                w.step(0.05);
            }
            assert!(w.bodies[0].hits > 0, "exact {exact}: damage after growing");
        }
    }

    #[test]
    fn charge_delays_or_boosts_each_growth_step() {
        // one wall hit, then watch when (and how much) the solid grows
        let grow_after_one_hit = |charge: Charge| {
            let mut w = world(1, 0);
            w.charge = charge;
            w.start_growth(0);
            let step = w.bodies[0].grow_step;
            w.bodies[0].pos = w.centre() + Vec3A::X * 4.8;
            w.bodies[0].vel = Vec3A::X * 4.0;
            let mut hit_at = None;
            let mut grew_at = None;
            for k in 0..200 {
                w.step(0.002);
                let b = w.bodies[0];
                if hit_at.is_none() && (b.grow_pending > 0.0 || b.grow_progress > 0.0) {
                    hit_at = Some(k);
                }
                if grew_at.is_none() && b.grow_progress > 0.0 {
                    grew_at = Some(k);
                }
            }
            let (h, g) = (hit_at.unwrap(), grew_at.unwrap());
            ((g - h) as f32 * 0.002, w.bodies[0].grow_progress, step)
        };
        let (low, _, _) = grow_after_one_hit(Charge { delay: 0.1, boost: 1.0 });
        let (medium, _, _) = grow_after_one_hit(Charge::MEDIUM);
        let (high, p, step) = grow_after_one_hit(Charge { delay: 0.0, boost: 1.1 });
        assert!((low - 0.1).abs() < 0.005, "low: 100 ms, got {low}");
        assert!((medium - 0.05).abs() < 0.005, "medium: 50 ms, got {medium}");
        assert_eq!(high, 0.0, "high: straight away");
        assert!((p - step).abs() < 1e-6, "one step for one hit");
        // high's steps are 10% bigger, so it reaches full size in fewer impacts
        let mut w = world(1, 0);
        w.charge = Charge { delay: 0.0, boost: 1.1 };
        for _ in 0..200 {
            w.start_growth(0);
            let n = 1.0 / w.bodies[0].grow_step;
            assert!(n >= GROW_IMPACTS.0 as f32 / 1.1 - 1e-3 && n <= GROW_IMPACTS.1 as f32 / 1.1 + 1e-3);
        }
    }

    #[test]
    fn pop_spawns_grow_steadily_over_5_to_15_seconds() {
        for from in [0.1, 0.3, 0.6] {
            let mut w = world(1, 0);
            w.spawn = Spawn::Pop(from);
            let mut secs = Vec::new();
            for _ in 0..200 {
                w.start_growth(0);
                secs.push(1.0 / w.bodies[0].pop_rate);
            }
            assert!(secs.iter().all(|s| (POP_SECONDS.0 - 1e-3..=POP_SECONDS.1 + 1e-3).contains(s)));
            assert!(secs.iter().any(|&s| s < 6.0) && secs.iter().any(|&s| s > 14.0), "the whole range");
            let b = w.bodies[0];
            assert!((b.size - from).abs() < 1e-6, "pops in at {from}");
            let total = 1.0 / b.pop_rate;
            // halfway through, halfway grown, whatever it hits (and no damage)
            w.bodies[0].pos = w.centre();
            w.bodies[0].vel = Vec3A::X * TAP_SPEED;
            let mut t = 0.0;
            while t < total / 2.0 {
                w.step(0.05);
                t += 0.05;
            }
            let half = from + (1.0 - from) * (t / total);
            assert!((w.bodies[0].size - half).abs() < 0.01, "from {from}: {} vs {half}", w.bodies[0].size);
            assert_eq!(w.bodies[0].hits, 0);
            while t < total + 0.2 {
                w.step(0.05);
                t += 0.05;
            }
            assert!(!w.bodies[0].growing() && w.bodies[0].size == 1.0, "full grown by its timer");
        }
    }

    #[test]
    fn growing_solids_collide_at_their_size() {
        let mut w = world(2, 0);
        w.charge = Charge { delay: 0.0, boost: 1.0 };
        w.start_growth(0);
        let (a, b) = head_on(&mut w);
        // a speck hitting a full-size solid barely moves it, and bounces back
        assert!(b.abs() < 0.01 && a < -3.0, "{a} {b}");
        assert!((w.bodies[0].grow_progress - w.bodies[0].grow_step).abs() < 1e-6, "and grows a step");
        // respawned solids grow when growth is on
        let mut rng = fastrand::Rng::with_seed(2);
        w.growth = true;
        w.bodies[1].alive = false;
        w.respawn(1, &mut rng);
        assert!(w.bodies[1].growing() && w.bodies[1].size == SPAWN_SIZE);
        w.finish_growth();
        assert!(w.bodies.iter().all(|b| b.size == 1.0 && !b.growing()));
    }
    #[test]
    fn pick_finds_nearest_along_ray() {
        let mut w = world(2, 0);
        w.bodies[0].pos = Vec3A::new(0.0, 5.0, -2.0);
        w.bodies[1].pos = Vec3A::new(0.0, 5.0, 3.0);
        let ro = Vec3A::new(0.0, 5.0, 20.0);
        assert_eq!(w.pick(ro, -Vec3A::Z, 1.0), Some(1));
        assert_eq!(w.pick(ro, Vec3A::Z, 1.0), None);
        assert_eq!(w.pick(Vec3A::new(5.0, 5.0, 20.0), -Vec3A::Z, 1.0), None);
    }

    #[test]
    fn gradients_differ_and_hsv_is_right() {
        assert_eq!(hsv(0.0, 1.0, 1.0), 0xff0000);
        assert_eq!(hsv(1.0 / 3.0, 1.0, 1.0), 0x00ff00);
        assert_eq!(hsv(0.5, 0.0, 1.0), 0xffffff);
        let g: Vec<[u32; 3]> = (0..MAX_SOLIDS).map(|i| gradient_stops(i, 0.3)).collect();
        for i in 0..g.len() {
            for j in i + 1..g.len() {
                assert_ne!(g[i], g[j]);
            }
        }
    }
}
