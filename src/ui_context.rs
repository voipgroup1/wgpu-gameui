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
    /// for vertical. Unknown tokens fall back to the previous component.
    /// Empty input returns `Default`.
    fn parse(spec: &str, base: Self) -> Self {
        let mut h = base.h;
        let mut v = base.v;
        for token in spec.split_ascii_whitespace() {
            match token {
                "left" => h = AlignH::Left,
                "center" => h = AlignH::Center,
                "right" => h = AlignH::Right,
                "top" => v = AlignV::Top,
                "middle" | "center_v" => v = AlignV::Middle,
                "bottom" => v = AlignV::Bottom,
                _ => {}
            }
        }
        Self { h, v }
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

/// Teardown-style façade over `DrawList`. Owns no draw state — borrows the
/// list for the duration of the build.
pub struct UiContext<'a> {
    list: &'a mut DrawList,
    align_stack: Vec<AlignSpec>,
}

impl<'a> UiContext<'a> {
    /// Wrap an existing `DrawList`.
    pub fn new(list: &'a mut DrawList) -> Self {
        Self {
            list,
            align_stack: vec![AlignSpec::DEFAULT],
        }
    }

    /// Push transform + tint + align (Teardown's `UiPush`).
    pub fn push(&mut self) {
        self.list.push_transform();
        self.list.push_tint();
        let top = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        self.align_stack.push(top);
    }

    /// Pop transform + tint + align (Teardown's `UiPop`).
    pub fn pop(&mut self) {
        self.list.pop_transform();
        self.list.pop_tint();
        if self.align_stack.len() > 1 {
            self.align_stack.pop();
        }
    }

    /// Shift the local origin (Teardown's `UiTranslate`).
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.list.translate(dx, dy);
    }

    /// Rotate the local coordinate frame (Teardown's `UiRotate` is in degrees;
    /// we take radians to match Rust convention. Use `f32::to_radians()` to
    /// convert from degrees at the call site).
    pub fn rotate(&mut self, angle_radians: f32) {
        self.list.rotate(angle_radians);
    }

    /// Non-uniform scale the local coordinate frame (Teardown's `UiScale`).
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.list.scale(sx, sy);
    }

    /// Set alignment for subsequent placement helpers (Teardown's `UiAlign`).
    pub fn align(&mut self, spec: &str) {
        let base = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let new_spec = AlignSpec::parse(spec, base);
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
        self.list.set_tint([r, g, b, a]);
    }

    /// Multiply into the current tint (Teardown's `UiColorFilter`).
    pub fn color_filter(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.list.multiply_tint([r, g, b, a]);
    }

    /// Return the current world-space cursor position (origin of the local
    /// frame after all active transforms).
    pub fn cursor(&self) -> [f32; 2] {
        self.list.current_transform().transform_point([0.0, 0.0])
    }

    /// Compute the world-space rect for a widget of the given local size at
    /// the current origin under the active alignment, then transform through
    /// the active affine. For translate-only and axis-aligned-scale transforms
    /// this is exact; for rotated/sheared transforms it returns the AABB of
    /// the rotated quad and the result will not match a rotated draw exactly.
    pub fn place_rect(&self, width: f32, height: f32) -> Rect {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(width, height);
        let local = Rect::new(ox, oy, width, height);
        self.list
            .current_transform()
            .transform_rect_aabb(local)
    }

    /// Draw a colored quad of the given size at the aligned origin.
    pub fn quad(&mut self, w: f32, h: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.list.quad(ox, oy, w, h, color);
    }

    /// Draw a rounded rect of the given size at the aligned origin.
    pub fn rounded_rect(&mut self, w: f32, h: f32, radius: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.list
            .rounded_rect(Rect::new(ox, oy, w, h), radius, color);
    }

    /// Draw a text block whose origin honours align/transform.
    ///
    /// Width for alignment is derived from the block's `max_width`; height
    /// from `line_height`. Use `measure_text` if you need pixel-perfect text
    /// alignment.
    pub fn text(&mut self, mut block: TextBlock) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let w = block.max_width;
        let h = block.line_height;
        let [ox, oy] = align.offset(w, h);
        block.x += ox;
        block.y += oy;
        self.list.text(block);
    }

    /// Direct access to the underlying `DrawList`. Calls still honour the
    /// active transform / tint / clip stacks because those live on the list.
    pub fn list(&mut self) -> &mut DrawList {
        self.list
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
        let ui = UiContext::new(&mut list);
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
}
