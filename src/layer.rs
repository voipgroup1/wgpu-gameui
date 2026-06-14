//! Z-ordered layer stack for modals, popups, and tooltips.
//!
//! A `LayerStack` owns a base `DrawList` plus an ordered queue of additional
//! `Layer` entries pushed during a frame. Each `Layer` has a kind (Modal,
//! Popup, Tooltip) and its own `DrawList`. The renderer consumes them in
//! order: base first, then each pushed layer in push order.
//!
//! ```ignore
//! let mut layers = LayerStack::with_font_system(font_system);
//!
//! // 1. Push every layer (base + overlays) onto the stack first so we know
//! //    the FULL z-order before resolving input. Drawing into them can
//! //    happen in any order — only the eventual `render_layers` call cares
//! //    about render order, and it walks `layers.layers()` itself.
//! //
//! //    A common pattern: push the modal early (with its rect), draw into
//! //    the base layer with the dispatched input, then come back and draw
//! //    into the modal layer with its own dispatched input.
//!
//! let modal_idx = if modal_open {
//!     Some(layers.push_modal(modal_rect))
//! } else {
//!     None
//! };
//!
//! // 2. Dispatch input. `input_for_base` returns a clone of the raw input
//! //    where `mouse_consumed = true` if any open layer above is gobbling
//! //    the cursor — base widgets that read this clone won't fire while
//! //    the modal is up.
//! let base_input = layers.input_for_base(&raw_input);
//! draw_base_widgets(layers.base_mut(), &base_input);
//!
//! // 3. Each higher layer gets its own dispatch view via `input_for_layer`.
//! //    Layers above it (if any) can still consume input on top of it.
//! if let Some(idx) = modal_idx {
//!     let modal_input = layers.input_for_layer(idx, &raw_input);
//!     draw_modal_contents(&mut layers.layers_mut()[idx].list, &modal_input);
//!     layers.pop_layer();
//! }
//!
//! // 4. Render
//! ui_renderer.render_layers(..., &layers);
//! ```
//!
//! ## Input gobbling
//!
//! Modal layers cover all lower layers — when the mouse is anywhere on screen
//! and a modal is open, lower layers see `InputState::mouse_consumed = true`
//! unless the cursor is over *no* modal at all (in which case the modal
//! itself captures the click for "click-outside-to-close" semantics).
//!
//! Popups behave like modals but only block input within their own bounding
//! rect; clicks outside the popup pass through to lower layers.
//!
//! Tooltips never block input — they're purely visual.

use crate::InputState;
use crate::layout::Rect;
use crate::text::FontSystemHandle;
use crate::widgets::DrawList;

/// What kind of overlay this layer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// Always-on-top, no input passes through to lower layers.
    Modal,
    /// Blocks input only within its own rect (e.g. dropdown menu).
    Popup,
    /// Visual only — never blocks input.
    Tooltip,
}

/// A single overlay layer.
pub struct Layer {
    pub kind: LayerKind,
    /// Bounding rect used for popup hit-testing. For modals this is the modal
    /// dialog rect — clicks *outside* it route to the modal layer (which can
    /// implement click-outside-to-close), not to lower layers. For popups,
    /// clicks inside it stay in the popup; clicks outside fall through.
    pub rect: Rect,
    pub list: DrawList,
}

/// Ordered collection of layers for one frame.
pub struct LayerStack {
    base: DrawList,
    layers: Vec<Layer>,
    /// Stack of indices into `layers`, recording the active layer when nested
    /// `push_*` calls happened. Top of the stack is the layer current draw
    /// commands route to. Empty = base list is current.
    active: Vec<usize>,
    font_system: Option<FontSystemHandle>,
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerStack {
    pub fn new() -> Self {
        Self {
            base: DrawList::new(),
            layers: Vec::new(),
            active: Vec::new(),
            font_system: None,
        }
    }

    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self {
            base: DrawList::with_font_system(font_system.clone()),
            layers: Vec::new(),
            active: Vec::new(),
            font_system: Some(font_system),
        }
    }

    /// Reset all layers but keep allocated capacity.
    pub fn clear(&mut self) {
        self.base.clear();
        self.layers.clear();
        self.active.clear();
    }

    /// Borrow the base draw list (lowest layer).
    pub fn base(&self) -> &DrawList {
        &self.base
    }

