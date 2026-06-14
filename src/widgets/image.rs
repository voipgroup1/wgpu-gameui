//! Image / sprite widget.
//!
//! Draws a sprite into a destination box with a choice of scaling [`ImageFit`],
//! alignment within leftover space, a color tint, and (for `Cover`) automatic
//! UV cropping. It is a thin, stateless layer over the `DrawList` image
//! primitives — the same path the wgpu-game HUD uses for object-browser
//! thumbnails — adding the aspect-ratio handling those raw calls lack.
//!
//! # Source size
//! Aspect-aware fits (`Contain`/`Cover`/`ScaleDown`/`None`) need the sprite's
//! natural pixel size, which the `DrawList` cannot know. Provide it via
//! [`Image::natural_size`] — callers typically read it from
//! [`crate::UiRenderer::image_size`]. Without it, those fits fall back to
//! `Stretch`.
//!
//! # Example
//! ```ignore
//! let (w, h) = renderer.image_size("portrait.png").unwrap_or((0, 0));
//! Image::key("portrait.png")
//!     .natural_size(w as f32, h as f32)
//!     .fit(ImageFit::Cover)
//!     .draw(dest_rect, &mut list);
//! ```

use crate::SpriteId;
use crate::layout::Rect;

use super::DrawList;

/// How an image is scaled to its destination box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// Fill the box exactly, ignoring aspect ratio (may distort).
    #[default]
    Stretch,
    /// Scale to fit *inside* the box preserving aspect (letterbox/pillarbox).
    Contain,
    /// Scale to *cover* the box preserving aspect, cropping the overflow via UV.
    /// Requires a [`SpriteId`] source for the crop; a string-key source falls
    /// back to `Stretch`.
    Cover,
    /// Like `Contain`, but never scale *up* beyond the natural size.
    ScaleDown,
    /// Draw at natural pixel size with no scaling, aligned within the box (may
    /// overflow — push a clip rect if you need it bounded).
    None,
}

/// Placement of the scaled image within leftover box space (used by
/// `Contain`/`ScaleDown`/`None`, and to bias the `Cover` crop). Each axis is
/// `0.0` = start (left/top), `0.5` = center, `1.0` = end (right/bottom).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageAlign {
    pub x: f32,
    pub y: f32,
}

impl Default for ImageAlign {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 }
    }
}

impl ImageAlign {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    pub const TOP_LEFT: Self = Self { x: 0.0, y: 0.0 };
}

#[derive(Clone)]
enum Source {
    /// Pre-resolved atlas handle. Supports per-draw tint and UV crop.
    Sprite(SpriteId),
    /// String key resolved by name at render time (like `DrawList::icon`).
    /// Tint comes from the draw-list's current tint, not the widget; UV crop is
    /// unavailable, so `Cover` degrades to `Stretch`.
    Key(String),
}

/// Image widget — draws a sprite into a box with a fit mode, alignment, and tint.
#[derive(Clone)]
pub struct Image {
    source: Source,
    natural: Option<(f32, f32)>,
    fit: ImageFit,
    align: ImageAlign,
    tint: [f32; 4],
}

impl Image {
    /// Draw a pre-resolved sprite handle (supports tint + UV crop).
    pub fn sprite(id: SpriteId) -> Self {
        Self::with_source(Source::Sprite(id))
    }

    /// Draw a string-keyed sprite, resolved by name at render time.
    pub fn key(key: impl Into<String>) -> Self {
        Self::with_source(Source::Key(key.into()))
    }

    fn with_source(source: Source) -> Self {
        Self {
            source,
            natural: None,
            fit: ImageFit::Stretch,
            align: ImageAlign::CENTER,
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }

    /// Natural (source) pixel size, required for aspect-aware fits.
    pub fn natural_size(mut self, w: f32, h: f32) -> Self {
        self.natural = Some((w, h));
        self
    }

    /// Set the scaling mode (default [`ImageFit::Stretch`]).
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.fit = fit;
        self
    }

    /// Set placement within leftover box space (default center).
    pub fn align(mut self, align: ImageAlign) -> Self {
        self.align = align;
        self
    }

