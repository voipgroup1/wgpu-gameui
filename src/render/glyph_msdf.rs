//! MSDF (multi-channel signed distance field) generation for a single glyph.
//!
//! This is the **swappable generator seam**. Everything that knows about `fdsm`
//! lives here behind one function — [`generate_glyph_msdf`] — so that if `fdsm`
//! ever produces artifacts on real strings we can swap in the `msdf` C++ binding
//! crate or a hand-rolled generator by rewriting this one file, with no churn in
//! the atlas or render layers.
//!
//! ## Pipeline
//!
//! 1. `fdsm_ttf_parser::load_shape_from_face` lifts the glyph outline into an
//!    `fdsm::shape::Shape<Contour>` in **font units** (Y-up, origin at the glyph
//!    pen position on the baseline).
//! 2. We build an affine that maps font units → **tile pixels** (Y-down, with a
//!    `padding` margin around the glyph bbox so the distance ramp doesn't clip at
//!    the tile edge) and apply it to the shape via `fdsm::transform::Transform`.
//! 3. `edge_coloring_simple` assigns R/G/B channels to edges, `.prepare()` builds
//!    the acceleration structure, `generate_msdf` fills the RGB image, and
//!    `correct_sign_msdf` fixes the inside/outside sign (median > 0.5 == inside).
//!
//! The returned [`GlyphMetrics`] carries the tile's bounds in **EM fractions**
//! (units of `font_size`) relative to the pen-on-baseline origin, x rightward and
//! y upward, so the render layer can place a quad at any `font_size`:
//!
//! ```text
//! x_left   = pen_x    + left_em   * font_size
//! x_right  = pen_x    + right_em  * font_size
//! y_top    = baseline - top_em    * font_size   // top_em > 0 (above baseline)
//! y_bottom = baseline - bottom_em * font_size   // bottom_em < 0 for descenders
//! ```
//!
//! These bounds INCLUDE the SDF padding, so the quad covers the whole tile and the
//! uv rect maps 1:1.

use fdsm::bezier::scanline::FillRule;
use fdsm::generate::generate_msdf;
use fdsm::render::correct_sign_msdf;
use fdsm::shape::{ColoredContour, Shape};
use fdsm::transform::Transform;
use image::RgbImage;
use nalgebra::{Affine2, Matrix3};
use ttf_parser::{Face, GlyphId};

/// Sine of the edge-coloring angle threshold (3°, matching msdfgen's default).
/// Edges meeting at a sharper corner than this get distinct color channels.
const EDGE_COLORING_SIN_ALPHA: f64 = 0.052_335_956; // (3°).to_radians().sin()
/// Deterministic seed for `edge_coloring_simple` so generation is reproducible
/// (important for tests and for stable atlas contents across runs).
const EDGE_COLORING_SEED: u64 = 0;

/// Placement metrics for a generated glyph tile, in EM fractions (units of
/// `font_size`) relative to the pen-on-baseline origin. x rightward, y **upward**
/// (font convention). Bounds include the SDF padding margin, so the quad they
/// describe covers the full tile.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphMetrics {
    /// Tile width in pixels (at the generation reference size).
    pub width_px: u32,
    /// Tile height in pixels (at the generation reference size).
    pub height_px: u32,
    /// Left edge of the tile, EM fraction (typically slightly negative due to padding/bearing).
    pub left_em: f32,
    /// Right edge of the tile, EM fraction.
    pub right_em: f32,
    /// Top edge of the tile, EM fraction (positive == above baseline).
    pub top_em: f32,
    /// Bottom edge of the tile, EM fraction (negative for descenders).
    pub bottom_em: f32,
}

/// A generated glyph: its MSDF tile (RGB8) plus placement metrics.
pub struct GlyphMsdf {
    /// The generated MSDF tile (RGB8, distances in each channel).
    pub image: RgbImage,
    /// EM-fraction placement metrics for positioning the tile's quad.
    pub metrics: GlyphMetrics,
}

