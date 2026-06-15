//! Color picker widget — saturation/value square + hue bar + optional alpha bar.
//!
//! The picker keeps **HSV as the source of truth**: the caller owns an [`Hsva`]
//! (threaded in and returned each frame, exactly how [`Slider`](super::Slider)
//! threads its `value`), so dragging value or saturation to an edge never loses
//! the hue (see the [`color`](crate::color) module note on the RGB→HSV round-trip
//! hazard). Drag ownership across the three sub-regions is arbitrated through a
//! caller-owned [`DragCapture`] keyed off a single [`DragId`] — internally each
//! region derives a distinct id (see [`region_id`]) so the SV square, hue bar and
//! alpha bar can't all follow one mouse drag, and they don't collide with
//! plainly-numbered sibling drag ids elsewhere in the UI.
//!
//! # Example
//! ```ignore
//! // `color` is a caller-owned `Hsva`, `capture` a shared `DragCapture`.
//! let out = ColorPicker::new()
//!     .with_alpha(true)
//!     .draw(color, 0, &mut capture, rect, &mut ctx);
//! if out.changed {
//!     color = out.hsva;
//!     let rgba = out.rgba; // straight RGBA, ready for the draw list / theme
//! }
//! ```

use crate::color::{Hsva, hsv_to_rgb};
use crate::layout::Rect;
use crate::StyleKey;

use super::{DragCapture, DragId, DrawContext};

/// Output from drawing a [`ColorPicker`].
pub struct ColorPickerOutput {
    /// The (possibly updated) color in HSVA — feed this back as the caller-owned
    /// state next frame.
    pub hsva: Hsva,
    /// The same color as straight (non-premultiplied) RGBA in `[0, 1]`.
    pub rgba: [f32; 4],
    /// Whether any channel changed this frame.
    pub changed: bool,
    /// Whether any sub-region was actively being dragged this frame.
    pub dragging: bool,
}

/// Region salts for [`region_id`] — one per draggable sub-area.
const REGION_SV: u64 = 0;
const REGION_HUE: u64 = 1;
const REGION_ALPHA: u64 = 2;

/// Derive a stable, collision-resistant [`DragId`] for one of a picker's
/// sub-regions from the picker's base id.
///
/// Mixing (rather than `base + region`) keeps a picker's three regions distinct
/// from each other *and* far from plainly-numbered sibling drag ids, so two
/// pickers with adjacent base ids — or a picker next to a slider id — never share
/// a region id by accident.
fn region_id(base: DragId, region: u64) -> DragId {
    (base.wrapping_add(1))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ region
}

/// Color picker: an SV square with a vertical hue bar (and optional alpha bar).
///
/// Procedural: the SV square is a single per-corner gradient (white → full-hue
/// across, → black down — exactly the HSV value/saturation field), the hue bar
/// is six gradient segments across the spectrum, and the alpha bar is a
/// checkerboard under an opaque→transparent fade. Sizes/colors of the chrome
/// (borders, cursor) derive from the [`Theme`](crate::Theme) so it re-themes and
/// DPI-scales like the rest of the UI.
pub struct ColorPicker {
    alpha: bool,
    /// Width of the hue/alpha bars in px.
    bar_w: f32,
    /// Gap between the SV square and the bars (and between bars) in px.
    gap: f32,
}

impl Default for ColorPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorPicker {
    /// A picker with no alpha bar.
    pub fn new() -> Self {
        Self {
            alpha: false,
            bar_w: 16.0,
            gap: 8.0,
        }
    }

    /// Show an alpha bar (and let the picker edit the color's alpha channel).
    pub fn with_alpha(mut self, on: bool) -> Self {
        self.alpha = on;
        self
    }

    /// Override the hue/alpha bar width (px).
    pub fn with_bar_width(mut self, w: f32) -> Self {
        self.bar_w = w.max(1.0);
        self
    }

    /// Override the gap between the SV square and the bars (px).
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// Sub-region rects within `rect`: `(sv_square, hue_bar, alpha_bar?)`.
    fn regions(&self, rect: Rect) -> (Rect, Rect, Option<Rect>) {
        let bars = if self.alpha { 2.0 } else { 1.0 };
        let reserved = bars * (self.bar_w + self.gap);
        let sv_w = (rect.width - reserved).max(1.0);

        let sv = Rect::new(rect.x, rect.y, sv_w, rect.height);
        let hue_x = rect.x + sv_w + self.gap;
        let hue = Rect::new(hue_x, rect.y, self.bar_w, rect.height);
        let alpha = if self.alpha {
            Some(Rect::new(
                hue_x + self.bar_w + self.gap,
                rect.y,
                self.bar_w,
                rect.height,
            ))
        } else {
            None
        };
        (sv, hue, alpha)
    }

