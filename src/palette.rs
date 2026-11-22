//! Random colour palettes the solids can be drawn in (two each of 3-, 4-,
//! 5- and 6-bit, i.e. 8 to 64 colours), the saturation control, and the
//! nearest-colour lookup that maps any colour onto a palette.
//!
//! The palettes are generated once, saved with the settings, and only
//! replaced when the palette button in the dialog is pressed.

use crate::physics::hsv;

/// Bits per colour of each palette, in the order the dialog lists them.
pub const BITS: [u32; 8] = [3, 3, 4, 4, 5, 5, 6, 6];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palettes {
    /// One `0xRRGGBB` list per entry of [`BITS`], `2^bits` colours long,
    /// darkest first.
    pub sets: Vec<Vec<u32>>,
}

/// Perceived brightness of `0xRRGGBB`, 0..=255.
pub fn luma(c: u32) -> f32 {
    let (r, g, b) = (((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32);
    0.299 * r + 0.587 * g + 0.114 * b
}

/// Brightness (luma) of a palette's darkest and lightest colours.
const LUMA_RANGE: (f32, f32) = (10.0, 245.0);
/// How far a ramp's ends turn from its own hue towards the cold shadow hue
/// and the warm highlight hue (1: all the way).
const HUE_SHIFT: f32 = 0.55;
/// How much a ramp's ends lose of its midtone saturation.
const END_DESATURATE: f32 = 0.5;

/// How a palette of `bits` is built: whether it has one shared shadow and
/// one shared highlight (strict 8- and 16-colour palettes), and its number
/// of hue ramps × steps per ramp.
pub fn plan(bits: u32) -> (bool, usize, usize) {
    match bits {
        3 => (true, 2, 3),  // shadow + 2 ramps × 3 + highlight = 8
        4 => (true, 2, 7),  // shadow + 2 ramps × 7 + highlight = 16
        5 => (false, 4, 8), // 4 ramps × 8 = 32
        _ => (false, 8, 8), // 8 ramps × 8 = 64
    }
}

/// `a` turned towards hue `b` by `t` (0..=1), the short way round the wheel.
fn hue_towards(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + 0.5).rem_euclid(1.0) - 0.5;
    (a + d * t).rem_euclid(1.0)
}

/// The colour of hue `h` and saturation `s` at luma `target`: by value
/// alone where the pure colour is bright enough, else (bright highlights
/// only) with just as much saturation given up as it takes.
pub fn at_luma(h: f32, s: f32, target: f32) -> u32 {
    let full = hsv(h, s, 1.0);
    let l = luma(full);
    if target <= l {
        return hsv(h, s, (target / l.max(1e-3)).min(1.0));
    }
    let (mut lo, mut hi) = (0.0f32, s);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if luma(hsv(h, mid, 1.0)) >= target { lo = mid } else { hi = mid }
    }
    hsv(h, lo, 1.0)
}

/// A palette of `2^bits` colours (darkest first), and its hue ramps (each
/// dark to light), following pixel-art practice:
///
/// - hue shifting: each ramp shades towards a cold blue-violet and
///   highlights towards a warm yellow, instead of towards black and white;
/// - a value ladder: every colour has its own brightness, evenly spaced
///   from very dark to very light (interleaved across ramps), so no two
///   merge into one grey;
/// - shared ends: 8- and 16-colour palettes have one shadow and one
///   highlight that every ramp runs into;
/// - saturation: rich midtones, the darkest and lightest steps calmer.
pub fn build(bits: u32, rng: &mut fastrand::Rng) -> (Vec<u32>, Vec<Vec<u32>>) {
    let (shared, ramps, steps) = plan(bits);
    let (lo, hi) = LUMA_RANGE;
    // the temperature ends, a little different each time
    let cold = 0.69 + 0.06 * (rng.f32() - 0.5); // indigo to blue-violet
    let warm = 0.13 + 0.03 * (rng.f32() - 0.5); // yellow
    let h0 = rng.f32();
    // brightness slots: interleaved, so ramp r step l gets slot l·ramps + r
    let m = ramps * steps;
    let slot_luma = |j: usize| {
        if shared {
            lo + (hi - lo) * (j + 1) as f32 / (m + 1) as f32 // between the shared ends
        } else {
            lo + (hi - lo) * j as f32 / (m - 1) as f32
        }
    };
    let ramp_list: Vec<Vec<u32>> = (0..ramps)
        .map(|r| {
            let base = (h0 + r as f32 / ramps as f32 + 0.08 * (rng.f32() - 0.5)).rem_euclid(1.0);
            let sat = 0.7 + 0.25 * rng.f32();
            (0..steps)
                .map(|l| {
                    // position along the ramp, -1 (shadow) to 1 (highlight);
                    // ramps between shared ends stop short of them
                    let u = if shared { (l + 1) as f32 / (steps + 1) as f32 } else { l as f32 / (steps - 1) as f32 };
                    let d = 2.0 * u - 1.0;
                    let hue = if d < 0.0 { hue_towards(base, cold, HUE_SHIFT * -d) } else { hue_towards(base, warm, HUE_SHIFT * d) };
                    let s = sat * (1.0 - END_DESATURATE * d * d);
                    at_luma(hue, s, slot_luma(l * ramps + r))
                })
                .collect()
        })
        .collect();
    let mut flat: Vec<u32> = ramp_list.iter().flatten().copied().collect();
    if shared {
        flat.push(at_luma(cold, 0.5, lo)); // the one deep shadow
        flat.push(at_luma(warm, 0.3, hi)); // the one pale highlight
    }
    flat.sort_by(|a, b| luma(*a).total_cmp(&luma(*b)));
    (flat, ramp_list)
}

