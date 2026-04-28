//! Teardown-style immediate-mode façade over `DrawList`.
//!
//! `UiContext` is a thin borrow over an existing `DrawList`. The transform and
//! tint stacks live on `DrawList` (so existing widget calls that take an
//! absolute `Rect` are transparently transform-aware); `UiContext` just adds
//! Teardown-flavoured verbs (`push`, `pop`, `translate`, `align`, `center`,
//! `color`, `color_filter`, `place_rect`) plus a per-stack-frame alignment.
//!
//! Pop is explicit. There is no `Drop`-based auto-pop, mirroring Teardown's
//! `UiPush`/`UiPop` semantics.

use crate::layer::{LayerKind, LayerStack};
use crate::layout::Rect;
use crate::text::TextBlock;
use crate::widgets::DrawList;

/// Horizontal alignment relative to the current origin.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignH {
    Left,
    Center,
    Right,
}

/// Vertical alignment relative to the current origin.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignV {
    Top,
    Middle,
    Bottom,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct AlignSpec {
    h: AlignH,
    v: AlignV,
}

impl AlignSpec {
    const DEFAULT: Self = Self {
        h: AlignH::Left,
        v: AlignV::Top,
    };

    /// Parse a Teardown-style align string. Accepts space-separated tokens in
    /// any order: `left|center|right` for horizontal and `top|middle|bottom`
    /// for vertical. Unknown tokens fall back to the previous component (and
    /// are returned in the second tuple element so the caller can surface a
    /// warning). Empty input returns `base` unchanged.
    fn parse(spec: &str, base: Self) -> (Self, Vec<String>) {
        let mut h = base.h;
        let mut v = base.v;
        let mut unknown = Vec::new();
        for token in spec.split_ascii_whitespace() {
            match token {
                "left" => h = AlignH::Left,
                "center" => h = AlignH::Center,
                "right" => h = AlignH::Right,
                "top" => v = AlignV::Top,
                "middle" | "center_v" => v = AlignV::Middle,
                "bottom" => v = AlignV::Bottom,
                other => unknown.push(other.to_string()),
            }
        }
        (Self { h, v }, unknown)
    }

    fn offset(&self, w: f32, h: f32) -> [f32; 2] {
        let x = match self.h {
            AlignH::Left => 0.0,
            AlignH::Center => -w * 0.5,
            AlignH::Right => -w,
        };
        let y = match self.v {
            AlignV::Top => 0.0,
            AlignV::Middle => -h * 0.5,
            AlignV::Bottom => -h,
        };
        [x, y]
    }
}

/// What `UiContext` is rendering into.
enum Backend<'a> {
    /// Plain draw list (no layer system; modal_begin/popup_begin will panic
    /// in debug if called).
    List(&'a mut DrawList),
    /// Full layer stack — modal_begin/popup_begin route here.
    Layers(&'a mut LayerStack),
}

impl<'a> Backend<'a> {
    fn list_mut(&mut self) -> &mut DrawList {
        match self {
            Backend::List(l) => l,
            Backend::Layers(s) => s.current_mut(),
        }
    }
}

/// Teardown-style façade over a `DrawList` or `LayerStack`. Owns no draw
/// state — borrows the backend for the duration of the build.
pub struct UiContext<'a> {
    backend: Backend<'a>,
    align_stack: Vec<AlignSpec>,
    /// Stack of layer kinds still open — used by Drop debug_assert, by
    /// modal_end / popup_end to verify the caller closed the right kind, and
    /// to detect unbalanced begin/end pairs. Length == number of open layers.
    open_layer_kinds: Vec<LayerKind>,
    /// Names of unknown align tokens we've already warned about, to keep one
    /// typo from spamming the log every frame.
    warned_align_tokens: std::collections::HashSet<String>,
}

impl<'a> UiContext<'a> {
    /// Wrap an existing `DrawList`. `modal_begin`/`popup_begin` will
    /// debug_assert when called on this variant — switch to
    /// [`UiContext::with_layers`] for full layer support.
    pub fn new(list: &'a mut DrawList) -> Self {
        Self {
            backend: Backend::List(list),
            align_stack: vec![AlignSpec::DEFAULT],
            open_layer_kinds: Vec::new(),
            warned_align_tokens: std::collections::HashSet::new(),
        }
    }

    /// Wrap a `LayerStack`. Enables `modal_begin`/`popup_begin`.
    pub fn with_layers(layers: &'a mut LayerStack) -> Self {
        Self {
            backend: Backend::Layers(layers),
            align_stack: vec![AlignSpec::DEFAULT],
            open_layer_kinds: Vec::new(),
            warned_align_tokens: std::collections::HashSet::new(),
        }
    }