    /// Multiply the sampled image color by `tint` (sprite-source only).
    pub fn tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    /// Compute the on-screen rect and optional UV sub-rect for `dest`.
    /// `can_crop` is true only when the source supports UV cropping.
    fn resolve(&self, dest: Rect, can_crop: bool) -> (Rect, Option<[f32; 4]>) {
        let (nw, nh) = match self.natural {
            Some((w, h)) if w > 0.0 && h > 0.0 => (w, h),
            // Unknown source size: nothing to base aspect on -> fill the box.
            _ => return (dest, None),
        };

        match self.fit {
            ImageFit::Stretch => (dest, None),
            ImageFit::Contain | ImageFit::ScaleDown => {
                let mut scale = (dest.width / nw).min(dest.height / nh);
                if self.fit == ImageFit::ScaleDown {
                    scale = scale.min(1.0);
                }
                (self.aligned(dest, nw * scale, nh * scale), None)
            }
            ImageFit::None => (self.aligned(dest, nw, nh), None),
            ImageFit::Cover => {
                if !can_crop {
                    // No UV crop available (string-key source): fill the box.
                    return (dest, None);
                }
                let scale = (dest.width / nw).max(dest.height / nh);
                let (sw, sh) = (nw * scale, nh * scale);
                // Fraction of the source visible along each axis (<= 1).
                let u = (dest.width / sw).min(1.0);
                let v = (dest.height / sh).min(1.0);
                // Bias the visible window by alignment.
                let u0 = (1.0 - u) * self.align.x;
                let v0 = (1.0 - v) * self.align.y;
                (dest, Some([u0, v0, u0 + u, v0 + v]))
            }
        }
    }

    /// Place a `w`x`h` image within `dest` according to `align`.
    fn aligned(&self, dest: Rect, w: f32, h: f32) -> Rect {
        let x = dest.x + (dest.width - w) * self.align.x;
        let y = dest.y + (dest.height - h) * self.align.y;
        Rect::new(x, y, w, h)
    }

    /// Draw the image into `dest`.
    pub fn draw(&self, dest: Rect, list: &mut DrawList) {
        if dest.width <= 0.0 || dest.height <= 0.0 {
            return;
        }
        match &self.source {
            Source::Sprite(id) => {
                let (r, uv) = self.resolve(dest, true);
                match uv {
                    Some(uv) => list.image_cropped(*id, r, uv, self.tint),
                    None => list.image(*id, r, self.tint),
                }
            }
            Source::Key(key) => {
                let (r, _) = self.resolve(dest, false);
                // Key path: tint/crop unsupported; current draw-list tint applies.
                list.icon(key, r.x, r.y, r.width, r.height);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // With the identity transform, IconDraw corners are TL, TR, BR, BL — so the
    // drawn rect is recoverable from corner 0 (TL) and corner 2 (BR).
    fn drawn_rect(list: &DrawList) -> Rect {
        let c = list.icons.last().expect("an icon was queued").corners;
        Rect::new(c[0][0], c[0][1], c[2][0] - c[0][0], c[2][1] - c[0][1])
    }

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "expected {b}, got {a}");
    }

    const ID: SpriteId = 0;

    #[test]
    fn stretch_fills_dest_exactly() {
        let mut list = DrawList::new();
        let dest = Rect::new(10.0, 20.0, 200.0, 100.0);
        Image::sprite(ID)
            .fit(ImageFit::Stretch)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.x, 10.0);
        approx(r.y, 20.0);
        approx(r.width, 200.0);
        approx(r.height, 100.0);
        assert!(
            list.icons[0].src.is_none(),
            "stretch draws the whole sprite"
        );
    }

