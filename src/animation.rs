//! Hover/press transition animations — an easing clock for smoothing the
//! state-driven color changes widgets make (idle→hover→press, check/uncheck,
//! tab activate). Today those colors switch instantly; an [`AnimationState`]
//! eases them over a short, themeable duration.
//!
//! ## Model: animate *toward the resolved color*
//!
//! A widget still resolves its target color discretely each frame (e.g.
//! `if pressed { Pressed } else if hovered { Hover } else { Button }`). Instead
//! of drawing that target directly, it asks the [`AnimationState`] for the value
//! to draw *this* frame — the state holds the currently-displayed color per
//! `(widget-id, slot)` and walks it toward the target over `duration` seconds
//! with an [`Easing`] curve. When the target changes mid-transition the walk
//! re-bases from wherever it currently is, so it never blends two non-adjacent
//! states into a muddy in-between.
//!
//! ## Caller-owned, like every other state in this crate
//!
//! [`AnimationState`] is constructed by the caller, threaded by `&mut`, and
//! ticked once per frame ([`AnimationState::tick`]) with the frame delta —
//! mirroring [`TooltipLayer::tick`](crate::TooltipLayer) and the
//! `FocusState`/`DragCapture` ownership pattern. Widgets reach it through
//! [`DrawContext`](crate::DrawContext) (the optional `animations` field). With no
//! `AnimationState` threaded — or `animation_duration == 0` — every value is
//! returned unchanged, so rendering is byte-identical to the un-animated path.
//!
//! Per-widget continuity needs a stable id: widgets that take an id (or the
//! `UiContext` façade, which auto-assigns one per verb) animate; an id-less raw
//! draw simply snaps as before.

use std::collections::{HashMap, HashSet};

/// Easing curve applied to a normalized `0..1` transition progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    /// Constant rate.
    Linear,
    /// Slow start, cubic.
    EaseIn,
    /// Fast start, settle out — the default for hover/press (feels responsive).
    #[default]
    EaseOut,
    /// Slow at both ends, cubic.
    EaseInOut,
}

/// Apply `easing` to `t`, which is clamped to `0..=1` first.
pub fn ease(easing: Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => t,
        Easing::EaseIn => t * t * t,
        Easing::EaseOut => {
            let u = 1.0 - t;
            1.0 - u * u * u
        }
        Easing::EaseInOut => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                let u = -2.0 * t + 2.0;
                1.0 - (u * u * u) / 2.0
            }
        }
    }
}

/// Linear interpolation with **endpoint snapping**: `t <= 0` returns `a` and
/// `t >= 1` returns `b` bit-for-bit (so an un-animated / settled transition is
/// exactly the target, not an off-by-an-ULP approximation).
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        a + (b - a) * t
    }
}

/// Per-channel [`lerp`] of two RGBA colors (endpoint-snapping).
pub fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ]
    }
}

/// Which visual property of a widget is being animated. Lets one `widget-id`
/// drive several independent transitions (a button's fill *and* border, a
/// checkbox's hover overlay *and* box fill).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimSlot {
    /// Background / fill of the chrome.
    Bg,
    /// Border / outline color.
    Border,
    /// Text / label color.
    Text,
    /// A translucent hover/press overlay's alpha.
    Overlay,
    /// An active/selection indicator (tab underline, etc.).
    Indicator,
    /// A toggled fill (checkbox box, radio dot).
    Fill,
}

/// One in-flight color transition: walks `start`→`target` as `t` advances to
/// `duration`. Settled when `start == target`.
#[derive(Debug, Clone, Copy)]
struct ColorAnim {
    start: [f32; 4],
    target: [f32; 4],
    t: f32,
}

impl ColorAnim {
    fn current(&self, easing: Easing, duration: f32) -> [f32; 4] {
        let frac = if duration <= 0.0 { 1.0 } else { self.t / duration };
        lerp_color(self.start, self.target, ease(easing, frac))
    }
}

/// One in-flight scalar transition (e.g. an overlay alpha).
#[derive(Debug, Clone, Copy)]
struct ScalarAnim {
    start: f32,
    target: f32,
    t: f32,
}

impl ScalarAnim {
    fn current(&self, easing: Easing, duration: f32) -> f32 {
        let frac = if duration <= 0.0 { 1.0 } else { self.t / duration };
        lerp(self.start, self.target, ease(easing, frac))
    }
}

/// Caller-owned store of in-flight hover/press transitions. See the
/// module docs.
///
/// Construct with [`new`](Self::new), [`tick`](Self::tick) once per frame with
/// the frame delta, and thread `&mut` into widgets (via
/// [`DrawContext::with_animations`](crate::DrawContext::with_animations) or the
/// `UiContext` façade).
#[derive(Debug, Default)]
pub struct AnimationState {
    color_anims: HashMap<(u64, AnimSlot), ColorAnim>,
    scalar_anims: HashMap<(u64, AnimSlot), ScalarAnim>,
    /// Keys touched this frame; used by [`tick`](Self::tick) to reap entries for
    /// widgets that stopped being drawn. Reused (cleared each tick) to avoid a
    /// per-frame allocation.
    seen: HashSet<(u64, AnimSlot)>,
    dt: f32,
}

