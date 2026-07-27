//! Functionality for preparing a shape before generating the
//! distance field, converting it into a form that allows distance
//! field generation to work faster.
//!
//! This module contains types for Bézier curves of particular
//! orders, eliminating the need to branch based on the order of a
//! [`Segment`].
//!
//! [`PreparedColoredShape`] is the prepared version of a
//! [`Shape<ColoredContour>`]. You generally don’t need to construct
//! this explicitly, as [`crate::generate::generate_sdf`] and
//! [`crate::generate::generate_msdf`] constructs one transparently.

use crate::{
    bezier::{solve_cubic_equation, solve_quadratic},
    color::Color,
    distance::{norm2, DistanceAndOrthogonality, DistanceResult, TrackedDistanceAndOrthogonality},
    shape::{ColoredContour, ColoredSegment, Contour, Shape},
};

use super::{solve_cubic_normed, Order, Point, Segment, Vect};

/// A prepared segment that supports operations for finding the signed distance.
pub trait PreparedSegment {
    /// The order of this segment type.
    const ORDER: usize;
    /// Returns the position of the curve at parameter `t`.
    fn get(&self, t: f64) -> Point;
    /// Returns the direction of the curve at the parameter `t`.
    fn direction_at(&self, t: f64) -> Vect;
    /// Returns the unsigned distance between `p` and the nearest
    /// point on the curve.
    fn distance(&self, p: Point) -> DistanceResult;

    fn make_distance_result(&self, t: f64, p: Point) -> DistanceResult {
        let q = self.get(t.clamp(0.0, 1.0));
        DistanceResult {
            distance_squared: norm2(p - q),
            parameter: t,
            point: q,
        }
    }

    /// Returns the signed distance and orthogonality between `p`
    /// and the nearest point on the curve.
    fn signed_distance(&self, p: Point) -> DistanceAndOrthogonality {
        let mut distance = self.distance(p);
        let db = self
            .direction_at(distance.parameter.clamp(0.0, 1.0))
            .normalize();
        let pmb = (distance.point - p).normalize();
        let cross = db.perp(&pmb);
        distance.distance_squared *= if cross < 0.0 { -1.0 } else { 1.0 };
        let orthogonality = cross.abs();
        DistanceAndOrthogonality {
            distance_squared: distance.distance_squared,
            parameter: distance.parameter,
            orthogonality,
        }
    }

    /// Gets the points at which the curve intersects a scanline
    /// at `y`.
    ///
    /// The return value is the number of points. If this is `n`, then:
    ///
    /// * `xs[..n]` will be filled with the *x*-coordinates of the
    ///   intersection points.
    /// * `deltas[..n]` will be filled with the changes in fill
    ///   value associated with each point: `1` for entering the
    ///   shape and `-1` for exiting it.
    fn get_scanline_points(&self, xs: &mut [f64; 3], deltas: &mut [i32; 3], y: f64) -> usize;
}

/// A prepared linear segment.
#[derive(Clone, Copy, Debug)]
pub struct PreparedLinearSegment {
    p0: Point,
    off: Vect,
}

impl PreparedLinearSegment {
    /// Converts this segment back into an unprepared form.
    ///
    /// Note that since this stores an origin point and an offset
    /// rather than two endpoints, a round trip from [`Segment`] to
    /// [`PreparedLinearSegment`] and back might not produce an
    /// exactly identical result to the starting value.
    fn to_unprepared(self) -> Segment {
        Segment::line(self.p0, self.p0 + self.off)
    }
}

impl PreparedSegment for PreparedLinearSegment {
    const ORDER: usize = 1;

    fn get(&self, t: f64) -> Point {
        self.p0 + self.off * t
    }

    fn distance(&self, p: Point) -> DistanceResult {
        let t = (p - self.p0).dot(&self.off) / self.off.dot(&self.off);
        self.make_distance_result(t, p)
    }

    fn direction_at(&self, _t: f64) -> Vect {
        self.off
    }

    fn get_scanline_points(&self, xs: &mut [f64; 3], deltas: &mut [i32; 3], y: f64) -> usize {
        let dy = y - self.p0.y;
        if dy >= 0.0 && dy < self.off.y || dy >= self.off.y && dy < 0.0 {
            let t = dy / self.off.y;
            xs[0] = self.p0.x + t * self.off.x;
            deltas[0] = if self.off.y > 0.0 { 1 } else { -1 };
            1
        } else {
            0
        }
    }
}

