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
use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;

/// Resolved interaction state of a button, shared by the chrome/overlay helpers.
pub(crate) struct ButtonVisual {
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
}

impl ButtonVisual {
    /// Background fill for the current state (disabled dims the idle color).
    fn bg_color(&self, theme: &Theme) -> [f32; 4] {
        if !self.enabled {
            let mut c = theme.button;
            c[3] = 0.5;
            c
        } else if self.pressed {
            theme.button_pressed
        } else if self.hovered {
            theme.button_hover
        } else {
            theme.button
        }
    }
}

/// Draw a button's background + rounded border for the given state.
///
/// Shared by [`Button`] and [`ImageButton`](super::ImageButton) so both get the
/// same rounded chrome from a single place. Honors [`Theme::border_radius`]
/// (0 => square) via [`DrawList::rounded_rect`]/[`DrawList::rounded_rect_outline`].
pub(crate) fn draw_chrome(list: &mut DrawList, theme: &Theme, rect: Rect, v: &ButtonVisual) {
    let bg = v.bg_color(theme);
    let border_color = if v.hovered && v.enabled {
        theme.accent
    } else {
        theme.button_border
    };
    // One instanced SDF rounded-rect carries fill + border. When the border is
    // disabled (`border_width == 0`) a transparent border color collapses the
    // SDF to a plain fill. `chrome_rect` falls back to immediate tessellation
    // under a rotated/scaled transform, so correctness is universal.
    let border = if theme.border_width > 0.0 {
        border_color
    } else {
        [0.0, 0.0, 0.0, 0.0]
    };
    list.chrome_rect(
        rect,
        theme.border_radius,
        theme.border_width,
        bg,
        border,
    );
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

/// Centered, vertically-aligned button label clipped to the inner width.
fn draw_label(list: &mut DrawList, theme: &Theme, rect: Rect, label: &str, enabled: bool) {
    let text_color = if enabled { theme.text } else { theme.text_dim };
    list.text(
        TextBlock::new(
            label,
            rect.x + theme.padding,
            rect.y + (rect.height - theme.font_size) / 2.0,
        )
        .with_size(theme.font_size)
        .with_color(
            (text_color[0] * 255.0) as u8,
            (text_color[1] * 255.0) as u8,
            (text_color[2] * 255.0) as u8,
        )
        .with_max_width(rect.width - theme.padding * 2.0),
    );
}

/// Button widget — a clickable text label with optional rounded chrome.
#[derive(Clone)]
pub struct Button {
    label: String,
    enabled: bool,
    /// Draw the background + rounded border (default true). `false` => bare.
    chrome: bool,
}

impl Button {
    /// A button showing `label`, drawn at a `Rect` via [`Button::draw`].
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            chrome: true,
        }
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
    pub fn draw(&self, rect: Rect, list: &mut DrawList, theme: &Theme, input: &InputState) -> bool {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return false;
        }

        let hovered = self.enabled && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;
        let v = ButtonVisual {
            enabled: self.enabled,
            hovered,
            pressed,
        };

        if self.chrome {
            draw_chrome(list, theme, rect, &v);
        } else {
            draw_bare_overlay(list, rect, &v);
        }
        draw_label(list, theme, rect, &self.label, self.enabled);

        clicked
    }

    /// Draw a chrome button at a layout-computed rect. Returns true if clicked.
    ///
    /// Convenience for the common case; equivalent to
    /// `Button::new(label).enabled(enabled).draw(rect, …)`.
    pub fn draw_at(
        label: &str,
        rect: Rect,
        enabled: bool,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
    ) -> bool {
        Button::new(label).enabled(enabled).draw(rect, list, theme, input)
    }

    /// Draw a nine-slice textured button at a layout-computed rect. Returns true if clicked.
    pub fn draw_nine_slice(
        label: &str,
        rect: Rect,
        enabled: bool,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
        texture_key: &str,
    ) -> bool {
        let hovered = enabled && rect.contains(input.mouse_x, input.mouse_y);
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
        draw_label(list, theme, rect, label, enabled);

        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn click_inside_returns_true() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        assert!(Button::new("Go").draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn click_outside_returns_false() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(500.0, 500.0, true, true);
        assert!(!Button::new("Go").draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn disabled_never_clicks() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        assert!(!Button::new("Go")
            .enabled(false)
            .draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn zero_rect_draws_nothing_and_no_click() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, true, true);
        assert!(!Button::new("Go").draw(Rect::new(0.0, 0.0, 0.0, 32.0), &mut list, &theme, &input));
    }

    #[test]
    fn bare_omits_chrome_geometry() {
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);
        let mut chrome = DrawList::new();
        Button::new("Go").draw(rect(), &mut chrome, &theme, &input);
        let mut bare = DrawList::new();
        Button::new("Go")
            .bare()
            .draw(rect(), &mut bare, &theme, &input);
        // Chrome records one instanced rounded-rect (background + border) that
        // the bare idle variant omits entirely (no chrome, no overlay when idle).
        assert_eq!(chrome.chrome_instances.len(), 1, "chrome draws one instance");
        assert!(bare.chrome_instances.is_empty(), "bare draws no chrome");
        assert!(
            bare.vertices.is_empty(),
            "bare idle draws no background geometry"
        );
    }

    #[test]
    fn bare_hover_adds_overlay() {
        let theme = Theme::default();
        let mut idle = DrawList::new();
        Button::new("Go")
            .bare()
            .draw(rect(), &mut idle, &theme, &input_at(0.0, 0.0, false, false));
        let mut hot = DrawList::new();
        Button::new("Go")
            .bare()
            .draw(rect(), &mut hot, &theme, &input_at(50.0, 25.0, false, false));
        assert!(
            hot.chrome_instances.len() > idle.chrome_instances.len(),
            "bare hover should add an overlay quad (instanced)"
        );
    }

    #[test]
    fn draw_at_matches_builder() {
        let theme = Theme::default();
        let input = input_at(50.0, 25.0, true, true);
        let mut a = DrawList::new();
        let ra = Button::draw_at("Go", rect(), true, &mut a, &theme, &input);
        let mut b = DrawList::new();
        let rb = Button::new("Go").draw(rect(), &mut b, &theme, &input);
        assert_eq!(ra, rb);
        assert_eq!(a.vertices.len(), b.vertices.len());
    }
}