    /// Draw the picker and return the (possibly updated) color.
    pub fn draw(
        &self,
        hsva: Hsva,
        id: DragId,
        capture: &mut DragCapture,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> ColorPickerOutput {
        // Snapshot input up front so we can borrow the draw list mutably later.
        let input = ctx.input;
        let mx = input.mouse_x;
        let my = input.mouse_y;
        let mouse_down = input.mouse_down;
        let mouse_clicked = input.mouse_clicked;
        let mouse_consumed = input.mouse_consumed;

        let (sv_rect, hue_rect, alpha_rect) = self.regions(rect);

        // --- Interaction -----------------------------------------------------
        // Per region: release on mouse-up (no-op unless we own it), then claim
        // on a fresh in-region click while the capture is free. Mirrors
        // `Slider::draw`'s release-then-claim so overlapping regions never both
        // follow one drag.
        let mut new = hsva;
        let mut dragging = false;

        let sv_id = region_id(id, REGION_SV);
        let hue_id = region_id(id, REGION_HUE);
        let alpha_id = region_id(id, REGION_ALPHA);

        let mut handle = |rid: DragId, region: Rect| -> bool {
            if !mouse_down {
                capture.release(rid);
            }
            let hovered = region.contains(mx, my) && !mouse_consumed;
            if hovered && mouse_clicked && capture.is_free() {
                capture.try_begin(rid);
            }
            capture.is_active(rid)
        };

        // SV square: x → saturation, y → value (top = bright).
        if handle(sv_id, sv_rect) {
            dragging = true;
            if sv_rect.width > 0.0 {
                new.s = ((mx - sv_rect.x) / sv_rect.width).clamp(0.0, 1.0);
            }
            if sv_rect.height > 0.0 {
                new.v = (1.0 - (my - sv_rect.y) / sv_rect.height).clamp(0.0, 1.0);
            }
        }
        // Hue bar: y → hue (top = 0°).
        if handle(hue_id, hue_rect) {
            dragging = true;
            if hue_rect.height > 0.0 {
                new.h = (((my - hue_rect.y) / hue_rect.height) * 360.0).clamp(0.0, 359.999);
            }
        }
        // Alpha bar: y → alpha (top = opaque).
        if let Some(ar) = alpha_rect.filter(|&ar| handle(alpha_id, ar)) {
            dragging = true;
            if ar.height > 0.0 {
                new.a = (1.0 - (my - ar.y) / ar.height).clamp(0.0, 1.0);
            }
        }

        let changed = new.h != hsva.h || new.s != hsva.s || new.v != hsva.v || new.a != hsva.a;

        // --- Render ----------------------------------------------------------
        let border = ctx.color(StyleKey::InputBorder);
        let border_w = ctx.scalar(StyleKey::BorderWidth).max(1.0);
        let list = &mut *ctx.draw_list;

        let white = [1.0, 1.0, 1.0, 1.0];
        let black = [0.0, 0.0, 0.0, 1.0];
        let hue_rgb = hsv_to_rgb(new.h, 1.0, 1.0);
        let hue_color = [hue_rgb[0], hue_rgb[1], hue_rgb[2], 1.0];

        // SV square: bilinear field — TL white, TR full hue, bottom black.
        list.quad_gradient(sv_rect, [white, hue_color, black, black]);
        list.rect_outline(sv_rect, border_w, border);
        // Cursor ring at (s, 1-v), double-stroked for contrast on any backing.
        let cx = sv_rect.x + new.s * sv_rect.width;
        let cy = sv_rect.y + (1.0 - new.v) * sv_rect.height;
        list.circle_outline((cx, cy), 5.0, 2.0, black);
        list.circle_outline((cx, cy), 5.0, 1.0, white);

        // Hue bar: six spectrum segments, top→bottom.
        let seg_h = hue_rect.height / 6.0;
        for i in 0..6 {
            let top = hsv_to_rgb(i as f32 * 60.0, 1.0, 1.0);
            let bot = hsv_to_rgb((i + 1) as f32 * 60.0, 1.0, 1.0);
            let top_c = [top[0], top[1], top[2], 1.0];
            let bot_c = [bot[0], bot[1], bot[2], 1.0];
            let seg = Rect::new(hue_rect.x, hue_rect.y + i as f32 * seg_h, hue_rect.width, seg_h);
            list.quad_gradient(seg, [top_c, top_c, bot_c, bot_c]);
        }
        list.rect_outline(hue_rect, border_w, border);
        draw_bar_cursor(list, hue_rect, new.h / 360.0);

        // Alpha bar: checkerboard under an opaque→transparent fade of the color.
        if let Some(ar) = alpha_rect {
            draw_checkerboard(list, ar, self.bar_w * 0.5);
            let rgb = hsv_to_rgb(new.h, new.s, new.v);
            let top_c = [rgb[0], rgb[1], rgb[2], 1.0];
            let bot_c = [rgb[0], rgb[1], rgb[2], 0.0];
            list.quad_gradient(ar, [top_c, top_c, bot_c, bot_c]);
            list.rect_outline(ar, border_w, border);
            draw_bar_cursor(list, ar, 1.0 - new.a);
        }

        ColorPickerOutput {
            hsva: new,
            rgba: new.to_rgba(),
            changed,
            dragging,
        }
    }
}

/// A horizontal cursor tick across a vertical bar at fractional position `t`
/// (0 = top, 1 = bottom): a white bar overhanging both edges with a dark outline.
fn draw_bar_cursor(list: &mut super::DrawList, bar: Rect, t: f32) {
    let y = bar.y + t.clamp(0.0, 1.0) * bar.height;
    let h = 3.0;
    let over = 2.0;
    let tick = Rect::new(bar.x - over, y - h * 0.5, bar.width + 2.0 * over, h);
    list.quad(tick.x, tick.y, tick.width, tick.height, [1.0, 1.0, 1.0, 1.0]);
    list.rect_outline(tick, 1.0, [0.0, 0.0, 0.0, 1.0]);
}

/// Fill `rect` with a two-tone checkerboard of `cell`-sized squares — the
/// classic transparency backing the alpha fade is drawn over.
fn draw_checkerboard(list: &mut super::DrawList, rect: Rect, cell: f32) {
    let cell = cell.max(1.0);
    let light = [0.75, 0.75, 0.75, 1.0];
    let dark = [0.5, 0.5, 0.5, 1.0];
    let cols = (rect.width / cell).ceil() as i32;
    let rows = (rect.height / cell).ceil() as i32;
    for row in 0..rows {
        for col in 0..cols {
            let x = rect.x + col as f32 * cell;
            let y = rect.y + row as f32 * cell;
            // Clamp the trailing cells so the pattern never spills past `rect`.
            let w = (rect.x + rect.width - x).min(cell);
            let h = (rect.y + rect.height - y).min(cell);
            let color = if (row + col) % 2 == 0 { light } else { dark };
            list.quad(x, y, w, h, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn rect() -> Rect {
        // SV square ~ left, hue bar on the right. 200 wide, 120 tall.
        Rect::new(0.0, 0.0, 200.0, 120.0)
    }

    fn draw(
        picker: &ColorPicker,
        hsva: Hsva,
        id: DragId,
        cap: &mut DragCapture,
        input: &InputState,
    ) -> ColorPickerOutput {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = Theme::default();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        picker.draw(hsva, id, cap, rect(), &mut ctx)
    }

    fn press_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    fn hold_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
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
    fn region_ids_are_distinct_and_stable() {
        let sv = region_id(7, REGION_SV);
        let hue = region_id(7, REGION_HUE);
        let alpha = region_id(7, REGION_ALPHA);
        assert!(sv != hue && hue != alpha && sv != alpha, "three distinct ids");
        assert_eq!(sv, region_id(7, REGION_SV), "stable across calls");
        // A sibling picker with an adjacent base must not collide.
        assert!(region_id(8, REGION_SV) != sv);
    }

    #[test]
    fn press_in_sv_square_sets_saturation_and_value() {
        let picker = ColorPicker::new();
        let mut cap = DragCapture::new();
        // SV square spans x∈[0,200-bar-gap]=~[0,176], y∈[0,120].
        // Press near right edge (high saturation), near top (high value).
        let out = draw(&picker, Hsva::opaque(0.0, 0.5, 0.5), 0, &mut cap, &press_at(170.0, 6.0));
        assert!(out.dragging);
        assert!(out.changed);
        assert!(out.hsva.s > 0.9, "right edge → high saturation, got {}", out.hsva.s);
        assert!(out.hsva.v > 0.9, "top → high value, got {}", out.hsva.v);
        // Hue untouched by an SV drag.
        assert_eq!(out.hsva.h, 0.0);
    }

    #[test]
    fn press_in_hue_bar_sets_hue() {
        let picker = ColorPicker::new();
        let mut cap = DragCapture::new();
        // Hue bar is the right-most 16px column. x ≈ 184..200.
        // Press at mid-height → hue ≈ 180.
        let out = draw(&picker, Hsva::opaque(0.0, 1.0, 1.0), 0, &mut cap, &press_at(192.0, 60.0));
        assert!(out.dragging);
        assert!(
            (out.hsva.h - 180.0).abs() < 5.0,
            "mid hue bar → ~180°, got {}",
            out.hsva.h
        );
    }

    #[test]
    fn press_in_alpha_bar_sets_alpha_when_enabled() {
        let picker = ColorPicker::new().with_alpha(true);
        let mut cap = DragCapture::new();
        // With alpha on: SV | hue | alpha. Alpha is the right-most 16px column,
        // x ≈ 184..200; hue is the column to its left. Press low → low alpha.
        let out = draw(&picker, Hsva::new(0.0, 1.0, 1.0, 1.0), 0, &mut cap, &press_at(192.0, 114.0));
        assert!(out.dragging);
        assert!(out.hsva.a < 0.1, "bottom of alpha bar → ~0 alpha, got {}", out.hsva.a);
    }

    #[test]
    fn alpha_region_inert_when_disabled() {
        let picker = ColorPicker::new(); // no alpha
        let mut cap = DragCapture::new();
        // Clicking where the alpha bar *would* be lands in the hue bar instead
        // (no alpha column exists); alpha stays put.
        let start = Hsva::new(0.0, 1.0, 1.0, 0.4);
        let out = draw(&picker, start, 0, &mut cap, &press_at(192.0, 114.0));
        assert_eq!(out.hsva.a, 0.4, "alpha unchanged without an alpha bar");
    }

    #[test]
    fn sv_and_hue_do_not_both_claim_one_press() {
        // Two pickers can't be tested for cross-region here, but within one
        // picker only the region under the cursor claims. Press in the SV square;
        // hue must be untouched, proving the hue region didn't also grab it.
        let picker = ColorPicker::new();
        let mut cap = DragCapture::new();
        let out = draw(&picker, Hsva::opaque(123.0, 0.5, 0.5), 0, &mut cap, &press_at(80.0, 60.0));
        assert!(out.dragging);
        assert_eq!(out.hsva.h, 123.0, "SV drag leaves hue alone");
    }

    #[test]
    fn consumed_mouse_does_not_start_drag() {
        let picker = ColorPicker::new();
        let mut cap = DragCapture::new();
        let mut input = press_at(80.0, 60.0);
        input.mouse_consumed = true;
        let out = draw(&picker, Hsva::opaque(0.0, 0.5, 0.5), 0, &mut cap, &input);
        assert!(!out.dragging);
        assert!(!out.changed);
        assert!(cap.is_free());
    }

    #[test]
    fn release_frees_capture_and_drag_continues_until_then() {
        let picker = ColorPicker::new();
        let mut cap = DragCapture::new();
        // Press in SV.
        let down = press_at(40.0, 30.0);
        let o1 = draw(&picker, Hsva::opaque(0.0, 0.5, 0.5), 0, &mut cap, &down);
        assert!(o1.dragging);
        // Hold, cursor leaves the rect vertically but the owner keeps tracking.
        let mov = hold_at(120.0, 500.0);
        let o2 = draw(&picker, o1.hsva, 0, &mut cap, &mov);
        assert!(o2.dragging, "held drag continues outside the rect");
        assert!(o2.hsva.v <= 0.0001, "y far below → value clamped to 0");
        // Release frees it.
        let up = release_at(120.0, 500.0);
        let o3 = draw(&picker, o2.hsva, 0, &mut cap, &up);
        assert!(!o3.dragging);
        assert!(cap.is_free());
    }

    #[test]
    fn rgba_output_matches_hsva() {
        let picker = ColorPicker::new();
        let mut cap = DragCapture::new();
        let c = Hsva::new(120.0, 1.0, 1.0, 0.5);
        let out = draw(&picker, c, 0, &mut cap, &release_at(-1.0, -1.0));
        assert_eq!(out.rgba, c.to_rgba());
        assert!(!out.changed, "no interaction → unchanged");
    }
}