    /// Push transform + tint + align (Teardown's `UiPush`).
    pub fn push(&mut self) {
        let list = self.backend.list_mut();
        list.push_transform();
        list.push_tint();
        let top = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        self.align_stack.push(top);
    }

    /// Pop transform + tint + align (Teardown's `UiPop`).
    pub fn pop(&mut self) {
        let list = self.backend.list_mut();
        list.pop_transform();
        list.pop_tint();
        if self.align_stack.len() > 1 {
            self.align_stack.pop();
        }
    }

    /// Shift the local origin (Teardown's `UiTranslate`).
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.backend.list_mut().translate(dx, dy);
    }

    /// Rotate the local coordinate frame (Teardown's `UiRotate` is in degrees;
    /// we take radians to match Rust convention. Use `f32::to_radians()` to
    /// convert from degrees at the call site).
    pub fn rotate(&mut self, angle_radians: f32) {
        self.backend.list_mut().rotate(angle_radians);
    }

    /// Non-uniform scale the local coordinate frame (Teardown's `UiScale`).
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.backend.list_mut().scale(sx, sy);
    }

    /// Set alignment for subsequent placement helpers (Teardown's `UiAlign`).
    pub fn align(&mut self, spec: &str) {
        let base = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let (new_spec, unknown) = AlignSpec::parse(spec, base);
        for token in unknown {
            if self.warned_align_tokens.insert(token.clone()) {
                log::warn!(
                    "wgpu-gameui: UiContext::align received unknown token '{}' \
                     (expected one of: left|center|right, top|middle|bottom) — \
                     ignoring",
                    token
                );
            }
        }
        if let Some(top) = self.align_stack.last_mut() {
            *top = new_spec;
        }
    }

    /// Shorthand for `align("center middle")` (Teardown's `UiCenter`).
    pub fn center(&mut self) {
        if let Some(top) = self.align_stack.last_mut() {
            *top = AlignSpec {
                h: AlignH::Center,
                v: AlignV::Middle,
            };
        }
    }

    /// Replace the current tint (Teardown's `UiColor`).
    pub fn color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.backend.list_mut().set_tint([r, g, b, a]);
    }

    /// Multiply into the current tint (Teardown's `UiColorFilter`).
    pub fn color_filter(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.backend.list_mut().multiply_tint([r, g, b, a]);
    }

    /// Return the current world-space cursor position (origin of the local
    /// frame after all active transforms).
    pub fn cursor(&mut self) -> [f32; 2] {
        self.backend
            .list_mut()
            .current_transform()
            .transform_point([0.0, 0.0])
    }

    /// Compute the world-space rect for a widget of the given local size at
    /// the current origin under the active alignment, then transform through
    /// the active affine.
    pub fn place_rect(&mut self, width: f32, height: f32) -> Rect {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(width, height);
        let local = Rect::new(ox, oy, width, height);
        self.backend
            .list_mut()
            .current_transform()
            .transform_rect_aabb(local)
    }

    /// Draw a colored quad of the given size at the aligned origin.
    pub fn quad(&mut self, w: f32, h: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend.list_mut().quad(ox, oy, w, h, color);
    }

    /// Draw a rounded rect of the given size at the aligned origin.
    pub fn rounded_rect(&mut self, w: f32, h: f32, radius: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend
            .list_mut()
            .rounded_rect(Rect::new(ox, oy, w, h), radius, color);
    }

    /// Draw a text block whose origin honours align/transform.
    pub fn text(&mut self, mut block: TextBlock) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let w = block.max_width;
        let h = block.line_height;
        let [ox, oy] = align.offset(w, h);
        block.x += ox;
        block.y += oy;
        self.backend.list_mut().text(block);
    }

    /// Direct access to the underlying `DrawList` (the currently active layer's
    /// list when running on a `LayerStack`).
    pub fn list(&mut self) -> &mut DrawList {
        self.backend.list_mut()
    }

    /// Open a modal layer covering `rect`. Subsequent draw calls go to the
    /// modal layer until `modal_end` is called. Lower layers receive
    /// `mouse_consumed = true` for input dispatch.
    ///
    /// Calling this on a `UiContext::new(DrawList)` (no layers) hits a
    /// `debug_assert!` — switch to `UiContext::with_layers` for modal support.
    pub fn modal_begin(&mut self, rect: Rect) {
        match &mut self.backend {
            Backend::List(_) => {
                debug_assert!(
                    false,
                    "UiContext::modal_begin requires a LayerStack backend; \
                     construct via UiContext::with_layers(...)"
                );
            }
            Backend::Layers(s) => {
                s.push_modal(rect);
                self.open_layer_kinds.push(LayerKind::Modal);
            }
        }
    }

    /// Close the most recent modal layer. Debug-asserts that the most-recent
    /// open layer was opened with `modal_begin`.
    pub fn modal_end(&mut self) {
        self.close_layer(LayerKind::Modal);
    }

    /// Open a popup layer with bounding `rect`. Clicks inside `rect` are
    /// captured (lower layers see `mouse_consumed`); clicks outside fall
    /// through.
    pub fn popup_begin(&mut self, rect: Rect) {
        match &mut self.backend {
            Backend::List(_) => {
                debug_assert!(
                    false,
                    "UiContext::popup_begin requires a LayerStack backend; \
                     construct via UiContext::with_layers(...)"
                );
            }
            Backend::Layers(s) => {
                s.push_popup(rect);
                self.open_layer_kinds.push(LayerKind::Popup);
            }
        }
    }

    /// Close the most recent popup layer. Debug-asserts that the most-recent
    /// open layer was opened with `popup_begin`.
    pub fn popup_end(&mut self) {
        self.close_layer(LayerKind::Popup);
    }

    fn close_layer(&mut self, expected: LayerKind) {
        match &mut self.backend {
            Backend::List(_) => {
                debug_assert!(
                    false,
                    "UiContext::*_end called on a UiContext that has no layer backend"
                );
            }
            Backend::Layers(s) => {
                let top = self.open_layer_kinds.last().copied();
                // Pop *before* asserting so a kind-mismatch panic doesn't
                // turn into a double-panic via Drop's balance check.
                if !self.open_layer_kinds.is_empty() {
                    s.pop_layer();
                    self.open_layer_kinds.pop();
                }
                debug_assert!(
                    top.is_some(),
                    "UiContext::*_end called with no open layer"
                );
                debug_assert!(
                    top == Some(expected),
                    "UiContext layer kind mismatch: expected to close a {:?}, but the most-recent open layer is a {:?}",
                    expected,
                    top
                );
            }
        }
    }
}

