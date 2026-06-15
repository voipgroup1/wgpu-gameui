//! Separator / divider widget — a thin rule between content.
//!
//! Non-interactive and stateless: it just paints a 1-ish-px line, centered
//! within the [`Rect`] it's handed, optionally inset from the ends. Like
//! [`Panel`](super::Panel) it draws through a [`StyleResolver`] rather than a
//! `DrawContext` (no focus/input to track), so it composes inside any layout
//! cell or chrome.
//!
//! Defaults are theme-relative: thickness comes from
//! [`StyleKey::BorderWidth`](crate::StyleKey::BorderWidth) and color from
//! [`StyleKey::PanelBorder`](crate::StyleKey::PanelBorder), so a separator
//! matches the surrounding panel borders and scales with DPI. Override either
//! with [`with_thickness`](Self::with_thickness) /
//! [`with_color`](Self::with_color), and trim the ends with
//! [`with_inset`](Self::with_inset).
//!
//! ```no_run
//! # use wgpu_gameui::{Separator, layout::Rect};
//! # fn demo(list: &mut wgpu_gameui::DrawList, style: &wgpu_gameui::StyleResolver) {
//! // A full-width horizontal rule occupying a 1px-tall (or taller) cell.
//! Separator::horizontal().draw(Rect::new(0.0, 100.0, 240.0, 1.0), list, style);
//! // A vertical divider between two columns, inset 6px top and bottom.
//! Separator::vertical()
//!     .with_inset(6.0)
//!     .draw(Rect::new(120.0, 0.0, 1.0, 48.0), list, style);
//! # }
//! ```

use crate::layout::Rect;
use crate::{StyleKey, StyleResolver};

use super::DrawList;

/// Which way a [`Separator`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// A horizontal rule (runs left↔right; thickness is its height).
    Horizontal,
    /// A vertical rule (runs top↕bottom; thickness is its width).
    Vertical,
}

/// A thin dividing line drawn centered within its [`Rect`].
///
/// The rule is always centered on the cross axis of the rect it's given, so a
/// caller can hand it a taller/wider rect (e.g. a layout row carrying its own
/// spacing) and the line sits in the middle rather than hugging an edge.
pub struct Separator {
    /// Direction the rule runs.
    pub orientation: Orientation,
    /// Line thickness in px. `None` → [`StyleKey::BorderWidth`] (min 1px).
    pub thickness: Option<f32>,
    /// Line color. `None` → [`StyleKey::PanelBorder`].
    pub color: Option<[f32; 4]>,
    /// Symmetric trim along the line's length (px removed from each end).
    pub inset: f32,
}

impl Separator {
    /// A horizontal rule (left↔right).
    pub fn horizontal() -> Self {
        Self {
            orientation: Orientation::Horizontal,
            thickness: None,
            color: None,
            inset: 0.0,
        }
    }

    /// A vertical rule (top↕bottom).
    pub fn vertical() -> Self {
        Self {
            orientation: Orientation::Vertical,
            thickness: None,
            color: None,
            inset: 0.0,
        }
    }

    /// Override the line thickness (px). Defaults to the theme border width.
    pub fn with_thickness(mut self, thickness: f32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    /// Override the line color. Defaults to the theme panel-border color.
    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = Some(color);
        self
    }

    /// Trim `inset` px from each end of the line (so it doesn't touch the
    /// neighbouring content/borders). Clamped so the line never goes negative.
    pub fn with_inset(mut self, inset: f32) -> Self {
        self.inset = inset.max(0.0);
        self
    }

    /// Draw the rule centered within `rect`.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, style: &StyleResolver) {
        let thickness = self
            .thickness
            .unwrap_or_else(|| style.scalar(StyleKey::BorderWidth).max(1.0));
        let color = self
            .color
            .unwrap_or_else(|| style.color(StyleKey::PanelBorder));

