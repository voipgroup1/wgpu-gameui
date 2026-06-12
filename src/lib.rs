//! wgpu-gameui - Custom wgpu-based game UI library.
//!
//! Provides polished UI widgets for wgpu applications.
//! Designed to replace egui for game UIs where aesthetics matter.
//!
//! # Layout System
//!
//! Use anchors and stacks for positioning:
//!
//! ```ignore
//! use wgpu_gameui::layout::*;
//!
//! let floor_ui = Positioned::new(
//!     Anchor::TopRight { offset: (-10.0, 10.0) },
//!     Size::fixed(80.0, 120.0),
//!     VStack::new(8.0)
//!         .child(30.0, 60.0)  // up button
//!         .child(24.0, 60.0)  // floor label
//!         .child(30.0, 60.0), // down button
//! );
//!
//! let result = floor_ui.layout_screen(1280.0, 720.0);
//! // result.get(0) = container rect
//! // result.get(1) = up button rect
//! // result.get(2) = floor label rect
//! // result.get(3) = down button rect
//! ```

mod text;

pub use text::{
    CaretPos, FontHandle, FontSystemHandle, TextAlign, TextBlock, TextGlow, TextMeasurer,
    TextOutline, TextRenderer, TextShadow, TextSpan, WrapMode, byte_at_point,
    byte_on_adjacent_line, caret_for_byte, load_font_bytes, load_font_file,
    register_bundled_fonts, resolve_span_color, shared_font_system, text_caret_layout,
    text_cursor_positions,
};

/// Font weight and style selectors (re-exported from `glyphon`/`cosmic-text`)
/// for `TextBlock::with_weight`/`with_style` and the `UiContext` font stack.
pub use glyphon::{Style, Weight};

pub mod affine;
mod click_tracker;
mod drag_tracker;
pub mod layer;
pub mod layout;
pub mod projection;
pub mod render;
mod theme;
mod ui_context;
mod widgets;

pub use affine::Affine2;
pub use click_tracker::{ClickTracker, DEFAULT_DOUBLE_CLICK_THRESHOLD, DEFAULT_HOLD_THRESHOLD};
pub use drag_tracker::{DragTracker, DEFAULT_DRAG_THRESHOLD};
pub use projection::{world_to_screen, world_to_screen_na};
pub use layer::{Layer, LayerKind, LayerStack};
pub use render::{NineSliceMeta, SpriteAtlas, SpriteId, UiRenderer};
pub use theme::Theme;
pub use ui_context::{AlignH, AlignV, FontSpec, UiContext, UiState};
pub use widgets::*;

