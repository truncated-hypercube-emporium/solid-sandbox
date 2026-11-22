//! Display-ratio presets and the terminal viewport they produce.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// (id, width, height) — width/height only define the ratio.
pub const PRESETS: [(&str, u32, u32); 14] = [
    ("vga", 640, 480),
    ("svga", 800, 600),
    ("xga", 1024, 768),
    ("sxga", 1280, 1024),
    ("wxga", 1280, 800),
    ("hd", 1280, 720),
    ("wxga+", 1440, 900),
    ("fhd", 1920, 1080),
    ("wuxga", 1920, 1200),
    ("qhd", 2560, 1440),
    ("uwfhd", 2560, 1080),
    ("uwqhd", 3440, 1440),
    ("uhd", 3840, 2160),
    ("square", 1, 1),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AspectMode {
    /// Shrink the picture to the ratio; blank margins.
    #[default]
    Letterbox,
    /// Fill the terminal, framed for the ratio; overflow is cut off.
    Crop,
}

impl FromStr for AspectMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "letterbox" => Ok(Self::Letterbox),
            "crop" => Ok(Self::Crop),
            _ => Err(format!("unknown aspect mode '{s}' (letterbox|crop)")),
        }
    }
}

/// `fill` → `None`; a preset or `W:H` → `Some(width / height)`.
pub fn parse_aspect(s: &str) -> Result<Option<f32>, String> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() || s == "fill" {
        return Ok(None);
    }
    if let Some((_, w, h)) = PRESETS.iter().find(|p| p.0 == s) {
        return Ok(Some(*w as f32 / *h as f32));
    }
    if let Some((a, b)) = s.split_once([':', 'x', '/'])
        && let (Ok(w), Ok(h)) = (a.trim().parse::<f32>(), b.trim().parse::<f32>())
        && w.is_finite()
        && h.is_finite()
        && w > 0.0
        && h > 0.0
        && (0.05..=20.0).contains(&(w / h))
    {
        return Ok(Some(w / h));
    }
    Err(format!("unknown aspect '{s}': use fill, a preset (see --list aspects) or W:H such as 21:9"))
}

/// Where the picture goes in the terminal, in cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    /// Picture ratio (width / height) the camera frames for; `None` = the viewport's own.
    pub fit: Option<f32>,
}

/// `cols × rows` cells available; `char_aspect` = cell width / height.
pub fn viewport(cols: usize, rows: usize, ratio: Option<f32>, mode: AspectMode, char_aspect: f32) -> Viewport {
    let full = Viewport { x: 0, y: 0, w: cols, h: rows, fit: ratio };
    let Some(r) = ratio else { return full };
    if cols == 0 || rows == 0 || mode == AspectMode::Crop {
        return full;
    }
    let ca = char_aspect.max(0.05);
    let area = cols as f32 * ca / rows as f32;
    let (w, h) = if area > r {
        (((r * rows as f32 / ca).round() as usize).clamp(1, cols), rows)
    } else {
        (cols, ((cols as f32 * ca / r).round() as usize).clamp(1, rows))
    };
    Viewport { x: (cols - w) / 2, y: (rows - h) / 2, w, h, fit: ratio }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_custom() {
        assert_eq!(parse_aspect("fill").unwrap(), None);
        assert_eq!(parse_aspect("VGA").unwrap(), Some(4.0 / 3.0));
        assert_eq!(parse_aspect("wxga").unwrap(), Some(1.6));
        assert_eq!(parse_aspect("wxga+").unwrap(), Some(1.6));
        assert!((parse_aspect("uwqhd").unwrap().unwrap() - 2.3889).abs() < 1e-3);
        assert_eq!(parse_aspect("21:9").unwrap(), Some(21.0 / 9.0));
        assert_eq!(parse_aspect("1920x1080").unwrap(), Some(16.0 / 9.0));
        assert!(parse_aspect("uwxg").is_err());
        assert!(parse_aspect("0:9").is_err());
        assert!(parse_aspect("16:").is_err());
        for (id, _, _) in PRESETS {
            assert!(parse_aspect(id).unwrap().is_some(), "{id}");
        }
    }

    #[test]
    fn letterbox_keeps_ratio_and_centres() {
        // 120×40 cells at 0.5 → 60:40 = 1.5 picture area
        let vp = viewport(120, 40, Some(4.0 / 3.0), AspectMode::Letterbox, 0.5);
        assert_eq!((vp.w, vp.h), (107, 40), "narrower: pillarbox");
        assert_eq!(vp.x, 6);
        let got = vp.w as f32 * 0.5 / vp.h as f32;
        assert!((got - 4.0 / 3.0).abs() < 0.02);

        let vp = viewport(120, 40, Some(21.0 / 9.0), AspectMode::Letterbox, 0.5);
        assert_eq!((vp.w, vp.h), (120, 26), "wider: letterbox");
        assert_eq!(vp.y, 7);
    }

    #[test]
    fn crop_and_fill_use_whole_terminal() {
        let vp = viewport(120, 40, Some(21.0 / 9.0), AspectMode::Crop, 0.5);
        assert_eq!((vp.x, vp.y, vp.w, vp.h), (0, 0, 120, 40));
        assert_eq!(vp.fit, Some(21.0 / 9.0));
        let vp = viewport(120, 40, None, AspectMode::Letterbox, 0.5);
        assert_eq!((vp.w, vp.h, vp.fit), (120, 40, None));
        assert_eq!(viewport(0, 0, Some(1.0), AspectMode::Letterbox, 0.5).w, 0);
    }
}
