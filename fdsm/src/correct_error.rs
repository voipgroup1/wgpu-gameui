//! Error correction algorithms.
//!
//! The MSDF generation algorithm can produce artifacts when two
//! false edges come too close together (see Subsection 4.4.3 of
//! (Chlumský, 2015)). To mitigate this, it is possible to correct
//! areas of the distance field that might cause artifacts.
//!
//! `fdsm` uses the standard error-correction algorithm found in
//! `msdfgen` (corresponding to `msdfErrorCorrection`), except that
//! `fdsm` does not implement overlap support. Other algorithms
//! implemented by `msdfgen` (namely, the ‘shapeless’ and ‘legacy’
//! algorithms) are not currently implemented.
//!
//! Error correction is currently an experimental feature in `fdsm`
//! and might have bugs.

mod artifact_classifier;

use image::{GenericImage, GenericImageView, GrayImage, ImageBuffer, Luma, Pixel, Rgb, Rgba};
use na::Vector2;

use crate::{
    bezier::{prepared::PreparedColoredShape, Point},
    color::Color,
    render::median,
    shape::{ColoredContour, Shape},
};

use self::artifact_classifier::{
    has_diagonal_artifact, has_linear_artifact, BaseArtifactClassifier, ShapeDistanceChecker,
};

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
/// The mode of operation for the MSDF error correction pass.
pub enum ErrorCorrectionMode {
    /// Skips the error correction pass.
    Disabled,
    /// Corrects all discontinuities of the distance field regardless of whether edges are adversely affected.
    Indiscriminate,
    /// Corrects artifacts at edges and other discontinuous distances only if doing so does not affect edges or corners.
    #[default]
    EdgePriority,
    /// Only corrects artifacts at edges.
    EdgeOnly,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Configuration of whether to use an algorithm that computes the exact shape distance at the positions of suspected artifacts. This algorithm can be much slower.
pub enum DistanceCheckMode {
    /// Never computes exact shape distance.
    Never,
    /// Only computes exact shape distance at edges. Provides a good balance between speed and precision.
    #[default]
    AtEdge,
    /// Computes and compares the exact shape distance for each suspected artifact.
    Always,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
/// The configuration of the MSDF error correction pass.
pub struct ErrorCorrectionConfig {
    /// The mode of error correction.
    pub mode: ErrorCorrectionMode,
    /// Specifies when to compute exact distances.
    pub distance_check: DistanceCheckMode,
    /// The minimum ratio between the actual and maximum expected distance delta to be considered an error.
    pub min_deviation_ratio: f64,
    /// The minimum ratio between the pre-correction distance error and the post-correction distance error. Has no effect for [`DistanceCheckMode::Never`].
    pub min_improve_ratio: f64,
}

impl Default for ErrorCorrectionConfig {
    fn default() -> Self {
        Self {
            mode: Default::default(),
            distance_check: Default::default(),
            min_deviation_ratio: 10.0 / 9.0,
            min_improve_ratio: 10.0 / 9.0,
        }
    }
}

const ERROR: u8 = 1;
const PROTECTED: u8 = 2;
const PROTECTION_RADIUS_TOLERANCE: f64 = 1.001;

#[derive(Debug)]
struct ErrorCorrection<'a> {
    stencil: &'a mut GrayImage,
    config: &'a ErrorCorrectionConfig,
    inv_range: f64,
}

// Adapted from msdfgen’s implementation.

fn mix(a1: f32, a2: f32, t: f32) -> f32 {
    a1 * (1.0 - t) + a2 * t
}

fn edge_between_texels_channel(a: Rgb<f32>, b: Rgb<f32>, channel: usize) -> bool {
    let t = (a[channel] - 0.5) / (a[channel] - b[channel]);
    if t > 0.0 && t < 1.0 {
        let c = a.map2(&b, |a, b| mix(a, b, t));
        median(c) == c[channel]
    } else {
        false
    }
}

fn edge_between_texels(a: Rgb<f32>, b: Rgb<f32>) -> Color {
    let mut c = Color::BLACK;
    if edge_between_texels_channel(a, b, 0) {
        c |= Color::RED;
    }
    if edge_between_texels_channel(a, b, 1) {
        c |= Color::GREEN;
    }
    if edge_between_texels_channel(a, b, 2) {
        c |= Color::BLUE;
    }
    c
}

impl<'a> ErrorCorrection<'a> {
    unsafe fn get_stencil_pixel_unchecked(&mut self, x: u32, y: u32) -> &mut u8 {
        let index = self.stencil.width() * y + x;
        self.stencil.get_unchecked_mut(index as usize)
    }

