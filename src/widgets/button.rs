//! Button widget.
//!
//! A clickable text button. By default it draws **chrome** — a rounded
//! background plus a rounded border that tracks hover/press/disabled state. The
//! [`Button::bare`] variant drops the background and border so only the label
//! shows, with a translucent hover/press overlay for feedback (handy for
//! toolbar-style or inline buttons that shouldn't look like raised controls).
//!
//! Corner rounding follows [`Theme::border_radius`]; set it to `0.0` for square
//! buttons. The border is drawn with [`DrawList::rounded_rect_outline`] so it
//! hugs the rounded background instead of squaring off its corners.
//!
//! # Example
//! ```ignore
//! if Button::new("Save").draw(rect, &mut list, &theme, &input) { save(); }
//! if Button::new("Cancel").bare().draw(rect2, &mut list, &theme, &input) { … }
//! ```

use crate::layout::Rect;
use crate::text::{TextAlign, TextBlock};
use crate::{AnimSlot, StyleKey, StyleResolver};

use super::{DrawContext, DrawList, FocusId};

/// Resolved interaction state of a button, shared by the chrome/overlay helpers.
pub(crate) struct ButtonVisual {
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
}

impl ButtonVisual {
    /// Background fill for the current state (disabled dims the idle color).
    fn bg_color(&self, s: &StyleResolver) -> [f32; 4] {
        if !self.enabled {
            let mut c = s.color(StyleKey::Button);
            c[3] = 0.5;
            c
        } else if self.pressed {
            s.color(StyleKey::ButtonPressed)
        } else if self.hovered {
            s.color(StyleKey::ButtonHover)
        } else {
            s.color(StyleKey::Button)
        }
    }
}

/// Draw a button's background + rounded border for the given state.
///
/// Shared by [`Button`] and [`ImageButton`](super::ImageButton) so both get the
/// same rounded chrome from a single place. Honors [`Theme::border_radius`]
/// (0 => square) via [`DrawList::rounded_rect`]/[`DrawList::rounded_rect_outline`].
pub(crate) fn draw_chrome(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    radius: f32,
    v: &ButtonVisual,
) {
    let bg = v.bg_color(s);
    let border_color = if v.hovered && v.enabled {
        s.color(StyleKey::Accent)
    } else {
        s.color(StyleKey::ButtonBorder)
    };
    let border_width = s.scalar(StyleKey::BorderWidth);
    draw_chrome_colors(list, rect, radius, bg, border_color, border_width);
}

/// Low-level chrome draw from already-resolved colors — the animation-aware path
/// ([`Button::draw`]) computes eased `bg`/`border_color` and calls this directly,
/// while [`draw_chrome`] resolves them discretely for the un-animated callers
/// (e.g. [`ImageButton`](super::ImageButton)).
pub(crate) fn draw_chrome_colors(
    list: &mut DrawList,
    rect: Rect,
    radius: f32,
    bg: [f32; 4],
    border_color: [f32; 4],
    border_width: f32,
) {
    // One instanced SDF rounded-rect carries fill + border. When the border is
    // disabled (`border_width == 0`) a transparent border color collapses the
    // SDF to a plain fill. `chrome_rect` falls back to immediate tessellation
    // under a rotated/scaled transform, so correctness is universal.
    let border = if border_width > 0.0 {
        border_color
    } else {
        [0.0, 0.0, 0.0, 0.0]
    };
    list.chrome_rect(rect, radius, border_width, bg, border);
}

/// Draw the bare-button feedback overlay (no background/border): a dim for
/// disabled, a darken for pressed, a subtle lighten for hover. Drawn on top of
/// whatever content the bare button shows.
pub(crate) fn draw_bare_overlay(list: &mut DrawList, rect: Rect, v: &ButtonVisual) {
    let overlay = if !v.enabled {
        [0.0, 0.0, 0.0, 0.4]
    } else if v.pressed {
        [0.0, 0.0, 0.0, 0.2]
    } else if v.hovered {
        [1.0, 1.0, 1.0, 0.08]
    } else {
        return;
    };
    list.quad(rect.x, rect.y, rect.width, rect.height, overlay);
}

