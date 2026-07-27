use image::{GenericImageView, Pixel, Rgb};
use na::Vector2;

use crate::{
    bezier::{prepared::PreparedColoredShape, solve_quadratic, Point},
    interpolate::sample_bilinear,
    render::{median, median3},
};

use super::mix;

pub const CANDIDATE: u8 = 1;
pub const ARTIFACT: u8 = 2;

/// An object that classifies artifacts in signed distance fields.
pub trait ArtifactClassifier {
    /// Evaluates whether the median value `xm` interpolated at `xt`
    /// in the range between `am` at `at` and `bm` at `bt` indicates
    /// an artifact.
    ///
    /// # Parameters
    ///
    /// * `at`: the parameter associated with the left end of the range
    /// * `bt`: the parameter associated with the right end of the range
    /// * `xt`: the parameter associated with the point of interest
    /// * `am`: the value associated with the left end of the range
    /// * `bm`: the value associated with the right end of the range
    /// * `xm`: the value associated with the point of interest
    ///
    /// # Returns
    ///
    /// A `u8` with the [`CANDIDATE`] bit set if this point is
    /// deemed a candidate for further evaluation, and with the
    /// [`ARTIFACT`] bit set if it is deemed as an artifact.
    fn range_test(&self, at: f64, bt: f64, xt: f64, am: f64, bm: f64, xm: f64) -> u8;
    /// Given the return value from [`ArtifactClassifier::evaluate`],
    /// returns whether the point of interest is an artifact.
    ///
    /// # Parameters
    ///
    /// * `t`: the parameter associated with the point of interest
    /// * `m`: the median value associated with the point of interest
    fn evaluate(&self, t: f64, m: f32, flags: u8) -> bool;
}

const ARTIFACT_T_EPSILON: f32 = 0.01;

// For some reason, rustc chooses not to inline this by default.
#[inline(always)]
fn is_in_range(t: f32) -> bool {
    t > ARTIFACT_T_EPSILON && t < (1.0 - ARTIFACT_T_EPSILON)
}

/// Checks if a linear interpolation artifact will occur at a point
/// where two specific color channels are equal – such points have
/// extreme median values.
///
/// # Parameters
///
/// * `classifier`: the [`ArtifactClassifier`] to use
/// * `am`: the median of `a`. This is passed in to avoid
///   recalculation, as the same value of `a` is passed in
///   many times.
/// * `bm`: the median of `b`.
/// * `a`: the color of the central texel
/// * `b`: the color of the peripheral texel
/// * `da`, `db`: the difference between a pair of channels for both
///   `a` and `b`.
fn has_linear_artifact_inner(
    classifier: &impl ArtifactClassifier,
    am: f32,
    bm: f32,
    a: Rgb<f32>,
    b: Rgb<f32>,
    da: f32,
    db: f32,
) -> bool {
    // Find interpolation ratio t (0 < t < 1) where two color channels are equal (`mix(da, db, t) == 0.0``).
    let t = da / (da - db);
    if is_in_range(t) {
        // Interpolate median at t and let the classifier decide if its value indicates an artifact.
        let xm = interpolated_median(a, b, t);
        classifier.evaluate(
            t as f64,
            xm,
            classifier.range_test(0.0, 1.0, t as f64, am as f64, bm as f64, xm as f64),
        )
    } else {
        false
    }
}

/// Checks if a linear interpolation artifact will occur in between
/// two horizontally or vertically adjacent texels `a` and `b`.
///
/// # Parameters
///
/// * `classifier`: the [`ArtifactClassifier`] to use
/// * `am`: the median of `a`. This is passed in to avoid
///   recalculation, as the same value of `a` is passed in
///   many times.
/// * `a`: the color of the central texel
/// * `b`: the color of the peripheral texel
pub fn has_linear_artifact(
    classifier: &impl ArtifactClassifier,
    am: f32,
    a: Rgb<f32>,
    b: Rgb<f32>,
) -> bool {
    let bm = median(b);
    (am - 0.5).abs() > (bm - 0.5).abs()
        && (has_linear_artifact_inner(classifier, am, bm, a, b, a[1] - a[0], b[1] - b[0])
            || has_linear_artifact_inner(classifier, am, bm, a, b, a[2] - a[1], b[2] - b[1])
            || has_linear_artifact_inner(classifier, am, bm, a, b, a[0] - a[2], b[0] - b[2]))
}

struct Alq {
    a: Rgb<f32>,
    l: [f32; 3],
    q: [f32; 3],
    am: f32,
    dm: f32,
}

