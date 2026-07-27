use image::{GenericImageView, Pixel, Primitive};
use num_traits::{NumCast, ToPrimitive};

use crate::bezier::Point;

fn mix(a1: f64, a2: f64, t: f64) -> f64 {
    a1 * (1.0 - t) + a2 * t
}

pub fn sample_bilinear<P: Pixel>(input: &impl GenericImageView<Pixel = P>, p: Point) -> P
where
    P::Subpixel: Primitive,
{
    let x1 = (p.x - 0.5).floor();
    let y1 = (p.y - 0.5).floor();
    let x2 = x1 + 1.0;
    let y2 = y1 + 1.0;
    let wx = p.x - x1 - 0.5;
    let wy = p.y - y1 - 0.5;
    let x1 = (x1 as u32).min(input.width() - 1);
    let y1 = (y1 as u32).min(input.height() - 1);
    let x2 = (x2 as u32).min(input.width() - 1);
    let y2 = (y2 as u32).min(input.height() - 1);
    let ll = input.get_pixel(x1, y1);
    let ul = input.get_pixel(x2, y1);
    let lu = input.get_pixel(x1, y2);
    let uu = input.get_pixel(x2, y2);
    let mut out = ll;
    for (i, out) in out.channels_mut().iter_mut().enumerate() {
        let ll = ll.channels()[i].to_f64().unwrap();
        let ul = ul.channels()[i].to_f64().unwrap();
        let lu = lu.channels()[i].to_f64().unwrap();
        let uu = uu.channels()[i].to_f64().unwrap();
        let x = mix(mix(ll, ul, wx), mix(lu, uu, wx), wy);
        *out = <P::Subpixel as NumCast>::from(x).unwrap();
    }
    out
}
