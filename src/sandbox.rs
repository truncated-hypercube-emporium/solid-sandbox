//! The sandbox: physics world + platomon's path-traced scene, turned into a
//! grid of coloured glyphs with the bounding cube drawn as a wireframe.

use glam::{Quat, Vec3, Vec3A};
use platomon::ascii::{Ramp, tone_map};
use platomon::color::Gradient;
use platomon::render::{NO_HIT, Projection, RenderSettings, Renderer, TraceSettings};
use platomon::scene::sdf::SolidKind;
#[cfg(test)]
use platomon::scene::sdf::inverse_rotation;
use glam::Mat3A;
use platomon::scene::{AnimParams, Camera, CameraParams, GroundParams, Layout, Light, Scene, SlotLayout};
use std::f32::consts::TAU;

use crate::palette::{Palettes, Quantizer, bit_shift_24, saturate};
use crate::physics::{self, World};
use crate::settings::{ColourBy, Explosions, Glow, Settings, WireColour};
#[cfg(test)]
use crate::physics::TAP_SPEED;

/// Seconds per camera revolution round the cube.
const ORBIT_PERIOD: f32 = 80.0;
/// Camera elevation above the cube's centre, radians.
const ORBIT_ELEVATION: f32 = 0.42;
/// Camera distance from the container's centre: close in, so the container
/// overfills the view (the largest circumradius, the octahedron's, is ~9.1).
/// 15 in 1.1, now 10% closer.
const CAMERA_DISTANCE: f32 = 15.0 * 0.9;
const FOV_DEG: f32 = 50.0;
/// Dynamic camera: it backs off by up to this share of [`CAMERA_DISTANCE`]
/// as the solids' average speed (averaged over [`CAMERA_WINDOW`] real
/// seconds) rises to tap speed, never moving faster than the container's
/// edge turns at the camera's distance.
pub const CAMERA_PULL: f32 = 0.6;
pub const CAMERA_WINDOW: f32 = 2.0;
/// Glow on a glowing solid's own surface: how much of its shading survives
/// (the rest is flattened by the light), and how much darker dithered
/// cells in its shaded half get.
const GLOW_SHADING: f32 = 0.65;
pub const GLOW_SHADE_DARKEN: f32 = 0.5;
/// White glow keeps this share of a lit solid's chroma at full light.
pub const WHITE_GLOW_CHROMA: f32 = 0.5;

/// 4×4 ordered-dither threshold for cell (`x`, `y`), in (0, 1).
fn bayer(x: usize, y: usize) -> f32 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (M[y % 4][x % 4] as f32 + 0.5) / 16.0
}

/// `c` with its chroma scaled by `k`: each channel moved towards its luma.
fn chroma(c: u32, k: f32) -> u32 {
    let ch = |s: u32| ((c >> s) & 0xff) as f32;
    let l = 0.299 * ch(16) + 0.587 * ch(8) + 0.114 * ch(0);
    let out = |s: u32| ((l + (ch(s) - l) * k).round().clamp(0.0, 255.0) as u32) << s;
    out(16) | out(8) | out(0)
}
/// Magnification of the view: the box appears 30% larger in the window.
const ZOOM: f32 = 1.3;

/// Field of view after [`ZOOM`]: narrowing the view scales the image uniformly.
fn zoomed_fov_deg() -> f32 {
    2.0 * ((FOV_DEG.to_radians() * 0.5).tan() / ZOOM).atan().to_degrees()
}
const EXPOSURE: f32 = 1.0;
/// Fingertip slack when picking a solid: the tap target is a sphere this many
/// times the solid's collision radius (1.35 in 1.1.0, now 30% larger).
const PICK_SLACK: f32 = 1.35 * 1.3;
const EDGE_COLOR: u32 = 0x56_66_80;
/// Grey level of a solid's wireframe at rest and at tap speed.
const WIRE_DIM: f32 = 0.22;
const WIRE_BRIGHT: f32 = 1.0;
/// How far behind the visible surface a wire may lie and still be drawn,
/// relative to the solid's size (cells are coarse, so a front edge's depth
/// differs a little from the surface traced through the cell).
const WIRE_DEPTH_SLACK: f32 = 0.3;
/// Colour mode "Spin": the spin rate at the top of the gradient, rad/s.
const SPIN_REF: f32 = 6.0;
/// Where an exploded solid's (now meshless) shape is parked, far outside
/// everything the renderer marches.
const HIDDEN: Vec3A = Vec3A::new(0.0, -1000.0, 0.0);
/// No wire in this cell (see `Sandbox::under`).
const NO_WIRE: u32 = u32::MAX;
/// Explosion: speed the faces leave at (× violence), world units/s, on top
/// of the solid's own velocity; their spin, rad/s (× violence); and how fast
/// air drag slows them, per second.
const BLAST_SPEED: f32 = 7.0;
const BLAST_SPIN: (f32, f32) = (5.0, 14.0);
const BLAST_DRAG: f32 = 1.2;
/// Spin at the blast ([`World::spin_factor`](physics::World::spin_factor),
/// 0..=1) makes the faces fly up to this many times faster and spin harder,
/// and widens the spread of their speeds from ±[`BLAST_SPREAD`].0 to
/// ±[`BLAST_SPREAD`].1.
pub const SPIN_BLAST: f32 = 1.5;
pub const BLAST_SPREAD: (f32, f32) = (0.2, 0.75);
/// Typical seconds a face lasts (2.2 before 2.0, halved so exploded solids
/// come back sooner); each one's own life is this × a random factor in
/// [`BLAST_LIFE_SPREAD`], so they don't all go out together.
pub const BLAST_LIFE: f32 = 1.1;
pub const BLAST_LIFE_SPREAD: (f32, f32) = (0.25, 2.2);
/// Each face's fade curve: brightness ∝ (1 − age / life)^k, k random in
/// this range (below 1 holds its glow then drops fast, above 1 dims early).
pub const BLAST_FADE_CURVE: (f32, f32) = (0.25, 4.0);
/// Each face also flickers out by up to this share as it fades.
const BLAST_FLICKER: f32 = 0.5;
/// A face's wireframe fades this many times as fast as the face (50% faster).
pub const FRAGMENT_WIRE_FADE: f32 = 1.5;

/// Sparks: glyphs thrown straight out from a solid's edges. A solid whose
/// collision counter is past [`SPARK_FROM`] of 255 throws [`SPARK_RATE`] per
/// edge per second; an explosion throws [`BLAST_SPARKS`] at once (twice as
/// fast for a violence-2 one).
pub const SPARK_FROM: f32 = 0.9;
pub const SPARK_RATE: f32 = 3.0;
pub const BLAST_SPARKS: usize = 500;
/// Spark speed off the edge, world units/s, and life, simulated seconds.
pub const SPARK_SPEED: (f32, f32) = (11.0, 22.0);
pub const SPARK_LIFE: (f32, f32) = (0.18, 0.5);
const SPARK_DRAG: f32 = 2.5;
const MAX_SPARKS: usize = 8000;

/// Glow "Medium" and "High": at most this much further reach, as a share of
/// the radius (Medium by glow level, High by the collision counter).
pub const GLOW_GROW_MEDIUM: f32 = 0.4;
pub const GLOW_GROW_HIGH: f32 = 0.7;
/// With glow on, an explosion lights its spot and boosts every glow by this
/// factor for [`FLASH_TIME`] real seconds.
pub const FLASH_BOOST: f32 = 1.5;
pub const FLASH_TIME: f32 = 0.1;

/// One spark in flight.
#[derive(Debug, Clone)]
pub struct Spark {
    pub pos: Vec3A,
    pub vel: Vec3A,
    pub age: f32,
    pub life: f32,
    /// The solid it came from and its colour position then.
    pub solid: usize,
    pub t: f32,
}

impl Spark {
    /// Its brightness: down the lower half of the ramp, from 0.5 to 0, fast.
    pub fn lum(&self) -> f32 {
        let left = (1.0 - self.age / self.life).clamp(0.0, 1.0);
        0.5 * left * left
    }
}

/// A light source for glow: a glowing solid, or an explosion's flash.
#[derive(Debug, Clone, Copy)]
struct Emitter {
    pos: Vec3A,
    /// Radius of the sphere its light is cast from, and how far past that
    /// the light reaches.
    radius: f32,
    throw: f32,
    level: f32,
    /// The solid whose colours it shines in, and whether it is that solid
    /// (which it lights fully) or an explosion where it was.
    solid: usize,
    own: bool,
    t: f32,
}

/// One face of an exploded solid, flying out on its own.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// The solid it came from (for its colours).
    pub solid: usize,
    /// Centre of the face, world space.
    pub centre: Vec3A,
    /// Corners relative to the centre, as the face was when it blew off.
    pub corners: Vec<Vec3A>,
    /// Outward normal as it blew off.
    pub normal: Vec3A,
    pub vel: Vec3A,
    /// It spins about this axis through its centre.
    pub axis: Vec3A,
    pub spin: f32,
    pub angle: f32,
    /// Simulated seconds since the blast, and how long it lasts.
    pub age: f32,
    pub life: f32,
    /// Its fade curve exponent (see [`BLAST_FADE_CURVE`]) and flicker phase
    /// and rate.
    pub fade: f32,
    pub flicker: (f32, f32),
    /// The solid's colour position when it blew.
    pub t: f32,
}

impl Fragment {
    /// Its orientation change since the blast.
    fn turn(&self) -> Quat {
        Quat::from_axis_angle(Vec3::from(self.axis), self.angle)
    }

    /// How much of its light is left, 1 at the blast to 0 at the end of its
    /// life, along its own curve, with a flicker that grows as it dies.
    pub fn remaining(&self) -> f32 {
        self.remaining_at(self.age)
    }

    /// What [`remaining`](Self::remaining) is for its wireframe, which
    /// fades [`FRAGMENT_WIRE_FADE`] times as fast.
    pub fn wire_remaining(&self) -> f32 {
        self.remaining_at(self.age * FRAGMENT_WIRE_FADE)
    }

    fn remaining_at(&self, age: f32) -> f32 {
        let left = (1.0 - age / self.life).clamp(0.0, 1.0);
        let wobble = 1.0 - BLAST_FLICKER * (1.0 - left) * (0.5 + 0.5 * (self.flicker.0 + self.flicker.1 * self.age).sin());
        left.powf(self.fade) * wobble
    }
}

/// Each face of `poly` as (outward normal, corners in order round it), at
/// circumradius 1 in the solid's own frame.
fn face_rings(poly: &platomon::scene::sdf::Polyhedron) -> Vec<(Vec3A, Vec<Vec3A>)> {
    let verts = poly.vertices();
    poly.face_planes()
        .into_iter()
        .filter_map(|(n, d)| {
            let on: Vec<Vec3A> = verts.iter().copied().filter(|v| (v.dot(n) - d).abs() < 1e-3).collect();
            if on.len() < 3 {
                return None;
            }
            let m = on.iter().copied().sum::<Vec3A>() / on.len() as f32;
            let u = (on[0] - m).normalize();
            let w = n.cross(u);
            let mut ring: Vec<(f32, Vec3A)> = on.iter().map(|&v| ((v - m).dot(w).atan2((v - m).dot(u)), v)).collect();
            ring.sort_by(|a, b| a.0.total_cmp(&b.0));
            Some((n, ring.into_iter().map(|(_, v)| v).collect()))
        })
        .collect()
}

/// Whether grid point `p` is inside the convex polygon `poly` (either winding).
fn in_convex(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let (mut pos, mut neg) = (false, false);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        pos |= cross > 0.0;
        neg |= cross < 0.0;
    }
    !(pos && neg)
}

/// Glow: how far the light reaches past a solid's surface, as a share of
/// its radius.
pub const GLOW_THROW: f32 = 0.5;
/// Glow: a solid starts to glow above this share of tap speed (counting
/// spin as speed by energy), and is at full emission from [`GLOW_FULL`].
const GLOW_START: f32 = 0.35;
const GLOW_FULL: f32 = 0.9;
/// Glow: the halo in the air round a glowing solid, at most this bright.
const HALO: f32 = 0.6;