/// Checks if a bilinear interpolation artifact will occur at a point
/// where two specific color channels are equal – such points have
/// extreme median values.
///
/// # Paramters
///
/// * `classifier`: the [`ArtifactClassifier`] to use
/// * `alq.am`: the median of `a`. This is passed in to avoid
///   recalculation, as the same value of `a` is passed in
///   many times.
/// * `alq.dm`: the median of the color of the diagonally adjacent
///   texel.
/// * `a`: the color of the central texel
/// * `l`: linear term
/// * `q`: quadratic term
/// * `da`, `dd`: the difference between a pair of channels for both
///   `a` and `d`.
/// * `dbc`: `db + dc - da`, where `db` and `dc` are defined similarly
///   for `b` and `c`
/// * `t_ex0`, `t_ex1`: The interpolation ratios for the local
///   extrema of the two color channels.
fn has_diagonal_artifact_inner(
    classifier: &impl ArtifactClassifier,
    alq: &Alq,
    da: f32,
    dbc_minus_da: f32,
    dd: f32,
    t_ex0: f32,
    t_ex1: f32,
) -> bool {
    let ts = solve_quadratic(
        (dd - dbc_minus_da) as f64,
        (dbc_minus_da - da) as f64,
        da as f64,
    );
    for t in ts {
        let t = t as f32;
        if is_in_range(t) {
            let xm = interpolated_median2(alq, t);
            let mut range_flags =
                classifier.range_test(0.0, 1.0, t as f64, alq.am as f64, alq.dm as f64, xm as f64);

            for t_ex in [t_ex0, t_ex1] {
                if t_ex > 0.0 && t_ex < 1.0 {
                    let mut t_end = [0.0, 1.0];
                    let mut em = [alq.am, alq.dm];
                    t_end[(t_ex > t) as usize] = t_ex;
                    em[(t_ex > t) as usize] = interpolated_median2(alq, t_ex);
                    range_flags |= classifier.range_test(
                        t_end[0] as f64,
                        t_end[1] as f64,
                        t as f64,
                        em[0] as f64,
                        em[1] as f64,
                        xm as f64,
                    )
                }
            }

            if classifier.evaluate(t as f64, xm, range_flags) {
                return true;
            }
        }
    }
    false
}

/// Checks if a bilinear interpolation artifact will occur in between
/// two diagonally adjacent texels `a` and `d` (with `b` and `c`
/// forming the other diagonal).
///
/// # Parameters
///
/// * `classifier`: the [`ArtifactClassifier`] to use
/// * `am`: the median of `a`. This is passed in to avoid
///   recalculation, as the same value of `a` is passed in
///   many times.
/// * `a`: the color of the central texel
/// * `b`, `c`: the colors of the two texels neighboring both `a`
///   and `d`
/// * `d`: the color of the texel diagonally adjacent to `a`
pub fn has_diagonal_artifact(
    classifier: &impl ArtifactClassifier,
    am: f32,
    a: &Rgb<f32>,
    b: &Rgb<f32>,
    c: &Rgb<f32>,
    d: &Rgb<f32>,
) -> bool {
    let dm = median(*d);
    // Out of the pair, only report artifacts for the texel farthest from the edge to minimize side effects.
    if (am - 0.5).abs() >= (dm - 0.5).abs() {
        // TODO: these operations could be vectorized, and
        // has_diagonal_artifact_inner could be partially
        // vectorized as well (taking `f32x4`s for da, dbc_minus_da,
        // dd, t_ex0, and t_ex1).
        // However, stdsimd is not yet stable, and I don’t want to
        // deal with writing architecture-specific code.
        let abc = [a[0] - b[0] - c[0], a[1] - b[1] - c[1], a[2] - b[2] - c[2]];
        let l = [-a[0] - abc[0], -a[1] - abc[1], -a[2] - abc[2]];
        let q = [d[0] + abc[0], d[1] + abc[1], d[2] + abc[2]];
        let t_ex = [-0.5 * l[0] / q[0], -0.5 * l[1] / q[1], -0.5 * l[2] / q[2]];
        let alq = Alq {
            a: *a,
            l,
            q,
            am,
            dm,
        };
        has_diagonal_artifact_inner(
            classifier,
            &alq,
            a[1] - a[0],
            abc[0] - abc[1],
            d[1] - d[0],
            t_ex[0],
            t_ex[1],
        ) || has_diagonal_artifact_inner(
            classifier,
            &alq,
            a[2] - a[1],
            abc[1] - abc[2],
            d[2] - d[1],
            t_ex[1],
            t_ex[2],
        ) || has_diagonal_artifact_inner(
            classifier,
            &alq,
            a[0] - a[2],
            abc[2] - abc[0],
            d[0] - d[2],
            t_ex[2],
            t_ex[0],
        )
    } else {
        false
    }
}

#[inline]
fn interpolated_median(a: Rgb<f32>, b: Rgb<f32>, t: f32) -> f32 {
    median(a.map2(&b, |a, b| mix(a, b, t)))
}

fn interpolated_median2(alq: &Alq, t: f32) -> f32 {
    median3(
        t * (t * alq.q[0] + alq.l[0]) + alq.a[0],
        t * (t * alq.q[1] + alq.l[1]) + alq.a[1],
        t * (t * alq.q[2] + alq.l[2]) + alq.a[2],
    )
}