/// Input state passed to UI for interaction.
#[derive(Default, Clone)]
pub struct InputState {
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub mouse_down: bool,
    pub mouse_clicked: bool,
    pub mouse_released: bool,
    /// True on the frame of a double-click (two presses of the primary button
    /// within the double-click threshold). Computed by [`ClickTracker::update`];
    /// a per-frame edge event (cleared by `end_frame`, zeroed by `consumed()`).
    /// `mouse_clicked` is also true on that same frame (the double-click press
    /// is still a click), so widgets that don't care about double-click don't
    /// need to change.
    pub mouse_double_clicked: bool,
    /// True while the primary button has been held past the hold threshold.
    /// Computed by [`ClickTracker::update`]. Latches `true` until release; the
    /// tracker re-asserts it each frame via `update`, so `end_frame` may safely
    /// clear it.
    pub mouse_held: bool,
    // ---- Right mouse button (context menus, alternate actions) ----
    /// Right button is currently held.
    pub mouse_right_down: bool,
    /// Right button went down this frame (press edge).
    pub mouse_right_clicked: bool,
    /// Right button went up this frame (release edge).
    pub mouse_right_released: bool,
    // ---- Middle mouse button (e.g. pan, close tab) ----
    /// Middle button is currently held.
    pub mouse_middle_down: bool,
    /// Middle button went down this frame (press edge).
    pub mouse_middle_clicked: bool,
    /// Middle button went up this frame (release edge).
    pub mouse_middle_released: bool,
    /// True while the pointer is dragging: the button has been held since a
    /// press and moved past the drag threshold. Computed each frame by
    /// [`DragTracker::update`]; defaults to `false`. Latches until release, so
    /// a drag that momentarily stops moving stays `true`. Widgets read this to
    /// distinguish a drag gesture from a click.
    pub is_dragging: bool,
    /// Per-frame pointer movement `[dx, dy]` (logical px) while [`is_dragging`].
    /// `[0, 0]` when not dragging (including sub-threshold jitter on a click).
    /// Computed by [`DragTracker::update`].
    ///
    /// [`is_dragging`]: InputState::is_dragging
    pub drag_delta: [f32; 2],
    /// Scroll wheel delta (positive = scroll up, negative = scroll down)
    pub scroll_delta: f32,
    // Text input
    pub text_input: String,
    pub backspace_pressed: bool,
    pub enter_pressed: bool,
    /// The in-progress IME composition ("preedit") string, shown inline and
    /// underlined inside the focused text field but NOT yet part of its value.
    /// `""` when not composing. Set by the windowing layer on `Ime::Preedit`;
    /// cleared (to `""`) on `Ime::Commit`/`Ime::Disabled`. This is held state
    /// (like the modifier flags), so [`InputState::end_frame`] does NOT clear
    /// it — the windowing layer owns its lifetime. Commit reuses the normal
    /// [`InputState::text_input`] insertion path.
    pub preedit: String,
    /// Byte range `[start, end]` within [`InputState::preedit`] that the IME
    /// marks as its cursor/selection (winit's `Ime::Preedit` second field). The
    /// inline caret is drawn at `start`. `None` → caret at the end of the
    /// preedit.
    pub preedit_cursor: Option<[usize; 2]>,
    /// True when the layer dispatcher has decided this input is already
    /// consumed by a higher layer (modal, popup, etc.). Widgets must treat
    /// this as if the mouse is not over them and not clicking. Widgets that
    /// use [`InputState::is_hovered`] get this for free; widgets that test
    /// `rect.contains(mouse_x, mouse_y)` directly should AND that with
    /// `!input.mouse_consumed`.
    pub mouse_consumed: bool,
    /// True when the wheel delta has been claimed by an inner scrollable
    /// (e.g. a `ScrollView` that actually changed offset, or one at a clamp
    /// boundary that absorbed the input). Outer scrollables should skip
    /// applying `scroll_delta` when this is set so wheel events don't
    /// "bubble out" of an inner viewport when the cursor is over it.
    pub scroll_consumed: bool,
    // ---- Keyboard events (for text editing) ----
    /// Arrow Left was pressed this frame.
    pub key_left: bool,
    /// Arrow Right was pressed this frame.
    pub key_right: bool,
    /// Home key was pressed this frame.
    pub key_home: bool,
    /// End key was pressed this frame.
    pub key_end: bool,
    /// Delete key was pressed this frame.
    pub key_delete: bool,
    /// Tab key was pressed this frame. Drives focus navigation
    /// (Shift+Tab reverses via [`shift_pressed`]).
    pub key_tab: bool,
    /// Escape key was pressed this frame. Blurs the focused widget.
    pub key_escape: bool,
    /// Space key was pressed this frame.
    pub key_space: bool,
    /// Arrow Up was pressed this frame.
    pub key_up: bool,
    /// Arrow Down was pressed this frame.
    pub key_down: bool,
    /// Shift key is currently held.
    pub shift_pressed: bool,
    /// Ctrl (or Cmd on macOS) key is currently held.
    pub ctrl_pressed: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear per-frame state (clicked, released, text input, scroll, keyboard events)
    pub fn end_frame(&mut self) {
        self.mouse_clicked = false;
        self.mouse_released = false;
        // Click-tracker outputs; the tracker re-asserts them each update call.
        self.mouse_double_clicked = false;
        self.mouse_held = false;
        self.mouse_right_clicked = false;
        self.mouse_right_released = false;
        self.mouse_middle_clicked = false;
        self.mouse_middle_released = false;
        // Drag outputs are recomputed each frame by `DragTracker::update`; clear
        // them so a consumer that stops calling it (or never does) doesn't leave
        // a stale drag latched on.
        self.is_dragging = false;
        self.drag_delta = [0.0, 0.0];
        self.scroll_delta = 0.0;
        self.text_input.clear();
        self.backspace_pressed = false;
        self.enter_pressed = false;
        // `preedit`/`preedit_cursor` are deliberately NOT cleared here: the IME
        // composition is held state owned by the windowing layer (set on
        // `Ime::Preedit`, cleared on commit/disable), just like the modifiers.
        // `mouse_consumed` is a per-layer dispatch flag, not a per-frame
        // input event — it's set by the layer system every frame, so we
        // clear it here too for cleanliness.
        self.mouse_consumed = false;
        self.scroll_consumed = false;
        self.key_left = false;
        self.key_right = false;
        self.key_home = false;
        self.key_end = false;
        self.key_delete = false;
        self.key_tab = false;
        self.key_escape = false;
        self.key_space = false;
        self.key_up = false;
        self.key_down = false;
        // shift_pressed and ctrl_pressed are held-state, cleared by the
        // windowing layer on key-up; they persist across frames.
    }

