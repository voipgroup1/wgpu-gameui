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

pub mod layout;
pub mod render;
mod theme;
mod widgets;

pub use render::{NineSliceMeta, SpriteAtlas, SpriteId, UiRenderer};
pub use theme::Theme;
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
    }

    /// Check if a rectangle is hovered
    pub fn is_hovered(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        self.mouse_x >= x && self.mouse_x < x + width &&
        self.mouse_y >= y && self.mouse_y < y + height
    }
}
