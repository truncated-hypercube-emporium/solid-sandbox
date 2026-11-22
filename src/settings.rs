//! User settings: each is an index into a fixed list of choices, so the
//! dialog can step through them and the file stores the choice's name.

use platomon::scene::sdf::SolidKind;

use crate::physics::{self, CONTAINERS};

/// One setting: its label and the names of its choices.
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub choices: &'static [&'static str],
}

/// The fields of each dialog, in the order shown: the menu button's
/// "Settings", the cogwheel's "Simulation Parameter" and the colour button's
/// "Colour".
pub const SETTINGS_FIELDS: [usize; 9] = [0, 1, 2, 3, 9, 13, 15, 16, CAMERA_FIELD];
pub const SIMULATION_FIELDS: [usize; 9] = [17, 18, 19, CHAR_SCALE_FIELD, 5, 7, 8, 4, 6];
pub const COLOUR_FIELDS: [usize; 5] = [10, PALETTE_FIELD, 12, 14, GLOW_COLOUR_FIELD];
/// Fields that need a fresh scene when they change (these and
/// [`SPAWN_FIELD`]); the rest apply live.
pub const SCENE_FIELDS: std::ops::Range<usize> = 0..9;
/// How solids spawn: it shows at launch, so it restarts the scene too.
pub const SPAWN_FIELD: usize = 16;
/// The palette field, which has the palette button beside it.
pub const PALETTE_FIELD: usize = 11;
/// Character scale: the size of the scene's text cells.
pub const CHAR_SCALE_FIELD: usize = 20;
/// The dynamic camera, and the glow's colour.
pub const CAMERA_FIELD: usize = 21;
pub const GLOW_COLOUR_FIELD: usize = 22;

pub const FIELDS: [Field; 23] = [
    Field {
        key: "container",
        label: "Container",
        choices: &["Random", "Cube", "Octahedron", "Dodecahedron", "Icosahedron"],
    },
    Field { key: "solids", label: "Solids", choices: &["Random", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23", "24"] },
    Field { key: "wireframe", label: "Wireframe", choices: &["On", "Off"] },
    Field { key: "tap_speed", label: "Tap speed", choices: &["Slow", "Normal", "Fast"] },
    Field { key: "gravity", label: "Gravity", choices: &["Off", "Low", "On"] },
    Field { key: "collisions", label: "Collisions", choices: &["Spheres", "Exact"] },
    Field { key: "mass", label: "Mass variability", choices: &["Off", "Low", "Medium", "High"] },
    Field { key: "elasticity", label: "Elasticity", choices: &["Off", "Low", "High"] },
    Field { key: "damping", label: "Damping", choices: &["Off", "Low", "Medium", "High"] },
    Field { key: "glow", label: "Glow", choices: &["Off", "Low", "Medium", "High"] },
    Field { key: "colour_mode", label: "Colour mode", choices: &["Velocity", "Spin", "Collisions"] },
    Field {
        key: "palette",
        label: "Palette",
        choices: &["Off", "3-bit A", "3-bit B", "4-bit A", "4-bit B", "5-bit A", "5-bit B", "6-bit A", "6-bit B"],
    },
    Field { key: "saturation", label: "Saturation", choices: &["Low", "Medium", "High"] },
    Field { key: "explosions", label: "Explosions", choices: &["Off", "Weak", "Strong"] },
    Field { key: "wire_colour", label: "Wire colour", choices: &["Velocity", "White", "Inverted", "Bit-shift"] },
    Field { key: "charge", label: "Charge", choices: &["Low", "Medium", "High"] },
    Field { key: "spawn", label: "Spawn", choices: &["Grow", "Pop low", "Pop medium", "Pop high"] },
    // Performance page
    Field { key: "sim_speed", label: "Sim speed", choices: &["0.1x", "0.175x", "0.25x", "0.5x", "1x", "1.5x", "2x"] },
    Field { key: "framerate", label: "Framerate", choices: &["15 fps", "20 fps", "30 fps", "45 fps", "60 fps"] },
    Field { key: "stepping", label: "Sim stepping", choices: &["120 Hz", "240 Hz", "480 Hz", "960 Hz"] },
    Field { key: "char_scale", label: "Character scale", choices: &["0.5", "1", "2"] },
    Field { key: "camera", label: "Dynamic camera", choices: &["Off", "On"] },
    Field { key: "glow_colour", label: "Glow colour", choices: &["Inherited", "White"] },
];

/// Text-cell size multiplier per "Character scale".
const CHAR_SCALES: [f32; 3] = [0.5, 1.0, 2.0];

/// How solids glow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glow {
    Off,
    /// Fast solids glow and light their neighbours (the 1.10 glow).
    Low,
    /// Spin counts as well as speed, and the glow reaches up to 40% of a
    /// radius further at full glow.
    Medium,
    /// As Medium, but the light adds to whatever it passes in front of, and
    /// reaches up to 70% of a radius further as the collision counter fills.
    High,
}

/// What explosions do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Explosions {
    Off,
    /// The faces fly out (the 1.10 explosion).
    Weak,
    /// As Weak, plus a shock wave that pushes neighbours out and damages them.
    Strong,
}