impl AnimationState {
    /// A fresh, empty animation store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the frame clock by `dt` seconds. Call **once per frame** before
    /// drawing. Also reaps transitions for `(id, slot)` keys that weren't drawn
    /// last frame (so a UI whose widget set changes doesn't accumulate stale
    /// entries).
    pub fn tick(&mut self, dt: f32) {
        // Keep only what was drawn last frame; this frame's draws re-insert.
        self.color_anims.retain(|k, _| self.seen.contains(k));
        self.scalar_anims.retain(|k, _| self.seen.contains(k));
        self.seen.clear();
        self.dt = dt.max(0.0);
    }

    /// Ease the stored color for `(id, slot)` toward `target` and return the
    /// value to draw this frame.
    ///
    /// - First time a key is seen (or `duration <= 0`): returns `target`
    ///   immediately (no fade-in pop, snap-to when disabled).
    /// - `dt == 0`: returns the stored value unchanged.
    /// - Otherwise advances by the frame `dt` along `easing`; a settled
    ///   transition returns `target` bit-for-bit.
    pub fn animate_color(
        &mut self,
        id: u64,
        slot: AnimSlot,
        target: [f32; 4],
        duration: f32,
        easing: Easing,
    ) -> [f32; 4] {
        let key = (id, slot);
        self.seen.insert(key);

        if duration <= 0.0 {
            self.color_anims.insert(
                key,
                ColorAnim {
                    start: target,
                    target,
                    t: 0.0,
                },
            );
            return target;
        }

        let dt = self.dt;
        match self.color_anims.get_mut(&key) {
            None => {
                // First sight: appear at rest on the target (settled).
                self.color_anims.insert(
                    key,
                    ColorAnim {
                        start: target,
                        target,
                        t: duration,
                    },
                );
                target
            }
            Some(a) => {
                if a.target != target {
                    // Target moved: re-base from the currently-shown color.
                    let cur = a.current(easing, duration);
                    a.start = cur;
                    a.target = target;
                    a.t = 0.0;
                }
                if dt > 0.0 {
                    a.t += dt;
                    if a.t >= duration {
                        a.t = duration;
                        a.start = target; // settle exactly
                    }
                }
                a.current(easing, duration)
            }
        }
    }

    /// Scalar counterpart of [`animate_color`](Self::animate_color) — e.g. a
    /// hover overlay's alpha walking `0.0 ↔ 1.0`.
    pub fn animate_scalar(
        &mut self,
        id: u64,
        slot: AnimSlot,
        target: f32,
        duration: f32,
        easing: Easing,
    ) -> f32 {
        let key = (id, slot);
        self.seen.insert(key);

        if duration <= 0.0 {
            self.scalar_anims.insert(
                key,
                ScalarAnim {
                    start: target,
                    target,
                    t: 0.0,
                },
            );
            return target;
        }

        let dt = self.dt;
        match self.scalar_anims.get_mut(&key) {
            None => {
                self.scalar_anims.insert(
                    key,
                    ScalarAnim {
                        start: target,
                        target,
                        t: duration,
                    },
                );
                target
            }
            Some(a) => {
                if a.target != target {
                    let cur = a.current(easing, duration);
                    a.start = cur;
                    a.target = target;
                    a.t = 0.0;
                }
                if dt > 0.0 {
                    a.t += dt;
                    if a.t >= duration {
                        a.t = duration;
                        a.start = target;
                    }
                }
                a.current(easing, duration)
            }
        }
    }

    /// Number of in-flight transitions tracked (test/introspection helper).
    pub fn len(&self) -> usize {
        self.color_anims.len() + self.scalar_anims.len()
    }

