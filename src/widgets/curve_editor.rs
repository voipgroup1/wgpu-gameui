//! Curve editor — the design's keyframe curve (Gallery III "curve editor").
//!
//! A sunken plot area with a grid, a filled region under an interpolated
//! curve, and draggable key points. Keys are caller-owned `[[x, y]; N]` in
//! normalized `[0,1]` space (y up); the widget renders and reports drags and
//! inserts. `x` must stay sorted — the widget keeps order when a drag would
//! cross a neighbor.

use crate::layout::Rect;
use crate::style::StyleKey;

use super::DrawContext;
use super::drag::{DragCapture, DragId};
use super::material::draw_inset_shadow;

/// Outcome of drawing the curve editor.
#[derive(Debug, Clone, Default)]
pub struct CurveOutput {
    /// Index of a key being dragged this frame.
    pub dragging: Option<usize>,
    /// The dragged key's new position `(x, y)` (caller stores it).
    pub dragged: Option<[f32; 2]>,
    /// A click on empty plot space requested a key insert at `(x, y)`.
    pub insert: Option<[f32; 2]>,
}

/// Map plot-space `[0,1]²` (y up) to screen px inside `plot`.
fn to_screen(plot: Rect, x: f32, y: f32) -> [f32; 2] {
    [plot.x + x * plot.width, plot.y + (1.0 - y) * plot.height]
}

/// Map screen px back to plot space.
fn to_plot(plot: Rect, px: f32, py: f32) -> [f32; 2] {
    [
        ((px - plot.x) / plot.width).clamp(0.0, 1.0),
        (1.0 - (py - plot.y) / plot.height).clamp(0.0, 1.0),
    ]
}

