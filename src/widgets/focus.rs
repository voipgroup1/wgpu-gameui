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
//! ## Layer-scoped Tab trapping
//!
//! Call [`register_layer`](FocusState::register_layer) for focusables inside
//! modal/popup layers and pass the layer index to
//! [`end_frame`](FocusState::end_frame) — Tab cycling is then scoped to that
//! layer's ring exclusively, preventing Tab from reaching base-layer widgets
//! while a modal is open. Click-to-focus already respects `mouse_consumed`,
//! so a modal won't mis-focus a base-layer widget on click either.

use crate::InputState;
use crate::layout::Rect;

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
///
/// ## Layer-scoped Tab trapping
///
/// When a modal or popup layer is active, Tab cycling should be scoped to that
/// layer's focusables instead of the entire base layer. Callers register
/// layer-specific focusables via [`register_layer`](Self::register_layer), then
/// pass the active layer index to [`end_frame`](Self::end_frame):
///
/// ```ignore
/// // Base layer focusables
/// focus.register(INPUT_1);
/// focus.register(INPUT_2);
///
/// // Modal layer focusables
/// focus.register_layer(MODAL_INPUT, modal_idx);
///
/// // End frame with active layer — Tab cycles only within modal scope.
/// focus.end_frame(Some(modal_idx));
/// ```
#[derive(Debug, Default, Clone)]
pub struct FocusState {
    /// The widget that currently holds focus, if any.
    focused: Option<FocusId>,
    /// Focusables registered this frame for the base layer (no layer index).
    order: Vec<FocusId>,
    /// Focusables registered per layer index (for modal/popup trapping).
    /// Indexed by the layer index from [`LayerStack`].
    layer_orders: Vec<Vec<FocusId>>,
    /// Tab direction captured this frame: `+1` forward, `-1` backward, `0` none.
    tab: i32,
    /// Escape was pressed this frame.
    escape: bool,
    /// A click happened this frame (from the dispatched input).
    mouse_clicked: bool,
    /// A focusable claimed this frame's click (suppresses click-elsewhere blur).
    click_claimed: bool,
    /// Set by the focused text widget during draw: it wants IME composition and
    /// here is its caret rect (screen coords) for candidate-window placement.
    /// `None` means no text field is focused this frame, so the windowing layer
    /// should disable IME — preventing an active IME from swallowing keystrokes
    /// (e.g. WASD) while the user isn't editing text. Drained each frame via
    /// [`take_ime_request`](FocusState::take_ime_request).
    ime_request: Option<Rect>,
}

impl FocusState {
    /// A fresh focus owner with nothing focused.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a frame: clear the draw-order ring and capture this frame's
    /// navigation edges from `input` — [`nav.next`]/[`nav.prev`] cycle focus
    /// (mapped from Tab / Shift+Tab by default) and [`nav.cancel`] blurs (Escape)
    /// — plus the click edge. Call once per frame, before drawing the focusable
    /// widgets, with the same input those widgets receive (its `nav` intents must
    /// already be populated by the frame's [`NavMap`](crate::NavMap)).
    ///
    /// [`nav.next`]: crate::NavInput::next
    /// [`nav.prev`]: crate::NavInput::prev
    /// [`nav.cancel`]: crate::NavInput::cancel
    pub fn begin_frame(&mut self, input: &InputState) {
        self.order.clear();
        self.tab = if input.nav.next {
            1
        } else if input.nav.prev {
            -1
        } else {
            0
        };
        self.escape = input.nav.cancel;
        self.mouse_clicked = input.mouse_clicked;
        self.ime_request = None;
        self.click_claimed = false;
    }

    /// Register `id` as focusable in the base layer (no layer scope).
    /// Establishes its position in the Tab ring (draw order).
    /// Every focusable calls this each frame it is drawn.
    pub fn register(&mut self, id: FocusId) {
        self.order.push(id);
    }

