//! Slider widget - a horizontal bar with a draggable scrubber.

use crate::layout::Rect;
use crate::{InputState, Theme};
use crate::text::TextBlock;

use super::DrawList;

/// Icon key for the scrubber texture (must be loaded into the icon atlas).
pub const SLIDER_SCRUBBER_ICON: &str = "textures/ui/scrubber.png";

/// Nine-slice key for the track texture.
pub const SLIDER_TRACK_NINE_SLICE: &str = "track";

/// Output from drawing a slider.
pub struct SliderOutput {
    /// The new value after interaction (always in `min..=max` range).
    pub value: f32,
    /// Whether the slider was actively being dragged this frame.
    pub dragging: bool,
    /// Whether the value changed this frame.
    pub changed: bool,
}

/// Slider widget - a horizontal track with a draggable scrubber handle.
///
/// The scrubber is drawn as an icon (not nine-sliced) and is capped at 20px tall.
/// The track is nine-sliced horizontally.
///
/// # Example
/// ```ignore
/// let output = Slider::new(0.0, 100.0)
///     .draw(value, &mut dragging, rect, &mut draw_list, &theme, &input);
/// if output.changed {
///     value = output.value;
/// }
/// ```
pub struct Slider {
    min: f32,
    max: f32,
    step: Option<f32>,
    show_value: bool,
}

impl Slider {
    pub fn new(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            step: None,
            show_value: false,
        }
    }

    /// Snap to discrete steps (e.g., 1.0 for integer values).
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }

    /// Show the current value as text to the right of the slider.
    pub fn with_value_display(mut self, show: bool) -> Self {
        self.show_value = show;
        self
    }

    /// Draw the slider. `dragging` is persistent state the caller must store
    /// between frames to track whether the user is mid-drag.
    pub fn draw(
        &self,
        value: f32,
        dragging: &mut bool,
        rect: Rect,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
    ) -> SliderOutput {
        let scrubber_size = rect.height.min(20.0);
        let value_text_width = if self.show_value { 40.0 } else { 0.0 };

        // Track area (full width minus optional value text)
        let track_width = rect.width - value_text_width;
        let track_height = rect.height.min(12.0);
        let track_y = rect.y + (rect.height - track_height) / 2.0;

        // Scrubber slides within the track, inset by half scrubber width on each side
        let half_scrub = scrubber_size / 2.0;
        let slide_left = rect.x + half_scrub;
        let slide_right = rect.x + track_width - half_scrub;
        let slide_range = slide_right - slide_left;

        let range = self.max - self.min;

        // Interaction: start drag on click, continue while mouse held
        let track_rect = Rect::new(rect.x, rect.y, track_width, rect.height);
        let hovered = track_rect.contains(input.mouse_x, input.mouse_y);

        if hovered && input.mouse_clicked {
            *dragging = true;
        }
        if !input.mouse_down {
            *dragging = false;
        }

        // Calculate new value from mouse position while dragging
        let mut new_value = value;
        if *dragging && slide_range > 0.0 {
            let mouse_t = ((input.mouse_x - slide_left) / slide_range).clamp(0.0, 1.0);
            new_value = self.min + mouse_t * range;

            if let Some(step) = self.step {
                new_value = (new_value / step).round() * step;
            }
            new_value = new_value.clamp(self.min, self.max);
        }

        let changed = (new_value - value).abs() > f32::EPSILON;

        // Recalculate scrubber position with potentially updated value
        let display_t = if range > 0.0 {
            ((new_value - self.min) / range).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let display_scrubber_x = slide_left + display_t * slide_range;

        // Draw track (nine-slice)
        list.nine_slice(rect.x, track_y, track_width, track_height, SLIDER_TRACK_NINE_SLICE);

        // Draw scrubber (icon, centered on current position)
        let scrubber_x = display_scrubber_x - scrubber_size / 2.0;
        let scrubber_y = rect.y + (rect.height - scrubber_size) / 2.0;
        list.icon(SLIDER_SCRUBBER_ICON, scrubber_x, scrubber_y, scrubber_size, scrubber_size);

        // Hover highlight on scrubber
        if *dragging || hovered {
            list.quad(scrubber_x, scrubber_y, scrubber_size, scrubber_size, [1.0, 1.0, 1.0, 0.06]);
        }

        // Value text
        if self.show_value {
            let display = if self.step.is_some_and(|s| s >= 1.0) {
                format!("{}", new_value as i32)
            } else {
                format!("{:.1}", new_value)
            };
            let text_color = theme.text;
            let font_size = theme.font_size * 0.8;
            let text_x = rect.x + track_width + 6.0;
            let text_y = rect.y + (rect.height - font_size) / 2.0;
            let block = TextBlock::new(&display, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                );
            list.text(block);
        }

        SliderOutput {
            value: new_value,
            dragging: *dragging,
            changed,
        }
    }
}