    pub fn base_mut(&mut self) -> &mut DrawList {
        &mut self.base
    }

    /// All layers, in render order.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Mutable access to all layers (in render order). Useful for drawing
    /// into a specific layer's `list` after dispatching `input_for_layer`.
    pub fn layers_mut(&mut self) -> &mut [Layer] {
        &mut self.layers
    }

    /// The draw list that primitive calls should currently target — the
    /// most-recently-pushed layer, or the base list if no layer is active.
    pub fn current_mut(&mut self) -> &mut DrawList {
        match self.active.last().copied() {
            Some(idx) => &mut self.layers[idx].list,
            None => &mut self.base,
        }
    }

    fn make_list(&self) -> DrawList {
        match &self.font_system {
            Some(fs) => DrawList::with_font_system(fs.clone()),
            None => DrawList::new(),
        }
    }

    /// Open a modal layer covering `rect` (typically the modal dialog rect).
    /// Returns the layer index for the caller's records.
    pub fn push_modal(&mut self, rect: Rect) -> usize {
        let list = self.make_list();
        self.layers.push(Layer {
            kind: LayerKind::Modal,
            rect,
            list,
        });
        let idx = self.layers.len() - 1;
        self.active.push(idx);
        idx
    }

    /// Open a popup layer with the given bounding rect.
    pub fn push_popup(&mut self, rect: Rect) -> usize {
        let list = self.make_list();
        self.layers.push(Layer {
            kind: LayerKind::Popup,
            rect,
            list,
        });
        let idx = self.layers.len() - 1;
        self.active.push(idx);
        idx
    }

    /// Open a tooltip layer (purely visual).
    pub fn push_tooltip(&mut self, rect: Rect) -> usize {
        let list = self.make_list();
        self.layers.push(Layer {
            kind: LayerKind::Tooltip,
            rect,
            list,
        });
        let idx = self.layers.len() - 1;
        self.active.push(idx);
        idx
    }

    /// Pop the most recently opened layer.
    pub fn pop_layer(&mut self) {
        debug_assert!(
            !self.active.is_empty(),
            "LayerStack::pop_layer called with no active layer"
        );
        self.active.pop();
    }

    /// Returns whether any layer is currently active (i.e. inside a
    /// push/pop block). Useful for debug asserts at end-of-frame.
    pub fn has_active_layer(&self) -> bool {
        !self.active.is_empty()
    }

    /// Resolve the input state that a layer at `index` should see, taking
    /// into account higher-z modals/popups that may have captured the cursor.
    ///
    /// Layers higher in the stack are checked first. A higher-z **modal**
    /// always consumes input for everything below it. A higher-z **popup**
    /// consumes input only when the cursor is inside its rect. **Tooltips**
    /// never consume input.
    pub fn input_for_layer(&self, index: usize, base: &InputState) -> InputState {
        for higher in self.layers.iter().skip(index + 1) {
            match higher.kind {
                LayerKind::Modal => return base.consumed(),
                LayerKind::Popup => {
                    if higher.rect.contains(base.mouse_x, base.mouse_y) {
                        return base.consumed();
                    }
                }
                LayerKind::Tooltip => {}
            }
        }
        base.clone()
    }

    /// Resolve the input state the base layer should see.
    pub fn input_for_base(&self, base: &InputState) -> InputState {
        for higher in &self.layers {
            match higher.kind {
                LayerKind::Modal => return base.consumed(),
                LayerKind::Popup => {
                    if higher.rect.contains(base.mouse_x, base.mouse_y) {
                        return base.consumed();
                    }
                }
                LayerKind::Tooltip => {}
            }
        }
        base.clone()
    }
}

impl Drop for LayerStack {
    fn drop(&mut self) {
        debug_assert!(
            self.active.is_empty(),
            "LayerStack dropped with {} unbalanced push/pop pair(s)",
            self.active.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            mouse_down: true,
            scroll_delta: 1.0,
            ..InputState::default()
        }
    }

    #[test]
    fn current_mut_routes_to_base_when_no_layers() {
        let mut s = LayerStack::new();
        // A translate-only quad records one SDF chrome instance (not soup).
        s.current_mut().quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(s.base().chrome_instances.len(), 1);
    }

