//! Splitter — the design's draggable pane divider (Gallery II "splitter").
//!
//! A vertical or horizontal grip strip between two panes: a dark inset bar
//! with a 1px center grip line that lights up (accent, with a glow) while
//! dragged. Drag is captured through the caller-owned
//! [`DragCapture`](super::DragCapture), so only one splitter owns the pointer
//! at a time.

use super::drag::{DragCapture, DragId};
use crate::layout::Rect;
use crate::style::StyleKey;

use super::DrawContext;

/// Orientation of the divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// Vertical bar; dragging moves the split left/right (col-resize).
    Vertical,
    /// Horizontal bar; dragging moves the split up/down (row-resize).
    Horizontal,
}

/// Outcome of drawing a [`Splitter`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitterOutput {
    /// The pointer delta along the split axis this frame (positive = right /
    /// down), while the splitter owns the drag. `0.0` otherwise.
    pub delta: f32,
    /// Whether this splitter owns the active drag.
    pub dragging: bool,
}

/// A pane divider. Draw between two panes; the caller sizes the bar via the
/// rect handed to [`Splitter::draw`] (the constructors' `thickness` argument
/// documents the intended floor).
#[derive(Clone, Copy)]
pub struct Splitter {
    axis: SplitAxis,
}

impl Splitter {
    /// A vertical divider (drag moves the split left/right).
    pub fn vertical(_thickness: f32) -> Self {
        Self {
            axis: SplitAxis::Vertical,
        }
    }

    /// A horizontal divider (drag moves the split up/down).
    pub fn horizontal(_thickness: f32) -> Self {
        Self {
            axis: SplitAxis::Horizontal,
        }
    }

    /// Draw the splitter at `rect`; the widget only reports deltas, the caller
    /// applies them to its pane sizes.
    pub fn draw(
        &self,
        id: DragId,
        capture: &mut DragCapture,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> SplitterOutput {
        ctx.push_debug_scope_rect("Splitter", rect);
        let input = ctx.input;
        let s = ctx.styles();

        let (px, py) = (input.mouse_x, input.mouse_y);
        let hovered = rect.contains(px, py) && !input.mouse_consumed;
        let grab_axis = match self.axis {
            SplitAxis::Vertical => crate::CursorIcon::ResizeHorizontal,
            SplitAxis::Horizontal => crate::CursorIcon::ResizeVertical,
        };

        if !input.mouse_down {
            capture.release(id);
        }
        let claimed_now = hovered && input.mouse_clicked && capture.is_free();
        if claimed_now {
            capture.try_begin(id);
        }
        let dragging = capture.is_active(id);

        if dragging {
            ctx.request_cursor(crate::CursorIcon::Grabbing);
        } else if hovered {
            ctx.request_cursor(grab_axis);
        }

        // Bar: dark inset slab; while dragging it lightens and the grip line
        // goes accent.
        let base = if dragging {
            let mut c = s.color(StyleKey::ButtonPressed);
            c[3] = 0.9;
            c
        } else {
            s.color(StyleKey::Plinth)
        };
        let under = s.color(StyleKey::EdgeShadow);
        let grip_color = if dragging {
            s.color(StyleKey::Accent)
        } else if hovered {
            s.color(StyleKey::TextDim)
        } else {
            let c = s.color(StyleKey::Text);
            [c[0], c[1], c[2], 0.28]
        };
        {
            let list = &mut *ctx.draw_list;
            let radius = s.scalar(StyleKey::BorderRadius);
            list.chrome_rect(rect, radius, 1.0, base, [0.0, 0.0, 0.0, 0.6]);
            // Edge highlights along the bar (inset light/dark pair).
            match self.axis {
                SplitAxis::Vertical => {
                    list.quad(rect.x, rect.y, 1.0, rect.height, [0.0, 0.0, 0.0, 0.6]);
                    list.quad(rect.x + 1.0, rect.y, 1.0, rect.height, under);
                }
                SplitAxis::Horizontal => {
                    list.quad(rect.x, rect.y, rect.width, 1.0, [0.0, 0.0, 0.0, 0.6]);
                    list.quad(rect.x, rect.y + 1.0, rect.width, 1.0, under);
                }
            }

            // Center grip line.
            match self.axis {
                SplitAxis::Vertical => {
                    let cx = rect.x + rect.width * 0.5;
                    list.quad(cx, rect.y + rect.height * 0.5 - 11.0, 1.0, 22.0, grip_color);
                }
                SplitAxis::Horizontal => {
                    let cy = rect.y + rect.height * 0.5;
                    list.quad(rect.x + rect.width * 0.5 - 11.0, cy, 22.0, 1.0, grip_color);
                }
            }
        }

        // Pointer delta along the axis while we own the drag. `drag_delta` is
        // the DragTracker-computed per-frame movement.
        let delta = if dragging {
            match self.axis {
                SplitAxis::Vertical => input.drag_delta[0],
                SplitAxis::Horizontal => input.drag_delta[1],
            }
        } else {
            0.0
        };

        ctx.pop_debug_scope();
        SplitterOutput { delta, dragging }
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

    #[test]
    fn drag_captures_and_reports_deltas() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let rect = Rect::new(100.0, 0.0, 5.0, 200.0);

        // Press inside: claim.
        let input = InputState {
            mouse_x: 102.0,
            mouse_y: 50.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            rect,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.dragging, "press on the bar claims the drag");

        // Move 6px right while held.
        let input = InputState {
            mouse_x: 108.0,
            mouse_y: 50.0,
            mouse_down: true,
            is_dragging: true,
            drag_delta: [6.0, 0.0],
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            rect,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.dragging);
        assert_eq!(out.delta, 6.0);

        // Release: drag ends.
        let input = InputState {
            mouse_x: 108.0,
            mouse_y: 50.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            rect,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(!out.dragging && out.delta == 0.0);
    }

    #[test]
    fn a_second_splitter_cannot_steal_an_active_drag() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let a = Rect::new(0.0, 0.0, 5.0, 200.0);
        let b = Rect::new(50.0, 0.0, 5.0, 200.0);

        let press = InputState {
            mouse_x: 2.0,
            mouse_y: 30.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            a,
            &mut ctx(&mut list, &mut focus, &theme, &press),
        );
        assert!(out.dragging);

        // Splitter B sees the same held pointer but must not report dragging.
        let input = InputState {
            mouse_x: 52.0,
            mouse_y: 30.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = Splitter::vertical(5.0).draw(
            2,
            &mut capture,
            b,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(!out.dragging, "the capture is owned by splitter A");
    }
}
