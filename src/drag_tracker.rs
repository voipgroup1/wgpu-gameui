//! Pointer-drag detection: distinguishes a click from a drag and reports the
//! per-frame movement while a drag is held.
//!
//! This is **complementary to** [`DragCapture`](crate::DragCapture), not a
//! replacement. `DragCapture` answers *which* widget owns an in-progress drag
//! (so two overlapping draggables can't both follow one pointer);
//! `DragTracker` answers *whether the pointer is dragging at all* and *how far
//! it moved this frame*. A window-mover uses both: `DragTracker` for the delta,
//! `DragCapture` to claim ownership.
//!
//! ## Why a separate, caller-owned tracker
//!
//! Drag detection needs state that survives between frames — the press origin
//! and the click-vs-drag latch. [`InputState`] can't hold it, because some
//! consumers rebuild a fresh `InputState` every frame (the values would reset).
//! So the persistent state lives here, in a tracker the caller owns and threads
//! across frames, exactly like [`DragCapture`](crate::DragCapture),
//! [`ScrollState`](crate::ScrollState) and [`FocusState`](crate::FocusState).
//!
//! ## Usage
//!
//! Construct one per UI surface, then call [`DragTracker::update`] once per
//! frame **after** filling the mouse fields of your [`InputState`] and **before**
//! drawing widgets. It writes [`InputState::is_dragging`] and
//! [`InputState::drag_delta`], which widgets then read.
//!
//! ```
//! use wgpu_gameui::{DragTracker, InputState};
//!
//! let mut drag = DragTracker::new();
//! let mut input = InputState::default();
//!
//! // Frame 1: press at (10, 10) — a press is not yet a drag.
//! input.mouse_x = 10.0; input.mouse_y = 10.0;
//! input.mouse_down = true; input.mouse_clicked = true;
//! drag.update(&mut input);
//! assert!(!input.is_dragging);
//!
//! // Frame 2: held and moved well past the threshold — now it's a drag.
//! input.mouse_clicked = false;
//! input.mouse_x = 40.0;
//! drag.update(&mut input);
//! assert!(input.is_dragging);
//! assert_eq!(input.drag_delta, [30.0, 0.0]);
//! ```

use crate::InputState;

/// Default distance (logical px) the pointer must travel from the press origin
/// before a held click is promoted to a drag. Keeps a slightly shaky click from
/// registering as a drag.
pub const DEFAULT_DRAG_THRESHOLD: f32 = 4.0;

/// Tracks an in-progress pointer drag across frames and classifies a held click
/// as either a (still) click or a drag once it moves past a threshold.
///
/// Caller-owned; persists across frames. See the [module docs](self) for the
/// relationship to [`DragCapture`](crate::DragCapture).
#[derive(Debug, Clone)]
pub struct DragTracker {
    /// Where the current press started; `None` when no button is held.
    origin: Option<(f32, f32)>,
    /// Pointer position at the previous `update`, for the per-frame delta.
    prev: (f32, f32),
    /// Whether the current press has crossed the threshold. Latches `true`
    /// until the button is released, so a drag that momentarily stops moving
    /// stays a drag.
    active: bool,
    /// Threshold distance in logical px (kept for `threshold()`).
    threshold: f32,
    /// Squared threshold, compared against squared distance to avoid `sqrt`.
    threshold_sq: f32,
}