/// Draw the curve editor. `keys` are the caller's points (sorted by x);
/// `selected` is the caller-owned selected index. Returns [`CurveOutput`].
pub fn draw(
    plot: Rect,
    keys: &[[f32; 2]],
    selected: usize,
    drag: &mut Option<usize>,
    capture: &mut DragCapture,
    id: DragId,
    ctx: &mut DrawContext,
) -> CurveOutput {
    // Declared scope includes handle headroom: key circles (r ≈ 4.5) at the
    // corners poke ~5px past the plot well, and their cast shadows another
    // 1px, by design.
    ctx.push_debug_scope_rect(
        "CurveEditor",
        Rect::new(
            plot.x - 6.0,
            plot.y - 6.0,
            plot.width + 12.0,
            plot.height + 12.0,
        ),
    );
    let input = ctx.input;
    let s = ctx.styles();
    let mut out = CurveOutput::default();
    let list = &mut *ctx.draw_list;
    let radius = s.scalar(StyleKey::BorderRadius);

    // ---- Plot surface: sunken well ----
    list.chrome_rect(
        plot,
        radius,
        1.0,
        [0.0, 0.0, 0.0, 0.45],
        [0.0, 0.0, 0.0, 0.7],
    );
    draw_inset_shadow(list, &s, plot, s.scalar(StyleKey::InnerShadowDepth), 1.0);

    // ---- Grid: 4×4 hairlines (25% steps) ----
    let grid = s.color(StyleKey::TextDim);
    let grid = [grid[0], grid[1], grid[2], 0.10];
    for i in 1..4 {
        let t = i as f32 / 4.0;
        // vertical
        let gx = plot.x + t * plot.width;
        list.quad(gx, plot.y + 1.0, 1.0, plot.height - 2.0, grid);
        // horizontal
        let gy = plot.y + t * plot.height;
        list.quad(plot.x + 1.0, gy, plot.width - 2.0, 1.0, grid);
    }

    // ---- Curve: fill under the polyline, then the line itself ----
    let accent = s.color(StyleKey::Accent);
    if keys.len() >= 2 {
        // Fill: the design closes the curve polygon at the bottom corners and
        // fades its alpha down. One strip per segment: the under-chord
        // trapezoid `(a, b, b↓bottom, a↓bottom)` split on the diagonal
        // `b → a↓bottom`, corner colors fading from ~0.30 alpha at the curve
        // to ~0.05 at the bottom (interpolated per-vertex by the GPU).
        let bottom = plot.y + plot.height - 1.0;
        let mut fill_a = accent;
        fill_a[3] = 0.30;
        let mut fill_b = accent;
        fill_b[3] = 0.05;
        for w in keys.windows(2) {
            let (a, b) = (
                to_screen(plot, w[0][0], w[0][1]),
                to_screen(plot, w[1][0], w[1][1]),
            );
            list.triangle_gradient(
                (a[0], a[1]),
                (b[0], b[1]),
                (a[0], bottom),
                fill_a,
                fill_a,
                fill_b,
            );
            list.triangle_gradient(
                (b[0], b[1]),
                (b[0], bottom),
                (a[0], bottom),
                fill_a,
                fill_b,
                fill_b,
            );
        }
        // Line: chords between keys.
        let line_c = accent;
        for w in keys.windows(2) {
            let a = to_screen(plot, w[0][0], w[0][1]);
            let b = to_screen(plot, w[1][0], w[1][1]);
            list.line(a, b, 1.6, line_c);
        }
    } else if keys.len() == 1 {
        let p = to_screen(plot, keys[0][0], keys[0][1]);
        list.line(
            [plot.x + 1.0, p[1]],
            [plot.right() - 1.0, p[1]],
            1.6,
            accent,
        );
    }

    // ---- Keys ----
    if !input.mouse_down {
        capture.release(id);
        *drag = None;
    }

    let key_hit_r = 7.0;
    let mut hit: Option<usize> = None;
    for (i, k) in keys.iter().enumerate() {
        let sp = to_screen(plot, k[0], k[1]);
        if input.mouse_x >= sp[0] - key_hit_r
            && input.mouse_x <= sp[0] + key_hit_r
            && input.mouse_y >= sp[1] - key_hit_r
            && input.mouse_y <= sp[1] + key_hit_r
            && !input.mouse_consumed
        {
            hit = Some(i);
            break;
        }
    }
    if hit.is_none()
        && input.mouse_clicked
        && plot.contains(input.mouse_x, input.mouse_y)
        && !input.mouse_consumed
    {
        out.insert = Some(to_plot(plot, input.mouse_x, input.mouse_y));
    }

    for (i, k) in keys.iter().enumerate() {
        let sp = to_screen(plot, k[0], k[1]);
        let active = i == selected || Some(i) == *drag;
        let hovered = hit == Some(i);
        let r = if active { 4.5 } else { 3.5 };
        // Drop shadow under the key plate (`0 1px 3px` in the design).
        list.drop_shadow(
            Rect::new(sp[0] - r, sp[1] - r, 2.0 * r, 2.0 * r),
            1.0,
            1.5,
            r,
            [0.0, 0.0, 0.0, 0.6],
        );
        let fill_c = if active {
            s.color(StyleKey::TextHighlight)
        } else if hovered {
            s.color(StyleKey::Text)
        } else {
            s.color(StyleKey::Accent)
        };
        list.circle((sp[0], sp[1]), r, fill_c);
        list.circle_outline((sp[0], sp[1]), r, 1.0, [0.0, 0.0, 0.0, 0.65]);
        if hovered && input.mouse_clicked && capture.is_free() {
            capture.try_begin(id);
            *drag = Some(i);
        }
    }

    // Drag: move the key, clamped so x stays between its neighbors (order
    // preservation is the widget's job; the caller just stores).
    if let Some(si) = *drag {
        if capture.is_active(id) {
            let mut p = to_plot(plot, input.mouse_x, input.mouse_y);
            let lo = if si > 0 { keys[si - 1][0] } else { 0.0 };
            let hi = if si + 1 < keys.len() {
                keys[si + 1][0]
            } else {
                1.0
            };
            p[0] = p[0].clamp(lo, hi);
            out.dragging = Some(si);
            out.dragged = Some(p);
        } else {
            *drag = None;
        }
    }

    // Pointer hint (crosshair isn't in CursorIcon; Grab reads as "draggable").
    if plot.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed {
        ctx.request_cursor(crate::CursorIcon::Grab);
    }

    ctx.pop_debug_scope();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::DrawList;
    use crate::{FocusState, InputState, Theme};

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    fn keys() -> Vec<[f32; 2]> {
        vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]]
    }

    #[test]
    fn screen_mapping_round_trips() {
        let plot = Rect::new(10.0, 20.0, 100.0, 50.0);
        let sp = to_screen(plot, 0.5, 0.5);
        assert!((sp[0] - 60.0).abs() < 1e-4);
        assert!((sp[1] - 45.0).abs() < 1e-4, "y flips");
        let back = to_plot(plot, sp[0], sp[1]);
        assert!((back[0] - 0.5).abs() < 1e-4 && (back[1] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn dragging_a_key_moves_it_and_preserves_order() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut drag: Option<usize> = None;
        let plot = Rect::new(0.0, 0.0, 150.0, 150.0);
        let keys = keys();

        // Grab the middle key at (75, 45) (y=0.7 → 0.3 from the top).
        let input = InputState {
            mouse_x: 75.0,
            mouse_y: 45.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let _ = draw(
            plot,
            &keys,
            1,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(drag, Some(1), "middle key grabbed");

        // Drag right past the last key's x: clamped to 1.0.
        let input = InputState {
            mouse_x: 149.0,
            mouse_y: 20.0,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = draw(
            plot,
            &keys,
            1,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        let p = out.dragged.unwrap();
        assert!((p[0] - 1.0).abs() < 0.02, "x clamped to the right neighbor");

        // Drag the middle key onto the first: clamped to the neighbor's x.
        let input = InputState {
            mouse_x: 5.0,
            mouse_y: 120.0,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = draw(
            plot,
            &keys,
            1,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        let p = out.dragged.unwrap();
        assert!(
            p[0] >= keys[0][0] - 1e-4,
            "x never crosses the left neighbor"
        );
    }

    #[test]
    fn clicking_empty_space_requests_an_insert() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut drag = None;
        let plot = Rect::new(0.0, 0.0, 150.0, 150.0);
        let keys = keys();
        // Click away from any key (keys are near x=0, 75, 150; y depends).
        let input = InputState {
            mouse_x: 40.0,
            mouse_y: 100.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = draw(
            plot,
            &keys,
            0,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.insert.is_some(), "empty-space click inserts a key");
    }

    #[test]
    fn plot_paints_grid_fill_and_keys() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut drag = None;
        let input = InputState {
            mouse_x: -10.0,
            mouse_y: -10.0,
            ..Default::default()
        };
        let keys = keys();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        draw(
            Rect::new(0.0, 0.0, 150.0, 150.0),
            &keys,
            0,
            &mut drag,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        // 6 grid quads + 2 fill triangles per segment + keys as circles.
        assert!(list.chrome_instances.len() >= 2, "well + shadow");
        assert!(list.circle_instances.len() >= 3, "three key handles");
        assert!(!list.vertices.is_empty(), "grid, fills, chords");
    }
}
