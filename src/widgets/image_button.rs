//! Image / icon button widget.
//!
//! A clickable button whose content is an image rather than a text label —
//! Teardown's `UiImageButton`. It layers the stateless [`Image`] widget (for the
//! icon, with full [`ImageFit`]/[`ImageAlign`]/tint support) over `Button`-style
//! chrome (background, border, hover/press/disabled feedback), and reports a
//! click the same way [`Button`](super::Button) does.
//!
//! The chrome is optional: [`ImageButton::bare`] drops the background and border
//! so the image itself is the hit target, with only a translucent overlay on
//! hover/press. Disabled buttons dim via an overlay (which works for string-key
//! sources that can't be tinted) and never report hover or clicks.
//!
//! # Example
//! ```ignore
//! let (w, h) = renderer.image_size("play.png").unwrap_or((0, 0));
//! let clicked = ImageButton::key("play.png")
//!     .natural_size(w as f32, h as f32)
//!     .fit(ImageFit::Contain)
//!     .draw(rect, &mut list, &theme, &input);
//! ```

use crate::layout::Rect;
use crate::{InputState, SpriteId, Theme};

use super::button::{draw_chrome, ButtonVisual};
use super::{DrawList, Image, ImageAlign, ImageFit};

/// Image / icon button — an [`Image`] with clickable button chrome.
#[derive(Clone)]
pub struct ImageButton {
    image: Image,
    enabled: bool,
    /// Inset between the button edge and the image. `None` uses a quarter of
    /// `theme.padding` — icon buttons want the glyph to nearly fill, unlike the
    /// full text inset a label button uses.
    padding: Option<f32>,
    /// Draw the background + border chrome (default true). `false` => bare.
    chrome: bool,
}

impl ImageButton {
    /// Button showing a pre-resolved sprite handle (supports tint + UV crop).
    pub fn sprite(id: SpriteId) -> Self {
        Self::from_image(Image::sprite(id))
    }

    /// Button showing a string-keyed sprite, resolved by name at render time.
    pub fn key(key: impl Into<String>) -> Self {
        Self::from_image(Image::key(key))
    }

    /// Wrap an already-configured [`Image`] as a button.
    pub fn from_image(image: Image) -> Self {
        Self {
            image,
            enabled: true,
            padding: None,
            chrome: true,
        }
    }

    /// Natural (source) pixel size, required for aspect-aware fits.
    pub fn natural_size(mut self, w: f32, h: f32) -> Self {
        self.image = self.image.natural_size(w, h);
        self
    }

