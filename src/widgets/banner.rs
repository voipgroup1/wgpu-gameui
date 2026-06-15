//! Banner — an inline severity message strip (info / success / warning / error).
//!
//! Stateless and non-interactive (draws through a [`StyleResolver`] like
//! [`Panel`](super::Panel)): a tinted background with a colored accent bar down
//! the left edge, an optional bold title, and a wrapped message. It's the visual
//! building block the [`ToastStack`](super::ToastStack) renders, and is equally
//! usable on its own as a persistent inline notice.
//!
//! ```no_run
//! # use wgpu_gameui::{Banner, Severity, layout::Rect};
//! # fn demo(list: &mut wgpu_gameui::DrawList, style: &wgpu_gameui::StyleResolver) {
//! Banner::error("Save failed: disk full")
//!     .with_title("Error")
//!     .draw(Rect::new(0.0, 0.0, 320.0, 56.0), list, style);
//! # }
//! ```

use crate::layout::Rect;
use crate::{StyleKey, StyleResolver};

use super::DrawList;

/// Severity level of a [`Banner`] / toast — selects the accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Neutral informational notice (blue).
    Info,
    /// Success / confirmation (green).
    Success,
    /// Caution (amber).
    Warning,
    /// Failure / error (red).
    Error,
}

impl Severity {
    /// The accent color for this severity.
    ///
    /// Fixed RGBA rather than theme-sourced: the [`Theme`](crate::Theme) has no
    /// success/warning colors yet (only `accent`/`error`), so the four levels are
    /// defined here for a consistent, legible palette. The separate "semantic
    /// policy out of theme" P2 can later route these through the style system.
    pub fn accent(self) -> [f32; 4] {
        match self {
            Severity::Info => [0.22, 0.55, 0.95, 1.0],
            Severity::Success => [0.26, 0.72, 0.42, 1.0],
            Severity::Warning => [0.95, 0.70, 0.20, 1.0],
            Severity::Error => [0.90, 0.32, 0.34, 1.0],
        }
    }

    /// Translucent background tint derived from the accent.
    fn background(self) -> [f32; 4] {
        let a = self.accent();
        [a[0], a[1], a[2], 0.16]
    }
}

/// Geometry constant: width of the left accent bar in px.
const BAR_W: f32 = 4.0;

/// An inline severity message strip.
pub struct Banner<'a> {
    severity: Severity,
    title: Option<&'a str>,
    message: &'a str,
}

impl<'a> Banner<'a> {
    /// A banner with an explicit severity.
    pub fn new(severity: Severity, message: &'a str) -> Self {
        Self {
            severity,
            title: None,
            message,
        }
    }