/// Saturation scale for "Low", "Medium" (the colours as designed) and "High".
const SATURATION: [f32; 3] = [0.45, 1.0, 1.6];

/// What a solid's colour follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourBy {
    /// Speed, 0 at rest to 1 at tap speed.
    Velocity,
    /// Rate of spin.
    Spin,
    /// Collision counter, 0 to 255.
    Collisions,
}

/// What colour the solids' wireframes are drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireColour {
    /// Grey, brighter the faster the solid moves.
    Velocity,
    /// Plain white.
    White,
    /// The inverse of the colour underneath.
    Inverted,
    /// The colour underneath with its palette index rotated by half the
    /// palette's bit depth (half of 24 bits without a palette).
    BitShift,
}

/// Collision growth per "Charge": "Low" shows each step 100 ms after its
/// impact, "Medium" 50 ms, "High" at once and 10% bigger.
const CHARGES: [physics::Charge; 3] = [
    physics::Charge { delay: 0.1, boost: 1.0 },
    physics::Charge::MEDIUM,
    physics::Charge { delay: 0.0, boost: 1.1 },
];
/// Spawn size for "Pop low", "Pop medium" and "Pop high".
const POP_SIZES: [f32; 3] = [0.1, 0.3, 0.6];

/// Simulated seconds per real second for each "Sim speed".
const SIM_SPEEDS: [f32; 7] = [0.1, 0.175, 0.25, 0.5, 1.0, 1.5, 2.0];
/// Frame-rate caps.
const FRAMERATES: [f32; 5] = [15.0, 20.0, 30.0, 45.0, 60.0];
/// Physics sub-steps per simulated second for each "Sim stepping".
const STEP_RATES: [f32; 4] = [120.0, physics::STEP_HZ, 480.0, 960.0];

/// Tap speeds for "Slow", "Normal" and "Fast", world units per second.
const TAP_SPEEDS: [f32; 3] = [6.0, physics::TAP_SPEED, 13.0];
/// Share of the accelerometer's pull for gravity "Off", "Low" (85% less) and "On".
const GRAVITY_SCALES: [f32; 3] = [0.0, 0.15, 1.0];
/// Lightest solid's density relative to the heaviest, per mass variability.
const LIGHTEST: [f32; 4] = [1.0, 0.5, 0.2, 0.05];
/// Restitution for elasticity "Off" (dead stop), "Low" and "High".
const ELASTICITY: [f32; 3] = [0.0, 0.6, physics::RESTITUTION];
/// Share of the impulse two solids exchange for damping "Off", "Low",
/// "Medium", "High".
const DAMPING: [f32; 4] = [1.0, 0.75, 0.62, 0.5];
/// And the drag that goes with each: air drag on speed and on spin (per
/// second), and the share of spin each counted impact takes away.
const DRAGS: [physics::Drag; 4] = [
    physics::Drag::NONE,
    physics::Drag { linear: 0.04, angular: 0.25, impact_spin: 0.15 },
    physics::Drag { linear: 0.1, angular: 0.6, impact_spin: 0.3 },
    physics::Drag { linear: 0.2, angular: 1.2, impact_spin: 0.5 },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Choice index per entry of [`FIELDS`].
    pub pick: [usize; FIELDS.len()],
}

