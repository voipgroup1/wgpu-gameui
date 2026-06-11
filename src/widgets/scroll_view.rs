//! General-purpose scrollable viewport widget.
//!
//! `ScrollView` clips its content to a fixed viewport `Rect`, applies a
//! caller-owned `ScrollState` offset to the content via the existing transform
//! stack, and draws minimal scrollbars (vertical + horizontal) when the
//! content overflows. Scrollbar thumbs can be dragged; the wheel is consumed
//! when the cursor is over the viewport.
//!
//! State is **caller-owned** so the widget remains a transient struct that
//! can be re-built every frame, matching the rest of this crate's
//! immediate-mode style.
//!
//! ```ignore
//! let mut scroll = ScrollState::default();
//!
//! ScrollView::new(viewport_rect, [200.0, 800.0])
//!     .draw(&mut scroll, list, theme, input, |list, content_origin| {
//!         // Draw your content here. The transform stack has already been
//!         // translated by `-offset`, so draw in content-local coordinates
//!         // anchored at `content_origin` (which equals the viewport's top-left
//!         // in world space).
//!     });
//! ```
//!
//! `content_origin` is the viewport's world-space top-left after the active
//! transform; it's what `(0,0)` inside the closure now maps to. Most callers
//! draw at world-space rects derived from `viewport.x + col, viewport.y + row`.

use crate::layout::Rect;
use crate::{InputState, Theme};

use super::DrawList;

/// Caller-owned scroll state.
///
/// `offset` is the scroll offset (positive = scrolled right/down). `content_size`
/// is what the most recent draw reported as the natural content extent — used
/// for clamping and scrollbar sizing on subsequent frames.
///
/// `_drag_*` fields track the currently-dragged scrollbar thumb.
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    pub offset: [f32; 2],
    pub content_size: [f32; 2],
    /// Which scrollbar is being dragged this frame (None when not dragging).
    drag_axis: Option<ScrollAxis>,
    /// Mouse position at drag start (world coords).
    drag_start_mouse: f32,
    /// Scroll offset at drag start (along drag_axis).
    drag_start_offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}

impl ScrollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when content overflows in the given axis (0 = X, 1 = Y).
    pub fn overflows(&self, axis: usize, viewport_size: f32) -> bool {
        self.content_size[axis] > viewport_size + 0.5
    }

    /// Maximum legal offset along axis (>= 0).
    pub fn max_offset(&self, axis: usize, viewport_size: f32) -> f32 {
        (self.content_size[axis] - viewport_size).max(0.0)
    }

    /// Clamp the current offset against the latest content/viewport sizes.
    pub fn clamp(&mut self, viewport: [f32; 2]) {
        self.offset[0] = self.offset[0].clamp(0.0, self.max_offset(0, viewport[0]));
        self.offset[1] = self.offset[1].clamp(0.0, self.max_offset(1, viewport[1]));
    }

    /// Reset offset to (0, 0).
    pub fn reset(&mut self) {
        self.offset = [0.0, 0.0];
    }
}

/// Configuration for a single ScrollView call.
pub struct ScrollView {
    viewport: Rect,
    /// Width of the scrollbar (track + thumb), in pixels.
    bar_thickness: f32,
    /// Minimum thumb extent so a tiny content/viewport ratio still produces a
    /// grabbable target.
    min_thumb: f32,
    /// Pixels of scroll per wheel notch.
    wheel_speed: f32,
    /// If false, vertical scrolling is disabled (content is clipped vertically
    /// but the offset.y is forced to 0). Same for horizontal.
    enable_vertical: bool,
    enable_horizontal: bool,
}

/// Geometry returned by [`ScrollView::begin`] and handed back to
/// [`ScrollView::end`]. Holds the inner viewport rect plus which scrollbars are
/// visible and the inner extents (so `end` can place the bars without
/// recomputing visibility).
#[derive(Debug, Clone, Copy)]
pub struct ScrollBegin {
    /// The scrollable region in world space (viewport minus any visible bars).
    pub inner: Rect,
    v_visible: bool,
    h_visible: bool,
    inner_w: f32,
    inner_h: f32,
}

impl ScrollView {
    pub fn new(viewport: Rect) -> Self {
        Self {
            viewport,
            bar_thickness: 6.0,
            min_thumb: 16.0,
            wheel_speed: 20.0,
            enable_vertical: true,
            enable_horizontal: true,
        }
    }