/// A prepared quadratic segment.
#[derive(Clone, Copy, Debug)]
pub struct PreparedQuadraticSegment {
    points: [Point; 3],
}

const BTHIRD_OVER_A_THRESHOLD: f64 = 1e6 / 3.0;

impl PreparedQuadraticSegment {
    /// Returns true if this segment is actually linear.
    fn should_be_linear(&self) -> bool {
        let pv1 = self.points[1] - self.points[0];
        let pv2 = self.points[2] - self.points[1] * 2.0 + self.points[0].coords;
        let bthird = pv1.dot(&pv2);
        let a = pv2.dot(&pv2);
        (bthird / a).abs() > BTHIRD_OVER_A_THRESHOLD
    }

    /// Converts this segment back into an unprepared form.
    fn to_unprepared(self) -> Segment {
        Segment::quad(self.points[0], self.points[1], self.points[2])
    }
}

impl PreparedSegment for PreparedQuadraticSegment {
    const ORDER: usize = 2;

    fn get(&self, t: f64) -> Point {
        self.points[0]
            + (self.points[1] - self.points[0]) * (2.0 * t)
            + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (t * t)
    }

    fn distance(&self, p: Point) -> DistanceResult {
        let pv = p - self.points[0];
        let pv1 = self.points[1] - self.points[0];
        let pv2 = self.points[2] - self.points[1] * 2.0 + self.points[0].coords;
        let ainv = pv2.dot(&pv2).recip();
        let ts = solve_cubic_normed(
            3.0 * pv1.dot(&pv2) * ainv,
            (2.0 * pv1.dot(&pv1) - pv2.dot(&pv)) * ainv,
            -pv1.dot(&pv) * ainv,
        );
        let mut d = {
            let ep_dir = self.points[1] - self.points[0];
            let mut min_distance2 = norm2(pv);
            let mut t = pv.dot(&ep_dir) / norm2(ep_dir);
            let ep_dir = self.points[2] - self.points[1];
            let p2mo = self.points[2] - p;
            let distance2 = norm2(p2mo);
            if distance2 < min_distance2 {
                min_distance2 = distance2;
                t = (p - self.points[1]).dot(&ep_dir) / norm2(ep_dir);
            }
            DistanceResult {
                distance_squared: min_distance2,
                parameter: t,
                point: self.get(t.clamp(0.0, 1.0)),
            }
        };
        for t in ts {
            if (0.0..=1.0).contains(&t) {
                let distance_result = {
                    let q = self.points[0] + pv1 * (2.0 * t) + pv2 * (t * t);
                    DistanceResult {
                        distance_squared: norm2(q - p),
                        parameter: t,
                        point: q,
                    }
                };
                d = d.min(distance_result);
            }
        }
        d
    }

    fn direction_at(&self, t: f64) -> Vect {
        (self.points[1] - self.points[0]) * 2.0
            + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (2.0 * t)
    }

    fn get_scanline_points(&self, xs: &mut [f64; 3], deltas: &mut [i32; 3], y: f64) -> usize {
        let mut total = 0;
        let mut next_dy = if y > self.points[0].y { 1 } else { -1 };
        xs[total] = self.points[0].x;
        if self.points[0].y == y {
            if (self.points[0].y, self.points[0].y) < (self.points[1].y, self.points[2].y) {
                deltas[total] = 1;
                total += 1;
            } else {
                next_dy = 1;
            }
        }
        let ab = self.points[1] - self.points[0];
        let br = self.points[2] - self.points[1] - ab;
        let mut solutions = solve_quadratic(br.y, 2.0 * ab.y, self.points[0].y - y);
        if solutions[0] > solutions[1] {
            solutions.swap(0, 1);
        }
        for t in solutions {
            if total >= 2 {
                break;
            }
            if (0.0..=1.0).contains(&t) {
                xs[total] = self.points[0].x + 2.0 * t * ab.x + t * t * br.x;
                if (next_dy as f64) * (ab.y + t * br.y) >= 0.0 {
                    deltas[total] = next_dy;
                    next_dy = -next_dy;
                    total += 1;
                }
            }
        }
        if self.points[2].y == y {
            if next_dy > 0 && total > 0 {
                total -= 1;
                next_dy = -1;
            }
            if (self.points[2].y, self.points[2].y) < (self.points[1].y, self.points[0].y)
                && total < 2
            {
                xs[total] = self.points[2].x;
                if next_dy < 0 {
                    deltas[total] = -1;
                    next_dy = 1;
                    total += 1;
                }
            }
        }
        if next_dy != if y >= self.points[2].y { 1 } else { -1 } {
            if total > 0 {
                total -= 1;
            } else {
                if (self.points[2].y - y).abs() < (self.points[0].y - y).abs() {
                    xs[total] = self.points[2].x;
                }
                deltas[total] = next_dy;
                total += 1;
            }
        }
        total
    }
}

