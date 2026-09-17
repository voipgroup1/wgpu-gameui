//! Checkbox widget.

#[cfg(feature = "phosphor-icons")]
use crate::PhosphorIcon;
use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{AnimSlot, SpriteId, StyleKey, StyleResolver};

use super::material::draw_inset_shadow;
use super::{DrawContext, DrawList, FocusId};

/// Icon keys for checkbox textures. Only used by the string-keyed
/// [`Checkbox::with_icon_keys`] path; the default rendering is vector-drawn and
/// needs no atlas assets.
pub const CHECKBOX_ICON: &str = "textures/ui/checkbox.png";
/// Texture key for the checked checkbox box (string-keyed path only).
pub const CHECKBOX_CHECKED_ICON: &str = "textures/ui/checkbox_checked.png";

/// How the checkbox box is rendered.
#[derive(Clone)]
enum BoxStyle {
    /// Theme-driven box + checkmark drawn from vector primitives. Works with no
    /// atlas assets — the default, so a checkbox is never blank.
    Vector,
    /// Pre-resolved sprite handles `(unchecked, checked)`. A `SpriteId` is only
    /// obtained from [`crate::UiRenderer::sprite_id`] *after* the texture is
    /// registered, so this path can never reference a missing sprite.
    Sprites {
        unchecked: SpriteId,
        checked: SpriteId,
    },
    /// String-keyed textures `(unchecked, checked)`. Resolved by name at render
    /// time; if the atlas lacks the key the renderer skips it, so this path can
    /// render blank — kept only for callers that knowingly preload these keys.
    Keys {
        unchecked: &'static str,
        checked: &'static str,
    },
}

/// Checkbox widget - a toggleable box with an optional label.
///
/// By default the box is drawn from vector primitives using [`Theme`](crate::Theme) colors,
/// so it renders correctly with zero atlas assets. Callers that registered
/// checkbox textures can opt into them with [`Checkbox::with_icons`] (passing
/// pre-resolved [`SpriteId`]s) or [`Checkbox::with_icon_keys`].
///
/// # Example
/// ```ignore
/// // Vector (no assets needed):
/// if Checkbox::new().draw(checked, "Enabled", rect, &mut ctx) {
///     checked = !checked;
/// }
/// ```
#[derive(Clone)]
pub struct Checkbox {
    style: BoxStyle,
    /// When set, the checkbox joins the Tab ring under this [`FocusId`] and can
    /// be toggled by Space/Enter while focused.
    focus_id: Option<FocusId>,
    /// When set (and an [`AnimationState`](crate::AnimationState) is attached to
    /// the context), the box fill and hover highlight ease between states instead
    /// of switching instantly.
    anim_id: Option<u64>,
}

impl Default for Checkbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Checkbox {
    /// A checkbox rendered from vector primitives (no atlas assets required).
    pub fn new() -> Self {
        Self {
            style: BoxStyle::Vector,
            focus_id: None,
            anim_id: None,
        }
    }

    /// Natural row size for `label`: the themed checkbox square, label gap, and
    /// the label's actual width in the themed font.
    pub fn intrinsic_size(
        &self,
        label: &str,
        list: &mut DrawList,
        styles: &StyleResolver,
    ) -> (f32, f32) {
        let height = styles.scalar(StyleKey::FontSize).max(20.0);
        let label_width = if label.is_empty() {
            0.0
        } else {
            list.measure_block(&styles.text_block(label, 0.0, 0.0)).0 + 6.0
        };
        (height + label_width, height)
    }

    /// Smooth the box fill (check/uncheck) and hover highlight transitions using
    /// the context's [`AnimationState`](crate::AnimationState), keyed by `id`. A
    /// no-op when no animation state is attached (byte-identical to the instant
    /// path). `id` must be stable across frames for the same checkbox.
    pub fn animated(mut self, id: u64) -> Self {
        self.anim_id = Some(id);
        self
    }

    /// Make the checkbox keyboard-focusable under `id`: it joins the Tab ring,
    /// draws a focus ring while focused, and toggles on Space/Enter (in addition
    /// to mouse clicks). Clicking it also moves focus to it.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// Render with pre-resolved sprite textures instead of the vector fallback.
    ///
    /// Resolve the handles once via [`crate::UiRenderer::sprite_id`]; because a
    /// `SpriteId` only exists for a registered sprite, this path is guaranteed
    /// non-blank.
    pub fn with_icons(mut self, unchecked: SpriteId, checked: SpriteId) -> Self {
        self.style = BoxStyle::Sprites { unchecked, checked };
        self
    }

