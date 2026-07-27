//! Handling of Bézier curves as needed for MSDF generation.

pub mod prepared;
pub mod scanline;

use std::fmt::{Debug, Display};

use na::Affine2;
use nalgebra::{Point2, Vector2};

use crate::{
    distance::{norm2, DistanceAndOrthogonality, DistanceFieldType, DistanceResult},
    transform::Transform,
};

use self::prepared::{PreparedSegment, PreparedSegmentResult};

/// A type alias for the point type used.
pub type Point = Point2<f64>;
/// A type alias for the vector type used.
pub type Vect = Vector2<f64>;

/// A supported order of a Bézier curve.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Order {
    /// Indicates a linear segment with two control points.
    Linear,
    /// Indicates a quadratic Bézier curve with three control points.
    Quadratic,
    /// Indicates a cubic Bézier curve with four control points.
    Cubic,
}

/// The orientation of one direction relative to another.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Orientation {
    Counterclockwise,
    Clockwise,
}

impl Orientation {
    /// Creates an orientation from the perpendicular product `a.perp(b)`,
    /// where `a` and `b` are vectors corresponding to the directions
    /// before and after the turn, respectively.
    #[inline]
    pub fn from_perp_product(pp: f64) -> Self {
        if pp < 0.0 {
            Orientation::Clockwise
        } else {
            Orientation::Counterclockwise
        }
    }
}

/// A Bézier curve making up a segment of a shape.
#[derive(Copy, Clone)]
pub struct Segment {
    points: [Point; 4],
    order: Order,
}

impl Segment {
    /// Creates a new line segment.
    #[inline]
    pub fn line(x0: Point, x1: Point) -> Self {
        Self {
            points: [x0, x1, Point::default(), Point::default()],
            order: Order::Linear,
        }
    }

    /// Creates a new quadratic Bézier segment.
    #[inline]
    pub fn quad(x0: Point, x1: Point, x2: Point) -> Self {
        Self {
            points: [x0, x1, x2, Point::default()],
            order: Order::Quadratic,
        }
    }

    /// Creates a new cubic Bézier segment.
    #[inline]
    pub fn cubic(x0: Point, x1: Point, x2: Point, x3: Point) -> Self {
        Self {
            points: [x0, x1, x2, x3],
            order: Order::Cubic,
        }
    }

    /// Returns the order of the curve.
    #[inline]
    pub fn order(&self) -> Order {
        self.order
    }

    pub(crate) fn order_int(&self) -> usize {
        (self.order as usize) + 1
    }

    /// Returns the control point at index `index`.
    #[inline]
    pub fn control_point(&self, index: usize) -> Point {
        self.points[index]
    }

    /// Returns the starting point of the curve.
    ///
    /// This is faster than `self.get(0.0)`.
    #[inline]
    pub fn start(&self) -> Point {
        self.points[0]
    }

    /// Returns the ending point of the curve.
    ///
    /// This is faster than `self.get(1.0)`.
    #[inline]
    pub fn end(&self) -> Point {
        self.points[self.order_int()]
    }

    fn get_linear(&self, t: f64) -> Point {
        self.points[0] + (self.points[1] - self.points[0]) * t
    }

    fn get_quadratic(&self, t: f64) -> Point {
        self.points[0]
            + (self.points[1] - self.points[0]) * (2.0 * t)
            + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (t * t)
    }

    fn get_cubic(&self, t: f64) -> Point {
        self.points[0]
            + (self.points[1] - self.points[0]) * (3.0 * t)
            + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (3.0 * t * t)
            + (self.points[3] - self.points[2] * 3.0 + (self.points[1] * 3.0 - self.points[0]))
                * (t * t * t)
    }

    /// Returns the position of the curve at parameter `t`.
    ///
    /// If `t` is `0.0` or `1.0`, then consider using [`Segment::start`] or [`Segment::end`] instead.
    pub fn get(&self, t: f64) -> Point {
        match self.order {
            Order::Linear => self.get_linear(t),
            Order::Quadratic => self.get_quadratic(t),
            Order::Cubic => self.get_cubic(t),
        }
    }

