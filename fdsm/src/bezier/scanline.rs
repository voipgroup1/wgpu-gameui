//! Functionality for getting scanlines of a shape.

use super::prepared::{PreparedColoredShape, PreparedComponent, PreparedSegment};

/// Indicates which fill values are interpreted as being inside or
/// outside of a shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FillRule {
    /// Treat any nonzero fill value as filled.
    Nonzero,
    /// Treat odd fill values as filled.
    Odd,
    /// Treat positive fill values as filled.
    Positive,
    /// Treat negative fill values as filled.
    Negative,
}

impl FillRule {
    /// Returns true if this fill rule treats the given fill value
    /// as filled.
    pub fn fills(self, intersections: i32) -> bool {
        match self {
            FillRule::Nonzero => intersections != 0,
            FillRule::Odd => intersections % 2 != 0,
            FillRule::Positive => intersections > 0,
            FillRule::Negative => intersections < 0,
        }
    }
}

/// A description of the intersections between a shape and a horizontal line.
///
/// Each intersection increases or decreases the intersection count
/// by one, depending on the direction from the point that goes
/// “inside” the shape. Thus, each *x*-coordinate is associated with
/// a fill value, which is interpreted as inside or outside the shape
/// by a [fill rule][FillRule].
#[derive(Clone, Debug)]
pub struct Scanline {
    xs: Vec<f64>,
    totals: Vec<i32>,
}

/// A cursor that can navigate through a [scanline][Scanline].
#[derive(Clone, Debug)]
pub struct ScanlineCursor<'a> {
    scanline: &'a Scanline,
    index: usize,
}

impl Scanline {
    /// Creates a scanline from a list of `(x, delta)` values,
    /// where `delta` indicates the change in fill value.
    fn new(mut points: Vec<(f64, i32)>) -> Self {
        points.sort_unstable_by_key(|(x, _)| x.to_bits());
        let (xs, mut totals): (Vec<f64>, Vec<i32>) = points.into_iter().unzip();
        let mut total = 0;
        for e in &mut totals {
            total += *e;
            *e = total;
        }
        Self { xs, totals }
    }

    /// Creates a new [`ScanlineCursor`] that refers to this scanline.
    pub fn cursor(&self) -> ScanlineCursor<'_> {
        ScanlineCursor {
            scanline: self,
            index: 0,
        }
    }
}

impl<'a> ScanlineCursor<'a> {
    fn move_to(&mut self, x: f64) -> Option<usize> {
        if self.scanline.xs.is_empty() {
            return None;
        }
        if x < self.scanline.xs[self.index] {
            while x < self.scanline.xs[self.index] {
                if self.index == 0 {
                    return None;
                }
                self.index -= 1;
            }
        } else {
            while self.index < self.scanline.xs.len() - 1 && x >= self.scanline.xs[self.index + 1] {
                self.index += 1;
            }
        }
        Some(self.index)
    }

    /// Returns the fill value at a given *x*-coordinate.
    pub fn sum_intersections(&mut self, x: f64) -> i32 {
        self.move_to(x).map_or(0, |i| self.scanline.totals[i])
    }

    /// Returns whether the shape is filled at agiven *x*-coordinate
    /// according to the given [`FillRule`].
    pub fn filled(&mut self, x: f64, fill_rule: FillRule) -> bool {
        fill_rule.fills(self.sum_intersections(x))
    }
}

impl PreparedComponent {
    /// Creates a [`Scanline`] from this shape.
    pub fn scanline(&self, y: f64) -> Scanline {
        let mut points = Vec::new();
        self.insert_scanline_points(y, &mut points);
        Scanline::new(points)
    }

    fn insert_scanline_points(&self, y: f64, points: &mut Vec<(f64, i32)>) {
        let mut xs_tmp = [0.0; 3];
        let mut deltas_tmp = [0; 3];
        for linear in &self.linears {
            let n = linear.get_scanline_points(&mut xs_tmp, &mut deltas_tmp, y);
            for i in 0..n {
                points.push((xs_tmp[i], deltas_tmp[i]));
            }
        }
        for quad in &self.quads {
            let n = quad.get_scanline_points(&mut xs_tmp, &mut deltas_tmp, y);
            for i in 0..n {
                points.push((xs_tmp[i], deltas_tmp[i]));
            }
        }
        for cubic in &self.cubics {
            let n = cubic.get_scanline_points(&mut xs_tmp, &mut deltas_tmp, y);
            for i in 0..n {
                points.push((xs_tmp[i], deltas_tmp[i]));
            }
        }
    }
}

impl PreparedColoredShape {
    /// Creates a [`Scanline`] from this shape.
    pub fn scanline(&self, y: f64) -> Scanline {
        let mut points = Vec::new();
        for component in &self.components {
            component.insert_scanline_points(y, &mut points);
        }
        Scanline::new(points)
    }
}
