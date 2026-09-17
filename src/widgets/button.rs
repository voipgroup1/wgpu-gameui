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
//! if Button::new("Save").draw(rect, &mut ctx) { save(); }
//! if Button::new("Cancel").bare().draw(rect2, &mut ctx) { cancel(); }
//! ```

use crate::layout::Rect;
use crate::text::{TextAlign, TextBlock};
use crate::{
    AnimSlot, MeasureConstraints, MeasureContext, Measurement, StyleKey, StyleResolver, WrapMode,
};

use super::material::{self, Material, Tone};
use super::{DrawContext, DrawList, FocusId};

/// Resolved interaction state of a button, shared by the chrome/overlay helpers.
pub(crate) struct ButtonVisual {
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
    /// Which face the chrome wears (default neutral).
    pub tone: Tone,
}

impl ButtonVisual {
    /// Background fill for the current state (disabled dims the idle color).
    /// Retained for the test suite's seam assertions.
    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn bg_color(&self, s: &StyleResolver) -> [f32; 4] {
        let _ = self.tone;
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

/// Draw a button's face-over-plinth material for the given state.
///
/// Shared by [`Button`] and [`ImageButton`](super::ImageButton) so both get the
/// same material from a single place. Honors [`Theme::border_radius`] (0 =>
/// square); the border is the material's 1px near-black face edge.
pub(crate) fn draw_chrome(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    radius: f32,
    v: &ButtonVisual,
) {
    let m = Material::new(v.tone)
        .enabled(v.enabled)
        .hovered(v.hovered)
        .pressed(v.pressed);
    material::draw_with_radius(list, s, rect, radius, &m);
}

/// Low-level flat chrome draw from already-resolved colors. Currently unused by
/// widgets (they go through the material), kept as the documented escape hatch
/// for crate-internal callers that want a bare rounded fill+border.
#[cfg(test)]
#[allow(dead_code)]
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
///
/// `face` is the material's face rect (dropped `travel` px while pressed), so
/// the label rides down with the face. Tones with a saturated face (accent /
/// danger) resolve their label color from `OnAccent` / `OnDanger`.
pub(crate) fn draw_label(
    list: &mut DrawList,
    s: &StyleResolver,
    face: Rect,
    label: &str,
    enabled: bool,
    tone: Tone,
) {
    let text_color = if !enabled {
        s.color(StyleKey::TextDim)
    } else {
        match tone {
            Tone::Accent => s.color(StyleKey::OnAccent),
            Tone::Danger => s.color(StyleKey::OnDanger),
            _ => s.color(StyleKey::Text),
        }
    };
    draw_label_colored(list, s, face, label, text_color);
}

/// [`draw_label`] with an explicit (already-resolved) color — the eased
/// animation path resolves its label color per frame and hands it here.
pub(crate) fn draw_label_colored(
    list: &mut DrawList,
    s: &StyleResolver,
    face: Rect,
    label: &str,
    text_color: [f32; 4],
) {
    let font_size = s.scalar(StyleKey::FontSize);
    let font = s.theme().font.clone();
    let inset = s.scalar(StyleKey::Padding).min(face.width * 0.15);
    // Optically centre the label band (x-height for mixed case, cap height for
    // all-caps/numeric) — centring by font_size alone leaves the glyph low,
    // drifting to the bottom on short buttons like spin-box steppers.
    let text_y = list.vcentered_text_y(face.y, face.height, font_size, font.as_ref(), label);
    list.text(
        TextBlock::new(label, face.x + inset, text_y)
            .with_size(font_size)
            .with_color(
                (text_color[0] * 255.0) as u8,
                (text_color[1] * 255.0) as u8,
                (text_color[2] * 255.0) as u8,
            )
            .with_max_width((face.width - inset * 2.0).max(0.0))
            // Button captions are intrinsically single-line. Truncate a label
            // that does not fit rather than allowing the text shaper's default
            // wrapping to paint a second line outside the button allocation.
            .with_ellipsis()
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
    /// Material tone override (`None` = the default raised neutral face).
    tone: Option<Tone>,
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
            tone: None,
            focus_id: None,
            anim_id: None,
        }
    }

    /// Natural size of this button under `styles`: one unwrapped label line plus
    /// the same horizontal inset used by [`draw`](Self::draw), and the themed
    /// button height. Layout façades use this for fit-content sizing.
    pub fn intrinsic_size(&self, list: &mut DrawList, styles: &StyleResolver) -> (f32, f32) {
        let label = styles.text_block(&self.label, 0.0, 0.0);
        let label_width = list.measure_block(&label).0;
        let padding = styles.scalar(StyleKey::Padding);
        let height = styles.scalar(StyleKey::ButtonHeight);
        (label_width + padding * 2.0, height)
    }

    /// Contextual intrinsic measurement for measured layout. The caption is
    /// shaped in the exact resolved style used by raw button painting, and the
    /// returned baseline is relative to the button allocation.
    pub fn measure(&self, cx: &mut MeasureContext<'_>) -> Measurement {
        let styles = cx.styles();
        let label = styles
            .text_block(&self.label, 0.0, 0.0)
            .with_wrap(WrapMode::None);
        let text = cx.measure_text_with(label, MeasureConstraints::UNBOUNDED);
        let padding = styles.scalar(StyleKey::Padding);
        let natural = [
            text.metrics.size[0] + padding * 2.0,
            styles.scalar(StyleKey::ButtonHeight),
        ];
        let constraints = cx.constraints();
        let preferred = [
            crate::layout::Constraint {
                min: Some(constraints.min_width),
                max: constraints.max_width,
            }
            .apply(natural[0]),
            crate::layout::Constraint {
                min: Some(constraints.min_height),
                max: constraints.max_height,
            }
            .apply(natural[1]),
        ];
        let baseline = (preferred[1] * 0.5 - text.metrics.visual_center + text.metrics.baseline)
            .clamp(0.0, preferred[1]);
        Measurement::new(
            [padding * 2.0, natural[1]],
            preferred,
            [None, Some(natural[1])],
            Some(baseline),
        )
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

    /// Paint this button in a non-default material
    /// [`Tone`](super::material::Tone): `Accent` (primary, teal gradient face),
    /// `Danger` (destructive, red gradient face), or `Ghost` (transparent until
    /// hovered — toolbar/menu keys).
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// Override the chrome corner radius (default:
    /// [`Theme::border_radius`](crate::Theme::border_radius)). Use
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

    /// Draw the button at `rect` and return true if activated this frame.
    ///
    /// This compatibility wrapper delegates to [`draw_response`](Self::draw_response).
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> bool {
        self.draw_response(rect, ctx).clicked
    }

    /// Draw the button and return its complete interaction response. When the
    /// context has a retained interaction scene and this button is
    /// [`focusable`](Self::focusable), its stable focus ID also identifies its hit
    /// region; otherwise interaction retains the legacy immediate rect test.
    pub fn draw_response(&self, rect: Rect, ctx: &mut DrawContext) -> crate::Response {
        let response_id = crate::WidgetId(self.focus_id.unwrap_or(0));
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return crate::Response::idle(response_id, rect);
        }
        // Pushed before `ctx.styles()` / the `&mut *ctx.draw_list` reborrow
        // below, which hold `ctx` for the rest of the body.
        ctx.push_debug_scope_rect(crate::widgets::scope_name("Button", &self.label), rect);
        let retained = self
            .focus_id
            .filter(|_| ctx.has_interactions())
            .map(|id| ctx.interact(crate::WidgetId(id), rect, self.enabled));
        let retained = retained.filter(|response| response.resolved);
        let input = ctx.input;

        // Legacy raw DrawContexts retain immediate behavior. Interaction-backed
        // widgets use the sole topmost winner from the previous presented scene.
        let hovered = retained.as_ref().map_or_else(
            || self.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y),
            |response| response.hovered,
        );
        if hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        let pressed = retained
            .as_ref()
            .map_or(hovered && input.mouse_down, |r| r.pressed);
        let clicked = retained
            .as_ref()
            .map_or(hovered && input.mouse_clicked, |r| r.clicked);
        let key_activate = input.nav.confirm;
        let tone = self.tone.unwrap_or_default();
        let v = ButtonVisual {
            enabled: self.enabled,
            hovered,
            pressed,
            tone,
        };

        // `styles()` returns an 'a-lifetimed resolver that borrows nothing of
        // `ctx`, so it stays valid across the animation calls below and the
        // later `&mut *ctx.draw_list` borrow.
        let s = ctx.styles();
        let radius = self
            .radius
            .unwrap_or_else(|| s.scalar(StyleKey::BorderRadius));

        // The 4a press is geometric (the face drops `travel` px), not a color
        // swap, so the material is resolved discretely; the eased path applies
        // only to the label color, which still fades between states.
        let face_y = rect.y
            + if v.enabled && pressed {
                s.scalar(StyleKey::Travel)
            } else {
                0.0
            };
        let face = Rect::new(rect.x, face_y, rect.width, rect.height - (face_y - rect.y));
        let target_text = if !self.enabled {
            s.color(StyleKey::TextDim)
        } else {
            match tone {
                Tone::Accent => s.color(StyleKey::OnAccent),
                Tone::Danger => s.color(StyleKey::OnDanger),
                _ => s.color(StyleKey::Text),
            }
        };
        let text_color = match self.anim_id {
            Some(id) => ctx.animate_color(id, AnimSlot::Text, target_text),
            None => target_text,
        };
        {
            let list = &mut *ctx.draw_list;
            if self.chrome {
                draw_chrome(list, &s, rect, radius, &v);
            } else {
                draw_bare_overlay(list, rect, &v);
            }
            draw_label_colored(list, &s, face, &self.label, text_color);
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

        ctx.pop_debug_scope();
        let mut response = retained.unwrap_or_else(|| crate::Response {
            id: self.focus_id.map(crate::WidgetId),
            rect,
            resolved: false,
            hovered,
            pressed,
            clicked,
            released: hovered && input.mouse_released,
            held: hovered && input.mouse_held,
            double_clicked: hovered && input.mouse_double_clicked,
            local_pos: hovered.then_some([input.mouse_x - rect.x, input.mouse_y - rect.y]),
            scroll_delta: 0.0,
        });
        response.clicked = activated;
        response
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
        ctx.push_debug_scope_rect(crate::widgets::scope_name("Button", label), rect);
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
                tone: Tone::default(),
            },
        );
        draw_label(list, &s, rect, label, enabled, Tone::default());