    pub fn with_bar_thickness(mut self, t: f32) -> Self {
        self.bar_thickness = t;
        self
    }

    pub fn with_wheel_speed(mut self, s: f32) -> Self {
        self.wheel_speed = s;
        self
    }

    pub fn vertical_only(mut self) -> Self {
        self.enable_horizontal = false;
        self
    }

    pub fn horizontal_only(mut self) -> Self {
        self.enable_vertical = false;
        self
    }

    /// Draw the ScrollView and run `content` to populate it.
    ///
    /// The closure is called with the `DrawList` already translated by the
    /// negative scroll offset and clipped to the viewport. The closure receives
    /// the world-space `Rect` representing the *scrollable region* in
    /// content-local coordinates — i.e. its `x`/`y` are `viewport.x/y` and its
    /// `width`/`height` are the viewport size; widgets inside should draw at
    /// rects starting at `(viewport.x, viewport.y)` and any content beyond
    /// the viewport extents is clipped/scrolled automatically.
    ///
    /// `state.content_size` must be set by the caller *before* calling `draw`
    /// (the ScrollView cannot know how tall arbitrary content is until it has
    /// been measured). A common pattern is to compute it once based on item
    /// counts, then pass it in.
    ///
    /// This is a thin wrapper over [`begin`](Self::begin) + [`end`](Self::end)
    /// for callers that draw their content in a Rust closure. Immediate-mode
    /// callers (e.g. a scripting binding) can use `begin`/`end` directly.
    pub fn draw<F>(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        theme: &Theme,
        input: &mut InputState,
        mut content: F,
    ) where
        F: FnMut(&mut DrawList, Rect),
    {
        let begun = self.begin(state, list, theme, input);
        content(list, begun.inner);
        self.end(state, list, theme, input, begun);
    }

    /// Begin a scroll region: handle wheel + thumb-drag input, push the clip and
    /// the `-offset` transform, and return the viewport geometry. The caller
    /// then draws content (in world-space pre-offset coords anchored at
    /// `ScrollBegin::inner`) and **must** call [`end`](Self::end) with the
    /// returned value to pop the clip/transform and draw the scrollbars.
    ///
    /// `state.content_size` must be set before calling (see [`draw`](Self::draw)).
    pub fn begin(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        _theme: &Theme,
        input: &mut InputState,
    ) -> ScrollBegin {
        // Force-disable axes where content fits.
        if !self.enable_vertical {
            state.offset[1] = 0.0;
        }
        if !self.enable_horizontal {
            state.offset[0] = 0.0;
        }

        // Reserve space for visible scrollbars so content doesn't slide under them.
        let v_visible = self.enable_vertical && state.overflows(1, self.viewport.height);
        let h_visible = self.enable_horizontal && state.overflows(0, self.viewport.width);
        let inner_w = self.viewport.width - if v_visible { self.bar_thickness } else { 0.0 };
        let inner_h = self.viewport.height - if h_visible { self.bar_thickness } else { 0.0 };
        let inner = Rect::new(self.viewport.x, self.viewport.y, inner_w, inner_h);

        // Re-clamp against inner viewport (now that we know which bars take space).
        state.clamp([inner_w, inner_h]);

        let mouse_over_inner = inner.contains(input.mouse_x, input.mouse_y)
            && !input.mouse_consumed;

        // Wheel input — consumed when over the inner viewport, regardless of
        // whether the offset actually changed (e.g. at a clamp boundary the
        // wheel is still claimed so it doesn't bubble to an outer scrollable).
        if mouse_over_inner
            && input.scroll_delta != 0.0
            && !input.scroll_consumed
            && self.enable_vertical
        {
            state.offset[1] = (state.offset[1] - input.scroll_delta * self.wheel_speed)
                .clamp(0.0, state.max_offset(1, inner_h));
            input.scroll_consumed = true;
            input.scroll_delta = 0.0;
        }

        // Handle thumb drag for both axes.
        if let Some(axis) = state.drag_axis {
            if !input.mouse_down {
                state.drag_axis = None;
            } else {
                match axis {
                    ScrollAxis::Vertical => {
                        let track_h = inner_h;
                        let thumb_h = thumb_extent(track_h, state.content_size[1], self.min_thumb);
                        let drag_range = (track_h - thumb_h).max(1.0);
                        let max_off = state.max_offset(1, inner_h);
                        let dy = input.mouse_y - state.drag_start_mouse;
                        state.offset[1] = (state.drag_start_offset + dy * (max_off / drag_range))
                            .clamp(0.0, max_off);
                    }
                    ScrollAxis::Horizontal => {
                        let track_w = inner_w;
                        let thumb_w = thumb_extent(track_w, state.content_size[0], self.min_thumb);
                        let drag_range = (track_w - thumb_w).max(1.0);
                        let max_off = state.max_offset(0, inner_w);
                        let dx = input.mouse_x - state.drag_start_mouse;
                        state.offset[0] = (state.drag_start_offset + dx * (max_off / drag_range))
                            .clamp(0.0, max_off);
                    }
                }
            }
        }

        // Set up clip + transform for the content the caller is about to draw.
        list.push_clip(inner);
        list.push_transform();
        list.translate(-state.offset[0], -state.offset[1]);

        ScrollBegin {
            inner,
            v_visible,
            h_visible,
            inner_w,
            inner_h,
        }
    }

