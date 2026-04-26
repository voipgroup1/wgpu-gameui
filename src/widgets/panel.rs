//! Panel widget and label/title helpers.

use crate::Theme;
use crate::layout::Rect;

use super::DrawList;

/// Panel widget - a container with background.
pub struct Panel {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Panel {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Center the panel on screen.
    pub fn centered(width: f32, height: f32, screen_width: f32, screen_height: f32) -> Self {
        Self {
            x: (screen_width - width) / 2.0,
            y: (screen_height - height) / 2.0,
            width,
            height,
        }
    }

    pub fn draw(&self, list: &mut DrawList, theme: &Theme) {
        // Draw background
        let rect = Rect::new(self.x, self.y, self.width, self.height);
        if theme.border_radius > 0.0 {
            list.rounded_rect(rect, theme.border_radius, theme.panel);
        } else {
            list.quad(self.x, self.y, self.width, self.height, theme.panel);
        }

        // Draw border (4 inset quads — no corner overlap, no half-thickness bleed)
        let border = theme.border_width;
        list.quad(self.x, self.y, self.width, border, theme.panel_border);
        list.quad(
            self.x,
            self.y + self.height - border,
            self.width,
            border,
            theme.panel_border,
        );
        list.quad(self.x, self.y, border, self.height, theme.panel_border);
        list.quad(
            self.x + self.width - border,
            self.y,
            border,
            self.height,
            theme.panel_border,
        );
    }

    /// Draw a panel at a layout-computed rect.
    pub fn draw_at(rect: Rect, list: &mut DrawList, theme: &Theme) {
        // Draw background
        if theme.border_radius > 0.0 {
            list.rounded_rect(rect, theme.border_radius, theme.panel);
        } else {
            list.quad(rect.x, rect.y, rect.width, rect.height, theme.panel);
        }

        // Draw border (4 inset quads — no corner overlap, no half-thickness bleed)
        let border = theme.border_width;
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
    }

    /// Draw a nine-slice textured panel at a layout-computed rect.
    pub fn draw_nine_slice(rect: Rect, list: &mut DrawList, texture_key: &str) {
        list.nine_slice(rect.x, rect.y, rect.width, rect.height, texture_key);
    }
}

/// Label - simple text display.
pub fn label(list: &mut DrawList, theme: &Theme, text: &str, x: f32, y: f32) {
    list.text(theme.text(text, x, y));
}

/// Label at a layout-computed rect (vertically centered).
pub fn label_at(list: &mut DrawList, theme: &Theme, text: &str, rect: Rect) {
    let y = rect.y + (rect.height - theme.font_size) / 2.0;
    list.text(theme.text(text, rect.x + theme.padding, y));
}

/// Label centered horizontally and vertically in a rect.
pub fn label_centered_at(list: &mut DrawList, theme: &Theme, text: &str, rect: Rect) {
    let (text_width, _) = list.measure_text(text, theme.font_size, None);
    let x = rect.x + (rect.width - text_width) / 2.0;
    let y = rect.y + (rect.height - theme.font_size) / 2.0;
    list.text(theme.text(text, x, y));
}

/// Title - larger text display.
pub fn title(list: &mut DrawList, theme: &Theme, text: &str, x: f32, y: f32) {
    list.text(theme.title(text, x, y));
}

/// Title at a layout-computed rect (vertically centered).
pub fn title_at(list: &mut DrawList, theme: &Theme, text: &str, rect: Rect) {
    let y = rect.y + (rect.height - theme.font_size_title) / 2.0;
    list.text(theme.title(text, rect.x + theme.padding, y));
}
