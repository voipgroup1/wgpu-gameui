//! Checkbox widget.

use crate::layout::Rect;
use crate::{InputState, Theme};
use crate::text::TextBlock;

use super::DrawList;

/// Icon keys for checkbox textures (must be loaded into the icon atlas).
pub const CHECKBOX_ICON: &str = "textures/ui/checkbox.png";
pub const CHECKBOX_CHECKED_ICON: &str = "textures/ui/checkbox_checked.png";

/// Checkbox widget - a toggleable box with an optional label.
pub struct Checkbox;

impl Checkbox {
    /// Draw a checkbox at the given rect. Returns true if clicked (toggled).
    ///
    /// The checkbox icon is drawn at the left of the rect, with the label to its right.
    pub fn draw(
        checked: bool,
        label: &str,
        rect: Rect,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
    ) -> bool {
        let hovered = rect.contains(input.mouse_x, input.mouse_y);
        let clicked = hovered && input.mouse_clicked;

        // Checkbox icon (square, fitted to rect height)
        let icon_size = rect.height;
        let icon_key = if checked {
            CHECKBOX_CHECKED_ICON
        } else {
            CHECKBOX_ICON
        };
        list.icon(icon_key, rect.x, rect.y, icon_size, icon_size);

        // Hover highlight over the icon area
        if hovered {
            list.quad(rect.x, rect.y, icon_size, icon_size, [1.0, 1.0, 1.0, 0.08]);
        }

        // Label to the right of the checkbox
        if !label.is_empty() {
            let text_x = rect.x + icon_size + 6.0;
            let text_y = rect.y + (rect.height - theme.font_size) / 2.0;
            let text_color = theme.text;
            let text = TextBlock::new(label, text_x, text_y)
                .with_size(theme.font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                );
            list.text(text);
        }

        clicked
    }
}
