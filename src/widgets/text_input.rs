//! Text input widget.

use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;

/// Text input widget.
pub struct TextInput {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub value: String,
    pub placeholder: String,
    pub focused: bool,
}

impl TextInput {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            value: String::new(),
            placeholder: String::new(),
            focused: false,
        }
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Draw the input and handle text changes. Returns true if clicked (to focus).
    pub fn draw(&mut self, list: &mut DrawList, theme: &Theme, input: &InputState) -> bool {
        let hovered = input.is_hovered(self.x, self.y, self.width, self.height);
        let clicked = hovered && input.mouse_clicked;

        if self.focused {
            if !input.text_input.is_empty() {
                self.value.push_str(&input.text_input);
            }
            if input.backspace_pressed && !self.value.is_empty() {
                self.value.pop();
            }
        }

        list.quad(
            self.x,
            self.y,
            self.width,
            self.height,
            theme.input_background,
        );

        let border = theme.border_width;
        let border_color = if self.focused {
            theme.input_focus_border
        } else if hovered {
            theme.accent
        } else {
            theme.input_border
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

        let (text_content, text_color) = if self.value.is_empty() {
            (&self.placeholder, theme.text_dim)
        } else {
            (&self.value, theme.text)
        };

        let text = TextBlock::new(
            text_content,
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

        if self.focused {
            let (cursor_offset, _) = list.measure_text(&self.value, theme.font_size);
            let cursor_x = self.x + theme.padding + cursor_offset;
            let cursor_y = self.y + (self.height - theme.font_size) / 2.0;
            list.quad(cursor_x, cursor_y, 2.0, theme.font_size, theme.text);
        }

        clicked
    }
}