    /// Returns the direction of the curve at its starting point.
    ///
    /// This is faster than `self.direction_at(0.0)` but differs from it by
    /// a constant factor.
    #[inline]
    pub fn direction_at_start(&self) -> Vect {
        self.points[1] - self.points[0]
    }

    /// Returns the direction of the curve at its ending point.
    ///
    /// This is faster than `self.direction_at(1.0)` but differs from it by
    /// a constant factor.
    #[inline]
    pub fn direction_at_end(&self) -> Vect {
        let o = self.order_int();
        self.points[o] - self.points[o - 1]
    }

    /// Returns the direction of the curve at the parameter `t`.
    pub fn direction_at(&self, t: f64) -> Vect {
        match self.order {
            Order::Linear => self.points[1] - self.points[0],
            Order::Quadratic => {
                (self.points[1] - self.points[0]) * 2.0
                    + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (2.0 * t)
            }
            Order::Cubic => {
                (self.points[1] - self.points[0]) * 3.0
                    + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (6.0 * t)
                    + (self.points[3] - self.points[2] * 3.0
                        + (self.points[1] * 3.0 - self.points[0]))
                        * (3.0 * t * t)
            }
        }
    }

    /// If the intersection between `self` and `next` forms a sharp corner, then
    /// returns the orientation. Otherwise, returns [`None`].
    ///
    /// `sin_alpha` is the sine of the maximum angle at which a corner is
    /// considered ‘sharp’.
    pub fn corners_into(&self, next: &Segment, sin_alpha: f64) -> Option<Orientation> {
        let a = self.direction_at_end();
        let b = next.direction_at_start();
        forms_corner(&a, &b, sin_alpha)
    }

    pub(crate) fn make_base_distance_result(&self, p: Point) -> DistanceResult {
        let dr0 = {
            let q = self.start();
            DistanceResult {
                distance_squared: norm2(p - q),
                parameter: -f64::INFINITY,
                point: q,
            }
        };
        let dr1 = {
            let q = self.end();
            DistanceResult {
                distance_squared: norm2(p - q),
                parameter: f64::INFINITY,
                point: q,
            }
        };
        dr0.min(dr1)
    }

    /// Gets the signed distance of a point from this curve, as well as the
    /// orthogonality rating.
    ///
    /// See Section 2.4 of (Chlumský, 2015) for the definition of orthogonality.
    pub fn signed_distance_and_orthogonality<Df: DistanceFieldType>(
        &self,
        p: Point,
    ) -> DistanceAndOrthogonality {
        match self.prepare() {
            PreparedSegmentResult::Linear(s) => s.signed_distance(p),
            PreparedSegmentResult::Quadratic(s) => s.signed_distance(p),
            PreparedSegmentResult::Cubic(s) => s.signed_distance(p),
        }
    }

    pub fn distance_to_pseudodistance(&self, distance: &DistanceAndOrthogonality, p: Point) -> f64 {
        if distance.parameter < 0.0 {
            let dir = self.direction_at_start().normalize();
            let aq = p - self.start();
            let ts = aq.dot(&dir);
            if ts < 0.0 {
                let pseudodistance = aq.perp(&dir);
                if pseudodistance * pseudodistance <= distance.distance_squared.abs() {
                    return pseudodistance;
                }
            }
        } else if distance.parameter > 1.0 {
            let dir = self.direction_at_end().normalize();
            let bq = p - self.end();
            let ts = bq.dot(&dir);
            if ts > 0.0 {
                let pseudodistance = bq.perp(&dir);
                if pseudodistance * pseudodistance <= distance.distance_squared.abs() {
                    return pseudodistance;
                }
            }
        }
        distance.distance()
    }