impl Default for Settings {
    /// Random container, random count, wireframe on, normal tap speed, no
    /// gravity, sphere collisions, equal masses, high elasticity, no
    /// damping, no glow, colour by velocity in the solids' own gradients at
    /// medium saturation, explosions on; real-time speed, 30 fps, 240
    /// sub-steps a second.
    fn default() -> Self {
        Self { pick: [0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0] }
    }
}

impl Settings {
    /// Fixed container shape, or `None` for a random one.
    pub fn container(&self) -> Option<SolidKind> {
        self.pick[0].checked_sub(1).map(|i| CONTAINERS[i])
    }

    /// Fixed number of solids, or `None` for a random one.
    pub fn solids(&self) -> Option<usize> {
        self.pick[1].checked_sub(1).map(|i| physics::MIN_SOLIDS + i)
    }

    pub fn wireframe(&self) -> bool {
        self.pick[2] == 0
    }

    pub fn tap_speed(&self) -> f32 {
        TAP_SPEEDS[self.pick[3]]
    }

    /// Share of the accelerometer's pull applied as gravity (0: off).
    pub fn gravity(&self) -> f32 {
        GRAVITY_SCALES[self.pick[4]]
    }

    /// Exact face/corner collisions instead of spheres.
    pub fn exact(&self) -> bool {
        self.pick[5] == 1
    }

    /// Lightest solid's density relative to the heaviest (1: all equal).
    pub fn lightest(&self) -> f32 {
        LIGHTEST[self.pick[6]]
    }

    /// Restitution of contacts.
    pub fn elasticity(&self) -> f32 {
        ELASTICITY[self.pick[7]]
    }

    /// Share of the impulse colliding solids exchange.
    pub fn damping(&self) -> f32 {
        DAMPING[self.pick[8]]
    }

    /// Drag on speed and spin for the damping setting.
    pub fn drag(&self) -> physics::Drag {
        DRAGS[self.pick[8]]
    }

    /// Whether the camera backs off as the solids speed up.
    pub fn dynamic_camera(&self) -> bool {
        self.pick[CAMERA_FIELD] == 1
    }

    /// Whether the glow is white (picking up half the chroma of the solids
    /// it passes over) instead of the glowing solid's colour.
    pub fn white_glow(&self) -> bool {
        self.pick[GLOW_COLOUR_FIELD] == 1
    }

    /// How solids glow.
    pub fn glow(&self) -> Glow {
        [Glow::Off, Glow::Low, Glow::Medium, Glow::High][self.pick[9]]
    }

    /// Text-cell size multiplier (1: as 1.10 sized them).
    pub fn char_scale(&self) -> f32 {
        CHAR_SCALES[self.pick[CHAR_SCALE_FIELD]]
    }

    pub fn colour_by(&self) -> ColourBy {
        [ColourBy::Velocity, ColourBy::Spin, ColourBy::Collisions][self.pick[10]]
    }

    /// Which of the eight palettes the solids are drawn in, or `None` for
    /// their own gradients.
    pub fn palette(&self) -> Option<usize> {
        self.pick[PALETTE_FIELD].checked_sub(1)
    }

    /// Saturation scale (1: as designed).
    pub fn saturation(&self) -> f32 {
        SATURATION[self.pick[12]]
    }

    /// Whether worn-out solids can explode, and how hard.
    pub fn explosions(&self) -> Explosions {
        [Explosions::Off, Explosions::Weak, Explosions::Strong][self.pick[13]]
    }

    pub fn wire_colour(&self) -> WireColour {
        [WireColour::Velocity, WireColour::White, WireColour::Inverted, WireColour::BitShift][self.pick[14]]
    }

    /// How impacts feed collision growth.
    pub fn charge(&self) -> physics::Charge {
        CHARGES[self.pick[15]]
    }

    /// How new solids come in.
    pub fn spawn(&self) -> physics::Spawn {
        match self.pick[SPAWN_FIELD] {
            0 => physics::Spawn::Grow,
            k => physics::Spawn::Pop(POP_SIZES[k - 1]),
        }
    }

    /// Simulated seconds per real second.
    pub fn sim_speed(&self) -> f32 {
        SIM_SPEEDS[self.pick[17]]
    }

    /// Frame-rate cap, frames per second.
    pub fn fps(&self) -> f32 {
        FRAMERATES[self.pick[18]]
    }

    /// Physics sub-steps per simulated second.
    pub fn step_hz(&self) -> f32 {
        STEP_RATES[self.pick[19]]
    }

