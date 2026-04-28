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

pub use text::{FontSystemHandle, TextBlock, TextMeasurer, TextRenderer, shared_font_system};

pub mod affine;
pub mod layer;
pub mod layout;
pub mod render;
mod theme;
mod ui_context;
mod widgets;

pub use affine::Affine2;
pub use layer::{Layer, LayerKind, LayerStack};
pub use render::{NineSliceMeta, SpriteAtlas, SpriteId, UiRenderer};
pub use theme::Theme;
pub use ui_context::{AlignH, AlignV, UiContext};
pub use widgets::*;

/// Input state passed to UI for interaction.
#[derive(Default, Clone)]
pub struct InputState {
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub mouse_down: bool,
    pub mouse_clicked: bool,
    pub mouse_released: bool,
    /// Scroll wheel delta (positive = scroll up, negative = scroll down)
    pub scroll_delta: f32,
    // Text input
    pub text_input: String,
    pub backspace_pressed: bool,
    pub enter_pressed: bool,
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
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear per-frame state (clicked, released, text input, scroll)
    pub fn end_frame(&mut self) {
        self.mouse_clicked = false;
        self.mouse_released = false;
        self.scroll_delta = 0.0;
        self.text_input.clear();
        self.backspace_pressed = false;
        self.enter_pressed = false;
        // `mouse_consumed` is a per-layer dispatch flag, not a per-frame
        // input event — it's set by the layer system every frame, so we
        // clear it here too for cleanliness.
        self.mouse_consumed = false;
        self.scroll_consumed = false;
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
            scroll_delta: 0.0,
            text_input: String::new(),
            backspace_pressed: false,
            enter_pressed: false,
            ..self.clone()
        }
    }
}