impl<'a> Drop for UiContext<'a> {
    /// Surfaces unbalanced `push`/`pop` calls in debug builds.
    fn drop(&mut self) {
        debug_assert_eq!(
            self.align_stack.len(),
            1,
            "UiContext dropped with {} unbalanced push/pop pair(s) on the align stack",
            self.align_stack.len() - 1
        );
        debug_assert_eq!(
            self.open_layer_kinds.len(),
            0,
            "UiContext dropped with {} unbalanced modal_begin/end or popup_begin/end pair(s)",
            self.open_layer_kinds.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn align_left_top_at_origin() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        let r = ui.place_rect(10.0, 20.0);
        assert_eq!(r, Rect::new(0.0, 0.0, 10.0, 20.0));
    }

    #[test]
    fn align_center_middle_centers_rect() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("center middle");
        let r = ui.place_rect(10.0, 10.0);
        assert_eq!(r, Rect::new(-5.0, -5.0, 10.0, 10.0));
    }

    #[test]
    fn align_right_bottom_offsets_rect() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("right bottom");
        let r = ui.place_rect(10.0, 20.0);
        assert_eq!(r, Rect::new(-10.0, -20.0, 10.0, 20.0));
    }

    #[test]
    fn translate_then_place_shifts() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.translate(100.0, 50.0);
        let r = ui.place_rect(10.0, 10.0);
        assert_eq!(r, Rect::new(100.0, 50.0, 10.0, 10.0));
    }

    #[test]
    fn scale_doubles_size_under_translate_only() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.scale(2.0, 2.0);
        let r = ui.place_rect(10.0, 10.0);
        assert!(approx(r.width, 20.0));
        assert!(approx(r.height, 20.0));
    }

    #[test]
    fn color_replaces_tint() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.color(0.5, 0.5, 0.5, 1.0);
        ui.color(0.25, 0.25, 0.25, 1.0);
        assert_eq!(ui.list().current_tint(), [0.25, 0.25, 0.25, 1.0]);
    }

    #[test]
    fn color_filter_multiplies_tint() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.color(0.5, 0.5, 0.5, 1.0);
        ui.color_filter(0.5, 0.5, 0.5, 1.0);
        let t = ui.list().current_tint();
        assert!(approx(t[0], 0.25));
        assert!(approx(t[1], 0.25));
        assert!(approx(t[2], 0.25));
        assert!(approx(t[3], 1.0));
    }

    #[test]
    fn push_pop_balances_align_too() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("center middle");
        ui.push();
        ui.align("right bottom");
        let r1 = ui.place_rect(10.0, 10.0);
        assert_eq!(r1, Rect::new(-10.0, -10.0, 10.0, 10.0));
        ui.pop();
        let r2 = ui.place_rect(10.0, 10.0);
        assert_eq!(r2, Rect::new(-5.0, -5.0, 10.0, 10.0));
    }

    #[test]
    fn cursor_returns_world_origin() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.translate(7.0, 11.0);
        ui.scale(2.0, 2.0);
        ui.translate(3.0, 4.0);
        let c = ui.cursor();
        // local (0,0) -> scale -> (0,0) -> translate(3,4) -> ... but that's
        // local-side. Composed: translate(7,11) * scale(2,2) * translate(3,4)
        // applied to (0,0) is translate(7,11) * scale(2,2) of (3,4) = (7+6, 11+8).
        assert!(approx(c[0], 13.0));
        assert!(approx(c[1], 19.0));
    }

    #[test]
    fn center_is_shorthand_for_center_middle() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.center();
        let r = ui.place_rect(10.0, 10.0);
        assert_eq!(r, Rect::new(-5.0, -5.0, 10.0, 10.0));
    }

    #[test]
    fn quad_via_context_uses_align() {
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::new(&mut list);
            ui.translate(100.0, 100.0);
            ui.center();
            ui.quad(20.0, 20.0, [1.0, 1.0, 1.0, 1.0]);
        }
        // First vertex should be at (100 - 10, 100 - 10) = (90, 90).
        assert_eq!(list.vertices[0].position, [90.0, 90.0]);
    }

    #[test]
    fn align_unknown_token_is_collected() {
        let base = AlignSpec::DEFAULT;
        let (spec, unknown) = AlignSpec::parse("center wibble bottom", base);
        assert_eq!(spec.h, AlignH::Center);
        assert_eq!(spec.v, AlignV::Bottom);
        assert_eq!(unknown, vec!["wibble".to_string()]);
    }

    #[test]
    fn modal_begin_routes_draws_to_modal_layer() {
        let mut layers = LayerStack::new();
        {
            let mut ui = UiContext::with_layers(&mut layers);
            ui.quad(10.0, 10.0, [1.0; 4]); // base
            ui.modal_begin(Rect::new(0.0, 0.0, 50.0, 50.0));
            ui.quad(20.0, 20.0, [1.0; 4]); // routed to modal
            ui.modal_end();
            ui.quad(5.0, 5.0, [1.0; 4]); // base again
        }
        // Base list got 2 quads (8 verts), modal got 1 quad (4 verts).
        assert_eq!(layers.base().vertices.len(), 8);
        assert_eq!(layers.layers().len(), 1);
        assert_eq!(layers.layers()[0].list.vertices.len(), 4);
    }

    #[test]
    fn nested_modal_popup_balanced() {
        let mut layers = LayerStack::new();
        {
            let mut ui = UiContext::with_layers(&mut layers);
            ui.modal_begin(Rect::new(0.0, 0.0, 200.0, 200.0));
            ui.popup_begin(Rect::new(50.0, 50.0, 50.0, 50.0));
            ui.popup_end();
            ui.modal_end();
        }
        assert!(!layers.has_active_layer());
        assert_eq!(layers.layers().len(), 2);
    }

    #[test]
    #[should_panic]
    fn modal_begin_on_drawlist_only_panics_in_debug() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.modal_begin(Rect::new(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    #[should_panic(expected = "unbalanced modal_begin")]
    fn unbalanced_modal_drop_panics_in_debug() {
        // Box the LayerStack so we can leak it on panic-unwind to avoid a
        // double-panic from its own balance assertion.
        let mut layers = Box::new(LayerStack::new());
        let layers_ptr: *mut LayerStack = &mut *layers;
        // SAFETY: forget the box to prevent its Drop firing during unwind.
        std::mem::forget(layers);
        // SAFETY: still pointing at valid memory we won't touch after the
        // panic; the test process tears down regardless.
        let layers_ref = unsafe { &mut *layers_ptr };
        let mut ui = UiContext::with_layers(layers_ref);
        ui.modal_begin(Rect::new(0.0, 0.0, 1.0, 1.0));
        // Drop of `ui` fires the debug_assert.
    }

    #[test]
    #[should_panic(expected = "layer kind mismatch")]
    fn popup_begin_followed_by_modal_end_panics_in_debug() {
        let mut layers = LayerStack::new();
        let mut ui = UiContext::with_layers(&mut layers);
        ui.popup_begin(Rect::new(0.0, 0.0, 1.0, 1.0));
        ui.modal_end(); // wrong kind -> debug_assert; layer still popped
    }

    #[test]
    fn align_call_warns_once_per_unknown_token() {
        // Same unknown token across multiple align() calls should be deduped.
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("typo");
        ui.align("typo");
        assert_eq!(ui.warned_align_tokens.len(), 1);
        ui.align("other_typo");
        assert_eq!(ui.warned_align_tokens.len(), 2);
    }
}