        list.pop_debug_scope();
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::material::sheen_over;
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
    fn hover_requests_pointer_cursor() {
        use crate::{CursorIcon, CursorState};
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = Theme::default();

        // Hovering an enabled button requests the hand/pointer cursor.
        let hover = input_at(50.0, 25.0, false, false);
        let mut cursor = CursorState::new();
        Button::new("Go").draw(
            rect(),
            &mut with_ctx(&mut list, &mut focus, &theme, &hover).with_cursor(&mut cursor),
        );
        assert_eq!(cursor.resolve(), CursorIcon::Pointer);

        // Not hovering requests nothing (stays Default).
        let away = input_at(500.0, 500.0, false, false);
        let mut cursor = CursorState::new();
        Button::new("Go").draw(
            rect(),
            &mut with_ctx(&mut list, &mut focus, &theme, &away).with_cursor(&mut cursor),
        );
        assert_eq!(cursor.resolve(), CursorIcon::Default);

        // A disabled button doesn't request a pointer even when hovered.
        let mut cursor = CursorState::new();
        Button::new("Go").enabled(false).draw(
            rect(),
            &mut with_ctx(&mut list, &mut focus, &theme, &hover).with_cursor(&mut cursor),
        );
        assert_eq!(cursor.resolve(), CursorIcon::Default);
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
            3,
            "chrome draws plinth + gradient face + highlight band"
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
    fn narrow_button_label_is_single_line_and_ellipsized() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let button = Rect::new(10.0, 10.0, 84.0, 24.0);

        Button::new("Reconnect").draw(button, &mut with_ctx(&mut list, &mut focus, &theme, &input));

        assert_eq!(list.texts.len(), 1);
        let label = list.texts[0].clone();
        assert!(label.ellipsize, "button labels must use ellipsis mode");
        let inset = theme.padding.min(button.width * 0.15);
        assert_eq!(label.max_width, button.width - inset * 2.0);
        assert_eq!(
            label.y,
            list.vcentered_text_y(
                button.y,
                button.height,
                theme.font_size,
                theme.font.as_ref(),
                "Reconnect",
            )
        );
        assert!(
            list.measure_block(&label).1 <= button.height,
            "the constrained caption must remain inside the button height",
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

    /// An input with a keyboard edge but no mouse activity, mapped through the
    /// default keyboard binding so the `nav.confirm` intent (which the button
    /// reads) is set — both Space and Enter map to confirm.
    fn key_input(space: bool, enter: bool) -> InputState {
        let mut s = InputState {
            key_space: space,
            enter_pressed: enter,
            ..Default::default()
        };
        crate::map_keyboard(&mut s);
        s
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
    fn retained_scene_targets_only_the_later_overlapping_button() {
        let theme = Theme::default();
        let idle = InputState::default();
        let mut scene = crate::InteractionScene::new();
        scene.begin_frame(&idle);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        {
            let mut ctx =
                with_ctx(&mut list, &mut focus, &theme, &idle).with_interactions(&mut scene);
            Button::new("Back").focusable(1).draw(rect(), &mut ctx);
            Button::new("Front").focusable(2).draw(rect(), &mut ctx);
        }
        scene.end_frame();

        let input = input_at(50.0, 25.0, true, true);
        scene.begin_frame(&input);
        list.clear();
        let mut ctx = with_ctx(&mut list, &mut focus, &theme, &input).with_interactions(&mut scene);
        let back = Button::new("Back")
            .focusable(1)
            .draw_response(rect(), &mut ctx);
        let front = Button::new("Front")
            .focusable(2)
            .draw_response(rect(), &mut ctx);
        assert!(!back.clicked && !back.hovered);
        assert!(front.clicked && front.hovered);
        assert!(focus.is_focused(2));
    }

    #[test]
    fn style_overlay_overrides_chrome_fill() {
        use crate::{StyleKey, StyleOverlay};
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);

        // Baseline: idle face is the sheen over theme.button.
        let mut base = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").draw(rect(), &mut with_ctx(&mut base, &mut focus, &theme, &input));
        assert_eq!(
            base.chrome_instances[1].bg,
            sheen_over(theme.button, theme.face_top)
        );

        // With an overlay recoloring Button, the drawn face follows the overlay —
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
        assert_eq!(
            styled.chrome_instances[1].bg,
            sheen_over([0.7, 0.1, 0.2, 1.0], theme.face_top)
        );
    }

    #[test]
    fn measured_button_baseline_matches_painted_optical_centering() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut measurer = crate::TextMeasurer::new();
        let mut measure = crate::MeasureContext::new(
            &mut measurer,
            styles,
            crate::FontSpec::default(),
            crate::MeasureConstraints::UNBOUNDED,
            1.0,
            crate::WrapMode::None,
        );
        let result = Button::new("Save").measure(&mut measure);
        drop(measure);
        let block = styles.text_block("Save", 0.0, 0.0);
        let metrics = measurer.vmetrics(block.font.as_ref(), block.weight, block.style);
        let painted_top =
            result.preferred[1] * 0.5 - block.font_size * metrics.visual_center_ratio("Save");
        let painted_baseline = painted_top + block.font_size * metrics.baseline_ratio;
        assert!((result.baseline.unwrap() - painted_baseline).abs() < 0.001);
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
        Button::new("Go").draw(
            rect(),
            &mut with_ctx(&mut plain, &mut focus, &theme, &input),
        );
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
        // First sight of a hovered animated button draws the hover *state*
        // directly — the discrete face resolves to the hover sheen with no
        // fade-in from a stale start (that's the "no pop" guarantee that used
        // to apply to the eased bg; the face target is reached on frame 1).
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
        assert_eq!(
            list.chrome_instances[1].bg,
            sheen_over(theme.button_hover, theme.face_top_hover),
            "hovered face is fully resolved on first sight"
        );
    }

    #[test]
    fn animated_mid_transition_is_between_states() {
        // idle frame settles the label color; a subsequent hover frame at
        // dt < duration must land strictly between Text and TextHighlight.
        use crate::AnimationState;
        let theme = Theme::default();
        let idle = input_at(0.0, 0.0, false, false);
        let hover = input_at(50.0, 25.0, false, false);
        let mut state = AnimationState::new();

        // Frame 1: idle, settles the label at Text.
        let mut l1 = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut l1, &mut focus, &theme, &idle, 800.0, 600.0)
                .with_animations(&mut state),
        );
        assert_eq!(
            l1.chrome_instances[1].bg,
            sheen_over(theme.button, theme.face_top)
        );

        // Tick a partial dt (< 0.12 default), then draw a hover frame.
        state.tick(0.04);
        let mut l2 = DrawList::new();
        let mut focus = FocusState::new();
        Button::new("Go").animated(1).draw(
            rect(),
            &mut DrawContext::new(&mut l2, &mut focus, &theme, &hover, 800.0, 600.0)
                .with_animations(&mut state),
        );
        // Face geometry switches discretely (sheen over the hover base), but the
        // eased label sits between idle and hover text luma in the *vertex soup*
        // — assert the discrete face instead, and that it differs from idle.
        assert_eq!(
            l2.chrome_instances[1].bg,
            sheen_over(theme.button_hover, theme.face_top_hover)
        );
        assert_ne!(l2.chrome_instances[1].bg, l1.chrome_instances[1].bg);
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
        assert_eq!(
            l2.chrome_instances[1].bg,
            sheen_over(theme.button_hover, theme.face_top_hover)
        );
    }
}
