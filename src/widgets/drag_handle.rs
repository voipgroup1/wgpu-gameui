//! Drag handle / window-mover: a draggable hit-zone that reports pointer
//! movement so the caller can reposition a window, panel, node, etc.
//!
//! A `DragHandle` is the grab area (typically a window title bar). On press it
//! claims a caller-owned [`DragCapture`] keyed by a stable [`DragId`], so it can
//! never fight a slider, scroll thumb, or an adjacent handle for the same
//! pointer gesture. While it owns the capture it reports the per-frame pointer
//! movement as [`DragHandleOutput::delta`], which the caller adds to whatever it
//! is moving.
//!
//! ## Composition: needs a [`DragTracker`]
//!
//! The handle does **not** re-derive the pointer delta itself — it reads
//! [`InputState::drag_delta`], which a caller-owned
//! [`DragTracker`](crate::DragTracker) writes once per frame. This is the
//! intended split (see the `DragTracker` module docs): `DragCapture` answers
//! *who* owns the drag, `DragTracker` answers *how far the pointer moved*. So a
//! window-mover wires **both**: run `DragTracker::update(&mut input)` each frame
//! before drawing, then thread a shared `DragCapture` into every `DragHandle`.
//!
//! Because `drag_delta` only becomes non-zero once the gesture crosses the
//! tracker's threshold, a stationary click never nudges the window, and the
//! grab point stays under the cursor (minus the small pre-threshold dead-zone).
//!
//! ## Example
//!
//! ```ignore
//! // Per frame, after filling `input`'s mouse fields:
//! drag_tracker.update(&mut input);              // writes input.drag_delta
//!
//! let bar = Rect::new(win.x, win.y, win.width, 24.0);
//! let out = DragHandle::new().with_label("Inspector")
//!     .drag_rect(WIN_ID, &mut capture, bar, &mut win, &mut ctx);
//! // `win` has now moved by the drag; `out.dragging` is true while held.
//! ```

use crate::layout::Rect;
use crate::text::TextBlock;

use super::{DragCapture, DragId, DrawContext};

/// Result of drawing a [`DragHandle`] for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragHandleOutput {
    /// True while this handle owns the drag capture (mouse held since it
    /// claimed the gesture on this handle).
    pub dragging: bool,
    /// True only on the frame the handle claimed the capture (the press edge).
    pub started: bool,
    /// True only on the frame the handle released the capture (the mouse-up
    /// edge while it owned the drag).
    pub released: bool,
    /// Per-frame pointer movement to apply to the moved object. `[0.0, 0.0]`
    /// unless this handle is dragging *and* the gesture has crossed the
    /// [`DragTracker`](crate::DragTracker) threshold.
    pub delta: [f32; 2],
}

impl DragHandleOutput {
    /// A "nothing happened" result (not dragging, no movement).
    pub fn idle() -> Self {
        Self {
            dragging: false,
            started: false,
            released: false,
            delta: [0.0, 0.0],
        }
    }
}

/// A draggable grab-zone for moving windows/panels. See the [module docs](self).
///
/// Drawn against a layout-computed [`Rect`]. Drag ownership is arbitrated via a
/// caller-owned [`DragCapture`] + a stable [`DragId`]; the per-frame delta comes
/// from a caller-owned [`DragTracker`](crate::DragTracker) via
/// [`InputState::drag_delta`](crate::InputState::drag_delta).
pub struct DragHandle {
    label: Option<String>,
    grip: bool,
    chrome: bool,
}

impl Default for DragHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl DragHandle {
    /// A handle that draws title-bar chrome (background + a centred grip glyph).
    pub fn new() -> Self {
        Self {
            label: None,
            grip: true,
            chrome: true,
        }
    }

    /// A chrome-less handle: no background, no grip — just the hit zone and the
    /// drag delta. Use when you draw your own title bar and only need the
    /// movement. Mirrors [`ImageButton::bare`](super::ImageButton).
    pub fn bare() -> Self {
        Self {
            label: None,
            grip: false,
            chrome: false,
        }
    }

    /// Show `label` (left-aligned, vertically centred) on the bar.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Toggle the centred grip-dots glyph (on by default for [`new`](Self::new)).
    pub fn with_grip(mut self, grip: bool) -> Self {
        self.grip = grip;
        self
    }

