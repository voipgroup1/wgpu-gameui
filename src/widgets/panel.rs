//! Panel widget and label/title helpers.

use crate::layout::Rect;
use crate::{StyleKey, StyleResolver};

use super::DrawList;

/// Panel widget - a container with background.
pub struct Panel {
    /// Left edge, in pixels.
    pub x: f32,
    /// Top edge, in pixels.
    pub y: f32,
    /// Width, in pixels.
    pub width: f32,
    /// Height, in pixels.
    pub height: f32,
}

impl Panel {
    /// Create a panel at the given rect.
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

    /// Draw the panel at its configured rect.
    pub fn draw(&self, list: &mut DrawList, style: &StyleResolver) {
        Self::draw_at(
            Rect::new(self.x, self.y, self.width, self.height),
            list,
            style,
        );
    }

    /// Draw a panel at a layout-computed rect.
    pub fn draw_at(rect: Rect, list: &mut DrawList, style: &StyleResolver) {
        let radius = style.scalar(StyleKey::BorderRadius);
        let panel = style.color(StyleKey::Panel);
        let panel_border = style.color(StyleKey::PanelBorder);
        // Draw background
        if radius > 0.0 {
            list.rounded_rect(rect, radius, panel);
        } else {
            list.quad(rect.x, rect.y, rect.width, rect.height, panel);
        }

        // Draw border (4 non-overlapping inset quads). Top/bottom span the
        // full width; left/right span only the inner height between them so
        // the corners are painted exactly once — important for any
        // semi-transparent panel_border color.
        let border = style.scalar(StyleKey::BorderWidth);
        let inner_h = (rect.height - 2.0 * border).max(0.0);
        list.quad(rect.x, rect.y, rect.width, border, panel_border);
        list.quad(
            rect.x,
            rect.y + rect.height - border,
            rect.width,
            border,
            panel_border,
        );
        list.quad(rect.x, rect.y + border, border, inner_h, panel_border);
        list.quad(
            rect.x + rect.width - border,
            rect.y + border,
            border,
            inner_h,
            panel_border,
        );
    }

    /// Draw a nine-slice textured panel at a layout-computed rect.
    pub fn draw_nine_slice(rect: Rect, list: &mut DrawList, texture_key: &str) {
        list.nine_slice(rect.x, rect.y, rect.width, rect.height, texture_key);
    }
}

/// Label - simple text display.
pub fn label(list: &mut DrawList, style: &StyleResolver, text: &str, x: f32, y: f32) {
    list.text(style.text_block(text, x, y));
}

/// Label at a layout-computed rect (vertically centered).
pub fn label_at(list: &mut DrawList, style: &StyleResolver, text: &str, rect: Rect) {
    let y = list.vcentered_text_y(
        rect.y,
        rect.height,
        style.scalar(StyleKey::FontSize),
        style.theme().font.as_ref(),
        text,
    );
    list.text(style.text_block(text, rect.x + style.scalar(StyleKey::Padding), y));
}

/// Label centered horizontally and vertically in a rect.
pub fn label_centered_at(list: &mut DrawList, style: &StyleResolver, text: &str, rect: Rect) {
    let (text_width, _) = list.measure_text(text, style.scalar(StyleKey::FontSize), None);
    let x = rect.x + (rect.width - text_width) / 2.0;
    let y = list.vcentered_text_y(
        rect.y,
        rect.height,
        style.scalar(StyleKey::FontSize),
        style.theme().font.as_ref(),
        text,
    );
    list.text(style.text_block(text, x, y));
}

/// Title - larger text display.
pub fn title(list: &mut DrawList, style: &StyleResolver, text: &str, x: f32, y: f32) {
    list.text(style.title_block(text, x, y));
}

/// Title at a layout-computed rect (vertically centered).
pub fn title_at(list: &mut DrawList, style: &StyleResolver, text: &str, rect: Rect) {
    let y = list.vcentered_text_y(
        rect.y,
        rect.height,
        style.scalar(StyleKey::FontSizeTitle),
        style.theme().font.as_ref(),
        text,
    );
    list.text(style.title_block(text, rect.x + style.scalar(StyleKey::Padding), y));
}