/// A prepared cubic segment.
#[derive(Clone, Copy, Debug)]
pub struct PreparedCubicSegment {
    points: [Point; 4],
}

impl PreparedCubicSegment {
    /// Converts this segment back into an unprepared form.
    fn to_unprepared(self) -> Segment {
        Segment::cubic(
            self.points[0],
            self.points[1],
            self.points[2],
            self.points[3],
        )
    }
}

impl PreparedSegment for PreparedCubicSegment {
    const ORDER: usize = 3;

    fn get(&self, t: f64) -> Point {
        self.points[0]
            + (self.points[1] - self.points[0]) * (3.0 * t)
            + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (3.0 * t * t)
            + (self.points[3] - self.points[2] * 3.0 + (self.points[1] * 3.0 - self.points[0]))
                * (t * t * t)
    }

    fn distance(&self, p: Point) -> DistanceResult {
        // Adapted from the msdfgen implementation:
        // https://github.com/Chlumsky/msdfgen/blob/master/core/edge-segments.cpp#L191
        let pv = self.points[0] - p;
        let pv1 = self.points[1] - self.points[0];
        let pv2 = self.points[2] - self.points[1] * 2.0 + self.points[0].coords;
        let pv3 = (self.points[3] - self.points[2] * 3.0) + (self.points[1] * 3.0 - self.points[0]);

        let mut d = {
            let ep_dir = pv1;
            let mut min_distance2 = norm2(pv);
            let mut t = -pv.dot(&ep_dir) / norm2(ep_dir);
            let ep_dir = self.points[3] - self.points[2];
            let p3mo = self.points[3] - p;
            let distance2 = norm2(p3mo);
            if distance2 < min_distance2 {
                min_distance2 = distance2;
                t = (p - self.points[2]).dot(&ep_dir) / norm2(ep_dir);
            }
            DistanceResult {
                distance_squared: min_distance2,
                parameter: t,
                point: self.get(t.clamp(0.0, 1.0)),
            }
        };

        let pv1_x3 = 3.0 * pv1;
        let pv2_x3 = 3.0 * pv2;

        for i in 0..=4 {
            let mut t = (i as f64) / 4.0;
            let mut qe = pv + t * pv1_x3 + t * (t * pv2_x3) + t * (t * t * pv3);
            for _step in 0..4 {
                let d1 = pv1_x3 + 2.0 * (t * pv2_x3) + 3.0 * (t * t * pv3);
                let d2 = 2.0 * pv2_x3 + 6.0 * t * pv3;
                t -= qe.dot(&d1) / (d1.dot(&d1) + qe.dot(&d2));
                if t <= 0.0 || t >= 1.0 {
                    break;
                }
                qe = pv + t * pv1_x3 + t * (t * pv2_x3) + t * (t * t * pv3);
                // Equivalent to self.make_distance_result(t, p)
                let distance_result = DistanceResult {
                    distance_squared: norm2(qe),
                    parameter: t,
                    point: p + qe,
                };
                d = d.min(distance_result);
            }
        }

        d
    }

    fn direction_at(&self, t: f64) -> Vect {
        (self.points[1] - self.points[0]) * 3.0
            + (self.points[2] - self.points[1] * 2.0 + self.points[0].coords) * (6.0 * t)
            + (self.points[3] - self.points[2] * 3.0 + (self.points[1] * 3.0 - self.points[0]))
                * (3.0 * t * t)
    }

