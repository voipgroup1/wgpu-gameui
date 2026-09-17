//! Breadcrumb + pager — the design's linear navigation strips (Gallery III).
//!
//! - [`Breadcrumb`]: a `path / to / here` strip of clickable labels separated
//!   by hairline slashes; returns the index clicked (if any).
//! - [`Pager`]: `◀ 3 / 12 ▶` stepper strip (plus an optional jump field is the
//!   caller's concern — the widget stays a pure strip).

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::{DrawContext, DrawList};

/// A breadcrumb trail. `segments` are the labels; all are drawn, the last one
/// as the current location (bright, not clickable).
#[derive(Clone)]
pub struct Breadcrumb<'a> {
    segments: &'a [&'a str],
    /// Horizontal gap around the separator glyphs.
    gap: f32,
}

impl<'a> Breadcrumb<'a> {
    /// A trail over `segments` (in path order).
    pub fn new(segments: &'a [&'a str]) -> Self {
        Self { segments, gap: 5.0 }
    }

    /// Override the separator gap (default 5px each side).
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Draw the trail; returns the index of a *non-final* segment clicked this
    /// frame, else `None` (the final segment is the current location).
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> Option<usize> {
        ctx.push_debug_scope_rect("Breadcrumb", rect);
        let input = ctx.input;
        let s = ctx.styles();
        let font_size = s.scalar(StyleKey::FontSize);
        let sep_color = s.color(StyleKey::TextDim);
        let dim = s.color(StyleKey::TextDim);
        let hot = s.color(StyleKey::Text);

        let mut clicked = None;
        let text_y_baseline;
        let mut x = rect.x;
        {
            let list = &mut *ctx.draw_list;
            text_y_baseline =
                list.vcentered_text_y(rect.y, rect.height, font_size, s.theme().font.as_ref(), "|");
        }

        for (i, seg) in self.segments.iter().enumerate() {
            let last = i == self.segments.len() - 1;
            let color = if last { hot } else { dim };
            let w;
            {
                let list = &mut *ctx.draw_list;
                w = list.measure_text(seg, font_size, None).0;
            }
            let seg_rect = Rect::new(x, rect.y, w, rect.height);

            if !last {
                let hovered =
                    seg_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
                if hovered {
                    ctx.request_cursor(crate::CursorIcon::Pointer);
                    let mut hl = s.color(StyleKey::Accent);
                    hl[3] = 0.2;
                    let list = &mut *ctx.draw_list;
                    list.quad(seg_rect.x, seg_rect.y, seg_rect.width, seg_rect.height, hl);
                    if input.mouse_clicked {
                        clicked = Some(i);
                    }
                }
            }

            {
                let list = &mut *ctx.draw_list;
                list.text(
                    TextBlock::new(*seg, seg_rect.x, text_y_baseline)
                        .with_size(font_size)
                        .with_color(
                            (color[0] * 255.0) as u8,
                            (color[1] * 255.0) as u8,
                            (color[2] * 255.0) as u8,
                        )
                        .with_font_opt(s.theme().font.clone()),
                );
            }
            x += w;

            if !last {
                let sep = "/";
                let (sw, _) = {
                    let list = &mut *ctx.draw_list;
                    list.measure_text(sep, font_size, None)
                };
                let list = &mut *ctx.draw_list;
                list.text(
                    TextBlock::new(sep, x + self.gap, text_y_baseline)
                        .with_size(font_size)
                        .with_color(
                            (sep_color[0] * 255.0) as u8,
                            (sep_color[1] * 255.0) as u8,
                            (sep_color[2] * 255.0) as u8,
                        )
                        .with_font_opt(s.theme().font.clone()),
                );
                x += self.gap + sw + self.gap;
            }
        }

        ctx.pop_debug_scope();
        clicked
    }

    /// Intrinsic width of the trail under `styles`.
    pub fn intrinsic_width(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let font_size = s.scalar(StyleKey::FontSize);
        let mut w = 0.0;
        for (i, seg) in self.segments.iter().enumerate() {
            let (sw, _) = list.measure_text(seg, font_size, None);
            w += sw;
            if i < self.segments.len() - 1 {
                let (sep_w, _) = list.measure_text("/", font_size, None);
                w += self.gap * 2.0 + sep_w;
            }
        }
        w
    }
}

