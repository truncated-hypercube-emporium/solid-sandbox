//! Monochrome path tracer: Lambertian surfaces, cosine-weighted bounces,
//! next-event estimation toward the spherical area light.

use glam::Vec3A;
use std::f32::consts::TAU;

use super::march::{GROUND, Hit, march, march_solids, normal};
use crate::scene::Scene;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceSettings {
    /// Indirect bounces after the primary hit.
    pub bounces: u32,
    pub max_steps: u32,
    pub max_dist: f32,
}

impl Default for TraceSettings {
    fn default() -> Self {
        Self { bounces: 2, max_steps: 64, max_dist: 50.0 }
    }
}

/// Offset along the normal for secondary-ray origins.
const RAY_OFFSET: f32 = 2e-3;
/// Rough sky-visibility factor for paths truncated at the last bounce.
const AMBIENT_OCCLUSION: f32 = 0.5;

/// Orthonormal basis around unit `n` (Duff et al. 2017).
#[inline(always)]
pub fn onb(n: Vec3A) -> (Vec3A, Vec3A) {
    let s = 1.0f32.copysign(n.z);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    (Vec3A::new(1.0 + s * n.x * n.x * a, s * b, -s * n.x), Vec3A::new(b, s + n.y * n.y * a, -n.y))
}

#[inline(always)]
pub fn cosine_hemisphere(n: Vec3A, rng: &mut fastrand::Rng) -> Vec3A {
    let u1 = rng.f32();
    let u2 = rng.f32();
    let r = u1.sqrt();
    let (s, c) = (TAU * u2).sin_cos();
    let (t, b) = onb(n);
    (t * (r * c) + b * (r * s) + n * (1.0 - u1).max(0.0).sqrt()).normalize()
}

/// Distance along the ray to the light sphere, if hit.
#[inline(always)]
fn light_hit(scene: &Scene, ro: Vec3A, rd: Vec3A) -> Option<f32> {
    let l = &scene.light;
    let oc = ro - l.center;
    let b = oc.dot(rd);
    let c = oc.length_squared() - l.radius * l.radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

/// Direct light at `p` (normal `n`) by solid-angle sampling of the light
/// sphere. Returns radiance / albedo.
#[inline]
fn direct_light(scene: &Scene, p: Vec3A, n: Vec3A, rng: &mut fastrand::Rng, s: &TraceSettings) -> f32 {
    let l = &scene.light;
    let to_c = l.center - p;
    let d2 = to_c.length_squared();
    let d = d2.sqrt();
    let w = to_c / d;
    let sin2_max = (l.radius * l.radius / d2).min(1.0);
    let cos_max = (1.0 - sin2_max).sqrt();

    let u1 = rng.f32();
    let u2 = rng.f32();
    let cos_t = 1.0 - u1 * (1.0 - cos_max);
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let (sp, cp) = (TAU * u2).sin_cos();
    let (t, b) = onb(w);
    let dir = t * (cp * sin_t) + b * (sp * sin_t) + w * cos_t;

    let ndl = n.dot(dir);
    if ndl <= 0.0 {
        return 0.0;
    }
    // Light is above the ground, so only solids can occlude it.
    if march_solids(scene, p, dir, 0.0, d - l.radius, s.max_steps).is_some() {
        return 0.0;
    }
    // (albedo/π)·L·cosθ / pdf, pdf = 1 / (2π(1 − cosθmax)); albedo applied by caller
    l.emission * ndl * 2.0 * (1.0 - cos_max)
}

/// Radiance along one camera ray.
pub fn radiance(scene: &Scene, ro: Vec3A, rd: Vec3A, rng: &mut fastrand::Rng, s: &TraceSettings) -> f32 {
    radiance_primary(scene, ro, rd, rng, s).0
}

/// Radiance along one camera ray, plus the primary hit (`None` when the ray
/// escapes or sees the light first).
pub fn radiance_primary(
    scene: &Scene,
    mut ro: Vec3A,
    mut rd: Vec3A,
    rng: &mut fastrand::Rng,
    s: &TraceSettings,
) -> (f32, Option<Hit>) {
    let mut acc = 0.0f32;
    let mut throughput = 1.0f32;
    let mut primary = None;

    for depth in 0..=s.bounces {
        let hit = march(scene, ro, rd, s.max_dist, s.max_steps);

        if depth == 0 {
            if let Some(tl) = light_hit(scene, ro, rd)
                && hit.is_none_or(|h| tl < h.t)
            {
                return (scene.light.emission, None);
            }
            primary = hit;
        }

        let Some(h) = hit else {
            acc += throughput * if depth == 0 { scene.background } else { scene.sky };
            break;
        };

        let p = ro + rd * h.t;
        let n = normal(scene, h.id, p);
        let albedo = if h.id == GROUND { scene.ground.albedo(p, h.t) } else { scene.solids[h.id as usize].albedo };
        let origin = p + n * RAY_OFFSET;

        acc += throughput * albedo * direct_light(scene, origin, n, rng, s);

        if depth == s.bounces {
            // Truncated path: approximate the remaining sky light.
            acc += throughput * albedo * scene.sky * AMBIENT_OCCLUSION;
            break;
        }
        throughput *= albedo;
        ro = origin;
        rd = cosine_hemisphere(n, rng);
    }
    (acc, primary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onb_is_orthonormal() {
        for n in [Vec3A::Y, -Vec3A::Z, Vec3A::new(0.3, -0.5, 0.8).normalize(), Vec3A::Z] {
            let (t, b) = onb(n);
            assert!(t.dot(n).abs() < 1e-5 && b.dot(n).abs() < 1e-5 && t.dot(b).abs() < 1e-5);
            assert!((t.length() - 1.0).abs() < 1e-5 && (b.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn hemisphere_samples_face_normal() {
        let mut rng = fastrand::Rng::with_seed(7);
        let n = Vec3A::new(0.2, 0.9, -0.1).normalize();
        let mut mean_cos = 0.0;
        for _ in 0..20_000 {
            let d = cosine_hemisphere(n, &mut rng);
            let c = d.dot(n);
            assert!(c >= -1e-5);
            mean_cos += c;
        }
        // E[cosθ] for cosine-weighted sampling = 2/3
        assert!((mean_cos / 20_000.0 - 2.0 / 3.0).abs() < 0.01);
    }
}