    /// Draw the handle and report this frame's drag result.
    ///
    /// `id` is this handle's stable [`DragId`]; `capture` is the shared
    /// [`DragCapture`]. The handle claims the drag only when the capture is free
    /// and the press lands on `rect` (and the pointer isn't already consumed by
    /// a higher layer), and reports a non-zero [`delta`](DragHandleOutput::delta)
    /// only while it owns the capture.
    pub fn draw(
        &self,
        id: DragId,
        capture: &mut DragCapture,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> DragHandleOutput {
        let input = ctx.input;
        let theme = ctx.theme;

        let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let was_active = capture.is_active(id);

        // Release first so a mouse-up frame reports not-dragging. `release` is a
        // no-op unless we own the capture, so it never clobbers another widget's
        // active drag.
        if !input.mouse_down {
            capture.release(id);
        }

        // Claim the drag on a fresh press over the handle, if nothing owns it.
        let mut started = false;
        if hovered && input.mouse_clicked && capture.is_free() {
            started = capture.try_begin(id);
        }

        let dragging = capture.is_active(id);
        let released = was_active && !dragging;

        // The per-frame movement comes from the caller's DragTracker. It is
        // already `[0,0]` until the gesture crosses the threshold and is zeroed
        // when the pointer is consumed, so applying it only while we own the
        // capture is all the gating we need.
        let delta = if dragging {
            input.drag_delta
        } else {
            [0.0, 0.0]
        };

        // --- chrome ------------------------------------------------------
        if self.chrome {
            let list = &mut *ctx.draw_list;
            if theme.border_radius > 0.0 {
                list.rounded_rect(rect, theme.border_radius, theme.panel);
            } else {
                list.quad(rect.x, rect.y, rect.width, rect.height, theme.panel);
            }
            // Brighten on hover / while dragging for affordance.
            if dragging || hovered {
                let a = if dragging { 0.12 } else { 0.06 };
                if theme.border_radius > 0.0 {
                    list.rounded_rect(rect, theme.border_radius, [1.0, 1.0, 1.0, a]);
                } else {
                    list.quad(rect.x, rect.y, rect.width, rect.height, [1.0, 1.0, 1.0, a]);
                }
            }
        }

        if self.grip {
            self.draw_grip(ctx, rect);
        }

        if let Some(label) = &self.label {
            let list = &mut *ctx.draw_list;
            let font_size = theme.font_size;
            let text_x = rect.x + theme.padding;
            let text_y =
                list.vcentered_text_y(rect.y, rect.height, font_size, theme.font.as_ref(), label);
            let c = theme.text;
            let block = TextBlock::new(label, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (c[0] * 255.0) as u8,
                    (c[1] * 255.0) as u8,
                    (c[2] * 255.0) as u8,
                )
                .with_font_opt(theme.font.clone());
            list.text(block);
        }

        DragHandleOutput {
            dragging,
            started,
            released,
            delta,
        }
    }

    /// Convenience: draw the handle at `handle_rect` and apply the resulting
    /// delta directly to `target`, moving it. Returns the same output as
    /// [`draw`](Self::draw); inspect [`delta`](DragHandleOutput::delta) if you
    /// also want to move the handle rect to follow (callers usually recompute
    /// `handle_rect` from the moved `target` next frame).
    pub fn drag_rect(
        &self,
        id: DragId,
        capture: &mut DragCapture,
        handle_rect: Rect,
        target: &mut Rect,
        ctx: &mut DrawContext,
    ) -> DragHandleOutput {
        let out = self.draw(id, capture, handle_rect, ctx);
        target.x += out.delta[0];
        target.y += out.delta[1];
        out
    }

    /// Draw a 2×3 grid of dots centred in `rect` as a grab affordance.
    fn draw_grip(&self, ctx: &mut DrawContext, rect: Rect) {
        let color = ctx.theme.text_dim;
        let dot = 2.0_f32;
        let gap = 3.0_f32;
        let cols = 3;
        let rows = 2;
        let grid_w = cols as f32 * dot + (cols as f32 - 1.0) * gap;
        let grid_h = rows as f32 * dot + (rows as f32 - 1.0) * gap;
        // Centre the dot grid in the bar. With a label present the grip would
        // collide with text, so only draw it when unlabeled.
        if self.label.is_some() {
            return;
        }
        let ox = rect.x + (rect.width - grid_w) / 2.0;
        let oy = rect.y + (rect.height - grid_h) / 2.0;
        let list = &mut *ctx.draw_list;
        for r in 0..rows {
            for c in 0..cols {
                let x = ox + c as f32 * (dot + gap);
                let y = oy + r as f32 * (dot + gap);
                list.quad(x, y, dot, dot, color);
            }
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

    /// Draw a handle into a throwaway context and return (output, quad count).
    fn draw_handle(
        handle: &DragHandle,
        id: DragId,
        cap: &mut DragCapture,
        rect: Rect,
        input: &InputState,
    ) -> (DragHandleOutput, usize) {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = theme();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        let out = handle.draw(id, cap, rect, &mut ctx);
        // Filled rects/rounded-rects emit `chrome_instances` (not vertex soup)
        // under the identity transform; the grip is plain quads through the
        // same path. Count all geometry buffers so the metric is path-agnostic.
        let geom = list.chrome_instances.len() + list.vertices.len() + list.texts.len();
        (out, geom)
    }

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 24.0)
    }

    /// Press (down + click edge) at (x, y).
    fn press_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    /// Held (down, no fresh click) at (x, y), already dragging `delta`.
    fn drag_at(x: f32, y: f32, delta: [f32; 2]) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            is_dragging: true,
            drag_delta: delta,
            ..InputState::default()
        }
    }

    fn release_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    #[test]
    fn press_on_handle_claims_capture_without_jumping() {
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        let (out, _) = draw_handle(&h, 1, &mut cap, rect(), &press_at(20.0, 12.0));
        assert!(out.dragging, "press over the handle begins the drag");
        assert!(out.started, "the claim edge is reported");
        assert!(cap.is_active(1));
        assert_eq!(out.delta, [0.0, 0.0], "no movement on the press frame");
    }

    #[test]
    fn delta_passes_through_while_owning_capture() {
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        draw_handle(&h, 1, &mut cap, rect(), &press_at(20.0, 12.0));

        let moved = drag_at(35.0, 18.0, [15.0, 6.0]);
        let (out, _) = draw_handle(&h, 1, &mut cap, rect(), &moved);
        assert!(out.dragging);
        assert!(!out.started, "claim edge only fires once");
        assert_eq!(out.delta, [15.0, 6.0], "owner applies the tracker delta");
    }

    #[test]
    fn release_frees_capture_and_reports_edge() {
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        draw_handle(&h, 1, &mut cap, rect(), &press_at(20.0, 12.0));

        let (out, _) = draw_handle(&h, 1, &mut cap, rect(), &release_at(40.0, 12.0));
        assert!(!out.dragging);
        assert!(out.released, "the release edge is reported");
        assert!(cap.is_free());
        assert_eq!(out.delta, [0.0, 0.0]);
    }

    #[test]
    fn consumed_pointer_does_not_start_drag() {
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        let mut input = press_at(20.0, 12.0);
        input.mouse_consumed = true; // a higher layer took the click
        let (out, _) = draw_handle(&h, 1, &mut cap, rect(), &input);
        assert!(!out.dragging);
        assert!(cap.is_free());
    }

    #[test]
    fn second_handle_cannot_steal_active_drag() {
        // Two handles over the same rect sharing one capture: only the first to
        // claim the gesture follows the pointer.
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        let r = rect();

        let down = press_at(20.0, 12.0);
        let (a, _) = draw_handle(&h, 0, &mut cap, r, &down);
        let (b, _) = draw_handle(&h, 1, &mut cap, r, &down);
        assert!(a.dragging, "first handle claims");
        assert!(!b.dragging, "second cannot also grab the same press");

        let moved = drag_at(60.0, 12.0, [40.0, 0.0]);
        let (a2, _) = draw_handle(&h, 0, &mut cap, r, &moved);
        let (b2, _) = draw_handle(&h, 1, &mut cap, r, &moved);
        assert_eq!(a2.delta, [40.0, 0.0]);
        assert_eq!(b2.delta, [0.0, 0.0], "non-owner reports no movement");
    }

    #[test]
    fn non_owner_ignores_tracker_delta() {
        // A handle that never claimed the capture must not move even though the
        // global `drag_delta` is non-zero (some other widget owns the gesture).
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        cap.try_begin(99); // someone else owns it
        let moved = drag_at(35.0, 12.0, [15.0, 0.0]);
        let (out, _) = draw_handle(&h, 1, &mut cap, rect(), &moved);
        assert!(!out.dragging);
        assert_eq!(out.delta, [0.0, 0.0]);
    }

    #[test]
    fn drag_rect_moves_target_by_delta() {
        let h = DragHandle::new();
        let mut cap = DragCapture::new();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = theme();

        // Claim on press.
        let down = press_at(20.0, 12.0);
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &down, 800.0, 600.0);
            h.draw(0, &mut cap, rect(), &mut ctx);
        }

        // Move: drag_rect should translate the window.
        let mut win = Rect::new(0.0, 0.0, 200.0, 120.0);
        let moved = drag_at(30.0, 20.0, [10.0, 8.0]);
        let out = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &moved, 800.0, 600.0);
            h.drag_rect(0, &mut cap, rect(), &mut win, &mut ctx)
        };
        assert_eq!(out.delta, [10.0, 8.0]);
        assert_eq!((win.x, win.y), (10.0, 8.0), "target moved by the delta");
        assert_eq!((win.width, win.height), (200.0, 120.0), "size unchanged");
    }

    #[test]
    fn bare_draws_no_chrome() {
        let mut cap = DragCapture::new();
        let idle = InputState::default();
        let (_, chrome_verts) = draw_handle(&DragHandle::new(), 0, &mut cap, rect(), &idle);
        let mut cap2 = DragCapture::new();
        let (_, bare_verts) = draw_handle(&DragHandle::bare(), 0, &mut cap2, rect(), &idle);
        assert_eq!(bare_verts, 0, "bare handle emits no geometry");
        assert!(chrome_verts > 0, "default handle draws a background + grip");
    }
}