/// An artifact classifier that recognizes artifacts based on the
/// contents of the SDF alone.
#[derive(Debug, Clone)]
pub struct BaseArtifactClassifier {
    pub span: f64,
    pub protected: bool,
}

impl ArtifactClassifier for BaseArtifactClassifier {
    fn range_test(&self, at: f64, bt: f64, xt: f64, am: f64, bm: f64, xm: f64) -> u8 {
        if (am > 0.5 && bm > 0.5 && xm <= 0.5)
            || (am < 0.5 && bm < 0.5 && xm >= 0.5)
            || (!self.protected && median3(am, bm, xm) != xm)
        {
            let ax_span = (xt - at) * self.span;
            let bx_span = (bt - xt) * self.span;
            if !((xm - am).abs() <= ax_span && (xm - bm).abs() <= bx_span) {
                CANDIDATE | ARTIFACT
            } else {
                CANDIDATE
            }
        } else {
            0
        }
    }

    fn evaluate(&self, _t: f64, _m: f32, flags: u8) -> bool {
        (flags & ARTIFACT) != 0
    }
}

/// Evaluates the exact shape distance to find additional artifacts at a significant performance cost.
// No contour combiner support unlike msdfgen.
#[derive(Debug, Clone)]
pub struct ShapeDistanceChecker<'a, Px: Pixel<Subpixel = f32>, Sdf: GenericImageView<Pixel = Px>> {
    pub shape_coord: Point,
    pub sdf_coord: Point,
    pub msd: Rgb<f32>,
    pub protected: bool,
    prepared_shape: &'a PreparedColoredShape,
    sdf: &'a Sdf,
    inv_range: f64,
    texel_size: Vector2<f64>,
    min_improve_ratio: f64,
}

/// An artifact classifier that evaluates the exact shape distance to
/// find additional artifacts at a significant performance cost.
#[derive(Debug)]
pub struct ShapeDistanceCheckerArtifactClassifier<
    'p,
    'a,
    Px: Pixel<Subpixel = f32>,
    Sdf: GenericImageView<Pixel = Px>,
> {
    base: BaseArtifactClassifier,
    parent: &'p ShapeDistanceChecker<'a, Px, Sdf>,
    direction: Vector2<f64>,
}

impl<'a, Px: Pixel<Subpixel = f32>, Sdf: GenericImageView<Pixel = Px>>
    ShapeDistanceChecker<'a, Px, Sdf>
{
    pub fn new(
        sdf: &'a Sdf,
        prepared_shape: &'a PreparedColoredShape,
        inv_range: f64,
        min_improve_ratio: f64,
    ) -> Self {
        Self {
            shape_coord: Default::default(),
            sdf_coord: Default::default(),
            msd: Rgb([0.0; 3]),
            protected: Default::default(),
            prepared_shape,
            sdf,
            inv_range,
            texel_size: Vector2::new(1.0, 1.0),
            min_improve_ratio,
        }
    }

    pub fn classifier(
        &mut self,
        direction: Vector2<f64>,
        span: f64,
    ) -> ShapeDistanceCheckerArtifactClassifier<'_, 'a, Px, Sdf> {
        ShapeDistanceCheckerArtifactClassifier {
            base: BaseArtifactClassifier {
                span,
                protected: self.protected,
            },
            parent: self,
            direction,
        }
    }
}

impl<'p, 'a, Px: Pixel<Subpixel = f32>, Sdf: GenericImageView<Pixel = Px>> ArtifactClassifier
    for ShapeDistanceCheckerArtifactClassifier<'p, 'a, Px, Sdf>
{
    fn range_test(&self, at: f64, bt: f64, xt: f64, am: f64, bm: f64, xm: f64) -> u8 {
        self.base.range_test(at, bt, xt, am, bm, xm)
    }

    fn evaluate(&self, t: f64, _m: f32, flags: u8) -> bool {
        if (flags & CANDIDATE) != 0 {
            if (flags & ARTIFACT) != 0 {
                return true;
            }
            let t_vector = t * self.direction;
            let sdf_coord = self.parent.sdf_coord + t_vector;
            let old_msd = sample_bilinear(self.parent.sdf, sdf_coord);
            let a_weight = (1.0 - t_vector.x.abs()) * (1.0 - t_vector.y.abs());
            let a_psd = median(self.parent.msd);
            let new_msd = old_msd.to_rgb().map2(&self.parent.msd, |old_msd, msd| {
                old_msd + (a_weight * (a_psd - msd) as f64) as f32
            });
            let old_psd = median(old_msd.to_rgb());
            let new_psd = median(new_msd);
            let p = self.parent.shape_coord + t_vector.component_mul(&self.parent.texel_size);
            let ref_psd = self.parent.inv_range
                * self.parent.prepared_shape.distance4(p)[3].signed_pseudo_distance(p)
                + 0.5;
            self.parent.min_improve_ratio * (new_psd as f64 - ref_psd).abs()
                < (old_psd as f64 - ref_psd).abs()
        } else {
            false
        }
    }
}