    /// Whether nothing is currently tracked.
    pub fn is_empty(&self) -> bool {
        self.color_anims.is_empty() && self.scalar_anims.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUR: f32 = 0.1;
    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    #[test]
    fn ease_endpoints_and_clamp() {
        for e in [Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
            assert_eq!(ease(e, 0.0), 0.0, "{e:?} at 0");
            assert_eq!(ease(e, 1.0), 1.0, "{e:?} at 1");
            assert_eq!(ease(e, -5.0), 0.0, "{e:?} clamps low");
            assert_eq!(ease(e, 5.0), 1.0, "{e:?} clamps high");
        }
        assert_eq!(ease(Easing::Linear, 0.42), 0.42);
    }

    #[test]
    fn ease_is_monotonic() {
        for e in [Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
            let mut prev = -1.0;
            for i in 0..=20 {
                let v = ease(e, i as f32 / 20.0);
                assert!(v >= prev - 1e-6, "{e:?} not monotonic at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn lerp_endpoint_snap_is_bit_exact() {
        assert_eq!(lerp(3.0, 9.0, 0.0), 3.0);
        assert_eq!(lerp(3.0, 9.0, 1.0), 9.0);
        assert_eq!(lerp(3.0, 9.0, -0.1), 3.0);
        assert_eq!(lerp(3.0, 9.0, 1.1), 9.0);
        assert_eq!(lerp(3.0, 9.0, 0.5), 6.0);
        assert_eq!(lerp_color(RED, BLUE, 0.0), RED);
        assert_eq!(lerp_color(RED, BLUE, 1.0), BLUE);
        assert_eq!(lerp_color(RED, BLUE, 0.5), [0.5, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn first_sight_returns_target_exactly() {
        let mut a = AnimationState::new();
        a.tick(1.0 / 60.0);
        assert_eq!(a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::EaseOut), RED);
    }

    #[test]
    fn zero_duration_snaps() {
        let mut a = AnimationState::new();
        a.tick(1.0 / 60.0);
        // First sight settled at RED.
        assert_eq!(a.animate_color(1, AnimSlot::Bg, RED, 0.0, Easing::EaseOut), RED);
        a.tick(1.0 / 60.0);
        // Target jumps to BLUE with duration 0 → snaps immediately.
        assert_eq!(a.animate_color(1, AnimSlot::Bg, BLUE, 0.0, Easing::EaseOut), BLUE);
    }

    #[test]
    fn transition_is_intermediate_then_settles() {
        let mut a = AnimationState::new();
        // Frame 1: settle at RED.
        a.tick(0.0);
        assert_eq!(a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear), RED);
        // Frame 2: target → BLUE, advance half the duration → strictly between.
        a.tick(DUR / 2.0);
        let mid = a.animate_color(1, AnimSlot::Bg, BLUE, DUR, Easing::Linear);
        assert!(mid != RED && mid != BLUE, "mid-transition: {mid:?}");
        assert!(mid[2] > 0.0 && mid[2] < 1.0, "blue channel mid: {mid:?}");
        // Frame 3: advance past the end → settles exactly on BLUE.
        a.tick(DUR);
        let done = a.animate_color(1, AnimSlot::Bg, BLUE, DUR, Easing::Linear);
        assert_eq!(done, BLUE);
    }

    #[test]
    fn dt_zero_holds_stored_value() {
        let mut a = AnimationState::new();
        a.tick(0.0);
        a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear);
        a.tick(DUR / 2.0);
        let mid = a.animate_color(1, AnimSlot::Bg, BLUE, DUR, Easing::Linear);
        // dt==0 next frame: same target, value must not move.
        a.tick(0.0);
        let held = a.animate_color(1, AnimSlot::Bg, BLUE, DUR, Easing::Linear);
        assert_eq!(held, mid, "dt==0 must hold the stored value");
    }

    #[test]
    fn retarget_midflight_rebases_smoothly() {
        let mut a = AnimationState::new();
        a.tick(0.0);
        a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear);
        a.tick(DUR / 2.0);
        let mid = a.animate_color(1, AnimSlot::Bg, BLUE, DUR, Easing::Linear);
        // Reverse the target back to RED from the mid value — no jump.
        a.tick(0.0);
        let after = a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear);
        assert_eq!(after, mid, "retarget on dt==0 must start from current value");
    }

    #[test]
    fn scalar_overlay_alpha_animates() {
        let mut a = AnimationState::new();
        a.tick(0.0);
        assert_eq!(a.animate_scalar(7, AnimSlot::Overlay, 0.0, DUR, Easing::Linear), 0.0);
        a.tick(DUR / 2.0);
        let mid = a.animate_scalar(7, AnimSlot::Overlay, 1.0, DUR, Easing::Linear);
        assert!(mid > 0.0 && mid < 1.0, "alpha mid: {mid}");
    }

    #[test]
    fn tick_reaps_undrawn_entries() {
        let mut a = AnimationState::new();
        a.tick(1.0 / 60.0);
        a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear);
        a.animate_color(2, AnimSlot::Bg, BLUE, DUR, Easing::Linear);
        assert_eq!(a.len(), 2);
        // Next frame: only redraw id 1. id 2 wasn't drawn → reaped on the tick
        // after that.
        a.tick(1.0 / 60.0);
        a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear);
        a.tick(1.0 / 60.0);
        assert_eq!(a.len(), 1, "undrawn entry should be reaped");
    }

    #[test]
    fn distinct_slots_are_independent() {
        let mut a = AnimationState::new();
        a.tick(0.0);
        a.animate_color(1, AnimSlot::Bg, RED, DUR, Easing::Linear);
        a.animate_color(1, AnimSlot::Border, BLUE, DUR, Easing::Linear);
        assert_eq!(a.len(), 2, "same id, two slots = two transitions");
    }
}
