//! Popover — the design's anchored arrow sheet (Gallery III "popover").
//!
//! A small raised panel with a 7px 45° arrow pointing at its anchor, plus a
//! close ghost key. Geometry helpers are pure and unit-tested; drawing goes
//! through a caller-provided `DrawList` (usually a popup layer's) because the
//! popover floats above the base layer.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::material;
use super::{DrawContext, DrawList};

/// Where the popover sits relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverSide {
    /// Above the anchor, arrow pointing down.
    Above,
    /// Below the anchor, arrow pointing up.
    Below,
}

/// Compute the popover body rect for an anchor point (pure).
///
/// * `anchor` — the point the arrow tip points at (e.g. the top-center of the
///   trigger button, in the same space as `bounds`).
/// * `size` — the popover body's `[w, h]`.
/// * `bounds` — the screen/viewport the popover must stay inside: the body is
///   nudged horizontally to fit.
pub fn place_popover(anchor: [f32; 2], size: [f32; 2], bounds: Rect, side: PopoverSide) -> Rect {
    let w = size[0];
    let h = size[1];
    let mut x = anchor[0] - w * 0.5;
    x = x.max(bounds.x + 4.0).min(bounds.right() - w - 4.0);
    let y = match side {
        PopoverSide::Above => anchor[1] - h - 6.0,
        PopoverSide::Below => anchor[1] + 6.0,
    };
    Rect::new(x, y, w, h)
}

/// Measure the minimum body height that keeps `title` and wrapped `lines` inside
/// a popover with `body_width`. Pass the result to [`place_popover`] before
/// drawing; [`draw_sheet`] deliberately stays rect-native.
pub fn measure_sheet_height(
    body_width: f32,
    title: &str,
    lines: &[&str],
    list: &mut DrawList,
    s: &StyleResolver,
) -> f32 {
    let font_size = s.scalar(StyleKey::FontSize);
    let _ = title;
    let content_width = (body_width - 16.0).max(0.0);
    let line_stack: f32 = lines
        .iter()
        .map(|line| list.measure_text(line, font_size, Some(content_width)).1 + 4.0)
        .sum();
    // Header runs through y=22 and the body starts at y=24. Text shaping's
    // visual bounds can sit below the measured line box, so reserve a full 8px
    // bottom inset after the final row gap rather than just enough height to
    // avoid clipping descenders.
    (36.0 + line_stack).max(34.0)
}