    /// Adjusts the start point of this curve so that its appearance is not
    /// altered any more than necessary.
    ///
    /// See Subsection 3.1.2 of (Chlumský, 2015) for more information.
    pub fn adjust_start_point(&mut self, p0p: Point) {
        match self.order {
            Order::Linear => (),
            Order::Quadratic => {
                self.points[1] += (self.points[2] - self.points[1])
                    * (({
                        let a: &Vect = &(self.points[0] - self.points[1]);
                        let b: &Vect = &(p0p - self.points[0]);
                        a.perp(b)
                    }) / {
                        let a: &Vect = &(self.points[0] - self.points[1]);
                        let b: &Vect = &(self.points[2] - self.points[1]);
                        a.perp(b)
                    })
            }
            Order::Cubic => {
                self.points[1] += p0p - self.points[0];
            }
        }
        self.points[0] = p0p;
    }

    /// Adjusts the end point of this curve so that its appearance is not
    /// altered any more than necessary.
    ///
    /// See Subsection 3.1.2 of (Chlumský, 2015) for more information.
    pub fn adjust_end_point(&mut self, pxp: Point) {
        match self.order {
            Order::Linear => (),
            Order::Quadratic => {
                self.points[1] += (self.points[0] - self.points[1])
                    * (({
                        let a: &Vect = &(self.points[2] - self.points[1]);
                        let b: &Vect = &(pxp - self.points[2]);
                        a.perp(b)
                    }) / {
                        let a: &Vect = &(self.points[2] - self.points[1]);
                        let b: &Vect = &(self.points[0] - self.points[1]);
                        a.perp(b)
                    })
            }
            Order::Cubic => {
                self.points[2] += pxp - self.points[3];
            }
        }
        self.points[self.order_int()] = pxp;
    }

    /// Splits this curve into thirds, at the points corresponding to *t* = 1/3
    /// and *t* = 2/3.
    // Based on https://github.com/Chlumsky/msdfgen/blob/master/core/edge-segments.cpp#L465
    pub fn split_in_thirds(&self) -> [Self; 3] {
        match self.order {
            Order::Linear => {
                let a = self.get(1.0 / 3.0);
                let b = self.get(2.0 / 3.0);
                [
                    Self::line(self.points[0], a),
                    Self::line(a, b),
                    Self::line(b, self.points[1]),
                ]
            }
            Order::Quadratic => {
                let a = self.get(1.0 / 3.0);
                let b = self.get(2.0 / 3.0);
                [
                    Self::quad(
                        self.points[0],
                        self.points[0].lerp(&self.points[1], 1.0 / 3.0),
                        a,
                    ),
                    Self::quad(
                        a,
                        Point::lerp(
                            &self.points[0].lerp(&self.points[1], 5.0 / 9.0),
                            &self.points[1].lerp(&self.points[2], 4.0 / 9.0),
                            0.5,
                        ),
                        b,
                    ),
                    Self::quad(
                        b,
                        self.points[1].lerp(&self.points[2], 2.0 / 3.0),
                        self.points[2],
                    ),
                ]
            }
            Order::Cubic => {
                let a = self.get(1.0 / 3.0);
                let b = self.get(2.0 / 3.0);
                let c1a = self.points[0].lerp(&self.points[1], 1.0 / 3.0);
                let c1b = self.points[0].lerp(&self.points[1], 2.0 / 3.0);
                let c2a = self.points[1].lerp(&self.points[2], 1.0 / 3.0);
                let c2b = self.points[1].lerp(&self.points[2], 2.0 / 3.0);
                let c3a = self.points[2].lerp(&self.points[3], 1.0 / 3.0);
                let c3b = self.points[2].lerp(&self.points[3], 2.0 / 3.0);
                [
                    Self::cubic(
                        self.points[0],
                        if self.points[0] == self.points[1] {
                            self.points[0]
                        } else {
                            c1a
                        },
                        c1a.lerp(&c2a, 1.0 / 3.0),
                        a,
                    ),
                    Self::cubic(
                        a,
                        Point::lerp(
                            &c1a.lerp(&c2a, 1.0 / 3.0),
                            &c2a.lerp(&c3a, 1.0 / 3.0),
                            2.0 / 3.0,
                        ),
                        Point::lerp(
                            &c1b.lerp(&c2b, 2.0 / 3.0),
                            &c2b.lerp(&c3b, 2.0 / 3.0),
                            1.0 / 3.0,
                        ),
                        b,
                    ),
                    Self::cubic(
                        b,
                        c2b.lerp(&c3b, 2.0 / 3.0),
                        if self.points[2] == self.points[3] {
                            self.points[3]
                        } else {
                            c3b
                        },
                        self.points[3],
                    ),
                ]
            }
        }
    }
}

