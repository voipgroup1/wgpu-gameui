//! Keyboard-focus arbitration: a single focused widget across the whole UI.
//!
//! Immediate-mode focusable widgets (text inputs today; buttons/sliders later)
//! each receive a stable [`FocusId`] and a shared `&mut FocusState`. Every frame
//! a focusable *registers* itself (in draw order) and, when clicked, *requests*
//! focus. At most one [`FocusId`] is focused at a time, so only the focused
//! widget consumes keyboard input — overlapping or adjacent text fields can't all
//! activate at once.
//!
//! Navigation is resolved once per frame in [`FocusState::end_frame`], driven by
//! the per-frame Tab/Escape/click edges captured in [`FocusState::begin_frame`]:
//! - **Tab / Shift+Tab** cycle focus forward/backward through the widgets
//!   registered this frame (the draw-order ring).
//! - **Escape** blurs.
//! - A click that no focusable claimed (empty space, a button, a slider) blurs —
//!   "click elsewhere to dismiss".
//!
//! `FocusState` is caller-owned and persists across frames — construct one per UI
//! surface and thread `&mut` into every focusable you draw, the same way the
//! crate already threads caller-owned `DragCapture` into `Slider` and
//! `ScrollState` into `ScrollView`. The lifecycle mirrors the
//! [`InputState::end_frame`](crate::InputState::end_frame) call already in the
//! event loop: call [`begin_frame`](FocusState::begin_frame) before drawing the
//! focusables and [`end_frame`](FocusState::end_frame) after.
//!
//! ## Timing
//! Click-to-focus and Escape/click-blur take effect the same frame. **Tab**
//! navigation is resolved at `end_frame` against the order registered this frame,
//! so a Tab press moves the visible focus on the *next* frame — one frame (~16 ms)
//! of latency. This is the standard immediate-mode approach (e.g. Dear ImGui) and
//! is imperceptible in practice.
//!
//! ## Out of scope
//! Modal/popup-scoped Tab trapping. Click-to-focus already respects
//! `mouse_consumed` (focusables AND their hit test with `!mouse_consumed`), so a
//! modal won't mis-focus a base-layer widget; only Tab cycling is not yet scoped
//! to the active layer.

use crate::InputState;

/// Stable identity for a focusable widget within one UI surface.
///
/// Any scheme that is unique per focusable per frame works: a hash of a widget
/// path, an enum discriminant, a loop index, etc. `0` is a valid id.
pub type FocusId = u64;

/// Arbitrates which widget currently holds keyboard focus.
///
/// At most one [`FocusId`] is focused at a time. Caller-owned; persists across
/// frames. Drive it with [`begin_frame`](Self::begin_frame) /
/// [`end_frame`](Self::end_frame) around the focusable draws each frame.
#[derive(Debug, Default, Clone)]
pub struct FocusState {
    /// The widget that currently holds focus, if any.
    focused: Option<FocusId>,
    /// Focusables registered this frame, in draw order — the Tab ring.
    order: Vec<FocusId>,
    /// Tab direction captured this frame: `+1` forward, `-1` backward, `0` none.
    tab: i32,
    /// Escape was pressed this frame.
    escape: bool,
    /// A click happened this frame (from the dispatched input).
    mouse_clicked: bool,
    /// A focusable claimed this frame's click (suppresses click-elsewhere blur).
    click_claimed: bool,
}

impl FocusState {
    /// A fresh focus owner with nothing focused.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a frame: clear the draw-order ring and capture this frame's
    /// Tab/Escape/click edges from `input`. Call once per frame, before drawing
    /// the focusable widgets, with the same input those widgets receive.
    pub fn begin_frame(&mut self, input: &InputState) {
        self.order.clear();
        self.tab = if input.key_tab {
            if input.shift_pressed {
                -1
            } else {
                1
            }
        } else {
            0
        };
        self.escape = input.key_escape;
        self.mouse_clicked = input.mouse_clicked;
        self.click_claimed = false;
    }

    /// Register `id` as focusable this frame. Establishes its position in the
    /// Tab ring (draw order). Every focusable calls this each frame it is drawn.
    pub fn register(&mut self, id: FocusId) {
        self.order.push(id);
    }

    /// True when `id` currently holds focus.
    pub fn is_focused(&self, id: FocusId) -> bool {
        self.focused == Some(id)
    }

    /// The widget that currently holds focus, if any.
    pub fn focused(&self) -> Option<FocusId> {
        self.focused
    }

    /// Request focus for `id` (e.g. when the widget is clicked). Takes effect
    /// immediately and marks this frame's click as claimed, so it won't also
    /// trigger a click-elsewhere blur.
    pub fn request(&mut self, id: FocusId) {
        self.focused = Some(id);
        self.click_claimed = true;
    }

    /// Programmatically focus `id` without consuming a click (e.g. to seed
    /// initial focus, or focus a field after a validation error).
    pub fn focus(&mut self, id: FocusId) {
        self.focused = Some(id);
    }