        match self.orientation {
            Orientation::Horizontal => {
                // Centered vertically; spans the width minus the end insets.
                let y = rect.y + (rect.height - thickness) * 0.5;
                let x = rect.x + self.inset;
                let w = (rect.width - 2.0 * self.inset).max(0.0);
                if w > 0.0 {
                    list.quad(x, y, w, thickness, color);
                }
            }
            Orientation::Vertical => {
                // Centered horizontally; spans the height minus the end insets.
                let x = rect.x + (rect.width - thickness) * 0.5;
                let y = rect.y + self.inset;
                let h = (rect.height - 2.0 * self.inset).max(0.0);
                if h > 0.0 {
                    list.quad(x, y, thickness, h, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn style() -> StyleResolver<'static> {
        // Leak a theme so the resolver can borrow it for the test's lifetime.
        let theme: &'static Theme = Box::leak(Box::new(Theme::default()));
        StyleResolver::new(theme)
    }

    /// The single emitted quad as (x, y, w, h). `quad` takes the translate-only
    /// fast path under the identity transform, recording one fill-only
    /// `ChromeInstance` (radius 0) rather than soup vertices.
    fn only_quad(list: &DrawList) -> (f32, f32, f32, f32) {
        assert_eq!(
            list.chrome_instances.len(),
            1,
            "expected exactly one fill instance"
        );
        let r = list.chrome_instances[0].rect;
        (r[0], r[1], r[2], r[3])
    }

    #[test]
    fn horizontal_is_centered_and_full_width() {
        let s = style();
        let mut list = DrawList::new();
        Separator::horizontal()
            .with_thickness(2.0)
            .draw(Rect::new(10.0, 20.0, 100.0, 10.0), &mut list, &s);
        let (x, y, w, h) = only_quad(&list);
        assert_eq!((x, w), (10.0, 100.0), "spans the full width");
        assert_eq!(h, 2.0, "thickness honored");
        // Centered in a 10px-tall cell: (10 - 2) / 2 = 4 → y = 20 + 4 = 24.
        assert_eq!(y, 24.0, "centered vertically in the cell");
    }

    #[test]
    fn vertical_is_centered_and_full_height() {
        let s = style();
        let mut list = DrawList::new();
        Separator::vertical()
            .with_thickness(4.0)
            .draw(Rect::new(10.0, 20.0, 12.0, 80.0), &mut list, &s);
        let (x, y, w, h) = only_quad(&list);
        assert_eq!((y, h), (20.0, 80.0), "spans the full height");
        assert_eq!(w, 4.0, "thickness honored");
        // Centered in a 12px-wide cell: (12 - 4) / 2 = 4 → x = 10 + 4 = 14.
        assert_eq!(x, 14.0, "centered horizontally in the cell");
    }

    #[test]
    fn inset_trims_both_ends() {
        let s = style();
        let mut list = DrawList::new();
        Separator::horizontal()
            .with_thickness(1.0)
            .with_inset(6.0)
            .draw(Rect::new(0.0, 0.0, 100.0, 1.0), &mut list, &s);
        let (x, _, w, _) = only_quad(&list);
        assert_eq!(x, 6.0, "left end inset");
        assert_eq!(w, 88.0, "100 - 2*6 = 88");
    }

    #[test]
    fn default_thickness_follows_theme_border_width() {
        let s = style();
        let bw = s.scalar(StyleKey::BorderWidth).max(1.0);
        let mut list = DrawList::new();
        Separator::horizontal().draw(Rect::new(0.0, 0.0, 50.0, 8.0), &mut list, &s);
        let (_, _, _, h) = only_quad(&list);
        assert_eq!(h, bw, "thickness defaults to theme border width");
    }

    #[test]
    fn default_color_follows_theme_panel_border() {
        let s = style();
        let expected = s.color(StyleKey::PanelBorder);
        let mut list = DrawList::new();
        Separator::horizontal().draw(Rect::new(0.0, 0.0, 50.0, 4.0), &mut list, &s);
        assert_eq!(list.chrome_instances.len(), 1);
        assert_eq!(
            list.chrome_instances[0].bg, expected,
            "fill uses the theme panel-border color"
        );
    }

    #[test]
    fn zero_length_after_inset_draws_nothing() {
        let s = style();
        let mut list = DrawList::new();
        // Inset larger than half the width collapses the line entirely.
        Separator::horizontal()
            .with_inset(60.0)
            .draw(Rect::new(0.0, 0.0, 100.0, 1.0), &mut list, &s);
        assert!(
            list.chrome_instances.is_empty(),
            "no quad when length ≤ 0"
        );
    }
}