    fn get_scanline_points(&self, xs: &mut [f64; 3], deltas: &mut [i32; 3], y: f64) -> usize {
        let mut total = 0;
        let mut next_dy = if y > self.points[0].y { 1 } else { -1 };
        xs[total] = self.points[0].x;
        if self.points[0].y == y {
            if (self.points[0].y, self.points[0].y, self.points[0].y)
                < (self.points[1].y, self.points[2].y, self.points[3].y)
            {
                deltas[total] = 1;
                total += 1;
            } else {
                next_dy = 1;
            }
        }
        let v12 = self.points[2] - self.points[1];
        let ab = self.points[1] - self.points[0];
        let br = v12 - ab;
        let as_ = (self.points[3] - self.points[2]) - v12 - br;
        let mut solutions =
            solve_cubic_equation(as_.y, 3.0 * br.y, 3.0 * ab.y, self.points[0].y - y);
        if solutions[0] > solutions[1] {
            solutions.swap(0, 1);
        }
        if solutions[1] > solutions[2] {
            solutions.swap(1, 2);
            if solutions[0] > solutions[1] {
                solutions.swap(0, 1);
            }
        }
        for t in solutions {
            if total >= 3 {
                break;
            }
            if (0.0..=1.0).contains(&t) {
                xs[total] =
                    self.points[0].x + 3.0 * t * ab.x + 3.0 * t * t * br.x + t * t * t * as_.x;
                if (next_dy as f64) * (ab.y + 2.0 * t * br.y + t * t * as_.y) >= 0.0 {
                    deltas[total] = next_dy;
                    next_dy = -next_dy;
                    total += 1;
                }
            }
        }
        if self.points[3].y == y {
            if next_dy > 0 && total > 0 {
                total -= 1;
                next_dy = -1;
            }
            if (self.points[3].y, self.points[3].y, self.points[3].y)
                < (self.points[2].y, self.points[1].y, self.points[0].y)
                && total < 3
            {
                xs[total] = self.points[3].x;
                if next_dy < 0 {
                    deltas[total] = -1;
                    next_dy = 1;
                    total += 1;
                }
            }
        }
        if next_dy != if y >= self.points[3].y { 1 } else { -1 } {
            if total > 0 {
                total -= 1;
            } else {
                if (self.points[3].y - y).abs() < (self.points[0].y - y).abs() {
                    xs[total] = self.points[3].x;
                }
                deltas[total] = next_dy;
                total += 1;
            }
        }
        total
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PreparedSegmentResult {
    Linear(PreparedLinearSegment),
    Quadratic(PreparedQuadraticSegment),
    Cubic(PreparedCubicSegment),
}

impl Segment {
    pub(crate) fn prepare(&self) -> PreparedSegmentResult {
        match self.order {
            Order::Linear => PreparedSegmentResult::Linear(PreparedLinearSegment {
                p0: self.points[0],
                off: self.points[1] - self.points[0],
            }),
            Order::Quadratic => {
                let qs = PreparedQuadraticSegment {
                    points: [self.points[0], self.points[1], self.points[2]],
                };
                if qs.should_be_linear() {
                    PreparedSegmentResult::Linear(PreparedLinearSegment {
                        p0: self.points[0],
                        off: self.points[2] - self.points[0],
                    })
                } else {
                    PreparedSegmentResult::Quadratic(qs)
                }
            }
            Order::Cubic => PreparedSegmentResult::Cubic(PreparedCubicSegment {
                points: self.points,
            }),
        }
    }
}

/// A prepared version of [`Shape<Contour>`], which reduces
/// the number of branches taken for distance calculations.
///
/// This struct separates the segments of a shape by their order,
/// so that distances can be compared to all linear segments, then
/// to all quadratic segments, and finally to all cubic segments.
#[derive(Clone, Debug, Default)]
pub struct PreparedComponent {
    pub(crate) linears: Vec<PreparedLinearSegment>,
    pub(crate) quads: Vec<PreparedQuadraticSegment>,
    pub(crate) cubics: Vec<PreparedCubicSegment>,
}

impl PreparedComponent {
    fn add_segment(&mut self, segment: &Segment) {
        match segment.prepare() {
            PreparedSegmentResult::Linear(linear) => self.linears.push(linear),
            PreparedSegmentResult::Quadratic(quad) => self.quads.push(quad),
            PreparedSegmentResult::Cubic(cubic) => self.cubics.push(cubic),
        }
    }

    /// Returns a [`TrackedDistanceAndOrthogonality`] describing the
    /// distance to the nearest segment in this shape.
    pub fn distance(&self, point: Point) -> TrackedDistanceAndOrthogonality {
        let d_linear = self
            .linears
            .iter()
            .map(|s| (s.signed_distance(point), s))
            .min_by_key(|a| a.0);
        let d_quads = self
            .quads
            .iter()
            .map(|s| (s.signed_distance(point), s))
            .min_by_key(|a| a.0);
        let d_cubics = self
            .cubics
            .iter()
            .map(|s| (s.signed_distance(point), s))
            .min_by_key(|a| a.0);
        let mut d = TrackedDistanceAndOrthogonality::default();
        if let Some((sd, s)) = d_linear {
            if sd < d.value {
                d = TrackedDistanceAndOrthogonality {
                    value: sd,
                    segment: Some(s.to_unprepared()),
                };
            }
        }
        if let Some((sd, s)) = d_quads {
            if sd < d.value {
                d = TrackedDistanceAndOrthogonality {
                    value: sd,
                    segment: Some(s.to_unprepared()),
                };
            }
        }
        if let Some((sd, s)) = d_cubics {
            if sd < d.value {
                d = TrackedDistanceAndOrthogonality {
                    value: sd,
                    segment: Some(s.to_unprepared()),
                };
            }
        }
        d
    }
}

/// A prepared version of [`Shape<ColoredContour>`], which reduces
/// the number of branches taken for distance calculations.
///
/// In addition to separating segments by order, this struct separates
/// them by edge color, avoiding the need to branch based on the color
/// of a segment.
#[derive(Clone, Debug, Default)]
pub struct PreparedColoredShape {
    // Edges can only be of one of 4 colors
    pub(crate) components: [PreparedComponent; 4],
}

impl PreparedColoredShape {
    fn add_segment(&mut self, segment: &ColoredSegment) {
        let idx = match segment.color {
            Color::WHITE => 0,
            Color::YELLOW => 1,
            Color::CYAN => 2,
            Color::MAGENTA => 3,
            c => unreachable!("invalid color for edge: {c:?}"),
        };
        self.components[idx].add_segment(&segment.segment);
    }

    /// Returns a [`TrackedDistanceAndOrthogonality`] for each
    /// channel describing the distance to the nearest segment
    /// with that channel in this shape.
    pub fn distance3(&self, point: Point) -> [TrackedDistanceAndOrthogonality; 3] {
        let d_white = self.components[0].distance(point);
        let d_yellow = self.components[1].distance(point);
        let d_cyan = self.components[2].distance(point);
        let d_magenta = self.components[3].distance(point);
        let d1 = d_white.min(d_yellow);
        let d_red = d1.min(d_magenta);
        [
            // Red:
            d_red,
            // Green:
            d1.min(d_cyan),
            // Blue:
            d_white.min(d_cyan).min(d_magenta),
        ]
    }

    /// Returns a [`TrackedDistanceAndOrthogonality`] for each
    /// channel describing the distance to the nearest segment
    /// with that channel in this shape, as well as the distance
    /// to the nearest segment of any color as the last element of
    /// the return value.
    pub fn distance4(&self, point: Point) -> [TrackedDistanceAndOrthogonality; 4] {
        let d_white = self.components[0].distance(point);
        let d_yellow = self.components[1].distance(point);
        let d_cyan = self.components[2].distance(point);
        let d_magenta = self.components[3].distance(point);
        let d1 = d_white.min(d_yellow);
        let d_red = d1.min(d_magenta);
        [
            // Red:
            d_red,
            // Green:
            d1.min(d_cyan),
            // Blue:
            d_white.min(d_cyan).min(d_magenta),
            d_red.min(d_cyan),
        ]
    }

    // pub fn distance4t(&self, point: Point) -> [TrackedDistanceAndOrthogonality; 4] {
    //     let d_white = self.components[0].distance(point);
    //     let d_yellow = self.components[1].distance(point);
    //     let d_cyan = self.components[2].distance(point);
    //     let d_magenta = self.components[3].distance(point);
    //     dbg!(d_white);
    //     dbg!(d_yellow);
    //     dbg!(d_cyan);
    //     dbg!(d_magenta);
    //     let d1 = d_white.min(d_yellow);
    //     let d_red = d1.min(d_magenta);
    //     [
    //         // Red:
    //         d_red,
    //         // Green:
    //         d1.min(d_cyan),
    //         // Blue:
    //         d_white.min(d_cyan).min(d_magenta),
    //         d_red.min(d_cyan),
    //     ]
    // }
}

impl Shape<ColoredContour> {
    /// Constructs a [`PreparedColoredShape`] from a [`Shape<ColoredContour>`].
    pub fn prepare(&self) -> PreparedColoredShape {
        let mut prepared = PreparedColoredShape::default();
        for contour in &self.contours {
            for segment in &contour.segments {
                prepared.add_segment(segment);
            }
        }
        prepared
    }
}

impl Shape<Contour> {
    /// Constructs a [`PreparedComponent`] from a [`Shape<Contour>`].
    pub fn prepare(&self) -> PreparedComponent {
        let mut prepared = PreparedComponent::default();
        for contour in &self.contours {
            for segment in &contour.segments {
                prepared.add_segment(segment);
            }
        }
        prepared
    }
}

#[cfg(test)]
mod tests {
    use crate::bezier::{prepared::PreparedCubicSegment, Point, Vect};

    use super::{PreparedLinearSegment, PreparedQuadraticSegment, PreparedSegment};

    #[test]
    fn test_get_scanline_points_linear() {
        let mut xs = [0.0; 3];
        let mut deltas = [0; 3];
        let segment = PreparedLinearSegment {
            p0: Point::new(5.0, 5.0),
            off: Vect::new(10.0, 10.0),
        };
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 7.0);
        assert_eq!(n, 1);
        assert_eq!(xs[0], 7.0);
        assert_eq!(deltas[0], 1);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 2.0);
        assert_eq!(n, 0);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 17.4);
        assert_eq!(n, 0);
    }

    #[test]
    fn test_get_scanline_points_quad() {
        let mut xs = [0.0; 3];
        let mut deltas = [0; 3];
        let segment = PreparedQuadraticSegment {
            points: [
                Point::new(5.0, 5.0),
                Point::new(15.0, 15.0),
                Point::new(25.0, 5.0),
            ],
        };
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 7.0);
        assert_eq!(n, 2);
        assert_eq!(deltas[0], 1);
        assert_eq!(deltas[1], -1);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 3.0);
        assert_eq!(n, 0);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 152.0);
        assert_eq!(n, 0);
    }

    #[test]
    fn test_get_scanline_points_cubic() {
        let mut xs = [0.0; 3];
        let mut deltas = [0; 3];
        let segment = PreparedCubicSegment {
            points: [
                Point::new(5.0, 5.0),
                Point::new(9.0, 30.0),
                Point::new(16.0, 0.0),
                Point::new(20.0, 25.0),
            ],
        };
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 15.0);
        assert_eq!(n, 3);
        assert_eq!(deltas[0], 1);
        assert_eq!(deltas[1], -1);
        assert_eq!(deltas[2], 1);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 20.0);
        assert_eq!(n, 1);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 26.0);
        assert_eq!(n, 0);
    }

    #[test]
    fn test_get_scanline_points_cubic_2() {
        let mut xs = [0.0; 3];
        let mut deltas = [0; 3];
        let segment = PreparedCubicSegment {
            points: [
                Point::new(697.0, 1344.0),
                Point::new(635.0, 1129.0),
                Point::new(587.0, 988.0),
                Point::new(446.0, 582.0),
            ],
        };
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 1000.0);
        assert_eq!(n, 1);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 300.0);
        assert_eq!(n, 0);
        let n = segment.get_scanline_points(&mut xs, &mut deltas, 1400.0);
        assert_eq!(n, 0);
    }
}
