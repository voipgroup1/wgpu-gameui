//! Hit-zone / interactive sensor — a region that reports pointer interaction
//! **without drawing anything** (Teardown's `UiMakeInteractive`).
//!
//! Every other widget in this crate draws into a [`DrawList`](super::DrawList);
//! `HitZone` is the deliberate exception. It takes a [`Rect`] and an
//! [`InputState`] and answers "is the pointer over this, and what did it do?" —
//! nothing is appended to any draw list. That makes it the right tool for
//! **sensors over things this UI didn't draw**: a clickable region over a
//! 3D-rendered object, a custom-painted canvas, a world-space label, etc. The
//! caller draws (or renders in 3D) however it likes, then lays a `HitZone` over
//! the same screen rect to pick up hover/click/scroll.
//!
//! Because it never touches the draw list it takes a plain `&InputState` rather
//! than a [`DrawContext`](super::DrawContext) — there is nothing for a context
//! to carry. It still honors [`InputState::mouse_consumed`] so a zone beneath a
//! modal/popup stays inert, exactly like the drawing widgets.
//!
//! It does **not** itself set `mouse_consumed`: like [`Button`](super::Button)
//! and the rest of the per-layer widgets, it only *reports*. When you use a zone
//! to guard world-picking ("UI wins over the 3D scene"), gate your picking on
//! `!out.hovered` — the zone tells you whether the pointer is over UI.
//!
//! # Example
//! ```ignore
//! // A clickable sensor over a 3D viewport region the engine rendered itself:
//! let out = HitZone::new().test(viewport_rect, &input);
//! if out.clicked      { select_under_cursor(out.local_pos.unwrap()); }
//! if out.right_clicked { open_context_menu(); }
//! if out.hovered      { skip_world_picking_this_frame(); }
//! ```

use crate::layout::Rect;
use crate::InputState;

/// Result of probing a [`HitZone`] for one frame.
///
/// All event fields are `false` (and [`scroll_delta`](Self::scroll_delta) `0.0`,
/// [`local_pos`](Self::local_pos) `None`) unless the pointer is over the zone and
/// not [consumed](InputState::mouse_consumed) by a higher layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitZoneOutput {
    /// Pointer is inside the zone (and not consumed by a higher layer / disabled).
    pub hovered: bool,
    /// Primary button is currently held while the pointer is over the zone.
    pub pressed: bool,
    /// Primary button was clicked on the zone this frame.
    pub clicked: bool,
    /// Primary button was released over the zone this frame.
    pub released: bool,
    /// Right button was clicked on the zone this frame (context menus).
    pub right_clicked: bool,
    /// Middle button was clicked on the zone this frame.
    pub middle_clicked: bool,
    /// Primary button was double-clicked on the zone this frame. (`clicked` is
    /// also true on that frame — the double-click press is still a click.)
    pub double_clicked: bool,
    /// Primary button has been held past the click-and-hold threshold while over
    /// the zone (see [`InputState::mouse_held`]).
    pub held: bool,
    /// Scroll-wheel delta while hovered (`0.0` otherwise). Positive = up.
    pub scroll_delta: f32,
    /// Pointer position relative to the zone's top-left corner, when hovered.
    /// `None` when not hovered. Handy for picking *within* the sensor (e.g. where
    /// in a 3D viewport the click landed).
    pub local_pos: Option<[f32; 2]>,
}

impl HitZoneOutput {
    /// A "nothing happened" result.
    pub fn idle() -> Self {
        Self {
            hovered: false,
            pressed: false,
            clicked: false,
            released: false,
            right_clicked: false,
            middle_clicked: false,
            double_clicked: false,
            held: false,
            scroll_delta: 0.0,
            local_pos: None,
        }
    }

    /// True if any mouse button (left, right, or middle) was clicked on the zone
    /// this frame.
    pub fn any_click(&self) -> bool {
        self.clicked || self.right_clicked || self.middle_clicked
    }
}

/// A draw-free interactive region. See the [module docs](self).
///
/// Construct, optionally [disable](Self::enabled), then [`test`](Self::test) a
/// screen-space [`Rect`] against the frame's [`InputState`]. Cheap and stateless
/// — make one per probe (or reuse; it carries only an `enabled` flag).
#[derive(Debug, Clone, Copy)]
pub struct HitZone {
    enabled: bool,
}

impl Default for HitZone {
    fn default() -> Self {
        Self::new()
    }
}

