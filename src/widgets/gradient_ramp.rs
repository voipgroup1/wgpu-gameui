//! Gradient ramp — the design's gradient bar with draggable stops
//! (Gallery III "gradient ramp").
//!
//! The ramp is a horizontal strip filled by interpolating the color stops
//! (per-channel linear interpolation between neighboring stops). Stops are
//! caller-owned `[offset, [r,g,b,a]]`; the widget draws one frame and reports
//! drags/clicks for the caller to apply.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::drag::{DragCapture, DragId};
use super::{DrawContext, DrawList};

/// One color stop: position along the ramp `[0,1]` and its color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// Position along the bar, `0.0` = left, `1.0` = right.
    pub pos: f32,
    /// Stop color (straight RGBA).
    pub color: [f32; 4],
}

impl GradientStop {
    /// A stop at `pos` with `color`.
    pub fn new(pos: f32, color: [f32; 4]) -> Self {
        Self { pos, color }
    }
}

/// Evaluate the ramp color at `t` (clamped); stops must be sorted by `pos`.
pub fn sample(stops: &[GradientStop], t: f32) -> [f32; 4] {
    if stops.is_empty() {
        return [0.0; 4];
    }
    let t = t.clamp(0.0, 1.0);
    if t <= stops[0].pos {
        return stops[0].color;
    }
    if t >= stops[stops.len() - 1].pos {
        return stops[stops.len() - 1].color;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t >= a.pos && t <= b.pos {
            let span = (b.pos - a.pos).max(1e-6);
            let k = (t - a.pos) / span;
            let mix = |x: f32, y: f32| x + (y - x) * k;
            return [
                mix(a.color[0], b.color[0]),
                mix(a.color[1], b.color[1]),
                mix(a.color[2], b.color[2]),
                mix(a.color[3], b.color[3]),
            ];
        }
    }
    stops[stops.len() - 1].color
}

/// Outcome of drawing the ramp.
#[derive(Debug, Clone, Copy, Default)]
pub struct RampOutput {
    /// Index of the stop being dragged this frame (`None` when idle).
    pub dragging: Option<usize>,
    /// The dragged stop's new position (caller stores it).
    pub dragged_pos: Option<f32>,
    /// The bar (not a stop handle) was clicked — the caller may insert a new
    /// stop at `click_pos`.
    pub insert_requested: bool,
    /// Where on the bar was clicked (`[0,1]`), when `insert_requested`.
    pub click_pos: f32,
}

/// Draw the ramp bar with stop handles. Returns [`RampOutput`].
///
/// `rect` must include headroom for the stop handles: the bar is drawn at the
/// **bottom 18px** of `rect`, handles occupy the space above. `selected` is
/// the caller-owned selected stop (drawn active).
pub fn draw(
    rect: Rect,
    stops: &[GradientStop],
    selected: usize,
    drag: &mut Option<usize>,
    capture: &mut DragCapture,
    id: DragId,
    ctx: &mut DrawContext,
) -> RampOutput {
    // Declared scope includes the handle band: 64-sample geometry overruns the
    // last segment by ceil() rounding and handles have their own zone, so the
    // scope is the rect the caller reserved *including* handle headroom.
    ctx.push_debug_scope_rect(
        "GradientRamp",
        Rect::new(
            rect.x - 5.0,
            rect.y - 4.0,
            rect.width + 10.0,
            rect.height + 8.0,
        ),
    );
    let input = ctx.input;
    let s = ctx.styles();
    let mut out = RampOutput::default();
    let list = &mut *ctx.draw_list;

    // The bar: N segments, each a horizontal gradient between stop colors.
    let bar_h = 18.0f32.min(rect.height);
    let bar_y = rect.bottom() - bar_h;
    let bar = Rect::new(rect.x, bar_y, rect.width, bar_h);
    let radius = s.scalar(StyleKey::BorderRadius).min(3.0);
    let seg_w = rect.width / 64.0;
    let mut i = 0usize;
    while i < 64 {
        let t0 = i as f32 / 64.0;
        let x0 = rect.x + i as f32 * seg_w;
        let c = sample(stops, t0 + 0.5 / 64.0);
        let seg = Rect::new(x0, bar.y, seg_w.ceil() + 0.5, bar.height);
        list.quad_gradient(seg, [c, c, c, c]);
        i += 1;
    }
    // Border on the bar.
    list.rounded_rect_outline(bar, radius, 1.0, [0.0, 0.0, 0.0, 0.75]);
    // Top highlight.
    let hl = s.color(StyleKey::EdgeHighlight);
    list.quad(
        bar.x + 1.0,
        bar.y + 1.0,
        (bar.width - 2.0).max(0.0),
        1.0,
        [hl[0], hl[1], hl[2], 0.18],
    );
    // Under line (raised read).
    let under = s.color(StyleKey::EdgeShadow);
    list.quad(
        bar.x + 1.0,
        bar.y + bar.height - 1.0,
        (bar.width - 2.0).max(0.0),
        1.0,
        under,
    );

    // Handle geometry: 10px tall diamonds in the headroom above the bar.
    let handle_h = 10.0f32.min((rect.height - bar_h).max(6.0));
    let handle_zone_h = handle_h + 4.0;
    let handle_zone_y = bar_y - handle_zone_h - 2.0;

    if !input.mouse_down {
        capture.release(id);
        *drag = None;
    }

    // Hit-test handles first (drawn after, on top).
    let mut hit: Option<usize> = None;
    for (si, stop) in stops.iter().enumerate() {
        let hx = rect.x + stop.pos * rect.width;
        let zone = Rect::new(hx - 6.0, handle_zone_y, 12.0, handle_zone_h);
        if zone.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed {
            hit = Some(si);
            break;
        }
    }
    if hit.is_none()
        && input.mouse_clicked
        && bar.contains(input.mouse_x, input.mouse_y)
        && !input.mouse_consumed
    {
        out.insert_requested = true;
        out.click_pos = ((input.mouse_x - rect.x) / rect.width).clamp(0.0, 1.0);
    }

    for (si, stop) in stops.iter().enumerate() {
        let hx = rect.x + stop.pos * rect.width;
        let cy = handle_zone_y + handle_h * 0.5;
        let active = si == selected || Some(si) == *drag;
        let hovered = hit == Some(si);
        // Diamond as two triangles.
        let w = 5.0;
        let color = if active {
            s.color(StyleKey::TextHighlight)
        } else if hovered {
            s.color(StyleKey::Text)
        } else {
            let c = s.color(StyleKey::TextDim);
            c
        };
        list.triangle((hx - w, cy), (hx, cy - w), (hx, cy + w), color);
        list.triangle((hx - w, cy), (hx, cy + w), (hx + w, cy), color);
        if hovered && input.mouse_clicked && capture.is_free() {
            capture.try_begin(id);
            *drag = Some(si);
        }
    }

    // Apply drag: move the grabbed stop with the pointer.
    if let Some(si) = *drag {
        if capture.is_active(id) {
            out.dragging = Some(si);
            out.dragged_pos = Some(((input.mouse_x - rect.x) / rect.width).clamp(0.0, 1.0));
        } else {
            *drag = None;
        }
    }

    // Pointer hint over the bar.
    if bar.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed {
        ctx.request_cursor(crate::CursorIcon::Pointer);
    }

    ctx.pop_debug_scope();
    out
}