impl Transform for Segment {
    fn transform(&mut self, transformation: &Affine2<f64>) {
        self.points
            .iter_mut()
            .for_each(|p| *p = transformation.transform_point(p));
    }
}

impl Debug for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.points[0], f)?;
        for i in 1..=self.order_int() {
            f.write_str(" -- ")?;
            Display::fmt(&self.points[i], f)?;
        }
        Ok(())
    }
}

fn forms_corner(a: &Vect, b: &Vect, sin_alpha: f64) -> Option<Orientation> {
    if a.dot(b) <= 0.0 {
        let axb = a.perp(b);
        return Some(Orientation::from_perp_product(axb));
    }
    let a_norm = a.normalize();
    let b_norm = b.normalize();
    let axb_norm = a_norm.perp(&b_norm);
    if axb_norm.abs() <= sin_alpha {
        None
    } else {
        Some(Orientation::from_perp_product(axb_norm))
    }
}

pub(crate) fn solve_quadratic(a: f64, b: f64, c: f64) -> [f64; 2] {
    if a == 0.0 || b.abs() > 1e12 * a.abs() {
        // bx + c = 0 ⇒ x = -c/b
        return [-c / b, f64::NAN];
    }
    let bhalf = 0.5 * b;
    let d = bhalf * bhalf - a * c;
    if d == 0.0 {
        [-bhalf / a, f64::NAN]
    } else {
        [(-bhalf - d.sqrt()) / a, (-bhalf + d.sqrt()) / a]
    }
}

// If there are fewer than 3 real roots, then return NAN for the gaps.
fn solve_cubic_equation(a: f64, b: f64, c: f64, d: f64) -> [f64; 3] {
    if a == 0.0 || (b / a).abs() > 1e6 {
        // a = 0 => find roots of quadratic polynomial
        let (a, b, c) = (b, c, d);
        if (b / a).abs() > 1e12 {
            if b == 0.0 {
                if c == 0.0 {
                    return [0.0, f64::NAN, f64::NAN];
                }
                return [f64::NAN; 3];
            }
            return [-c / b, f64::NAN, f64::NAN];
        }
        let d = b * b - 4.0 * a * c;
        if d.abs() == 0.0 {
            return [-b / (2.0 * a), f64::NAN, f64::NAN];
        }
        let sd = d.sqrt();
        let r1 = (-b + sd) / (2.0 * a);
        let r2 = (-b - sd) / (2.0 * a);
        return [r1, r2, f64::NAN];
    }
    // We divide the polynomial by `a` first, then derive the depressed cubic equation
    solve_cubic_normed(b / a, c / a, d / a)
}

const SQRT_3_OVER_2: f64 = 0.8660254037844386;

