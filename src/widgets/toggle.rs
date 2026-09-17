//! Toggle switch — the design's slide switch (Gallery III "toggle").
//!
//! A 28×15 pill: sunken dark trough when off, solid accent face when on, with
//! a light circular knob sliding between the ends. Click (or Space/Enter when
//! focused) flips it.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::material::draw_inset_shadow;
use super::{DrawContext, DrawList, FocusId};

/// Width of the switch pill, in pixels.
const SWITCH_W: f32 = 28.0;
/// Height of the switch pill, in pixels.
const SWITCH_H: f32 = 15.0;

/// A toggle switch widget. Persistent state (`on`) is caller-owned.
#[derive(Clone)]
pub struct Toggle {
    /// Stable focus id for the Tab ring / keyboard activation.
    focus_id: Option<FocusId>,
    /// Optional label drawn to the right of the switch.
    label: Option<String>,
}

impl Default for Toggle {
    fn default() -> Self {
        Self::new()
    }
}

impl Toggle {
    /// A bare switch.
    pub fn new() -> Self {
        Self {
            focus_id: None,
            label: None,
        }
    }

    /// Join the Tab ring under `id`; Space/Enter toggles while focused.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// Draw a label to the right of the switch.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Natural size: the switch, plus the label when set.
    pub fn intrinsic_size(&self, list: &mut DrawList, s: &StyleResolver) -> (f32, f32) {
        let mut w = SWITCH_W;
        if let Some(label) = &self.label {
            let block = s.text_block(label, 0.0, 0.0);
            w += 6.0 + list.measure_block(&block).0;
        }
        (w, SWITCH_H.max(s.scalar(StyleKey::FontSize)))
    }

    /// Draw the switch; returns the new state.
    pub fn draw(&self, on: bool, rect: Rect, ctx: &mut DrawContext) -> bool {
        ctx.push_debug_scope_rect(
            crate::widgets::scope_name("Toggle", self.label.as_deref().unwrap_or("")),
            rect,
        );
        let input = ctx.input;
        let s = ctx.styles();
        let switch_rect = Rect::new(
            rect.x,
            rect.y,
            SWITCH_W.min(rect.width),
            SWITCH_H.min(rect.height),
        );

        // Hit test the whole allocation (switch + label row).
        let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        if hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        let clicked = hovered && input.mouse_clicked;

        let list = &mut *ctx.draw_list;
        let radius = SWITCH_H * 0.5;
        if on {
            let face = s.color(StyleKey::AccentFaceBottom);
            list.chrome_rect(switch_rect, radius, 1.0, face, [0.0, 0.0, 0.0, 0.65]);
        } else {
            let fill = s.color(StyleKey::InputBackground);
            list.chrome_rect(switch_rect, radius, 1.0, fill, [0.0, 0.0, 0.0, 0.65]);
            // Inset shadow (the sunken off state).
            draw_inset_shadow(
                list,
                &s,
                switch_rect,
                s.scalar(StyleKey::InnerShadowDepth),
                1.0,
            );
        }

        // Knob: a light disc riding the active end. Design: white→#c9d1d8
        // gradient + black edge; we draw disc + outline.
        let knob_d = SWITCH_H - 6.0;
        let margin = 2.0;
        let knob_x = if on {
            switch_rect.x + switch_rect.width - knob_d - margin
        } else {
            switch_rect.x + margin
        };
        let cy = switch_rect.y + switch_rect.height * 0.5;
        let (lo, hi) = (0.7882f32, 1.0f32);
        let mut knob = s.color(StyleKey::Text);
        knob[0] = lo + (hi - lo) * knob[0];
        list.circle((knob_x + knob_d * 0.5, cy), knob_d * 0.5, knob);
        list.circle_outline(
            (knob_x + knob_d * 0.5, cy),
            knob_d * 0.5,
            1.0,
            [0.0, 0.0, 0.0, 0.6],
        );

        // Optional label.
        if let Some(label) = &self.label {
            let font_size = s.scalar(StyleKey::FontSize);
            let text_color = s.color(StyleKey::Text);
            let text_y = list.vcentered_text_y(
                rect.y,
                rect.height,
                font_size,
                s.theme().font.as_ref(),
                label,
            );
            list.text(
                TextBlock::new(label.as_str(), switch_rect.right() + 6.0, text_y)
                    .with_size(font_size)
                    .with_color(
                        (text_color[0] * 255.0) as u8,
                        (text_color[1] * 255.0) as u8,
                        (text_color[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone()),
            );
        }

        // Keyboard focus + activation.
        let mut toggled = clicked;
        if let Some(id) = self.focus_id {
            ctx.register_focus(id);
            if clicked {
                ctx.focus.request(id);
            }
            if ctx.focus.is_focused(id) {
                if input.nav.confirm {
                    toggled = true;
                }
                ctx.draw_focus_ring(switch_rect);
            }
        }

        ctx.pop_debug_scope();
        toggled != on
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

    fn rect() -> Rect {
        Rect::new(10.0, 10.0, 60.0, 20.0)
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    #[test]
    fn click_flips_the_state() {
        let theme = Theme::default();
        let input = click_at(12.0, 12.0);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let on = Toggle::new().draw(
            false,
            rect(),
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(on, "clicking an off toggle turns it on");
    }

    #[test]
    fn draw_is_pure_no_state_leak() {
        // Same input, same output geometry — the widget keeps no state.
        let theme = Theme::default();
        let input = InputState::default();
        let mut a = DrawList::new();
        let mut focus = FocusState::new();
        Toggle::new().draw(true, rect(), &mut ctx(&mut a, &mut focus, &theme, &input));
        let mut b = DrawList::new();
        let mut focus = FocusState::new();
        Toggle::new().draw(true, rect(), &mut ctx(&mut b, &mut focus, &theme, &input));
        assert_eq!(a.chrome_instances.len(), b.chrome_instances.len());
    }

    #[test]
    fn on_switch_paints_accent_trough_off_paints_sunken() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut on_list = DrawList::new();
        let mut focus = FocusState::new();
        Toggle::new().draw(
            true,
            rect(),
            &mut ctx(&mut on_list, &mut focus, &theme, &input),
        );
        let mut off_list = DrawList::new();
        let mut focus = FocusState::new();
        Toggle::new().draw(
            false,
            rect(),
            &mut ctx(&mut off_list, &mut focus, &theme, &input),
        );
        assert_eq!(
            on_list.chrome_instances[0].bg, on_list.chrome_instances[0].bg,
            "sanity"
        );
        assert_ne!(
            on_list.chrome_instances[0].bg, off_list.chrome_instances[0].bg,
            "on trough is the accent face, off is the sunken well"
        );
        assert!(
            off_list.chrome_instances.len() > on_list.chrome_instances.len(),
            "off draws the extra inset-shadow band"
        );
    }

    #[test]
    fn keyboard_toggles_only_when_focused() {
        let theme = Theme::default();
        let input = InputState {
            nav: crate::NavInput {
                confirm: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toggle::new().focusable(9).draw(false, rect(), &mut c);
        assert!(!out, "unfocused Space does nothing");

        // Focus it via a click, then Space toggles.
        let input = InputState {
            mouse_x: 12.0,
            mouse_y: 12.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        let _ = Toggle::new().focusable(9).draw(false, rect(), &mut c);
        assert!(focus.is_focused(9));
        let input = InputState {
            nav: crate::NavInput {
                confirm: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toggle::new().focusable(9).draw(false, rect(), &mut c);
        assert!(out, "focused Space toggles");
    }
}