    #[test]
    fn current_mut_routes_to_top_layer() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 100.0, 100.0));
        s.current_mut().quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(s.base().chrome_instances.len(), 0);
        assert_eq!(s.layers()[0].list.chrome_instances.len(), 1);
        s.pop_layer();
    }

    #[test]
    fn modal_consumes_input_for_lower_layers() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(10.0, 10.0, 50.0, 50.0));
        s.pop_layer();

        let inp = input_at(0.0, 0.0); // outside modal rect
        let base_in = s.input_for_base(&inp);
        assert!(base_in.mouse_consumed);
        assert!(!base_in.mouse_clicked);
        assert_eq!(base_in.scroll_delta, 0.0);
    }

    #[test]
    fn popup_only_consumes_input_inside_its_rect() {
        let mut s = LayerStack::new();
        s.push_popup(Rect::new(100.0, 100.0, 50.0, 50.0));
        s.pop_layer();

        // Outside popup -> not consumed.
        let outside = input_at(0.0, 0.0);
        assert!(!s.input_for_base(&outside).mouse_consumed);

        // Inside popup -> consumed.
        let inside = input_at(120.0, 120.0);
        assert!(s.input_for_base(&inside).mouse_consumed);
    }

    #[test]
    fn tooltip_never_consumes_input() {
        let mut s = LayerStack::new();
        s.push_tooltip(Rect::new(0.0, 0.0, 200.0, 200.0));
        s.pop_layer();
        let inp = input_at(50.0, 50.0);
        assert!(!s.input_for_base(&inp).mouse_consumed);
    }

    #[test]
    fn nested_push_pop_balanced() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.push_popup(Rect::new(5.0, 5.0, 5.0, 5.0));
        s.pop_layer();
        s.pop_layer();
        assert!(!s.has_active_layer());
    }

    #[test]
    fn layer_below_popup_unaffected_outside_popup_rect() {
        let mut s = LayerStack::new();
        // First layer (index 0) — modal-ish thing
        let _ = s.push_modal(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        s.pop_layer();
        // Second layer (index 1) — popup
        let _ = s.push_popup(Rect::new(500.0, 500.0, 50.0, 50.0));
        s.pop_layer();

        // Layer 0 (modal): when computing input for it, only layers above
        // (the popup at index 1) may consume. Cursor at (0,0) is outside the
        // popup -> layer 0 still gets full input.
        let inp = input_at(0.0, 0.0);
        let layer0 = s.input_for_layer(0, &inp);
        assert!(!layer0.mouse_consumed);

        // But base layer is below the modal, so it IS consumed.
        let base = s.input_for_base(&inp);
        assert!(base.mouse_consumed);
    }

    #[test]
    fn full_frame_modal_dispatch_routes_input_correctly() {
        // Simulates: push modal, then ask `input_for_base` and
        // `input_for_layer` for a single raw input. The base view must be
        // marked consumed; the modal view must NOT be (no higher layer is
        // above it), so the modal's "close button" hit-test would fire.
        let mut s = LayerStack::new();
        let modal_idx = s.push_modal(Rect::new(100.0, 100.0, 200.0, 200.0));

        // Mouse over a "base button" rect at (10,10)-(40,40).
        let raw = InputState {
            mouse_x: 25.0,
            mouse_y: 25.0,
            mouse_clicked: true,
            mouse_down: true,
            scroll_delta: -2.0,
            ..InputState::default()
        };

        let base_view = s.input_for_base(&raw);
        // Base layer must see the input as consumed -> a base hit-test would
        // not fire.
        assert!(base_view.mouse_consumed);
        assert!(!base_view.mouse_clicked);
        assert_eq!(base_view.scroll_delta, 0.0);
        assert!(base_view.scroll_consumed);

        // The modal layer is the topmost; nothing above it consumes input.
        let modal_view = s.input_for_layer(modal_idx, &raw);
        assert!(!modal_view.mouse_consumed);
        assert!(modal_view.mouse_clicked);
        assert_eq!(modal_view.scroll_delta, -2.0);
        assert!(!modal_view.scroll_consumed);

        s.pop_layer();
    }

    #[test]
    #[should_panic]
    fn drop_with_unbalanced_layer_panics_in_debug() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 1.0, 1.0));
        // forgot to pop -> drop fires debug_assert
    }
}
