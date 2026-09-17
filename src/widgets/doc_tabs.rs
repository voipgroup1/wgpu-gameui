//! Document tabs — the design's editor tab strip (Gallery II "document tabs").
//!
//! Trapezoid-ish keys that join the sheet: the active tab is flush with the
//! content below (no bottom gap), inactive tabs sit 2px up over a sunken
//! background. Unsaved state is a small glowing accent dot; hovering a tab
//! fades in a ✕ key in place of the dot (the design's dot→close swap).

use crate::layout::Rect;
use crate::style::StyleKey;
use crate::text::TextBlock;

use super::DrawContext;
use super::material;

/// One document tab's data.
pub struct DocTab<'a> {
    /// The tab label.
    pub label: &'a str,
    /// Unsaved-changes dot.
    pub dirty: bool,
}

/// Outcome of drawing the strip.
#[derive(Debug, Clone, Copy, Default)]
pub struct DocTabsOutput {
    /// Index of a tab activated this frame (click on the tab body).
    pub activated: Option<usize>,
    /// Index of a tab whose ✕ was clicked this frame.
    pub closed: Option<usize>,
}

/// Draw the tab strip over `rect`; `active` is the selected index. Tab widths
/// split the strip evenly (like [`Tabs`](super::Tabs)); dirty dots and ✕ keys
/// sit left/right of the label.
pub fn draw(
    rect: Rect,
    tabs: &[DocTab<'_>],
    active: usize,
    ctx: &mut DrawContext,
) -> DocTabsOutput {
    ctx.push_debug_scope_rect("DocTabs", rect);
    let input = ctx.input;
    let s = ctx.styles();
    let mut out = DocTabsOutput::default();
    let list = &mut *ctx.draw_list;

    if tabs.is_empty() {
        ctx.pop_debug_scope();
        return out;
    }

    let tab_w = rect.width / tabs.len() as f32;
    let radius = s.scalar(StyleKey::BorderRadius);
    let font_size = s.scalar(StyleKey::FontSize) * 0.9;

    for (i, tab) in tabs.iter().enumerate() {
        let x = rect.x + i as f32 * tab_w;
        let is_active = i == active;
        // Active: full height, flush with the sheet below. Inactive: 2px short.
        let drop = if is_active { 0.0 } else { 2.0 };
        let face = Rect::new(x, rect.y, tab_w, rect.height - drop);
        let hovered = face.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let clicked = hovered && input.mouse_clicked;

        let base = if is_active {
            s.color(StyleKey::TabActive)
        } else if hovered {
            s.color(StyleKey::TabHover)
        } else {
            s.color(StyleKey::TabInactive)
        };
        let sheen = if is_active {
            s.color(StyleKey::FaceTopHover)
        } else {
            s.color(StyleKey::FaceTop)
        };
        let top = material::sheen_over(base, sheen);
        let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
        list.chrome_rect_gradient(
            face,
            radius,
            1.0,
            top,
            bottom,
            [0.0, 0.0, 0.0, if is_active { 0.55 } else { 0.35 }],
        );
        if is_active {
            let hl = s.color(StyleKey::EdgeHighlight);
            list.quad(
                face.x + 1.0,
                face.y + 1.0,
                (face.width - 2.0).max(0.0),
                1.0,
                hl,
            );
        }

        // Label (centered, reserving 8px each side for dot / ✕).
        let text_color = if is_active {
            s.color(StyleKey::TextHighlight)
        } else if hovered {
            s.color(StyleKey::Text)
        } else {
            s.color(StyleKey::TextDim)
        };
        let ty = list.vcentered_text_y(
            face.y,
            face.height,
            font_size,
            s.theme().font.as_ref(),
            tab.label,
        );
        let (tw, _) = list.measure_text(tab.label, font_size, None);
        list.text(
            TextBlock::new(tab.label, x + (tab_w - tw) * 0.5, ty)
                .with_size(font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_ellipsis()
                .with_max_width((tab_w - 18.0).max(12.0))
                .with_font_opt(s.theme().font.clone()),
        );

        // Right slot: ✕ when hovered (or active), else the dirty dot.
        let show_close = hovered || (is_active && tab.dirty);
        let slot = Rect::new(
            x + tab_w - 13.0,
            rect.y + (rect.height - 11.0) * 0.5,
            11.0,
            11.0,
        );
        if show_close {
            if hovered && slot.contains(input.mouse_x, input.mouse_y) {
                list.quad(
                    slot.x,
                    slot.y,
                    slot.width,
                    slot.height,
                    [1.0, 1.0, 1.0, 0.09],
                );
            }
            let xc = s.color(StyleKey::TextDim);
            let xty =
                list.vcentered_text_y(slot.y, slot.height, 10.0, s.theme().font.as_ref(), "✕");
            list.text(
                TextBlock::new("✕", slot.x + 1.0, xty)
                    .with_size(10.0)
                    .with_color(
                        (xc[0] * 255.0) as u8,
                        (xc[1] * 255.0) as u8,
                        (xc[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone()),
            );
            if hovered && slot.contains(input.mouse_x, input.mouse_y) && input.mouse_clicked {
                out.closed = Some(i);
            }
        } else if tab.dirty {
            // Glowing accent dot.
            let accent = s.color(StyleKey::Accent);
            let d = Rect::new(slot.x + 3.0, slot.y + 3.0, 5.0, 5.0);
            list.chrome_rect(d, 3.0, 0.0, accent, [0.0; 4]);
        }

        if clicked && out.closed != Some(i) {
            out.activated = Some(i);
        }
    }

    ctx.pop_debug_scope();
    out
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

    fn tabs() -> Vec<DocTab<'static>> {
        vec![
            DocTab {
                label: "Level_01",
                dirty: true,
            },
            DocTab {
                label: "Arena",
                dirty: false,
            },
        ]
    }

    #[test]
    fn clicking_a_tab_activates_and_clicking_x_closes() {
        let theme = Theme::default();
        let rect = Rect::new(0.0, 0.0, 200.0, 24.0);
        let ts = tabs();

        // Click the middle of tab 0 → activation.
        let input = InputState {
            mouse_x: 40.0,
            mouse_y: 12.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = draw(
            rect,
            &ts,
            0,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.activated, Some(0));
        assert!(out.closed.is_none());

        // Hover tab 0 then click its ✕ slot (right 11px of the tab).
        let input = InputState {
            mouse_x: 92.0,
            mouse_y: 12.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = draw(
            rect,
            &ts,
            0,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(
            out.closed,
            Some(0),
            "the ✕ slot closes instead of activating"
        );
    }

    #[test]
    fn active_tab_is_full_height_inactive_is_dropped() {
        let theme = Theme::default();
        let input = InputState {
            mouse_x: -10.0,
            mouse_y: -10.0,
            ..Default::default()
        };
        let ts = tabs();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        draw(
            Rect::new(0.0, 0.0, 200.0, 24.0),
            &ts,
            0,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        // First chrome instance is tab 0 (active, full height 24), the next is
        // tab 1 (inactive, 22).
        let h = |x: f32| {
            list.chrome_instances
                .iter()
                .find(|c| (c.rect[0] - x).abs() < 0.01 && c.rect[2] > 90.0)
                .map(|c| c.rect[3])
                .unwrap_or(0.0)
        };
        assert!((h(0.0) - 24.0).abs() < 0.01, "active tab full height");
        assert!((h(100.0) - 22.0).abs() < 0.01, "inactive tab dropped 2px");
    }
}
