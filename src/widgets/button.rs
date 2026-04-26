//! Button widget.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;

/// Button widget.
pub struct Button {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub label: String,
    pub enabled: bool,
}

impl Button {
    pub fn new(label: impl Into<String>, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            label: label.into(),
            enabled: true,
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draw the button and return true if clicked.
    pub fn draw(&self, list: &mut DrawList, theme: &Theme, input: &InputState) -> bool {
        let hovered = self.enabled && input.is_hovered(self.x, self.y, self.width, self.height);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;

        let bg_color = if !self.enabled {
            let mut c = theme.button;
            c[3] = 0.5;
            c
        } else if pressed {
            theme.button_pressed
        } else if hovered {
            theme.button_hover
        } else {
            theme.button
        };

        if theme.border_radius > 0.0 {
            list.rounded_rect(
                Rect::new(self.x, self.y, self.width, self.height),
                theme.border_radius,
                bg_color,
            );
        } else {
            list.quad(self.x, self.y, self.width, self.height, bg_color);
        }

        let border = theme.border_width;
        let border_color = if hovered && self.enabled {
            theme.accent
        } else {
            theme.button_border
        };

        list.quad(self.x, self.y, self.width, border, border_color);
        list.quad(
            self.x,
            self.y + self.height - border,
            self.width,
            border,
            border_color,
        );
        list.quad(self.x, self.y, border, self.height, border_color);
        list.quad(
            self.x + self.width - border,
            self.y,
            border,
            self.height,
            border_color,
        );

        let text_color = if self.enabled {
            theme.text
        } else {
            theme.text_dim
        };
        let text = TextBlock::new(
            &self.label,
            self.x + theme.padding,
            self.y + (self.height - theme.font_size) / 2.0,
        )
        .with_size(theme.font_size)
        .with_color(
            (text_color[0] * 255.0) as u8,
            (text_color[1] * 255.0) as u8,
            (text_color[2] * 255.0) as u8,
        )
        .with_max_width(self.width - theme.padding * 2.0);
        list.text(text);

        clicked
    }

    /// Draw the button at a layout-computed rect. Returns true if clicked.
    pub fn draw_at(
        label: &str,
        rect: Rect,
        enabled: bool,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
    ) -> bool {
        let hovered = enabled && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;

        let bg_color = if !enabled {
            let mut c = theme.button;
            c[3] = 0.5;
            c
        } else if pressed {
            theme.button_pressed
        } else if hovered {
            theme.button_hover
        } else {
            theme.button
        };

        if theme.border_radius > 0.0 {
            list.rounded_rect(rect, theme.border_radius, bg_color);
        } else {
            list.quad(rect.x, rect.y, rect.width, rect.height, bg_color);
        }

        let border = theme.border_width;
        let border_color = if hovered && enabled {
            theme.accent
        } else {
            theme.button_border
        };

        list.quad(rect.x, rect.y, rect.width, border, border_color);
        list.quad(
            rect.x,
            rect.y + rect.height - border,
            rect.width,
            border,
            border_color,
        );
        list.quad(rect.x, rect.y, border, rect.height, border_color);
        list.quad(
            rect.x + rect.width - border,
            rect.y,
            border,
            rect.height,
            border_color,
        );

        let text_color = if enabled { theme.text } else { theme.text_dim };
        let text = TextBlock::new(
            label,
            rect.x + theme.padding,
            rect.y + (rect.height - theme.font_size) / 2.0,
        )
        .with_size(theme.font_size)
        .with_color(
            (text_color[0] * 255.0) as u8,
            (text_color[1] * 255.0) as u8,
            (text_color[2] * 255.0) as u8,
        )
        .with_max_width(rect.width - theme.padding * 2.0);
        list.text(text);

        clicked
    }

    /// Draw a nine-slice textured button at a layout-computed rect. Returns true if clicked.
    pub fn draw_nine_slice(
        label: &str,
        rect: Rect,
        enabled: bool,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
        texture_key: &str,
    ) -> bool {
        let hovered = enabled && rect.contains(input.mouse_x, input.mouse_y);
        let clicked = hovered && input.mouse_clicked;

        list.nine_slice(rect.x, rect.y, rect.width, rect.height, texture_key);

        if !enabled {
            list.quad(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                [0.0, 0.0, 0.0, 0.4],
            );
        } else if hovered && input.mouse_down {
            list.quad(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                [0.0, 0.0, 0.0, 0.2],
            );
        } else if hovered {
            list.quad(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                [1.0, 1.0, 1.0, 0.08],
            );
        }

        let text_color = if enabled { theme.text } else { theme.text_dim };
        let text = TextBlock::new(
            label,
            rect.x + theme.padding,
            rect.y + (rect.height - theme.font_size) / 2.0,
        )
        .with_size(theme.font_size)
        .with_color(
            (text_color[0] * 255.0) as u8,
            (text_color[1] * 255.0) as u8,
            (text_color[2] * 255.0) as u8,
        )
        .with_max_width(rect.width - theme.padding * 2.0);
        list.text(text);

        clicked
    }
}
