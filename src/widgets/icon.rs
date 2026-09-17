//! Vector icon widget.
//!
//! A thin, stateless layer over [`DrawList::icon_msdf`] that draws a
//! [`PhosphorIcon`] fit-and-centered into a destination rect, rendered crisp at
//! any size through the MSDF icon atlas. Mirrors [`Image`](super::Image): no
//! caller state, builder-style configuration, `draw(rect, list)`.
//!
//! Unlike text glyphs, an icon's placement is driven by the glyph tile's own
//! extent (centered, contain-fit), so it sits in the visual center of the rect
//! regardless of the font's side bearings — exactly what icon affordances want.
//!
//! Gated behind the `phosphor-icons` feature.
//!
//! # Example
//! ```ignore
//! Icon::new(PhosphorIcon::Gear).draw(rect, &mut list);
//! Icon::new(PhosphorIcon::Trash).tint([0.9, 0.2, 0.2, 1.0]).draw(rect, &mut list);
//!
//! // A glyph from an application-registered icon font.
//! Icon::from_glyph(wedge).draw(rect, &mut list);
//! ```

use crate::layout::Rect;
use crate::render::{IconGlyph, PhosphorIcon};

use super::DrawList;

/// A vector icon — from the built-in [`PhosphorIcon`] set or any registered icon
/// font — drawn through the MSDF icon atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Icon {
    /// Resolved at construction. `None` when the source enum had no glyph in the
    /// font, in which case the widget simply draws nothing.
    glyph: Option<IconGlyph>,
    tint: [f32; 4],
}

impl Icon {
    /// A built-in Phosphor icon at its natural color (white fill, modulated by
    /// the draw list's current tint).
    pub fn new(icon: PhosphorIcon) -> Self {
        Self {
            glyph: icon.glyph(),
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }

    /// An icon from a pre-resolved glyph — the route for a font registered with
    /// [`register_icon_font`](crate::render::register_icon_font).
    pub fn from_glyph(glyph: IconGlyph) -> Self {
        Self {
            glyph: Some(glyph),
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }

    /// Multiply the icon's fill by `tint`.
    pub fn tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    /// Draw the icon fit-centered into `rect`. No-op for a zero-area rect or an
    /// unresolvable glyph.
    pub fn draw(&self, rect: Rect, list: &mut DrawList) {
        let Some(glyph) = self.glyph else {
            return;
        };
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        list.push_debug_scope_rect("Icon", rect);
        list.icon_msdf(rect, glyph, self.tint);
        list.pop_debug_scope();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_pushes_one_icon_record() {
        let mut list = DrawList::new();
        Icon::new(PhosphorIcon::Plus).draw(Rect::new(10.0, 20.0, 24.0, 24.0), &mut list);
        assert_eq!(list.icons_msdf.len(), 1);
        let rec = list.icons_msdf[0];
        assert_eq!(rec.local, Rect::new(10.0, 20.0, 24.0, 24.0));
        assert_eq!(rec.tint, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn tint_is_forwarded() {
        let mut list = DrawList::new();
        Icon::new(PhosphorIcon::Check)
            .tint([0.0, 1.0, 0.0, 1.0])
            .draw(Rect::new(0.0, 0.0, 16.0, 16.0), &mut list);
        assert_eq!(list.icons_msdf[0].tint, [0.0, 1.0, 0.0, 1.0]);
    }

    /// The two constructors must land on the same glyph for the same icon —
    /// `from_glyph` is the escape hatch, not a different rendering path.
    #[test]
    fn from_glyph_matches_the_phosphor_constructor() {
        let g = PhosphorIcon::Gear.glyph().expect("gear resolves");
        let mut a = DrawList::new();
        let mut b = DrawList::new();
        let rect = Rect::new(4.0, 5.0, 20.0, 20.0);
        Icon::new(PhosphorIcon::Gear).draw(rect, &mut a);
        Icon::from_glyph(g).draw(rect, &mut b);
        assert_eq!(a.icons_msdf, b.icons_msdf);
        assert_eq!(a.icons_msdf[0].glyph, g);
    }

    #[test]
    fn zero_rect_draws_nothing() {
        let mut list = DrawList::new();
        Icon::new(PhosphorIcon::X).draw(Rect::new(0.0, 0.0, 0.0, 16.0), &mut list);
        assert!(list.icons_msdf.is_empty());
    }
}