    /// Render with string-keyed textures resolved by name at render time.
    ///
    /// Only use this if you have definitely registered `unchecked`/`checked`
    /// into the atlas — a missing key renders nothing. Prefer
    /// [`Checkbox::with_icons`] (resolved handles) or the vector default.
    pub fn with_icon_keys(mut self, unchecked: &'static str, checked: &'static str) -> Self {
        self.style = BoxStyle::Keys { unchecked, checked };
        self
    }

    /// Draw a checkbox at the given rect. Returns true if clicked (toggled).
    pub fn draw(&self, checked: bool, label: &str, rect: Rect, ctx: &mut DrawContext) -> bool {
        self.draw_response(checked, label, rect, ctx).clicked
    }

    /// Draw the checkbox and return its complete interaction response. The whole
    /// allocated row is the canonical hit geometry; the focus ring still hugs
    /// only the square control.
    pub fn draw_response(
        &self,
        checked: bool,
        label: &str,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> crate::Response {
        ctx.push_debug_scope_rect(crate::widgets::scope_name("Checkbox", label), rect);
        let retained = self
            .focus_id
            .filter(|_| ctx.has_interactions())
            .map(|id| ctx.interact(crate::WidgetId(id), rect, true))
            .filter(|response| response.resolved);
        let input = ctx.input;
        let s = ctx.styles();
        // Honor layer capture (`mouse_consumed`) on the compatibility path.
        let hovered = retained.as_ref().map_or_else(
            || rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed,
            |response| response.hovered,
        );
        if hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        let clicked = retained
            .as_ref()
            .map_or(hovered && input.mouse_clicked, |response| response.clicked);
        let key_activate = input.nav.confirm;

        // Checkbox box (square, fitted to rect height).
        let size = rect.height;
        let box_rect = Rect::new(rect.x, rect.y, size, size);

        // Resolve animated values *before* borrowing `draw_list` (the
        // `animate_*` helpers take `&mut ctx`). Without an AnimationState/anim_id
        // these return their targets unchanged, so geometry is byte-identical:
        // the unchecked fill stays `InputBackground`, checked stays `Accent`, and
        // the hover quad is emitted only when its alpha is non-zero.
        let target_fill = if checked {
            s.color(StyleKey::Accent)
        } else {
            s.color(StyleKey::InputBackground)
        };
        let (fill, hover_alpha) = match self.anim_id {
            Some(id) => (
                ctx.animate_color(id, AnimSlot::Fill, target_fill),
                ctx.animate_scalar(id, AnimSlot::Overlay, if hovered { 1.0 } else { 0.0 }) * 0.08,
            ),
            None => (target_fill, if hovered { 0.08 } else { 0.0 }),
        };

        {
            let list = &mut *ctx.draw_list;
            match &self.style {
                BoxStyle::Vector => draw_vector_box(list, &s, box_rect, checked, fill),
                BoxStyle::Sprites {
                    unchecked,
                    checked: checked_id,
                } => {
                    let sprite = if checked { *checked_id } else { *unchecked };
                    list.icon_sprite(
                        sprite,
                        box_rect.x,
                        box_rect.y,
                        size,
                        size,
                        [1.0, 1.0, 1.0, 1.0],
                    );
                }
                BoxStyle::Keys {
                    unchecked,
                    checked: checked_key,
                } => {
                    let key = if checked { *checked_key } else { *unchecked };
                    list.icon(key, box_rect.x, box_rect.y, size, size);
                }
            }

            // Hover highlight over the box area (eased alpha when animated).
            if hover_alpha > 0.0 {
                list.quad(
                    box_rect.x,
                    box_rect.y,
                    size,
                    size,
                    [1.0, 1.0, 1.0, hover_alpha],
                );
            }

            // Label to the right of the checkbox.
            if !label.is_empty() {
                let text_x = rect.x + size + 6.0;
                let text_y = list.vcentered_text_y(
                    rect.y,
                    rect.height,
                    s.scalar(StyleKey::FontSize),
                    s.theme().font.as_ref(),
                    label,
                );
                let text_color = s.color(StyleKey::Text);
                let text = TextBlock::new(label, text_x, text_y)
                    .with_size(s.scalar(StyleKey::FontSize))
                    .with_color(
                        (text_color[0] * 255.0) as u8,
                        (text_color[1] * 255.0) as u8,
                        (text_color[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone());
                list.text(text);
            }
        }

        // Keyboard focus + Space/Enter toggle (opt-in via `focusable`). The ring
        // hugs the box (the control), not the wide label area.
        let mut toggled = clicked;
        if let Some(id) = self.focus_id {
            ctx.register_focus(id);
            if clicked {
                ctx.focus.request(id);
            }
            if ctx.focus.is_focused(id) {
                if key_activate {
                    toggled = true;
                }
                ctx.draw_focus_ring(box_rect);
            }
        }

        ctx.pop_debug_scope();
        let mut response = retained.unwrap_or_else(|| crate::Response {
            id: self.focus_id.map(crate::WidgetId),
            rect,
            resolved: false,
            hovered,
            pressed: hovered && input.mouse_down,
            clicked,
            released: hovered && input.mouse_released,
            held: hovered && input.mouse_held,
            double_clicked: hovered && input.mouse_double_clicked,
            local_pos: hovered.then_some([input.mouse_x - rect.x, input.mouse_y - rect.y]),
            scroll_delta: 0.0,
        });
        response.clicked = toggled;
        response
    }
}

/// Draw the theme-driven vector checkbox: a rounded box, filled with the accent
/// color and stamped with a contrast checkmark when `checked`.
fn draw_vector_box(
    list: &mut DrawList,
    s: &StyleResolver,
    box_rect: Rect,
    checked: bool,
    fill: [f32; 4],
) {
    let size = box_rect.width.min(box_rect.height);
    let radius = s.scalar(StyleKey::BorderRadius).min(size * 0.3).max(0.0);
    let border = s.scalar(StyleKey::BorderWidth).max(1.0).min(size * 0.5);

    if checked {
        // Checked = the accent face raised in the box: gradient fill (eased
        // toward the accent tokens) + a dark on-accent tick. The design's mark
        // color is fixed (`#04171d`), which `OnAccent` resolves to.
        list.chrome_rect_gradient(box_rect, radius, border, fill, fill, [0.0, 0.0, 0.0, 0.5]);
        let mark = s.color(StyleKey::OnAccent);
        // The built-in MSDF icon keeps the small checkmark smooth at every DPI.
        // Keep vector geometry as the no-feature fallback so checkbox rendering
        // never depends on an optional font asset.
        #[cfg(feature = "phosphor-icons")]
        list.phosphor_icon(box_rect.inset(size * 0.18), PhosphorIcon::Check, mark);
        #[cfg(not(feature = "phosphor-icons"))]
        {
            let t = (size * 0.14).max(1.5);
            // Tick: down-stroke into the low-left, up-stroke to the high-right.
            let pts = [
                [box_rect.x + size * 0.22, box_rect.y + size * 0.52],
                [box_rect.x + size * 0.42, box_rect.y + size * 0.72],
                [box_rect.x + size * 0.78, box_rect.y + size * 0.28],
            ];
            list.polyline(&pts, t, mark);
        }
        // 1px inset highlight under the top edge (raised-face read).
        let hl = s.color(StyleKey::EdgeHighlight);
        list.quad(
            box_rect.x + border,
            box_rect.y + border,
            (size - border * 2.0).max(0.0),
            1.0,
            hl,
        );
    } else {
        // Empty box: the sunken tone — dark fill + black edge + inset shadow.
        list.chrome_rect(box_rect, radius, border, fill, [0.0, 0.0, 0.0, 0.6]);
        draw_inset_shadow(
            list,
            s,
            box_rect,
            s.scalar(StyleKey::InnerShadowDepth),
            border,
        );
    }
}

/// Pick black or white for maximum contrast against `bg` using perceptual
/// (Rec. 709) luminance. Kept for themes that resolve a light accent.
#[cfg(test)]
fn contrast_color(bg: [f32; 4]) -> [f32; 4] {
    let lum = 0.2126 * bg[0] + 0.7152 * bg[1] + 0.0722 * bg[2];
    if lum > 0.5 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    fn theme() -> Theme {
        Theme::default()
    }

    /// Draw a checkbox into a fresh `DrawContext` and return the populated draw
    /// list (for geometry assertions) plus the click result.
    fn draw_cb(
        cb: &Checkbox,
        checked: bool,
        label: &str,
        rect: Rect,
        input: &InputState,
    ) -> (DrawList, bool) {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = theme();
        let clicked = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            cb.draw(checked, label, rect, &mut ctx)
        };
        (list, clicked)
    }

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, 120.0, 20.0)
    }