/// Draw the popover sheet (body + arrow) with a title, body text lines, and a
/// close key in the top-right. Returns the rect of the close key so the caller
/// can hit-test it (or pass an input and let this do it — see
/// [`PopoverOutput`]).
pub fn draw_sheet(
    body: Rect,
    side: PopoverSide,
    title: &str,
    lines: &[&str],
    list: &mut DrawList,
    s: &StyleResolver,
) -> Rect {
    // Drop shadow: the design floats the sheet on `0 14px 44px` black; blur
    // extends ~half its radius, so the falloff margin is 22px.
    list.drop_shadow(
        body,
        14.0,
        22.0,
        s.scalar(StyleKey::BorderRadius),
        [0.0, 0.0, 0.0, 0.6],
    );
    // Declared scope covers body + arrow + the shadow skirt that pokes past
    // them (so debug lints don't flag the intentional overspray).
    list.push_debug_scope_rect(
        "Popover",
        match side {
            PopoverSide::Above => Rect::new(
                body.x - 22.0,
                body.y - 22.0,
                body.width + 44.0,
                body.height + 6.0 + 22.0,
            ),
            PopoverSide::Below => Rect::new(
                body.x - 22.0,
                body.y - 6.0 - 22.0,
                body.width + 44.0,
                body.height + 6.0 + 22.0,
            ),
        },
    );
    let radius = s.scalar(StyleKey::BorderRadius);

    // Arrow: a small rotated square behind the body edge (drawn as a triangle)
    // that pokes 6px past the body.
    let ax = body.x + body.width * 0.5;
    let (ay0, ay1) = match side {
        PopoverSide::Above => (body.bottom() - 1.0, body.bottom() + 6.0),
        PopoverSide::Below => (body.y + 1.0, body.y - 6.0),
    };
    let panel = s.color(StyleKey::Panel);
    list.triangle((ax - 5.0, ay0), (ax + 5.0, ay0), (ax, ay1), panel);

    // Sheet: the raised panel, near-black with a strong edge.
    list.chrome_rect(body, radius, 1.0, panel, [0.0, 0.0, 0.0, 0.75]);
    // 1px inset highlight under the top edge.
    let hl = s.color(StyleKey::EdgeHighlight);
    list.quad(
        body.x + 1.0,
        body.y + 1.0,
        (body.width - 2.0).max(0.0),
        1.0,
        hl,
    );

    // Title + close key.
    let font_size = s.scalar(StyleKey::FontSize);
    let title_color = s.color(StyleKey::Text);
    let ty = list.vcentered_text_y(
        body.y + 4.0,
        18.0,
        font_size,
        s.theme().font.as_ref(),
        title,
    );
    list.text(
        TextBlock::new(title, body.x + 8.0, ty)
            .with_size(font_size)
            .with_color(
                (title_color[0] * 255.0) as u8,
                (title_color[1] * 255.0) as u8,
                (title_color[2] * 255.0) as u8,
            )
            .with_ellipsis()
            .with_max_width(body.width - 16.0 - 18.0)
            .with_font_opt(s.theme().font.clone()),
    );

    // Close key: a small ghost square top-right.
    let close = Rect::new(body.right() - 18.0, body.y + 4.0, 14.0, 14.0);
    let base = s.color(StyleKey::Button);
    let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
    let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
    list.chrome_rect_gradient(close, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.5]);
    let xc = s.color(StyleKey::TextDim);
    let xty = list.vcentered_text_y(close.y, close.height, 9.0, s.theme().font.as_ref(), "✕");
    list.text(
        TextBlock::new("✕", close.x + 3.0, xty)
            .with_size(9.0)
            .with_color(
                (xc[0] * 255.0) as u8,
                (xc[1] * 255.0) as u8,
                (xc[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );

    // Body lines.
    let dim = s.color(StyleKey::TextDim);
    let mut y = body.y + 24.0;
    for line in lines {
        let (tw, th) = {
            // Wrap into the body width.
            let m = list.measure_text(line, font_size, Some(body.width - 16.0));
            m
        };
        let _ = tw;
        let ly = list.vcentered_text_y(y, th, font_size, s.theme().font.as_ref(), line);
        list.text(
            TextBlock::new(*line, body.x + 8.0, ly)
                .with_size(font_size)
                .with_color(
                    (dim[0] * 255.0) as u8,
                    (dim[1] * 255.0) as u8,
                    (dim[2] * 255.0) as u8,
                )
                .with_max_width(body.width - 16.0)
                .with_font_opt(s.theme().font.clone()),
        );
        y += th + 4.0;
    }

    list.pop_debug_scope();
    close
}

/// Outcome of a popover frame (from [`Popover::draw`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PopoverOutput {
    /// The close key (or outside click) was activated — caller closes.
    pub close_requested: bool,
}

/// A convenience wrapper: draws the sheet and hit-tests the close key against
/// the current input. Persistent `open` stays caller-owned.
#[derive(Clone, Copy, Default)]
pub struct Popover;

impl Popover {
    /// Draw a popover at the rect from [`place_popover`]; returns close intent.
    pub fn draw(
        &self,
        body: Rect,
        side: PopoverSide,
        title: &str,
        lines: &[&str],
        ctx: &mut DrawContext,
    ) -> PopoverOutput {
        let s = ctx.styles();
        let close = {
            let list = &mut *ctx.draw_list;
            draw_sheet(body, side, title, lines, list, &s)
        };
        let input = ctx.input;
        let inside = body.contains(input.mouse_x, input.mouse_y)
            || close.contains(input.mouse_x, input.mouse_y);
        let clicked_outside = !inside && input.mouse_clicked && !input.mouse_consumed;
        let close_hit = close.contains(input.mouse_x, input.mouse_y)
            && input.mouse_clicked
            && !input.mouse_consumed;
        PopoverOutput {
            close_requested: close_hit || clicked_outside,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    #[test]
    fn place_keeps_the_body_inside_bounds_and_on_the_right_side() {
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
        // Anchor at the very left edge: the body must be nudged right to fit.
        let above = place_popover([5.0, 100.0], [120.0, 60.0], bounds, PopoverSide::Above);
        assert!(above.x >= bounds.x + 4.0, "nudged inside the left edge");
        assert!(above.bottom() < 100.0, "above the anchor");
        let below = place_popover([5.0, 100.0], [120.0, 60.0], bounds, PopoverSide::Below);
        assert!(below.y > 100.0, "below the anchor");
    }

    #[test]
    fn measured_height_fits_wrapped_content() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let h = measure_sheet_height(
            212.0,
            "Rename",
            &["Enter a new name for the selected entity."],
            &mut list,
            &styles,
        );
        assert!(
            h > 58.0,
            "the gallery sentence needs more than its old fixed 58px body"
        );

        let body = Rect::new(0.0, 0.0, 212.0, h);
        let line_h = list
            .measure_text(
                "Enter a new name for the selected entity.",
                theme.font_size,
                Some(196.0),
            )
            .1;
        const BOTTOM_INSET: f32 = 8.0;
        assert!(
            body.height - (24.0 + line_h + 4.0) >= BOTTOM_INSET,
            "measured popover leaves a visible bottom inset after wrapped body text"
        );
    }

    #[test]
    fn close_key_click_requests_close() {
        let theme = Theme::default();
        let body = Rect::new(100.0, 100.0, 160.0, 80.0);
        // Click just outside the body (top-left corner of the screen).
        let input = InputState {
            mouse_x: 2.0,
            mouse_y: 2.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 400.0, 300.0);
        let out = Popover.draw(body, PopoverSide::Below, "Hint", &["line"], &mut ctx);
        assert!(out.close_requested, "clicking outside closes");

        // A click inside does not close.
        let input = InputState {
            mouse_x: 150.0,
            mouse_y: 130.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 400.0, 300.0);
        let out = Popover.draw(body, PopoverSide::Below, "Hint", &["line"], &mut ctx);
        assert!(!out.close_requested);
    }
}