    /// Register `id` as focusable inside a specific layer (modal/popup).
    /// Focusables registered here are only reachable via Tab when that layer
    /// is the active layer passed to [`end_frame`](Self::end_frame).
    pub fn register_layer(&mut self, id: FocusId, layer: usize) {
        while self.layer_orders.len() <= layer {
            self.layer_orders.push(Vec::new());
        }
        self.layer_orders[layer].push(id);
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

    /// Declare that the focused text widget wants IME composition this frame,
    /// passing its caret rect (screen coords) for IME candidate placement.
    /// Called by the text widget during draw while it holds focus. The
    /// windowing layer drains this with [`take_ime_request`](Self::take_ime_request)
    /// to gate `set_ime_allowed`, so an active IME only intercepts keys while a
    /// text field is actually being edited.
    pub fn request_ime(&mut self, caret_rect: Rect) {
        self.ime_request = Some(caret_rect);
    }

    /// Peek this frame's IME request (the focused text field's caret rect), if
    /// any. Non-consuming; see [`take_ime_request`](Self::take_ime_request).
    pub fn ime_request(&self) -> Option<Rect> {
        self.ime_request
    }

    /// Take and clear this frame's IME request. The windowing layer calls this
    /// once per frame after drawing the UI: `Some(rect)` → enable IME and point
    /// its candidate window at `rect`; `None` → disable IME. Self-clearing, so
    /// it reverts to "no IME" the instant no text field reports one — correct
    /// even when [`begin_frame`](Self::begin_frame) isn't being driven.
    pub fn take_ime_request(&mut self) -> Option<Rect> {
        self.ime_request.take()
    }

    /// End a frame: resolve Escape, click-elsewhere blur, and Tab navigation.
    ///
    /// `active_layer` scopes Tab cycling to a specific modal/popup layer's
    /// focusables (registered via [`register_layer`](Self::register_layer)).
    /// Pass `None` for the base layer (Tab cycles through all base-layer
    /// focusables as before).
    ///
    /// Call once per frame, after drawing the focusable widgets.
    pub fn end_frame(&mut self, active_layer: Option<usize>) {
        // Escape blurs.
        if self.escape {
            self.focused = None;
        }
        // A click that no focusable claimed blurs the current focus.
        if self.mouse_clicked && !self.click_claimed {
            self.focused = None;
        }
        // Tab / Shift+Tab cycle through the active layer's focusables only.
        // If no layer-specific order exists, fall back to the base order.
        if self.tab != 0 {
            let order = active_layer
                .and_then(|idx| self.layer_orders.get(idx))
                .filter(|o| !o.is_empty())
                .map(|o| o.as_slice())
                .unwrap_or(&self.order[..]);
            if !order.is_empty() {
                let len = order.len() as i32;
                let next = match self
                    .focused
                    .and_then(|f| order.iter().position(|&x| x == f))
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
                self.focused = Some(order[next]);
            }
        }
        // Clear all orders for the next frame.
        self.order.clear();
        self.layer_orders.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `InputState` with the focus-relevant edges set, mapped through the
    /// default keyboard binding so `nav` intents (which `begin_frame` reads) are
    /// populated — Tab → `nav.next`, Shift+Tab → `nav.prev`, Escape → `nav.cancel`.
    fn input(tab: bool, shift: bool, escape: bool, clicked: bool) -> InputState {
        let mut s = InputState {
            key_tab: tab,
            shift_pressed: shift,
            key_escape: escape,
            mouse_clicked: clicked,
            ..Default::default()
        };
        crate::map_keyboard(&mut s);
        s
    }

    #[test]
    fn fresh_state_has_no_focus() {
        let f = FocusState::new();
        assert_eq!(f.focused(), None);
        assert!(!f.is_focused(0));
        assert_eq!(f.ime_request(), None, "no IME requested by default");
    }

    #[test]
    fn request_ime_is_peekable_then_taken() {
        let mut f = FocusState::new();
        let rect = Rect::new(10.0, 20.0, 1.5, 18.0);
        f.request_ime(rect);
        // Peek doesn't consume.
        assert_eq!(f.ime_request(), Some(rect));
        assert_eq!(f.ime_request(), Some(rect));
        // Take consumes.
        assert_eq!(f.take_ime_request(), Some(rect));
        assert_eq!(f.take_ime_request(), None, "second take is empty");
    }

    #[test]
    fn begin_frame_clears_stale_ime_request() {
        let mut f = FocusState::new();
        f.request_ime(Rect::new(1.0, 2.0, 1.5, 12.0));
        // A new frame with no text field drawn must not leave a stale request:
        // begin_frame clears it, and nothing re-requests it this frame.
        f.begin_frame(&input(false, false, false, false));
        f.end_frame(None);
        assert_eq!(
            f.take_ime_request(),
            None,
            "stale IME request cleared when no field is focused"
        );
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
        f.end_frame(None);
        assert!(f.is_focused(2)); // claimed click → no blur
    }

    #[test]
    fn click_elsewhere_blurs() {
        let mut f = FocusState::new();
        f.focus(9);
        // A click occurs, but no focusable requests it.
        f.begin_frame(&input(false, false, false, true));
        f.register(9);
        f.end_frame(None);
        assert_eq!(f.focused(), None);
    }

    #[test]
    fn claimed_click_does_not_blur() {
        let mut f = FocusState::new();
        f.focus(9);
        f.begin_frame(&input(false, false, false, true));
        f.register(9);
        f.request(9); // the focused widget itself was clicked
        f.end_frame(None);
        assert!(f.is_focused(9));
    }

    #[test]
    fn no_click_keeps_focus() {
        let mut f = FocusState::new();
        f.focus(3);
        f.begin_frame(&input(false, false, false, false));
        f.register(3);
        f.end_frame(None);
        assert!(f.is_focused(3));
    }

    #[test]
    fn escape_blurs() {
        let mut f = FocusState::new();
        f.focus(5);
        f.begin_frame(&input(false, false, true, false));
        f.register(5);
        f.end_frame(None);
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
        f.end_frame(None);
        assert!(f.is_focused(20));
        // Frame 2: Tab forward 20 -> 30.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register(30);
        f.end_frame(None);
        assert!(f.is_focused(30));
        // Frame 3: Tab forward 30 -> wrap to 10.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register(30);
        f.end_frame(None);
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
        f.end_frame(None);
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
        f.end_frame(None);
        assert!(f.is_focused(7));

        // Backward → last.
        let mut g = FocusState::new();
        g.begin_frame(&input(true, true, false, false));
        g.register(7);
        g.register(8);
        g.register(9);
        g.end_frame(None);
        assert!(g.is_focused(9));
    }

    #[test]
    fn tab_with_empty_order_is_noop() {
        let mut f = FocusState::new();
        f.focus(1);
        f.begin_frame(&input(true, false, false, false));
        // No focusables registered this frame.
        f.end_frame(None);
        assert!(f.is_focused(1));
    }

    // ---- Layer-scoped Tab trapping ----

    #[test]
    fn tab_with_active_layer_cycles_within_layer_only() {
        let mut f = FocusState::new();
        f.focus(10);
        // Register base-layer focusables.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        // Register layer-2 focusables (simulating modal at index 2).
        f.register_layer(100, 2);
        f.register_layer(200, 2);
        // End frame with active layer = 2 — Tab should only cycle through
        // [100, 200], ignoring 10 and 20.
        f.end_frame(Some(2));
        assert!(
            f.is_focused(100),
            "Tab jumped to first focusable in layer 2"
        );

        // Next frame: Tab again, stays within layer 2.
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register_layer(100, 2);
        f.register_layer(200, 2);
        f.end_frame(Some(2));
        assert!(
            f.is_focused(200),
            "Tab cycled to second focusable in layer 2"
        );

        // Next frame: Tab goes back to 100 (wraps within layer 2).
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register_layer(100, 2);
        f.register_layer(200, 2);
        f.end_frame(Some(2));
        assert!(f.is_focused(100), "Tab wrapped within layer 2");
    }

    #[test]
    fn tab_without_active_layer_uses_base_order() {
        // Same setup as above but end_frame(None) — should cycle through
        // the full base order, not the layer-specific one.
        let mut f = FocusState::new();
        f.focus(10);
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        f.register_layer(100, 2);
        f.register_layer(200, 2);
        f.end_frame(None);
        assert!(
            f.is_focused(20),
            "Tab cycled within base order, ignoring layer"
        );
    }

    #[test]
    fn tab_active_layer_with_no_focusables_falls_back_to_base() {
        // active_layer points to a layer index that has no focusables.
        let mut f = FocusState::new();
        f.focus(10);
        f.begin_frame(&input(true, false, false, false));
        f.register(10);
        f.register(20);
        // Layer 99 has nothing registered.
        f.end_frame(Some(99));
        assert!(
            f.is_focused(20),
            "falls back to base order when layer has no focusables"
        );
    }

    #[test]
    fn shift_tab_with_active_layer_wraps_backward() {
        let mut f = FocusState::new();
        f.focus(200);
        f.begin_frame(&input(true, true, false, false)); // Shift+Tab
        f.register(999); // base — ignored
        f.register_layer(100, 1);
        f.register_layer(200, 1);
        f.end_frame(Some(1));
        assert!(f.is_focused(100), "Shift+Tab wraps backward within layer 1");
    }

    #[test]
    fn click_in_base_layer_does_not_claim_layer_focus() {
        // A click in the base layer should not be claimed by a layer focusable.
        let mut f = FocusState::new();
        f.focus(100);
        f.begin_frame(&input(false, false, false, true)); // clicked = true
        f.register(10);
        f.register_layer(100, 2);
        f.register_layer(200, 2);
        // No request comes in — end_frame should blur due to unclaimed click.
        f.end_frame(Some(2));
        assert_eq!(
            f.focused(),
            None,
            "unclaimed click blurs even with active layer"
        );
    }
}
