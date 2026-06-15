//! Group / titled panel — a bordered container with a header bar.
//!
//! The workshop equivalent of a `UiWindow`: draws a [`Panel`] body (background +
//! border) with a lightened title strip across the top, and **returns the inner
//! content [`Rect`]** so the caller lays widgets/children inside it. Stateless
//! and non-interactive, so — like [`Panel`] and [`Separator`](super::Separator) —
//! it draws through a [`StyleResolver`] rather than a `DrawContext`.
//!
//! ```no_run
//! # use wgpu_gameui::{Group, layout::Rect};
//! # fn demo(list: &mut wgpu_gameui::DrawList, style: &wgpu_gameui::StyleResolver) {
//! let content = Group::new("Settings").draw(Rect::new(0.0, 0.0, 240.0, 160.0), list, style);
//! // `content` is the area below the header, inset by padding — place children here.
//! # let _ = content;
//! # }
//! ```

use crate::layout::Rect;
use crate::{StyleKey, StyleResolver};

use super::{DrawList, Panel};

/// A titled panel container. Build with [`Group::new`], optionally tweak the
/// content padding, then [`draw`](Self::draw) to render it and get the inner
/// content rect.
pub struct Group<'a> {
    title: &'a str,
    /// Content inset (px). `None` → theme [`StyleKey::Padding`].
    padding: Option<f32>,
}

impl<'a> Group<'a> {
    /// A group with the given header title.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            padding: None,
        }
    }

    /// Override the content padding (px). Defaults to the theme padding.
    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = Some(padding.max(0.0));
        self
    }

    /// Header bar height for a given padding + title font size.
    fn header_height(pad: f32, title_size: f32) -> f32 {
        title_size + 2.0 * pad
    }

    /// The inner content rect for `rect` without drawing (useful for measuring
    /// or pre-laying-out children). Clamped so width/height never go negative.
    pub fn content_rect(&self, rect: Rect, style: &StyleResolver) -> Rect {
        let pad = self.padding.unwrap_or_else(|| style.scalar(StyleKey::Padding));
        let title_size = style.scalar(StyleKey::FontSize);
        let header_h = Self::header_height(pad, title_size);
        Rect::new(
            rect.x + pad,
            rect.y + header_h + pad,
            (rect.width - 2.0 * pad).max(0.0),
            (rect.height - header_h - 2.0 * pad).max(0.0),
        )
    }

    /// Draw the group and return its inner content rect.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, style: &StyleResolver) -> Rect {
        let pad = self.padding.unwrap_or_else(|| style.scalar(StyleKey::Padding));
        let title_size = style.scalar(StyleKey::FontSize);
        let header_h = Self::header_height(pad, title_size);
        let border = style.scalar(StyleKey::BorderWidth).max(1.0);

        // Body: reuse the Panel bg + 4-quad border.
        Panel::draw_at(rect, list, style);

        // Header strip: a theme-agnostic translucent lighten so it reads as a
        // title bar on any theme, kept inside the border. A separator line under
        // it divides the header from the content.
        let inner_x = rect.x + border;
        let inner_w = (rect.width - 2.0 * border).max(0.0);
        if inner_w > 0.0 && header_h > border {
            list.quad(
                inner_x,
                rect.y + border,
                inner_w,
                header_h - border,
                [1.0, 1.0, 1.0, 0.05],
            );
            list.quad(
                inner_x,
                rect.y + header_h,
                inner_w,
                border,
                style.color(StyleKey::PanelBorder),
            );
        }

        // Title text: highlight-colored, left-padded, vertically centered in the
        // header band.
        let title_color = style.color(StyleKey::TextHighlight);
        let ty = list.vcentered_text_y(
            rect.y,
            header_h,
            title_size,
            style.theme().font.as_ref(),
            self.title,
        );
        let block = style
            .text_block(self.title, rect.x + pad, ty)
            .with_size(title_size)
            .with_color(
                (title_color[0] * 255.0) as u8,
                (title_color[1] * 255.0) as u8,
                (title_color[2] * 255.0) as u8,
            );
        list.text(block);

        self.content_rect(rect, style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, Theme};

    fn style() -> StyleResolver<'static> {
        let theme: &'static Theme = Box::leak(Box::new(Theme::default()));
        StyleResolver::new(theme)
    }

    #[test]
    fn content_rect_sits_below_header_and_is_inset() {
        let s = style();
        let pad = s.scalar(StyleKey::Padding);
        let title_size = s.scalar(StyleKey::FontSize);
        let header_h = Group::header_height(pad, title_size);

        let outer = Rect::new(10.0, 20.0, 200.0, 160.0);
        let content = Group::new("Title").content_rect(outer, &s);

        assert_eq!(content.x, outer.x + pad, "left inset by padding");
        assert_eq!(content.y, outer.y + header_h + pad, "below the header");
        assert_eq!(content.width, outer.width - 2.0 * pad);
        assert_eq!(content.height, outer.height - header_h - 2.0 * pad);
    }

    #[test]
    fn draw_returns_same_rect_as_content_rect() {
        let s = style();
        let mut list = DrawList::new();
        let outer = Rect::new(0.0, 0.0, 200.0, 160.0);
        let returned = Group::new("Title").draw(outer, &mut list, &s);
        assert_eq!(returned, Group::new("Title").content_rect(outer, &s));
    }

    #[test]
    fn with_padding_changes_inset() {
        let s = style();
        let outer = Rect::new(0.0, 0.0, 200.0, 160.0);
        let tight = Group::new("T").with_padding(2.0).content_rect(outer, &s);
        let loose = Group::new("T").with_padding(20.0).content_rect(outer, &s);
        assert!(tight.width > loose.width, "smaller padding → wider content");
        assert_eq!(tight.x, 2.0);
        assert_eq!(loose.x, 20.0);
    }

    #[test]
    fn small_rect_clamps_without_panicking() {
        let s = style();
        // Smaller than the header + padding → content collapses to 0, not negative.
        let content = Group::new("T").content_rect(Rect::new(0.0, 0.0, 4.0, 4.0), &s);
        assert_eq!(content.width, 0.0);
        assert_eq!(content.height, 0.0);
    }

    #[test]
    fn draw_emits_body_header_and_title() {
        let s = style();
        let mut list = DrawList::new();
        Group::new("Hello").draw(Rect::new(0.0, 0.0, 200.0, 120.0), &mut list, &s);
        // Panel body + border + header lighten + separator → several chrome
        // instances; exactly one text block (the title).
        assert!(
            !list.chrome_instances.is_empty(),
            "body/border/header drawn"
        );
        assert_eq!(list.texts.len(), 1, "title text drawn");
        assert_eq!(list.texts[0].content, "Hello");
    }
}
