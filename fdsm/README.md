# fdsm

A pure-Rust reimplementation of [multi-channel signed distance field generation](https://github.com/Chlumsky/msdfgen).

This implementation mostly follows [Victor Chlumský’s master thesis](https://github.com/Chlumsky/msdfgen/files/3050967/thesis.pdf).
Although it is not an exact translation of the C++ [`msdfgen`](https://github.com/Chlumsky/msdfgen) library, it does follow it for some parts of the code.

`fdsm` also uses code adapted from [`msdfgen-rs`](https://github.com/katyo/msdfgen-rs) for its tests.

## Crate features

* `visualize`: helpers for visualization

The `ttf-parser` feature has now been moved to a separate crate, [`fdsm-ttf-parser`](https://docs.rs/fdsm-ttf-parser).

## Usage

```rust,no_run
use fdsm::bezier::scanline::FillRule;
use fdsm::generate::generate_msdf;
use fdsm::render::{correct_sign_msdf, render_msdf};
use fdsm::shape::Shape;
# use fdsm::shape::Contour;
use fdsm::transform::Transform;
use image::{GrayImage, RgbImage};
use nalgebra::{Affine2, Similarity2, Vector2};

// First, acquire a [`Shape`]. This can be done by procedurally
// generating one or by loading one from a font:

# #[cfg(false)] {
use ttf_parser::Face;
let face = Face::parse(notosans::REGULAR_TTF, 0).unwrap();
let glyph_id = face.glyph_index('A').unwrap();
let mut shape = fdsm_ttf_parser::load_shape_from_face(&face, glyph_id);
let bbox = face.glyph_bounding_box(glyph_id).unwrap();
# manipulate_shape(shape, bbox);
# }

# fn manipulate_shape(mut shape: Shape<Contour>, bbox: ttf_parser::Rect) {
// Prepare your transformation matrix and calculate the dimensions of
// the resulting signed distance field. As an example, we set this up
// using ‘shrinkage’ (font units per texel) and ‘range’ (number of
// texels for the margin) values.
// Note that since font files interpret a positive y-offset as
// pointing up, the resulting distance field will be upside-down.
// This can be corrected either by flipping the resulting image
// vertically or by modifying the transformation matrix. We omit
// this fix for simplicity.

const RANGE: f64 = 4.0;
const SHRINKAGE: f64 = 16.0;
let transformation = nalgebra::convert::<_, Affine2<f64>>(Similarity2::new(
    Vector2::new(
        RANGE - bbox.x_min as f64 / SHRINKAGE,
        RANGE - bbox.y_min as f64 / SHRINKAGE,
    ),
    0.0,
    1.0 / SHRINKAGE,
));
let width =
    ((bbox.x_max as f64 - bbox.x_min as f64) / SHRINKAGE + 2.0 * RANGE).ceil() as u32;
let height =
    ((bbox.y_max as f64 - bbox.y_min as f64) / SHRINKAGE + 2.0 * RANGE).ceil() as u32;

// Unlike msdfgen, the transformation is not passed into the
// `generate_msdf` function – the coordinates of the control points
// must be expressed in terms of pixels on the distance field. To get
// the correct units, we pre-transform the shape:

shape.transform(&transformation);

// We now color the edges of the shape. We also have to prepare
// it for calculations:

let colored_shape = Shape::edge_coloring_simple(shape, 0.03, 69441337420);
let prepared_colored_shape = colored_shape.prepare();

// Set up the resulting image and generate the distance field:

let mut msdf = RgbImage::new(width, height);
generate_msdf(&prepared_colored_shape, RANGE, &mut msdf);
correct_sign_msdf(&mut msdf, &prepared_colored_shape, FillRule::Nonzero);

// As a test, try previewing the distance field:

let mut preview = GrayImage::new(msdf.width() * 10, msdf.height() * 10);
render_msdf(&msdf, &mut preview, RANGE);
# }
```

## Roadmap

Currently, `fdsm` has the basic functionality of generating MSDFs and generates correct distance fields for the glyphs `A` to `Z` in Noto Sans. However, it does not have all of the features present in `msdfgen`.

* [X] Error correction
* [ ] Error estimation
* [X] Sign correction
* [ ] Shape simplification (cf. Section 3.1 of (Chlumský, 2015))
* [ ] Alternative edge-coloring algorithms
* [X] Benchmarks against `msdfgen`