/// Outcome of drawing a [`Pager`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PagerOutput {
    /// The page was changed this frame (by arrows — or a direct page click
    /// when `pages` is small enough to show numbers).
    pub changed: bool,
    /// The new page after this frame's input.
    pub page: usize,
}

/// A `◀ page / total ▶` pager strip. Persistent `page` is caller-owned.
#[derive(Clone, Copy, Default)]
pub struct Pager {
    /// Draw numeric page buttons instead of only arrows (small page counts).
    pub numeric: bool,
}

impl Pager {
    /// An arrows-only pager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show one number key per page (only sensible for small totals).
    pub fn numeric(mut self) -> Self {
        self.numeric = true;
        self
    }

    /// Draw the pager over `page` of `total`; arrows clamp at the ends.
    pub fn draw(
        &self,
        page: usize,
        total: usize,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> PagerOutput {
        ctx.push_debug_scope_rect("Pager", rect);
        let input = ctx.input;
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        let radius = s.scalar(StyleKey::BorderRadius);
        let mut out = PagerOutput {
            changed: false,
            page,
        };
        let arrow_w = rect.height.min(18.0);

        // Helper: one arrow key at `x` pointing `left`.
        let mut arrow = |x: f32, left: bool, enabled: bool, out: &mut PagerOutput| {
            let r = Rect::new(x, rect.y, arrow_w, rect.height);
            let hovered =
                enabled && r.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
            let clicked = hovered && input.mouse_clicked;
            let base = if !enabled {
                let mut c = s.color(StyleKey::Button);
                c[3] = 0.45;
                c
            } else if clicked || (hovered && input.mouse_down) {
                s.color(StyleKey::ButtonPressed)
            } else if hovered {
                s.color(StyleKey::ButtonHover)
            } else {
                s.color(StyleKey::Button)
            };
            let top = super::material::sheen_over(base, s.color(StyleKey::FaceTop));
            let bottom = super::material::sheen_over(base, s.color(StyleKey::FaceBottom));
            list.chrome_rect_gradient(r, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.5]);
            let hl = s.color(StyleKey::EdgeHighlight);
            list.quad(r.x + 1.0, r.y + 1.0, (r.width - 2.0).max(0.0), 1.0, hl);
            // Chevron glyph as two lines.
            let color = if enabled {
                s.color(StyleKey::Text)
            } else {
                s.color(StyleKey::TextDim)
            };
            let cx = r.x + r.width * 0.5;
            let cy = r.y + r.height * 0.5;
            let d = r.height * 0.22;
            let (x1, x2) = if left {
                (cx + d * 0.4, cx - d * 0.4)
            } else {
                (cx - d * 0.4, cx + d * 0.4)
            };
            list.line([x1, cy - d], [x2, cy], 1.5, color);
            list.line([x2, cy], [x1, cy + d], 1.5, color);
            if clicked {
                out.page = if left {
                    out.page.saturating_sub(1)
                } else {
                    (out.page + 1).min(total.saturating_sub(1))
                };
                out.changed = true;
            }
        };

        // Numeric mode: one key per page (clipped to the rect) + no arrows
        // unless they fit.
        if self.numeric && total > 0 && total <= 12 {
            let key_w = ((rect.width - total as f32 * 2.0) / total as f32).min(24.0);
            for i in 0..total {
                let x = rect.x + i as f32 * (key_w + 2.0);
                let r = Rect::new(x, rect.y, key_w, rect.height);
                let active = i == page;
                let hovered = r.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
                let clicked = hovered && input.mouse_clicked;
                let base = if active {
                    s.color(StyleKey::ButtonPressed)
                } else if hovered {
                    s.color(StyleKey::ButtonHover)
                } else {
                    s.color(StyleKey::Button)
                };
                let top = super::material::sheen_over(base, s.color(StyleKey::FaceTop));
                let bottom = super::material::sheen_over(base, s.color(StyleKey::FaceBottom));
                list.chrome_rect_gradient(
                    r,
                    radius,
                    1.0,
                    top,
                    bottom,
                    [0.0, 0.0, 0.0, if active { 0.6 } else { 0.45 }],
                );
                let label = (i + 1).to_string();
                let font_size = s.scalar(StyleKey::FontSize) * 0.85;
                let color = if active {
                    s.color(StyleKey::TextHighlight)
                } else {
                    s.color(StyleKey::Text)
                };
                let (tw, _) = list.measure_text(&label, font_size, None);
                let ty = list.vcentered_text_y(
                    r.y,
                    r.height,
                    font_size,
                    s.theme().font.as_ref(),
                    &label,
                );
                list.text(
                    TextBlock::new(label.as_str(), r.x + (r.width - tw) * 0.5, ty)
                        .with_size(font_size)
                        .with_color(
                            (color[0] * 255.0) as u8,
                            (color[1] * 255.0) as u8,
                            (color[2] * 255.0) as u8,
                        )
                        .with_font_opt(s.theme().font.clone()),
                );
                if clicked && !active {
                    out.page = i;
                    out.changed = true;
                }
            }
            ctx.pop_debug_scope();
            return out;
        }

        arrow(rect.x, true, page > 0, &mut out);
        arrow(rect.right() - arrow_w, false, page + 1 < total, &mut out);

        // Page readout between the arrows.
        let readout = format!("{} / {}", page + 1, total.max(1));
        let font_size = s.scalar(StyleKey::FontSize) * 0.85;
        let (tw, _) = list.measure_text(&readout, font_size, None);
        let color = s.color(StyleKey::Text);
        let ty = list.vcentered_text_y(
            rect.y,
            rect.height,
            font_size,
            s.theme().font.as_ref(),
            &readout,
        );
        list.text(
            TextBlock::new(readout.as_str(), rect.x + (rect.width - tw) * 0.5, ty)
                .with_size(font_size)
                .with_color(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                )
                .with_font_opt(s.theme().font.clone()),
        );

        ctx.pop_debug_scope();
        out
    }
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

