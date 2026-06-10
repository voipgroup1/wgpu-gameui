//! Slider widget - a horizontal bar with a draggable scrubber.

use crate::layout::Rect;
use crate::{InputState, Theme};
use crate::text::TextBlock;

use super::{DragCapture, DragId, DrawList};

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
/// Drag ownership is arbitrated through a caller-owned [`DragCapture`] keyed by
/// a stable [`DragId`], so adjacent or overlapping sliders never both follow a
/// single mouse drag. Give each slider a distinct id and share one
/// `DragCapture` across the whole UI surface.
///
/// # Example
/// ```ignore
/// // `capture` is a caller-owned `DragCapture`, persisted across frames.
/// let output = Slider::new(0.0, 100.0)
///     .draw(value, 0, &mut capture, rect, &mut draw_list, &theme, &input);
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

    /// Draw the slider.
    ///
    /// `id` is this slider's stable [`DragId`] and `capture` is the caller-owned
    /// [`DragCapture`] shared across the UI. The slider only begins a drag when
    /// the capture is free, and only updates its value while it owns the
    /// capture — so a fast mouse drag can't bleed between adjacent sliders.
    pub fn draw(
        &self,
        value: f32,
        id: DragId,
        capture: &mut DragCapture,
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

        // Interaction: start drag on click, continue while mouse held.
        // Honor layer capture (`mouse_consumed`) so a slider under a modal/popup
        // doesn't grab the drag.
        let track_rect = Rect::new(rect.x, rect.y, track_width, rect.height);
        let hovered = track_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;

        // Release first so a mouse-up frame reports not-dragging. `release` is a
        // no-op unless we own the capture, so it never clobbers another
        // slider's active drag.
        if !input.mouse_down {
            capture.release(id);
        }
        // Then claim the drag if nothing else already owns it this gesture.
        if hovered && input.mouse_clicked && capture.is_free() {
            capture.try_begin(id);
        }
        let dragging = capture.is_active(id);

        // Calculate new value from mouse position while dragging
        let mut new_value = value;
        if dragging && slide_range > 0.0 {
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
        if dragging || hovered {
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
            dragging,
            changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::default()
    }

    /// Mouse pressed (down + clicked this frame) at (x, y).
    fn press_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    /// Mouse held (down, not a fresh click) at (x, y).
    fn hold_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            ..InputState::default()
        }
    }

    /// Mouse released at (x, y).
    fn release_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn rect() -> Rect {
        // 100px-wide track at the origin, 20px tall.
        Rect::new(0.0, 0.0, 100.0, 20.0)
    }

    #[test]
    fn click_in_track_starts_drag_and_sets_value() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let mut list = DrawList::new();
        // Press near the right of the track (interior; the right edge is
        // exclusive) -> value above the starting 50.
        let input = press_at(85.0, 10.0);
        let out = slider.draw(50.0, 0, &mut cap, rect(), &mut list, &theme(), &input);
        assert!(out.dragging, "click inside the track should begin a drag");
        assert!(cap.is_active(0));
        assert!(out.value > 50.0, "dragging to the right edge raises the value");
    }

    #[test]
    fn release_ends_drag() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let mut list = DrawList::new();

        let down = press_at(50.0, 10.0);
        slider.draw(50.0, 0, &mut cap, rect(), &mut list, &theme(), &down);
        assert!(cap.is_active(0));

        let up = release_at(50.0, 10.0);
        let out = slider.draw(50.0, 0, &mut cap, rect(), &mut list, &theme(), &up);
        assert!(!out.dragging);
        assert!(cap.is_free(), "mouse-up frees the capture");
    }

    #[test]
    fn second_slider_cannot_steal_active_drag() {
        // Slider A (id 0) and slider B (id 1) occupy the *same* rect, so a naive
        // `bool`-per-slider model would have both follow the mouse. With shared
        // capture, only the first to claim it reacts.
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let mut list = DrawList::new();
        let r = rect();

        // Frame 1: press. A is drawn first and claims the drag.
        let down = press_at(20.0, 10.0);
        let a1 = slider.draw(10.0, 0, &mut cap, r, &mut list, &theme(), &down);
        let b1 = slider.draw(90.0, 1, &mut cap, r, &mut list, &theme(), &down);
        assert!(a1.dragging, "first slider claims the drag");
        assert!(!b1.dragging, "second slider must not also grab the same press");
        assert!(cap.is_active(0));

        // Frame 2: mouse moves while held. Only A tracks it.
        let mov = hold_at(80.0, 10.0);
        let a2 = slider.draw(a1.value, 0, &mut cap, r, &mut list, &theme(), &mov);
        let b2 = slider.draw(90.0, 1, &mut cap, r, &mut list, &theme(), &mov);
        assert!(a2.dragging);
        assert!(!b2.dragging);
        assert!(!b2.changed, "non-owning slider's value is untouched");
    }

    #[test]
    fn drag_continues_across_frames_until_release() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let mut list = DrawList::new();
        let r = rect();

        let down = press_at(50.0, 10.0);
        let mut value = slider
            .draw(50.0, 0, &mut cap, r, &mut list, &theme(), &down)
            .value;

        // Mouse leaves the track rect vertically but stays held: owner keeps
        // tracking because capture, not hit-testing, gates an in-progress drag.
        let mov = hold_at(95.0, 500.0);
        let out = slider.draw(value, 0, &mut cap, r, &mut list, &theme(), &mov);
        assert!(out.dragging, "held drag continues even when cursor leaves the rect");
        value = out.value;
        assert!(value > 50.0);

        let up = release_at(95.0, 500.0);
        let out = slider.draw(value, 0, &mut cap, r, &mut list, &theme(), &up);
        assert!(!out.dragging);
        assert!(cap.is_free());
    }

    #[test]
    fn consumed_mouse_does_not_start_drag() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let mut list = DrawList::new();
        let mut input = press_at(50.0, 10.0);
        input.mouse_consumed = true; // a higher layer already took this click
        let out = slider.draw(50.0, 0, &mut cap, rect(), &mut list, &theme(), &input);
        assert!(!out.dragging);
        assert!(cap.is_free());
    }
}
