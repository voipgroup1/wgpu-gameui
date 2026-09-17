//! Vector field — the design's XYZ scrub inputs (Gallery II "vector field").
//!
//! One row per vector (`Position`, `Rotation`, …): a label, then one cell per
//! component. Each cell is a sunken well with a colored axis tag on the left
//! (X red / Y green / Z blue, the design's axis tints) and the value on the
//! right. Pointer-drag on the tag scrubs the value; all state is caller-owned.

use crate::layout::Rect;
use crate::style::StyleKey;
use crate::text::TextBlock;

use super::DrawContext;
use super::drag::{DragCapture, DragId};
use super::material;

/// Default axis tag tints (design axis colors, linear-ish sRGB).
pub const AXIS_TINTS: [[f32; 4]; 3] = [
    [0.7862, 0.3895, 0.3661, 1.0], // X — red
    [0.3722, 0.6185, 0.3817, 1.0], // Y — green
    [0.3398, 0.5344, 0.7811, 1.0], // Z — blue
];

/// A drag in progress on one component. Caller-owned (like [`DragCapture`]);
/// `[row, component]` identify the active cell.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct VectorScrub {
    /// Row index (which vector).
    pub row: usize,
    /// Component index (0=X, 1=Y, 2=Z).
    pub component: usize,
    /// Pointer x at gesture start.
    pub start_x: f32,
    /// Value at gesture start.
    pub start_value: f32,
    /// Units per pixel of drag.
    pub step: f32,
}

/// Per-frame output of [`VectorField::draw`].
#[derive(Debug, Clone)]
pub struct VectorFieldOutput {
    /// Component deltas applied this frame, as `(row, component, new_value)`.
    /// The caller writes these back into its vectors.
    pub changed: Vec<(usize, usize, f32)>,
}

/// A labeled set of 3-component scrub fields.
#[derive(Clone)]
pub struct VectorField<'a> {
    rows: &'a [(&'a str, [f32; 3])],
    /// Units per pixel of scrub per row (defaults to 0.02).
    step: f32,
}