/// Generate an MSDF tile for one glyph.
///
/// * `face` — the parsed font face (from `cosmic_text::Font::data()`).
/// * `glyph` — the glyph id to render (post-shaping, from cosmic-text layout).
/// * `ref_px` — the reference EM size in pixels the tile is generated at. Larger
///   gives more distance-field resolution (crisper at large display sizes) at the
///   cost of atlas space. ~48–64 is typical.
/// * `px_range` — the width of the distance ramp in tile pixels. The shader uses
///   this to scale screen-space AA. ~4–6 is typical. Also sets the tile padding.
///
/// Returns `None` for glyphs with no outline (whitespace) — the caller advances
/// the pen without drawing a tile.
pub fn generate_glyph_msdf(
    face: &Face,
    glyph: GlyphId,
    ref_px: f32,
    px_range: f32,
) -> Option<GlyphMsdf> {
    // Whitespace and other outline-less glyphs have no shape → no tile.
    let shape = fdsm_ttf_parser::load_shape_from_face(face, glyph)?;
    let bbox = face.glyph_bounding_box(glyph)?;
    let upm = face.units_per_em() as f64;
    if upm <= 0.0 {
        return None;
    }

    let ref_px = ref_px as f64;
    let px_range = px_range.max(1.0) as f64;
    // Padding (in tile pixels) on every side so the full distance ramp fits. The
    // ramp reaches +-px_range/2 around the outline, so padding >= px_range/2; we
    // use the full px_range plus a pixel for safety.
    let padding = px_range.ceil() + 1.0;

    let scale = ref_px / upm; // font units -> pixels

    let x_min = bbox.x_min as f64;
    let y_min = bbox.y_min as f64;
    let x_max = bbox.x_max as f64;
    let y_max = bbox.y_max as f64;

    // Degenerate bbox (e.g. a zero-area control glyph) — nothing to render.
    if x_max <= x_min || y_max <= y_min {
        return None;
    }

    let glyph_w_px = (x_max - x_min) * scale;
    let glyph_h_px = (y_max - y_min) * scale;
    let width_px = (glyph_w_px + 2.0 * padding).ceil() as u32;
    let height_px = (glyph_h_px + 2.0 * padding).ceil() as u32;
    if width_px == 0 || height_px == 0 {
        return None;
    }

    // Affine: font units (Y-up) -> tile pixels (Y-down).
    //   px = scale * fx + tx           with tx = padding - x_min * scale
    //   py = -scale * fy + ty          with ty = padding + y_max * scale
    // So the glyph's top-left (x_min, y_max) maps to (padding, padding).
    let tx = padding - x_min * scale;
    let ty = padding + y_max * scale;
    #[rustfmt::skip]
    let affine = Affine2::from_matrix_unchecked(Matrix3::new(
        scale, 0.0,    tx,
        0.0,   -scale, ty,
        0.0,   0.0,    1.0,
    ));

    let mut shape = shape;
    shape.transform(&affine);

    let colored = Shape::<ColoredContour>::edge_coloring_simple(
        shape,
        EDGE_COLORING_SIN_ALPHA,
        EDGE_COLORING_SEED,
    );
    let prepared = colored.prepare();

    let mut image = RgbImage::new(width_px, height_px);
    generate_msdf(&prepared, px_range, &mut image);
    correct_sign_msdf(&mut image, &prepared, FillRule::Nonzero);

    // Tile-corner -> font units (invert the affine analytically), then -> EM
    // fraction. Computing from the actual tile dimensions (which were ceil'd)
    // keeps the quad's uv mapping exact.
    //   top-left  pixel (0, 0):        fx = -tx/scale,             fy = ty/scale
    //   bot-right pixel (W, H): fx = (W - tx)/scale,  fy = (ty - H)/scale
    let left_fu = -tx / scale;
    let right_fu = (width_px as f64 - tx) / scale;
    let top_fu = ty / scale;
    let bottom_fu = (ty - height_px as f64) / scale;

    let metrics = GlyphMetrics {
        width_px,
        height_px,
        left_em: (left_fu / upm) as f32,
        right_em: (right_fu / upm) as f32,
        top_em: (top_fu / upm) as f32,
        bottom_em: (bottom_fu / upm) as f32,
    };

    Some(GlyphMsdf { image, metrics })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_face() -> Face<'static> {
        Face::parse(notosans::REGULAR_TTF, 0).expect("parse NotoSans")
    }

    fn glyph_for(face: &Face, c: char) -> GlyphId {
        face.glyph_index(c).expect("glyph present")
    }

    /// median of an RGB pixel, normalized to 0..1. The MSDF reconstruction uses
    /// the median channel; > 0.5 == inside the glyph, < 0.5 == outside.
    fn median01(p: &image::Rgb<u8>) -> f32 {
        let mut v = [p.0[0], p.0[1], p.0[2]];
        v.sort_unstable();
        v[1] as f32 / 255.0
    }

    #[test]
    fn whitespace_glyph_has_no_tile() {
        let face = test_face();
        let space = glyph_for(&face, ' ');
        assert!(
            generate_glyph_msdf(&face, space, 48.0, 4.0).is_none(),
            "space should have no outline tile"
        );
    }

    #[test]
    fn glyph_generates_nonempty_tile() {
        let face = test_face();
        let a = glyph_for(&face, 'A');
        let g = generate_glyph_msdf(&face, a, 48.0, 4.0).expect("A generates");
        assert!(g.metrics.width_px > 0 && g.metrics.height_px > 0);
        assert_eq!(g.image.width(), g.metrics.width_px);
        assert_eq!(g.image.height(), g.metrics.height_px);
        // EM metrics should be sane: the glyph is above the baseline and has
        // positive horizontal extent.
        assert!(g.metrics.right_em > g.metrics.left_em);
        assert!(g.metrics.top_em > g.metrics.bottom_em);
        assert!(g.metrics.top_em > 0.0, "uppercase A extends above baseline");
    }

    #[test]
    fn sign_is_correct_inside_vs_outside() {
        // The tile corners are in the padding margin → always outside (median < 0.5).
        // At least one interior pixel must be inside (median > 0.5). A sign
        // inversion or empty output fails both checks.
        let face = test_face();
        for c in ['A', 'M', 'H', 'g', '0', '@'] {
            let gid = glyph_for(&face, c);
            let g = generate_glyph_msdf(&face, gid, 48.0, 4.0)
                .unwrap_or_else(|| panic!("'{c}' generates"));
            let img = &g.image;
            let (w, h) = (img.width(), img.height());

            // All four corners sit in padding → outside.
            for &(x, y) in &[(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                let m = median01(img.get_pixel(x, y));
                assert!(
                    m < 0.5,
                    "'{c}' corner ({x},{y}) median {m} should be < 0.5 (outside)"
                );
            }

            // Some pixel is inside.
            let any_inside = img.pixels().any(|p| median01(p) > 0.5);
            assert!(any_inside, "'{c}' has no interior pixels (median > 0.5)");
        }
    }

    /// Eyeball check for fdsm quality across the printable-ASCII set — the
    /// maturity risk gate from the plan. Writes per-glyph MSDF PNGs (RGB encoded
    /// directly, so you see the raw 3-channel field) into `test_output/msdf/`.
    /// Run with: `cargo test -p wgpu-gameui dump_ascii_msdf -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes PNG files for manual inspection"]
    fn dump_ascii_msdf() {
        let face = test_face();
        let dir = std::path::Path::new("test_output/msdf");
        std::fs::create_dir_all(dir).expect("create test_output/msdf");
        let mut generated = 0usize;
        for code in 0x21u8..=0x7e {
            let c = code as char;
            let gid = match face.glyph_index(c) {
                Some(g) => g,
                None => continue,
            };
            let Some(g) = generate_glyph_msdf(&face, gid, 48.0, 4.0) else {
                continue;
            };
            let safe = format!("{:02x}_{}", code, if c.is_alphanumeric() { c } else { '_' });
            let path = dir.join(format!("{safe}.png"));
            g.image.save(&path).expect("save png");
            generated += 1;
        }
        eprintln!("wrote {generated} glyph MSDFs to {}", dir.display());
        assert!(
            generated > 90,
            "expected most of printable ASCII to generate"
        );
    }
}
