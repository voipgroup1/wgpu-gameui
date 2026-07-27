//! Handling distances.
//!
//! The MSDF generation algorithm has several notions of distance:
//!
//! * *unsigned* and *signed* distances
//! * distances coupled with *orthogonality ratings* (see Section 2.4 of (Chlumský, 2015))
//! * true and pseudo-distances
//!
//! This modules provide types to describe these concepts.

use std::cmp::Ordering;

use crate::bezier::{Point, Segment, Vect};

/// The result returned from [`PreparedSegment::distance`][crate::bezier::prepared::PreparedSegment::distance].
///
/// This not only has the distance itself but also information
/// used by [`Segment::signed_distance_and_orthogonality`]
/// in calculating the orthogonality rating.
#[derive(Copy, Clone, Debug)]
pub struct DistanceResult {
    /// The squared distance from the point to the curve.
    pub distance_squared: f64,
    /// The parameter *t* at which the curve is the closest to the point.
    ///
    /// Even when computing the true distance, this can fall outside
    /// `0.0..=1.0`.
    pub parameter: f64,
    /// The point on the curve corresponding to the `parameter`.
    pub point: Point,
}

impl DistanceResult {
    #[inline]
    pub fn min(&self, p: DistanceResult) -> DistanceResult {
        let md = self.distance_squared.min(p.distance_squared);
        if md == self.distance_squared {
            *self
        } else {
            p
        }
    }
}

impl Default for DistanceResult {
    #[inline]
    fn default() -> Self {
        Self {
            distance_squared: f64::INFINITY,
            parameter: f64::NAN,
            point: Point::from([f64::NAN; 2]),
        }
    }
}

/// The result returned from [`Segment::signed_distance_and_orthogonality`].
#[derive(Copy, Clone, Debug)]
pub struct DistanceAndOrthogonality {
    /// The square of the distance from the point to the curve, times
    /// its sign.
    ///
    /// To get the signed distance itself, use [`DistanceAndOrthogonality::distance`].
    pub distance_squared: f64,
    /// The parameter *t* at which the curve is the closest to the point.
    ///
    /// Even when computing the true distance, this can fall outside
    /// `0.0..=1.0`.
    pub parameter: f64,
    /// The orthogonality rating between the point and the curve.
    ///
    /// See Section 2.4 of (Chlumský, 2015) for the definition of orthogonality.
    pub orthogonality: f64,
}

impl DistanceAndOrthogonality {
    pub fn distance(&self) -> f64 {
        if self.distance_squared >= 0.0 {
            self.distance_squared.sqrt()
        } else {
            -(-self.distance_squared).sqrt()
        }
    }
}

