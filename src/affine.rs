//! 2x3 affine transform for the UI transform stack.
//!
//! Stored row-major as `[[a, b, tx], [c, d, ty]]` representing
//!
//! ```text
//! x' = a * x + b * y + tx
//! y' = c * x + d * y + ty
//! ```
//!
//! The implicit bottom row is `[0, 0, 1]`. Composition is
//! `compose(other) = self * other` so `parent.compose(child)` applies the
//! child transform first, then the parent — the natural meaning for a
//! push/translate/rotate stack.
//!
//! No external math dependency: ~80 LOC of straight-line math keeps wgpu-gameui
//! self-contained.

use crate::layout::Rect;

/// 2x3 affine transform.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Affine2 {
    pub a: f32,
    pub b: f32,
    pub tx: f32,
    pub c: f32,
    pub d: f32,
    pub ty: f32,
}

impl Affine2 {
    /// Identity transform.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        tx: 0.0,
        c: 0.0,
        d: 1.0,
        ty: 0.0,
    };

    /// Construct directly from elements `[[a, b, tx], [c, d, ty]]`.
    pub const fn new(a: f32, b: f32, tx: f32, c: f32, d: f32, ty: f32) -> Self {
        Self { a, b, tx, c, d, ty }
    }

    /// Identity transform.
    pub const fn identity() -> Self {
        Self::IDENTITY
    }

    /// Pure translation.
    pub const fn translation(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            tx,
            c: 0.0,
            d: 1.0,
            ty,
        }
    }

    /// Pure rotation about origin (radians, screen-space CW since Y is down).
    pub fn rotation(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            a: c,
            b: -s,
            tx: 0.0,
            c: s,
            d: c,
            ty: 0.0,
        }
    }

    /// Non-uniform scale about origin.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            tx: 0.0,
            c: 0.0,
            d: sy,
            ty: 0.0,
        }
    }

    /// Compose `self * other` (other applied first, then self).
    pub fn compose(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            tx: self.a * other.tx + self.b * other.ty + self.tx,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            ty: self.c * other.tx + self.d * other.ty + self.ty,
        }
    }

    /// Transform a point.
    #[inline]
    pub fn transform_point(&self, p: [f32; 2]) -> [f32; 2] {
        [
            self.a * p[0] + self.b * p[1] + self.tx,
            self.c * p[0] + self.d * p[1] + self.ty,
        ]
    }

    /// Returns true if the linear part is the identity (a=d=1, b=c=0).
    /// In that case rectangle transforms remain axis-aligned and exact.
    pub fn is_translate_only(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }

    /// Returns true if there is no rotation or shear (off-diagonals are zero).
    /// Translation + axis-aligned scale only — rectangles remain axis-aligned.
    pub fn is_axis_aligned(&self) -> bool {
        self.b == 0.0 && self.c == 0.0
    }

    /// Determinant of the linear (2x2) part — the signed area scale factor.
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// Geometric-mean uniform scale factor: `sqrt(|det|)`. Useful for scaling
    /// quantities that aren't tied to a single axis (font size, line width).
    /// Equals `s` for `Affine2::scale(s, s)`, and the square root of the area
    /// scale for non-uniform `scale(sx, sy)` (so `scale(2,8)` yields ~4).
    pub fn uniform_scale(&self) -> f32 {
        self.determinant().abs().sqrt()
    }

    /// Transform the four corners of a rect and return its axis-aligned
    /// bounding box. For translate-only or axis-aligned-scale transforms this
    /// is exact; otherwise it is the AABB of the rotated/sheared quad.
    pub fn transform_rect_aabb(&self, rect: Rect) -> Rect {
        let p0 = self.transform_point([rect.x, rect.y]);
        let p1 = self.transform_point([rect.x + rect.width, rect.y]);
        let p2 = self.transform_point([rect.x + rect.width, rect.y + rect.height]);
        let p3 = self.transform_point([rect.x, rect.y + rect.height]);

        let min_x = p0[0].min(p1[0]).min(p2[0]).min(p3[0]);
        let min_y = p0[1].min(p1[1]).min(p2[1]).min(p3[1]);
        let max_x = p0[0].max(p1[0]).max(p2[0]).max(p3[0]);
        let max_y = p0[1].max(p1[1]).max(p2[1]).max(p3[1]);

        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    /// The inverse transform: `self.compose(&self.inverse())` is the identity.
    /// Returns [`IDENTITY`](Self::IDENTITY) for a degenerate (zero-determinant)
    /// transform rather than producing NaNs.
    ///
    /// Maps world-space coordinates back into the current local frame — e.g. a
    /// widget handed a world-space rect that the draw list will re-transform
    /// needs the *local* rect (and local mouse) to draw and hit-test correctly.
    pub fn inverse(&self) -> Self {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return Self::IDENTITY;
        }
        let inv_det = 1.0 / det;
        let ia = self.d * inv_det;
        let ib = -self.b * inv_det;
        let ic = -self.c * inv_det;
        let id = self.a * inv_det;
        Self {
            a: ia,
            b: ib,
            tx: -(ia * self.tx + ib * self.ty),
            c: ic,
            d: id,
            ty: -(ic * self.tx + id * self.ty),
        }
    }

    /// Transform the four corners of a rect, returning them in TL, TR, BR, BL
    /// order (matching the per-vertex order used by `quad`/`icon`).
    pub fn transform_rect_corners(&self, rect: Rect) -> [[f32; 2]; 4] {
        [
            self.transform_point([rect.x, rect.y]),
            self.transform_point([rect.x + rect.width, rect.y]),
            self.transform_point([rect.x + rect.width, rect.y + rect.height]),
            self.transform_point([rect.x, rect.y + rect.height]),
        ]
    }
}