    /// Set the image scaling mode (default [`ImageFit::Stretch`]).
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.image = self.image.fit(fit);
        self
    }

    /// Set image placement within leftover box space (default center).
    pub fn align(mut self, align: ImageAlign) -> Self {
        self.image = self.image.align(align);
        self
    }

    /// Multiply the sampled image color by `tint` (sprite-source only).
    pub fn tint(mut self, tint: [f32; 4]) -> Self {
        self.image = self.image.tint(tint);
        self
    }

    /// Enable/disable the button (disabled => dimmed, no hover/click).
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Inset between the button edge and the image (default `theme.padding/4`).
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Some(padding);
        self
    }

    /// Drop the background + border chrome; the image becomes the button face
    /// with only a hover/press overlay.
    pub fn bare(mut self) -> Self {
        self.chrome = false;
        self
    }

    /// Draw the button at `rect` and return true if clicked.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, theme: &Theme, input: &InputState) -> bool {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return false;
        }

        let hovered =
            self.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;

        if self.chrome {
            draw_chrome(
                list,
                theme,
                rect,
                &ButtonVisual {
                    enabled: self.enabled,
                    hovered,
                    pressed,
                },
            );
        }

        // Image content, inset by padding.
        let pad = self.padding.unwrap_or(theme.padding * 0.25);
        let inner = Rect::new(
            rect.x + pad,
            rect.y + pad,
            (rect.width - pad * 2.0).max(0.0),
            (rect.height - pad * 2.0).max(0.0),
        );
        self.image.draw(inner, list);

        // State overlay on top of the image, so feedback shows even for
        // string-key sources whose tint can't be modulated.
        if !self.enabled {
            list.quad(rect.x, rect.y, rect.width, rect.height, [0.0, 0.0, 0.0, 0.4]);
        } else if !self.chrome {
            // Bare buttons get their only feedback from the overlay.
            if pressed {
                list.quad(rect.x, rect.y, rect.width, rect.height, [0.0, 0.0, 0.0, 0.2]);
            } else if hovered {
                list.quad(rect.x, rect.y, rect.width, rect.height, [1.0, 1.0, 1.0, 0.08]);
            }
        }

        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: SpriteId = 0;

    /// Input with the mouse at a point, optionally down/clicked.
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
        Rect::new(10.0, 10.0, 80.0, 80.0)
    }

    #[test]
    fn click_inside_returns_true() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(50.0, 50.0, true, true);
        assert!(ImageButton::sprite(ID).draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn click_outside_returns_false() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(200.0, 200.0, true, true);
        assert!(!ImageButton::sprite(ID).draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn disabled_never_clicks() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(50.0, 50.0, true, true);
        assert!(!ImageButton::sprite(ID)
            .enabled(false)
            .draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn consumed_input_blocks_click() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let mut input = input_at(50.0, 50.0, true, true);
        input.mouse_consumed = true;
        assert!(!ImageButton::sprite(ID).draw(rect(), &mut list, &theme, &input));
    }

    #[test]
    fn image_is_inset_by_padding() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);
        ImageButton::sprite(ID)
            .padding(12.0)
            .draw(rect(), &mut list, &theme, &input);
        // Stretch fills the inset box; recover it from the last icon's TL corner.
        let c = list.icons.last().expect("an icon was drawn").corners;
        assert!((c[0][0] - 22.0).abs() < 1e-3, "inset x: {}", c[0][0]); // 10 + 12
        assert!((c[0][1] - 22.0).abs() < 1e-3, "inset y: {}", c[0][1]);
        // BR corner: (10+80-12, 10+80-12) = (78, 78)
        assert!((c[2][0] - 78.0).abs() < 1e-3, "inset right: {}", c[2][0]);
    }

    #[test]
    fn chrome_draws_background_quads() {
        let mut chrome = DrawList::new();
        let mut bare = DrawList::new();
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);
        ImageButton::sprite(ID).draw(rect(), &mut chrome, &theme, &input);
        ImageButton::sprite(ID)
            .bare()
            .draw(rect(), &mut bare, &theme, &input);
        // Chrome adds background + border geometry the bare variant omits.
        assert!(
            chrome.vertices.len() > bare.vertices.len(),
            "chrome ({}) should add geometry over bare ({})",
            chrome.vertices.len(),
            bare.vertices.len()
        );
    }

    #[test]
    fn bare_hover_adds_overlay() {
        let theme = Theme::default();
        let mut idle = DrawList::new();
        ImageButton::sprite(ID)
            .bare()
            .draw(rect(), &mut idle, &theme, &input_at(0.0, 0.0, false, false));
        let mut hot = DrawList::new();
        ImageButton::sprite(ID)
            .bare()
            .draw(rect(), &mut hot, &theme, &input_at(50.0, 50.0, false, false));
        assert!(
            hot.vertices.len() > idle.vertices.len(),
            "hover overlay should add a quad"
        );
    }

    #[test]
    fn disabled_adds_dim_overlay() {
        let theme = Theme::default();
        let mut enabled = DrawList::new();
        ImageButton::sprite(ID)
            .bare()
            .draw(rect(), &mut enabled, &theme, &input_at(0.0, 0.0, false, false));
        let mut disabled = DrawList::new();
        ImageButton::sprite(ID)
            .bare()
            .enabled(false)
            .draw(rect(), &mut disabled, &theme, &input_at(0.0, 0.0, false, false));
        assert!(
            disabled.vertices.len() > enabled.vertices.len(),
            "disabled dim overlay should add a quad"
        );
    }

    #[test]
    fn zero_rect_draws_nothing_and_no_click() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, true, true);
        assert!(!ImageButton::sprite(ID).draw(Rect::new(0.0, 0.0, 0.0, 50.0), &mut list, &theme, &input));
        assert!(list.icons.is_empty());
        assert!(list.vertices.is_empty());
    }

    #[test]
    fn tint_forwards_to_image() {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = input_at(0.0, 0.0, false, false);
        ImageButton::sprite(ID)
            .bare()
            .tint([1.0, 0.0, 0.0, 1.0])
            .draw(rect(), &mut list, &theme, &input);
        assert_eq!(list.icons.last().unwrap().tint, [1.0, 0.0, 0.0, 1.0]);
    }
}
