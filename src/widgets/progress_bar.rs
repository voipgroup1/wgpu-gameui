//! Progress bar widget.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{StyleKey, StyleResolver};

use super::DrawList;
use super::material::draw_inset_shadow;

/// How a [`ProgressBar`] picks its fill color from its value — the caller-owned
/// *semantic policy*. The [`Theme`](crate::Theme) only supplies the palette
/// (`ProgressFill` / `ProgressFillLow` / `ProgressFillMedium`); this type decides
/// which one a given value maps to, so "low = bad" is a choice the call site
/// makes rather than something baked into the widget or theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressFill {
    /// One color across the whole range, from any style key. Use this for neutral
    /// progress (downloads, loading) where low isn't "bad".
    Solid(StyleKey),
    /// Three-band "stat" coloring (health/hunger bars): `value < low` →
    /// [`ProgressFillLow`](StyleKey::ProgressFillLow), `value < medium` →
    /// [`ProgressFillMedium`](StyleKey::ProgressFillMedium), otherwise
    /// [`ProgressFill`](StyleKey::ProgressFill). Thresholds are caller-tunable.
    Stat {
        /// Below this fraction, use the "low" color band.
        low: f32,
        /// Below this fraction (but >= `low`), use the "medium" color band.
        medium: f32,
    },
}

impl Default for ProgressFill {
    /// The classic stat banding (`low` < 0.25, `medium` < 0.5) — preserves the
    /// historical built-in behavior, now as an overridable default.
    fn default() -> Self {
        ProgressFill::Stat {
            low: 0.25,
            medium: 0.5,
        }
    }
}

impl ProgressFill {
    /// Resolve the fill color for `value` under `style`.
    fn color(self, value: f32, style: &StyleResolver) -> [f32; 4] {
        match self {
            ProgressFill::Solid(key) => style.color(key),
            ProgressFill::Stat { low, medium } => {
                if value < low {
                    style.color(StyleKey::ProgressFillLow)
                } else if value < medium {
                    style.color(StyleKey::ProgressFillMedium)
                } else {
                    style.color(StyleKey::ProgressFill)
                }
            }
        }
    }
}

/// Progress bar widget - shows a value as a filled bar.
pub struct ProgressBar {
    /// Fill fraction in `0.0..=1.0`.
    pub value: f32, // 0.0 to 1.0
    /// Whether to overlay the value as a percentage label.
    pub show_text: bool, // Show percentage text
    /// Caller-owned color policy. Defaults to [`ProgressFill::default`] (stat
    /// banding); set via [`with_fill`](Self::with_fill) for solid or custom bands.
    pub fill: ProgressFill,
}