/// `2^bits` colours, darkest first (see [`build`]).
fn random_palette(bits: u32, rng: &mut fastrand::Rng) -> Vec<u32> {
    build(bits, rng).0
}

impl Palettes {
    /// A fresh set of eight random palettes.
    pub fn generate(seed: u64) -> Self {
        let mut rng = fastrand::Rng::with_seed(seed);
        Self { sets: BITS.iter().map(|&b| random_palette(b, &mut rng)).collect() }
    }

    /// One line per palette: its colours as hex, space separated.
    pub fn to_text(&self) -> String {
        self.sets
            .iter()
            .map(|p| p.iter().map(|c| format!("{c:06x}")).collect::<Vec<_>>().join(" ") + "\n")
            .collect()
    }

    /// Read [`to_text`](Self::to_text) output; `None` unless every palette
    /// is there at its full size.
    pub fn from_text(text: &str) -> Option<Self> {
        let sets: Vec<Vec<u32>> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.split_whitespace().map(|h| u32::from_str_radix(h, 16).ok().filter(|&c| c <= 0xff_ff_ff)).collect())
            .collect::<Option<_>>()?;
        let sizes_ok = sets.len() == BITS.len() && sets.iter().zip(BITS).all(|(p, b)| p.len() == 1 << b);
        sizes_ok.then_some(Self { sets })
    }

    /// Palette `i` of [`BITS`].
    pub fn get(&self, i: usize) -> Option<&[u32]> {
        self.sets.get(i).map(Vec::as_slice)
    }
}

/// Nearest-colour lookup onto one palette, through a 4096-entry table (4
/// bits per channel), so mapping a frame's cells costs one load each.
#[derive(Debug, Clone)]
pub struct Quantizer {
    lut: Vec<u32>,
    /// The palette and its bit depth (log2 of its size).
    palette: Vec<u32>,
    bits: u32,
}

/// `index`'s low `bits` bits rotated left by `by`.
pub fn rotate_bits(index: u32, by: u32, bits: u32) -> u32 {
    let mask = (1u32 << bits) - 1;
    let (i, by) = (index & mask, by % bits.max(1));
    ((i << by) | (i >> (bits - by))) & mask
}

impl Quantizer {
    pub fn new(palette: &[u32]) -> Self {
        let ch = |c: u32, s: u32| ((c >> s) & 0xff) as f32;
        let lut = (0..4096u32)
            .map(|k| {
                // the middle of this table cell
                let (r, g, b) = (((k >> 8) << 4 | 8) as f32, ((k >> 4 & 15) << 4 | 8) as f32, ((k & 15) << 4 | 8) as f32);
                let dist = |p: u32| {
                    let (dr, dg, db) = (ch(p, 16) - r, ch(p, 8) - g, ch(p, 0) - b);
                    // weighted towards green, as the eye is
                    2.0 * dr * dr + 4.0 * dg * dg + 3.0 * db * db
                };
                palette.iter().copied().min_by(|a, b| dist(*a).total_cmp(&dist(*b))).unwrap_or(0)
            })
            .collect();
        let bits = (palette.len().max(1) as f32).log2().floor() as u32;
        Self { lut, palette: palette.to_vec(), bits }
    }

    /// Bit depth of the palette.
    pub fn bits(&self) -> u32 {
        self.bits
    }

    /// The palette colour whose index is `c`'s nearest colour's index rotated
    /// left by half the palette's bit depth (rounded down: 1 bit of 3, 2 of
    /// 4 or 5, 3 of 6).
    pub fn bit_shift(&self, c: u32) -> u32 {
        let near = self.map(c);
        let i = self.palette.iter().position(|&p| p == near).unwrap_or(0) as u32;
        self.palette[rotate_bits(i, self.bits / 2, self.bits) as usize % self.palette.len()]
    }

