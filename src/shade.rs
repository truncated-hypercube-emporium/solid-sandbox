//! The optimized shader and ambient light.
//!
//! The path tracer marches every cell through every solid's SDF, then traces
//! a shadow ray and a bounce. The solids are flat-faced, so the optimized
//! shader instead rasterizes their front faces straight into the cell grid
//! and lights each face once per frame:
//!
//! - a face's light is one dot product (its normal against the direction to
//!   the scene's light) and one load from a tone-map lookup table, so the
//!   cost per frame grows with the number of faces, not of cells;
//! - each face's brightness falls between two glyph levels; the face keeps
//!   both levels' glyph and colour, and a 4×4 ordered dither picks one per
//!   cell, so a cell costs a coverage test, a depth test and one compare;
//! - ambient light darkens a face by how much of its view the other solids'
//!   spheres fill (distance between centres and the face's facing), a few
//!   multiply-adds per face and solid.
//!
//! With the raymarch shader, ambient light instead casts a few short rays
//! off each visible surface ([`ray_occlusion`]).

use glam::Vec3A;
use platomon::ascii::{Ramp, tone_map};
use platomon::render::march::{march_solids, normal};
use platomon::render::trace::onb;
use platomon::render::{NO_HIT, Projection};
use platomon::scene::Scene;

/// 4×4 ordered-dither threshold for cell (`x`, `y`), in (0, 1).
#[inline(always)]
pub fn bayer(x: usize, y: usize) -> f32 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (M[y & 3][x & 3] as f32 + 0.5) / 16.0
}

/// Radiance the tone-map table covers; brighter saturates (tone-mapped to
/// within 2% of white there anyway).
const LUT_MAX: f32 = 16.0;
const LUT_SIZE: usize = 1024;

/// Radiance → display luminosity (Reinhard and gamma, as the path tracer's
/// frames are tone-mapped), as a table.
#[derive(Debug, Clone)]
pub struct ToneLut(Vec<f32>);

impl ToneLut {
    pub fn new(exposure: f32) -> Self {
        Self((0..LUT_SIZE).map(|i| tone_map(i as f32 * LUT_MAX / (LUT_SIZE - 1) as f32, exposure)).collect())
    }

    #[inline(always)]
    pub fn get(&self, radiance: f32) -> f32 {
        let i = (radiance.max(0.0) * ((LUT_SIZE - 1) as f32 / LUT_MAX) + 0.5) as usize;
        self.0[i.min(LUT_SIZE - 1)]
    }
}

/// Light the optimized shader stands in for the path tracer's bounce with:
/// sky and other solids lighting a face from all round, as radiance.
pub const FILL: f32 = 0.09;
/// Ambient light: share of a face's light the nearby solids can take away
/// at most.
pub const AMBIENT_STRENGTH: f32 = 0.7;
/// Optimized ambient light: the spheres' form factors are small unless the
/// solids nearly touch, so they count this many times over.
pub const AMBIENT_GAIN: f32 = 2.5;
/// Raymarch ambient light: how far the short rays reach, as a share of the
/// solid's circumradius, and the most steps each may take.
pub const AO_REACH: f32 = 2.5;
pub const AO_STEPS: u32 = 16;
/// Fraction of a solid's circumradius that blocks light for the optimized
/// ambient light (the spheres sit between the in- and circumradius).
pub const OCCLUDER_SCALE: f32 = 0.85;

/// How brightly the scene's light shines on a face square to it, as
/// radiance: albedo × the light's emission × the solid angle it fills (as
/// the path tracer's next-event estimate works it out).
pub fn direct_scale(scene: &Scene, at: Vec3A, albedo: f32) -> f32 {
    let l = &scene.light;
    let d2 = (l.center - at).length_squared();
    let cos_max = (1.0 - (l.radius * l.radius / d2).min(1.0)).sqrt();
    albedo * l.emission * 2.0 * (1.0 - cos_max)
}

/// How much of the view from face centre `p` (outward normal `n`) the
/// spheres `occluders` (centre, radius) fill, cosine-weighted, 0..=1: each
/// sphere's form factor, (r / d)² × its facing.
#[inline]
pub fn sphere_occlusion(p: Vec3A, n: Vec3A, occluders: impl Iterator<Item = (Vec3A, f32)>) -> f32 {
    let mut occ = 0.0;
    for (c, r) in occluders {
        let d = c - p;
        let d2 = d.length_squared().max(r * r);
        let facing = n.dot(d);
        if facing > 0.0 {
            occ += facing / d2.sqrt() * (r * r / d2);
        }
    }
    occ.min(1.0)
}

/// How far the occlusion rays lean off the normal (tan of the angle: ~58°,
/// where a cosine-weighted hemisphere sample's median ray lies further out
/// than straight up).
const TILT: f32 = 1.6;

