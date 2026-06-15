//! Tabs widget - a row of tab buttons.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{InputState, StyleKey, StyleResolver};

use super::DrawList;

/// Output from drawing tabs.
pub struct TabsOutput {
    /// The tab that was clicked, if any.
    pub clicked: Option<usize>,
    /// The rect occupied by the tab bar.
    pub rect: Rect,
}

/// Tabs widget - a row of tab buttons.
///
/// # Example
/// ```ignore
/// let output = Tabs::new(&["Overview", "Stats", "Needs"])
///     .draw(rect, active_tab, &mut draw_list, &theme, &input);
/// if let Some(clicked) = output.clicked {
///     active_tab = clicked;
/// }
/// ```
pub struct Tabs<'a> {
    labels: &'a [&'a str],
    tab_height: f32,
}

impl<'a> Tabs<'a> {
    pub fn new(labels: &'a [&'a str]) -> Self {
        Self {
            labels,
            tab_height: 28.0,
        }
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.tab_height = height;
        self
    }

    /// Draw the tabs at the top of the given rect.
    /// Returns which tab was clicked (if any) and the rect used.
    pub fn draw(
        &self,
        rect: Rect,
        active: usize,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &InputState,
    ) -> TabsOutput {
        let tab_count = self.labels.len();
        if tab_count == 0 {
            return TabsOutput {
                clicked: None,
                rect: Rect::new(rect.x, rect.y, rect.width, 0.0),
            };
        }

        let tab_width = rect.width / tab_count as f32;
        let bar_rect = Rect::new(rect.x, rect.y, rect.width, self.tab_height);
        let mut clicked = None;

        // Draw background for entire tab bar
        list.quad(
            bar_rect.x,
            bar_rect.y,
            bar_rect.width,
            bar_rect.height,
            style.color(StyleKey::TabInactive),
        );

        // Draw each tab
        for (i, label) in self.labels.iter().enumerate() {
            let tab_x = rect.x + i as f32 * tab_width;
            let tab_rect = Rect::new(tab_x, rect.y, tab_width, self.tab_height);

            let is_active = i == active;
            let is_hovered = tab_rect.contains(input.mouse_x, input.mouse_y);
            let is_clicked = is_hovered && input.mouse_clicked;

            if is_clicked {
                clicked = Some(i);
            }

            // Tab background
            let bg_color = if is_active {
                style.color(StyleKey::TabActive)
            } else if is_hovered {
                style.color(StyleKey::TabHover)
            } else {
                style.color(StyleKey::TabInactive)
            };
            list.quad(tab_x, rect.y, tab_width, self.tab_height, bg_color);

            // Active indicator (bottom border for active tab)
            if is_active {
                list.quad(
                    tab_x,
                    rect.y + self.tab_height - 2.0,
                    tab_width,
                    2.0,
                    style.color(StyleKey::Accent),
                );
            }

            // Tab separator (right edge, except for last tab)
            if i < tab_count - 1 {
                list.quad(
                    tab_x + tab_width - 1.0,
                    rect.y + 4.0,
                    1.0,
                    self.tab_height - 8.0,
                    style.color(StyleKey::TabBorder),
                );
            }

            // Tab label (centered)
            let text_color = if is_active {
                style.color(StyleKey::TextHighlight)
            } else {
                style.color(StyleKey::Text)
            };
            let font_size = style.scalar(StyleKey::FontSize) * 0.8;
            let text_y = list.vcentered_text_y(
                rect.y,
                self.tab_height,
                font_size,
                style.theme().font.as_ref(),
                label,
            );
            let (text_width, _) = list.measure_text(label, font_size, None);
            let text_x = tab_x + (tab_width - text_width) / 2.0;

            let text = TextBlock::new(*label, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_font_opt(style.theme().font.clone());
            list.text(text);
        }

        // Bottom border for tab bar
        list.quad(
            rect.x,
            rect.y + self.tab_height - 1.0,
            rect.width,
            1.0,
            style.color(StyleKey::TabBorder),
        );

        TabsOutput {
            clicked,
            rect: bar_rect,
        }
    }
}