    /// The palette colour nearest `c`.
    #[inline]
    pub fn map(&self, c: u32) -> u32 {
        self.lut[(((c >> 20) & 15) << 8 | ((c >> 12) & 15) << 4 | ((c >> 4) & 15)) as usize]
    }
}

/// Without a palette, "half the bit value" of a 24-bit colour: `c` rotated
/// by 12 bits, which swaps its high and low halves.
pub fn bit_shift_24(c: u32) -> u32 {
    rotate_bits(c, 12, 24)
}

/// `c` with its saturation scaled by `k` (1 leaves it as it is, 0 makes it
/// grey, above 1 pushes it further from grey), keeping its brightness.
pub fn saturate(c: u32, k: f32) -> u32 {
    let y = luma(c);
    let ch = |s: u32| {
        let x = ((c >> s) & 0xff) as f32;
        ((y + (x - y) * k).round().clamp(0.0, 255.0) as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_have_the_right_sizes_and_span_dark_to_light() {
        let p = Palettes::generate(1);
        assert_eq!(p.sets.iter().map(Vec::len).collect::<Vec<_>>(), [8, 8, 16, 16, 32, 32, 64, 64]);
        for s in &p.sets {
            assert!(s.windows(2).all(|w| luma(w[0]) <= luma(w[1])), "darkest first");
            assert!(luma(s[0]) < 70.0 && luma(*s.last().unwrap()) > 150.0, "spans the brightness range");
        }
        assert_ne!(p.sets[0], p.sets[1], "the two of a size differ");
        assert_ne!(Palettes::generate(2), p, "a new seed gives new palettes");
    }

    /// HSV saturation of `0xRRGGBB`.
    fn sat(c: u32) -> f32 {
        let v = [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff].map(|x| x as f32);
        let (mx, mn) = (v.iter().cloned().fold(0.0, f32::max), v.iter().cloned().fold(255.0, f32::min));
        if mx > 0.0 { (mx - mn) / mx } else { 0.0 }
    }

    /// HSV hue of `0xRRGGBB`, 0..1.
    fn hue(c: u32) -> f32 {
        let (r, g, b) = (((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32);
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        let d = (mx - mn).max(1e-6);
        let h = if mx == r { ((g - b) / d).rem_euclid(6.0) } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
        h / 6.0
    }

    /// Distance round the colour wheel, 0..=0.5.
    fn hue_gap(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(1.0);
        d.min(1.0 - d)
    }

    /// How blue (cold) against yellow (warm) a colour is, -1..=1.
    fn coldness(c: u32) -> f32 {
        let (r, g, b) = (((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32);
        (b - (r + g) / 2.0) / 255.0
    }

    #[test]
    fn every_colour_has_its_own_grey() {
        for seed in 0..20 {
            let p = Palettes::generate(seed);
            for s in &p.sets {
                let l: Vec<f32> = s.iter().map(|&c| luma(c)).collect();
                let gap = (LUMA_RANGE.1 - LUMA_RANGE.0) / s.len() as f32;
                for w in l.windows(2) {
                    assert!(w[1] - w[0] > 0.5 * gap, "seed {seed}, {} colours: greys {:.1} and {:.1} too close", s.len(), w[0], w[1]);
                }
                assert!(l[0] < 15.0 && *l.last().unwrap() > 240.0, "very dark to very light");
            }
        }
    }

    #[test]
    fn ramps_shift_hue_cold_in_shadow_and_warm_in_light() {
        for seed in 0..20 {
            let mut rng = fastrand::Rng::with_seed(seed);
            for bits in [3, 4, 5, 6] {
                let (_, ramps) = build(bits, &mut rng);
                for ramp in &ramps {
                    let (dark, light) = (ramp[0], *ramp.last().unwrap());
                    // the shadow's hue is nearer blue-violet, the light's nearer yellow
                    let (cold, warm) = (0.69, 0.13);
                    assert!(
                        hue_gap(hue(dark), cold) < hue_gap(hue(light), cold) && hue_gap(hue(light), warm) < hue_gap(hue(dark), warm),
                        "seed {seed} {bits}-bit: shadow {dark:06x} should be colder, light {light:06x} warmer"
                    );
                    // the middle is the richest, the ends calmer
                    let mid = ramp.iter().map(|&c| sat(c)).fold(0.0, f32::max);
                    assert!(sat(dark) < mid + 1e-3 && sat(light) < mid + 1e-3);
                    assert!(ramp.windows(2).all(|w| luma(w[0]) < luma(w[1])), "each ramp dark to light");
                }
                // hue shifting, not black and white: shadows keep colour, lights stay off-white
                let lit = ramps.iter().map(|r| *r.last().unwrap());
                assert!(lit.clone().all(|c| c != 0xffffff));
            }
        }
    }

    #[test]
    fn strict_palettes_share_one_shadow_and_one_highlight() {
        let mut rng = fastrand::Rng::with_seed(4);
        for bits in [3, 4] {
            let (flat, ramps) = build(bits, &mut rng);
            let in_ramps: Vec<u32> = ramps.iter().flatten().copied().collect();
            let extra: Vec<u32> = flat.iter().copied().filter(|c| !in_ramps.contains(c)).collect();
            assert_eq!(extra.len(), 2, "{bits}-bit: a shadow and a highlight outside the ramps");
            assert_eq!((extra[0], extra[1]), (flat[0], *flat.last().unwrap()), "they are the darkest and lightest");
            assert!(coldness(flat[0]) > 0.0, "the shadow is cold: {:06x}", flat[0]);
            assert!(coldness(*flat.last().unwrap()) < 0.0, "the highlight warm: {:06x}", flat.last().unwrap());
            // every ramp sits between them
            for r in &ramps {
                assert!(luma(r[0]) > luma(flat[0]) && luma(*r.last().unwrap()) < luma(*flat.last().unwrap()));
            }
        }
        for bits in [5, 6] {
            let (flat, ramps) = build(bits, &mut rng);
            assert_eq!(flat.len(), ramps.iter().map(Vec::len).sum::<usize>(), "bigger palettes are all ramps");
        }
    }

    #[test]
    fn palettes_survive_a_round_trip_and_reject_damage() {
        let p = Palettes::generate(7);
        assert_eq!(Palettes::from_text(&p.to_text()), Some(p.clone()));
        let text = p.to_text();
        let short: String = text.lines().take(7).map(|l| format!("{l}\n")).collect();
        assert_eq!(Palettes::from_text(&short), None, "a palette missing");
        assert_eq!(Palettes::from_text(&text.replacen(' ', " zz ", 1)), None, "junk");
        assert_eq!(Palettes::from_text(""), None);
    }

    #[test]
    fn quantizer_maps_onto_the_palette() {
        let pal = [0x000000, 0xff0000, 0x00ff00, 0x0000ff, 0xffffff];
        let q = Quantizer::new(&pal);
        assert_eq!(q.map(0xee1010), 0xff0000);
        assert_eq!(q.map(0x10f020), 0x00ff00);
        assert_eq!(q.map(0x202020), 0x000000);
        assert_eq!(q.map(0xf0f0f0), 0xffffff);
        let p = Palettes::generate(3);
        let q = Quantizer::new(p.get(7).unwrap());
        for c in [0x123456, 0xabcdef, 0x808080, 0xff00ff] {
            assert!(p.sets[7].contains(&q.map(c)));
        }
    }

    #[test]
    fn bit_shift_rotates_by_half_the_depth() {
        assert_eq!(rotate_bits(0b001, 1, 3), 0b010);
        assert_eq!(rotate_bits(0b100, 1, 3), 0b001, "wraps");
        assert_eq!(rotate_bits(0b000111, 3, 6), 0b111000);
        assert_eq!(bit_shift_24(0x123456), 0x456123);
        let p = Palettes::generate(2);
        for (set, half) in p.sets.iter().zip([1, 1, 2, 2, 2, 2, 3, 3]) {
            let q = Quantizer::new(set);
            assert_eq!(q.bits() / 2, half);
            for (i, &c) in set.iter().enumerate() {
                let want = set[rotate_bits(i as u32, half, q.bits()) as usize];
                // a palette colour maps to itself, then its index is rotated
                if q.map(c) == c {
                    assert_eq!(q.bit_shift(c), want);
                }
                assert!(set.contains(&q.bit_shift(c)), "stays in the palette");
            }
        }
    }

    #[test]
    fn saturation_scales_away_from_grey() {
        assert_eq!(saturate(0x808080, 2.0), 0x808080, "greys stay grey");
        assert_eq!(saturate(0xc04040, 1.0), 0xc04040);
        let spread = |c: u32| {
            let v = [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff];
            v.iter().max().unwrap() - v.iter().min().unwrap()
        };
        assert!(spread(saturate(0xc04040, 0.5)) < spread(0xc04040));
        assert!(spread(saturate(0xc04040, 1.6)) > spread(0xc04040));
        assert!((luma(saturate(0xc04040, 0.5)) - luma(0xc04040)).abs() < 1.5, "brightness kept");
    }
}