/// Directions for `rays` short occlusion rays round normal `n`, the ring
/// turned by `phase` (radians) so neighbouring cells sample differently.
fn ao_dirs(n: Vec3A, rays: usize, phase: f32) -> [Vec3A; 3] {
    let (t, b) = onb(n);
    let tilt = |a: f32, k: f32| (n + (t * a.cos() + b * a.sin()) * k).normalize();
    match rays {
        0 | 1 => [n; 3],
        2 => [tilt(phase, TILT), tilt(phase + std::f32::consts::PI, TILT), n],
        _ => {
            let third = std::f32::consts::TAU / 3.0;
            [tilt(phase, TILT), tilt(phase + third, TILT), tilt(phase + 2.0 * third, TILT)]
        }
    }
}

/// Raymarch ambient light: how occluded the surface of solid `id` at `p` is,
/// 0..=1, from `rays` (1 to 3) short rays cast off it, each counting the
/// nearer what it hits the more. `phase` turns the rays (dither).
pub fn ray_occlusion(scene: &Scene, id: u8, p: Vec3A, rays: usize, phase: f32) -> f32 {
    let s = &scene.solids[id as usize];
    let n = normal(scene, id, p);
    let reach = AO_REACH * s.scale;
    let origin = p + n * (2e-3 * s.scale.max(0.1));
    let rays = rays.clamp(1, 3);
    let dirs = ao_dirs(n, rays, phase);
    let mut occ = 0.0;
    for &d in &dirs[..rays] {
        if let Some(h) = march_solids(scene, origin, d, 0.0, reach, AO_STEPS) {
            occ += 1.0 - h.t / reach;
        }
    }
    occ / rays as f32
}

/// One rasterized face's shading: its continuous luminosity, and the two
/// glyph levels either side of it with their colours, the upper one used
/// in cells whose dither threshold is below `frac`.
#[derive(Debug, Clone, Copy)]
pub struct FaceShade {
    pub lum: f32,
    pub glyph: [char; 2],
    pub color: [u32; 2],
    pub frac: f32,
}

impl FaceShade {
    /// Shading for luminosity `lum` on `ramp`, coloured by `color(level
    /// brightness 0..=1)`. Solids never show the ramp's blank.
    pub fn new(lum: f32, ramp: &Ramp, color: impl Fn(f32) -> u32) -> Self {
        let levels = ramp.len();
        let top = (levels - 1) as f32;
        // levels sit at the middle of their buckets
        let x = (lum.clamp(0.0, 1.0) * levels as f32 - 0.5).clamp(0.0, top);
        let lo = x.floor();
        let hi = (lo + 1.0).min(top);
        let glyph = |l: f32| match ramp.glyph((l + 0.5) / levels as f32) {
            ' ' => '.',
            g => g,
        };
        Self { lum, glyph: [glyph(lo), glyph(hi)], color: [color(lo / top), color(hi / top)], frac: x - lo }
    }

    /// The glyph and colour for cell (`x`, `y`).
    #[inline(always)]
    pub fn at(&self, x: usize, y: usize) -> (char, u32) {
        let up = (bayer(x, y) < self.frac) as usize;
        (self.glyph[up], self.color[up])
    }
}

/// Per cell: the nearest front face's index into the frame's face list.
pub const NO_FACE: u32 = u32::MAX;

/// The optimized shader's G-buffer: what [`platomon::render::FrameView`]
/// gives for the path tracer, plus the face in each cell.
#[derive(Debug, Default)]
pub struct Raster {
    pub depth: Vec<f32>,
    pub ids: Vec<u8>,
    pub face: Vec<u32>,
}

impl Raster {
    pub fn clear(&mut self, cells: usize) {
        self.depth.clear();
        self.depth.resize(cells, f32::INFINITY);
        self.ids.clear();
        self.ids.resize(cells, NO_HIT);
        self.face.clear();
        self.face.resize(cells, NO_FACE);
    }

