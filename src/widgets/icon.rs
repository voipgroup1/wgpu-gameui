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
//! ```

use crate::layout::Rect;
use crate::render::PhosphorIcon;

use super::DrawList;

/// A vector icon ([`PhosphorIcon`]) drawn through the MSDF icon atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Icon {
    icon: PhosphorIcon,
    tint: [f32; 4],
}

impl Icon {
    /// An icon at its natural color (white fill, modulated by the draw list's
    /// current tint).
    pub fn new(icon: PhosphorIcon) -> Self {
        Self {
            icon,
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }

    /// Multiply the icon's fill by `tint`.
    pub fn tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    /// Draw the icon fit-centered into `rect`. No-op for a zero-area rect.
    pub fn draw(&self, rect: Rect, list: &mut DrawList) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        list.icon_msdf(rect, self.icon, self.tint);
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

    #[test]
    fn zero_rect_draws_nothing() {
        let mut list = DrawList::new();
        Icon::new(PhosphorIcon::X).draw(Rect::new(0.0, 0.0, 0.0, 16.0), &mut list);
        assert!(list.icons_msdf.is_empty());
    }
}
