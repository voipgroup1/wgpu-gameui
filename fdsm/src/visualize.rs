//! Helpers for visualization.
//!
//! This module is available only when the `visualize` feature is
//! enabled. Unless you’re working on `fdsm` itself, you shouldn’t need
//! this.

use std::mem;

use image::{GenericImage, Pixel, Rgb, Rgba};
use na::Affine2;
use oklab::Oklab;

use crate::{
    bezier::{Order, Point, Segment},
    color::Color,
    distance::{norm2, DistanceAndOrthogonality, DistanceField},
    generate::render,
    shape::{ColoredContour, Shape},
    transform::Transform,
};

fn fpart(x: f64) -> f64 {
    x - x.floor()
}

fn rfpart(x: f64) -> f64 {
    1.0 - fpart(x)
}

/// Draws a line using [Xiaolin Wu’s line algorithm](https://en.wikipedia.org/wiki/Xiaolin_Wu%27s_line_algorithm).
fn draw_line(dest: &mut impl GenericImage<Pixel = Rgb<u8>>, color: Rgb<u8>, p0: Point, p1: Point) {
    let mut plot = |x: i32, y: i32, c: f64| {
        let Ok(x) = u32::try_from(x) else { return };
        let Ok(y) = u32::try_from(y) else { return };
        if !dest.in_bounds(x, y) {
            return;
        }
        let p = dest.get_pixel(x, y);
        let mut p = p.to_rgba();
        p.blend(&Rgba([
            color.0[0],
            color.0[1],
            color.0[2],
            (c * 255.0).round() as u8,
        ]));
        dest.put_pixel(x, y, p.to_rgb())
    };

    let mut x0 = p0.x;
    let mut y0 = p0.y;
    let mut x1 = p1.x;
    let mut y1 = p1.y;
    let steep = (y1 - y0).abs() > (x1 - x0).abs();
    if steep {
        mem::swap(&mut x0, &mut y0);
        mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        mem::swap(&mut x0, &mut x1);
        mem::swap(&mut y0, &mut y1);
    }

    let dx = x1 - x0;
    let dy = y1 - y0;
    let gradient = if dx == 0.0 { 1.0 } else { dy / dx };

    let x_end = x0.round();
    let y_end = y0 + gradient * (x_end - x0);
    let x_gap = rfpart(x0 + 0.5);
    let x_pxl1 = x_end as i32;
    let y_pxl1 = y_end.floor() as i32;
    if steep {
        plot(y_pxl1, x_pxl1, rfpart(y_end) * x_gap);
        plot(y_pxl1 + 1, x_pxl1, fpart(y_end) * x_gap);
    } else {
        plot(x_pxl1, y_pxl1, rfpart(y_end) * x_gap);
        plot(x_pxl1, y_pxl1 + 1, fpart(y_end) * x_gap);
    }
    let mut inter_y = y_end + gradient;

    let x_end = x1.round();
    let y_end = y1 + gradient * (x_end - x1);
    let x_gap = fpart(x1 + 0.5);
    let x_pxl2 = x_end as i32;
    let y_pxl2 = y_end.floor() as i32;
    if steep {
        plot(y_pxl2, x_pxl2, rfpart(y_end) * x_gap);
        plot(y_pxl2 + 1, x_pxl2, fpart(y_end) * x_gap);
    } else {
        plot(x_pxl2, y_pxl2, rfpart(y_end) * x_gap);
        plot(x_pxl2, y_pxl2 + 1, fpart(y_end) * x_gap);
    }

    if steep {
        for x in (x_pxl1 + 1)..x_pxl2 {
            plot(inter_y.floor() as i32, x, rfpart(inter_y));
            plot(inter_y.floor() as i32 + 1, x, fpart(inter_y));
            inter_y += gradient;
        }
    } else {
        for x in (x_pxl1 + 1)..x_pxl2 {
            plot(x, inter_y.floor() as i32, rfpart(inter_y));
            plot(x, inter_y.floor() as i32 + 1, fpart(inter_y));
            inter_y += gradient;
        }
    }
}

impl Segment {
    fn draw(&self, dest: &mut impl GenericImage<Pixel = Rgb<u8>>, color: Rgb<u8>) {
        if self.order() == Order::Linear {
            draw_line(dest, color, self.start(), self.end());
        } else {
            self.draw_aux(dest, color, 0.0, 1.0);
        }
    }

    // FIXME: not correct when curve is closed
    fn draw_aux(
        &self,
        dest: &mut impl GenericImage<Pixel = Rgb<u8>>,
        color: Rgb<u8>,
        start: f64,
        end: f64,
    ) {
        let sp = self.get(start);
        let ep = self.get(end);
        if norm2(ep - sp) < 4.0 {
            draw_line(dest, color, sp, ep);
        } else {
            let mid = (start + end) / 2.0;
            self.draw_aux(dest, color, start, mid);
            self.draw_aux(dest, color, mid, end);
        }
    }
}

fn sampler_vis(shape: &Shape<ColoredContour>, point: Point) -> Rgb<u8> {
    let mut d_min = DistanceAndOrthogonality::default();
    let mut color = Color::BLACK;
    let mut current_ordinal = 0;
    let mut best_ordinal = 0;
    for contour in &shape.contours {
        for segment in &contour.segments {
            let d = segment
                .segment
                .signed_distance_and_orthogonality::<DistanceField>(point);
            if d < d_min {
                d_min = d;
                color = segment.color;
                best_ordinal = current_ordinal;
            }
            current_ordinal += 1;
        }
    }
    let t = best_ordinal as f32 * (1.0 + 5.0_f32.sqrt()) / (std::f32::consts::TAU);
    let c = Oklab {
        l: 1.0,
        a: t.cos(),
        b: t.sin(),
    };
    let c = oklab::oklab_to_srgb(c);
    let _c1 = Rgb([c.r, c.g, c.b]);
    let _c2 = Rgb([
        255 * (color.has_red() as u8),
        255 * (color.has_green() as u8),
        255 * (color.has_blue() as u8),
    ]);
    _c2
}

pub fn generate_vis<I: GenericImage<Pixel = Rgb<u8>>>(
    shape: &Shape<ColoredContour>,
    transformation: &Affine2<f64>,
    dest: &mut I,
) {
    render(
        &transformation.inverse(),
        |point| sampler_vis(shape, point),
        dest,
    );
    for contour in &shape.contours {
        for segment in &contour.segments {
            let mut segment2 = segment.segment;
            segment2.transform(transformation);
            segment2.draw(dest, Rgb([0, 0, 0]));
        }
    }
}