impl<'a> VectorField<'a> {
    /// Rows of `(label, [x, y, z])`. The values are *inputs*; changed values
    /// come back in [`VectorFieldOutput::changed`] and the caller stores them.
    pub fn new(rows: &'a [(&'a str, [f32; 3])]) -> Self {
        Self { rows, step: 0.02 }
    }

    /// Set the scrub sensitivity (units per pixel).
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Height of the whole field for `rows.len()` rows at a given cell height.
    pub fn height(&self, cell_h: f32) -> f32 {
        self.rows.len() as f32 * (cell_h + 4.0) - 4.0
    }

    /// Draw the field; `capture` arbitrates the active scrub across all cells.
    pub fn draw(
        &self,
        rect: Rect,
        scrub: &mut Option<VectorScrub>,
        capture: &mut DragCapture,
        id: DragId,
        ctx: &mut DrawContext,
    ) -> VectorFieldOutput {
        ctx.push_debug_scope_rect("VectorField", rect);
        let input = ctx.input;
        let s = ctx.styles();
        let mut out = VectorFieldOutput {
            changed: Vec::new(),
        };
        // Pointer over the cell area asks for the resize cursor (before the
        // `draw_list` borrow; per-tag requests would need a re-borrow in the
        // loop).
        if rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed {
            ctx.request_cursor(crate::CursorIcon::ResizeHorizontal);
        }
        let list = &mut *ctx.draw_list;

        let font_size = s.scalar(StyleKey::FontSize) * 0.85;
        let cell_h = 20.0f32.max(font_size + 7.0);
        let tag_w = 15.0;
        let gap = 4.0;
        let label_w = 56.0f32.min(rect.width * 0.3);
        let row_w = rect.width - label_w;
        let cell_w = ((row_w - gap * 2.0) / 3.0).max(20.0);
        let radius = s.scalar(StyleKey::BorderRadius);

        // Release first so an up frame ends the scrub.
        if !input.mouse_down {
            capture.release(id);
            let _ = scrub.take();
        }

        for (ri, (label, vec)) in self.rows.iter().enumerate() {
            let row_y = rect.y + ri as f32 * (cell_h + gap);

            // Row label.
            let dim = s.color(StyleKey::TextDim);
            let ty =
                list.vcentered_text_y(row_y, cell_h, font_size, s.theme().font.as_ref(), label);
            list.text(
                TextBlock::new(*label, rect.x, ty)
                    .with_size(font_size)
                    .with_color(
                        (dim[0] * 255.0) as u8,
                        (dim[1] * 255.0) as u8,
                        (dim[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone()),
            );

            for ci in 0..3 {
                let cx = rect.x + label_w + ci as f32 * (cell_w + gap);
                let cell = Rect::new(cx, row_y, cell_w, cell_h);
                let active = matches!(scrub, Some(sc) if sc.row == ri && sc.component == ci);

                // Well cell (accent border while scrubbing).
                material::draw_well_simple(list, &s, cell, active, false);

                // Axis tag: the tinted square on the cell's left; press starts
                // a scrub (if no other capture owns the pointer).
                let tag = Rect::new(cell.x, cell.y, tag_w, cell_h);
                let tag_hover = tag.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
                let tint = AXIS_TINTS[ci.min(2)];
                let tag_fill = if active {
                    tint
                } else {
                    material::sheen_over(tint, [0.0, 0.0, 0.0, 0.25])
                };
                list.chrome_rect(
                    Rect::new(cell.x + 1.0, cell.y + 1.0, tag_w - 1.0, cell_h - 2.0),
                    radius,
                    0.0,
                    tag_fill,
                    [0.0; 4],
                );
                // Axis glyph letter.
                let axis_label = ["X", "Y", "Z"][ci.min(2)];
                let white = [0.9569, 0.9686, 0.9804, 1.0];
                let gty =
                    list.vcentered_text_y(cell.y, cell_h, 8.0, s.theme().font.as_ref(), axis_label);
                list.text(
                    TextBlock::new(axis_label, cell.x + 4.0, gty)
                        .with_size(8.0)
                        .with_color(
                            (white[0] * 255.0) as u8,
                            (white[1] * 255.0) as u8,
                            (white[2] * 255.0) as u8,
                        )
                        .with_font_opt(s.theme().font.clone()),
                );

                // Value text (right-aligned-ish; measure then place).
                let val = vec[ci];
                let text = format!("{val:.2}");
                let (tw, _) = list.measure_text(&text, font_size, None);
                let vty = list.vcentered_text_y(
                    cell.y,
                    cell_h,
                    font_size,
                    s.theme().font.as_ref(),
                    &text,
                );
                let fg = s.color(StyleKey::Text);
                list.text(
                    TextBlock::new(text.as_str(), cell.right() - tw - 6.0, vty)
                        .with_size(font_size)
                        .with_color(
                            (fg[0] * 255.0) as u8,
                            (fg[1] * 255.0) as u8,
                            (fg[2] * 255.0) as u8,
                        )
                        .with_font_opt(s.theme().font.clone()),
                );

                // Start/continue scrub.
                if tag_hover && input.mouse_clicked && capture.is_free() {
                    capture.try_begin(id);
                    *scrub = Some(VectorScrub {
                        row: ri,
                        component: ci,
                        start_x: input.mouse_x,
                        start_value: val,
                        step: self.step,
                    });
                }
                if active && input.mouse_down {
                    if let Some(sc) = scrub {
                        let new = sc.start_value + (input.mouse_x - sc.start_x) * sc.step;
                        out.changed.push((ri, ci, new));
                    }
                }
            }
        }

        ctx.pop_debug_scope();
        out
    }
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

    fn rows() -> Vec<(&'static str, [f32; 3])> {
        vec![("Pos", [8.9, 12.9, 9.0]), ("Rot", [0.0, 45.0, 0.0])]
    }

    #[test]
    fn dragging_a_tag_scrubs_that_component_only() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut scrub: Option<VectorScrub> = None;
        let rect = Rect::new(0.0, 0.0, 260.0, 50.0);

        // Press on the first row's X tag (label 56 + 1 → the tag occupies
        // roughly x 57..71, y 1..21).
        let input = InputState {
            mouse_x: 60.0,
            mouse_y: 10.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let rows = rows();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = VectorField::new(&rows).draw(
            rect,
            &mut scrub,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(scrub.is_some(), "press claims the scrub");
        assert!(out.changed.is_empty(), "no delta on the press frame");

        // Drag right 50px → +1.0 at the default step.
        let input = InputState {
            mouse_x: 110.0,
            mouse_y: 10.0,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = VectorField::new(&rows).draw(
            rect,
            &mut scrub,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.changed.len(), 1);
        let (ri, ci, v) = &out.changed[0];
        assert_eq!((*ri, *ci), (0, 0));
        assert!(
            (v - (8.9 + 50.0 * 0.02)).abs() < 1e-3,
            "value = start + dx*step"
        );

        // Release ends the scrub.
        let input = InputState {
            mouse_x: 110.0,
            mouse_y: 10.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = VectorField::new(&rows).draw(
            rect,
            &mut scrub,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.changed.is_empty());
        assert!(scrub.is_none());
    }

    #[test]
    fn every_cell_paints_well_tag_and_value() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let mut scrub = None;
        let input = InputState::default();
        let rows = rows();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let _ = VectorField::new(&rows).draw(
            Rect::new(0.0, 0.0, 260.0, 50.0),
            &mut scrub,
            &mut capture,
            1,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        // 2 rows × 3 cells: wells + tags + row labels + values + axis glyphs.
        assert!(list.chrome_instances.len() >= 12);
        assert_eq!(list.texts.len(), 2 + 6 + 6);
    }
}