    #[test]
    fn vector_unchecked_emits_geometry() {
        // The core bug: a checkbox must never be blank without atlas assets.
        let (list, _) = draw_cb(&Checkbox::new(), false, "", rect(), &input_at(-1.0, -1.0));
        // Box fill + outline are translate-only rounded rects, so they record
        // chrome instances rather than soup geometry.
        assert!(
            !list.chrome_instances.is_empty(),
            "vector unchecked box must emit geometry (box fill + outline)"
        );
        assert!(list.icons.is_empty(), "vector path must not queue any icon");
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn vector_checked_uses_an_msdf_checkmark() {
        let (checked, _) = draw_cb(&Checkbox::new(), true, "", rect(), &input_at(-1.0, -1.0));

        assert_eq!(
            checked.icons_msdf.len(),
            1,
            "the checked mark should be rendered by the antialiased MSDF icon path"
        );
        let mark = checked.icons_msdf[0].local;
        const EPSILON: f32 = 0.001;
        assert!((mark.x - 3.6).abs() < EPSILON);
        assert!((mark.y - 3.6).abs() < EPSILON);
        assert!((mark.width - 12.8).abs() < EPSILON);
        assert!((mark.height - 12.8).abs() < EPSILON);
    }

    #[cfg(not(feature = "phosphor-icons"))]
    #[test]
    fn vector_checked_falls_back_to_checkmark_geometry() {
        let (checked, _) = draw_cb(&Checkbox::new(), true, "", rect(), &input_at(-1.0, -1.0));
        assert!(
            !checked.vertices.is_empty(),
            "no-icon builds retain a vector checkmark fallback"
        );
    }

    #[test]
    fn click_inside_toggles() {
        let (_, clicked) = draw_cb(
            &Checkbox::new(),
            false,
            "Label",
            rect(),
            &click_at(5.0, 10.0),
        );
        assert!(clicked, "a click inside the rect should report a toggle");
    }

    #[test]
    fn click_outside_does_not_toggle() {
        let (_, clicked) = draw_cb(
            &Checkbox::new(),
            false,
            "Label",
            rect(),
            &click_at(500.0, 500.0),
        );
        assert!(!clicked);
    }

    #[test]
    fn consumed_mouse_does_not_toggle() {
        let mut input = click_at(5.0, 10.0);
        input.mouse_consumed = true; // a higher layer took this click
        let (_, clicked) = draw_cb(&Checkbox::new(), false, "Label", rect(), &input);
        assert!(!clicked);
    }

    #[test]
    fn icon_keys_path_queues_icon_not_vector() {
        let cb = Checkbox::new().with_icon_keys(CHECKBOX_ICON, CHECKBOX_CHECKED_ICON);
        let (list, _) = draw_cb(&cb, true, "", rect(), &input_at(-1.0, -1.0));
        assert_eq!(list.icons.len(), 1, "icon-key path queues exactly one icon");
        assert_eq!(list.icons[0].icon_key, CHECKBOX_CHECKED_ICON);
    }

    #[test]
    fn contrast_color_picks_black_on_light_and_white_on_dark() {
        assert_eq!(contrast_color([1.0, 1.0, 1.0, 1.0]), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(contrast_color([0.0, 0.0, 0.0, 1.0]), [1.0, 1.0, 1.0, 1.0]);
    }

    // ---- Keyboard focus / activation ----

    fn key_input(space: bool, enter: bool) -> InputState {
        let mut s = InputState {
            key_space: space,
            enter_pressed: enter,
            ..Default::default()
        };
        crate::map_keyboard(&mut s); // Space/Enter → nav.confirm
        s
    }

    /// Draw with an explicitly seeded focus state.
    fn draw_focused(cb: &Checkbox, focus: &mut FocusState, input: &InputState) -> (DrawList, bool) {
        let mut list = DrawList::new();
        let theme = theme();
        let toggled = {
            let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
            cb.draw(false, "Label", rect(), &mut ctx)
        };
        (list, toggled)
    }

    #[test]
    fn space_toggles_only_when_focused() {
        let cb = Checkbox::new().focusable(1);
        // Unfocused: Space does nothing.
        let mut focus = FocusState::new();
        let (_, t) = draw_focused(&cb, &mut focus, &key_input(true, false));
        assert!(!t, "Space must not toggle an unfocused checkbox");
        // Focused: Space toggles.
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, t) = draw_focused(&cb, &mut focus, &key_input(true, false));
        assert!(t, "Space toggles the focused checkbox");
    }

    #[test]
    fn enter_toggles_only_when_focused() {
        let cb = Checkbox::new().focusable(1);
        let mut focus = FocusState::new();
        let (_, t) = draw_focused(&cb, &mut focus, &key_input(false, true));
        assert!(!t);
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, t) = draw_focused(&cb, &mut focus, &key_input(false, true));
        assert!(t);
    }