    /// Programmatically clear focus.
    pub fn blur(&mut self) {
        self.focused = None;
    }

    /// End a frame: resolve Escape, click-elsewhere blur, and Tab navigation.
    /// Call once per frame, after drawing the focusable widgets.
    pub fn end_frame(&mut self) {
        // Escape blurs.
        if self.escape {
            self.focused = None;
        }
        // A click that no focusable claimed blurs the current focus.
        if self.mouse_clicked && !self.click_claimed {
            self.focused = None;
        }
        // Tab / Shift+Tab cycle through this frame's registered focusables.
        if self.tab != 0 && !self.order.is_empty() {
            let len = self.order.len() as i32;
            let next = match self
                .focused
                .and_then(|f| self.order.iter().position(|&x| x == f))
            {
                Some(idx) => (idx as i32 + self.tab).rem_euclid(len) as usize,
                // Nothing focused yet: forward → first, backward → last.
                None => {
                    if self.tab > 0 {
                        0
                    } else {
                        (len - 1) as usize
                    }
                }
            };
            self.focused = Some(self.order[next]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `InputState` with the focus-relevant edges set.
    fn input(tab: bool, shift: bool, escape: bool, clicked: bool) -> InputState {
        InputState {
            key_tab: tab,
            shift_pressed: shift,
            key_escape: escape,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    #[test]
    fn fresh_state_has_no_focus() {
        let f = FocusState::new();
        assert_eq!(f.focused(), None);
        assert!(!f.is_focused(0));
    }

    #[test]
    fn single_owner_request_replaces() {
        let mut f = FocusState::new();
        f.begin_frame(&input(false, false, false, true));
        f.register(1);
        f.register(2);
        f.request(1);
        f.request(2); // second request wins; only one owner ever
        assert!(f.is_focused(2));
        assert!(!f.is_focused(1));
        f.end_frame();
        assert!(f.is_focused(2)); // claimed click → no blur
    }

    #[test]
    fn click_elsewhere_blurs() {
        let mut f = FocusState::new();
        f.focus(9);
        // A click occurs, but no focusable requests it.
        f.begin_frame(&input(false, false, false, true));
        f.register(9);
        f.end_frame();
        assert_eq!(f.focused(), None);
    }

    #[test]
    fn claimed_click_does_not_blur() {
        let mut f = FocusState::new();
        f.focus(9);
        f.begin_frame(&input(false, false, false, true));
        f.register(9);
        f.request(9); // the focused widget itself was clicked
        f.end_frame();
        assert!(f.is_focused(9));
    }

    #[test]
    fn no_click_keeps_focus() {
        let mut f = FocusState::new();
        f.focus(3);
        f.begin_frame(&input(false, false, false, false));
        f.register(3);
        f.end_frame();
        assert!(f.is_focused(3));
    }

    #[test]
    fn escape_blurs() {
        let mut f = FocusState::new();
        f.focus(5);
        f.begin_frame(&input(false, false, true, false));
        f.register(5);
        f.end_frame();
        assert_eq!(f.focused(), None);
    }

    #[test]
    fn tab_cycles_forward_with_wrap() {
        let mut f = FocusState::new();
        f.focus(10);
        // Frame 1: Tab forward 10 -> 20.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register(30);
        f.end_frame();
        assert!(f.is_focused(20));
        // Frame 2: Tab forward 20 -> 30.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register(30);
        f.end_frame();
        assert!(f.is_focused(30));
        // Frame 3: Tab forward 30 -> wrap to 10.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register(30);
        f.end_frame();
        assert!(f.is_focused(10));
    }

    #[test]
    fn shift_tab_cycles_backward_with_wrap() {
        let mut f = FocusState::new();
        f.focus(10);
        // Shift+Tab backward 10 -> wrap to 30.
        f.begin_frame(&input(true, true, false, false));
        f.register(10);
        f.register(20);
        f.register(30);
        f.end_frame();
        assert!(f.is_focused(30));
    }

    #[test]
    fn tab_with_nothing_focused_picks_ends() {
        // Forward → first.
        let mut f = FocusState::new();
        f.begin_frame(&input(true, false, false, false));
        f.register(7);
        f.register(8);
        f.register(9);
        f.end_frame();
        assert!(f.is_focused(7));

        // Backward → last.
        let mut g = FocusState::new();
        g.begin_frame(&input(true, true, false, false));
        g.register(7);
        g.register(8);
        g.register(9);
        g.end_frame();
        assert!(g.is_focused(9));
    }

    #[test]
    fn tab_with_empty_order_is_noop() {
        let mut f = FocusState::new();
        f.focus(1);
        f.begin_frame(&input(true, false, false, false));
        // No focusables registered this frame.
        f.end_frame();
        assert!(f.is_focused(1));
    }
}
