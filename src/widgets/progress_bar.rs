//! Progress bar widget.

use crate::Theme;
use crate::layout::Rect;
use crate::text::TextBlock;

use super::DrawList;

/// Progress bar widget - shows a value as a filled bar.
pub struct ProgressBar {
    pub value: f32,      // 0.0 to 1.0
    pub show_text: bool, // Show percentage text
}

impl ProgressBar {
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            show_text: false,
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

    pub fn with_text(mut self, show: bool) -> Self {
        self.show_text = show;
        self
    }

    /// Draw the progress bar at the given rect.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, theme: &Theme) {
        // Background
        if theme.border_radius > 0.0 {
            list.rounded_rect(rect, theme.border_radius, theme.progress_background);
        } else {
            list.quad(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                theme.progress_background,
            );
        }

        // Fill - color based on value
        let fill_color = if self.value < 0.25 {
            theme.progress_fill_low
        } else if self.value < 0.5 {
            theme.progress_fill_medium
        } else {
            theme.progress_fill
        };

        let fill_width = rect.width * self.value;
        if fill_width > 0.0 {
            list.quad(rect.x, rect.y, fill_width, rect.height, fill_color);
        }

        // Border
        let border = 1.0;
        list.quad(rect.x, rect.y, rect.width, border, theme.panel_border);
        list.quad(
            rect.x,
            rect.y + rect.height - border,
            rect.width,
            border,
            theme.panel_border,
        );
        list.quad(rect.x, rect.y, border, rect.height, theme.panel_border);
        list.quad(
            rect.x + rect.width - border,
            rect.y,
            border,
            rect.height,
            theme.panel_border,
        );

        // Text (percentage)
        if self.show_text {
            let pct = (self.value * 100.0) as u32;
            let text = format!("{}%", pct);
            let font_size = rect.height * 0.7;
            let (text_width, _) = list.measure_text(&text, font_size, None);
            let text_x = rect.x + (rect.width - text_width) / 2.0;
            let text_y =
                list.vcentered_text_y(rect.y, rect.height, font_size, theme.font.as_ref(), &text);

            let block = TextBlock::new(&text, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (theme.text[0] * 255.0) as u8,
                    (theme.text[1] * 255.0) as u8,
                    (theme.text[2] * 255.0) as u8,
                )
                .with_font_opt(theme.font.clone());
            list.text(block);
        }
    }

    /// Draw with a label to the left.
    pub fn draw_labeled(
        &self,
        label: &str,
        label_width: f32,
        rect: Rect,
        list: &mut DrawList,
        theme: &Theme,
    ) {
        // Label on the left
        let font_size = theme.font_size * 0.75;
        let label_y =
            list.vcentered_text_y(rect.y, rect.height, font_size, theme.font.as_ref(), label);
        let label_block = TextBlock::new(label, rect.x, label_y)
            .with_size(font_size)
            .with_color(
                (theme.text[0] * 255.0) as u8,
                (theme.text[1] * 255.0) as u8,
                (theme.text[2] * 255.0) as u8,
            )
            .with_font_opt(theme.font.clone());
        list.text(label_block);

        // Progress bar on the right
        let bar_rect = Rect::new(
            rect.x + label_width,
            rect.y,
            rect.width - label_width,
            rect.height,
        );
        self.draw(bar_rect, list, theme);
    }
}