/// Horizontally- and vertically-centered button label, clipped to the inner
/// width. The horizontal inset is the theme padding, but capped relative to the
/// button width so a small button (e.g. a spin-box `+`/`-` stepper) still leaves
/// room for the glyph instead of pushing it off the edge.
fn draw_label(list: &mut DrawList, s: &StyleResolver, rect: Rect, label: &str, enabled: bool) {
    let text_color = if enabled {
        s.color(StyleKey::Text)
    } else {
        s.color(StyleKey::TextDim)
    };
    let font_size = s.scalar(StyleKey::FontSize);
    let font = s.theme().font.clone();
    let inset = s.scalar(StyleKey::Padding).min(rect.width * 0.15);
    // Optically centre the label band (x-height for mixed case, cap height for
    // all-caps/numeric) — centring by font_size alone leaves the glyph low,
    // drifting to the bottom on short buttons like spin-box steppers.
    let text_y = list.vcentered_text_y(rect.y, rect.height, font_size, font.as_ref(), label);
    list.text(
        TextBlock::new(label, rect.x + inset, text_y)
            .with_size(font_size)
            .with_color(
                (text_color[0] * 255.0) as u8,
                (text_color[1] * 255.0) as u8,
                (text_color[2] * 255.0) as u8,
            )
            .with_max_width((rect.width - inset * 2.0).max(0.0))
            .with_align(TextAlign::Center)
            .with_font_opt(font),
    );
}

/// Button widget — a clickable text label with optional rounded chrome.
#[derive(Clone)]
pub struct Button {
    label: String,
    enabled: bool,
    /// Draw the background + rounded border (default true). `false` => bare.
    chrome: bool,
    /// Corner radius override for the chrome. `None` uses [`Theme::border_radius`];
    /// `Some(r)` forces that radius (e.g. `0.0` for square spin-box steppers that
    /// must sit flush against an adjacent field without rounded inner edges).
    radius: Option<f32>,
    /// When set, the button joins the Tab ring under this [`FocusId`] and can be
    /// activated by Space/Enter while focused.
    focus_id: Option<FocusId>,
    /// When set (and the [`DrawContext`] carries an `AnimationState`), the
    /// button's chrome fill + border ease between states under this id instead
    /// of switching instantly.
    anim_id: Option<u64>,
}