impl Default for DragTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DragTracker {
    /// A tracker with the [`DEFAULT_DRAG_THRESHOLD`].
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_DRAG_THRESHOLD)
    }

    /// A tracker whose press-to-drag threshold is `threshold` logical px.
    /// Negative values are clamped to `0.0` (every movement becomes a drag).
    pub fn with_threshold(threshold: f32) -> Self {
        let t = threshold.max(0.0);
        Self {
            origin: None,
            prev: (0.0, 0.0),
            active: false,
            threshold: t,
            threshold_sq: t * t,
        }
    }

    /// The press-to-drag threshold in logical px.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// True while the current gesture has been promoted to a drag.
    pub fn is_dragging(&self) -> bool {
        self.active
    }

    /// The press origin of the in-progress gesture (where the button went
    /// down), or `None` when no button is held. Subtract the current pointer
    /// position from this to get the total displacement since the drag started.
    pub fn origin(&self) -> Option<(f32, f32)> {
        self.origin
    }

    /// Abort any in-progress gesture (e.g. on focus loss / window blur / a
    /// cancel key). No drag is reported again until a fresh press.
    pub fn cancel(&mut self) {
        self.origin = None;
        self.active = false;
    }

    /// Advance the tracker by one frame and write the result into `input`.
    ///
    /// Reads `input.mouse_x`/`mouse_y`, `input.mouse_down`,
    /// `input.mouse_clicked` and `input.mouse_released`; sets
    /// [`InputState::is_dragging`] and [`InputState::drag_delta`]. `drag_delta`
    /// is the per-frame movement and stays `[0, 0]` until the gesture crosses
    /// the threshold, so sub-threshold jitter on a click can't nudge a widget.
    pub fn update(&mut self, input: &mut InputState) {
        let pos = (input.mouse_x, input.mouse_y);

        // A fresh press (re)seats the origin and clears the latch. Keying off
        // the press *edge* is what makes a click that never moves stay a click.
        if input.mouse_clicked {
            self.origin = Some(pos);
            self.prev = pos;
            self.active = false;
        }

        let mut delta = [0.0f32, 0.0];

        if input.mouse_down {
            if let Some(origin) = self.origin {
                // Per-frame movement since the previous update.
                delta = [pos.0 - self.prev.0, pos.1 - self.prev.1];
                self.prev = pos;

                if !self.active {
                    let dx = pos.0 - origin.0;
                    let dy = pos.1 - origin.1;
                    let dist_sq = dx * dx + dy * dy;
                    // A drag requires *movement*: the pointer must have actually
                    // left the origin and travelled at least the threshold. The
                    // `> 0` guard matters only at `threshold == 0`, where it
                    // means "any nonzero movement drags" rather than "a still
                    // press is instantly a drag".
                    if dist_sq > 0.0 && dist_sq >= self.threshold_sq {
                        self.active = true;
                    }
                }
            }
        }

        // The gesture ends on release, or any frame the button is simply up.
        if input.mouse_released || !input.mouse_down {
            self.origin = None;
            self.active = false;
        }

        input.is_dragging = self.active;
        input.drag_delta = if self.active { delta } else { [0.0, 0.0] };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a one-frame input snapshot.
    fn frame(x: f32, y: f32, down: bool, clicked: bool, released: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: down,
            mouse_clicked: clicked,
            mouse_released: released,
            ..InputState::default()
        }
    }

    #[test]
    fn press_alone_is_not_a_drag() {
        let mut d = DragTracker::new();
        let mut i = frame(10.0, 10.0, true, true, false);
        d.update(&mut i);
        assert!(!i.is_dragging);
        assert_eq!(i.drag_delta, [0.0, 0.0]);
        assert_eq!(d.origin(), Some((10.0, 10.0)));
    }

    #[test]
    fn click_without_movement_never_drags() {
        let mut d = DragTracker::new();
        let mut press = frame(10.0, 10.0, true, true, false);
        d.update(&mut press);
        // Held in place for several frames.
        for _ in 0..3 {
            let mut hold = frame(10.0, 10.0, true, false, false);
            d.update(&mut hold);
            assert!(!hold.is_dragging);
            assert_eq!(hold.drag_delta, [0.0, 0.0]);
        }
    }

    #[test]
    fn movement_past_threshold_promotes_to_drag() {
        let mut d = DragTracker::new(); // threshold 4.0
        let mut press = frame(10.0, 10.0, true, true, false);
        d.update(&mut press);

        // A 2px nudge stays under the 4px threshold → still a click.
        let mut small = frame(12.0, 10.0, true, false, false);
        d.update(&mut small);
        assert!(!small.is_dragging);
        assert_eq!(
            small.drag_delta,
            [0.0, 0.0],
            "sub-threshold delta is suppressed"
        );

        // Cross the threshold: now dragging, and the per-frame delta is from the
        // previous position (12,10) to the new one (20,10).
        let mut big = frame(20.0, 10.0, true, false, false);
        d.update(&mut big);
        assert!(big.is_dragging);
        assert_eq!(big.drag_delta, [8.0, 0.0]);
        assert!(d.is_dragging());
    }

    #[test]
    fn delta_is_per_frame_while_dragging() {
        let mut d = DragTracker::with_threshold(0.0); // every move drags
        let mut press = frame(0.0, 0.0, true, true, false);
        d.update(&mut press);

        let mut f1 = frame(5.0, 3.0, true, false, false);
        d.update(&mut f1);
        assert!(f1.is_dragging);
        assert_eq!(f1.drag_delta, [5.0, 3.0]);

        let mut f2 = frame(9.0, 3.0, true, false, false);
        d.update(&mut f2);
        assert_eq!(
            f2.drag_delta,
            [4.0, 0.0],
            "delta is movement since last frame"
        );

        // A still frame while held reports zero movement but stays a drag.
        let mut f3 = frame(9.0, 3.0, true, false, false);
        d.update(&mut f3);
        assert!(f3.is_dragging);
        assert_eq!(f3.drag_delta, [0.0, 0.0]);
    }

    #[test]
    fn release_ends_the_drag() {
        let mut d = DragTracker::with_threshold(0.0);
        let mut press = frame(0.0, 0.0, true, true, false);
        d.update(&mut press);
        let mut moved = frame(20.0, 0.0, true, false, false);
        d.update(&mut moved);
        assert!(moved.is_dragging);

        // Release edge: button up this frame.
        let mut rel = frame(20.0, 0.0, false, false, true);
        d.update(&mut rel);
        assert!(!rel.is_dragging);
        assert_eq!(rel.drag_delta, [0.0, 0.0]);
        assert_eq!(d.origin(), None);
    }

    #[test]
    fn new_press_after_release_starts_fresh() {
        let mut d = DragTracker::with_threshold(0.0);
        // First gesture.
        let mut p1 = frame(0.0, 0.0, true, true, false);
        d.update(&mut p1);
        let mut m1 = frame(20.0, 0.0, true, false, false);
        d.update(&mut m1);
        let mut r1 = frame(20.0, 0.0, false, false, true);
        d.update(&mut r1);

        // Second press elsewhere: origin reseats, not immediately dragging.
        let mut p2 = frame(100.0, 100.0, true, true, false);
        d.update(&mut p2);
        assert!(!p2.is_dragging);
        assert_eq!(d.origin(), Some((100.0, 100.0)));
    }

    #[test]
    fn cancel_aborts_active_drag_and_does_not_resume() {
        let mut d = DragTracker::with_threshold(0.0);
        let mut press = frame(0.0, 0.0, true, true, false);
        d.update(&mut press);
        let mut moved = frame(10.0, 0.0, true, false, false);
        d.update(&mut moved);
        assert!(moved.is_dragging);

        d.cancel();
        assert!(!d.is_dragging());
        assert_eq!(d.origin(), None);

        // Still held (no new click edge) → does not resume the drag.
        let mut still_held = frame(30.0, 0.0, true, false, false);
        d.update(&mut still_held);
        assert!(!still_held.is_dragging);
        assert_eq!(still_held.drag_delta, [0.0, 0.0]);
    }

    #[test]
    fn held_button_without_observed_press_does_not_drag() {
        // The button is already down on the first frame the tracker sees, with
        // no click edge → no origin, so no drag is fabricated.
        let mut d = DragTracker::with_threshold(0.0);
        let mut f1 = frame(0.0, 0.0, true, false, false);
        d.update(&mut f1);
        let mut f2 = frame(50.0, 50.0, true, false, false);
        d.update(&mut f2);
        assert!(!f2.is_dragging);
        assert_eq!(f2.drag_delta, [0.0, 0.0]);
    }

    #[test]
    fn threshold_is_configurable() {
        let mut d = DragTracker::with_threshold(20.0);
        let mut press = frame(0.0, 0.0, true, true, false);
        d.update(&mut press);

        // 10px < 20px threshold → not yet.
        let mut a = frame(10.0, 0.0, true, false, false);
        d.update(&mut a);
        assert!(!a.is_dragging);

        // 25px from origin ≥ 20px → drag.
        let mut b = frame(25.0, 0.0, true, false, false);
        d.update(&mut b);
        assert!(b.is_dragging);
        assert_eq!(d.threshold(), 20.0);
    }

    #[test]
    fn negative_threshold_clamps_to_zero() {
        let d = DragTracker::with_threshold(-5.0);
        assert_eq!(d.threshold(), 0.0);
    }

    #[test]
    fn diagonal_threshold_uses_euclidean_distance() {
        // (3,4) is exactly 5px from the origin — promotes at threshold 5.
        let mut d = DragTracker::with_threshold(5.0);
        let mut press = frame(0.0, 0.0, true, true, false);
        d.update(&mut press);
        let mut diag = frame(3.0, 4.0, true, false, false);
        d.update(&mut diag);
        assert!(diag.is_dragging);
    }

    #[test]
    fn end_frame_clears_drag_outputs() {
        let mut i = InputState::default();
        i.is_dragging = true;
        i.drag_delta = [3.0, 4.0];
        i.end_frame();
        assert!(!i.is_dragging);
        assert_eq!(i.drag_delta, [0.0, 0.0]);
    }

    #[test]
    fn consumed_suppresses_drag() {
        let mut i = InputState::default();
        i.is_dragging = true;
        i.drag_delta = [3.0, 4.0];
        let c = i.consumed();
        assert!(!c.is_dragging, "a layer under a modal must not see a drag");
        assert_eq!(c.drag_delta, [0.0, 0.0]);
    }
}
