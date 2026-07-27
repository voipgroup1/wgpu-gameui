//! Generating various types of distance fields.

use image::{GenericImage, Luma, Pixel, Primitive, Rgb, Rgba};
use na::Affine2;

use crate::bezier::{
    prepared::{PreparedColoredShape, PreparedComponent},
    Point,
};

pub(crate) fn pixel_from_f64<P: Primitive>(x: f64) -> P {
    let min = P::DEFAULT_MIN_VALUE.to_f64().unwrap();
    let max = P::DEFAULT_MAX_VALUE.to_f64().unwrap();
    P::from(x * (max - min) + min).unwrap()
}

pub(crate) fn signed_distance_to_pixel_value<P: Primitive>(sd: f64, range: f64) -> P {
    pixel_from_f64((sd / range + 0.5).clamp(0.0, 1.0))
}

pub(crate) fn pixel_value_to_signed_distance<P: Primitive>(pix: P, range: f64) -> f64 {
    let min = P::DEFAULT_MIN_VALUE.to_f64().unwrap();
    let max = P::DEFAULT_MAX_VALUE.to_f64().unwrap();
    let pix = pix.to_f64().unwrap();
    ((pix - min) / (max - min) - 0.5) * range
}

pub(crate) fn generate<Px: Pixel, I: GenericImage<Pixel = Px>, F: Fn(Point) -> Px>(
    sampler: F,
    dest: &mut I,
) {
    for y in 0..dest.height() {
        for x in 0..dest.width() {
            let p = Point::new((x as f64) + 0.5, (y as f64) + 0.5);
            dest.put_pixel(x, y, sampler(p))
        }
    }
}

pub(crate) fn render<Px: Pixel, I: GenericImage<Pixel = Px>, F: Fn(Point) -> Px>(
    transformation: &Affine2<f64>,
    sampler: F,
    dest: &mut I,
) {
    for y in 0..dest.height() {
        for x in 0..dest.width() {
            let p = Point::new((x as f64) + 0.5, (y as f64) + 0.5);
            let tp = transformation.transform_point(&p);
            dest.put_pixel(x, y, sampler(tp))
        }
    }
}

fn sampler_sdf<P: Primitive>(shape: &PreparedComponent, range: f64, point: Point) -> Luma<P> {
    let d_min = shape.distance(point);
    Luma([signed_distance_to_pixel_value(
        d_min.value.distance(),
        range,
    )])
}

/// Generates a single-channel signed distance field.
///
/// * `shape` is the shape for which the distance field should be generated.
/// * `transformation` is a transformation from the units used by `shape` to
///   pixel units.
/// * `range` is the range of the distances represented, so that the resulting
///   distance field represents values in the range `[-range / 2.0, range / 2.0]`.
/// * `dest` is an instance of [`GenericImage`] to which the resulting distance
///   field should be written.
pub fn generate_sdf<P: Primitive, I: GenericImage<Pixel = Luma<P>>>(
    shape: &PreparedComponent,
    range: f64,
    dest: &mut I,
) {
    generate(|point| sampler_sdf(shape, range, point), dest)
}

fn sampler_msdf<P: Primitive>(shape: &PreparedColoredShape, range: f64, point: Point) -> Rgb<P> {
    let [d_red, d_green, d_blue] = shape.distance3(point);
    // if point == Point::new(191.0, -40.0) {
    //     dbg!(shape.distance4t(point));
    //     // dbg!(d_red
    //     //     .segment
    //     //     .unwrap()
    //     //     .distance_to_pseudodistance_tr(&d_red.value, point));
    //     // dbg!(d_green
    //     //     .segment
    //     //     .unwrap()
    //     //     .distance_to_pseudodistance_tr(&d_green.value, point));
    //     // dbg!(d_blue
    //     //     .segment
    //     //     .unwrap()
    //     //     .distance_to_pseudodistance_tr(&d_blue.value, point));
    // }
    let d_red = d_red.signed_pseudo_distance(point);
    let d_green = d_green.signed_pseudo_distance(point);
    let d_blue = d_blue.signed_pseudo_distance(point);
    // if point == Point::new(977.0, 1320.0) {
    //     dbg!((d_red, d_green, d_blue));
    // }
    Rgb([
        signed_distance_to_pixel_value(d_red, range),
        signed_distance_to_pixel_value(d_green, range),
        signed_distance_to_pixel_value(d_blue, range),
    ])
}

fn sampler_mtsdf<P: Primitive>(shape: &PreparedColoredShape, range: f64, point: Point) -> Rgba<P> {
    let [d_red, d_green, d_blue, d_min] = shape.distance4(point);
    let d_red = d_red.signed_pseudo_distance(point);
    let d_green = d_green.signed_pseudo_distance(point);
    let d_blue = d_blue.signed_pseudo_distance(point);
    Rgba([
        signed_distance_to_pixel_value(d_red, range),
        signed_distance_to_pixel_value(d_green, range),
        signed_distance_to_pixel_value(d_blue, range),
        signed_distance_to_pixel_value(d_min.value.distance(), range),
    ])
}

/// Generates a multi-channel signed distance field.
///
/// * `shape` is the shape for which the distance field should be generated.
/// * `transformation` is a transformation from the units used by `shape` to
///   pixel units.
/// * `range` is the range of the distances represented, so that the resulting
///   distance field represents values in the range `[-range / 2.0, range / 2.0]`.
/// * `dest` is an instance of [`GenericImage`] to which the resulting distance
///   field should be written.
pub fn generate_msdf<P: Primitive, I: GenericImage<Pixel = Rgb<P>>>(
    shape: &PreparedColoredShape,
    range: f64,
    dest: &mut I,
) where
    Rgb<P>: Pixel,
{
    generate(|point| sampler_msdf(shape, range, point), dest)
}

/// Generates a multi-channel signed distance field along with a single-channel
/// (‘true’) SDF.
///
/// The red, green, and blue channels hold the MSDF, while the alpha channel
/// holds the single-channel SDF.
///
/// * `shape` is the shape for which the distance field should be generated.
/// * `transformation` is a transformation from the units used by `shape` to
///   pixel units.
/// * `range` is the range of the distances represented, so that the resulting
///   distance field represents values in the range `[-range / 2.0, range / 2.0]`.
/// * `dest` is an instance of [`GenericImage`] to which the resulting distance
///   field should be written.
pub fn generate_mtsdf<P: Primitive, I: GenericImage<Pixel = Rgba<P>>>(
    shape: &PreparedColoredShape,
    range: f64,
    dest: &mut I,
) where
    Rgba<P>: Pixel,
{
    generate(|point| sampler_mtsdf(shape, range, point), dest)
}
