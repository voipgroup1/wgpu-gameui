//! Double-click and click-and-hold detection.
//!
//! [`ClickTracker`] is a caller-owned cross-frame tracker (same pattern as
//! [`DragTracker`](crate::DragTracker), [`DragCapture`](crate::DragCapture),
//! and [`FocusState`](crate::FocusState)) that needs a wall-clock timestamp to
//! function. Call [`ClickTracker::update`] once per frame **after** filling
//! the mouse fields of your [`InputState`] and **before** drawing widgets. It
//! writes two fields:
//!
//! - [`InputState::mouse_double_clicked`] — `true` on the frame a second press
//!   of the primary button arrives within [`ClickTracker::double_click_threshold`]
//!   seconds of the first. `mouse_clicked` is also `true` on that frame, so
//!   widgets that don't distinguish clicks from double-clicks need not change.
//!   After a double-click fires, the window resets so a third click (however
//!   quick) does not register as a second double.
//!
//! - [`InputState::mouse_held`] — latches `true` once the primary button has
//!   been held for at least [`ClickTracker::hold_threshold`] seconds from the
//!   press edge. Stays `true` until release; the tracker re-asserts it every
//!   `update` call while held, so `end_frame`'s clearing is safe.
//!
//! ## Why timestamps, not frame counts
//!
//! Double-click and hold thresholds are platform conventions measured in
//! wall-clock time (typically ~400–500 ms for double-click, ~500 ms for hold).
//! Frame counts vary with frame rate and stutter, giving wildly different
//! behavior across machines. The crate intentionally has no access to `std::time`
//! in hot paths — pass `elapsed.as_secs_f64()` or the winit frame timestamp.
//!
//! ## Usage
//!
//! ```
//! use wgpu_gameui::{ClickTracker, InputState};
//!
//! let mut clicks = ClickTracker::new();
//! let mut input = InputState::default();
//!
//! // Frame 0: first click at t=0.
//! input.mouse_down = true; input.mouse_clicked = true;
//! clicks.update(&mut input, 0.0);
//! assert!(!input.mouse_double_clicked);
//!
//! input.mouse_clicked = false; // end_frame would do this
//! input.mouse_double_clicked = false;
//!
//! // Frame 1: second click within 450ms threshold.
//! input.mouse_clicked = true;
//! clicks.update(&mut input, 0.3);
//! assert!(input.mouse_double_clicked);
//! ```

use crate::InputState;

/// Default time window for double-click detection (seconds).
pub const DEFAULT_DOUBLE_CLICK_THRESHOLD: f64 = 0.45;

/// Default hold time before [`InputState::mouse_held`] fires (seconds).
pub const DEFAULT_HOLD_THRESHOLD: f64 = 0.5;

/// Detects double-clicks and click-and-hold gestures, writing the result into
/// [`InputState`] each frame.
///
/// Caller-owned; persists across frames. See [module docs](self) for design
/// rationale and usage.
#[derive(Debug, Clone)]
pub struct ClickTracker {
    /// Time of the most recent primary-button press, `f64::NEG_INFINITY` when
    /// none has been seen or after a double-click fires (to reset the window).
    last_click_time: f64,
    /// When the current press started; `None` when no press is active.
    down_since: Option<f64>,
    /// Whether the hold threshold has been crossed on the current gesture.
    /// Latches `true` and is re-asserted each frame until the button releases.
    hold_latched: bool,
    /// Time window for double-click detection (seconds).
    double_click_threshold: f64,
    /// Hold duration before [`InputState::mouse_held`] fires (seconds).
    hold_threshold: f64,
}

impl Default for ClickTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ClickTracker {
    /// A tracker with default thresholds (450 ms double-click, 500 ms hold).
    pub fn new() -> Self {
        Self {
            last_click_time: f64::NEG_INFINITY,
            down_since: None,
            hold_latched: false,
            double_click_threshold: DEFAULT_DOUBLE_CLICK_THRESHOLD,
            hold_threshold: DEFAULT_HOLD_THRESHOLD,
        }
    }

    /// A tracker with custom thresholds (both in seconds).
    pub fn with_thresholds(double_click: f64, hold: f64) -> Self {
        Self {
            double_click_threshold: double_click.max(0.0),
            hold_threshold: hold.max(0.0),
            ..Self::new()
        }
    }

    /// The double-click time window in seconds.
    pub fn double_click_threshold(&self) -> f64 {
        self.double_click_threshold
    }

    /// The hold duration in seconds.
    pub fn hold_threshold(&self) -> f64 {
        self.hold_threshold
    }

    /// True while a click-and-hold is in progress this frame.
    pub fn is_held(&self) -> bool {
        self.hold_latched
    }

    /// Abort any in-progress gesture (e.g. on window blur or focus loss).
    /// The double-click window also resets.
    pub fn cancel(&mut self) {
        self.last_click_time = f64::NEG_INFINITY;
        self.down_since = None;
        self.hold_latched = false;
    }