    #[test]
    fn contain_letterboxes_square_into_wide_box_centered() {
        let mut list = DrawList::new();
        // 100x100 source into a 200x100 box -> scaled to 100x100, centered.
        let dest = Rect::new(0.0, 0.0, 200.0, 100.0);
        Image::sprite(ID)
            .natural_size(100.0, 100.0)
            .fit(ImageFit::Contain)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.width, 100.0);
        approx(r.height, 100.0);
        approx(r.x, 50.0); // (200-100)/2 centered
        approx(r.y, 0.0);
        assert!(list.icons[0].src.is_none());
    }

    #[test]
    fn contain_align_left_pins_to_start() {
        let mut list = DrawList::new();
        let dest = Rect::new(0.0, 0.0, 200.0, 100.0);
        Image::sprite(ID)
            .natural_size(100.0, 100.0)
            .fit(ImageFit::Contain)
            .align(ImageAlign::TOP_LEFT)
            .draw(dest, &mut list);
        approx(drawn_rect(&list).x, 0.0);
    }

    #[test]
    fn scale_down_never_upscales() {
        let mut list = DrawList::new();
        // 50x50 source into a big box: ScaleDown keeps it at 50x50, centered.
        let dest = Rect::new(0.0, 0.0, 400.0, 400.0);
        Image::sprite(ID)
            .natural_size(50.0, 50.0)
            .fit(ImageFit::ScaleDown)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.width, 50.0);
        approx(r.height, 50.0);
        approx(r.x, 175.0); // (400-50)/2
    }

    #[test]
    fn contain_upscales_but_scale_down_does_not() {
        // Same small source, Contain *does* grow to fill one axis.
        let mut list = DrawList::new();
        let dest = Rect::new(0.0, 0.0, 400.0, 400.0);
        Image::sprite(ID)
            .natural_size(50.0, 50.0)
            .fit(ImageFit::Contain)
            .draw(dest, &mut list);
        approx(drawn_rect(&list).width, 400.0);
    }

    #[test]
    fn none_draws_natural_size() {
        let mut list = DrawList::new();
        let dest = Rect::new(0.0, 0.0, 400.0, 400.0);
        Image::sprite(ID)
            .natural_size(120.0, 80.0)
            .fit(ImageFit::None)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.width, 120.0);
        approx(r.height, 80.0);
        approx(r.x, 140.0); // (400-120)/2
        approx(r.y, 160.0); // (400-80)/2
    }

    #[test]
    fn cover_fills_box_and_crops_via_uv() {
        let mut list = DrawList::new();
        // 100x100 source into 200x100 box: scale=2 -> 200x200, crop vertically
        // to the centered middle half.
        let dest = Rect::new(0.0, 0.0, 200.0, 100.0);
        Image::sprite(ID)
            .natural_size(100.0, 100.0)
            .fit(ImageFit::Cover)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.width, 200.0);
        approx(r.height, 100.0);
        let uv = list.icons[0].src.expect("cover crops via UV");
        approx(uv[0], 0.0); // u spans full width
        approx(uv[2], 1.0);
        approx(uv[1], 0.25); // v centered: (1 - 0.5)/2
        approx(uv[3], 0.75);
    }

    #[test]
    fn cover_without_sprite_falls_back_to_stretch() {
        let mut list = DrawList::new();
        let dest = Rect::new(0.0, 0.0, 200.0, 100.0);
        Image::key("portrait.png")
            .natural_size(100.0, 100.0)
            .fit(ImageFit::Cover)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.width, 200.0);
        approx(r.height, 100.0);
        // Key path queues a name-keyed icon, no UV crop.
        assert_eq!(list.icons[0].icon_key, "portrait.png");
        assert!(list.icons[0].sprite.is_none());
        assert!(list.icons[0].src.is_none());
    }

    #[test]
    fn unknown_natural_size_falls_back_to_fill() {
        let mut list = DrawList::new();
        let dest = Rect::new(5.0, 5.0, 60.0, 40.0);
        // Contain without a natural size can't preserve aspect -> fills the box.
        Image::sprite(ID)
            .fit(ImageFit::Contain)
            .draw(dest, &mut list);
        let r = drawn_rect(&list);
        approx(r.width, 60.0);
        approx(r.height, 40.0);
    }

    #[test]
    fn tint_passes_through_on_sprite_source() {
        let mut list = DrawList::new();
        let dest = Rect::new(0.0, 0.0, 10.0, 10.0);
        Image::sprite(ID)
            .tint([1.0, 0.0, 0.0, 0.5])
            .draw(dest, &mut list);
        assert_eq!(list.icons[0].tint, [1.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn zero_dest_draws_nothing() {
        let mut list = DrawList::new();
        Image::sprite(ID).draw(Rect::new(0.0, 0.0, 0.0, 50.0), &mut list);
        assert!(list.icons.is_empty());
    }
}