    unsafe fn protect(&mut self, x: u32, y: u32) {
        self.stencil.unsafe_put_pixel(
            x,
            y,
            Luma([self.stencil.unsafe_get_pixel(x, y)[0] | PROTECTED]),
        )
    }

    unsafe fn mark_error(&mut self, x: u32, y: u32) {
        self.stencil
            .unsafe_put_pixel(x, y, Luma([self.stencil.unsafe_get_pixel(x, y)[0] | ERROR]))
    }

    fn protect_corners(&mut self, shape: &Shape<ColoredContour>) {
        for contour in &shape.contours {
            let segments = &contour.segments;
            for i in 0..segments.len() {
                let curr = &segments[i];
                let prev = &segments[i.checked_sub(1).unwrap_or(segments.len() - 1)];
                let common_color = prev.color & curr.color;
                if !common_color.is_bright() {
                    // This is a corner
                    let p = curr.segment.start();
                    let l = (p.x - 0.5).floor() as i64;
                    let b = (p.y - 0.5).floor() as i64;
                    let r = l + 1;
                    let t = b + 1;
                    if l < self.stencil.width() as i64
                        && b < self.stencil.height() as i64
                        && r >= 0
                        && t >= 0
                    {
                        let r = r as u32;
                        let t = t as u32;
                        unsafe {
                            if let Ok(b) = u32::try_from(b) {
                                if let Ok(l) = u32::try_from(l) {
                                    self.protect(l, b);
                                }
                                if r < self.stencil.width() {
                                    self.protect(r, b);
                                }
                            }
                            if t < self.stencil.height() {
                                if let Ok(l) = u32::try_from(l) {
                                    self.protect(l, t);
                                }
                                if r < self.stencil.width() {
                                    self.protect(r, t);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    unsafe fn protect_extreme_channels(
        &mut self,
        x: u32,
        y: u32,
        msd: Rgb<f32>,
        m: f32,
        mask: Color,
    ) {
        if (mask.has_red() && msd[0] != m)
            || (mask.has_green() && msd[1] != m)
            || (mask.has_blue() && msd[2] != m)
        {
            *self.get_stencil_pixel_unchecked(x, y) |= PROTECTED;
        }
    }

    fn protect_edges<Px: Pixel<Subpixel = f32>>(
        &mut self,
        sdf: &impl GenericImageView<Pixel = Px>,
    ) {
        let radius: f32 = (PROTECTION_RADIUS_TOLERANCE * self.inv_range) as f32;
        // Horizontal texel pairs
        for y in 0..sdf.height() {
            for x in 0..(sdf.width() - 1) {
                // Safety: These points are in-bounds.
                let left = unsafe { sdf.unsafe_get_pixel(x, y).to_rgb() };
                let right = unsafe { sdf.unsafe_get_pixel(x + 1, y).to_rgb() };
                let lm = median(left);
                let rm = median(right);
                if (lm - 0.5).abs() + (rm - 0.5).abs() < radius {
                    let mask = edge_between_texels(left, right);
                    unsafe {
                        self.protect_extreme_channels(x, y, left, lm, mask);
                        self.protect_extreme_channels(x + 1, y, right, rm, mask);
                    }
                }
            }
        }
        // Vertical texel pairs
        for y in 0..(sdf.height() - 1) {
            for x in 0..sdf.width() {
                // Safety: These points are in-bounds.
                let bottom = unsafe { sdf.unsafe_get_pixel(x, y).to_rgb() };
                let top = unsafe { sdf.unsafe_get_pixel(x, y + 1).to_rgb() };
                let bm = median(bottom);
                let tm = median(top);
                if (bm - 0.5).abs() + (tm - 0.5).abs() < radius {
                    let mask = edge_between_texels(bottom, top);
                    unsafe {
                        self.protect_extreme_channels(x, y, bottom, bm, mask);
                        self.protect_extreme_channels(x, y + 1, top, tm, mask);
                    }
                }
            }
        }
        // Diagonal texel pairs
        for y in 0..(sdf.height() - 1) {
            for x in 0..(sdf.width() - 1) {
                // Safety: These points are in-bounds.
                let lb = unsafe { sdf.unsafe_get_pixel(x, y).to_rgb() };
                let lt = unsafe { sdf.unsafe_get_pixel(x, y + 1).to_rgb() };
                let rb = unsafe { sdf.unsafe_get_pixel(x + 1, y).to_rgb() };
                let rt = unsafe { sdf.unsafe_get_pixel(x + 1, y + 1).to_rgb() };
                let mlb = median(lb);
                let mlt = median(lt);
                let mrb = median(rb);
                let mrt = median(rt);
                if (mlb - 0.5).abs() + (mrt - 0.5).abs() < radius {
                    let mask = edge_between_texels(lb, rt);
                    unsafe {
                        self.protect_extreme_channels(x, y, lb, mlb, mask);
                        self.protect_extreme_channels(x + 1, y + 1, rt, mrt, mask);
                    }
                }
                if (mrb - 0.5).abs() + (mlt - 0.5).abs() < radius {
                    let mask = edge_between_texels(rb, lt);
                    unsafe {
                        self.protect_extreme_channels(x + 1, y, rb, mrb, mask);
                        self.protect_extreme_channels(x, y + 1, lt, mlt, mask);
                    }
                }
            }
        }
    }

    fn protect_all(&mut self) {
        for pixel in self.stencil.pixels_mut() {
            pixel[0] |= PROTECTED;
        }
    }

    fn find_errors<Px: Pixel<Subpixel = f32>>(&mut self, sdf: &impl GenericImageView<Pixel = Px>) {
        let hspan = self.config.min_deviation_ratio * self.inv_range;
        let vspan = hspan;
        let dspan = hspan * 2.0f64.sqrt();
        for y in 0..sdf.height() {
            for x in 0..sdf.width() {
                unsafe {
                    // Safety: c is in bounds, and we check that
                    // the other points are in bounds.
                    let c = sdf.unsafe_get_pixel(x, y).to_rgb();
                    let cm = median(c.to_rgb());
                    let protected = (self.stencil.unsafe_get_pixel(x, y)[0] & PROTECTED) != 0;
                    let l = (x > 0).then(|| sdf.unsafe_get_pixel(x - 1, y).to_rgb());
                    let r = (x < sdf.width() - 1).then(|| sdf.unsafe_get_pixel(x + 1, y).to_rgb());
                    let b = (y > 0).then(|| sdf.unsafe_get_pixel(x, y - 1).to_rgb());
                    let t = (y < sdf.height() - 1).then(|| sdf.unsafe_get_pixel(x, y + 1).to_rgb());
                    let is_error = 'err: {
                        let hspan_class = BaseArtifactClassifier {
                            span: hspan,
                            protected,
                        };
                        let vspan_class = BaseArtifactClassifier {
                            span: vspan,
                            protected,
                        };
                        let dspan_class = BaseArtifactClassifier {
                            span: dspan,
                            protected,
                        };
                        if l.is_some_and(|l| has_linear_artifact(&hspan_class, cm, c, l))
                            || b.is_some_and(|b| has_linear_artifact(&vspan_class, cm, c, b))
                            || r.is_some_and(|r| has_linear_artifact(&hspan_class, cm, c, r))
                            || t.is_some_and(|t| has_linear_artifact(&vspan_class, cm, c, t))
                        {
                            break 'err true;
                        }
                        if let Some(l) = l {
                            if let Some(b) = b {
                                // Safety: If (x - 1, y) and (x, y - 1) are in bounds, then so is (x - 1, y - 1).
                                if has_diagonal_artifact(
                                    &dspan_class,
                                    cm,
                                    &c,
                                    &l,
                                    &b,
                                    &sdf.unsafe_get_pixel(x - 1, y - 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                            if let Some(t) = t {
                                // Safety: If (x - 1, y) and (x, y + 1) are in bounds, then so is (x - 1, y + 1).
                                if has_diagonal_artifact(
                                    &dspan_class,
                                    cm,
                                    &c,
                                    &l,
                                    &t,
                                    &sdf.unsafe_get_pixel(x - 1, y + 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                        }
                        if let Some(r) = r {
                            if let Some(b) = b {
                                // Safety: If (x + 1, y) and (x, y - 1) are in bounds, then so is (x + 1, y - 1).
                                if has_diagonal_artifact(
                                    &dspan_class,
                                    cm,
                                    &c,
                                    &r,
                                    &b,
                                    &sdf.unsafe_get_pixel(x + 1, y - 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                            if let Some(t) = t {
                                // Safety: If (x + 1, y) and (x, y + 1) are in bounds, then so is (x + 1, y + 1).
                                if has_diagonal_artifact(
                                    &dspan_class,
                                    cm,
                                    &c,
                                    &r,
                                    &t,
                                    &sdf.unsafe_get_pixel(x + 1, y + 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                        }
                        false
                    };
                    if is_error {
                        self.mark_error(x, y);
                    }
                }
            }
        }
    }

    fn find_errors_with_distance_check<Px: Pixel<Subpixel = f32>>(
        &mut self,
        sdf: &impl GenericImageView<Pixel = Px>,
        prepared_shape: &PreparedColoredShape,
    ) {
        let hspan = self.config.min_deviation_ratio * self.inv_range;
        let vspan = hspan;
        let dspan = hspan * 2.0f64.sqrt();
        let mut shape_distance_checker = ShapeDistanceChecker::new(
            sdf,
            prepared_shape,
            self.inv_range,
            self.config.min_improve_ratio,
        );

        for y in 0..sdf.height() {
            for x in 0..sdf.width() {
                unsafe {
                    let stencil = self.stencil.unsafe_get_pixel(x, y)[0];
                    if (stencil & ERROR) != 0 {
                        continue;
                    }
                    let c = sdf.unsafe_get_pixel(x, y).to_rgb();
                    shape_distance_checker.shape_coord = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                    shape_distance_checker.sdf_coord = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                    shape_distance_checker.msd = c;
                    shape_distance_checker.protected = (stencil & PROTECTED) != 0;
                    let cm = median(c);
                    let l = (x > 0).then(|| sdf.unsafe_get_pixel(x - 1, y).to_rgb());
                    let r = (x < sdf.width() - 1).then(|| sdf.unsafe_get_pixel(x + 1, y).to_rgb());
                    let b = (y > 0).then(|| sdf.unsafe_get_pixel(x, y - 1).to_rgb());
                    let t = (y < sdf.height() - 1).then(|| sdf.unsafe_get_pixel(x, y + 1).to_rgb());
                    let is_error = 'err: {
                        if l.is_some_and(|l| {
                            has_linear_artifact(
                                &shape_distance_checker.classifier(Vector2::new(-1.0, 0.0), hspan),
                                cm,
                                c,
                                l,
                            )
                        }) || b.is_some_and(|b| {
                            has_linear_artifact(
                                &shape_distance_checker.classifier(Vector2::new(0.0, -1.0), vspan),
                                cm,
                                c,
                                b,
                            )
                        }) || r.is_some_and(|r| {
                            has_linear_artifact(
                                &shape_distance_checker.classifier(Vector2::new(1.0, 0.0), hspan),
                                cm,
                                c,
                                r,
                            )
                        }) || t.is_some_and(|t| {
                            has_linear_artifact(
                                &shape_distance_checker.classifier(Vector2::new(0.0, 1.0), vspan),
                                cm,
                                c,
                                t,
                            )
                        }) {
                            break 'err true;
                        }
                        if let Some(l) = l {
                            if let Some(b) = b {
                                if has_diagonal_artifact(
                                    &shape_distance_checker
                                        .classifier(Vector2::new(-1.0, -1.0), dspan),
                                    cm,
                                    &c,
                                    &l,
                                    &b,
                                    &sdf.unsafe_get_pixel(x - 1, y - 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                            if let Some(t) = t {
                                if has_diagonal_artifact(
                                    &shape_distance_checker
                                        .classifier(Vector2::new(-1.0, 1.0), dspan),
                                    cm,
                                    &c,
                                    &l,
                                    &t,
                                    &sdf.unsafe_get_pixel(x - 1, y + 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                        }
                        if let Some(r) = r {
                            if let Some(b) = b {
                                if has_diagonal_artifact(
                                    &shape_distance_checker
                                        .classifier(Vector2::new(1.0, -1.0), dspan),
                                    cm,
                                    &c,
                                    &r,
                                    &b,
                                    &sdf.unsafe_get_pixel(x + 1, y - 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                            if let Some(t) = t {
                                if has_diagonal_artifact(
                                    &shape_distance_checker
                                        .classifier(Vector2::new(1.0, 1.0), dspan),
                                    cm,
                                    &c,
                                    &r,
                                    &t,
                                    &sdf.unsafe_get_pixel(x + 1, y + 1).to_rgb(),
                                ) {
                                    break 'err true;
                                }
                            }
                        }
                        false
                    };
                    if is_error {
                        self.mark_error(x, y);
                    }
                }
            }
        }
    }

    fn apply<Px: Pixel<Subpixel = f32>>(&self, sdf: &mut impl GenericImage<Pixel = Px>) {
        for y in 0..sdf.height() {
            for x in 0..sdf.width() {
                unsafe {
                    if (self.stencil.unsafe_get_pixel(x, y)[0] & ERROR) != 0 {
                        let mut p = sdf.unsafe_get_pixel(x, y);
                        let m = median(p.to_rgb());
                        let ch = p.channels_mut();
                        ch[0] = m;
                        ch[1] = m;
                        ch[2] = m;
                        sdf.unsafe_put_pixel(x, y, p);
                    }
                }
            }
        }
    }
}

fn get_error_correction_plan<'a, Px: Pixel<Subpixel = f32>>(
    stencil: &'a mut GrayImage,
    sdf: &impl GenericImageView<Pixel = Px>,
    shape: &Shape<ColoredContour>,
    prepared_shape: &PreparedColoredShape,
    range: f64,
    config: &'a ErrorCorrectionConfig,
) -> ErrorCorrection<'a> {
    let mut correction = ErrorCorrection {
        stencil,
        config,
        inv_range: 1.0 / range,
    };

    match config.mode {
        ErrorCorrectionMode::EdgePriority => {
            correction.protect_corners(shape);
            correction.protect_edges(sdf);
        }
        ErrorCorrectionMode::EdgeOnly => correction.protect_all(),
        _ => (),
    }
    if config.distance_check == DistanceCheckMode::Never
        || (config.distance_check == DistanceCheckMode::AtEdge
            && config.mode != ErrorCorrectionMode::EdgeOnly)
    {
        correction.find_errors(sdf);
        if config.distance_check == DistanceCheckMode::AtEdge {
            correction.protect_all();
        }
    }
    if matches!(
        config.distance_check,
        DistanceCheckMode::Always | DistanceCheckMode::AtEdge
    ) {
        correction.find_errors_with_distance_check(sdf, prepared_shape);
    }

    correction
}

fn correct_error_generic<Px: Pixel<Subpixel = f32>>(
    sdf: &mut impl GenericImage<Pixel = Px>,
    shape: &Shape<ColoredContour>,
    prepared_shape: &PreparedColoredShape,
    range: f64,
    config: &ErrorCorrectionConfig,
) {
    if config.mode == ErrorCorrectionMode::Disabled {
        return;
    }

    let mut stencil = ImageBuffer::new(sdf.width(), sdf.height());
    let correction =
        get_error_correction_plan(&mut stencil, sdf, shape, prepared_shape, range, config);

    correction.apply(sdf);
}

/// Predicts potential artifacts caused by the interpolation of the MSDF and corrects them by converting nearby texels to single-channel.
///
/// This is equivalent to msdfgen’s `msdfErrorCorrection`.
///
/// This function requires both the unprepared and prepared versions of the shape. `prepared_shape` must have been derived from `shape` by [`Shape::prepare`]. The unprepared shape is used for extracting information about corners, while the prepared version is used for distance checks. Because this function uses information about the shape, error correction must be done *before* sign correction.
///
/// `range` must be the same value as the range passed when generating the MSDF.
///
/// Error correction is supported only for `f32` channels.
pub fn correct_error_msdf(
    msdf: &mut impl GenericImage<Pixel = Rgb<f32>>,
    shape: &Shape<ColoredContour>,
    prepared_shape: &PreparedColoredShape,
    range: f64,
    config: &ErrorCorrectionConfig,
) {
    correct_error_generic(msdf, shape, prepared_shape, range, config)
}

/// Predicts potential artifacts caused by the interpolation of the MSDF and corrects them by converting nearby texels to single-channel.
///
/// This function works on MTSDFs. All other notes about [`correct_error_msdf`] apply to this function.
pub fn correct_error_mtsdf(
    mtsdf: &mut impl GenericImage<Pixel = Rgba<f32>>,
    shape: &Shape<ColoredContour>,
    prepared_shape: &PreparedColoredShape,
    range: f64,
    config: &ErrorCorrectionConfig,
) {
    correct_error_generic(mtsdf, shape, prepared_shape, range, config)
}