    /// Check if a rectangle is hovered. Returns false when `mouse_consumed`
    /// is set (a higher-z layer captured the cursor).
    pub fn is_hovered(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        if self.mouse_consumed {
            return false;
        }
        self.mouse_x >= x && self.mouse_x < x + width &&
        self.mouse_y >= y && self.mouse_y < y + height
    }

    /// Return a clone of this input that lower layers should see when a
    /// higher layer has captured input. Sets `mouse_consumed = true` and
    /// also zeroes scroll/click state so widgets that don't check the
    /// flag still won't fire.
    pub fn consumed(&self) -> Self {
        Self {
            mouse_consumed: true,
            scroll_consumed: true,
            mouse_clicked: false,
            mouse_released: false,
            mouse_double_clicked: false,
            mouse_held: false,
            mouse_right_clicked: false,
            mouse_right_released: false,
            mouse_middle_clicked: false,
            mouse_middle_released: false,
            // A layer under a modal/popup must not see an in-progress drag.
            is_dragging: false,
            drag_delta: [0.0, 0.0],
            scroll_delta: 0.0,
            text_input: String::new(),
            backspace_pressed: false,
            enter_pressed: false,
            // A layer under a modal/popup must not see the composition either.
            preedit: String::new(),
            preedit_cursor: None,
            key_left: false,
            key_right: false,
            key_home: false,
            key_end: false,
            key_delete: false,
            key_tab: false,
            key_escape: false,
            key_space: false,
            key_up: false,
            key_down: false,
            // shift/ctrl are modifier state, not events — preserve them
            // so modals that have text inputs still see modifier keys.
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod input_state_tests {
    use super::InputState;

    fn right_pressed() -> InputState {
        InputState {
            mouse_right_down: true,
            mouse_right_clicked: true,
            ..InputState::default()
        }
    }

    fn middle_pressed() -> InputState {
        InputState {
            mouse_middle_down: true,
            mouse_middle_clicked: true,
            ..InputState::default()
        }
    }

    #[test]
    fn right_click_edge_cleared_by_end_frame() {
        let mut i = right_pressed();
        i.end_frame();
        assert!(!i.mouse_right_clicked);
        assert!(!i.mouse_right_released);
        // down-state not cleared by end_frame (it's held state)
        assert!(i.mouse_right_down);
    }

    #[test]
    fn middle_click_edge_cleared_by_end_frame() {
        let mut i = middle_pressed();
        i.end_frame();
        assert!(!i.mouse_middle_clicked);
        assert!(!i.mouse_middle_released);
        assert!(i.mouse_middle_down);
    }

    #[test]
    fn consumed_zeros_right_click_edges() {
        let i = right_pressed();
        let c = i.consumed();
        assert!(c.mouse_consumed);
        assert!(!c.mouse_right_clicked, "consumed must zero right click edge");
        assert!(!c.mouse_right_released);
        // held-state passes through so other logic can see the button is down
        assert!(c.mouse_right_down, "consumed preserves down-state");
    }

    #[test]
    fn consumed_zeros_middle_click_edges() {
        let i = middle_pressed();
        let c = i.consumed();
        assert!(!c.mouse_middle_clicked);
        assert!(!c.mouse_middle_released);
        assert!(c.mouse_middle_down);
    }

    #[test]
    fn right_release_edge_cleared_by_end_frame() {
        let mut i = InputState {
            mouse_right_released: true,
            ..InputState::default()
        };
        i.end_frame();
        assert!(!i.mouse_right_released);
    }

    // ---- IME preedit ----

    #[test]
    fn preedit_survives_end_frame() {
        // The composition is held state owned by the windowing layer, like the
        // modifiers — end_frame must NOT drop it mid-composition.
        let mut i = InputState {
            preedit: "ㄓㄨ".into(),
            preedit_cursor: Some([3, 3]),
            ..InputState::default()
        };
        i.end_frame();
        assert_eq!(i.preedit, "ㄓㄨ", "preedit must persist across end_frame");
        assert_eq!(i.preedit_cursor, Some([3, 3]));
    }

    #[test]
    fn consumed_clears_preedit() {
        // A layer beneath a modal must not see the composition.
        let i = InputState {
            preedit: "ㄓㄨ".into(),
            preedit_cursor: Some([3, 3]),
            ..InputState::default()
        };
        let c = i.consumed();
        assert!(c.preedit.is_empty(), "consumed must clear preedit");
        assert_eq!(c.preedit_cursor, None);
    }
}
