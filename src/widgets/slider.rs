//! Slider widget - a horizontal bar with a draggable scrubber.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::StyleKey;

use super::{DragCapture, DragId, DrawContext, FocusId};

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
/// Rendering is fully procedural (no textures): a rounded "pill" track filled
/// with `theme.accent` up to the current value, and a circular knob (capped at
/// 20px) drawn with `theme.text` + a `theme.button_border` outline. Sizes and
/// colours derive from the [`Theme`](crate::Theme), so it re-themes and
/// DPI-scales like the rest of the UI.
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
///     .draw(value, 0, &mut capture, rect, &mut ctx);
/// if output.changed {
///     value = output.value;
/// }
/// ```
pub struct Slider {
    min: f32,
    max: f32,
    step: Option<f32>,
    show_value: bool,
    /// When set, the slider joins the Tab ring under this [`FocusId`] and can be
    /// adjusted with the arrow keys while focused.
    focus_id: Option<FocusId>,
}

impl Slider {
    /// Create a slider spanning the inclusive `min..=max` range.
    pub fn new(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            step: None,
            show_value: false,
            focus_id: None,
        }
    }

    /// Make the slider keyboard-focusable under `id` (a [`FocusId`], distinct
    /// from the [`DragId`] used for drag arbitration): it joins the Tab ring,
    /// draws a focus ring while focused, and adjusts on Left/Down (decrement) and
    /// Right/Up (increment) by `step` (or 1/20 of the range when no step is set).
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
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
        ctx: &mut DrawContext,
    ) -> SliderOutput {
        // Snapshot the input fields up front so we can mutate `ctx` (focus
        // registration) later without holding a borrow on `ctx.input`.
        let input = ctx.input;
        let mouse_x = input.mouse_x;
        let mouse_y = input.mouse_y;
        let mouse_down = input.mouse_down;
        let mouse_clicked = input.mouse_clicked;
        let mouse_consumed = input.mouse_consumed;
        let kb_dec = input.nav.left || input.nav.down;
        let kb_inc = input.nav.right || input.nav.up;

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
        let hovered = track_rect.contains(mouse_x, mouse_y) && !mouse_consumed;

        // Release first so a mouse-up frame reports not-dragging. `release` is a
        // no-op unless we own the capture, so it never clobbers another
        // slider's active drag.
        if !mouse_down {
            capture.release(id);
        }
        // Then claim the drag if nothing else already owns it this gesture.
        let claimed_now = hovered && mouse_clicked && capture.is_free();
        if claimed_now {
            capture.try_begin(id);
        }
        let dragging = capture.is_active(id);

        // Cursor: grabbing while scrubbing, grab when hovering the track.
        if dragging {
            ctx.request_cursor(crate::CursorIcon::Grabbing);
        } else if hovered {
            ctx.request_cursor(crate::CursorIcon::Grab);
        }

        // Calculate new value from mouse position while dragging
        let mut new_value = value;
        if dragging && slide_range > 0.0 {
            let mouse_t = ((mouse_x - slide_left) / slide_range).clamp(0.0, 1.0);
            new_value = self.min + mouse_t * range;

            if let Some(step) = self.step {
                new_value = (new_value / step).round() * step;
            }
            new_value = new_value.clamp(self.min, self.max);
        }

        // Keyboard focus + arrow-key adjustment (opt-in via `focusable`).
        let mut focused = false;
        if let Some(fid) = self.focus_id {
            ctx.register_focus(fid);
            if claimed_now {
                ctx.focus.request(fid);
            }
            focused = ctx.focus.is_focused(fid);
            if focused && (kb_dec || kb_inc) {
                let kb_step = self.step.unwrap_or_else(|| (range / 20.0).abs());
                if kb_dec {
                    new_value -= kb_step;
                }
                if kb_inc {
                    new_value += kb_step;
                }
                if let Some(step) = self.step {
                    new_value = (new_value / step).round() * step;
                }
                new_value = new_value.clamp(self.min, self.max);
            }
        }

        let changed = (new_value - value).abs() > f32::EPSILON;
        let theme = ctx.theme;
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;

        // Recalculate scrubber position with potentially updated value
        let display_t = if range > 0.0 {
            ((new_value - self.min) / range).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let display_scrubber_x = slide_left + display_t * slide_range;

        // ---- Track (procedural rounded pill) ----
        let track_rect = Rect::new(rect.x, track_y, track_width, track_height);
        let track_radius = track_height * 0.5;
        list.rounded_rect(track_rect, track_radius, s.color(StyleKey::InputBackground));

        // Filled portion: from the track's left up to the knob centre.
        let fill_w = (display_scrubber_x - rect.x).clamp(0.0, track_width);
        if fill_w > 0.0 {
            list.rounded_rect(
                Rect::new(rect.x, track_y, fill_w, track_height),
                track_radius,
                s.color(StyleKey::Accent),
            );
        }

        // Subtle outline for definition, matching the rounded inputs.
        list.rounded_rect_outline(
            track_rect,
            track_radius,
            s.scalar(StyleKey::BorderWidth),
            s.color(StyleKey::InputBorder),
        );

        // ---- Scrubber (procedural circle handle) ----
        let knob_center = (display_scrubber_x, rect.y + rect.height * 0.5);
        let knob_radius = scrubber_size * 0.5;
        // Hover/drag halo behind the knob.
        if dragging || hovered {
            list.circle(knob_center, knob_radius + 2.0, [1.0, 1.0, 1.0, 0.10]);
        }
        list.circle(knob_center, knob_radius, s.color(StyleKey::Text));
        list.circle_outline(
            knob_center,
            knob_radius,
            s.scalar(StyleKey::BorderWidth).max(1.0),
            s.color(StyleKey::ButtonBorder),
        );

        // Value text
        if self.show_value {
            let display = if self.step.is_some_and(|s| s >= 1.0) {
                format!("{}", new_value as i32)
            } else {
                format!("{:.1}", new_value)
            };
            let text_color = s.color(StyleKey::Text);
            let font_size = s.scalar(StyleKey::FontSize) * 0.8;
            let text_x = rect.x + track_width + 6.0;
            let text_y = list.vcentered_text_y(
                rect.y,
                rect.height,
                font_size,
                theme.font.as_ref(),
                &display,
            );
            let block = TextBlock::new(&display, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_font_opt(theme.font.clone());
            list.text(block);
        }

        if focused {
            ctx.draw_focus_ring(rect);
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
    use crate::{DrawList, FocusState, InputState, Theme};

    fn theme() -> Theme {
        Theme::default()
    }

    /// Draw a slider into a throwaway `DrawContext` and return its output. The
    /// slider draws no focusable, and no slider test inspects the draw list, so
    /// a fresh list/focus per call keeps the call sites terse.
    fn draw_slider(
        slider: &Slider,
        value: f32,
        id: DragId,
        cap: &mut DragCapture,
        rect: Rect,
        input: &InputState,
    ) -> SliderOutput {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = theme();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        slider.draw(value, id, cap, rect, &mut ctx)
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
        // Press near the right of the track (interior; the right edge is
        // exclusive) -> value above the starting 50.
        let input = press_at(85.0, 10.0);
        let out = draw_slider(&slider, 50.0, 0, &mut cap, rect(), &input);
        assert!(out.dragging, "click inside the track should begin a drag");
        assert!(cap.is_active(0));
        assert!(
            out.value > 50.0,
            "dragging to the right edge raises the value"
        );
    }

    #[test]
    fn release_ends_drag() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();

        let down = press_at(50.0, 10.0);
        draw_slider(&slider, 50.0, 0, &mut cap, rect(), &down);
        assert!(cap.is_active(0));

        let up = release_at(50.0, 10.0);
        let out = draw_slider(&slider, 50.0, 0, &mut cap, rect(), &up);
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
        let r = rect();

        // Frame 1: press. A is drawn first and claims the drag.
        let down = press_at(20.0, 10.0);
        let a1 = draw_slider(&slider, 10.0, 0, &mut cap, r, &down);
        let b1 = draw_slider(&slider, 90.0, 1, &mut cap, r, &down);
        assert!(a1.dragging, "first slider claims the drag");
        assert!(
            !b1.dragging,
            "second slider must not also grab the same press"
        );
        assert!(cap.is_active(0));

        // Frame 2: mouse moves while held. Only A tracks it.
        let mov = hold_at(80.0, 10.0);
        let a2 = draw_slider(&slider, a1.value, 0, &mut cap, r, &mov);
        let b2 = draw_slider(&slider, 90.0, 1, &mut cap, r, &mov);
        assert!(a2.dragging);
        assert!(!b2.dragging);
        assert!(!b2.changed, "non-owning slider's value is untouched");
    }

    #[test]
    fn drag_continues_across_frames_until_release() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let r = rect();

        let down = press_at(50.0, 10.0);
        let mut value = draw_slider(&slider, 50.0, 0, &mut cap, r, &down).value;

        // Mouse leaves the track rect vertically but stays held: owner keeps
        // tracking because capture, not hit-testing, gates an in-progress drag.
        let mov = hold_at(95.0, 500.0);
        let out = draw_slider(&slider, value, 0, &mut cap, r, &mov);
        assert!(
            out.dragging,
            "held drag continues even when cursor leaves the rect"
        );
        value = out.value;
        assert!(value > 50.0);

        let up = release_at(95.0, 500.0);
        let out = draw_slider(&slider, value, 0, &mut cap, r, &up);
        assert!(!out.dragging);
        assert!(cap.is_free());
    }

    #[test]
    fn consumed_mouse_does_not_start_drag() {
        let slider = Slider::new(0.0, 100.0);
        let mut cap = DragCapture::new();
        let mut input = press_at(50.0, 10.0);
        input.mouse_consumed = true; // a higher layer already took this click
        let out = draw_slider(&slider, 50.0, 0, &mut cap, rect(), &input);
        assert!(!out.dragging);
        assert!(cap.is_free());
    }

    // ---- Keyboard focus / arrow adjustment ----

    /// Draw a slider with an explicitly seeded focus state.
    fn draw_slider_focused(
        slider: &Slider,
        value: f32,
        id: DragId,
        cap: &mut DragCapture,
        focus: &mut FocusState,
        input: &InputState,
    ) -> SliderOutput {
        let mut list = DrawList::new();
        let theme = theme();
        let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
        slider.draw(value, id, cap, rect(), &mut ctx)
    }

    fn arrow(left: bool, right: bool, up: bool, down: bool) -> InputState {
        let mut s = InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            key_left: left,
            key_right: right,
            key_up: up,
            key_down: down,
            ..InputState::default()
        };
        crate::map_keyboard(&mut s); // arrows → nav directional
        s
    }

    #[test]
    fn arrows_adjust_only_when_focused() {
        let slider = Slider::new(0.0, 100.0).focusable(42);
        // Unfocused: Right is ignored.
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        let out = draw_slider_focused(
            &slider,
            50.0,
            0,
            &mut cap,
            &mut focus,
            &arrow(false, true, false, false),
        );
        assert!(!out.changed, "arrows must not move an unfocused slider");
        assert_eq!(out.value, 50.0);
        // Focused: Right increments by 1/20 of the range (5.0).
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        focus.focus(42);
        let out = draw_slider_focused(
            &slider,
            50.0,
            0,
            &mut cap,
            &mut focus,
            &arrow(false, true, false, false),
        );
        assert!(out.changed);
        assert!(
            (out.value - 55.0).abs() < 1e-3,
            "Right increments by range/20"
        );
    }

    #[test]
    fn left_decrements_when_focused() {
        let slider = Slider::new(0.0, 100.0).focusable(42);
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        focus.focus(42);
        let out = draw_slider_focused(
            &slider,
            50.0,
            0,
            &mut cap,
            &mut focus,
            &arrow(true, false, false, false),
        );
        assert!(
            (out.value - 45.0).abs() < 1e-3,
            "Left decrements by range/20"
        );
    }

    #[test]
    fn arrows_clamp_to_range() {
        let slider = Slider::new(0.0, 100.0).focusable(42);
        // Down at the floor stays at min.
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        focus.focus(42);
        let out = draw_slider_focused(
            &slider,
            0.0,
            0,
            &mut cap,
            &mut focus,
            &arrow(false, false, false, true),
        );
        assert_eq!(out.value, 0.0, "Down clamps at min");
        // Up at the ceiling stays at max.
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        focus.focus(42);
        let out = draw_slider_focused(
            &slider,
            100.0,
            0,
            &mut cap,
            &mut focus,
            &arrow(false, false, true, false),
        );
        assert_eq!(out.value, 100.0, "Up clamps at max");
    }

    #[test]
    fn arrow_step_uses_explicit_step() {
        let slider = Slider::new(0.0, 10.0).with_step(1.0).focusable(42);
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        focus.focus(42);
        let out = draw_slider_focused(
            &slider,
            5.0,
            0,
            &mut cap,
            &mut focus,
            &arrow(false, true, false, false),
        );
        assert_eq!(out.value, 6.0, "explicit step drives keyboard increments");
    }

    #[test]
    fn focus_ring_drawn_only_when_focused() {
        let slider = Slider::new(0.0, 100.0).focusable(42);
        let idle = InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..InputState::default()
        };
        // Unfocused.
        let mut cap = DragCapture::new();
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = theme();
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &idle, 800.0, 600.0);
            slider.draw(50.0, 0, &mut cap, rect(), &mut ctx);
        }
        let unfocused_chrome = list.chrome_instances.len();
        // Focused.
        let mut focus = FocusState::new();
        focus.focus(42);
        let mut list = DrawList::new();
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &idle, 800.0, 600.0);
            slider.draw(50.0, 0, &mut cap, rect(), &mut ctx);
        }
        assert!(
            list.chrome_instances.len() > unfocused_chrome,
            "focus ring adds outline geometry"
        );
    }
}