impl PartialEq for DistanceAndOrthogonality {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for DistanceAndOrthogonality {}

impl PartialOrd for DistanceAndOrthogonality {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DistanceAndOrthogonality {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        // What happens if one of these is NaN? YOLO!
        // Protip: make sure you compare the absolute (unsigned) distance here!
        f64::total_cmp(&self.distance_squared.abs(), &other.distance_squared.abs())
            .then(f64::total_cmp(&other.orthogonality, &self.orthogonality))
    }
}

impl Default for DistanceAndOrthogonality {
    fn default() -> Self {
        Self {
            distance_squared: f64::INFINITY,
            parameter: f64::NAN,
            orthogonality: 0.0,
        }
    }
}

/// A [`DistanceAndOrthogonality`] labeled with a reference to a segment.
///
/// Uses the underlying value for comparisons.
///
/// This is returned by [`crate::bezier::prepared::PreparedComponent::distance`],
/// which is used by [`crate::generate::generate_msdf`] because we need
/// to find the closest edge segment by true distance but output the
/// pseudo-distance to that edge in the result.
///
/// See Algorithm 7 of (Chlumský, 2015).
#[derive(Copy, Clone, Debug, Default)]
pub struct TrackedDistanceAndOrthogonality {
    pub value: DistanceAndOrthogonality,
    pub segment: Option<Segment>,
}

impl TrackedDistanceAndOrthogonality {
    pub fn signed_pseudo_distance(&self, point: Point) -> f64 {
        match self.segment {
            Some(e) => e.distance_to_pseudodistance(&self.value, point),
            None => f64::INFINITY,
        }
    }
}

impl PartialEq for TrackedDistanceAndOrthogonality {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for TrackedDistanceAndOrthogonality {}

impl PartialOrd for TrackedDistanceAndOrthogonality {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TrackedDistanceAndOrthogonality {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

/// A distance field type used in [`Segment::signed_distance_and_orthogonality`].
///
/// **This trait is no longer used by `fdsm`’s MSDF construction.**
pub trait DistanceFieldType {
    /// Clamps the parameter if applicable.
    fn clamp_t(t: f64) -> f64;
    /// Creates a default [`DistanceResult`] from the point to the
    /// segment.
    fn make_default(segment: &Segment, p: Point) -> DistanceResult;
}

/// An implementation of [`DistanceFieldType`] for true distance fields.
pub struct DistanceField;
/// An implementation of [`DistanceFieldType`] for pseudo-distance
/// fields where the curve is continued by unclamping the parameter.
///
/// Consider using [`TangentPseudoDistanceField`] instead, as it is
/// less prone to points falling on an unintended side of the curve.
/// Better yet, try using [`Segment::distance_to_pseudodistance`] if
/// you already have a [`DistanceAndOrthogonality`] for the true
/// distance, as this is cheaper than computing any type of
/// pseudodistance from scratch.
///
/// See Section 2.5 of (Chlumský, 2015) for more information.
pub struct ContinuationPseudoDistanceField;
/// An implementation of [`DistanceFieldType`] for pseudo-distance
/// fields where the curve is continued by extending it with tangent
/// lines on both endpoints.
///
/// See Section 2.5 of (Chlumský, 2015) for more information.
pub struct TangentPseudoDistanceField;

impl DistanceFieldType for DistanceField {
    fn clamp_t(t: f64) -> f64 {
        t.clamp(0.0, 1.0)
    }

    fn make_default(segment: &Segment, p: Point) -> DistanceResult {
        segment.make_base_distance_result(p)
    }
}

impl DistanceFieldType for ContinuationPseudoDistanceField {
    fn clamp_t(t: f64) -> f64 {
        t
    }

    fn make_default(_segment: &Segment, _p: Point) -> DistanceResult {
        DistanceResult::default()
    }
}

fn nearest_line_param(p0: Point, dir: Vect, p: Point) -> f64 {
    (p - p0).dot(&dir) / dir.dot(&dir)
}

impl DistanceFieldType for TangentPseudoDistanceField {
    fn clamp_t(t: f64) -> f64 {
        t.clamp(0.0, 1.0)
    }

    fn make_default(segment: &Segment, p: Point) -> DistanceResult {
        let p0 = segment.start();
        let d0 = segment.direction_at_start();
        let p1 = segment.end();
        let d1 = segment.direction_at_end();

        let t0 = nearest_line_param(p0, d0, p);
        let t1 = nearest_line_param(p1, d1, p);
        let t0 = t0.min(0.0);
        let t1 = t1.max(0.0);

        let q0 = p0 + d0 * t0;
        let dist0 = DistanceResult {
            distance_squared: norm2(p - q0),
            parameter: 0.0,
            point: q0,
        };

        let q1 = p1 + d1 * t1;
        let dist1 = DistanceResult {
            distance_squared: norm2(p - q1),
            parameter: 1.0,
            point: q1,
        };

        dist0.min(dist1)
    }
}

// faster than `v.norm_squared()`; see
// https://github.com/dimforge/nalgebra/issues/1264
pub(crate) fn norm2(v: Vect) -> f64 {
    v.dot(&v)
}