    /// Finish a scroll region opened by [`begin`](Self::begin): pop the
    /// transform + clip and draw the scrollbars.
    pub fn end(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
        begun: ScrollBegin,
    ) {
        list.pop_transform();
        list.pop_clip();

        // Draw scrollbars.
        if begun.v_visible {
            self.draw_v_bar(state, list, theme, input, begun.inner_h);
        }
        if begun.h_visible {
            self.draw_h_bar(state, list, theme, input, begun.inner_w);
        }

        // Fill the bottom-right corner gap when both scrollbars are visible
        // so the content underneath doesn't show through.
        if begun.v_visible && begun.h_visible {
            let corner = Rect::new(
                self.viewport.x + self.viewport.width - self.bar_thickness,
                self.viewport.y + self.viewport.height - self.bar_thickness,
                self.bar_thickness,
                self.bar_thickness,
            );
            list.quad(
                corner.x,
                corner.y,
                corner.width,
                corner.height,
                theme.input_background,
            );
        }
    }

    fn draw_v_bar(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
        inner_h: f32,
    ) {
        let track_x = self.viewport.x + self.viewport.width - self.bar_thickness;
        let track_y = self.viewport.y;
        let track_h = inner_h;
        let track = Rect::new(track_x, track_y, self.bar_thickness, track_h);
        let radius = self.bar_thickness * 0.5;

        // Track
        list.rounded_rect(track, radius, theme.input_background);

        let thumb_h = thumb_extent(track_h, state.content_size[1], self.min_thumb);
        let max_off = state.max_offset(1, inner_h).max(1e-6);
        let t = (state.offset[1] / max_off).clamp(0.0, 1.0);
        let thumb_y = track_y + (track_h - thumb_h) * t;
        let thumb = Rect::new(track_x, thumb_y, self.bar_thickness, thumb_h);

        let hovered = thumb.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let active = state.drag_axis == Some(ScrollAxis::Vertical);
        let color = if active {
            theme.accent
        } else if hovered {
            theme.button_hover
        } else {
            theme.button_border
        };
        list.rounded_rect(thumb, radius, color);

        if hovered && input.mouse_clicked && state.drag_axis.is_none() {
            state.drag_axis = Some(ScrollAxis::Vertical);
            state.drag_start_mouse = input.mouse_y;
            state.drag_start_offset = state.offset[1];
        }
    }

    fn draw_h_bar(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
        inner_w: f32,
    ) {
        let track_x = self.viewport.x;
        let track_y = self.viewport.y + self.viewport.height - self.bar_thickness;
        let track_w = inner_w;
        let track = Rect::new(track_x, track_y, track_w, self.bar_thickness);
        let radius = self.bar_thickness * 0.5;

        list.rounded_rect(track, radius, theme.input_background);

        let thumb_w = thumb_extent(track_w, state.content_size[0], self.min_thumb);
        let max_off = state.max_offset(0, inner_w).max(1e-6);
        let t = (state.offset[0] / max_off).clamp(0.0, 1.0);
        let thumb_x = track_x + (track_w - thumb_w) * t;
        let thumb = Rect::new(thumb_x, track_y, thumb_w, self.bar_thickness);

        let hovered = thumb.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let active = state.drag_axis == Some(ScrollAxis::Horizontal);
        let color = if active {
            theme.accent
        } else if hovered {
            theme.button_hover
        } else {
            theme.button_border
        };
        list.rounded_rect(thumb, radius, color);

        if hovered && input.mouse_clicked && state.drag_axis.is_none() {
            state.drag_axis = Some(ScrollAxis::Horizontal);
            state.drag_start_mouse = input.mouse_x;
            state.drag_start_offset = state.offset[0];
        }
    }
}