    /// Fill convex polygon `pts` (screen positions of a front face whose
    /// plane passes through `p0` with outward normal `n`) as face `face` of
    /// solid `id`, nearest-first by distance along each cell's centre ray.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(&mut self, proj: &Projection, (cols, rows): (usize, usize), pts: &[(f32, f32)], p0: Vec3A, n: Vec3A, id: u8, face: u32) {
        let (mut x0, mut x1, mut y0, mut y1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
        for p in pts {
            (x0, x1, y0, y1) = (x0.min(p.0), x1.max(p.0), y0.min(p.1), y1.max(p.1));
        }
        // cells whose centres can be inside
        let cx0 = (x0 - 0.5).ceil().max(0.0) as usize;
        let cy0 = (y0 - 0.5).ceil().max(0.0) as usize;
        let cx1 = ((x1 - 0.5).floor() + 1.0).clamp(0.0, cols as f32) as usize;
        let cy1 = ((y1 - 0.5).floor() + 1.0).clamp(0.0, rows as f32) as usize;
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        // edge functions: a front face winds one way on screen, so a cell is
        // inside when it is on the same side of every edge
        let m = pts.len();
        let mut edges = [(0.0f32, 0.0f32, 0.0f32); 12];
        let mut sign = 0.0;
        for i in 0..m.min(12) {
            let (a, b) = (pts[i], pts[(i + 1) % m]);
            let (ex, ey) = (b.0 - a.0, b.1 - a.1);
            edges[i] = (-ey, ex, ey * a.0 - ex * a.1);
            sign += ex * (b.1 + a.1);
        }
        let s = if sign < 0.0 { 1.0 } else { -1.0 };
        let edges = &edges[..m.min(12)];
        let eye = proj.cam.pos;
        let plane = n.dot(p0 - eye);
        for y in cy0..cy1 {
            let py = y as f32 + 0.5;
            for x in cx0..cx1 {
                let px = x as f32 + 0.5;
                if edges.iter().any(|&(a, b, c)| (a * px + b * py + c) * s < 0.0) {
                    continue;
                }
                let denom = n.dot(proj.ray(px, py));
                if denom >= -1e-6 {
                    continue;
                }
                let t = plane / denom;
                let k = y * cols + x;
                if t > 0.0 && t < self.depth[k] {
                    self.depth[k] = t;
                    self.ids[k] = id;
                    self.face[k] = face;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_table_matches_the_tone_map() {
        let lut = ToneLut::new(1.0);
        for r in [0.0, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 15.9] {
            assert!((lut.get(r) - tone_map(r, 1.0)).abs() < 0.01, "{r}");
        }
        assert!(lut.get(100.0) > 0.95 && lut.get(-1.0) == 0.0);
    }

    #[test]
    fn faces_dither_between_two_levels_in_proportion() {
        let ramp = Ramp::default();
        let n = ramp.len() as f32;
        // a luminosity a quarter of the way from one level to the next
        let lum = (6.0 + 0.5 + 0.25) / n;
        let f = FaceShade::new(lum, &ramp, |b| (b * 255.0) as u32);
        assert!((f.frac - 0.25).abs() < 1e-4);
        assert_ne!(f.glyph[0], f.glyph[1]);
        let up = (0..4).flat_map(|y| (0..4).map(move |x| (x, y))).filter(|&(x, y)| f.at(x, y).0 == f.glyph[1]).count();
        assert_eq!(up, 4, "a quarter of each 4x4 block takes the upper level");
        // the ends never dither past the ramp, and solids never go blank
        let dark = FaceShade::new(0.0, &ramp, |_| 0);
        assert!(dark.glyph.iter().all(|&g| g == '.') && dark.frac == 0.0);
        let lit = FaceShade::new(1.0, &ramp, |_| 0);
        assert_eq!(lit.glyph, ['@', '@']);
    }

    #[test]
    fn nearer_and_bigger_spheres_in_front_occlude_more() {
        let (p, n) = (Vec3A::ZERO, Vec3A::Y);
        let one = |c: Vec3A, r: f32| sphere_occlusion(p, n, std::iter::once((c, r)));
        assert_eq!(one(Vec3A::new(0.0, -3.0, 0.0), 1.0), 0.0, "behind the face: nothing");
        assert!(one(Vec3A::new(0.0, 2.0, 0.0), 1.0) > one(Vec3A::new(0.0, 4.0, 0.0), 1.0));
        assert!(one(Vec3A::new(0.0, 3.0, 0.0), 1.5) > one(Vec3A::new(0.0, 3.0, 0.0), 1.0));
        assert!(one(Vec3A::new(0.0, 3.0, 0.0), 1.0) > one(Vec3A::new(3.0, 0.5, 0.0), 1.0), "straight ahead counts most");
        assert!(sphere_occlusion(p, n, (0..20).map(|_| (Vec3A::new(0.0, 1.0, 0.0), 1.0))) <= 1.0);
    }

    #[test]
    fn ao_rays_leave_the_surface() {
        for rays in 1..=3 {
            for n in [Vec3A::Y, Vec3A::new(0.3, -0.5, 0.8).normalize()] {
                for &d in &ao_dirs(n, rays, 1.0)[..rays] {
                    assert!(d.dot(n) > 0.45 && (d.length() - 1.0).abs() < 1e-4, "{rays} rays: {d:?}");
                }
            }
        }
    }
}
