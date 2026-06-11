//! Checkbox widget.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{SpriteId, Theme};

use super::{DrawContext, DrawList};

/// Icon keys for checkbox textures. Only used by the string-keyed
/// [`Checkbox::with_icon_keys`] path; the default rendering is vector-drawn and
/// needs no atlas assets.
pub const CHECKBOX_ICON: &str = "textures/ui/checkbox.png";
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
    Sprites { unchecked: SpriteId, checked: SpriteId },
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
/// By default the box is drawn from vector primitives using [`Theme`] colors,
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
        }
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
    pub fn with_icon_keys(
        mut self,
        unchecked: &'static str,
        checked: &'static str,
    ) -> Self {
        self.style = BoxStyle::Keys { unchecked, checked };
        self
    }

    /// Draw a checkbox at the given rect. Returns true if clicked (toggled).
    ///
    /// The box is drawn at the left of the rect (square, fitted to rect height),
    /// with the label to its right.
    pub fn draw(
        &self,
        checked: bool,
        label: &str,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> bool {
        let list = &mut *ctx.draw_list;
        let theme = ctx.theme;
        let input = ctx.input;
        // Honor layer capture (`mouse_consumed`) so a checkbox under a
        // modal/popup doesn't react to clicks meant for the overlay.
        let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let clicked = hovered && input.mouse_clicked;

        // Checkbox box (square, fitted to rect height).
        let size = rect.height;
        let box_rect = Rect::new(rect.x, rect.y, size, size);

        match &self.style {
            BoxStyle::Vector => draw_vector_box(list, theme, box_rect, checked),
            BoxStyle::Sprites { unchecked, checked: checked_id } => {
                let sprite = if checked { *checked_id } else { *unchecked };
                list.icon_sprite(sprite, box_rect.x, box_rect.y, size, size, [1.0, 1.0, 1.0, 1.0]);
            }
            BoxStyle::Keys { unchecked, checked: checked_key } => {
                let key = if checked { *checked_key } else { *unchecked };
                list.icon(key, box_rect.x, box_rect.y, size, size);
            }
        }

        // Hover highlight over the box area.
        if hovered {
            list.quad(box_rect.x, box_rect.y, size, size, [1.0, 1.0, 1.0, 0.08]);
        }

        // Label to the right of the checkbox.
        if !label.is_empty() {
            let text_x = rect.x + size + 6.0;
            let text_y = rect.y + (rect.height - theme.font_size) / 2.0;
            let text_color = theme.text;
            let text = TextBlock::new(label, text_x, text_y)
                .with_size(theme.font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_font_opt(theme.font.clone());
            list.text(text);
        }

        clicked
    }
}

/// Draw the theme-driven vector checkbox: a rounded box, filled with the accent
/// color and stamped with a contrast checkmark when `checked`.
fn draw_vector_box(list: &mut DrawList, theme: &Theme, box_rect: Rect, checked: bool) {
    let size = box_rect.width.min(box_rect.height);
    let radius = theme.border_radius.min(size * 0.3).max(0.0);
    let border = theme.border_width.max(1.0).min(size * 0.5);

    if checked {
        // Filled box in accent + contrasting checkmark.
        list.rounded_rect(box_rect, radius, theme.accent);
        let mark = contrast_color(theme.accent);
        let t = (size * 0.14).max(1.5);
        // Tick: down-stroke into the low-left, up-stroke to the high-right.
        let pts = [
            [box_rect.x + size * 0.22, box_rect.y + size * 0.52],
            [box_rect.x + size * 0.42, box_rect.y + size * 0.72],
            [box_rect.x + size * 0.78, box_rect.y + size * 0.28],
        ];
        list.polyline(&pts, t, mark);
    } else {
        // Empty box: subtle fill + border.
        list.rounded_rect(box_rect, radius, theme.input_background);
        list.rounded_rect_outline(box_rect, radius, border, theme.input_border);
    }
}

/// Pick black or white for maximum contrast against `bg` using perceptual
/// (Rec. 709) luminance.
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
    use crate::{FocusState, InputState};

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

    #[test]
    fn vector_checked_adds_checkmark_over_fill() {
        let th = theme();
        // Reference: just the accent fill of the box, no checkmark. The box is
        // a square the height of the rect (20px), with the same radius the
        // widget computes.
        let size = rect().height;
        let radius = th.border_radius.min(size * 0.3).max(0.0);
        let mut fill_only = DrawList::new();
        fill_only.rounded_rect(Rect::new(0.0, 0.0, size, size), radius, th.accent);

        let (checked, _) = draw_cb(&Checkbox::new(), true, "", rect(), &input_at(-1.0, -1.0));

        // Checked = same fill + a checkmark polyline, so strictly more geometry.
        assert!(
            checked.vertices.len() > fill_only.vertices.len(),
            "checked box should add checkmark geometry beyond the accent fill"
        );
    }

    #[test]
    fn click_inside_toggles() {
        let (_, clicked) = draw_cb(&Checkbox::new(), false, "Label", rect(), &click_at(5.0, 10.0));
        assert!(clicked, "a click inside the rect should report a toggle");
    }

    #[test]
    fn click_outside_does_not_toggle() {
        let (_, clicked) = draw_cb(&Checkbox::new(), false, "Label", rect(), &click_at(500.0, 500.0));
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
}