    #[test]
    fn keyboard_ignored_without_focusable() {
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, t) = draw_focused(&Checkbox::new(), &mut focus, &key_input(true, true));
        assert!(!t, "non-focusable checkbox ignores the keyboard");
    }

    #[test]
    fn focus_ring_only_when_focused() {
        let cb = Checkbox::new().focusable(1);
        let idle = input_at(-1.0, -1.0);
        let mut focus = FocusState::new();
        let (unfocused, _) = draw_focused(&cb, &mut focus, &idle);
        let mut focus = FocusState::new();
        focus.focus(1);
        let (focused, _) = draw_focused(&cb, &mut focus, &idle);
        assert!(
            focused.chrome_instances.len() > unfocused.chrome_instances.len(),
            "focus ring should add outline geometry when focused"
        );
    }

    #[test]
    fn click_requests_focus() {
        let cb = Checkbox::new().focusable(5);
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = theme();
        let input = click_at(5.0, 10.0);
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
            cb.draw(false, "Label", rect(), &mut ctx);
        }
        assert!(
            focus.is_focused(5),
            "clicking a focusable checkbox focuses it"
        );
    }

    // ---- Animation ----

    /// Find the box-fill chrome instance (the first translate-only rounded rect
    /// at the box origin).
    fn box_fill(list: &DrawList) -> [f32; 4] {
        list.chrome_instances
            .iter()
            .find(|c| c.rect[0] == 0.0 && c.rect[1] == 0.0)
            .map(|c| c.bg)
            .expect("vector box should emit a fill chrome instance at the origin")
    }

    #[test]
    fn animated_without_state_is_byte_identical() {
        let th = theme();
        let idle = input_at(-1.0, -1.0);
        let (plain, _) = draw_cb(&Checkbox::new(), true, "", rect(), &idle);
        let (anim, _) = draw_cb(&Checkbox::new().animated(1), true, "", rect(), &idle);
        assert_eq!(box_fill(&plain), box_fill(&anim));
        // Both checked boxes resolve to the accent fill.
        assert_eq!(box_fill(&anim), th.accent);
    }

    #[test]
    fn animated_fill_eases_between_checked_states() {
        use crate::AnimationState;
        let th = theme();
        let idle = input_at(-1.0, -1.0);
        let mut state = AnimationState::new();

        // Frame 1: unchecked, settles fill at InputBackground.
        let mut l1 = DrawList::new();
        let mut focus = FocusState::new();
        {
            let mut ctx = DrawContext::new(&mut l1, &mut focus, &th, &idle, 800.0, 600.0)
                .with_animations(&mut state);
            Checkbox::new()
                .animated(1)
                .draw(false, "", rect(), &mut ctx);
        }
        assert_eq!(box_fill(&l1), th.input_background);

        // Tick a partial dt then draw checked: fill must be mid-way to accent.
        state.tick(0.04);
        let mut l2 = DrawList::new();
        let mut focus = FocusState::new();
        {
            let mut ctx = DrawContext::new(&mut l2, &mut focus, &th, &idle, 800.0, 600.0)
                .with_animations(&mut state);
            Checkbox::new().animated(1).draw(true, "", rect(), &mut ctx);
        }
        let fill = box_fill(&l2);
        let (lo, hi) = (
            th.input_background[0].min(th.accent[0]),
            th.input_background[0].max(th.accent[0]),
        );
        assert!(
            fill[0] > lo && fill[0] < hi,
            "mid-transition fill {} should be strictly between {} and {}",
            fill[0],
            lo,
            hi
        );
    }

    #[test]
    fn animated_hover_overlay_fades_in() {
        use crate::AnimationState;
        let th = theme();
        let mut state = AnimationState::new();

        // Frame 1: not hovered → overlay alpha settles at 0 (no extra quad).
        let idle = input_at(-1.0, -1.0);
        let mut l1 = DrawList::new();
        let mut focus = FocusState::new();
        {
            let mut ctx = DrawContext::new(&mut l1, &mut focus, &th, &idle, 800.0, 600.0)
                .with_animations(&mut state);
            Checkbox::new()
                .animated(1)
                .draw(false, "", rect(), &mut ctx);
        }
        let base_quads = l1.chrome_instances.len();

        // Tick then hover: overlay quad appears (a translate-only quad records an
        // extra chrome instance).
        state.tick(0.04);
        let hover = input_at(5.0, 10.0);
        let mut l2 = DrawList::new();
        let mut focus = FocusState::new();
        {
            let mut ctx = DrawContext::new(&mut l2, &mut focus, &th, &hover, 800.0, 600.0)
                .with_animations(&mut state);
            Checkbox::new()
                .animated(1)
                .draw(false, "", rect(), &mut ctx);
        }
        assert!(
            l2.chrome_instances.len() > base_quads,
            "fading-in hover overlay should add quad geometry"
        );
    }
}