    /// Whether `other` differs only in settings that apply live (colours,
    /// glow, explosions, charge, the Simulation Parameter page's performance
    /// options), so no restart is needed.
    pub fn same_scene(&self, other: &Self) -> bool {
        self.pick[SCENE_FIELDS] == other.pick[SCENE_FIELDS] && self.pick[SPAWN_FIELD] == other.pick[SPAWN_FIELD]
    }

    /// Name of the current choice for field `f`.
    pub fn value(&self, f: usize) -> &'static str {
        FIELDS[f].choices[self.pick[f]]
    }

    /// Step field `f` forwards (`+1`) or backwards (`-1`), wrapping.
    pub fn step(&mut self, f: usize, by: i32) {
        let n = FIELDS[f].choices.len() as i32;
        self.pick[f] = (self.pick[f] as i32 + by).rem_euclid(n) as usize;
    }

    /// `key=Choice` lines.
    pub fn to_text(&self) -> String {
        FIELDS.iter().enumerate().map(|(f, field)| format!("{}={}\n", field.key, self.value(f))).collect()
    }

    /// Read [`to_text`](Self::to_text) output; unknown keys or choices keep
    /// their defaults.
    pub fn from_text(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let Some(f) = FIELDS.iter().position(|field| field.key == k) else { continue };
            // 1.10's "On" for glow and explosions is now "Low" and "Weak"
            let renamed = (matches!(k, "glow" | "explosions") && v.eq_ignore_ascii_case("on")).then_some(1);
            if let Some(i) = FIELDS[f].choices.iter().position(|c| c.eq_ignore_ascii_case(v)).or(renamed) {
                s.pick[f] = i;
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_map_to_values() {
        let mut s = Settings::default();
        assert_eq!((s.container(), s.solids(), s.wireframe()), (None, None, true));
        assert_eq!(s.tap_speed(), physics::TAP_SPEED);
        assert_eq!(s.gravity(), 0.0, "gravity starts off");
        assert!(!s.exact(), "sphere collisions by default");
        assert_eq!((s.lightest(), s.elasticity(), s.damping(), s.glow()), (1.0, physics::RESTITUTION, 1.0, Glow::Off));
        assert_eq!((s.sim_speed(), s.fps(), s.step_hz()), (1.0, 30.0, physics::STEP_HZ));
        s.pick = [4, 9, 1, 2, 2, 1, 3, 0, 3, 1, 0, 0, 1, 1, 0, 1, 0, 6, 4, 3, 1, 0, 0];
        assert!(s.exact());
        assert_eq!(s.container(), Some(SolidKind::Icosahedron));
        assert_eq!(s.solids(), Some(12));
        assert!(!s.wireframe());
        assert_eq!(s.tap_speed(), 13.0);
        assert_eq!(s.gravity(), 1.0);
        assert_eq!((s.lightest(), s.elasticity(), s.damping(), s.glow()), (0.05, 0.0, 0.5, Glow::Low));
        assert_eq!((s.sim_speed(), s.fps(), s.step_hz()), (2.0, 60.0, 960.0));
        assert_eq!((s.colour_by(), s.palette(), s.saturation(), s.explosions()), (ColourBy::Velocity, None, 1.0, Explosions::Weak));
        s.pick[10..14].copy_from_slice(&[2, 8, 0, 0]);
        assert_eq!((s.colour_by(), s.palette(), s.saturation(), s.explosions()), (ColourBy::Collisions, Some(7), 0.45, Explosions::Off));
        assert_eq!(FIELDS[PALETTE_FIELD].choices.len(), crate::palette::BITS.len() + 1);
        assert_eq!(Settings::default().wire_colour(), WireColour::Velocity);
        let d = Settings::default();
        assert_eq!((d.charge(), d.spawn()), (physics::Charge::MEDIUM, physics::Spawn::Grow));
        let mut c = d;
        c.pick[15] = 0;
        assert_eq!(c.charge().delay, 0.1, "low: 100 ms");
        c.pick[15] = 2;
        assert_eq!((c.charge().delay, c.charge().boost), (0.0, 1.1), "high: 10% more");
        for (k, size) in [(1, 0.1), (2, 0.3), (3, 0.6)] {
            c.pick[SPAWN_FIELD] = k;
            assert_eq!(c.spawn(), physics::Spawn::Pop(size));
        }
        assert!(!c.same_scene(&d), "a new spawn mode restarts");
        c.pick[SPAWN_FIELD] = 0;
        assert!(c.same_scene(&d), "charge applies live");
        s.pick[14] = 3;
        assert_eq!(s.wire_colour(), WireColour::BitShift);
        // colours, explosions and performance apply live; physics needs a restart
        let mut live = s;
        live.pick[10..16].copy_from_slice(&[1, 3, 2, 1, 3, 2]);
        assert!(live.same_scene(&s));
        live.pick[9] = 3;
        live.pick[CHAR_SCALE_FIELD] = 2;
        assert!(live.same_scene(&s), "glow and character scale apply live");
        live.pick[8] = (live.pick[8] + 1) % 3;
        assert!(!live.same_scene(&s));
        s.pick[4] = 1;
        assert!((s.gravity() - 0.15).abs() < 1e-6, "low gravity is 85% less");
        // a 1.3.0 settings file (no gravity line) still loads
        let old = Settings::from_text("container=Cube\nsolids=5\nwireframe=Off\ntap_speed=Fast\n");
        assert_eq!(old.pick, [1, 2, 1, 2, 0, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 1, 0, 4, 2, 1, 1, 0, 0]);
        // and a 1.5.0 one with gravity on keeps it on
        assert_eq!(Settings::from_text("gravity=On\n").gravity(), 1.0);
        // and a 1.10 one's glow and explosions "On" read as "Low" and "Weak"
        let on = Settings::from_text("glow=On\nexplosions=On\n");
        assert_eq!((on.glow(), on.explosions()), (Glow::Low, Explosions::Weak));
        assert_eq!(Settings::from_text("glow=High\nexplosions=Strong\n").explosions(), Explosions::Strong);
        assert_eq!((d.char_scale(), Settings::from_text("char_scale=0.5\n").char_scale()), (1.0, 0.5));
        // slower sim speeds and a medium damping with drag
        let mut k = Settings::default();
        let speeds: Vec<f32> = (0..7).map(|i| { k.pick[17] = i; k.sim_speed() }).collect();
        assert_eq!(speeds, [0.1, 0.175, 0.25, 0.5, 1.0, 1.5, 2.0]);
        let drags: Vec<physics::Drag> = (0..4).map(|i| { k.pick[8] = i; k.drag() }).collect();
        assert_eq!(drags[0], physics::Drag::NONE);
        assert!(drags.windows(2).all(|w| w[1].linear > w[0].linear && w[1].angular > w[0].angular && w[1].impact_spin > w[0].impact_spin));
        assert_eq!(Settings::from_text("damping=High\n").damping(), 0.5, "1.11's High is still High");
        assert!(!d.dynamic_camera() && !d.white_glow(), "both start off");
        // every field is on exactly one dialog
        let mut all: Vec<usize> = SETTINGS_FIELDS.iter().chain(&SIMULATION_FIELDS).chain(&COLOUR_FIELDS).copied().collect();
        all.sort();
        assert_eq!(all, (0..FIELDS.len()).collect::<Vec<_>>());
        assert_eq!(FIELDS[1].choices.len(), physics::MAX_SOLIDS - physics::MIN_SOLIDS + 2);
        for (i, k) in CONTAINERS.iter().enumerate() {
            assert!(FIELDS[0].choices[i + 1].eq_ignore_ascii_case(k.id()));
        }
    }

    #[test]
    fn steps_wrap_both_ways() {
        let mut s = Settings::default();
        s.step(2, 1);
        assert!(!s.wireframe());
        s.step(2, 1);
        assert!(s.wireframe());
        s.step(0, -1);
        assert_eq!(s.value(0), "Icosahedron");
    }

    #[test]
    fn text_round_trips_and_tolerates_junk() {
        let s = Settings { pick: [2, 3, 1, 0, 1, 1, 2, 1, 1, 1, 0, 0, 1, 1, 0, 1, 0, 2, 4, 3, 1, 0, 0] };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        let t = Settings::from_text("junk\ncontainer=nope\nsolids = 7\nother=1\n");
        assert_eq!(t.solids(), Some(7));
        assert_eq!(t.container(), None);
    }
}
