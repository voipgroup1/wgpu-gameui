//! Color helpers: HSV(A) ↔ RGB(A) conversion.
//!
//! The crate works in linear-ish `[f32; 4]` RGBA throughout (the same shape the
//! draw list and theme use). This module adds an [`Hsva`] color and the
//! conversions a color picker needs.
//!
//! **Why a dedicated HSV type?** An interactive color picker must keep HSV as
//! its source of truth: HSV→RGB is total, but RGB→HSV is *lossy* at the
//! degenerate points — at `value == 0` (black) every hue/saturation maps to the
//! same RGB, and at `saturation == 0` (gray) every hue does. If a picker stored
//! RGB and re-derived HSV each frame, the hue/saturation cursors would snap to
//! zero the instant you dragged value or saturation to an edge. Storing [`Hsva`]
//! avoids that round-trip entirely.

/// A color in HSVA space.
///
/// - `h` (hue) is in degrees, normalized to `[0, 360)`.
/// - `s` (saturation), `v` (value/brightness) and `a` (alpha) are in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    /// Hue in degrees, `[0, 360)`.
    pub h: f32,
    /// Saturation, `[0, 1]`.
    pub s: f32,
    /// Value / brightness, `[0, 1]`.
    pub v: f32,
    /// Alpha / opacity, `[0, 1]`.
    pub a: f32,
}

impl Hsva {
    /// Construct an [`Hsva`]. Hue wraps into `[0, 360)`; `s`/`v`/`a` clamp to
    /// `[0, 1]` so a constructed value is always in range.
    pub fn new(h: f32, s: f32, v: f32, a: f32) -> Self {
        Self {
            h: h.rem_euclid(360.0),
            s: s.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    /// Opaque [`Hsva`] (`a = 1`).
    pub fn opaque(h: f32, s: f32, v: f32) -> Self {
        Self::new(h, s, v, 1.0)
    }

    /// Convert to straight (non-premultiplied) RGBA in `[0, 1]`.
    pub fn to_rgba(self) -> [f32; 4] {
        let [r, g, b] = hsv_to_rgb(self.h, self.s, self.v);
        [r, g, b, self.a]
    }

    /// Best-effort conversion from RGBA. Alpha passes through. Hue is `0` for
    /// grays/black (where it's undefined) — see the module note on why pickers
    /// shouldn't rely on this mid-drag.
    pub fn from_rgba(rgba: [f32; 4]) -> Self {
        let (h, s, v) = rgb_to_hsv([rgba[0], rgba[1], rgba[2]]);
        Self {
            h,
            s,
            v,
            a: rgba[3].clamp(0.0, 1.0),
        }
    }
}

/// HSV → RGB. `h` in degrees (wrapped to `[0, 360)`), `s`/`v` in `[0, 1]`;
/// returns straight RGB in `[0, 1]`. Standard six-sextant conversion.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s; // chroma
    let h6 = h / 60.0;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        // 5 and the h == 360→0 wrap (h6 can be exactly 6.0 only if h were 360,
        // which rem_euclid prevents, but guard the arm anyway).
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r1 + m, g1 + m, b1 + m]
}