/// Draw a readout label of `text` right of the bar (the design shows the
/// hex/position readout). Height matches the bar.
pub fn readout(list: &mut DrawList, s: &StyleResolver, x: f32, rect: Rect, text: &str) {
    let font_size = s.scalar(StyleKey::FontSize) * 0.8;
    let ty = list.vcentered_text_y(
        rect.y,
        rect.height,
        font_size,
        s.theme().font.as_ref(),
        text,
    );
    let color = s.color(StyleKey::TextDim);
    list.text(
        TextBlock::new(text, x, ty)
            .with_size(font_size)
            .with_color(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    fn stops() -> Vec<GradientStop> {
        vec![
            GradientStop::new(0.0, [0.1, 0.2, 0.3, 1.0]),
            GradientStop::new(1.0, [0.9, 0.8, 0.7, 1.0]),
        ]
    }

    #[test]
    fn sample_interpolates_and_clamps() {
        let stops = stops();
        let mid = sample(&stops, 0.5);
        assert!((mid[0] - 0.5).abs() < 1e-4);
        let before = sample(&stops, -0.5);
        assert_eq!(before, stops[0].color);
        let after = sample(&stops, 1.5);
        assert_eq!(after, stops[1].color);
        assert_eq!(sample(&[], 0.3), [0.0; 4]);
    }

    #[test]
    fn dragging_a_handle_reports_new_positions() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut drag: Option<usize> = None;
        let rect = Rect::new(20.0, 20.0, 200.0, 42.0);
        let stops = stops();

        // Press on stop 0's handle: the bar sits at the rect's bottom 18px,
        // handles occupy the band above it (y 20..22 area).
        let input = InputState {
            mouse_x: 20.0,
            mouse_y: 32.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        draw(
            rect,
            &stops,
            0,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(drag, Some(0), "press on the handle grabs it");

        // Drag to the middle.
        let input = InputState {
            mouse_x: 120.0,
            mouse_y: 32.0,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = draw(
            rect,
            &stops,
            0,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.dragging, Some(0));
        assert!(
            (out.dragged_pos.unwrap() - 0.5).abs() < 0.02,
            "pos follows the pointer"
        );
    }

    #[test]
    fn clicking_the_bar_requests_an_insert() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut drag = None;
        let rect = Rect::new(0.0, 20.0, 200.0, 24.0);
        let stops = stops();
        // Click inside the bar (bottom 18px of the rect).
        let input = InputState {
            mouse_x: 100.0,
            mouse_y: rect.bottom() - 9.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = draw(
            rect,
            &stops,
            0,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.insert_requested);
        assert!((out.click_pos - 0.5).abs() < 0.03);
    }
}