fn thumb_extent(track: f32, content: f32, min_thumb: f32) -> f32 {
    if content <= 0.0 {
        return track;
    }
    let ratio = (track / content).clamp(0.0, 1.0);
    (track * ratio).max(min_thumb).min(track)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::default()
    }

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    #[test]
    fn offset_is_clamped_to_max() {
        let mut s = ScrollState {
            offset: [9999.0, -5.0],
            content_size: [100.0, 200.0],
            ..ScrollState::default()
        };
        s.clamp([50.0, 80.0]);
        // X: clamp(9999, 0, 100-50=50) = 50
        // Y: clamp(-5, 0, 200-80=120) = 0
        assert_eq!(s.offset, [50.0, 0.0]);
    }

    #[test]
    fn no_overflow_when_content_fits() {
        let s = ScrollState {
            offset: [0.0, 0.0],
            content_size: [50.0, 80.0],
            ..ScrollState::default()
        };
        assert!(!s.overflows(0, 100.0));
        assert!(!s.overflows(1, 100.0));
    }

    #[test]
    fn overflow_when_content_exceeds_viewport() {
        let s = ScrollState {
            content_size: [100.0, 800.0],
            ..ScrollState::default()
        };
        assert!(s.overflows(1, 200.0));
        assert!(!s.overflows(0, 200.0));
    }

    #[test]
    fn max_offset_zero_when_content_fits() {
        let s = ScrollState {
            content_size: [50.0, 80.0],
            ..ScrollState::default()
        };
        assert_eq!(s.max_offset(0, 100.0), 0.0);
        assert_eq!(s.max_offset(1, 100.0), 0.0);
    }

    #[test]
    fn wheel_input_updates_vertical_offset() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0; // wheel down

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_l, _r| {},
        );
        assert!(state.offset[1] > 0.0, "wheel down should scroll content");
    }

    #[test]
    fn wheel_does_not_scroll_outside_viewport() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(500.0, 500.0); // outside
        input.scroll_delta = -3.0;

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert_eq!(state.offset[1], 0.0);
    }

    #[test]
    fn scrollbar_hidden_when_content_smaller_than_viewport() {
        let mut state = ScrollState {
            content_size: [50.0, 50.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(0.0, 0.0);

        // Track approximate vertex count: a hidden bar means no extra rounded
        // rect geometry beyond what the (empty) content closure adds.
        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert!(list.vertices.is_empty());
    }

    #[test]
    fn thumb_extent_proportional_to_visible_fraction() {
        // 200px viewport over 1000px content -> 20% -> 40px thumb (above min).
        assert!((thumb_extent(200.0, 1000.0, 16.0) - 40.0).abs() < 1e-3);
        // Tiny content fraction clamped to min_thumb.
        assert!((thumb_extent(200.0, 100000.0, 16.0) - 16.0).abs() < 1e-3);
        // Content fits — thumb spans entire track.
        assert!((thumb_extent(200.0, 0.0, 16.0) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn thumb_drag_updates_offset() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let viewport = Rect::new(0.0, 0.0, 200.0, 200.0);

        // Bar thickness default 6, so vertical track lives at x=194..200.
        // Initial thumb sits at top: y=0..40 (200/1000 = 20% of 200 = 40).
        // Click on the thumb to start a drag.
        let mut input = InputState {
            mouse_x: 197.0,
            mouse_y: 10.0,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        };
        ScrollView::new(viewport).draw(&mut state, &mut list, &theme, &mut input, |_, _| {});
        assert!(state.drag_axis.is_some());

        // Now drag down by 80 pixels with mouse held.
        list.clear();
        input.mouse_clicked = false;
        input.mouse_y = 90.0;
        ScrollView::new(viewport).draw(&mut state, &mut list, &theme, &mut input, |_, _| {});

        // Thumb travel = 200 - 40 = 160px.  Content travel = 1000 - 200 = 800px.
        // 80px of mouse drag -> 80 * (800/160) = 400px content offset.
        assert!(
            (state.offset[1] - 400.0).abs() < 1.0,
            "offset {} expected ~400",
            state.offset[1]
        );
    }

    #[test]
    fn drag_releases_on_mouse_up() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            drag_axis: Some(ScrollAxis::Vertical),
            drag_start_mouse: 0.0,
            drag_start_offset: 0.0,
            offset: [0.0, 100.0],
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = InputState {
            mouse_down: false,
            ..InputState::default()
        };
        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert!(state.drag_axis.is_none());
    }

    #[test]
    fn consumed_input_blocks_wheel() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        input.mouse_consumed = true;

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert_eq!(state.offset[1], 0.0);
    }

    #[test]
    fn wheel_marks_scroll_consumed_when_applied() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert!(input.scroll_consumed);
        assert_eq!(input.scroll_delta, 0.0);
    }

    #[test]
    fn outer_scroll_skipped_when_inner_consumes() {
        // Two ScrollViews. Cursor sits over both. Inner runs first and absorbs
        // the wheel; outer should see scroll_delta = 0 / scroll_consumed = true
        // and not move.
        let mut inner_state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut outer_state = ScrollState {
            content_size: [400.0, 4000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;

        // Inner viewport is fully inside outer.
        ScrollView::new(Rect::new(0.0, 0.0, 100.0, 100.0)).draw(
            &mut inner_state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert!(inner_state.offset[1] > 0.0, "inner should have scrolled");

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut outer_state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert_eq!(
            outer_state.offset[1], 0.0,
            "outer must not steal scroll when inner consumed it"
        );
    }

    #[test]
    fn outer_scrolls_when_inner_not_under_cursor() {
        // Inner is in a different region than the cursor — its draw shouldn't
        // claim the wheel, leaving the outer free to scroll.
        let mut inner_state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut outer_state = ScrollState {
            content_size: [400.0, 4000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(150.0, 150.0);
        input.scroll_delta = -3.0;

        // Inner viewport at (0,0..50,50) — cursor (150,150) is outside it.
        ScrollView::new(Rect::new(0.0, 0.0, 50.0, 50.0)).draw(
            &mut inner_state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert_eq!(inner_state.offset[1], 0.0);
        assert!(!input.scroll_consumed);

        ScrollView::new(Rect::new(100.0, 100.0, 200.0, 200.0)).draw(
            &mut outer_state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );
        assert!(outer_state.offset[1] > 0.0, "outer should have scrolled");
    }

    #[test]
    fn corner_quad_drawn_when_both_axes_visible() {
        let mut state = ScrollState {
            content_size: [800.0, 800.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(-10.0, -10.0);

        let viewport = Rect::new(0.0, 0.0, 100.0, 100.0);
        ScrollView::new(viewport).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |_, _| {},
        );

        // The corner quad should sit at (viewport.x + width - bar, y + height - bar)
        // = (100 - 6, 100 - 6) = (94, 94). `quad` now records a chrome instance
        // whose rect top-left is exactly that position.
        let bar = 6.0_f32;
        let cx = viewport.x + viewport.width - bar;
        let cy = viewport.y + viewport.height - bar;
        let found = list
            .chrome_instances
            .iter()
            .any(|i| (i.rect[0] - cx).abs() < 1e-3 && (i.rect[1] - cy).abs() < 1e-3);
        assert!(found, "expected a quad at corner ({}, {})", cx, cy);
    }

    #[test]
    fn content_translated_by_negative_offset() {
        let mut state = ScrollState {
            offset: [0.0, 50.0],
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(-10.0, -10.0);

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &theme,
            &mut input,
            |l, _vp| {
                // Quad at (0, 100) — should appear in world at (0, 50) due to
                // -50 vertical scroll. Translate-only, so it records a chrome
                // instance with the translated rect.
                l.quad(0.0, 100.0, 10.0, 10.0, [1.0; 4]);
            },
        );
        let found = list
            .chrome_instances
            .iter()
            .any(|i| i.rect == [0.0, 50.0, 10.0, 10.0]);
        assert!(found, "content quad should be translated to world (0, 50)");
    }
}
