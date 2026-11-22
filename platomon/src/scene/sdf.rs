//! Convex-polyhedron signed distance functions.
//!
//! Every solid is an intersection of half-spaces: `sdf(p) = maxᵢ (p·n̂ᵢ − dᵢ)`,
//! normalised to circumradius 1. Centrally symmetric solids store one normal
//! per opposite-face pair and use `|p·n̂| − d`.
//!
//! The five Platonic solids are joined by seven Archimedean solids (built by
//! truncating or rectifying a Platonic solid) and the rhombic dodecahedron,
//! rhombic triacontahedron and rhombicuboctahedron. Vertices are not
//! hard-coded: they are recovered from the face planes, which is also how
//! the circumradius used for normalisation is found.

use glam::{Mat3A, Vec3A};

pub const PHI: f32 = std::f32::consts::GOLDEN_RATIO;
const INV_PHI: f32 = PHI - 1.0;
const INV_SQRT3: f32 = 0.577_350_26;
const SQRT2: f32 = std::f32::consts::SQRT_2;
/// Inradius of a dodecahedron / icosahedron with circumradius 1.
const R_IN_DI: f32 = 0.794_654_5;
/// cos of the angle between adjacent vertices (circumradius 1).
const COS_ADJ_CUBE: f32 = 1.0 / 3.0;
const COS_ADJ_OCTA: f32 = 0.0;
const COS_ADJ_DODECA: f32 = 0.745_356; // √5 / 3
const COS_ADJ_ICOSA: f32 = 0.447_213_6; // 1 / √5
/// Fraction of each edge removed at both ends so a truncated p-gon becomes a
/// regular 2p-gon: 1 / (2 + 2·sin(interior/2)).
const CUT_TRI: f32 = 1.0 / 3.0;
const CUT_SQUARE: f32 = 0.292_893_2; // 1 / (2 + √2)
const CUT_PENTA: f32 = 0.276_393_2; // 1 / (2 + 2·sin 54°)

pub const MAX_AXES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SolidKind {
    Tetrahedron,
    Cube,
    Octahedron,
    Dodecahedron,
    Icosahedron,
    TruncatedTetrahedron,
    Cuboctahedron,
    TruncatedCube,
    TruncatedOctahedron,
    Rhombicuboctahedron,
    Icosidodecahedron,
    TruncatedIcosahedron,
    TruncatedDodecahedron,
    RhombicDodecahedron,
    RhombicTriacontahedron,
}

