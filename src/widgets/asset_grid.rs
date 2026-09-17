//! Asset grid — the design's thumbnail browser (Gallery II "asset grid").
//!
//! A flow of fixed-size cells: a gradient thumbnail plate with a glyph, and a
//! one-line label below. Selection is an accent inset ring + wash; hover lifts
//! the cell. State is caller-owned (`selected`); returns the index clicked.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::DrawContext;

/// Outcome of drawing an [`AssetGrid`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AssetGridOutput {
    /// Index of an asset clicked this frame.
    pub clicked: Option<usize>,
}

/// A fixed-cell thumbnail grid. `thumb_h` is the plate height; the cell adds
/// the label band. All cells share the fixed width (the grid is a simple
/// flow: as many columns as fit `rect.width`).
#[derive(Clone)]
pub struct AssetGrid<'a> {
    items: &'a [&'a str],
    /// Glyph drawn on each thumbnail (any string; an icon font key or symbol).
    glyph: &'a str,
    cell_w: f32,
    thumb_h: f32,
}

impl<'a> AssetGrid<'a> {
    /// A grid of `items` (labels), each cell showing `glyph`.
    pub fn new(items: &'a [&'a str], glyph: &'a str) -> Self {
        Self {
            items,
            glyph,
            cell_w: 86.0,
            thumb_h: 58.0,
        }
    }

    /// Fixed cell width (default 86).
    pub fn with_cell_width(mut self, w: f32) -> Self {
        self.cell_w = w;
        self
    }

    /// Thumbnail plate height (default 58).
    pub fn with_thumb_height(mut self, h: f32) -> Self {
        self.thumb_h = h;
        self
    }

    /// Cell height = thumb + label band.
    pub fn cell_height(&self, s: &StyleResolver) -> f32 {
        self.thumb_h + s.scalar(StyleKey::FontSize) * 0.8 + 8.0
    }

    /// Draw the grid into `rect`; returns clicks. `selected` is the
    /// caller-owned selected index (`usize::MAX` / a value ≥ len = none).
    pub fn draw(&self, rect: Rect, selected: usize, ctx: &mut DrawContext) -> AssetGridOutput {
        ctx.push_debug_scope_rect("AssetGrid", rect);
        let input = ctx.input;
        let s = ctx.styles();
        let mut out = AssetGridOutput::default();
        let list = &mut *ctx.draw_list;

        let radius = s.scalar(StyleKey::BorderRadius);
        let label_h = s.scalar(StyleKey::FontSize) * 0.8 + 8.0;
        let cell_h = self.thumb_h + label_h;
        // Thumbnail browsers need a quiet gutter around their cells: without it
        // the first plate reads as glued to the containing panel edge.
        let inset = s.scalar(StyleKey::Padding).max(2.0);
        let content = rect.inset(inset);
        let gap = s.scalar(StyleKey::Spacing).max(4.0);
        let cols = (((content.width + gap) / (self.cell_w + gap)).floor() as usize).max(1);

        for (i, label) in self.items.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = content.x + col as f32 * (self.cell_w + gap);
            let y = content.y + row as f32 * (cell_h + gap);
            if y + cell_h > rect.bottom() + cell_h {
                break; // past the allocated band
            }
            let cell = Rect::new(x, y, self.cell_w, cell_h);
            let is_sel = i == selected;
            let hovered = cell.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
            let clicked = hovered && input.mouse_clicked;
            if clicked {
                out.clicked = Some(i);
            }

            // Selection wash + inset ring.
            if is_sel {
                let mut wash = s.color(StyleKey::Accent);
                wash[3] = 0.22;
                list.quad(cell.x, cell.y, cell.width, cell.height, wash);
                let mut ring = s.color(StyleKey::Accent);
                ring[3] = 0.7;
                list.rounded_rect_outline(cell, radius, 1.0, ring);
            }

            // Thumbnail plate: the design's vertical gradient plate.
            let thumb = Rect::new(x, y, self.cell_w, self.thumb_h);
            list.chrome_rect_gradient(
                thumb,
                radius,
                1.0,
                [0.1647, 0.1922, 0.2196, 1.0], // #2a3138
                [0.0902, 0.1059, 0.1255, 1.0], // #171b20
                [0.0, 0.0, 0.0, 0.6],
            );
            let hl = s.color(StyleKey::EdgeHighlight);
            list.quad(
                thumb.x + 1.0,
                thumb.y + 1.0,
                (thumb.width - 2.0).max(0.0),
                1.0,
                [hl[0], hl[1], hl[2], 0.12],
            );

            // Center glyph.
            let glyph_color = if is_sel {
                s.color(StyleKey::Accent)
            } else {
                s.color(StyleKey::TextDim)
            };
            let (gw, gh) = {
                let m = list.measure_text(self.glyph, 19.0, None);
                m
            };
            let gy = list.vcentered_text_y(
                thumb.y,
                thumb.height,
                19.0,
                s.theme().font.as_ref(),
                self.glyph,
            );
            list.text(
                TextBlock::new(self.glyph, thumb.x + (thumb.width - gw) * 0.5, gy)
                    .with_size(19.0)
                    .with_color(
                        (glyph_color[0] * 255.0) as u8,
                        (glyph_color[1] * 255.0) as u8,
                        (glyph_color[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone()),
            );
            let _ = gh;

            // Label (one line, ellipsized).
            let label_color = if is_sel {
                s.color(StyleKey::Text)
            } else if hovered {
                s.color(StyleKey::Text)
            } else {
                s.color(StyleKey::TextDim)
            };
            let ly = y + self.thumb_h + 4.0;
            list.text(
                TextBlock::new(*label, x, ly)
                    .with_size(s.scalar(StyleKey::FontSize) * 0.8)
                    .with_color(
                        (label_color[0] * 255.0) as u8,
                        (label_color[1] * 255.0) as u8,
                        (label_color[2] * 255.0) as u8,
                    )
                    .with_ellipsis()
                    .with_max_width(self.cell_w)
                    .with_align(crate::text::TextAlign::Center)
                    .with_font_opt(s.theme().font.clone()),
            );
        }

        ctx.pop_debug_scope();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    fn items() -> Vec<&'static str> {
        vec!["Crate_A", "Barrel", "Lamp_Post"]
    }

    #[test]
    fn click_selects_and_selection_paints_a_ring() {
        let theme = Theme::default();
        let items = items();
        let rect = Rect::new(0.0, 0.0, 300.0, 80.0);

        // Click inside the first cell (86 wide, 58+label tall).
        let input = InputState {
            mouse_x: 40.0,
            mouse_y: 30.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = AssetGrid::new(&items, "▣").draw(
            rect,
            usize::MAX,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.clicked, Some(0));

        // Selected cell draws the outline ring: an extra stroke instance.
        let mut sel_list = DrawList::new();
        let idle = InputState {
            mouse_x: -10.0,
            mouse_y: -10.0,
            ..Default::default()
        };
        let _ = AssetGrid::new(&items, "▣").draw(
            rect,
            0,
            &mut ctx(&mut sel_list, &mut focus, &theme, &idle),
        );
        let mut plain_list = DrawList::new();
        let _ = AssetGrid::new(&items, "▣").draw(
            rect,
            usize::MAX,
            &mut ctx(&mut plain_list, &mut focus, &theme, &idle),
        );
        assert!(
            sel_list.chrome_instances.len() > plain_list.chrome_instances.len(),
            "selection ring adds a stroke instance"
        );
    }

    #[test]
    fn cells_flow_into_columns() {
        let theme = Theme::default();
        let items = items();
        let input = InputState {
            mouse_x: -10.0,
            mouse_y: -10.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        // 300px wide / 86px cells → 3 columns, one row.
        AssetGrid::new(&items, "▣").draw(
            Rect::new(0.0, 0.0, 300.0, 200.0),
            usize::MAX,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        // 3 thumbnail plates (chrome gradient instances).
        assert!(list.chrome_instances.len() >= 3);
        // 3 glyphs + 3 labels.
        assert_eq!(list.texts.len(), 6);
    }

    #[test]
    fn cells_are_inset_from_the_allocated_grid_rect() {
        let theme = Theme::default();
        let items = items();
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        AssetGrid::new(&items, "▣").draw(
            Rect::new(10.0, 20.0, 300.0, 200.0),
            usize::MAX,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let first_plate = list
            .chrome_instances
            .iter()
            .find(|instance| instance.rect[2] == 86.0 && instance.rect[3] == 58.0)
            .expect("the first thumbnail plate is emitted");
        assert_eq!(first_plate.rect[0], 10.0 + theme.padding.max(2.0));
        assert_eq!(first_plate.rect[1], 20.0 + theme.padding.max(2.0));
    }
}