    /// Advance the tracker by one frame and write the result into `input`.
    ///
    /// `time_secs` is the current wall-clock time in seconds (monotonically
    /// increasing; e.g. `elapsed.as_secs_f64()` or winit's frame timestamp).
    /// Reads `mouse_clicked`/`mouse_down`/`mouse_released`; writes
    /// [`InputState::mouse_double_clicked`] and [`InputState::mouse_held`].
    pub fn update(&mut self, input: &mut InputState, time_secs: f64) {
        // ---- Press edge ----
        if input.mouse_clicked {
            let gap = time_secs - self.last_click_time;
            if gap <= self.double_click_threshold {
                // Second click within the window → double-click.
                input.mouse_double_clicked = true;
                // Reset the window so a rapid third click is NOT another double.
                self.last_click_time = f64::NEG_INFINITY;
            } else {
                // First click in a new window; record the time.
                self.last_click_time = time_secs;
            }
            self.down_since = Some(time_secs);
            self.hold_latched = false;
        }

        // ---- Release / button-up ----
        // Must run before the held check so a release-frame reads mouse_held=false.
        if input.mouse_released || !input.mouse_down {
            self.down_since = None;
            self.hold_latched = false;
        }

        // ---- Hold detection ----
        if input.mouse_down {
            if let Some(since) = self.down_since {
                if time_secs - since >= self.hold_threshold {
                    self.hold_latched = true;
                }
            }
        }

        input.mouse_held = self.hold_latched;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_at(t: f64) -> (InputState, f64) {
        let i = InputState {
            mouse_x: 0.0,
            mouse_y: 0.0,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        };
        (i, t)
    }

    fn held(t: f64) -> (InputState, f64) {
        let i = InputState {
            mouse_down: true,
            ..InputState::default()
        };
        (i, t)
    }

    fn release(t: f64) -> (InputState, f64) {
        let i = InputState {
            mouse_released: true,
            ..InputState::default()
        };
        (i, t)
    }

    fn advance(tracker: &mut ClickTracker, input: &mut InputState, t: f64) {
        tracker.update(input, t);
    }

    // ---- Double-click ----

    #[test]
    fn single_click_is_not_a_double() {
        let mut ct = ClickTracker::new();
        let (mut i, t) = click_at(0.0);
        advance(&mut ct, &mut i, t);
        assert!(!i.mouse_double_clicked);
    }

    #[test]
    fn two_clicks_within_threshold_trigger_double() {
        let mut ct = ClickTracker::new(); // threshold 0.45s
        let (mut i1, t1) = click_at(0.0);
        advance(&mut ct, &mut i1, t1);

        let (mut i2, t2) = click_at(0.3); // 0.3s < 0.45s
        advance(&mut ct, &mut i2, t2);
        assert!(i2.mouse_double_clicked, "second click within window must be a double");
        // Left button click edge still fires on a double-click frame.
        assert!(i2.mouse_clicked, "mouse_clicked is also true on a double-click frame");
    }

    #[test]
    fn two_clicks_outside_threshold_no_double() {
        let mut ct = ClickTracker::new();
        let (mut i1, t1) = click_at(0.0);
        advance(&mut ct, &mut i1, t1);

        let (mut i2, t2) = click_at(0.6); // 0.6s > 0.45s
        advance(&mut ct, &mut i2, t2);
        assert!(!i2.mouse_double_clicked);
    }

    #[test]
    fn third_click_after_double_is_not_another_double() {
        // After a double fires the window resets, so a third rapid click
        // starts a new single-click window rather than tripling.
        let mut ct = ClickTracker::new();
        let (mut i1, _) = click_at(0.0);
        advance(&mut ct, &mut i1, 0.0);

        let (mut i2, _) = click_at(0.2);
        advance(&mut ct, &mut i2, 0.2);
        assert!(i2.mouse_double_clicked, "second click triggers double");

        // Third click — very quick, but window was reset.
        let (mut i3, _) = click_at(0.21);
        advance(&mut ct, &mut i3, 0.21);
        assert!(!i3.mouse_double_clicked, "third click after double must NOT be another double");
    }

    #[test]
    fn custom_double_click_threshold() {
        let mut ct = ClickTracker::with_thresholds(0.1, DEFAULT_HOLD_THRESHOLD);
        let (mut i1, _) = click_at(0.0);
        advance(&mut ct, &mut i1, 0.0);

        // 0.05s < 0.1s threshold → double.
        let (mut i2, _) = click_at(0.05);
        advance(&mut ct, &mut i2, 0.05);
        assert!(i2.mouse_double_clicked);

        // After the double, the window resets to NEG_INFINITY. A click RIGHT
        // after (0.06s) is 0.06 - NEG_INFINITY = ∞ > threshold → fresh single.
        let (mut i3, _) = click_at(0.06);
        advance(&mut ct, &mut i3, 0.06);
        assert!(!i3.mouse_double_clicked, "click just after double-reset must be a fresh single");

        // Two new clicks within the threshold of each other DO produce a double.
        let (mut i4, _) = click_at(0.06 + 0.07); // 0.07s < 0.1s
        advance(&mut ct, &mut i4, 0.06 + 0.07);
        assert!(i4.mouse_double_clicked, "two quick clicks after reset produce another double");
    }

    // ---- Hold detection ----

    #[test]
    fn button_held_past_threshold_fires_mouse_held() {
        let mut ct = ClickTracker::with_thresholds(DEFAULT_DOUBLE_CLICK_THRESHOLD, 0.5);
        let (mut press, _) = click_at(0.0);
        advance(&mut ct, &mut press, 0.0);
        assert!(!press.mouse_held, "fresh press is not a hold");

        // Hold for 0.6s > 0.5s threshold.
        let (mut h, _) = held(0.6);
        advance(&mut ct, &mut h, 0.6);
        assert!(h.mouse_held, "should be held after threshold");
        assert!(ct.is_held());
    }

    #[test]
    fn hold_stays_active_on_subsequent_frames() {
        let mut ct = ClickTracker::with_thresholds(DEFAULT_DOUBLE_CLICK_THRESHOLD, 0.5);
        let (mut press, _) = click_at(0.0);
        advance(&mut ct, &mut press, 0.0);

        let (mut h1, _) = held(0.6);
        advance(&mut ct, &mut h1, 0.6);
        assert!(h1.mouse_held);

        let (mut h2, _) = held(1.0);
        advance(&mut ct, &mut h2, 1.0);
        assert!(h2.mouse_held, "hold persists across frames");
    }

    #[test]
    fn hold_does_not_fire_before_threshold() {
        let mut ct = ClickTracker::with_thresholds(DEFAULT_DOUBLE_CLICK_THRESHOLD, 0.5);
        let (mut press, _) = click_at(0.0);
        advance(&mut ct, &mut press, 0.0);

        let (mut h, _) = held(0.4); // 0.4 < 0.5
        advance(&mut ct, &mut h, 0.4);
        assert!(!h.mouse_held);
    }

    #[test]
    fn release_ends_hold() {
        let mut ct = ClickTracker::with_thresholds(DEFAULT_DOUBLE_CLICK_THRESHOLD, 0.5);
        let (mut press, _) = click_at(0.0);
        advance(&mut ct, &mut press, 0.0);
        let (mut h, _) = held(1.0);
        advance(&mut ct, &mut h, 1.0);
        assert!(h.mouse_held);

        let (mut rel, _) = release(1.1);
        advance(&mut ct, &mut rel, 1.1);
        assert!(!rel.mouse_held, "release must clear hold");
        assert!(!ct.is_held());
    }

    #[test]
    fn held_button_without_observed_press_does_not_hold() {
        // Mouse is already down on the first frame the tracker sees —
        // no click edge → down_since is None → hold never fires.
        let mut ct = ClickTracker::with_thresholds(DEFAULT_DOUBLE_CLICK_THRESHOLD, 0.5);
        let (mut h, _) = held(1.0);
        advance(&mut ct, &mut h, 1.0);
        let (mut h2, _) = held(2.0);
        advance(&mut ct, &mut h2, 2.0);
        assert!(!h2.mouse_held, "no observed press → no hold");
    }

    // ---- cancel ----

    #[test]
    fn cancel_resets_double_click_window() {
        let mut ct = ClickTracker::new();
        let (mut i1, _) = click_at(0.0);
        advance(&mut ct, &mut i1, 0.0);

        ct.cancel();

        // Second click after cancel — window reset, so no double.
        let (mut i2, _) = click_at(0.1);
        advance(&mut ct, &mut i2, 0.1);
        assert!(!i2.mouse_double_clicked);
    }

    #[test]
    fn cancel_aborts_active_hold() {
        let mut ct = ClickTracker::with_thresholds(DEFAULT_DOUBLE_CLICK_THRESHOLD, 0.5);
        let (mut press, _) = click_at(0.0);
        advance(&mut ct, &mut press, 0.0);
        let (mut h, _) = held(1.0);
        advance(&mut ct, &mut h, 1.0);
        assert!(h.mouse_held);

        ct.cancel();
        assert!(!ct.is_held());

        // Still held (no release edge) — but tracker has no down_since, so no hold.
        let (mut still, _) = held(1.1);
        advance(&mut ct, &mut still, 1.1);
        assert!(!still.mouse_held);
    }

    // ---- InputState clearing ----

    #[test]
    fn end_frame_clears_double_clicked_and_held() {
        let mut i = InputState {
            mouse_double_clicked: true,
            mouse_held: true,
            ..InputState::default()
        };
        i.end_frame();
        assert!(!i.mouse_double_clicked);
        assert!(!i.mouse_held);
    }

    #[test]
    fn consumed_zeros_double_clicked_and_held() {
        let i = InputState {
            mouse_double_clicked: true,
            mouse_held: true,
            ..InputState::default()
        };
        let c = i.consumed();
        assert!(!c.mouse_double_clicked, "consumed must zero double_clicked");
        assert!(!c.mouse_held, "consumed must zero held");
    }
}