impl Button {
    /// A button showing `label`, drawn at a `Rect` via [`Button::draw`].
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            chrome: true,
            radius: None,
            focus_id: None,
            anim_id: None,
        }
    }

    /// Animate the chrome fill + border transitions under `id` (hover/press fade
    /// in/out over [`Theme::animation_duration`](crate::Theme::animation_duration)).
    /// Only takes effect when the [`DrawContext`] has an
    /// [`AnimationState`](crate::AnimationState) attached; otherwise the button
    /// switches instantly as before. `id` must be stable across frames.
    pub fn animated(mut self, id: u64) -> Self {
        self.anim_id = Some(id);
        self
    }

    /// Override the chrome corner radius (default: [`Theme::border_radius`]). Use
    /// `0.0` for square corners — e.g. spin-box `+`/`-` steppers that abut a
    /// field and should not round their inner edges.
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// Make the button keyboard-focusable under `id`: it joins the Tab ring,
    /// draws a focus ring while focused, and activates on Space/Enter (in
    /// addition to mouse clicks). Clicking it also moves focus to it.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// Enable/disable the button (disabled => dimmed, no hover/click).
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Drop the background + border chrome; only the label and a translucent
    /// hover/press overlay show.
    pub fn bare(mut self) -> Self {
        self.chrome = false;
        self
    }

    /// Draw the button at `rect` and return true if clicked this frame.
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> bool {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return false;
        }
        let input = ctx.input;

        // Honor layer capture so a button under a modal/popup doesn't react to
        // clicks meant for the overlay.
        let hovered =
            self.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;
        let key_activate = input.enter_pressed || input.key_space;
        let v = ButtonVisual {
            enabled: self.enabled,
            hovered,
            pressed,
        };

        // `styles()` returns an 'a-lifetimed resolver that borrows nothing of
        // `ctx`, so it stays valid across the `&mut self` `animate_color` calls
        // below and the later `&mut *ctx.draw_list` borrow.
        let s = ctx.styles();
        let radius = self.radius.unwrap_or_else(|| s.scalar(StyleKey::BorderRadius));
        // Resolve discrete target colors, then ease toward them (no-op without an
        // AnimationState/anim_id → returns the target unchanged, byte-identical).
        let target_bg = v.bg_color(&s);
        let target_border = if v.hovered && v.enabled {
            s.color(StyleKey::Accent)
        } else {
            s.color(StyleKey::ButtonBorder)
        };
        let border_width = s.scalar(StyleKey::BorderWidth);
        let (bg, border) = match self.anim_id {
            Some(id) => (
                ctx.animate_color(id, AnimSlot::Bg, target_bg),
                ctx.animate_color(id, AnimSlot::Border, target_border),
            ),
            None => (target_bg, target_border),
        };
        {
            let list = &mut *ctx.draw_list;
            if self.chrome {
                draw_chrome_colors(list, rect, radius, bg, border, border_width);
            } else {
                draw_bare_overlay(list, rect, &v);
            }
            draw_label(list, &s, rect, &self.label, self.enabled);
        }

        // Keyboard focus + Space/Enter activation (opt-in via `focusable`).
        let mut activated = clicked;
        if let Some(id) = self.focus_id {
            ctx.register_focus(id);
            if clicked {
                ctx.focus.request(id);
            }
            if ctx.focus.is_focused(id) {
                if self.enabled && key_activate {
                    activated = true;
                }
                ctx.draw_focus_ring(rect);
            }
        }

        activated
    }

    /// Draw a chrome button at a layout-computed rect. Returns true if clicked.
    ///
    /// Convenience for the common case; equivalent to
    /// `Button::new(label).enabled(enabled).draw(rect, ctx)`.
    pub fn draw_at(label: &str, rect: Rect, enabled: bool, ctx: &mut DrawContext) -> bool {
        Button::new(label).enabled(enabled).draw(rect, ctx)
    }

    /// Draw a nine-slice textured button at a layout-computed rect. Returns true if clicked.
    pub fn draw_nine_slice(
        label: &str,
        rect: Rect,
        enabled: bool,
        ctx: &mut DrawContext,
        texture_key: &str,
    ) -> bool {
        let s = ctx.styles();
        let input = ctx.input;
        let list = &mut *ctx.draw_list;
        let hovered =
            enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;

        list.nine_slice(rect.x, rect.y, rect.width, rect.height, texture_key);
        draw_bare_overlay(
            list,
            rect,
            &ButtonVisual {
                enabled,
                hovered,
                pressed,
            },
        );
        draw_label(list, &s, rect, label, enabled);

        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    fn input_at(x: f32, y: f32, down: bool, clicked: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: down,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    fn rect() -> Rect {
        Rect::new(10.0, 10.0, 100.0, 32.0)
    }

    fn with_ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    #[test]
    fn click_inside_returns_true() {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        assert!(
            Button::new("Go").draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
    }

    #[test]
    fn click_outside_returns_false() {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = Theme::default();
        let input = input_at(500.0, 500.0, true, true);
        assert!(
            !Button::new("Go").draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
    }

    #[test]
    fn disabled_never_clicks() {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        assert!(
            !Button::new("Go")
                .enabled(false)
                .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
    }

    #[test]
    fn zero_rect_draws_nothing_and_no_click() {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, true, true);
        assert!(!Button::new("Go").draw(
            Rect::new(0.0, 0.0, 0.0, 32.0),
            &mut with_ctx(&mut list, &mut focus, &theme, &input),
        ));
    }

    #[test]
    fn bare_omits_chrome_geometry() {
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);
        let mut focus = FocusState::new();
        let mut chrome = DrawList::new();
        Button::new("Go").draw(
            rect(),
            &mut with_ctx(&mut chrome, &mut focus, &theme, &input),
        );
        let mut bare = DrawList::new();
        Button::new("Go")
            .bare()
            .draw(rect(), &mut with_ctx(&mut bare, &mut focus, &theme, &input));
        assert_eq!(
            chrome.chrome_instances.len(),
            1,
            "chrome draws one instance"
        );
        assert!(bare.chrome_instances.is_empty(), "bare draws no chrome");
        assert!(
            bare.vertices.is_empty(),
            "bare idle draws no background geometry"
        );
    }

    #[test]
    fn bare_hover_adds_overlay() {
        let theme = Theme::default();
        let mut focus = FocusState::new();
        let mut idle = DrawList::new();
        Button::new("Go").bare().draw(
            rect(),
            &mut with_ctx(
                &mut idle,
                &mut focus,
                &theme,
                &input_at(0.0, 0.0, false, false),
            ),
        );
        let mut hot = DrawList::new();
        Button::new("Go").bare().draw(
            rect(),
            &mut with_ctx(
                &mut hot,
                &mut focus,
                &theme,
                &input_at(50.0, 25.0, false, false),
            ),
        );
        assert!(
            hot.chrome_instances.len() > idle.chrome_instances.len(),
            "bare hover should add an overlay quad (instanced)"
        );
    }

    #[test]
    fn draw_at_matches_builder() {
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        let mut focus = FocusState::new();
        let mut a = DrawList::new();
        let ra = Button::draw_at(
            "Go",
            rect(),
            true,
            &mut with_ctx(&mut a, &mut focus, &theme, &input),
        );
        let mut b = DrawList::new();
        let rb = Button::new("Go").draw(rect(), &mut with_ctx(&mut b, &mut focus, &theme, &input));
        assert_eq!(ra, rb);
        assert_eq!(a.vertices.len(), b.vertices.len());
    }

    // ---- Keyboard focus / activation ----

    /// An input with a keyboard edge but no mouse activity.
    fn key_input(space: bool, enter: bool) -> InputState {
        InputState {
            key_space: space,
            enter_pressed: enter,
            ..Default::default()
        }
    }

    #[test]
    fn space_activates_only_when_focused() {
        let theme = Theme::default();
        let input = key_input(true, false);
        // Unfocused: Space does nothing.
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        assert!(
            !Button::new("Go")
                .focusable(1)
                .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input)),
            "Space must not activate an unfocused button"
        );
        // Focused: Space activates.
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        focus.focus(1);
        assert!(
            Button::new("Go")
                .focusable(1)
                .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input)),
            "Space activates the focused button"
        );
    }

    #[test]
    fn enter_activates_only_when_focused() {
        let theme = Theme::default();
        let input = key_input(false, true);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        assert!(
            !Button::new("Go")
                .focusable(1)
                .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        focus.focus(1);
        assert!(
            Button::new("Go")
                .focusable(1)
                .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
    }

    #[test]
    fn disabled_button_ignores_keyboard() {
        let theme = Theme::default();
        let input = key_input(true, true);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        focus.focus(1);
        assert!(
            !Button::new("Go")
                .enabled(false)
                .focusable(1)
                .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
    }

    #[test]
    fn keyboard_ignored_without_focusable() {
        // A plain (non-focusable) button never reacts to the keyboard.
        let theme = Theme::default();
        let input = key_input(true, true);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        focus.focus(1);
        assert!(
            !Button::new("Go").draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input))
        );
    }

    #[test]
    fn focus_ring_only_when_focused() {
        let theme = Theme::default();
        let idle = input_at(0.0, 0.0, false, false);
        // Unfocused focusable button: chrome only, no ring.
        let mut unfocused = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").focusable(1).draw(
            rect(),
            &mut with_ctx(&mut unfocused, &mut focus, &theme, &idle),
        );
        // Focused: ring adds outline geometry.
        let mut focused = DrawList::new();
        let mut focus = FocusState::new();
        focus.focus(1);
        Button::new("Go").focusable(1).draw(
            rect(),
            &mut with_ctx(&mut focused, &mut focus, &theme, &idle),
        );
        assert!(
            focused.chrome_instances.len() > unfocused.chrome_instances.len(),
            "focus ring should add outline geometry when focused"
        );
    }

    #[test]
    fn click_registers_and_requests_focus() {
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go")
            .focusable(7)
            .draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input));
        assert!(
            focus.is_focused(7),
            "clicking a focusable button focuses it"
        );
    }

    #[test]
    fn style_overlay_overrides_chrome_fill() {
        use crate::{StyleKey, StyleOverlay};
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);

        // Baseline: idle fill is theme.button.
        let mut base = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").draw(rect(), &mut with_ctx(&mut base, &mut focus, &theme, &input));
        assert_eq!(base.chrome_instances[0].bg, theme.button);

        // With an overlay recoloring Button, the drawn fill follows the overlay —
        // proving the resolver seam actually reaches the widget, no theme clone.
        let mut overlay = StyleOverlay::new();
        overlay.set_color(StyleKey::Button, [0.7, 0.1, 0.2, 1.0]);
        let mut styled = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").draw(
            rect(),
            &mut DrawContext::new(&mut styled, &mut focus, &theme, &input, 800.0, 600.0)
                .with_style(&overlay),
        );
        assert_eq!(styled.chrome_instances[0].bg, [0.7, 0.1, 0.2, 1.0]);
    }

    #[test]
    fn mouse_consumed_suppresses_hover() {
        // A consumed click (e.g. a modal above) must not let the button hover/click.
        let theme = Theme::default();
        let input = InputState {
            mouse_x: 50.0,
            mouse_y: 25.0,
            mouse_down: true,
            mouse_clicked: true,
            mouse_consumed: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        assert!(
            !Button::new("Go").draw(rect(), &mut with_ctx(&mut list, &mut focus, &theme, &input)),
            "mouse_consumed must suppress the click"
        );
    }

    #[test]
    fn animated_without_state_is_byte_identical() {
        // `.animated(id)` with no AnimationState threaded must draw exactly the
        // un-animated fill — the back-compat safety net.
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);
        let mut plain = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").draw(rect(), &mut with_ctx(&mut plain, &mut focus, &theme, &input));
        let mut anim = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go")
            .animated(1)
            .draw(rect(), &mut with_ctx(&mut anim, &mut focus, &theme, &input));
        assert_eq!(plain.chrome_instances[0].bg, anim.chrome_instances[0].bg);
        assert_eq!(
            plain.chrome_instances[0].border,
            anim.chrome_instances[0].border
        );
    }

    #[test]
    fn animated_first_frame_is_target_no_pop() {
        // First sight of a hovered animated button must draw the hover color
        // directly (no fade-in from a stale/zero start).
        use crate::AnimationState;
        let theme = Theme::default();
        let hover = input_at(50.0, 25.0, false, false);
        let mut state = AnimationState::new();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut list, &mut focus, &theme, &hover, 800.0, 600.0)
                .with_animations(&mut state),
        );
        assert_eq!(list.chrome_instances[0].bg, theme.button_hover);
    }

    #[test]
    fn animated_mid_transition_is_between_states() {
        // idle frame settles the bg at `button`; a subsequent hover frame at
        // dt < duration must land strictly between `button` and `button_hover`.
        use crate::AnimationState;
        let theme = Theme::default();
        let idle = input_at(0.0, 0.0, false, false);
        let hover = input_at(50.0, 25.0, false, false);
        let mut state = AnimationState::new();

        // Frame 1: idle, settles at `button`.
        let mut l1 = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut l1, &mut focus, &theme, &idle, 800.0, 600.0)
                .with_animations(&mut state),
        );
        assert_eq!(l1.chrome_instances[0].bg, theme.button);

        // Tick a partial dt (< 0.12 default), then draw a hover frame.
        state.tick(0.04);
        let mut l2 = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut l2, &mut focus, &theme, &hover, 800.0, 600.0)
                .with_animations(&mut state),
        );
        let bg = l2.chrome_instances[0].bg;
        // Channel 0: button (0.2-ish) → button_hover; must be strictly between.
        let (lo, hi) = (theme.button[0], theme.button_hover[0]);
        let (lo, hi) = (lo.min(hi), lo.max(hi));
        assert!(
            bg[0] > lo && bg[0] < hi,
            "mid-transition bg {} should be strictly between {} and {}",
            bg[0],
            lo,
            hi
        );
    }

    #[test]
    fn animated_zero_duration_snaps() {
        // animation_duration == 0 disables easing: hover draws the target at once.
        use crate::AnimationState;
        let mut theme = Theme::default();
        theme.animation_duration = 0.0;
        let idle = input_at(0.0, 0.0, false, false);
        let hover = input_at(50.0, 25.0, false, false);
        let mut state = AnimationState::new();

        let mut l1 = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut l1, &mut focus, &theme, &idle, 800.0, 600.0)
                .with_animations(&mut state),
        );
        state.tick(0.001);
        let mut l2 = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut l2, &mut focus, &theme, &hover, 800.0, 600.0)
                .with_animations(&mut state),
        );
        assert_eq!(l2.chrome_instances[0].bg, theme.button_hover);
    }
}
