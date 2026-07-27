//! Handling shapes.
//!
//! A [*shape*][`Shape`] is made up of one or more
//! [*contours*][`TContour`], each of which has one or more
//! [*segments*][`Segment`].
//!
//! See Subsection 4.1.2 of (Chlumský, 2015) for more information.
//!
//! Once a shape is loaded or created, it can be
//! [*colored*][crate::color]. Conceptually, each edge has a color,
//! but the representation of colored contours assigns a color to
//! each *edge segment*. Segments that belong to the same edge are
//! conventionally assigned the same color.

use std::fmt::{self, Display, Write};

use na::Affine2;

use crate::{bezier::Segment, color::Color, transform::Transform};

/// A contour segment with an assigned color.
#[derive(Copy, Clone, Debug)]
pub struct ColoredSegment {
    pub segment: Segment,
    pub color: Color,
}

impl Transform for ColoredSegment {
    fn transform(&mut self, transformation: &Affine2<f64>) {
        self.segment.transform(transformation);
    }
}

/// A non-intersecting outline made of one or more Bézier curves.
///
/// This is a generic type with two type aliases, [`Contour`] and [`ColoredContour`]. `T` denotes the type of the segment.
#[derive(Clone, Debug)]
pub struct TContour<T> {
    pub segments: Vec<T>,
}

// This trait is manually implemented because we don’t want the `C: Default` bound.
impl<T> Default for TContour<T> {
    fn default() -> Self {
        Self {
            segments: Default::default(),
        }
    }
}

impl<T: Transform> Transform for TContour<T> {
    fn transform(&mut self, transformation: &Affine2<f64>) {
        for segment in &mut self.segments {
            segment.transform(transformation);
        }
    }
}

/// A non-intersecting outline made of one or more Bézier curves.
pub type Contour = TContour<Segment>;
/// A contour with color information.
pub type ColoredContour = TContour<ColoredSegment>;

impl ColoredContour {
    /// Creates a new `ColoredContour` from a [`Contour`], using the simple
    /// edge-coloring algorithm in Subsection 4.4.1 of (Chlumský, 2015).
    // This follows the code for the edgeColoringSimple algorithm (edge-coloring.cpp:45):
    // https://github.com/Chlumsky/msdfgen/blob/master/core/edge-coloring.cpp#L45
    pub fn edge_coloring_simple(contour: Contour, sin_alpha: f64, seed: u64) -> Self {
        let mut segments: Vec<_> = contour
            .segments
            .into_iter()
            .map(|segment| ColoredSegment {
                segment,
                color: Color::WHITE,
            })
            .collect();

        let mut corners = Vec::new();
        if let Some(last_segment) = segments.last() {
            if last_segment
                .segment
                .corners_into(&segments[0].segment, sin_alpha)
                .is_some()
            {
                corners.push(0);
            }
            for i in 0..(segments.len() - 1) {
                if segments[i]
                    .segment
                    .corners_into(&segments[i + 1].segment, sin_alpha)
                    .is_some()
                {
                    corners.push(i + 1);
                }
            }
        }

        // Unlike the original C++ implementation, we ensure that one of the
        // corners occurs on the index 0.
        // This simplifies some downstream code.
        {
            let s = corners.first().copied().unwrap_or(0);
            if s != 0 {
                segments.rotate_left(s);
                for c in &mut corners {
                    *c -= s;
                }
            }
        }

        if corners.is_empty() {
            // Leave all segments colored white
        } else if corners.len() == 1 {
            // “Teardrop” shape – split the edge into three parts.
            let (color0, seed) = Color::WHITE.switch(seed, Color::BLACK);
            let (color2, _seed) = color0.switch(seed, Color::BLACK);
            let colors = [color0, Color::WHITE, color2];
            match segments.len() {
                0 => (),
                // 1 or 2 segments: some edges need to be split
                1 => {
                    let split_segment = segments[0].segment.split_in_thirds();
                    segments = split_segment
                        .into_iter()
                        .zip(colors)
                        .map(|(segment, color)| ColoredSegment { segment, color })
                        .collect();
                }
                2 => {
                    let split_segment_0 = segments[0].segment.split_in_thirds();
                    let split_segment_1 = segments[1].segment.split_in_thirds();
                    segments = split_segment_0
                        .into_iter()
                        .chain(split_segment_1)
                        .enumerate()
                        .map(|(i, segment)| ColoredSegment {
                            segment,
                            color: colors[i / 2],
                        })
                        .collect();
                }
                // 3+ segments: no edges need to be split
                _ => {
                    let num_segments = segments.len();
                    for (i, segment) in segments.iter_mut().enumerate() {
                        let index = (num_segments - 1 + 46 * i) / (16 * (num_segments - 1));
                        segment.color = colors[index];
                    }
                }
            }
        } else {
            // Multiple corners
            let mut spline = 0;
            let (mut color, mut seed) = Color::WHITE.switch(seed, Color::BLACK);
            let initial_color = color;

            for (i, segment) in segments.iter_mut().enumerate() {
                if corners.get(spline + 1) == Some(&i) {
                    spline += 1;
                    (color, seed) = color.switch(
                        seed,
                        if spline == corners.len() - 1 {
                            initial_color
                        } else {
                            Color::BLACK
                        },
                    )
                }
                segment.color = color;
            }
        }

        ColoredContour { segments }
    }
}

/// A shape consisting of a number of contours.
#[derive(Clone, Debug)]
pub struct Shape<C> {
    pub contours: Vec<C>,
}

// This trait is manually implemented because we don’t want the `C: Default` bound.
impl<C> Default for Shape<C> {
    fn default() -> Self {
        Self {
            contours: Default::default(),
        }
    }
}

impl Shape<ColoredContour> {
    /// Colors a shape, using the simple edge-coloring algorithm in Subsection
    /// 4.4.1 of (Chlumský, 2015).
    pub fn edge_coloring_simple(shape: Shape<Contour>, sin_alpha: f64, seed: u64) -> Self {
        Shape {
            contours: shape
                .contours
                .into_iter()
                .map(|c| ColoredContour::edge_coloring_simple(c, sin_alpha, seed))
                .collect(),
        }
    }
}

impl Display for Shape<ColoredContour> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for contour in &self.contours {
            f.write_str("{\n")?;
            for edge in &contour.segments {
                let start = edge.segment.start();
                write!(f, "  {}, {};\n    ", start.x, start.y)?;
                let c = b"brgybmcy"[edge.color.value() as usize] as char;
                f.write_char(c)?;
                let n = edge.segment.order_int();
                if n > 1 {
                    f.write_char('(')?;
                    for i in 1..n {
                        if i > 1 {
                            f.write_str("; ")?;
                        }
                        let cp = edge.segment.control_point(i);
                        write!(f, "{}, {}", cp.x, cp.y)?;
                    }
                    f.write_char(')')?;
                }
                f.write_str(";\n")?;
            }
            f.write_str("  #\n}\n")?;
        }
        Ok(())
    }
}

impl<C: Transform> Transform for Shape<C> {
    fn transform(&mut self, transformation: &Affine2<f64>) {
        for contour in &mut self.contours {
            contour.transform(transformation);
        }
    }
}