    #[test]
    fn breadcrumb_reports_click_on_a_non_final_segment() {
        let theme = Theme::default();
        let segs = ["World", "Region", "Forest"];
        let mut focus = FocusState::new();
        // Measure where "Region" actually starts using the widget's own metric.
        let w0 = {
            let mut l = DrawList::new();
            l.measure_text(segs[0], 13.0, None).0
        };
        let sep_w = {
            let mut l = DrawList::new();
            l.measure_text("/", 13.0, None).0
        };
        let region_x = w0 + 5.0 * 2.0 + sep_w + 2.0;

        let mut input = InputState::default();
        input.mouse_x = region_x;
        input.mouse_y = 5.0;
        input.mouse_clicked = true;
        let mut list = DrawList::new();
        let mut focus2 = FocusState::new();
        let clicked = Breadcrumb::new(&segs).draw(
            Rect::new(0.0, 0.0, 300.0, 20.0),
            &mut ctx(&mut list, &mut focus2, &theme, &input),
        );
        assert!(
            clicked.is_some(),
            "a click on a non-final segment reports it"
        );
        assert_ne!(
            clicked,
            Some(segs.len() - 1),
            "the current location is not clickable"
        );

        let idle = InputState::default();
        let mut list = DrawList::new();
        let none = Breadcrumb::new(&segs).draw(
            Rect::new(0.0, 0.0, 300.0, 20.0),
            &mut ctx(&mut list, &mut focus, &theme, &idle),
        );
        assert!(none.is_none());
    }

    #[test]
    fn pager_arrows_step_and_clamp() {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();

        // Right arrow sits at the right edge.
        let input = InputState {
            mouse_x: 128.0,
            mouse_y: 8.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let out = Pager::new().draw(
            0,
            5,
            Rect::new(0.0, 0.0, 140.0, 18.0),
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.changed && out.page == 1, "next arrow advances");

        // At the last page the next arrow is disabled: no further change.
        let mut list = DrawList::new();
        let out = Pager::new().draw(
            4,
            5,
            Rect::new(0.0, 0.0, 140.0, 18.0),
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(
            !out.changed && out.page == 4,
            "the next arrow clamps at the end"
        );
    }

    #[test]
    fn numeric_pager_jumps_to_a_clicked_page() {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = InputState {
            mouse_x: 40.0,
            mouse_y: 8.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let out = Pager::new().numeric().draw(
            0,
            4,
            Rect::new(0.0, 0.0, 120.0, 18.0),
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.changed, "clicking a page number jumps to it");
        assert_ne!(out.page, 0);
    }
}