/// RGB → HSV. `rgb` in `[0, 1]`; returns `(h_degrees, s, v)` with `h ∈ [0, 360)`.
/// Hue is `0` when undefined (achromatic: `max == min`).
pub fn rgb_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let r = rgb[0].clamp(0.0, 1.0);
    let g = rgb[1].clamp(0.0, 1.0);
    let b = rgb[2].clamp(0.0, 1.0);

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    let s = if max <= 0.0 { 0.0 } else { delta / max };

    let h = if delta <= 0.0 {
        0.0 // achromatic — hue undefined
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    (h.rem_euclid(360.0), s, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn rgb_close(a: [f32; 3], b: [f32; 3]) -> bool {
        close(a[0], b[0]) && close(a[1], b[1]) && close(a[2], b[2])
    }

    #[test]
    fn primary_and_secondary_hue_stops() {
        assert!(rgb_close(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]), "red");
        assert!(
            rgb_close(hsv_to_rgb(60.0, 1.0, 1.0), [1.0, 1.0, 0.0]),
            "yellow"
        );
        assert!(
            rgb_close(hsv_to_rgb(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]),
            "green"
        );
        assert!(
            rgb_close(hsv_to_rgb(180.0, 1.0, 1.0), [0.0, 1.0, 1.0]),
            "cyan"
        );
        assert!(
            rgb_close(hsv_to_rgb(240.0, 1.0, 1.0), [0.0, 0.0, 1.0]),
            "blue"
        );
        assert!(
            rgb_close(hsv_to_rgb(300.0, 1.0, 1.0), [1.0, 0.0, 1.0]),
            "magenta"
        );
    }

    #[test]
    fn saturation_zero_is_gray() {
        // s = 0 → r == g == b == v regardless of hue.
        let g = hsv_to_rgb(123.0, 0.0, 0.5);
        assert!(rgb_close(g, [0.5, 0.5, 0.5]), "gray at v=0.5");
    }

    #[test]
    fn value_zero_is_black() {
        assert!(rgb_close(hsv_to_rgb(200.0, 0.8, 0.0), [0.0, 0.0, 0.0]));
    }

    #[test]
    fn hue_wraps() {
        // 360 wraps to 0 (red); negative wraps too.
        assert!(rgb_close(hsv_to_rgb(360.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(-60.0, 1.0, 1.0), [1.0, 0.0, 1.0]));
    }

    #[test]
    fn rgb_to_hsv_known_values() {
        let (h, s, v) = rgb_to_hsv([1.0, 0.0, 0.0]);
        assert!(close(h, 0.0) && close(s, 1.0) && close(v, 1.0), "red");
        let (h, s, v) = rgb_to_hsv([0.0, 1.0, 0.0]);
        assert!(close(h, 120.0) && close(s, 1.0) && close(v, 1.0), "green");
        let (h, s, v) = rgb_to_hsv([0.0, 0.0, 1.0]);
        assert!(close(h, 240.0) && close(s, 1.0) && close(v, 1.0), "blue");
        // Gray: hue undefined → 0, saturation 0.
        let (h, s, v) = rgb_to_hsv([0.4, 0.4, 0.4]);
        assert!(close(h, 0.0) && close(s, 0.0) && close(v, 0.4), "gray");
    }

    #[test]
    fn round_trips_for_nondegenerate_colors() {
        for &(h, s, v) in &[
            (30.0, 0.7, 0.9),
            (210.0, 0.4, 0.6),
            (290.0, 1.0, 0.5),
            (95.0, 0.55, 0.8),
        ] {
            let rgb = hsv_to_rgb(h, s, v);
            let (h2, s2, v2) = rgb_to_hsv(rgb);
            assert!(close(h, h2), "h round-trip {h} != {h2}");
            assert!(close(s, s2), "s round-trip {s} != {s2}");
            assert!(close(v, v2), "v round-trip {v} != {v2}");
        }
    }

    #[test]
    fn hsva_round_trip_and_alpha_passthrough() {
        let c = Hsva::new(210.0, 0.4, 0.6, 0.33);
        let rgba = c.to_rgba();
        assert!(close(rgba[3], 0.33), "alpha preserved to rgba");
        let back = Hsva::from_rgba(rgba);
        assert!(close(back.h, 210.0) && close(back.s, 0.4) && close(back.v, 0.6));
        assert!(close(back.a, 0.33), "alpha preserved from rgba");
    }

    #[test]
    fn new_normalizes_and_clamps() {
        let c = Hsva::new(400.0, 1.5, -0.2, 2.0);
        assert!(close(c.h, 40.0), "hue wrapped into [0,360)");
        assert_eq!(c.s, 1.0);
        assert_eq!(c.v, 0.0);
        assert_eq!(c.a, 1.0);
    }
}