impl HitZone {
    /// A new, enabled hit zone.
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// Enable/disable the zone. A disabled zone always reports
    /// [`HitZoneOutput::idle`] (no hover, no events), like a disabled button.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Probe `rect` (screen-space) against `input` and report this frame's
    /// interaction. Draws nothing. Honors [`InputState::mouse_consumed`].
    pub fn test(&self, rect: Rect, input: &InputState) -> HitZoneOutput {
        // Degenerate rects and disabled/consumed pointers are inert.
        if !self.enabled
            || rect.width <= 0.0
            || rect.height <= 0.0
            || input.mouse_consumed
            || !rect.contains(input.mouse_x, input.mouse_y)
        {
            return HitZoneOutput::idle();
        }

        HitZoneOutput {
            hovered: true,
            pressed: input.mouse_down,
            clicked: input.mouse_clicked,
            released: input.mouse_released,
            right_clicked: input.mouse_right_clicked,
            middle_clicked: input.mouse_middle_clicked,
            double_clicked: input.mouse_double_clicked,
            held: input.mouse_held,
            // Only surface the wheel if it wasn't already claimed by a scroll
            // container this frame.
            scroll_delta: if input.scroll_consumed {
                0.0
            } else {
                input.scroll_delta
            },
            local_pos: Some([input.mouse_x - rect.x, input.mouse_y - rect.y]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(10.0, 10.0, 100.0, 40.0)
    }

    /// Pointer at the centre of `rect()` with the given primary-button edges.
    fn at_center(down: bool, clicked: bool) -> InputState {
        InputState {
            mouse_x: 60.0,
            mouse_y: 30.0,
            mouse_down: down,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    #[test]
    fn click_inside_reports_clicked_and_hovered() {
        let out = HitZone::new().test(rect(), &at_center(true, true));
        assert!(out.hovered);
        assert!(out.clicked);
        assert!(out.pressed);
        assert!(out.any_click());
    }

    #[test]
    fn pointer_outside_is_idle() {
        let input = InputState {
            mouse_x: 500.0,
            mouse_y: 500.0,
            mouse_clicked: true,
            ..Default::default()
        };
        assert_eq!(HitZone::new().test(rect(), &input), HitZoneOutput::idle());
    }

    #[test]
    fn consumed_pointer_is_idle() {
        let input = InputState {
            mouse_consumed: true,
            ..at_center(true, true)
        };
        assert_eq!(HitZone::new().test(rect(), &input), HitZoneOutput::idle());
    }

    #[test]
    fn disabled_zone_is_idle() {
        let out = HitZone::new().enabled(false).test(rect(), &at_center(true, true));
        assert_eq!(out, HitZoneOutput::idle());
    }

    #[test]
    fn zero_size_rect_is_idle() {
        let out = HitZone::new().test(Rect::new(10.0, 10.0, 0.0, 40.0), &at_center(true, true));
        assert_eq!(out, HitZoneOutput::idle());
    }

    #[test]
    fn reports_right_and_middle_and_double_clicks() {
        let input = InputState {
            mouse_right_clicked: true,
            mouse_middle_clicked: true,
            mouse_double_clicked: true,
            mouse_clicked: true,
            ..at_center(false, false)
        };
        let out = HitZone::new().test(rect(), &input);
        assert!(out.right_clicked);
        assert!(out.middle_clicked);
        assert!(out.double_clicked);
        assert!(out.any_click());
    }

    #[test]
    fn right_click_outside_is_idle() {
        let input = InputState {
            mouse_x: 500.0,
            mouse_y: 500.0,
            mouse_right_clicked: true,
            ..Default::default()
        };
        assert!(!HitZone::new().test(rect(), &input).right_clicked);
    }

    #[test]
    fn released_reported_over_zone() {
        let input = InputState {
            mouse_released: true,
            ..at_center(false, false)
        };
        assert!(HitZone::new().test(rect(), &input).released);
    }

    #[test]
    fn scroll_only_when_hovered_and_unclaimed() {
        // Hovered + unclaimed wheel → reported.
        let hovered = InputState {
            scroll_delta: 3.0,
            ..at_center(false, false)
        };
        assert_eq!(HitZone::new().test(rect(), &hovered).scroll_delta, 3.0);

        // Already-claimed wheel → suppressed even while hovered.
        let claimed = InputState {
            scroll_delta: 3.0,
            scroll_consumed: true,
            ..at_center(false, false)
        };
        assert_eq!(HitZone::new().test(rect(), &claimed).scroll_delta, 0.0);

        // Not hovered → no wheel.
        let elsewhere = InputState {
            mouse_x: 500.0,
            mouse_y: 500.0,
            scroll_delta: 3.0,
            ..Default::default()
        };
        assert_eq!(HitZone::new().test(rect(), &elsewhere).scroll_delta, 0.0);
    }

    #[test]
    fn local_pos_is_relative_to_zone_origin() {
        let out = HitZone::new().test(rect(), &at_center(false, false));
        // Centre of a (10,10,100,40) rect is (60,30) → local (50,20).
        assert_eq!(out.local_pos, Some([50.0, 20.0]));
    }

    #[test]
    fn held_reported_over_zone() {
        let input = InputState {
            mouse_held: true,
            mouse_down: true,
            ..at_center(true, false)
        };
        assert!(HitZone::new().test(rect(), &input).held);
    }

    #[test]
    fn edge_is_exclusive() {
        // Rect::contains is edge-exclusive: the far corner is outside.
        let input = InputState {
            mouse_x: 110.0,
            mouse_y: 50.0,
            ..Default::default()
        };
        assert!(!HitZone::new().test(rect(), &input).hovered);
    }
}