fn solve_cubic_normed(a: f64, b: f64, c: f64) -> [f64; 3] {
    let a2 = a * a;
    let q = (1.0 / 9.0) * (a2 - 3.0 * b);
    let r = (1.0 / 54.0) * (a * (2.0 * a2 - 9.0 * b) + 27.0 * c);
    let r2 = r * r;
    let q3 = q * q * q;
    let a = a * (1.0 / 3.0);
    if r2 < q3 {
        let t = r / q3.sqrt();
        let t = t.clamp(-1.0, 1.0);
        let t = t.acos();
        let q = -2.0 * q.sqrt();
        let (sin_t_over_3, cos_t_over_3) = (t * (1.0 / 3.0)).sin_cos();
        [
            q * cos_t_over_3 - a,
            q * (-0.5 * cos_t_over_3 - SQRT_3_OVER_2 * sin_t_over_3) - a,
            q * (-0.5 * cos_t_over_3 + SQRT_3_OVER_2 * sin_t_over_3) - a,
        ]
    } else {
        let u = (if r < 0.0 { 1.0 } else { -1.0 }) * (r.abs() + (r2 - q3).sqrt()).cbrt();
        let v = if u == 0.0 { 0.0 } else { q / u };
        let x0 = (u + v) - a;
        if u == v || (u - v).abs() < 1e-12 * (u + v).abs() {
            [x0, -0.5 * (u + v) - a, f64::NAN]
        } else {
            [x0, f64::NAN, f64::NAN]
        }
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::solve_cubic_normed;

    fn test_cubic_solver(b: f64, c: f64, d: f64) {
        let roots = solve_cubic_normed(b, c, d);
        assert!(
            roots.into_iter().any(|x| !x.is_nan()),
            "roots are all NaN: {roots:?}"
        );
        for x in roots {
            if !x.is_nan() {
                assert_abs_diff_eq!(x * x * x + b * x * x + c * x + d, 0.0, epsilon = 1e-6);
            }
        }
    }

    #[test]
    fn test_cubic_solver_1() {
        test_cubic_solver(0.0, 0.0, 0.0);
        // Randomly generated test cases:
        // for ^20 { say "test_cubic_solver({((2 * rand - 1) xx 3).join(", ")});" }
        test_cubic_solver(0.3366309188552723, 0.2968678338004016, -0.45437712050737145);
        test_cubic_solver(
            0.005961108855410346,
            0.36064715388921664,
            -0.9181089716195059,
        );
        test_cubic_solver(
            -0.041673749833492035,
            -0.4144965570930039,
            -0.3017471736175603,
        );
        test_cubic_solver(0.3921596572892776, -0.4171063191109201, -0.9730479794987696);
        test_cubic_solver(0.9103752871473376, -0.3370719110109077, 0.7947230638180556);
        test_cubic_solver(
            -0.9644115186378965,
            -0.6862770032446657,
            0.46871720990960064,
        );
        test_cubic_solver(-0.5119349373007263, -0.1371117290817132, 0.5879584458335287);
        test_cubic_solver(0.1908317920730953, -0.2934171097507887, 0.11979014750552297);
        test_cubic_solver(0.6737233914013809, 0.08687681073473597, 0.35367327179607044);
        test_cubic_solver(
            -0.5331320725210258,
            -0.41819223907379954,
            -0.45606714343117405,
        );
        test_cubic_solver(0.5765938267116546, -0.788306894014297, 0.3068988990001891);
        test_cubic_solver(
            -0.7830048326424466,
            0.08224860064650041,
            -0.6590056707633136,
        );
        test_cubic_solver(
            -0.9935849280507334,
            0.3686779714167856,
            -0.26920957528492884,
        );
        test_cubic_solver(
            -0.2973202050918542,
            -0.004898933842882647,
            0.7716311082768601,
        );
        test_cubic_solver(0.7546640249581902, 0.06163338017403208, 0.8585476748019603);
        test_cubic_solver(
            -0.8587886209771709,
            -0.5086080308532517,
            -0.35186073213973623,
        );
        test_cubic_solver(-0.502191723419311, 0.3986058839390041, -0.661036719382766);
        test_cubic_solver(0.1998100083294403, 0.8334901085415585, 0.5188547664566809);
        test_cubic_solver(
            0.9114986847314581,
            -0.09210522774403729,
            0.05709995880308272,
        );
        test_cubic_solver(
            0.13019707704570238,
            -0.38582087978099255,
            -0.29275431447857914,
        );
    }
}