impl Default for Affine2 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn point_approx(a: [f32; 2], b: [f32; 2]) -> bool {
        approx_eq(a[0], b[0]) && approx_eq(a[1], b[1])
    }

    #[test]
    fn identity_compose_is_identity() {
        let a = Affine2::identity();
        let b = a.compose(&Affine2::identity());
        assert_eq!(a, b);
        assert_eq!(a.transform_point([3.0, 5.0]), [3.0, 5.0]);
    }

    #[test]
    fn translation_transforms_point() {
        let t = Affine2::translation(10.0, 20.0);
        assert_eq!(t.transform_point([1.0, 2.0]), [11.0, 22.0]);
    }

    #[test]
    fn scale_transforms_point() {
        let s = Affine2::scale(2.0, 3.0);
        assert_eq!(s.transform_point([4.0, 5.0]), [8.0, 15.0]);
    }

    #[test]
    fn inverse_roundtrips_translate_scale() {
        // translate then scale (the kind of stack the UI builds)
        let t = Affine2::translation(10.0, 20.0).compose(&Affine2::scale(2.0, 4.0));
        let inv = t.inverse();
        let p = [3.0, 5.0];
        let back = inv.transform_point(t.transform_point(p));
        assert!(point_approx(back, p), "got {back:?}");
        // compose(inverse) == identity
        let id = t.compose(&inv);
        assert!(point_approx(id.transform_point([7.0, -2.0]), [7.0, -2.0]));
    }

    #[test]
    fn inverse_of_identity_is_identity() {
        assert_eq!(Affine2::IDENTITY.inverse(), Affine2::IDENTITY);
    }

    #[test]
    fn inverse_of_degenerate_is_identity() {
        assert_eq!(Affine2::scale(0.0, 0.0).inverse(), Affine2::IDENTITY);
    }

    #[test]
    fn rotation_90_degrees_around_origin() {
        let r = Affine2::rotation(std::f32::consts::FRAC_PI_2);
        // (1, 0) → (0, 1) under our screen-space CW-positive rotation.
        let p = r.transform_point([1.0, 0.0]);
        assert!(point_approx(p, [0.0, 1.0]));
        let q = r.transform_point([0.0, 1.0]);
        assert!(point_approx(q, [-1.0, 0.0]));
    }

    #[test]
    fn translate_then_scale_order() {
        // `translate * scale` should scale first, then translate.
        let composed = Affine2::translation(10.0, 20.0).compose(&Affine2::scale(2.0, 2.0));
        let p = composed.transform_point([3.0, 4.0]);
        assert!(point_approx(p, [16.0, 28.0]));
    }

    #[test]
    fn scale_then_translate_order() {
        // `scale * translate` translates first, then scales the translated origin.
        let composed = Affine2::scale(2.0, 2.0).compose(&Affine2::translation(10.0, 20.0));
        let p = composed.transform_point([3.0, 4.0]);
        // Inner: (3+10, 4+20) = (13, 24). Outer scale: (26, 48).
        assert!(point_approx(p, [26.0, 48.0]));
    }

    #[test]
    fn translate_rotate_scale_compose() {
        // Build a translate * rotate(90) * scale(2,2) and verify a point.
        let m = Affine2::translation(100.0, 50.0)
            .compose(&Affine2::rotation(std::f32::consts::FRAC_PI_2))
            .compose(&Affine2::scale(2.0, 2.0));
        // (1,0) -> scale -> (2,0) -> rotate90 -> (0,2) -> translate -> (100,52)
        let p = m.transform_point([1.0, 0.0]);
        assert!(point_approx(p, [100.0, 52.0]));
    }

    #[test]
    fn rect_aabb_under_rotation() {
        let r = Affine2::rotation(std::f32::consts::FRAC_PI_2);
        let rect = Rect::new(0.0, 0.0, 10.0, 4.0);
        let aabb = r.transform_rect_aabb(rect);
        // After 90° CW rotation, width and height swap.
        assert!(approx_eq(aabb.width, 4.0));
        assert!(approx_eq(aabb.height, 10.0));
    }

    #[test]
    fn rect_aabb_translate_is_exact() {
        let t = Affine2::translation(5.0, 7.0);
        let rect = Rect::new(1.0, 2.0, 10.0, 20.0);
        let aabb = t.transform_rect_aabb(rect);
        assert!(approx_eq(aabb.x, 6.0));
        assert!(approx_eq(aabb.y, 9.0));
        assert!(approx_eq(aabb.width, 10.0));
        assert!(approx_eq(aabb.height, 20.0));
    }

    #[test]
    fn axis_aligned_flags() {
        assert!(Affine2::identity().is_translate_only());
        assert!(Affine2::identity().is_axis_aligned());
        assert!(Affine2::translation(3.0, 4.0).is_translate_only());
        assert!(!Affine2::scale(2.0, 2.0).is_translate_only());
        assert!(Affine2::scale(2.0, 2.0).is_axis_aligned());
        assert!(!Affine2::rotation(0.5).is_axis_aligned());
    }

    #[test]
    fn corners_in_tl_tr_br_bl_order() {
        let m = Affine2::translation(100.0, 200.0);
        let corners = m.transform_rect_corners(Rect::new(0.0, 0.0, 10.0, 5.0));
        assert_eq!(corners[0], [100.0, 200.0]); // TL
        assert_eq!(corners[1], [110.0, 200.0]); // TR
        assert_eq!(corners[2], [110.0, 205.0]); // BR
        assert_eq!(corners[3], [100.0, 205.0]); // BL
    }

    #[test]
    fn uniform_scale_returns_geometric_mean() {
        // identity → 1.0
        assert!(approx_eq(Affine2::identity().uniform_scale(), 1.0));
        // uniform scale s → s
        assert!(approx_eq(Affine2::scale(3.0, 3.0).uniform_scale(), 3.0));
        // non-uniform → sqrt(|det|): scale(2,8) → sqrt(16) = 4
        assert!(approx_eq(Affine2::scale(2.0, 8.0).uniform_scale(), 4.0));
        // rotation preserves area, so uniform_scale stays 1.0
        assert!(approx_eq(Affine2::rotation(0.7).uniform_scale(), 1.0));
        // translation has no effect on the linear part
        assert!(approx_eq(
            Affine2::translation(50.0, 50.0).uniform_scale(),
            1.0
        ));
    }
}