impl ProgressBar {
    /// Create a bar with `value` clamped to `0.0..=1.0` and default stat fill.
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            show_text: false,
            fill: ProgressFill::default(),
        }
    }

    /// Create from a u8 value (0-255 scale).
    pub fn from_u8(value: u8) -> Self {
        Self::new(value as f32 / 255.0)
    }

    /// Create from a u8 value with a custom max.
    pub fn from_u8_max(value: u8, max: u8) -> Self {
        if max == 0 {
            Self::new(0.0)
        } else {
            Self::new(value as f32 / max as f32)
        }
    }

    /// Toggle the overlaid percentage label.
    pub fn with_text(mut self, show: bool) -> Self {
        self.show_text = show;
        self
    }

    /// Set the fill color policy (solid vs stat banding). See [`ProgressFill`].
    pub fn with_fill(mut self, fill: ProgressFill) -> Self {
        self.fill = fill;
        self
    }

    /// Draw the progress bar at the given rect.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, style: &StyleResolver) {
        list.push_debug_scope_rect("ProgressBar", rect);
        let border_radius = style.scalar(StyleKey::BorderRadius).min(rect.height * 0.5);
        // Track: the sunken well (dark trough + inset shadow + under line).
        let track = Rect::new(rect.x, rect.y, rect.width, rect.height);
        list.chrome_rect(
            track,
            border_radius,
            1.0,
            style.color(StyleKey::ProgressBackground),
            [0.0, 0.0, 0.0, 0.6],
        );
        draw_inset_shadow(
            list,
            style,
            track,
            style.scalar(StyleKey::InnerShadowDepth),
            1.0,
        );

        // Fill - color from the caller-owned policy, painted as the accent
        // gradient (brighter at the top) with a 1px top highlight.
        let fill_color = self.fill.color(self.value, style);

        let fill_width = (rect.width * self.value).min((rect.width - 2.0).max(0.0));
        if fill_width > 0.0 {
            let fill_rect = Rect::new(
                rect.x + 1.0,
                rect.y + 1.0,
                fill_width,
                (rect.height - 2.0).max(0.0),
            );
            list.chrome_rect_gradient(
                fill_rect,
                border_radius,
                0.0,
                fill_color,
                [
                    fill_color[0] * 0.8,
                    fill_color[1] * 0.8,
                    fill_color[2] * 0.82,
                    fill_color[3],
                ],
                [0.0; 4],
            );
            let hl = style.color(StyleKey::EdgeHighlight);
            list.quad(
                fill_rect.x,
                fill_rect.y,
                fill_rect.width,
                1.0,
                [hl[0], hl[1], hl[2], 0.35],
            );
        }

        // Text (percentage)
        if self.show_text {
            let pct = (self.value * 100.0) as u32;
            let text = format!("{}%", pct);
            let font_size = rect.height * 0.7;
            let (text_width, _) = list.measure_text(&text, font_size, None);
            let text_x = rect.x + (rect.width - text_width) / 2.0;
            let text_y = list.vcentered_text_y(
                rect.y,
                rect.height,
                font_size,
                style.theme().font.as_ref(),
                &text,
            );

            let text_color = style.color(StyleKey::Text);
            let block = TextBlock::new(&text, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_font_opt(style.theme().font.clone());
            list.text(block);
        }
        list.pop_debug_scope();
    }

    /// Draw with a label to the left.
    pub fn draw_labeled(
        &self,
        label: &str,
        label_width: f32,
        rect: Rect,
        list: &mut DrawList,
        style: &StyleResolver,
    ) {
        // Unlike the other thin wrappers this one is scoped, because it passes a
        // *derived* rect inward: `ProgressBar (labeled)` declaring `rect` with a
        // child `ProgressBar` declaring `bar_rect` is the layout fact worth
        // recording, and the only thing that catches label_width >= rect.width.
        list.push_debug_scope_rect(crate::widgets::scope_name("ProgressBar", label), rect);

        // Label on the left
        let font_size = style.scalar(StyleKey::FontSize) * 0.75;
        let label_y = list.vcentered_text_y(
            rect.y,
            rect.height,
            font_size,
            style.theme().font.as_ref(),
            label,
        );
        let text_color = style.color(StyleKey::Text);
        let label_block = TextBlock::new(label, rect.x, label_y)
            .with_size(font_size)
            .with_color(
                (text_color[0] * 255.0) as u8,
                (text_color[1] * 255.0) as u8,
                (text_color[2] * 255.0) as u8,
            )
            .with_font_opt(style.theme().font.clone());
        list.text(label_block);

        // Progress bar on the right
        let bar_rect = Rect::new(
            rect.x + label_width,
            rect.y,
            rect.width - label_width,
            rect.height,
        );
        self.draw(bar_rect, list, style);
        list.pop_debug_scope();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    /// The fill quad is the second chrome instance (background is first).
    fn fill_color(bar: &ProgressBar, theme: &Theme) -> [f32; 4] {
        // The draw path paints exactly this policy color as the fill's gradient
        // top (the visual layering around it is covered by the gallery).
        let style = StyleResolver::new(theme);
        bar.fill.color(bar.value, &style)
    }

    #[test]
    fn default_policy_is_stat_banding() {
        assert_eq!(
            ProgressFill::default(),
            ProgressFill::Stat {
                low: 0.25,
                medium: 0.5
            }
        );
        assert_eq!(ProgressBar::new(0.5).fill, ProgressFill::default());
    }

    #[test]
    fn stat_banding_picks_palette_by_threshold() {
        let t = theme();
        assert_eq!(fill_color(&ProgressBar::new(0.10), &t), t.progress_fill_low);
        assert_eq!(
            fill_color(&ProgressBar::new(0.40), &t),
            t.progress_fill_medium
        );
        assert_eq!(fill_color(&ProgressBar::new(0.90), &t), t.progress_fill);
    }

    #[test]
    fn custom_thresholds_shift_the_bands() {
        let t = theme();
        // With low=0.5 the 0.40 value now reads as "low" rather than "medium".
        let bar = ProgressBar::new(0.40).with_fill(ProgressFill::Stat {
            low: 0.5,
            medium: 0.8,
        });
        assert_eq!(fill_color(&bar, &t), t.progress_fill_low);
    }

    #[test]
    fn solid_fill_ignores_value() {
        let t = theme();
        // A neutral solid bar uses one key regardless of value (low isn't "bad").
        let low = ProgressBar::new(0.05).with_fill(ProgressFill::Solid(StyleKey::Accent));
        let high = ProgressBar::new(0.95).with_fill(ProgressFill::Solid(StyleKey::Accent));
        assert_eq!(fill_color(&low, &t), t.accent);
        assert_eq!(fill_color(&high, &t), t.accent);
    }
}