impl SolidKind {
    pub const ALL: [SolidKind; 15] = [
        SolidKind::Tetrahedron,
        SolidKind::Cube,
        SolidKind::Octahedron,
        SolidKind::Dodecahedron,
        SolidKind::Icosahedron,
        SolidKind::TruncatedTetrahedron,
        SolidKind::Cuboctahedron,
        SolidKind::TruncatedCube,
        SolidKind::TruncatedOctahedron,
        SolidKind::Rhombicuboctahedron,
        SolidKind::Icosidodecahedron,
        SolidKind::TruncatedIcosahedron,
        SolidKind::TruncatedDodecahedron,
        SolidKind::RhombicDodecahedron,
        SolidKind::RhombicTriacontahedron,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SolidKind::Tetrahedron => "tetrahedron",
            SolidKind::Cube => "cube",
            SolidKind::Octahedron => "octahedron",
            SolidKind::Dodecahedron => "dodecahedron",
            SolidKind::Icosahedron => "icosahedron",
            SolidKind::TruncatedTetrahedron => "truncated-tetrahedron",
            SolidKind::Cuboctahedron => "cuboctahedron",
            SolidKind::TruncatedCube => "truncated-cube",
            SolidKind::TruncatedOctahedron => "truncated-octahedron",
            SolidKind::Rhombicuboctahedron => "rhombicuboctahedron",
            SolidKind::Icosidodecahedron => "icosidodecahedron",
            SolidKind::TruncatedIcosahedron => "truncated-icosahedron",
            SolidKind::TruncatedDodecahedron => "truncated-dodecahedron",
            SolidKind::RhombicDodecahedron => "rhombic-dodecahedron",
            SolidKind::RhombicTriacontahedron => "rhombic-triacontahedron",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace('_', "-");
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Expected (faces, vertices).
    pub fn counts(self) -> (usize, usize) {
        match self {
            SolidKind::Tetrahedron => (4, 4),
            SolidKind::Cube => (6, 8),
            SolidKind::Octahedron => (8, 6),
            SolidKind::Dodecahedron => (12, 20),
            SolidKind::Icosahedron => (20, 12),
            SolidKind::TruncatedTetrahedron => (8, 12),
            SolidKind::Cuboctahedron => (14, 12),
            SolidKind::TruncatedCube => (14, 24),
            SolidKind::TruncatedOctahedron => (14, 24),
            SolidKind::Rhombicuboctahedron => (26, 24),
            SolidKind::Icosidodecahedron => (32, 30),
            SolidKind::TruncatedIcosahedron => (32, 60),
            SolidKind::TruncatedDodecahedron => (32, 60),
            SolidKind::RhombicDodecahedron => (12, 14),
            SolidKind::RhombicTriacontahedron => (30, 32),
        }
    }

    /// Platonic or Archimedean: every vertex lies on the circumsphere.
    pub fn vertex_uniform(self) -> bool {
        !matches!(self, SolidKind::RhombicDodecahedron | SolidKind::RhombicTriacontahedron)
    }
}

fn v(x: f32, y: f32, z: f32) -> Vec3A {
    Vec3A::new(x, y, z).normalize()
}

/// Cyclic permutations (x,y,z), (z,x,y), (y,z,x).
fn cyclic(a: f32, b: f32, c: f32) -> [Vec3A; 3] {
    [v(a, b, c), v(c, a, b), v(b, c, a)]
}

/// Tetrahedron face normals.
fn set_t() -> Vec<Vec3A> {
    vec![v(1., 1., 1.), v(1., -1., -1.), v(-1., 1., -1.), v(-1., -1., 1.)]
}
/// Cube face normals (= octahedron vertex directions).
fn set_c() -> Vec<Vec3A> {
    vec![Vec3A::X, Vec3A::Y, Vec3A::Z]
}
/// Octahedron face normals (= cube vertex directions), one per ± pair.
fn set_o() -> Vec<Vec3A> {
    vec![v(1., 1., 1.), v(1., 1., -1.), v(1., -1., 1.), v(-1., 1., 1.)]
}
/// (1,1,0)-type directions (cube edge midpoints), one per ± pair.
fn set_e() -> Vec<Vec3A> {
    vec![v(1., 1., 0.), v(1., -1., 0.), v(1., 0., 1.), v(1., 0., -1.), v(0., 1., 1.), v(0., 1., -1.)]
}
/// Dodecahedron face normals (= icosahedron vertex directions).
fn set_d() -> Vec<Vec3A> {
    let mut s = cyclic(0., 1., PHI).to_vec();
    s.extend(cyclic(0., -1., PHI));
    s
}
/// Icosahedron face normals (= dodecahedron vertex directions).
fn set_i() -> Vec<Vec3A> {
    let mut s = set_o();
    s.extend(cyclic(0., PHI, INV_PHI));
    s.extend(cyclic(0., -PHI, INV_PHI));
    s
}
/// Rhombic triacontahedron face normals (= icosahedron edge midpoints).
fn set_rt() -> Vec<Vec3A> {
    let mut s = set_c();
    for (sa, sb, sc) in [(1., 1., 1.), (1., 1., -1.), (1., -1., 1.), (-1., 1., 1.)] {
        s.extend(cyclic(sa * PHI, sb * 1.0, sc * INV_PHI));
    }
    s
}

/// Face planes before normalisation: (normals, offsets, centrally symmetric).
fn planes(kind: SolidKind) -> (Vec<Vec3A>, Vec<f32>, bool) {
    let mut n: Vec<Vec3A> = Vec::new();
    let mut d: Vec<f32> = Vec::new();
    let mut add = |set: Vec<Vec3A>, off: f32| {
        d.extend(std::iter::repeat_n(off, set.len()));
        n.extend(set);
    };
    // Truncating/rectifying a solid with circumradius 1 adds planes along its
    // vertex directions at 1 + f·(cos_adj − 1), f = fraction of edge cut.
    let cut = |cos_adj: f32, f: f32| 1.0 + f * (cos_adj - 1.0);
    let sym = match kind {
        SolidKind::Tetrahedron => {
            add(set_t(), 1.0 / 3.0);
            false
        }
        SolidKind::Cube => {
            add(set_c(), INV_SQRT3);
            true
        }
        SolidKind::Octahedron => {
            add(set_o(), INV_SQRT3);
            true
        }
        SolidKind::Dodecahedron => {
            add(set_d(), R_IN_DI);
            true
        }
        SolidKind::Icosahedron => {
            add(set_i(), R_IN_DI);
            true
        }
        SolidKind::TruncatedTetrahedron => {
            add(set_t(), 1.0 / 3.0);
            add(set_t().into_iter().map(|a| -a).collect(), cut(-1.0 / 3.0, CUT_TRI));
            false
        }
        SolidKind::Cuboctahedron => {
            add(set_c(), INV_SQRT3);
            add(set_o(), cut(COS_ADJ_CUBE, 0.5));
            true
        }
        SolidKind::TruncatedCube => {
            add(set_c(), INV_SQRT3);
            add(set_o(), cut(COS_ADJ_CUBE, CUT_SQUARE));
            true
        }
        SolidKind::TruncatedOctahedron => {
            add(set_o(), INV_SQRT3);
            add(set_c(), cut(COS_ADJ_OCTA, CUT_TRI));
            true
        }
        SolidKind::Rhombicuboctahedron => {
            // vertices: all permutations of (±1, ±1, ±(1+√2))
            add(set_c(), 1.0 + SQRT2);
            add(set_o(), (3.0 + SQRT2) * INV_SQRT3);
            add(set_e(), (2.0 + SQRT2) / SQRT2);
            true
        }
        SolidKind::Icosidodecahedron => {
            add(set_d(), R_IN_DI);
            add(set_i(), cut(COS_ADJ_DODECA, 0.5));
            true
        }
        SolidKind::TruncatedIcosahedron => {
            add(set_i(), R_IN_DI);
            add(set_d(), cut(COS_ADJ_ICOSA, CUT_TRI));
            true
        }
        SolidKind::TruncatedDodecahedron => {
            add(set_d(), R_IN_DI);
            add(set_i(), cut(COS_ADJ_DODECA, CUT_PENTA));
            true
        }
        SolidKind::RhombicDodecahedron => {
            add(set_e(), 1.0);
            true
        }
        SolidKind::RhombicTriacontahedron => {
            add(set_rt(), 1.0);
            true
        }
    };
    (n, d, sym)
}

/// All planes, with the negated twin of each symmetric axis.
fn full_planes(n: &[Vec3A], d: &[f32], sym: bool) -> Vec<(Vec3A, f32)> {
    let mut out: Vec<(Vec3A, f32)> = n.iter().copied().zip(d.iter().copied()).collect();
    if sym {
        out.extend(n.iter().zip(d).map(|(a, o)| (-*a, *o)));
    }
    out
}

/// Vertices of the convex region `p·nᵢ ≤ dᵢ` (intersection of plane triples).
fn enumerate_vertices(pl: &[(Vec3A, f32)]) -> Vec<Vec3A> {
    let scale = pl.iter().map(|p| p.1.abs()).fold(0.0f32, f32::max).max(1e-6);
    let tol = 1e-4 * scale;
    let mut out: Vec<Vec3A> = Vec::new();
    for i in 0..pl.len() {
        for j in i + 1..pl.len() {
            for k in j + 1..pl.len() {
                let (a, b, c) = (pl[i], pl[j], pl[k]);
                let bc = b.0.cross(c.0);
                let det = a.0.dot(bc);
                if det.abs() < 1e-5 {
                    continue;
                }
                let x = (bc * a.1 + c.0.cross(a.0) * b.1 + a.0.cross(b.0) * c.1) / det;
                if pl.iter().all(|(n, o)| x.dot(*n) <= o + tol) && !out.iter().any(|q| (*q - x).length() < 10.0 * tol)
                {
                    out.push(x);
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct Polyhedron {
    kind: SolidKind,
    axes: [Vec3A; MAX_AXES],
    offs: [f32; MAX_AXES],
    n: usize,
    symmetric: bool,
    inradius: f32,
}

impl Polyhedron {
    pub fn new(kind: SolidKind) -> Self {
        let (normals, offsets, symmetric) = planes(kind);
        assert!(normals.len() <= MAX_AXES, "{kind:?}: too many axes");
        let verts = enumerate_vertices(&full_planes(&normals, &offsets, symmetric));
        let circum = verts.iter().map(|p| p.length()).fold(0.0f32, f32::max).max(1e-6);
        let mut axes = [Vec3A::ZERO; MAX_AXES];
        let mut offs = [0.0f32; MAX_AXES];
        axes[..normals.len()].copy_from_slice(&normals);
        for (o, d) in offs.iter_mut().zip(&offsets) {
            *o = d / circum;
        }
        let n = normals.len();
        let inradius = offs[..n].iter().copied().fold(f32::INFINITY, f32::min);
        Self { kind, axes, offs, n, symmetric, inradius }
    }

    pub fn kind(&self) -> SolidKind {
        self.kind
    }

    /// Distance from the centre to the nearest face.
    pub fn inradius(&self) -> f32 {
        self.inradius
    }

    /// All face planes (normal, offset), both members of symmetric pairs.
    pub fn face_planes(&self) -> Vec<(Vec3A, f32)> {
        full_planes(&self.axes[..self.n], &self.offs[..self.n], self.symmetric)
    }

    /// Vertices at circumradius 1.
    pub fn vertices(&self) -> Vec<Vec3A> {
        enumerate_vertices(&self.face_planes())
    }

    /// Edges as `(vertex a, vertex b, face 1, face 2)`, indexing
    /// [`vertices`](Self::vertices) and [`face_planes`](Self::face_planes):
    /// two vertices form an edge exactly when they share two faces.
    pub fn edges(&self) -> Vec<(usize, usize, usize, usize)> {
        let planes = self.face_planes();
        let verts = self.vertices();
        let on: Vec<Vec<usize>> = verts
            .iter()
            .map(|v| (0..planes.len()).filter(|&f| (v.dot(planes[f].0) - planes[f].1).abs() < 1e-3).collect())
            .collect();
        let mut out = Vec::new();
        for a in 0..verts.len() {
            for b in a + 1..verts.len() {
                let shared: Vec<usize> = on[a].iter().copied().filter(|f| on[b].contains(f)).collect();
                if shared.len() >= 2 {
                    out.push((a, b, shared[0], shared[1]));
                }
            }
        }
        out
    }

    /// Signed distance (a bound outside near edges/vertices) for circumradius 1.
    #[inline(always)]
    pub fn eval(&self, p: Vec3A) -> f32 {
        match self.kind {
            SolidKind::Cube => p.abs().max_element() - self.offs[0],
            SolidKind::Octahedron => p.abs().element_sum() * INV_SQRT3 - self.offs[0],
            _ => {
                let axes = &self.axes[..self.n];
                let offs = &self.offs[..self.n];
                let mut m = f32::MIN;
                if self.symmetric {
                    for (a, o) in axes.iter().zip(offs) {
                        m = m.max(p.dot(*a).abs() - o);
                    }
                } else {
                    for (a, o) in axes.iter().zip(offs) {
                        m = m.max(p.dot(*a) - o);
                    }
                }
                m
            }
        }
    }
}

/// Rotation R = Ry(yaw) · Rx(pitch); returns R⁻¹ = Rᵀ.
pub fn inverse_rotation(pitch: f32, yaw: f32) -> Mat3A {
    (Mat3A::from_rotation_y(yaw) * Mat3A::from_rotation_x(pitch)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn ids_roundtrip() {
        for k in SolidKind::ALL {
            assert_eq!(SolidKind::from_id(k.id()), Some(k));
        }
        assert_eq!(SolidKind::from_id("Rhombic_Dodecahedron"), Some(SolidKind::RhombicDodecahedron));
        assert_eq!(SolidKind::from_id("sphere"), None);
    }

    #[test]
    fn face_and_vertex_counts() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            let (faces, verts) = k.counts();
            assert_eq!(p.face_planes().len(), faces, "{k:?} faces");
            assert_eq!(p.vertices().len(), verts, "{k:?} vertices");
        }
    }

    #[test]
    fn circumradius_is_one() {
        for k in SolidKind::ALL {
            let vs = Polyhedron::new(k).vertices();
            let max = vs.iter().map(|q| q.length()).fold(0.0, f32::max);
            assert!((max - 1.0).abs() < EPS, "{k:?} R={max}");
            if k.vertex_uniform() {
                for q in &vs {
                    assert!((q.length() - 1.0).abs() < 2e-4, "{k:?} non-uniform vertex {q:?}");
                }
            }
        }
    }

    #[test]
    fn uniform_edge_lengths_for_archimedean() {
        // Regular faces ⇒ every edge has the same length; compare each
        // vertex's nearest-neighbour distance.
        for k in SolidKind::ALL.into_iter().filter(|k| k.vertex_uniform()) {
            let vs = Polyhedron::new(k).vertices();
            let nn: Vec<f32> = vs
                .iter()
                .map(|a| vs.iter().filter(|b| *b != a).map(|b| (*a - *b).length()).fold(f32::MAX, f32::min))
                .collect();
            let (lo, hi) = nn.iter().fold((f32::MAX, 0.0f32), |(l, h), &x| (l.min(x), h.max(x)));
            assert!(hi - lo < 2e-3, "{k:?} edges {lo}..{hi}");
        }
    }

    #[test]
    fn edges_satisfy_euler() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            let (f, v) = k.counts();
            let e = p.edges();
            assert_eq!(e.len(), v + f - 2, "{k:?}: V - E + F = 2");
            let vs = p.vertices();
            let fs = p.face_planes();
            for &(a, b, f1, f2) in &e {
                assert!(a != b && f1 != f2);
                // the edge midpoint lies on the surface
                let m = (vs[a] + vs[b]) * 0.5;
                assert!(p.eval(m).abs() < EPS, "{k:?}");
                assert!((m.dot(fs[f1].0) - fs[f1].1).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn zero_at_vertices() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            for vert in p.vertices() {
                let d = p.eval(vert);
                assert!(d.abs() < EPS, "{k:?} vertex {vert:?} d={d}");
            }
        }
    }

    #[test]
    fn zero_at_face_centers() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            for (n, o) in p.face_planes() {
                let d = p.eval(n * o);
                assert!(d.abs() < EPS, "{k:?} face {n:?} d={d}");
            }
        }
    }

    #[test]
    fn negative_inside_positive_outside() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            assert!((p.eval(Vec3A::ZERO) + p.inradius()).abs() < EPS, "{k:?}");
            assert!(p.eval(Vec3A::new(0.0, 2.0, 0.0)) > 0.0);
            for vert in p.vertices() {
                assert!(p.eval(vert * 0.5) < 0.0);
                assert!(p.eval(vert * 1.5) > 0.0);
            }
        }
    }

    #[test]
    fn bound_never_exceeds_true_distance_for_vertices() {
        for k in SolidKind::ALL {
            let p = Polyhedron::new(k);
            for vert in p.vertices() {
                let q = vert * 1.3;
                assert!(p.eval(q) <= (q - vert).length() + EPS, "{k:?}");
            }
        }
    }

    #[test]
    fn inverse_rotation_is_inverse() {
        let inv = inverse_rotation(0.7, -1.3);
        let r = Mat3A::from_rotation_y(-1.3) * Mat3A::from_rotation_x(0.7);
        assert!((inv * r).abs_diff_eq(Mat3A::IDENTITY, 1e-5));
    }
}
