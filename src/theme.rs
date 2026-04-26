//! UI theming - colors, fonts, spacing.

use crate::text::TextBlock;

/// UI theme with colors and styling.
#[derive(Clone)]
pub struct Theme {
    // Colors
    pub background: [f32; 4],
    pub panel: [f32; 4],
    pub panel_border: [f32; 4],
    pub button: [f32; 4],
    pub button_hover: [f32; 4],
    pub button_pressed: [f32; 4],
    pub button_border: [f32; 4],
    pub input_background: [f32; 4],
    pub input_border: [f32; 4],
    pub input_focus_border: [f32; 4],
    pub text: [f32; 4],
    pub text_dim: [f32; 4],
    pub text_highlight: [f32; 4],
    pub accent: [f32; 4],
    pub error: [f32; 4],

    // Tab colors
    pub tab_inactive: [f32; 4],
    pub tab_active: [f32; 4],
    pub tab_hover: [f32; 4],
    pub tab_border: [f32; 4],

    // Progress bar colors
    pub progress_background: [f32; 4],
    pub progress_fill: [f32; 4],
    pub progress_fill_low: [f32; 4],    // For low values (e.g., hunger critical)
    pub progress_fill_medium: [f32; 4], // For medium values

    // Sizing
    pub padding: f32,
    pub spacing: f32,
    pub border_radius: f32,
    pub border_width: f32,
    pub font_size: f32,
    pub font_size_title: f32,
    pub button_height: f32,
    pub input_height: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Dark, polished color scheme
            background: [0.08, 0.08, 0.12, 1.0],
            panel: [0.12, 0.12, 0.18, 0.95],
            panel_border: [0.25, 0.25, 0.35, 1.0],
            button: [0.18, 0.18, 0.25, 1.0],
            button_hover: [0.22, 0.22, 0.32, 1.0],
            button_pressed: [0.15, 0.15, 0.22, 1.0],
            button_border: [0.3, 0.3, 0.4, 1.0],
            input_background: [0.06, 0.06, 0.10, 1.0],
            input_border: [0.25, 0.25, 0.35, 1.0],
            input_focus_border: [0.4, 0.5, 0.8, 1.0],
            text: [0.9, 0.9, 0.95, 1.0],
            text_dim: [0.7, 0.7, 0.8, 1.0],
            text_highlight: [0.6, 0.8, 1.0, 1.0],
            accent: [0.3, 0.5, 0.9, 1.0],
            error: [0.9, 0.3, 0.3, 1.0],

            // Tab colors
            tab_inactive: [0.15, 0.15, 0.20, 1.0],
            tab_active: [0.20, 0.20, 0.28, 1.0],
            tab_hover: [0.18, 0.18, 0.25, 1.0],
            tab_border: [0.30, 0.30, 0.40, 1.0],

            // Progress bar colors
            progress_background: [0.10, 0.10, 0.15, 1.0],
            progress_fill: [0.3, 0.7, 0.4, 1.0],        // Green for good
            progress_fill_low: [0.8, 0.3, 0.3, 1.0],    // Red for critical
            progress_fill_medium: [0.8, 0.7, 0.2, 1.0], // Yellow for medium

            // Sizing
            padding: 16.0,
            spacing: 12.0,
            border_radius: 6.0,
            border_width: 1.0,
            font_size: 16.0,
            font_size_title: 28.0,
            button_height: 44.0,
            input_height: 40.0,
        }
    }
}

impl Theme {
    /// Create a text block with theme styling
    pub fn text(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size)
            .with_color(
                (self.text[0] * 255.0) as u8,
                (self.text[1] * 255.0) as u8,
                (self.text[2] * 255.0) as u8,
            )
    }

    /// Create a title text block
    pub fn title(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size_title)
            .with_color(
                (self.text[0] * 255.0) as u8,
                (self.text[1] * 255.0) as u8,
                (self.text[2] * 255.0) as u8,
            )
    }
}
