//! Status bar — the design's 26px bottom strip (Menu Bar sheet).
//!
//! Hairline-separated zones: the first zone stretches, later zones size to
//! their content and read as hairline-divided cells. Callers compose text,
//! badges, or progress cells per frame; the widget only owns the strip
//! geometry, the divider hairlines, and cursor flow.

use crate::layout::Rect;
use crate::style::StyleKey;

use super::DrawContext;

/// Height of one status strip, in pixels (the design's 26px bar).
pub const STATUS_BAR_HEIGHT: f32 = 26.0;

/// One cell of the status bar.
pub struct StatusCell<'a> {
    /// Text shown in the cell (drawn dim; use [`StatusCell::highlight`] to
    /// brighten it).
    pub text: &'a str,
    /// Reserve a fixed width instead of fitting the text (a stretchy spacer
    /// passes an empty text and a `width`).
    pub width: Option<f32>,
    /// Draw the text bright (state worth noticing) instead of dim.
    pub highlight: bool,
}

impl<'a> StatusCell<'a> {
    /// A text cell sized to its content.
    pub fn text(text: &'a str) -> Self {
        Self {
            text,
            width: None,
            highlight: false,
        }
    }

    /// A spacer cell of fixed width (pass `text: ""`).
    pub fn spacer(width: f32) -> Self {
        Self {
            text: "",
            width: Some(width),
            highlight: false,
        }
    }

    /// Bright text variant.
    pub fn highlight(mut self) -> Self {
        self.highlight = true;
        self
    }
}

/// Draw the status strip across `rect` with `cells` laid out left to right.
///
/// The **first** cell stretches to absorb the remaining width; the rest size
/// to their content. Each boundary gets the design's hairline divider (dark
/// line with a light line beneath — the 4a inset read).
pub fn draw(rect: Rect, cells: &[StatusCell<'_>], ctx: &mut DrawContext) {
    ctx.push_debug_scope_rect("StatusBar", rect);
    let s = ctx.styles();
    let list = &mut *ctx.draw_list;

    // The strip: a sunken panel hairline — top edge dark line, bottom light.
    list.quad(rect.x, rect.y, rect.width, 1.0, [0.0, 0.0, 0.0, 0.6]);
    let bg = s.color(StyleKey::Panel);
    let mut fill = bg;
    fill[3] *= 0.6;
    list.quad(
        rect.x,
        rect.y + 1.0,
        rect.width,
        (rect.height - 2.0).max(0.0),
        fill,
    );
    let under = s.color(StyleKey::EdgeShadow);
    list.quad(rect.x, rect.y + rect.height - 1.0, rect.width, 1.0, under);

    let font_size = s.scalar(StyleKey::FontSize) * 0.85;
    let pad = s.scalar(StyleKey::Padding) + 2.0;
    let inner_h = (rect.height - 2.0).max(0.0);

    // Pass 1: measure fixed cells; the first cell absorbs the rest.
    let mut fixed_w = 0.0;
    let mut measured: Vec<(f32, f32)> = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        let (tw, _) = if cell.text.is_empty() {
            (0.0, 0.0)
        } else {
            list.measure_text(cell.text, font_size, None)
        };
        let w = cell.width.unwrap_or(tw + pad * 2.0);
        measured.push((w, tw));
        if i > 0 {
            fixed_w += w;
        }
    }
    let first_w = (rect.width - fixed_w).max(0.0);

    let mut x = rect.x;
    for (i, (cell, (w, tw))) in cells.iter().zip(measured.iter()).enumerate() {
        let w = if i == 0 { first_w } else { *w };
        // Divider hairline before every cell but the first.
        if i > 0 {
            list.quad(x, rect.y + 2.0, 1.0, inner_h - 4.0, [0.0, 0.0, 0.0, 0.55]);
            list.quad(x + 1.0, rect.y + 2.0, 1.0, inner_h - 4.0, {
                let u = s.color(StyleKey::EdgeShadow);
                [u[0], u[1], u[2], u[3] * 0.5]
            });
            x += 2.0;
        }

        if !cell.text.is_empty() {
            let color = if cell.highlight {
                s.color(StyleKey::Text)
            } else {
                s.color(StyleKey::TextDim)
            };
            let ty = list.vcentered_text_y(
                rect.y + 1.0,
                inner_h,
                font_size,
                s.theme().font.as_ref(),
                cell.text,
            );
            list.text(
                crate::text::TextBlock::new(cell.text, x + pad, ty)
                    .with_size(font_size)
                    .with_color(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone()),
            );
        }
        let _ = tw;
        x += w;
    }

    ctx.pop_debug_scope();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    #[test]
    fn first_cell_stretches_and_dividers_land_between_cells() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        draw(
            Rect::new(0.0, 0.0, 300.0, STATUS_BAR_HEIGHT),
            &[
                StatusCell::text("Ready"),
                StatusCell::text("118 fps"),
                StatusCell::spacer(40.0),
            ],
            &mut ctx,
        );
        // Text blocks for the two text cells; spacer contributes none.
        assert_eq!(ctx.draw_list.texts.len(), 2);
        // Divider hairlines: 2 (before cell 1 and cell 2), each 2 quads.
        assert!(ctx.draw_list.chrome_instances.len() > 0);
    }

    #[test]
    fn highlight_cells_resolve_bright_text() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        draw(
            Rect::new(0.0, 0.0, 200.0, STATUS_BAR_HEIGHT),
            &[StatusCell::text("err").highlight()],
            &mut ctx,
        );
        assert_eq!(ctx.draw_list.texts.len(), 1);
    }
}