    /// Info banner.
    pub fn info(message: &'a str) -> Self {
        Self::new(Severity::Info, message)
    }
    /// Success banner.
    pub fn success(message: &'a str) -> Self {
        Self::new(Severity::Success, message)
    }
    /// Warning banner.
    pub fn warning(message: &'a str) -> Self {
        Self::new(Severity::Warning, message)
    }
    /// Error banner.
    pub fn error(message: &'a str) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Add a bold title line above the message.
    pub fn with_title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// The accent color for this banner's severity.
    pub fn accent(&self) -> [f32; 4] {
        self.severity.accent()
    }

    /// Left edge where text begins, given the left padding.
    fn text_x(rect: Rect, pad: f32) -> f32 {
        rect.x + BAR_W + pad
    }

    /// Inner text width available for the message (and title).
    fn inner_width(width: f32, pad: f32) -> f32 {
        (width - BAR_W - 2.0 * pad).max(0.0)
    }

    /// Title line height for a given font size (0 when there's no title).
    fn title_height(&self, font_size: f32) -> f32 {
        if self.title.is_some() {
            font_size * 1.3
        } else {
            0.0
        }
    }

    /// Natural height needed to render this banner at `width` (padding + title +
    /// wrapped message). Useful for auto-sizing a [`ToastStack`].
    pub fn measure_height(&self, list: &mut DrawList, style: &StyleResolver, width: f32) -> f32 {
        let pad = style.scalar(StyleKey::Padding);
        let font_size = style.scalar(StyleKey::FontSize);
        let inner_w = Self::inner_width(width, pad);
        let (_, msg_h) = list.measure_text(self.message, font_size, Some(inner_w));
        2.0 * pad + self.title_height(font_size) + msg_h
    }

    /// Draw the banner filling `rect`.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, style: &StyleResolver) {
        let pad = style.scalar(StyleKey::Padding);
        let font_size = style.scalar(StyleKey::FontSize);
        let accent = self.severity.accent();

        // Tinted background + left accent bar.
        list.quad(rect.x, rect.y, rect.width, rect.height, self.severity.background());
        list.quad(rect.x, rect.y, BAR_W, rect.height, accent);

        let text_x = Self::text_x(rect, pad);
        let inner_w = Self::inner_width(rect.width, pad);
        let mut cursor_y = rect.y + pad;

        if let Some(title) = self.title {
            let block = style
                .text_block(title, text_x, cursor_y)
                .with_size(font_size)
                .with_color(
                    (accent[0] * 255.0) as u8,
                    (accent[1] * 255.0) as u8,
                    (accent[2] * 255.0) as u8,
                );
            list.text(block);
            cursor_y += self.title_height(font_size);
        }

        let text = style.color(StyleKey::Text);
        let block = style
            .text_block(self.message, text_x, cursor_y)
            .with_size(font_size)
            .with_color(
                (text[0] * 255.0) as u8,
                (text[1] * 255.0) as u8,
                (text[2] * 255.0) as u8,
            )
            .with_max_width(inner_w);
        list.text(block);
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
    fn each_severity_has_a_distinct_accent() {
        let colors = [
            Severity::Info.accent(),
            Severity::Success.accent(),
            Severity::Warning.accent(),
            Severity::Error.accent(),
        ];
        for (i, a) in colors.iter().enumerate() {
            for b in &colors[i + 1..] {
                assert_ne!(a, b, "severity accents must differ");
            }
        }
    }

    #[test]
    fn convenience_ctors_pick_the_right_severity() {
        assert_eq!(Banner::info("x").accent(), Severity::Info.accent());
        assert_eq!(Banner::success("x").accent(), Severity::Success.accent());
        assert_eq!(Banner::warning("x").accent(), Severity::Warning.accent());
        assert_eq!(Banner::error("x").accent(), Severity::Error.accent());
    }

    #[test]
    fn title_adds_height() {
        let s = style();
        let mut list = DrawList::new();
        let plain = Banner::info("hello world").measure_height(&mut list, &s, 200.0);
        let titled = Banner::info("hello world")
            .with_title("Heads up")
            .measure_height(&mut list, &s, 200.0);
        assert!(titled > plain, "a title makes the banner taller");
    }

    #[test]
    fn narrower_width_wraps_taller() {
        let s = style();
        let mut list = DrawList::new();
        let msg = "This is a reasonably long message that should wrap onto multiple lines.";
        let wide = Banner::info(msg).measure_height(&mut list, &s, 400.0);
        let narrow = Banner::info(msg).measure_height(&mut list, &s, 120.0);
        assert!(narrow > wide, "narrower banner wraps to more lines");
    }

    #[test]
    fn draw_emits_bar_background_and_text() {
        let s = style();
        let mut list = DrawList::new();
        Banner::error("Disk full")
            .with_title("Error")
            .draw(Rect::new(0.0, 0.0, 300.0, 60.0), &mut list, &s);
        // background + accent bar → 2 chrome instances; title + message → 2 texts.
        assert_eq!(list.chrome_instances.len(), 2);
        assert_eq!(list.texts.len(), 2);
        assert_eq!(list.texts[0].content, "Error");
        assert_eq!(list.texts[1].content, "Disk full");
    }

    #[test]
    fn untitled_draws_one_text() {
        let s = style();
        let mut list = DrawList::new();
        Banner::success("Saved").draw(Rect::new(0.0, 0.0, 200.0, 40.0), &mut list, &s);
        assert_eq!(list.texts.len(), 1);
        assert_eq!(list.texts[0].content, "Saved");
    }
}
