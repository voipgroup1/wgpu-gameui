//! Rendering from distance fields.

use image::{GenericImage, GenericImageView, Luma, Pixel, Primitive, Rgb, Rgba};
use na::Scale2;

use crate::{
    bezier::{
        prepared::{PreparedColoredShape, PreparedComponent},
        scanline::FillRule,
        Point,
    },
    generate::{pixel_from_f64, pixel_value_to_signed_distance, render},
    interpolate::sample_bilinear,
};

pub(crate) fn median<P: Primitive>(sample: Rgb<P>) -> P {
    let [r, g, b] = sample.0;
    median3(r, g, b)
}

pub(crate) fn median3<P: Primitive>(r: P, g: P, b: P) -> P {
    fn mn<P: Primitive>(x: P, y: P) -> P {
        if x < y {
            x
        } else {
            y
        }
    }
    fn mx<P: Primitive>(x: P, y: P) -> P {
        if x > y {
            x
        } else {
            y
        }
    }
    mx(mn(r, g), mn(mx(r, g), b))
}

fn render_generic<In: Pixel, F: Fn(In) -> In::Subpixel>(
    input: &impl GenericImageView<Pixel = In>,
    output: &mut impl GenericImage<Pixel = Luma<u8>>,
    subpixel: F,
    px_range: f64,
) {
    let thresh = input.width() as f64 / output.width() as f64;
    render(
        &na::convert(Scale2::new(
            1.0 / output.width() as f64,
            1.0 / output.height() as f64,
        )),
        |point| {
            let sample = sample_bilinear(
                input,
                Point::new(
                    point.x * input.width() as f64,
                    point.y * input.height() as f64,
                ),
            );
            let sample = subpixel(sample);
            let distance = pixel_value_to_signed_distance(sample, px_range);
            let w = (distance / thresh + 0.5).clamp(0.0, 1.0);
            Luma([pixel_from_f64(w)])
        },
        output,
    )
}

/// Renders a raster image of a shape from its MSDF.
pub fn render_msdf(
    input: &impl GenericImageView<Pixel = Rgb<u8>>,
    output: &mut impl GenericImage<Pixel = Luma<u8>>,
    px_range: f64,
) {
    render_generic(input, output, median, px_range)
}

/// Renders a raster image of a shape from its SDF.
pub fn render_sdf(
    input: &impl GenericImageView<Pixel = Luma<u8>>,
    output: &mut impl GenericImage<Pixel = Luma<u8>>,
    px_range: f64,
) {
    render_generic(input, output, |sample| sample.0[0], px_range)
}

/// Corrects the sign of a signed distance field, so that points inside
/// the shape have positive values and points outside the shape have
/// negative values.
pub fn correct_sign_sdf<P: Primitive>(
    sdf: &mut impl GenericImage<Pixel = Luma<P>>,
    shape: &PreparedComponent,
    fill_rule: FillRule,
) {
    let half: P = pixel_from_f64(0.5);
    for y in 0..sdf.height() {
        let scanline = shape.scanline(y as f64 + 0.5);
        let mut cursor = scanline.cursor();
        for x in 0..sdf.width() {
            let fill = cursor.filled(x as f64 + 0.5, fill_rule);
            let mut sd = sdf.get_pixel(x, y);
            if (sd.0[0] > half) != fill {
                sd.invert();
                sdf.put_pixel(x, y, sd);
            }
        }
    }
}

/// Corrects the sign of a multi-channel signed distance field, so
/// that points inside the shape have positive values and points
/// outside the shape have negative values.
pub fn correct_sign_msdf<P: Primitive>(
    sdf: &mut impl GenericImage<Pixel = Rgb<P>>,
    shape: &PreparedColoredShape,
    fill_rule: FillRule,
) where
    Rgb<P>: Pixel,
{
    correct_sign_msdf_generic(sdf, shape, fill_rule);
}

/// Corrects the sign of a combined multi-channel and true signed
/// distance field, so that points inside the shape have positive
/// values and points outside the shape have negative values.
pub fn correct_sign_mtsdf<P: Primitive>(
    sdf: &mut impl GenericImage<Pixel = Rgba<P>>,
    shape: &PreparedColoredShape,
    fill_rule: FillRule,
) where
    Rgba<P>: Pixel,
{
    correct_sign_msdf_generic(sdf, shape, fill_rule);
}

fn correct_sign_msdf_generic<P: Primitive, Px: Pixel<Subpixel = P>>(
    sdf: &mut impl GenericImage<Pixel = Px>,
    shape: &PreparedColoredShape,
    fill_rule: FillRule,
) {
    let half: P = pixel_from_f64(0.5);
    let w = sdf.width();
    let h = sdf.height();
    if w * h == 0 {
        return;
    }
    let mut ambiguous = false;
    let mut match_map = vec![0i8; (w * h) as usize];
    let mut match_idx = 0;
    for y in 0..h {
        let scanline = shape.scanline(y as f64 + 0.5);
        let mut cursor = scanline.cursor();
        for x in 0..w {
            let fill = cursor.filled(x as f64 + 0.5, fill_rule);
            let mut msd = sdf.get_pixel(x, y);
            let sd = median(msd.to_rgb());
            if sd == half {
                ambiguous = true;
            } else if (sd > half) != fill {
                msd.invert();
                match_map[match_idx] = -1;
            } else {
                match_map[match_idx] = 1;
            }
            if msd.channels().len() >= 4 {
                let alpha = &mut msd.channels_mut()[3];
                if (*alpha > half) != fill {
                    *alpha = P::DEFAULT_MAX_VALUE - *alpha;
                }
            }
            sdf.put_pixel(x, y, msd);
            match_idx += 1;
        }
    }
    if ambiguous {
        match_idx = 0;
        for y in 0..h {
            for x in 0..w {
                if match_map[match_idx] == 0 {
                    let mut neighbor_match = 0;
                    if x > 0 {
                        neighbor_match += match_map[match_idx - 1];
                    }
                    if x < w - 1 {
                        neighbor_match += match_map[match_idx + 1];
                    }
                    if y > 0 {
                        neighbor_match += match_map[match_idx - w as usize];
                    }
                    if y < h - 1 {
                        neighbor_match += match_map[match_idx + w as usize];
                    }
                    if neighbor_match < 0 {
                        let mut msd = sdf.get_pixel(x, y);
                        msd.invert();
                        sdf.put_pixel(x, y, msd);
                    }
                }
                match_idx += 1;
            }
        }
    }
}