/// Glow level, 0..=1, for a solid whose kinetic energy per unit mass ×2 is
/// `specific_energy`, with taps at `tap_speed`.
pub fn glow_level(specific_energy: f32, tap_speed: f32) -> f32 {
    let s = specific_energy.max(0.0).sqrt() / tap_speed;
    let x = ((s - GLOW_START) / (GLOW_FULL - GLOW_START)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Light from a glowing solid at `dist` past its surface, `throw` the reach.
fn falloff(dist: f32, throw: f32) -> f32 {
    let x = (1.0 - dist.max(0.0) / throw).max(0.0);
    x * x
}

/// `c` (`0xRRGGBB`) with its brightness raised by received light `light`
/// (0..=1), keeping its hue: at 1 its brightest channel is at full.
fn brighten(c: u32, light: f32) -> u32 {
    let ch = |s: u32| ((c >> s) & 0xff) as f32;
    let top = ch(16).max(ch(8)).max(ch(0)).max(1.0);
    let k = 1.0 + (255.0 / top - 1.0) * light.clamp(0.0, 1.0);
    let out = |s: u32| ((ch(s) * k).round().min(255.0) as u32) << s;
    out(16) | out(8) | out(0)
}

/// `c` (`0xRRGGBB`) with every channel scaled by `k` (0..=1).
fn dim(c: u32, k: f32) -> u32 {
    let k = k.clamp(0.0, 1.0);
    let ch = |s: u32| ((((c >> s) & 0xff) as f32 * k).round() as u32) << s;
    ch(16) | ch(8) | ch(0)
}

/// How far a body's light reaches past that sphere.
fn glow_throw(b: &physics::Body) -> f32 {
    GLOW_THROW * b.radius
}

/// Monochrome wire colour for speed fraction `t` (0 at rest, 1 at tap speed).
pub fn wire_color(t: f32) -> u32 {
    let g = ((WIRE_DIM + (WIRE_BRIGHT - WIRE_DIM) * t.clamp(0.0, 1.0)) * 255.0).round() as u32;
    g * 0x01_01_01
}

/// Gravity in screen axes (x right, y up, z out of the glass, m/sÂ²) from a
/// raw accelerometer reading in Android's device axes and the display
/// rotation (`Surface.ROTATION_0..270` as 0..3). The accelerometer reports
/// the push that holds the phone up, so gravity is its negation.
pub fn screen_gravity(a: [f32; 3], rotation: i32) -> Vec3A {
    let (x, y) = match rotation & 3 {
        0 => (a[0], a[1]),
        1 => (-a[1], a[0]),
        2 => (-a[0], -a[1]),
        _ => (a[1], -a[0]),
    };
    -Vec3A::new(x, y, a[2])
}

/// A screen-axes vector in world space: what points down the screen points
/// down the view, and into the glass points away from the viewer.
pub fn screen_to_world(v: Vec3A, cam: &Camera) -> Vec3A {
    cam.right * v.x + cam.up * v.y - cam.forward * v.z
}

/// Line glyph for a screen-space direction.
fn line_glyph(dx: f32, dy: f32, char_aspect: f32) -> char {
    match (dy / char_aspect).atan2(dx).to_degrees().rem_euclid(180.0) {
        a if !(22.5..157.5).contains(&a) => '-',
        a if (67.5..112.5).contains(&a) => '|',
        a if a < 90.0 => '\\',
        _ => '/',
    }
}

/// Walk the segment `pa`â€“`pb` across the grid, calling `plot(cell, distance
/// from the camera, glyph)` for each cell it crosses.
fn trace_segment(
    proj: &Projection,
    (pa, pb): (Vec3A, Vec3A),
    (cols, rows): (usize, usize),
    char_aspect: f32,
    mut plot: impl FnMut(usize, f32, char),
) {
    let (Some(sa), Some(sb)) = (proj.project(pa), proj.project(pb)) else { return };
    let (dx, dy) = (sb.0 - sa.0, sb.1 - sa.1);
    let glyph = line_glyph(dx, dy, char_aspect);
    let steps = (dx.abs().max(dy.abs()) * 2.0).ceil().clamp(1.0, 4096.0) as usize;
    for s in 0..=steps {
        let u = s as f32 / steps as f32;
        let (x, y) = (sa.0 + dx * u, sa.1 + dy * u);
        if x < 0.0 || y < 0.0 || x >= cols as f32 || y >= rows as f32 {
            continue;
        }
        plot(y as usize * cols + x as usize, (pa + (pb - pa) * u - proj.cam.pos).length(), glyph);
    }
}

pub fn render_settings(char_aspect: f32) -> RenderSettings {
    RenderSettings {
        spp: 1,
        trace: TraceSettings { bounces: 1, max_steps: 64, max_dist: 60.0 },
        char_aspect,
        fit_aspect: None,
    }
}

pub struct Sandbox {
    pub world: World,
    pub scene: Scene,
    gradients: Vec<Gradient>,
    /// Each solid's corners (unit circumradius, body frame) and edges.
    wires: Vec<(Vec<Vec3A>, Vec<(usize, usize)>)>,
    /// Each solid's faces, for blowing it apart.
    faces: Vec<Vec<(Vec3A, Vec<Vec3A>)>>,
    /// Faces of exploded solids in flight.
    pub fragments: Vec<Fragment>,
    /// Exploded solids waiting to come back: simulated seconds left.
    respawn_in: Vec<Option<f32>>,
    /// Sparks in flight, and each solid's progress towards its next one.
    pub sparks: Vec<Spark>,
    spark_due: Vec<f32>,
    /// Explosion flashes lighting the glow: emitter and real seconds left.
    flashes: Vec<(Emitter, f32)>,
    /// What the solids' colours follow.
    pub colour_by: ColourBy,
    /// Saturation scale for the solids' colours (1: as designed).
    pub saturation: f32,
    /// Nearest-colour lookup for the palette in use, if any.
    quantizer: Option<Quantizer>,
    /// Cells showing a solid, its glow or its fragments: the ones the
    /// saturation and palette apply to.
    tinted: Vec<bool>,
    /// What colour the wireframes are.
    pub wire_colour: WireColour,
    /// Per cell under a wire: the colour the wire covered (`NO_WIRE` where
    /// there is none), whether that colour was a solid's, and how faded the
    /// wire is (1: not at all; a fragment's wire fades out).
    under: Vec<(u32, bool, f32)>,
    /// Whether to draw the solids' wireframes.
    pub wireframe: bool,
    /// Whether the camera backs off as the solids speed up, how far it has
    /// backed off, and the last [`CAMERA_WINDOW`] of average speeds (real
    /// seconds, speed).
    pub dynamic_camera: bool,
    pub camera_back: f32,
    speed_window: std::collections::VecDeque<(f32, f32)>,
    /// White glow instead of the glowing solid's colour.
    pub white_glow: bool,
    /// Share of the accelerometer's pull applied as gravity (0: off).
    pub gravity: f32,
    /// How fast solids glow and light their neighbours.
    pub glow: Glow,
    /// Simulated seconds per real second.
    pub speed: f32,
    /// Per-solid glow from the last frame, 0..=1.
    glow_levels: Vec<f32>,
    /// Latest raw accelerometer reading (m/sÂ², device axes) and display rotation.
    accel: Option<([f32; 3], i32)>,
    wire_depth: Vec<f32>,
    rng: fastrand::Rng,
    orbit: f32,
    renderer: Renderer,
    ramp: Ramp,
    proj: Option<Projection>,
    pub chars: Vec<char>,
    pub colors: Vec<u32>,
}

impl Sandbox {
    pub fn new(seed: u64) -> Self {
        Self::with_settings(seed, &Settings::default())
    }

    pub fn with_settings(seed: u64, settings: &Settings) -> Self {
        let mut rng = fastrand::Rng::with_seed(seed);
        let n = settings.solids().unwrap_or_else(|| rng.usize(physics::MIN_SOLIDS..=physics::MAX_SOLIDS));
        let slots: Vec<SlotLayout> =
            (0..n).map(|_| SlotLayout { solid: SolidKind::ALL[rng.usize(..SolidKind::ALL.len())], ring: None }).collect();
        let mut scene = Scene::with_layout(&slots, &AnimParams::default(), &CameraParams::default(), Layout::Sphere)
            .with_ground(&GroundParams { lum: 0.0, ..Default::default() });
        let inradii: Vec<f32> = scene.solids.iter().map(|s| s.poly.inradius()).collect();
        let shape = settings.container().unwrap_or_else(|| physics::CONTAINERS[rng.usize(..physics::CONTAINERS.len())]);
        let mut world = World::new(&inradii, shape, &mut rng);
        world.tap_speed = settings.tap_speed();
        world.set_hulls(&scene.solids.iter().map(|s| s.poly).collect::<Vec<_>>());
        world.exact = settings.exact();
        world.elasticity = settings.elasticity();
        world.damping = settings.damping();
        world.drag = settings.drag();
        if settings.lightest() < 1.0 {
            world.vary_mass(settings.lightest(), &mut rng);
        }
        let wires = scene
            .solids
            .iter()
            .map(|s| (s.poly.vertices(), s.poly.edges().into_iter().map(|(a, b, _, _)| (a, b)).collect()))
            .collect();
        let faces = scene.solids.iter().map(|s| face_rings(&s.poly)).collect();
        world.reseed(seed ^ 0xb1a5_7ed);
        // every solid starts small and grows, the way the Spawn setting says
        world.growth = true;
        world.spawn = settings.spawn();
        world.charge = settings.charge();
        for i in 0..n {
            world.start_growth(i);
        }

        let c = world.centre();
        scene.light = Light { center: c + Vec3A::new(-7.0, 11.0, 4.0).normalize() * 30.0, radius: 5.0, emission: 90.0 };
        scene.sky = 0.14;
        scene.background = 0.0;
        for s in &mut scene.solids {
            s.scale = physics::RADIUS;
            s.inv_scale = 1.0 / physics::RADIUS;
            s.albedo = 0.8;
        }
        let hue0 = rng.f32();
        let gradients = (0..n).map(|i| Gradient::from_stops(&physics::gradient_stops(i, hue0))).collect();
        let orbit = TAU * rng.f32();
        let mut sb = Self {
            world,
            scene,
            gradients,
            wires,
            faces,
            fragments: Vec::new(),
            respawn_in: vec![None; n],
            sparks: Vec::new(),
            spark_due: vec![0.0; n],
            flashes: Vec::new(),
            colour_by: settings.colour_by(),
            saturation: settings.saturation(),
            quantizer: None,
            tinted: Vec::new(),
            wire_colour: settings.wire_colour(),
            under: Vec::new(),
            wireframe: settings.wireframe(),
            dynamic_camera: settings.dynamic_camera(),
            camera_back: 0.0,
            speed_window: std::collections::VecDeque::new(),
            white_glow: settings.white_glow(),
            gravity: settings.gravity(),
            glow: settings.glow(),
            speed: 1.0,
            glow_levels: Vec::new(),
            accel: None,
            wire_depth: Vec::new(),
            rng,
            orbit,
            renderer: Renderer::new(),
            ramp: Ramp::default(),
            proj: None,
            chars: Vec::new(),
            colors: Vec::new(),
        };
        sb.set_live(settings, None);
        sb.sync();
        sb
    }

    /// Take the Performance page's simulation speed and sub-step rate (the
    /// frame rate is the render loop's), without restarting the scene.
    pub fn set_performance(&mut self, settings: &Settings) {
        self.speed = settings.sim_speed();
        self.world.step_hz = settings.step_hz();
    }

    /// Take every setting that applies without a restart: colour mode,
    /// saturation, the palette (out of `palettes`), explosions, and the
    /// Performance page.
    pub fn set_live(&mut self, settings: &Settings, palettes: Option<&Palettes>) {
        self.set_performance(settings);
        self.colour_by = settings.colour_by();
        self.saturation = settings.saturation();
        self.world.explosions = settings.explosions() != Explosions::Off;
        self.world.strong = settings.explosions() == Explosions::Strong;
        self.glow = settings.glow();
        self.dynamic_camera = settings.dynamic_camera();
        self.white_glow = settings.white_glow();
        self.wire_colour = settings.wire_colour();
        self.world.charge = settings.charge();
        self.set_palette(settings.palette().and_then(|i| palettes?.get(i)));
    }

    /// Draw the solids in this palette's colours, or `None` for their own.
    pub fn set_palette(&mut self, palette: Option<&[u32]>) {
        self.quantizer = palette.map(Quantizer::new);
    }

    /// Whether a palette is in use.
    pub fn has_palette(&self) -> bool {
        self.quantizer.is_some()
    }

    /// Copy body positions and orientations into the scene; place the camera.
    fn sync(&mut self) {
        for (i, b) in self.world.bodies.iter().enumerate() {
            let s = &mut self.scene.solids[i];
            s.center = if b.alive { b.pos } else { HIDDEN };
            s.scale = physics::RADIUS * b.size;
            s.inv_scale = 1.0 / s.scale;
            self.scene.angles[i] = b.angles;
        }
        // refresh_transforms rebuilds rotations from (pitch, yaw), which
        // can't hold an arbitrary rigid-body orientation: set them from the
        // body's quaternion afterwards (the same matrix in sphere mode)
        self.scene.refresh_transforms();
        for (i, b) in self.world.bodies.iter().enumerate() {
            self.scene.solids[i].inv_rot = Mat3A::from_quat(b.rot).transpose();
        }
        let c = self.world.centre();
        let (se, ce) = ORBIT_ELEVATION.sin_cos();
        let (so, co) = self.orbit.sin_cos();
        let pos = c + Vec3A::new(co * ce, se, so * ce) * (CAMERA_DISTANCE + self.camera_back);
        let fov = zoomed_fov_deg();
        self.scene.camera = Camera::look_at(pos, c, fov, fov);
    }

    /// Feed a raw accelerometer reading (m/sÂ², device axes) and the display
    /// rotation (0..3); `None` when there is no sensor.
    pub fn set_accel(&mut self, reading: Option<([f32; 3], i32)>) {
        self.accel = reading;
    }

    /// World gravity for the current reading and camera: the raw vector's
    /// direction in world space, 1 g â†¦ [`physics::GRAVITY`], capped at 3 g so
    /// a hard shake doesn't fling everything through the walls.
    pub fn world_gravity(&self) -> Vec3A {
        match self.accel {
            Some((a, rot)) if self.gravity > 0.0 => {
                let g = screen_to_world(screen_gravity(a, rot), &self.scene.camera) * (physics::GRAVITY / physics::STANDARD_G);
                g.clamp_length_max(3.0 * physics::GRAVITY) * self.gravity
            }
            _ => Vec3A::ZERO,
        }
    }

    /// Advance by `dt` real seconds: the solids by `dt` × [`speed`](Self::speed)
    /// of simulated time, the camera's orbit in real time.
    pub fn step(&mut self, dt: f32) {
        // a stall (the app paused, a slow frame) counts as at most 0.1 s
        let dt = dt.clamp(0.0, 0.1);
        let sim = dt * self.speed;
        self.world.gravity = self.world_gravity();
        self.world.step(sim);
        for (i, violence) in std::mem::take(&mut self.world.blasts) {
            self.explode(i, violence);
        }
        self.fly_fragments(sim);
        self.throw_sparks(sim);
        self.fly_sparks(sim);
        for (_, left) in &mut self.flashes {
            *left -= dt;
        }
        self.flashes.retain(|f| f.1 > 0.0);
        self.orbit = (self.orbit + TAU / ORBIT_PERIOD * dt).rem_euclid(TAU);
        self.move_camera(dt);
        self.sync();
    }

    /// Blow solid `i` apart: each face flies straight out from the centre
    /// along its normal, like pins in a pincushion, spinning about a random
    /// axis through its own centre. `violence` 2 doubles speed and spin.
    pub fn explode(&mut self, i: usize, violence: f32) {
        let b = self.world.bodies[i];
        let t = self.colour_t(i);
        let scale = self.scene.solids[i].scale;
        let whirl = self.world.spin_factor(i);
        let (boost, spread) = (1.0 + SPIN_BLAST * whirl, BLAST_SPREAD.0 + (BLAST_SPREAD.1 - BLAST_SPREAD.0) * whirl);
        for (n, ring) in &self.faces[i] {
            let corners: Vec<Vec3A> = ring.iter().map(|&v| b.pos + b.rot * (v * scale)).collect();
            let centre = corners.iter().copied().sum::<Vec3A>() / corners.len() as f32;
            let normal = b.rot * *n;
            let out = BLAST_SPEED * violence * boost * (1.0 + spread * (2.0 * self.rng.f32() - 1.0));
            let spin = (BLAST_SPIN.0 + (BLAST_SPIN.1 - BLAST_SPIN.0) * self.rng.f32()) * violence * boost;
            let life = BLAST_LIFE * (BLAST_LIFE_SPREAD.0 + (BLAST_LIFE_SPREAD.1 - BLAST_LIFE_SPREAD.0) * self.rng.f32());
            let fade = BLAST_FADE_CURVE.0 + (BLAST_FADE_CURVE.1 - BLAST_FADE_CURVE.0) * self.rng.f32();
            let flicker = (TAU * self.rng.f32(), 8.0 + 22.0 * self.rng.f32());
            self.fragments.push(Fragment {
                solid: i,
                centre,
                corners: corners.iter().map(|&c| c - centre).collect(),
                normal,
                vel: b.vel + normal * out,
                axis: physics::random_dir(&mut self.rng),
                spin: if self.rng.bool() { spin } else { -spin },
                angle: 0.0,
                age: 0.0,
                life,
                fade,
                flicker,
                t,
            });
        }
        // it comes back once its last face has gone out
        self.respawn_in[i] = Some(0.0);
        for _ in 0..BLAST_SPARKS {
            self.spark(i, violence);
        }
        if self.glow != Glow::Off {
            let e = Emitter { pos: b.pos, radius: b.radius, throw: glow_throw(&b), level: FLASH_BOOST, solid: i, own: false, t };
            self.flashes.push((e, FLASH_TIME));
        }
    }

    /// Throw one spark from a random point on one of solid `i`'s edges,
    /// straight out from its centre; `violence` 2 throws it twice as fast.
    fn spark(&mut self, i: usize, violence: f32) {
        let (verts, edges) = &self.wires[i];
        if edges.is_empty() || self.sparks.len() >= MAX_SPARKS {
            return;
        }
        let b = self.world.bodies[i];
        let (a, c) = edges[self.rng.usize(..edges.len())];
        let u = self.rng.f32();
        let local = verts[a] + (verts[c] - verts[a]) * u;
        let pos = b.pos + b.rot * (local * physics::RADIUS * b.size);
        let out = (pos - b.pos).normalize_or(Vec3A::Y);
        let speed = (SPARK_SPEED.0 + (SPARK_SPEED.1 - SPARK_SPEED.0) * self.rng.f32()) * violence;
        let life = SPARK_LIFE.0 + (SPARK_LIFE.1 - SPARK_LIFE.0) * self.rng.f32();
        let t = self.colour_t(i);
        self.sparks.push(Spark { pos, vel: b.vel + out * speed, age: 0.0, life, solid: i, t });
    }

    /// Solids nearly worn out (counter past [`SPARK_FROM`]) throw
    /// [`SPARK_RATE`] sparks per edge per simulated second.
    fn throw_sparks(&mut self, dt: f32) {
        for i in 0..self.world.bodies.len() {
            let b = &self.world.bodies[i];
            if !b.alive || b.growing() || (b.hits as f32) <= SPARK_FROM * 255.0 {
                self.spark_due[i] = 0.0;
                continue;
            }
            self.spark_due[i] += self.wires[i].1.len() as f32 * SPARK_RATE * dt;
            while self.spark_due[i] >= 1.0 {
                self.spark_due[i] -= 1.0;
                self.spark(i, 1.0);
            }
        }
    }

    fn fly_sparks(&mut self, dt: f32) {
        let drag = (-SPARK_DRAG * dt).exp();
        for s in &mut self.sparks {
            s.age += dt;
            s.vel *= drag;
            s.pos += s.vel * dt;
        }
        self.sparks.retain(|s| s.age < s.life);
    }

    /// Dynamic camera: ease the camera's distance towards where the last
    /// [`CAMERA_WINDOW`] real seconds of average speed put it, no faster than
    /// the orbit sweeps the camera sideways.
    fn move_camera(&mut self, dt: f32) {
        let live: Vec<f32> = self.world.bodies.iter().filter(|b| b.alive).map(|b| b.speed()).collect();
        let avg = if live.is_empty() { 0.0 } else { live.iter().sum::<f32>() / live.len() as f32 };
        self.speed_window.push_back((dt, avg));
        while self.speed_window.len() > 1 && self.speed_window.iter().map(|s| s.0).sum::<f32>() - self.speed_window[0].0 >= CAMERA_WINDOW {
            self.speed_window.pop_front();
        }
        let span: f32 = self.speed_window.iter().map(|s| s.0).sum();
        let smooth = if span > 0.0 { self.speed_window.iter().map(|s| s.0 * s.1).sum::<f32>() / span } else { avg };
        let target = if self.dynamic_camera { CAMERA_DISTANCE * CAMERA_PULL * (smooth / self.world.tap_speed).min(1.0) } else { 0.0 };
        let max_step = TAU / ORBIT_PERIOD * (CAMERA_DISTANCE + self.camera_back) * dt;
        self.camera_back += (target - self.camera_back).clamp(-max_step, max_step);
    }

    /// The scoreboard: impacts counted, solids destroyed, and the live
    /// solids' average speed (world units per second).
    pub fn stats(&self) -> [String; 3] {
        let live: Vec<f32> = self.world.bodies.iter().filter(|b| b.alive).map(|b| b.speed()).collect();
        let avg = if live.is_empty() { 0.0 } else { live.iter().sum::<f32>() / live.len() as f32 };
        [format!("COLLISIONS {}", self.world.collisions), format!("DESTROYED {}", self.world.destroyed), format!("AVG SPEED {avg:.2}")]
    }

    /// Move the fragments on by `dt` simulated seconds, drop the spent ones,
    /// and bring back solids whose fragments are gone.
    fn fly_fragments(&mut self, dt: f32) {
        let (g, drag) = (self.world.gravity, (-BLAST_DRAG * dt).exp());
        for f in &mut self.fragments {
            f.age += dt;
            f.vel = f.vel * drag + g * dt;
            f.centre += f.vel * dt;
            f.angle += f.spin * dt;
        }
        self.fragments.retain(|f| f.age < f.life);
        for i in 0..self.respawn_in.len() {
            if self.respawn_in[i].is_some() && !self.fragments.iter().any(|f| f.solid == i) {
                self.respawn_in[i] = None;
                self.world.respawn(i, &mut self.rng);
            }
        }
    }

    /// Body `i`'s position on its colour gradient for the colour mode: speed,
    /// spin or collision counter, 0..=1.
    pub fn colour_t(&self, i: usize) -> f32 {
        let b = &self.world.bodies[i];
        match self.colour_by {
            ColourBy::Velocity => self.speed_t(i),
            ColourBy::Spin => (b.spin_rate(self.world.is_exact()) / SPIN_REF).clamp(0.0, 1.0),
            ColourBy::Collisions => b.hits as f32 / 255.0,
        }
    }

    /// A tap at continuous grid position (`x` columns, `y` rows): kick the
    /// solid under it. Returns the solid's index.
    pub fn tap(&mut self, x: f32, y: f32) -> Option<usize> {
        let p = self.proj?;
        let i = self.world.pick(p.cam.pos, p.ray(x, y), PICK_SLACK)?;
        self.world.kick(i, &mut self.rng);
        Some(i)
    }

    /// Gradient position for body `i`: 0 at rest, 1 at tap speed or faster.
    pub fn speed_t(&self, i: usize) -> f32 {
        (self.world.bodies[i].speed() / self.world.tap_speed).clamp(0.0, 1.0)
    }

    /// Render a `cols Ã— rows` frame into `chars` / `colors` (`0xRRGGBB`).
    pub fn render(&mut self, cols: usize, rows: usize, rs: &RenderSettings) {
        self.renderer.render(&self.scene, cols, rows, rs);
        let Some(view) = self.renderer.view() else { return };
        self.proj = Some(view.proj);
        let n = self.world.bodies.len();
        // the colour mode picks the gradient position; wires always follow speed
        let t: Vec<f32> = (0..n).map(|i| self.colour_t(i)).collect();
        let speeds: Vec<f32> = (0..n).map(|i| self.speed_t(i)).collect();
        self.chars.clear();
        self.colors.clear();
        self.tinted.clear();
        self.tinted.extend(view.ids.iter().map(|&id| (id as usize) < n));
        self.under.clear();
        self.under.resize(cols * rows, (NO_WIRE, false, 1.0));
        let lums: Vec<f32> = view.radiance.iter().map(|&rad| tone_map(rad, EXPOSURE)).collect();
        for (k, &lum) in lums.iter().enumerate() {
            let id = view.ids[k] as usize;
            if id < n {
                let g = self.ramp.glyph(lum);
                self.chars.push(if g == ' ' { '.' } else { g });
                self.colors.push(self.gradients[id].sample(t[id], 0.4 + 0.6 * lum));
            } else {
                self.chars.push(self.ramp.glyph(lum));
                self.colors.push(0xff_ff_ff);
            }
        }
        let (depth, ids, proj) = (view.depth.to_vec(), view.ids.to_vec(), view.proj);
        let emitters = self.emitters(&t);
        if !emitters.is_empty() {
            self.draw_glow(cols, &proj, &depth, &ids, &lums, &emitters);
        }
        self.draw_box(cols, rows, &proj, &depth, &ids, rs.char_aspect);
        if self.wireframe {
            self.draw_wires(cols, rows, &proj, &depth, &speeds, rs.char_aspect);
        }
        if !self.fragments.is_empty() {
            self.draw_fragments(cols, rows, &proj, &depth, rs.char_aspect);
        }
        if !self.sparks.is_empty() {
            self.draw_sparks(cols, rows, &proj, &depth);
        }
        // saturation, then the palette, on everything coloured by a solid
        let (sat, q) = (self.saturation, self.quantizer.as_ref());
        let finish = |c: u32| {
            let s = if sat != 1.0 { saturate(c, sat) } else { c };
            q.map_or(s, |q| q.map(s))
        };
        if sat != 1.0 || q.is_some() {
            for (c, _) in self.colors.iter_mut().zip(&self.tinted).filter(|(_, on)| **on) {
                *c = finish(*c);
            }
        }
        // wires coloured from what they cover, now that it has its final colour
        if self.wire_colour != WireColour::Velocity {
            for (k, &(below, solid, fade)) in self.under.iter().enumerate().filter(|(_, u)| u.0 != NO_WIRE) {
                let below = if solid { finish(below) } else { below };
                let c = match self.wire_colour {
                    WireColour::Velocity => self.colors[k],
                    WireColour::White => 0xff_ff_ff,
                    WireColour::Inverted => below ^ 0xff_ff_ff,
                    WireColour::BitShift => q.map_or_else(|| bit_shift_24(below), |q| q.bit_shift(below)),
                };
                self.colors[k] = if fade < 1.0 && self.wire_colour != WireColour::Velocity { dim(c, fade) } else { c };
            }
        }
    }

    /// The flying faces, filled and shaded by the scene's light, each fading
    /// out over its own life and curve after a bright flash; depth-tested
    /// against the scene and each other.
    fn draw_fragments(&mut self, cols: usize, rows: usize, proj: &Projection, depth: &[f32], char_aspect: f32) {
        let eye = proj.cam.pos;
        let light = self.scene.light.center;
        let mut zbuf = vec![f32::INFINITY; cols * rows];
        for f in &self.fragments {
            let q = f.turn();
            let pts: Option<Vec<(f32, f32)>> = f.corners.iter().map(|&c| proj.project(f.centre + q * c)).collect();
            let (Some(pts), Some(mid)) = (pts, proj.project(f.centre)) else { continue };
            let d = (f.centre - eye).length();
            let n = q * f.normal;
            let flash = (1.0 - f.age / 0.35).max(0.0);
            let lit = 0.3 + 0.7 * n.dot((light - f.centre).normalize()).abs();
            let lum = (lit * f.remaining() + flash).min(1.0);
            if lum <= 0.02 {
                continue;
            }
            let g = self.ramp.glyph(lum);
            let glyph = if g == ' ' { '.' } else { g };
            let color = brighten(self.gradients[f.solid].sample(f.t, 0.4 + 0.6 * lum), flash);
            let (x0, x1) = pts.iter().fold((mid.0, mid.0), |(a, b), p| (a.min(p.0), b.max(p.0)));
            let (y0, y1) = pts.iter().fold((mid.1, mid.1), |(a, b), p| (a.min(p.1), b.max(p.1)));
            let (cx0, cx1) = (x0.floor().max(0.0) as usize, (x1.ceil().max(0.0) as usize).min(cols));
            let (cy0, cy1) = (y0.floor().max(0.0) as usize, (y1.ceil().max(0.0) as usize).min(rows));
            let mut plot = |k: usize| {
                if d < depth[k] && d < zbuf[k] {
                    zbuf[k] = d;
                    self.chars[k] = glyph;
                    self.colors[k] = color;
                    self.tinted[k] = true;
                    self.under[k].0 = NO_WIRE; // a fragment covers any wire here
                }
            };
            for y in cy0..cy1 {
                for x in cx0..cx1 {
                    if in_convex((x as f32 + 0.5, y as f32 + 0.5), &pts) {
                        plot(y * cols + x);
                    }
                }
            }
            // a face smaller than a cell still shows as one
            if mid.0 >= 0.0 && mid.1 >= 0.0 && (mid.0 as usize) < cols && (mid.1 as usize) < rows {
                plot(mid.1 as usize * cols + mid.0 as usize);
            }
            // its edges in the wireframe's colours, fading out faster
            let fade = f.wire_remaining();
            if !self.wireframe || fade <= 0.02 {
                continue;
            }
            let world: Vec<Vec3A> = f.corners.iter().map(|&c| f.centre + q * c).collect();
            let speed = (f.vel.length() / self.world.tap_speed).clamp(0.0, 1.0);
            let grey = dim(wire_color(speed), fade);
            let slack = 0.05 * d;
            let (chars, colors, tinted, under) = (&mut self.chars, &mut self.colors, &mut self.tinted, &mut self.under);
            for a in 0..world.len() {
                let seg = (world[a], world[(a + 1) % world.len()]);
                trace_segment(proj, seg, (cols, rows), char_aspect, |k, dk, glyph| {
                    if dk < depth[k] && dk <= zbuf[k] + slack {
                        zbuf[k] = zbuf[k].min(dk);
                        if under[k].0 == NO_WIRE {
                            under[k] = (colors[k], tinted[k], fade);
                        }
                        chars[k] = glyph;
                        colors[k] = grey;
                        tinted[k] = false;
                    }
                });
            }
        }
    }

    /// The sparks, each one glyph in its solid's colour down the lower half
    /// of the ramp, in front of whatever is behind it.
    fn draw_sparks(&mut self, cols: usize, rows: usize, proj: &Projection, depth: &[f32]) {
        let eye = proj.cam.pos;
        for s in &self.sparks {
            let Some((x, y)) = proj.project(s.pos) else { continue };
            if x < 0.0 || y < 0.0 || x >= cols as f32 || y >= rows as f32 {
                continue;
            }
            let k = y as usize * cols + x as usize;
            let lum = s.lum();
            let g = self.ramp.glyph(lum);
            if g == ' ' || (s.pos - eye).length() >= depth[k] {
                continue;
            }
            self.chars[k] = g;
            self.colors[k] = self.gradients[s.solid].sample(s.t, 0.4 + 0.6 * lum);
            self.tinted[k] = true;
            self.under[k].0 = NO_WIRE;
        }
    }

    /// The glow's light sources: each glowing solid (how it glows and how far
    /// its light reaches by the [`Glow`] setting), plus explosion flashes,
    /// all boosted by [`FLASH_BOOST`] while a flash lasts. Also keeps each
    /// solid's level in `glow_levels`.
    fn emitters(&mut self, t: &[f32]) -> Vec<Emitter> {
        let n = self.world.bodies.len();
        let boost = if self.flashes.is_empty() { 1.0 } else { FLASH_BOOST };
        let exact = self.world.is_exact();
        self.glow_levels = (0..n)
            .map(|i| {
                let b = &self.world.bodies[i];
                let energy = match self.glow {
                    Glow::Off => return 0.0,
                    Glow::Low => self.world.specific_energy(i),
                    // spin counts as the speed of the solid's rim
                    Glow::Medium | Glow::High if b.alive => b.vel.length_squared() + (b.spin_rate(exact) * b.radius).powi(2),
                    _ => 0.0,
                };
                glow_level(energy, self.world.tap_speed) * boost
            })
            .collect();
        let mut out: Vec<Emitter> = (0..n)
            .filter(|&i| self.glow_levels[i] > 0.0)
            .map(|i| {
                let b = &self.world.bodies[i];
                let level = self.glow_levels[i];
                let grow = match self.glow {
                    Glow::Medium => GLOW_GROW_MEDIUM * level.min(1.0),
                    Glow::High => GLOW_GROW_HIGH * b.hits as f32 / 255.0,
                    _ => 0.0,
                };
                Emitter { pos: b.pos, radius: b.radius, throw: glow_throw(b) + grow * b.radius, level, solid: i, own: true, t: t[i] }
            })
            .collect();
        if self.glow != Glow::Off {
            out.extend(self.flashes.iter().map(|(e, left)| Emitter { level: e.level * (left / FLASH_TIME).clamp(0.0, 1.0), ..*e }));
        }
        out
    }

    /// Glowing solids as light sources with a short throw. Light is cast
    /// from each glowing solid's light sphere (its collision sphere) out to
    /// [`GLOW_THROW`] × its radius: its own surface brightens towards full
    /// (100% emission at full glow), any part of another solid inside that
    /// reach has its luminosity raised, the more the nearer, and a faint
    /// halo fills the air within the same reach.
    fn draw_glow(&mut self, cols: usize, proj: &Projection, depth: &[f32], ids: &[u8], lums: &[f32], emitters: &[Emitter]) {
        let n = self.world.bodies.len();
        let eye = proj.cam.pos;
        // High: the light adds to whatever it passes in front of
        let additive = self.glow == Glow::High;
        for k in 0..ids.len() {
            let rd = proj.ray((k % cols) as f32 + 0.5, (k / cols) as f32 + 0.5);
            let id = ids[k] as usize;
            // nearest the ray through this cell comes to emitter `e` before
            // it reaches the surface it hit: the halo's measure
            let reach = |e: &Emitter| {
                let along = (e.pos - eye).dot(rd).clamp(0.0, depth[k]);
                falloff((eye + rd * along - e.pos).length() - e.radius, e.throw)
            };
            if id < n {
                // a solid's surface: its luminosity rises wherever an
                // emitter's light reaches, the same reach as the halo (at
                // full strength, so a neighbour touching it is lit fully)
                let own: f32 = emitters.iter().filter(|e| e.own && e.solid == id).map(|e| e.level).sum();
                let light: f32 = emitters.iter().map(|e| e.level * if e.own && e.solid == id { 1.0 } else { reach(e) }).sum();
                if light <= 0.0 {
                    continue;
                }
                let light = light.min(1.0);
                let base = lums[k];
                let (mut lum, mut glow) = (if additive { (base + light).min(1.0) } else { base + (1.0 - base) * light }, light);
                if own > 0.0 {
                    // a glowing solid keeps its shading: the light lifts its
                    // lit side more than its shaded side, and the shaded half
                    // is dithered half as bright again
                    let keep = 1.0 - GLOW_SHADING + GLOW_SHADING * base;
                    lum = (base + (1.0 - base) * light * keep).min(1.0);
                    glow = light * keep;
                    let shade = ((0.5 - base) / 0.5).clamp(0.0, 1.0);
                    if shade > bayer(k % cols, k / cols) {
                        lum *= 1.0 - GLOW_SHADE_DARKEN;
                        glow *= 1.0 - GLOW_SHADE_DARKEN;
                    }
                }
                let g = self.ramp.glyph(lum);
                self.chars[k] = if g == ' ' { '.' } else { g };
                let c = self.gradients[id].sample(self.colour_t(id), 0.4 + 0.6 * lum);
                // white glow: the light keeps only half the chroma of what it lights
                let c = if self.white_glow { chroma(c, 1.0 - (1.0 - WHITE_GLOW_CHROMA) * light) } else { c };
                self.colors[k] = brighten(c, glow);
            } else {
                // empty space: a halo where the ray passes within reach of an emitter
                let (mut halo, mut sum, mut color) = (0.0f32, 0.0f32, 0u32);
                for e in emitters {
                    let h = HALO * e.level * reach(e);
                    sum += h;
                    if h > halo {
                        let c = if self.white_glow { 0xff_ff_ff } else { self.gradients[e.solid].sample(e.t, 0.5 + 0.5 * h.min(1.0)) };
                        (halo, color) = (h, c);
                    }
                }
                let lum = if additive { (lums[k] + sum).min(1.0) } else { halo };
                let g = self.ramp.glyph(lum);
                if lum > lums[k] && g != ' ' {
                    self.chars[k] = g;
                    self.colors[k] = color;
                    self.tinted[k] = true;
                }
            }
        }
    }
    /// The container's edges. They never cover a solid's cells, even where
    /// the edge is nearer the camera; elsewhere they are depth-tested as usual.
    fn draw_box(&mut self, cols: usize, rows: usize, proj: &Projection, depth: &[f32], ids: &[u8], char_aspect: f32) {
        let n = self.world.bodies.len();
        let (chars, colors) = (&mut self.chars, &mut self.colors);
        for &(a, b) in &self.world.edges {
            let seg = (self.world.verts[a], self.world.verts[b]);
            trace_segment(proj, seg, (cols, rows), char_aspect, |k, d, glyph| {
                let on_solid = (ids[k] as usize) < n && ids[k] != NO_HIT;
                if !on_solid && (ids[k] == NO_HIT || depth[k] > d) {
                    chars[k] = glyph;
                    colors[k] = EDGE_COLOR;
                }
            });
        }
    }

    /// Each solid's edges in grey, brighter the faster it moves. Edges on the
    /// far side of a solid, or behind another one, are hidden.
    fn draw_wires(&mut self, cols: usize, rows: usize, proj: &Projection, depth: &[f32], t: &[f32], char_aspect: f32) {
        self.wire_depth.clear();
        self.wire_depth.resize(cols * rows, f32::INFINITY);
        let (chars, colors, zbuf) = (&mut self.chars, &mut self.colors, &mut self.wire_depth);
        let (tinted, under) = (&mut self.tinted, &mut self.under);
        for (i, inst) in self.scene.solids.iter().enumerate() {
            if !self.world.bodies[i].alive {
                continue;
            }
            let rot = inst.inv_rot.transpose();
            let (verts, edges) = &self.wires[i];
            let world: Vec<Vec3A> = verts.iter().map(|&v| inst.center + rot * (v * inst.scale)).collect();
            let (color, slack) = (wire_color(t[i]), WIRE_DEPTH_SLACK * inst.scale);
            for &(a, b) in edges {
                trace_segment(proj, (world[a], world[b]), (cols, rows), char_aspect, |k, d, glyph| {
                    if d <= depth[k] + slack && d < zbuf[k] {
                        zbuf[k] = d;
                        // keep what the first wire over a cell covered, for
                        // the colour modes that are worked out from it
                        if under[k].0 == NO_WIRE {
                            under[k] = (colors[k], tinted[k], 1.0);
                        }
                        chars[k] = glyph;
                        colors[k] = color;
                        // a wire is not a solid's colour: no saturation or palette
                        tinted[k] = false;
                    }
                });
            }
        }
    }
}

/// Seconds per rendered cell on this device, from a few frames large enough
/// that thread start-up doesn't dominate the timing.
pub fn seconds_per_cell(sb: &mut Sandbox, rs: &RenderSettings) -> f32 {
    let (w, h) = (96, 36);
    sb.render(w, h, rs);
    let t0 = std::time::Instant::now();
    for _ in 0..3 {
        sb.render(w, h, rs);
    }
    t0.elapsed().as_secs_f32() / (3 * w * h) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sandbox with its solids grown to full size, as most tests want them.
    fn full(mut sb: Sandbox) -> Sandbox {
        sb.world.finish_growth();
        sb.sync();
        sb
    }

    #[test]
    fn launches_with_random_count_inside_the_cube() {
        let mut counts = std::collections::BTreeSet::new();
        for seed in 0..40 {
            let sb = full(Sandbox::new(seed));
            let n = sb.world.bodies.len();
            assert!((physics::MIN_SOLIDS..=physics::MAX_SOLIDS).contains(&n));
            assert_eq!(sb.scene.solids.len(), n);
            counts.insert(n);
            for s in &sb.scene.solids {
                assert!(s.center.y - s.scale >= 0.0, "above the marching slab floor");
                assert!(s.center.y + s.scale <= sb.scene.slab_top);
            }
        }
        assert!(counts.len() > 3, "count varies: {counts:?}");
    }

    #[test]
    fn frame_shows_solids_in_their_colours_and_the_box() {
        let mut sb = full(Sandbox::new(11));
        let rs = render_settings(0.75);
        let (w, h) = (100, 40);
        sb.render(w, h, &rs);
        assert_eq!((sb.chars.len(), sb.colors.len()), (w * h, w * h));
        let edges = sb.colors.iter().filter(|&&c| c == EDGE_COLOR).count();
        assert!(edges > 100, "cube wireframe drawn: {edges}");
        let solid_cells = sb.colors.iter().filter(|&&c| c != EDGE_COLOR && c != 0xffffff).count();
        assert!(solid_cells > 50, "solids visible: {solid_cells}");
    }

    #[test]
    fn box_appears_30_percent_larger() {
        let mut sb = full(Sandbox::new(11));
        let rs = render_settings(0.75);
        let span = |sb: &mut Sandbox| {
            sb.render(160, 60, &rs);
            let p = sb.proj.unwrap();
            let (a, b) = (p.project(sb.world.min).unwrap(), p.project(sb.world.max).unwrap());
            ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
        };
        let zoomed = span(&mut sb);
        let c = sb.world.centre();
        let pos = sb.scene.camera.pos;
        sb.scene.camera = Camera::look_at(pos, c, FOV_DEG, FOV_DEG);
        sb.renderer.render(&sb.scene, 160, 60, &rs);
        let p = sb.renderer.view().unwrap().proj;
        let (a, b) = (p.project(sb.world.min).unwrap(), p.project(sb.world.max).unwrap());
        let plain = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        assert!((zoomed / plain - ZOOM).abs() < 0.02, "{zoomed} / {plain}");
    }

    #[test]
    fn tap_target_is_30_percent_larger() {
        let mut sb = full(Sandbox::new(3));
        sb.world.bodies.truncate(1);
        let b = sb.world.bodies[0];
        // a ray passing beside the solid, between the old and new target radii
        let ro = b.pos - Vec3A::Z * 30.0 + Vec3A::X * b.radius * 1.35 * 1.15;
        assert_eq!(sb.world.pick(ro, Vec3A::Z, 1.35), None, "missed at the old size");
        assert_eq!(sb.world.pick(ro, Vec3A::Z, PICK_SLACK), Some(0), "hit at the new size");
        assert!((PICK_SLACK / 1.35 - 1.3).abs() < 1e-6);
    }

    #[test]
    fn container_is_picked_at_random_and_drawn() {
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..40 {
            let mut sb = full(Sandbox::new(seed));
            seen.insert(sb.world.shape.id());
            if seed < 8 {
                sb.render(100, 40, &render_settings(0.75));
                let edges = sb.colors.iter().filter(|&&c| c == EDGE_COLOR).count();
                assert!(edges > 60, "{:?}: {edges} edge cells", sb.world.shape);
            }
        }
        assert_eq!(seen.len(), physics::CONTAINERS.len(), "{seen:?}");
    }

    #[test]
    fn settings_fix_container_count_wires_and_tap_speed() {
        let s = Settings { pick: [3, 3, 1, 2, 0, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }; // dodecahedron, 6 solids, no wires, fast
        for seed in 0..5 {
            let mut sb = full(Sandbox::with_settings(seed, &s));
            assert_eq!(sb.world.shape, platomon::scene::sdf::SolidKind::Dodecahedron);
            assert_eq!(sb.world.bodies.len(), 6);
            for b in &mut sb.world.bodies {
                b.vel = Vec3A::ZERO;
            }
            sb.render(160, 60, &render_settings(0.75));
            assert_eq!(sb.colors.iter().filter(|&&c| c == wire_color(0.0)).count(), 0, "no wireframe");
            sb.world.kick(0, &mut fastrand::Rng::with_seed(1));
            assert!((sb.world.bodies[0].speed() - s.tap_speed()).abs() < 1e-4);
            assert_eq!(sb.speed_t(0), 1.0);
        }
    }

    #[test]
    fn landscape_phone_held_upright_pulls_down_the_screen() {
        // held upright in landscape, the phone's left (rotation 90) or right
        // (270) edge is down: the accelerometer reads +g along that axis
        let g = physics::STANDARD_G;
        assert!((screen_gravity([g, 0.0, 0.0], 1) - Vec3A::new(0.0, -g, 0.0)).length() < 1e-4);
        assert!((screen_gravity([-g, 0.0, 0.0], 3) - Vec3A::new(0.0, -g, 0.0)).length() < 1e-4);
        // portrait upright, and flat on a table (pulls into the glass)
        assert!((screen_gravity([0.0, g, 0.0], 0) - Vec3A::new(0.0, -g, 0.0)).length() < 1e-4);
        assert!((screen_gravity([0.0, 0.0, g], 1) - Vec3A::new(0.0, 0.0, -g)).length() < 1e-4);
        // right edge lowered (rotation 90): "up" is screen left, device +y
        assert!(screen_gravity([0.0, g, 0.0], 1).x > 0.0);
    }

    #[test]
    fn gravity_points_where_the_screen_says() {
        let mut sb = full(Sandbox::with_settings(3, &Settings { pick: [1, 1, 0, 1, 2, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }));
        sb.render(160, 60, &render_settings(0.75));
        let p = sb.proj.unwrap();
        let c = sb.world.centre();
        let on_screen = |v: Vec3A| {
            let (a, b) = (p.project(c).unwrap(), p.project(c + v).unwrap());
            (b.0 - a.0, b.1 - a.1)
        };
        let g = physics::STANDARD_G;
        // upright landscape: down the screen (rows grow downwards)
        sb.set_accel(Some(([g, 0.0, 0.0], 1)));
        let down = sb.world_gravity();
        assert!((down.length() - physics::GRAVITY).abs() < 1e-3);
        let (dx, dy) = on_screen(down * 0.1);
        assert!(dy > 0.0 && dy.abs() > 5.0 * dx.abs(), "{dx} {dy}");
        // right edge lowered: towards the right of the screen
        sb.set_accel(Some(([0.0, g, 0.0], 1)));
        let (dx, dy) = on_screen(sb.world_gravity() * 0.1);
        assert!(dx > 0.0 && dx.abs() > 5.0 * dy.abs(), "{dx} {dy}");
        // flat on the table: away from the viewer
        sb.set_accel(Some(([0.0, 0.0, g], 1)));
        assert!(sb.world_gravity().dot(sb.scene.camera.forward) > 0.99 * physics::GRAVITY);
        // a hard shake is capped; off or no sensor means none
        sb.set_accel(Some(([0.0, 0.0, 10.0 * g], 1)));
        assert!((sb.world_gravity().length() - 3.0 * physics::GRAVITY).abs() < 1e-2);
        sb.set_accel(None);
        assert_eq!(sb.world_gravity(), Vec3A::ZERO);
        sb.set_accel(Some(([g, 0.0, 0.0], 1)));
        sb.gravity = 0.0;
        assert_eq!(sb.world_gravity(), Vec3A::ZERO);
        // low gravity: 85% less pull, same direction
        sb.gravity = 0.15;
        assert!((sb.world_gravity() - down * 0.15).length() < 1e-3);
        // and the solids fall that way
        sb.gravity = 1.0;
        let before: f32 = sb.world.bodies.iter().map(|b| b.pos.dot(down)).sum();
        for _ in 0..30 {
            sb.step(1.0 / 30.0);
        }
        let after: f32 = sb.world.bodies.iter().map(|b| b.pos.dot(down)).sum();
        assert!(after > before + 1.0, "{before} -> {after}");
    }

    #[test]
    fn exact_mode_renders_the_bodies_true_orientation() {
        // sphere mode: the quaternion gives the same matrix as (pitch, yaw)
        let mut sb = full(Sandbox::new(6));
        sb.step(0.2);
        for (i, b) in sb.world.bodies.iter().enumerate() {
            let m = inverse_rotation(b.angles.0, b.angles.1);
            assert!((sb.scene.solids[i].inv_rot - m).abs_diff_eq(Mat3A::ZERO, 1e-4));
        }
        // exact mode: the solids tumble freely and the scene follows
        let mut sb = full(Sandbox::with_settings(6, &Settings { pick: [2, 8, 0, 1, 0, 1, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }));
        assert!(sb.world.is_exact());
        let mut rng = fastrand::Rng::with_seed(2);
        for i in 0..sb.world.bodies.len() {
            sb.world.kick(i, &mut rng);
        }
        for _ in 0..60 {
            sb.step(1.0 / 30.0);
        }
        sb.render(120, 45, &render_settings(0.75));
        for (i, b) in sb.world.bodies.iter().enumerate() {
            let m = Mat3A::from_quat(b.rot).transpose();
            assert!((sb.scene.solids[i].inv_rot - m).abs_diff_eq(Mat3A::ZERO, 1e-5));
            assert!(sb.world.body_inside(i) || b.pos.is_finite());
        }
        assert!(sb.colors.iter().filter(|&&c| c != EDGE_COLOR && c != 0xffffff).count() > 50, "solids drawn");
    }

    #[test]
    fn settings_reach_the_physics() {
        // high mass variability, no elasticity, high damping
        let sb = full(Sandbox::with_settings(4, &Settings { pick: [1, 9, 0, 1, 0, 0, 3, 0, 3, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }));
        let m: Vec<f32> = sb.world.bodies.iter().map(|b| b.mass).collect();
        let lo = m.iter().copied().fold(f32::MAX, f32::min);
        assert!((lo - 0.05).abs() < 1e-5 && m.contains(&1.0), "{m:?}");
        assert_eq!((sb.world.elasticity, sb.world.damping), (0.0, 0.5));
        let sb = full(Sandbox::new(4));
        assert!(sb.world.bodies.iter().all(|b| b.mass == 1.0));
        assert_eq!((sb.world.elasticity, sb.world.damping, sb.glow), (physics::RESTITUTION, 1.0, Glow::Off));
    }

    #[test]
    fn sim_speed_and_stepping_apply_without_a_restart() {
        let travel = |pick_speed: usize, pick_step: usize| {
            let mut s = Settings::default();
            (s.pick[17], s.pick[19]) = (pick_speed, pick_step);
            let mut sb = Sandbox::with_settings(9, &Settings::default());
            sb.set_performance(&s);
            sb.world.bodies.truncate(1);
            sb.world.bodies[0].pos = sb.world.centre();
            sb.world.bodies[0].vel = Vec3A::X;
            let (x0, orbit0) = (sb.world.bodies[0].pos.x, sb.orbit);
            for _ in 0..10 {
                sb.step(0.1);
            }
            (sb.world.bodies[0].pos.x - x0, sb.orbit - orbit0, sb.world.step_hz)
        };
        let (normal, orbit, hz) = travel(4, 1);
        assert!((normal - 1.0).abs() < 1e-3 && hz == physics::STEP_HZ, "{normal}");
        let (fast, fast_orbit, _) = travel(6, 1);
        assert!((fast - 2.0).abs() < 1e-3, "2x: {fast}");
        assert!((fast_orbit - orbit).abs() < 1e-5, "the camera orbits in real time");
        let (slow, _, hz) = travel(2, 3);
        assert!((slow - 0.25).abs() < 1e-3 && hz == 960.0, "0.25x: {slow}");
        // a stall of a second still only advances 0.1 s × speed
        let mut sb = full(Sandbox::new(9));
        sb.speed = 2.0;
        sb.world.bodies.truncate(1);
        sb.world.bodies[0].pos = sb.world.centre();
        sb.world.bodies[0].vel = Vec3A::X;
        sb.step(1.0);
        assert!((sb.world.bodies[0].pos.x - sb.world.centre().x - 0.2).abs() < 1e-3);
    }

    #[test]
    fn glow_follows_energy() {
        assert_eq!(glow_level(0.0, TAP_SPEED), 0.0);
        assert_eq!(glow_level((0.3 * TAP_SPEED).powi(2), TAP_SPEED), 0.0, "drifting solids stay dark");
        assert_eq!(glow_level(TAP_SPEED * TAP_SPEED, TAP_SPEED), 1.0, "a freshly tapped solid glows fully");
        assert!(glow_level((0.6 * TAP_SPEED).powi(2), TAP_SPEED) > 0.0);
        assert!(falloff(0.0, 0.4) == 1.0 && falloff(0.4, 0.4) == 0.0 && falloff(1.0, 0.4) == 0.0);
    }

    #[test]
    fn glowing_solids_light_themselves_and_close_neighbours_only() {
        let s = Settings { pick: [1, 1, 0, 1, 0, 1, 0, 2, 0, 1, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }; // cube, 4 solids, exact, glow
        let mut sb = full(Sandbox::with_settings(7, &s));
        sb.wireframe = false;
        let c = sb.world.centre();
        // 0 glows; 1 sits just behind and beside it, so the halo's rays
        // round 0 run on into 1; 2 and 3 are far away
        let r0 = sb.world.bodies[0].radius;
        let (right, up, fwd) = (sb.scene.camera.right, sb.scene.camera.up, sb.scene.camera.forward);
        sb.world.bodies[0].pos = c;
        sb.world.bodies[1].pos = c + right * (r0 + sb.world.bodies[1].radius) * 0.85 + fwd * 1.2;
        sb.world.bodies[2].pos = c - right * 3.5;
        sb.world.bodies[3].pos = c + up * 3.5;
        for b in &mut sb.world.bodies {
            b.vel = Vec3A::ZERO;
            b.omega = Vec3A::ZERO;
        }
        sb.sync();
        let rs = render_settings(0.75);
        let (w, h) = (160, 60);
        let brightness = |c: u32| (c >> 16) + ((c >> 8) & 0xff) + (c & 0xff);
        let sum = |sb: &Sandbox, i: u8| -> u32 {
            let ids = sb.renderer.view().unwrap().ids;
            (0..w * h).filter(|&k| ids[k] == i).map(|k| brightness(sb.colors[k])).sum()
        };
        sb.render(w, h, &rs);
        let dark: Vec<u32> = (0..4).map(|i| sum(&sb, i)).collect();
        let empty = sb.colors.iter().filter(|&&c| c == 0xffffff).count();
        let dark_colors = sb.colors.clone();
        sb.world.bodies[0].vel = Vec3A::X * TAP_SPEED;
        sb.render(w, h, &rs);
        assert_eq!(sb.glow_levels[0], 1.0);
        let lit: Vec<u32> = (0..4).map(|i| sum(&sb, i)).collect();
        // (the path tracer's sampling noise moves a sum by well under 1%)
        let same = |a: u32, b: u32| a.abs_diff(b) * 100 <= a.max(b);
        assert!(lit[0] > dark[0] * 11 / 10, "the glowing solid brightens: {dark:?} -> {lit:?}");
        assert!(lit[1] > dark[1] * 105 / 100, "its neighbour is lit: {dark:?} -> {lit:?}");
        // exactly where a halo ray would reach: the neighbour's cells nearest
        // 0 on screen are lit, its far side is not
        let ids = sb.renderer.view().unwrap().ids.to_vec();
        let p = sb.proj.unwrap();
        let x0 = p.project(sb.world.bodies[0].pos).unwrap().0;
        let x1 = p.project(sb.world.bodies[1].pos).unwrap().0;
        let gain = |near: bool| -> i64 {
            (0..w * h)
                .filter(|&k| ids[k] == 1 && (((k % w) as f32) < x1) == near)
                .map(|k| brightness(sb.colors[k]) as i64 - brightness(dark_colors[k]) as i64)
                .sum()
        };
        let (near, far) = (gain(true), gain(false));
        assert!(x0 < x1 && near > 2000 && near > 20 * far.abs(), "near side gains {near}, far side {far}");
        assert!(same(lit[2], dark[2]) && same(lit[3], dark[3]), "far solids are not: {dark:?} -> {lit:?}");
        let halo = sb.colors.iter().filter(|&&c| c == 0xffffff).count();
        assert!(halo < empty, "a halo takes some empty cells: {empty} -> {halo}");
        // glow off: nothing changes
        sb.glow = Glow::Off;
        sb.render(w, h, &rs);
        assert!(same(sum(&sb, 1), dark[1]), "glow off: the neighbour is unlit");
    }

    #[test]
    fn colour_modes_follow_speed_spin_and_collisions() {
        let mut sb = full(Sandbox::new(4));
        let b = &mut sb.world.bodies[0];
        (b.vel, b.hits) = (Vec3A::X * TAP_SPEED * 0.5, 51);
        sb.colour_by = ColourBy::Velocity;
        assert!((sb.colour_t(0) - 0.5).abs() < 1e-4);
        sb.colour_by = ColourBy::Collisions;
        assert!((sb.colour_t(0) - 0.2).abs() < 1e-4);
        sb.colour_by = ColourBy::Spin;
        let sphere_spin = sb.colour_t(0);
        assert!(sphere_spin > 0.0 && sphere_spin <= 1.0);
        sb.world.exact = true;
        sb.world.bodies[0].omega = Vec3A::Y * SPIN_REF * 0.25;
        assert!((sb.colour_t(0) - 0.25).abs() < 1e-4, "exact mode: the real spin");
        // and the frame's solid colours change with the mode
        let mut sb = full(Sandbox::new(4));
        for b in &mut sb.world.bodies {
            (b.vel, b.hits) = (Vec3A::ZERO, 255);
        }
        let rs = render_settings(0.75);
        sb.render(120, 45, &rs);
        let by_speed = sb.colors.clone();
        sb.colour_by = ColourBy::Collisions;
        sb.render(120, 45, &rs);
        assert_ne!(by_speed, sb.colors);
    }

    #[test]
    fn palette_and_saturation_recolour_only_the_solids() {
        let rs = render_settings(0.75);
        let (w, h) = (120, 45);
        let mut sb = full(Sandbox::new(11));
        sb.render(w, h, &rs);
        let plain = sb.colors.clone();
        let palettes = Palettes::generate(5);
        let mut s = Settings::default();
        s.pick[crate::settings::PALETTE_FIELD] = 1; // 3-bit A
        sb.set_live(&s, Some(&palettes));
        assert!(sb.has_palette());
        sb.render(w, h, &rs);
        let ids = sb.renderer.view().unwrap().ids.to_vec();
        let n = sb.world.bodies.len();
        let mut solid_cells = 0;
        for k in 0..w * h {
            if (ids[k] as usize) < n && plain[k] != wire_color(sb.speed_t(ids[k] as usize)) {
                solid_cells += 1;
                assert!(palettes.sets[0].contains(&sb.colors[k]) || sb.colors[k] >> 16 == sb.colors[k] & 0xff, "solid cell {k} in the palette");
            }
            if plain[k] == EDGE_COLOR {
                assert_eq!(sb.colors[k], EDGE_COLOR, "the container's edges keep their colour");
            }
        }
        assert!(solid_cells > 50);
        let used: std::collections::BTreeSet<u32> = (0..w * h).filter(|&k| (ids[k] as usize) < n).map(|k| sb.colors[k]).collect();
        assert!(used.len() <= 8 + 20, "3-bit palette (plus grey wires): {} colours", used.len());
        // low saturation pulls the solids towards grey
        let mut s = Settings::default();
        s.pick[12] = 0;
        sb.set_live(&s, Some(&palettes));
        assert!(!sb.has_palette());
        sb.render(w, h, &rs);
        let spread = |c: u32| {
            let v = [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff];
            v.iter().max().unwrap() - v.iter().min().unwrap()
        };
        let total = |cs: &[u32]| (0..w * h).filter(|&k| (ids[k] as usize) < n).map(|k| spread(cs[k])).sum::<u32>();
        assert!(total(&sb.colors) < total(&plain) * 7 / 10);
    }

    #[test]
    fn an_explosion_throws_every_face_out_and_the_solid_comes_back() {
        let mut sb = full(Sandbox::with_settings(8, &Settings { pick: [1, 1, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }));
        let c = sb.world.centre();
        for b in &mut sb.world.bodies {
            b.vel = Vec3A::ZERO;
        }
        sb.world.bodies[0].pos = c;
        let faces = sb.faces[0].len();
        let rs = render_settings(0.75);
        sb.render(160, 60, &rs);
        let before = sb.colors.clone();
        sb.world.blast(0, 1.0);
        sb.step(1.0 / 30.0);
        assert_eq!(sb.fragments.len(), faces, "one fragment per face");
        assert!(!sb.world.bodies[0].alive && sb.scene.solids[0].center == HIDDEN);
        // they fly radially outward from the centre, each spinning
        for _ in 0..6 {
            sb.step(1.0 / 30.0);
        }
        for f in &sb.fragments {
            let out = f.centre - c;
            assert!(out.dot(f.normal) > 0.5 && out.normalize().dot(f.normal) > 0.95, "radial: {out:?} vs {:?}", f.normal);
            assert!(f.spin.abs() >= BLAST_SPIN.0 && f.angle != 0.0);
        }
        sb.render(160, 60, &rs);
        assert_ne!(before, sb.colors, "the fragments are drawn");
        assert!(sb.chars.iter().filter(|&&ch| ch != ' ').count() > 100);
        // twice as violent: twice as fast
        let mut sb2 = full(Sandbox::with_settings(8, &Settings { pick: [1, 1, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }));
        sb2.world.bodies[0].vel = Vec3A::ZERO;
        sb2.world.blast(0, 2.0);
        sb2.step(1.0 / 30.0);
        let speed = |s: &Sandbox| s.fragments.iter().map(|f| f.vel.length()).sum::<f32>() / s.fragments.len() as f32;
        assert!(speed(&sb2) > 1.5 * speed(&sb), "{} vs {}", speed(&sb2), speed(&sb));
        // the faces don't all go out together: each has its own life and curve
        let lives: Vec<f32> = sb.fragments.iter().map(|f| f.life).collect();
        let fades: Vec<f32> = sb.fragments.iter().map(|f| f.fade).collect();
        let spread = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread(&lives) > 0.3 && spread(&fades) > 0.3, "lives {lives:?}, curves {fades:?}");
        assert!(lives.iter().all(|&l| l >= BLAST_LIFE * BLAST_LIFE_SPREAD.0 && l <= BLAST_LIFE * BLAST_LIFE_SPREAD.1));
        let mut gone_at = Vec::new();
        // once its last face has gone out the solid is back, fresh
        for k in 0..160 {
            let before = sb.fragments.len();
            sb.step(1.0 / 30.0);
            if sb.fragments.len() < before {
                gone_at.push(k);
            }
            if sb.fragments.is_empty() {
                break;
            }
        }
        assert!(gone_at.len() > 1 && gone_at.last().unwrap() - gone_at[0] > 10, "they go out over a while: {gone_at:?}");
        assert!(sb.fragments.is_empty());
        let b = sb.world.bodies[0];
        assert!(b.alive && b.hits == 0 && sb.scene.solids[0].center == b.pos);
    }

    #[test]
    fn solids_launch_as_specks_and_are_drawn_at_their_size() {
        let mut sb = Sandbox::new(11);
        assert!(sb.world.growth);
        for (i, b) in sb.world.bodies.iter().enumerate() {
            assert!(b.growing() && b.size == physics::SPAWN_SIZE, "solid {i}");
            assert!((sb.scene.solids[i].scale - physics::RADIUS * physics::SPAWN_SIZE).abs() < 1e-6);
        }
        let rs = render_settings(0.75);
        sb.render(160, 60, &rs);
        let n = sb.world.bodies.len();
        let cells = |sb: &Sandbox| sb.renderer.view().unwrap().ids.iter().filter(|&&id| (id as usize) < n).count();
        let specks = cells(&sb);
        sb.world.finish_growth();
        sb.sync();
        sb.render(160, 60, &rs);
        assert!(cells(&sb) > 20 * specks.max(1), "full grown they fill far more cells: {specks} -> {}", cells(&sb));
        // a respawned solid starts small again
        sb.world.blast(0, 1.0);
        for _ in 0..160 {
            sb.step(1.0 / 30.0);
        }
        assert!(sb.world.bodies[0].alive && sb.world.bodies[0].growing());
    }

    /// A grown sandbox, at rest, with one solid at the centre.
    fn still(seed: u64, settings: &Settings) -> Sandbox {
        let mut sb = full(Sandbox::with_settings(seed, settings));
        let c = sb.world.centre();
        for b in &mut sb.world.bodies {
            (b.vel, b.omega) = (Vec3A::ZERO, Vec3A::ZERO);
        }
        sb.world.bodies[0].pos = c;
        sb.sync();
        sb
    }

    #[test]
    fn worn_out_solids_throw_sparks_from_their_edges() {
        let mut sb = still(5, &Settings::default());
        let edges = sb.wires[0].1.len() as f32;
        sb.world.bodies[0].hits = (SPARK_FROM * 255.0) as u8; // at 90%: none yet
        sb.step(0.1);
        assert!(sb.sparks.is_empty());
        sb.world.bodies[0].hits = 240;
        let mut thrown = 0;
        for _ in 0..30 {
            sb.world.bodies[0].vel = Vec3A::ZERO;
            sb.step(1.0 / 30.0);
            thrown += sb.sparks.iter().filter(|s| s.age < 1.0 / 30.0 + 1e-6).count();
        }
        assert!((thrown as f32 - edges * SPARK_RATE).abs() <= 2.0, "3 per edge per second: {thrown} for {edges} edges");
        let c = sb.world.bodies[0].pos;
        for s in &sb.sparks {
            assert_eq!(s.solid, 0);
            assert!(s.vel.normalize().dot((s.pos - c).normalize()) > 0.95, "radial");
            assert!(s.lum() <= 0.5, "the lower half of the ramp");
        }
        // sparks die fast, and they're drawn in the solid's colours
        assert!(sb.sparks.iter().all(|s| s.life <= SPARK_LIFE.1));
        sb.render(160, 60, &render_settings(0.75));
        // an explosion throws 500 more at once
        let mut sb = still(5, &Settings::default());
        sb.world.blast(0, 1.0);
        sb.step(1e-4);
        assert_eq!(sb.sparks.len(), BLAST_SPARKS);
        let v = sb.sparks.iter().map(|s| s.vel.length()).sum::<f32>() / BLAST_SPARKS as f32;
        assert!(v > SPARK_SPEED.0 * 0.95, "fast: {v}");
        let inks: Vec<u32> = sb.sparks.iter().map(|s| sb.gradients[0].sample(s.t, 0.4 + 0.6 * s.lum())).collect();
        sb.fragments.clear();
        sb.render(200, 80, &render_settings(0.75));
        let drawn = sb.colors.iter().filter(|c| inks.contains(c)).count();
        assert!(drawn > 20, "sparks take the solid's colour: {drawn} cells");
        // and they're gone within their short life
        for _ in 0..20 {
            sb.step(1.0 / 30.0);
        }
        assert!(sb.sparks.is_empty());
    }

    #[test]
    fn fragment_wires_fade_half_as_fast_again() {
        let f = Fragment {
            solid: 0,
            centre: Vec3A::ZERO,
            corners: vec![],
            normal: Vec3A::Y,
            vel: Vec3A::ZERO,
            axis: Vec3A::Y,
            spin: 0.0,
            angle: 0.0,
            age: 1.0,
            life: 3.0,
            fade: 1.0,
            flicker: (0.0, 0.0),
            t: 0.0,
        };
        // at 1/3 of its life the face has 2/3 left (less flicker), the wire 1/2
        let wobble = |left: f32| 1.0 - BLAST_FLICKER * (1.0 - left) * 0.5;
        assert!((f.remaining() - 2.0 / 3.0 * wobble(2.0 / 3.0)).abs() < 1e-4);
        assert!((f.wire_remaining() - 0.5 * wobble(0.5)).abs() < 1e-4);
        assert_eq!(Fragment { age: 2.0, ..f }.wire_remaining(), 0.0, "the wire is out at 2/3 of the face's life");
        // drawn: fragment edges show line glyphs with the wireframe on, none off
        let lines = |wireframe: bool| {
            let mut sb = still(8, &Settings::default());
            sb.wireframe = wireframe;
            sb.world.blast(0, 1.0);
            sb.step(1.0 / 30.0);
            sb.sparks.clear();
            for b in sb.world.bodies.iter_mut().skip(1) {
                b.alive = false;
            }
            sb.sync();
            sb.render(200, 80, &render_settings(0.75));
            sb.under.iter().filter(|u| u.0 != NO_WIRE && u.2 < 1.0).count()
        };
        assert!(lines(true) > 10, "fragment wires drawn");
        assert_eq!(lines(false), 0);
        // and the variability is wider than 1.10's
        assert!(BLAST_LIFE_SPREAD.1 / BLAST_LIFE_SPREAD.0 > 1.5 / 0.45 && BLAST_FADE_CURVE.1 / BLAST_FADE_CURVE.0 > 2.6 / 0.4);
    }

    #[test]
    fn glow_modes_reach_further_and_high_adds_up() {
        let mut s = Settings::default();
        s.pick[9] = 1;
        let mut sb = still(7, &s);
        let tap = sb.world.tap_speed;
        // a full-speed solid, otherwise idle, with a filling counter
        sb.world.bodies[0].vel = Vec3A::X * tap;
        sb.world.bodies[0].hits = 255;
        let t: Vec<f32> = (0..sb.world.bodies.len()).map(|i| sb.colour_t(i)).collect();
        let r = sb.world.bodies[0].radius;
        let throw = |sb: &mut Sandbox, g: Glow| {
            sb.glow = g;
            sb.emitters(&t).iter().find(|e| e.solid == 0 && e.own).map(|e| e.throw)
        };
        assert!((throw(&mut sb, Glow::Low).unwrap() - GLOW_THROW * r).abs() < 1e-5);
        assert!((throw(&mut sb, Glow::Medium).unwrap() - (GLOW_THROW + GLOW_GROW_MEDIUM) * r).abs() < 1e-5);
        assert!((throw(&mut sb, Glow::High).unwrap() - (GLOW_THROW + GLOW_GROW_HIGH) * r).abs() < 1e-5);
        sb.world.bodies[0].hits = 0;
        assert!((throw(&mut sb, Glow::High).unwrap() - GLOW_THROW * r).abs() < 1e-5, "no wear, no extra reach");
        assert_eq!(throw(&mut sb, Glow::Off), None);
        // medium and high count spin as well as speed; low (sphere mode) doesn't
        sb.world.bodies[0].vel = Vec3A::X * 0.01;
        sb.world.bodies[0].spin = (40.0, 40.0);
        assert_eq!(throw(&mut sb, Glow::Low), None, "spinning in place: dark on low");
        assert!(throw(&mut sb, Glow::Medium).is_some(), "lit on medium");
        // high: the halo adds to what's behind it instead of replacing it
        sb.world.bodies[0].spin = (0.0, 0.0);
        sb.world.bodies[0].vel = Vec3A::X * tap;
        sb.world.bodies[0].hits = 255;
        let rs = render_settings(0.75);
        let lit = |sb: &mut Sandbox, g: Glow| {
            sb.glow = g;
            sb.render(160, 60, &rs);
            // brightness by glyph: each cell's level on the ramp
            let level: std::collections::HashMap<char, usize> = (0..=400).map(|k| k as f32 / 400.0).map(|l| (sb.ramp.glyph(l), sb.ramp.level(l))).collect();
            sb.chars.iter().map(|c| level.get(c).copied().unwrap_or(0)).sum::<usize>()
        };
        let (low, high) = (lit(&mut sb, Glow::Low), lit(&mut sb, Glow::High));
        assert!(high as f32 > low as f32 * 1.002, "high lights more: {low} vs {high}");
    }

    #[test]
    fn an_explosion_flashes_the_glow() {
        let mut s = Settings::default();
        s.pick[9] = 1;
        let mut sb = still(7, &s);
        sb.world.bodies[1].vel = Vec3A::X * sb.world.tap_speed;
        let t: Vec<f32> = (0..sb.world.bodies.len()).map(|i| sb.colour_t(i)).collect();
        sb.emitters(&t);
        let calm = sb.glow_levels[1];
        sb.world.blast(0, 1.0);
        sb.step(0.02);
        sb.world.bodies[1].vel = Vec3A::X * sb.world.tap_speed;
        let es = sb.emitters(&t);
        assert!((sb.glow_levels[1] - calm * FLASH_BOOST).abs() < 1e-4, "150%: {calm} -> {}", sb.glow_levels[1]);
        assert!(es.iter().any(|e| !e.own && e.solid == 0), "the blast's own light");
        // over after 100 ms
        sb.step(0.05);
        sb.step(0.05);
        sb.world.bodies[1].vel = Vec3A::X * sb.world.tap_speed;
        sb.emitters(&t);
        assert!((sb.glow_levels[1] - calm).abs() < 1e-4);
        // no flash with glow off
        let mut sb = still(7, &Settings::default());
        sb.world.blast(0, 1.0);
        sb.step(0.02);
        assert!(sb.flashes.is_empty());
    }

    #[test]
    fn dynamic_camera_backs_off_smoothly_with_speed() {
        let mut s = Settings::default();
        s.pick[crate::settings::CAMERA_FIELD] = 1;
        let mut sb = still(3, &s);
        let tap = sb.world.tap_speed;
        let dist = |sb: &Sandbox| (sb.scene.camera.pos - sb.world.centre()).length();
        assert!((dist(&sb) - CAMERA_DISTANCE).abs() < 1e-3);
        let run = |sb: &mut Sandbox, secs: f32, speed: f32| {
            let mut fastest = 0.0f32;
            for _ in 0..(secs * 30.0) as usize {
                for b in &mut sb.world.bodies {
                    b.vel = b.vel.normalize_or(Vec3A::X) * speed;
                }
                let before = dist(sb);
                sb.step(1.0 / 30.0);
                fastest = fastest.max((dist(sb) - before).abs() * 30.0);
            }
            fastest
        };
        // never faster than the container's edge turns at the camera
        let fastest = run(&mut sb, 12.0, tap);
        let limit = TAU / ORBIT_PERIOD * CAMERA_DISTANCE * (1.0 + CAMERA_PULL);
        assert!(fastest <= limit * 1.01, "{fastest} vs {limit}");
        assert!((sb.camera_back - CAMERA_DISTANCE * CAMERA_PULL).abs() < 0.3, "backed right off at tap speed: {}", sb.camera_back);
        // a brief stop barely moves the 2 s average: the camera holds
        let back = sb.camera_back;
        run(&mut sb, 0.2, 0.0);
        assert!(back - sb.camera_back < back * 0.15, "smoothed: {back} -> {}", sb.camera_back);
        // and with it off the camera comes home
        sb.dynamic_camera = false;
        run(&mut sb, 15.0, tap);
        assert!(sb.camera_back.abs() < 1e-3);
    }

    #[test]
    fn white_glow_keeps_half_the_chroma_and_shading_dithers() {
        assert_eq!(chroma(0xff_00_00, 1.0), 0xff_00_00);
        let half = chroma(0xff_00_00, 0.5);
        let (r, g) = (half >> 16, (half >> 8) & 0xff);
        assert!(r > g && g > 0 && (r - g) as f32 > 0.45 * 255.0 && ((r - g) as f32) < 0.55 * 255.0, "{half:06x}");
        // dither thresholds cover (0, 1) evenly over a 4x4 tile
        let mut th: Vec<f32> = (0..16).map(|k| bayer(k % 4, k / 4)).collect();
        th.sort_by(f32::total_cmp);
        assert!(th.windows(2).all(|w| (w[1] - w[0] - 1.0 / 16.0).abs() < 1e-6));
        // rendered: white glow over a glowing solid is less colourful than inherited
        let mut s = Settings::default();
        s.pick[9] = 1;
        let mut sb = still(7, &s);
        sb.wireframe = false;
        sb.world.bodies[0].vel = Vec3A::X * sb.world.tap_speed;
        let rs = render_settings(0.75);
        let spread = |sb: &Sandbox| -> f32 {
            let ids = sb.renderer.view().unwrap().ids.to_vec();
            let cells: Vec<u32> = (0..ids.len()).filter(|&k| ids[k] == 0).map(|k| sb.colors[k]).collect();
            cells.iter().map(|&c| { let ch = [c >> 16, (c >> 8) & 0xff, c & 0xff]; (ch.iter().max().unwrap() - ch.iter().min().unwrap()) as f32 }).sum::<f32>() / cells.len().max(1) as f32
        };
        sb.render(160, 60, &rs);
        let inherited = spread(&sb);
        sb.white_glow = true;
        sb.render(160, 60, &rs);
        assert!(spread(&sb) < inherited * 0.8, "white glow washes the colour out: {inherited} -> {}", spread(&sb));
    }

    #[test]
    fn spin_makes_explosions_faster_and_more_uneven() {
        let blast = |spin: f32| {
            let mut s = Settings::default();
            s.pick[5] = 1; // exact: real spin
            let mut sb = still(8, &s);
            sb.world.bodies[0].omega = Vec3A::Y * spin;
            sb.world.blast(0, 1.0);
            sb.step(1e-4);
            let v: Vec<f32> = sb.fragments.iter().map(|f| f.vel.length()).collect();
            let mean = v.iter().sum::<f32>() / v.len() as f32;
            let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
            (mean, sd / mean)
        };
        let (calm, calm_cv) = blast(0.0);
        let (wild, wild_cv) = blast(60.0);
        assert!(wild > calm * 1.6, "faster: {calm} -> {wild}");
        assert!(wild_cv > calm_cv * 1.5, "more uneven: {calm_cv} -> {wild_cv}");
    }

    #[test]
    fn stats_count_collisions_destruction_and_speed() {
        let mut sb = still(3, &Settings::default());
        sb.world.collisions = 17;
        sb.world.blast(0, 1.0);
        let n = sb.world.bodies.len() - 1;
        sb.world.bodies[1].vel = Vec3A::X * n as f32 * 2.0;
        let [c, d, v] = sb.stats();
        assert_eq!((c.as_str(), d.as_str()), ("COLLISIONS 17", "DESTROYED 1"));
        assert_eq!(v, "AVG SPEED 2.00", "the live solids' mean");
    }

    #[test]
    fn fragments_fade_along_their_own_curves() {
        let mut f = Fragment {
            solid: 0,
            centre: Vec3A::ZERO,
            corners: vec![],
            normal: Vec3A::Y,
            vel: Vec3A::ZERO,
            axis: Vec3A::Y,
            spin: 0.0,
            angle: 0.0,
            age: 0.0,
            life: 2.0,
            fade: 1.0,
            flicker: (0.0, 0.0),
            t: 0.0,
        };
        assert_eq!(f.remaining(), 1.0);
        f.age = 2.0;
        assert_eq!(f.remaining(), 0.0);
        f.age = 1.0;
        let linear = f.remaining();
        f.fade = 0.4;
        let holds = f.remaining();
        f.fade = 2.6;
        let early = f.remaining();
        assert!(holds > linear && linear > early, "{holds} {linear} {early}");
        // the flicker only ever dims, and only once it has started to fade
        f.fade = 1.0;
        for phase in 0..20 {
            f.flicker = (phase as f32 * 0.3, 17.0);
            assert!(f.remaining() <= 0.5 + 1e-6 && f.remaining() >= 0.5 * (1.0 - BLAST_FLICKER) - 1e-6);
        }
    }

    #[test]
    fn pop_spawn_launches_solids_part_grown() {
        for (k, size) in [(1, 0.1), (2, 0.3), (3, 0.6)] {
            let mut s = Settings::default();
            s.pick[crate::settings::SPAWN_FIELD] = k;
            let mut sb = Sandbox::with_settings(5, &s);
            for (i, b) in sb.world.bodies.iter().enumerate() {
                assert!(b.growing() && (b.size - size).abs() < 1e-6 && b.pop_rate > 0.0, "solid {i}");
            }
            // 16 s later every one is full size, untouched
            for _ in 0..16 * 30 {
                sb.step(1.0 / 30.0);
            }
            assert!(sb.world.bodies.iter().filter(|b| b.alive).all(|b| !b.growing() && b.size == 1.0));
        }
        // charge from the settings reaches the world, live
        let mut sb = Sandbox::new(5);
        let mut s = Settings::default();
        s.pick[15] = 0;
        sb.set_live(&s, None);
        assert_eq!(sb.world.charge.delay, 0.1);
    }

    #[test]
    fn wire_colours_white_inverted_and_bit_shift() {
        let rs = render_settings(0.75);
        let (w, h) = (160, 60);
        let mut sb = full(Sandbox::new(11));
        for b in &mut sb.world.bodies {
            b.vel = Vec3A::ZERO;
        }
        sb.render(w, h, &rs);
        let wires: Vec<usize> = (0..w * h).filter(|&k| sb.under[k].0 != NO_WIRE).collect();
        assert!(wires.len() > 40);
        assert!(wires.iter().all(|&k| sb.colors[k] == wire_color(0.0)), "velocity: grey");
        sb.wire_colour = WireColour::White;
        sb.render(w, h, &rs);
        assert!(wires.iter().all(|&k| sb.colors[k] == 0xffffff));
        // (what a wire covers is compared within one frame: sampling noise
        // moves the scene's colours a little from frame to frame)
        sb.wire_colour = WireColour::Inverted;
        sb.render(w, h, &rs);
        for &k in &wires {
            assert_eq!(sb.colors[k], sb.under[k].0 ^ 0xffffff, "the inverse of what it covers");
        }
        sb.wire_colour = WireColour::BitShift;
        sb.render(w, h, &rs);
        for &k in &wires {
            assert_eq!(sb.colors[k], bit_shift_24(sb.under[k].0), "no palette: 24 bits rotated by 12");
        }
        // with a palette: the covered colour's palette index rotated by half the depth
        let palettes = Palettes::generate(3);
        let mut s = Settings::default();
        (s.pick[crate::settings::PALETTE_FIELD], s.pick[14]) = (7, 3); // 6-bit A, bit-shift
        sb.set_live(&s, Some(&palettes));
        sb.render(w, h, &rs);
        let q = Quantizer::new(&palettes.sets[6]);
        for &k in &wires {
            let (u, solid, _) = sb.under[k];
            let covered = if solid { q.map(u) } else { u };
            assert_eq!(sb.colors[k], q.bit_shift(covered));
            assert!(palettes.sets[6].contains(&sb.colors[k]));
        }
        // the palette no longer recolours velocity wires
        s.pick[14] = 0;
        sb.set_live(&s, Some(&palettes));
        sb.render(w, h, &rs);
        assert!(wires.iter().all(|&k| sb.colors[k] == wire_color(0.0)));
    }

    #[test]
    fn faces_cover_every_solid() {
        for k in SolidKind::ALL {
            let p = platomon::scene::sdf::Polyhedron::new(k);
            let faces = face_rings(&p);
            assert_eq!(faces.len(), k.counts().0, "{k:?}");
            for (n, ring) in &faces {
                assert!(ring.len() >= 3);
                // ordered round the face: consecutive corners turn the same way
                let m = ring.iter().copied().sum::<Vec3A>() / ring.len() as f32;
                for i in 0..ring.len() {
                    let turn = (ring[i] - m).cross(ring[(i + 1) % ring.len()] - m).dot(*n);
                    assert!(turn > 0.0, "{k:?}");
                }
            }
        }
        assert!(in_convex((0.5, 0.5), &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]));
        assert!(!in_convex((1.5, 0.5), &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]));
    }

    #[test]
    fn camera_is_10_percent_closer() {
        let sb = full(Sandbox::new(5));
        let d = (sb.scene.camera.pos - sb.world.centre()).length();
        assert!((d - 13.5).abs() < 1e-3, "{d}");
        assert!(sb.world.verts.iter().all(|v| (*v - sb.world.centre()).length() < d - 1.0), "outside the container");
    }

    #[test]
    fn wireframe_brightness_follows_speed() {
        assert!(wire_color(0.0) < wire_color(0.5) && wire_color(0.5) < wire_color(1.0));
        assert_eq!(wire_color(1.0), 0xffffff);
        let grey = |c: u32| c >> 16 == (c >> 8) & 0xff && c >> 16 == c & 0xff;
        let mut sb = full(Sandbox::new(11));
        let rs = render_settings(0.75);
        let count = |sb: &mut Sandbox, c: u32| {
            sb.render(160, 60, &rs);
            sb.colors.iter().filter(|&&x| x == c).count()
        };
        for b in &mut sb.world.bodies {
            b.vel = Vec3A::ZERO;
        }
        let still = count(&mut sb, wire_color(0.0));
        assert!(still > 40, "wires drawn on resting solids: {still}");
        assert!(sb.colors.iter().filter(|&&c| grey(c) && c != 0xffffff).count() >= still);
        for b in &mut sb.world.bodies {
            b.vel = Vec3A::X * TAP_SPEED;
        }
        assert_eq!(count(&mut sb, wire_color(0.0)), 0);
        assert!(count(&mut sb, wire_color(1.0)) > 40, "bright wires on fast solids");
    }

    #[test]
    fn colour_follows_speed() {
        let mut sb = full(Sandbox::new(4));
        let g = &sb.gradients[0];
        sb.world.bodies[0].vel = Vec3A::ZERO;
        let slow = g.sample(sb.speed_t(0), 1.0);
        sb.world.bodies[0].vel = Vec3A::X * TAP_SPEED;
        let fast = g.sample(sb.speed_t(0), 1.0);
        assert_ne!(slow, fast);
        assert_eq!(sb.speed_t(0), 1.0);
    }

    #[test]
    fn tapping_the_centre_of_a_solid_kicks_it() {
        let mut sb = full(Sandbox::new(21));
        let rs = render_settings(0.75);
        let (w, h) = (120, 45);
        sb.render(w, h, &rs);
        let proj = sb.proj.unwrap();
        // tap where solid 0 projects; it is kicked unless another solid hides it
        let (x, y) = proj.project(sb.world.bodies[0].pos).unwrap();
        let hit = sb.tap(x, y).expect("a solid under the tap");
        assert!((sb.world.bodies[hit].speed() - TAP_SPEED).abs() < 1e-3);
        // empty corner of the screen: nothing
        assert_eq!(sb.tap(0.5, 0.5), None);
    }

    /// `cargo test --release print_frame -- --ignored --nocapture` to look at a frame.
    #[test]
    #[ignore]
    fn print_frame() {
        let mut shown = Vec::new();
        for seed in 0.. {
            let mut sb = full(Sandbox::new(seed));
            if shown.contains(&sb.world.shape) {
                continue;
            }
            shown.push(sb.world.shape);
            let (w, h) = (110, 44);
            sb.render(w, h, &render_settings(0.75));
            println!("== {:?} (seed {seed})", sb.world.shape);
            for row in sb.chars.chunks(w) {
                println!("{}", row.iter().collect::<String>());
            }
            if shown.len() == physics::CONTAINERS.len() {
                break;
            }
        }
    }

    #[test]
    fn steps_keep_everything_finite() {
        let mut sb = full(Sandbox::new(2));
        for _ in 0..300 {
            sb.step(1.0 / 30.0);
        }
        assert!(sb.world.bodies.iter().all(|b| b.pos.is_finite() && b.vel.is_finite()));
    }
}
