//! UI theming - colors, fonts, spacing.

use crate::style::{CustomStyles, StyleKey, StyleValue};
use crate::text::{FontHandle, TextBlock};

/// UI theme with colors and styling.
#[derive(Clone)]
pub struct Theme {
    // Colors
    pub background: [f32; 4],
    pub panel: [f32; 4],
    pub panel_border: [f32; 4],
    pub button: [f32; 4],
    pub button_hover: [f32; 4],
    pub button_pressed: [f32; 4],
    pub button_border: [f32; 4],
    pub input_background: [f32; 4],
    pub input_border: [f32; 4],
    pub input_focus_border: [f32; 4],
    pub text: [f32; 4],
    pub text_dim: [f32; 4],
    pub text_highlight: [f32; 4],
    pub accent: [f32; 4],
    pub error: [f32; 4],
    /// Outline color drawn around the keyboard-focused widget. Bright by design
    /// so the focus ring reads clearly against any widget chrome.
    pub focus_ring: [f32; 4],

    // Tab colors
    pub tab_inactive: [f32; 4],
    pub tab_active: [f32; 4],
    pub tab_hover: [f32; 4],
    pub tab_border: [f32; 4],

    // Progress bar colors
    pub progress_background: [f32; 4],
    pub progress_fill: [f32; 4],
    pub progress_fill_low: [f32; 4], // For low values (e.g., hunger critical)
    pub progress_fill_medium: [f32; 4], // For medium values

    // Sizing
    pub padding: f32,
    pub spacing: f32,
    pub border_radius: f32,
    pub border_width: f32,
    pub font_size: f32,
    pub font_size_title: f32,
    pub button_height: f32,
    pub input_height: f32,

    /// UI-wide default font. `None` resolves to the default sans-serif (the
    /// bundled Noto Sans when the `bundled-font` feature is on, else the system
    /// sans-serif). Set to a loaded [`FontHandle`] to theme all widget text in a
    /// custom family; every widget that builds text through this `Theme` picks it
    /// up. Per-block `TextBlock::with_font` still overrides it.
    pub font: Option<FontHandle>,

    /// Mod-defined style values keyed by [`StyleKey::custom`] name-hash. Built-in
    /// styles live in the typed fields above; this map holds keys the core
    /// doesn't know about, so a custom widget can theme itself without core
    /// changes. Populate via [`Theme::register_style`]; read via [`Theme::style`]
    /// or the keyed [`Theme::get`].
    custom: CustomStyles,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Dark, polished color scheme
            background: [0.08, 0.08, 0.12, 1.0],
            panel: [0.12, 0.12, 0.18, 0.95],
            panel_border: [0.25, 0.25, 0.35, 1.0],
            button: [0.18, 0.18, 0.25, 1.0],
            button_hover: [0.22, 0.22, 0.32, 1.0],
            button_pressed: [0.15, 0.15, 0.22, 1.0],
            button_border: [0.3, 0.3, 0.4, 1.0],
            input_background: [0.06, 0.06, 0.10, 1.0],
            input_border: [0.25, 0.25, 0.35, 1.0],
            input_focus_border: [0.4, 0.5, 0.8, 1.0],
            text: [0.9, 0.9, 0.95, 1.0],
            text_dim: [0.7, 0.7, 0.8, 1.0],
            text_highlight: [0.6, 0.8, 1.0, 1.0],
            accent: [0.3, 0.5, 0.9, 1.0],
            error: [0.9, 0.3, 0.3, 1.0],
            focus_ring: [0.45, 0.62, 1.0, 1.0],

            // Tab colors
            tab_inactive: [0.15, 0.15, 0.20, 1.0],
            tab_active: [0.20, 0.20, 0.28, 1.0],
            tab_hover: [0.18, 0.18, 0.25, 1.0],
            tab_border: [0.30, 0.30, 0.40, 1.0],

            // Progress bar colors
            progress_background: [0.10, 0.10, 0.15, 1.0],
            progress_fill: [0.3, 0.7, 0.4, 1.0], // Green for good
            progress_fill_low: [0.8, 0.3, 0.3, 1.0], // Red for critical
            progress_fill_medium: [0.8, 0.7, 0.2, 1.0], // Yellow for medium

            // Sizing
            padding: 16.0,
            spacing: 12.0,
            border_radius: 6.0,
            border_width: 1.0,
            font_size: 16.0,
            font_size_title: 28.0,
            button_height: 44.0,
            input_height: 40.0,

            font: None,
            custom: CustomStyles::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_text_and_title_have_no_font_by_default() {
        let theme = Theme::default();
        assert!(theme.text("hi", 0.0, 0.0).font.is_none());
        assert!(theme.title("hi", 0.0, 0.0).font.is_none());
    }

    #[test]
    fn builtin_keys_round_trip_through_get_set() {
        use crate::style::{COLOR_KEYS, SCALAR_KEYS};
        let mut theme = Theme::default();
        // Every color key reads back the field, and set writes it.
        for &k in COLOR_KEYS {
            let orig = theme.get(k).unwrap().as_color().unwrap();
            let probe = [orig[0] * 0.5 + 0.1, 0.2, 0.3, 1.0];
            theme.set(k, StyleValue::Color(probe));
            assert_eq!(theme.get(k).unwrap().as_color().unwrap(), probe, "{k:?}");
        }
        for &k in SCALAR_KEYS {
            theme.set(k, StyleValue::Scalar(123.5));
            assert_eq!(theme.get(k).unwrap().as_scalar().unwrap(), 123.5, "{k:?}");
        }
    }

    #[test]
    fn get_set_match_typed_field_accessor() {
        let mut theme = Theme::default();
        theme.set(StyleKey::Accent, StyleValue::Color([0.1, 0.2, 0.3, 1.0]));
        assert_eq!(theme.accent, [0.1, 0.2, 0.3, 1.0], "set writes the typed field");
        theme.button = [0.4, 0.5, 0.6, 1.0];
        assert_eq!(
            theme.get(StyleKey::Button).unwrap().as_color().unwrap(),
            [0.4, 0.5, 0.6, 1.0],
            "get reads the typed field"
        );
    }

    #[test]
    fn register_and_read_custom_style() {
        let mut theme = Theme::default();
        assert_eq!(theme.style("mywidget.glow"), None);
        theme.register_style("mywidget.glow", StyleValue::Color([1.0, 0.5, 0.0, 1.0]));
        assert_eq!(
            theme.style("mywidget.glow"),
            Some(StyleValue::Color([1.0, 0.5, 0.0, 1.0]))
        );
        // Reachable via the keyed get with the same name.
        assert_eq!(
            theme.get(StyleKey::custom("mywidget.glow")),
            Some(StyleValue::Color([1.0, 0.5, 0.0, 1.0]))
        );
    }

    #[test]
    fn theme_font_applies_to_text_and_title() {
        let mut theme = Theme::default();
        theme.font = Some(FontHandle("Noto Sans".to_string()));
        assert_eq!(
            theme.text("hi", 0.0, 0.0).font.as_ref().unwrap().family(),
            "Noto Sans"
        );
        assert_eq!(
            theme.title("hi", 0.0, 0.0).font.as_ref().unwrap().family(),
            "Noto Sans"
        );
    }
}

impl Theme {
    /// Resolve a [`StyleKey`] to its value: built-in keys read the typed field,
    /// `Custom` keys read the [`register_style`](Self::register_style) map
    /// (`None` if unset). This is the keyed view of the theme that the
    /// [`StyleResolver`](crate::StyleResolver) reads through.
    pub fn get(&self, key: StyleKey) -> Option<StyleValue> {
        use StyleKey::*;
        let v = match key {
            // Colors
            Background => StyleValue::Color(self.background),
            Panel => StyleValue::Color(self.panel),
            PanelBorder => StyleValue::Color(self.panel_border),
            Button => StyleValue::Color(self.button),
            ButtonHover => StyleValue::Color(self.button_hover),
            ButtonPressed => StyleValue::Color(self.button_pressed),
            ButtonBorder => StyleValue::Color(self.button_border),
            InputBackground => StyleValue::Color(self.input_background),
            InputBorder => StyleValue::Color(self.input_border),
            InputFocusBorder => StyleValue::Color(self.input_focus_border),
            Text => StyleValue::Color(self.text),
            TextDim => StyleValue::Color(self.text_dim),
            TextHighlight => StyleValue::Color(self.text_highlight),
            Accent => StyleValue::Color(self.accent),
            Error => StyleValue::Color(self.error),
            FocusRing => StyleValue::Color(self.focus_ring),
            TabInactive => StyleValue::Color(self.tab_inactive),
            TabActive => StyleValue::Color(self.tab_active),
            TabHover => StyleValue::Color(self.tab_hover),
            TabBorder => StyleValue::Color(self.tab_border),
            ProgressBackground => StyleValue::Color(self.progress_background),
            ProgressFill => StyleValue::Color(self.progress_fill),
            ProgressFillLow => StyleValue::Color(self.progress_fill_low),
            ProgressFillMedium => StyleValue::Color(self.progress_fill_medium),
            // Scalars
            Padding => StyleValue::Scalar(self.padding),
            Spacing => StyleValue::Scalar(self.spacing),
            BorderRadius => StyleValue::Scalar(self.border_radius),
            BorderWidth => StyleValue::Scalar(self.border_width),
            FontSize => StyleValue::Scalar(self.font_size),
            FontSizeTitle => StyleValue::Scalar(self.font_size_title),
            ButtonHeight => StyleValue::Scalar(self.button_height),
            InputHeight => StyleValue::Scalar(self.input_height),
            // Custom namespace
            Custom(id) => return self.custom.get(&id).copied(),
        };
        Some(v)
    }

    /// Set a [`StyleKey`]'s value. Built-in keys write the typed field; a
    /// shape mismatch (e.g. a [`StyleValue::Scalar`] into a color field) is a
    /// `debug_assert` failure and a no-op in release. `Custom` keys write the
    /// custom map regardless of shape.
    pub fn set(&mut self, key: StyleKey, value: StyleValue) {
        use StyleKey::*;
        // Custom keys store whatever shape they're given.
        if let Custom(id) = key {
            self.custom.insert(id, value);
            return;
        }
        match (key, value) {
            (Background, StyleValue::Color(c)) => self.background = c,
            (Panel, StyleValue::Color(c)) => self.panel = c,
            (PanelBorder, StyleValue::Color(c)) => self.panel_border = c,
            (Button, StyleValue::Color(c)) => self.button = c,
            (ButtonHover, StyleValue::Color(c)) => self.button_hover = c,
            (ButtonPressed, StyleValue::Color(c)) => self.button_pressed = c,
            (ButtonBorder, StyleValue::Color(c)) => self.button_border = c,
            (InputBackground, StyleValue::Color(c)) => self.input_background = c,
            (InputBorder, StyleValue::Color(c)) => self.input_border = c,
            (InputFocusBorder, StyleValue::Color(c)) => self.input_focus_border = c,
            (Text, StyleValue::Color(c)) => self.text = c,
            (TextDim, StyleValue::Color(c)) => self.text_dim = c,
            (TextHighlight, StyleValue::Color(c)) => self.text_highlight = c,
            (Accent, StyleValue::Color(c)) => self.accent = c,
            (Error, StyleValue::Color(c)) => self.error = c,
            (FocusRing, StyleValue::Color(c)) => self.focus_ring = c,
            (TabInactive, StyleValue::Color(c)) => self.tab_inactive = c,
            (TabActive, StyleValue::Color(c)) => self.tab_active = c,
            (TabHover, StyleValue::Color(c)) => self.tab_hover = c,
            (TabBorder, StyleValue::Color(c)) => self.tab_border = c,
            (ProgressBackground, StyleValue::Color(c)) => self.progress_background = c,
            (ProgressFill, StyleValue::Color(c)) => self.progress_fill = c,
            (ProgressFillLow, StyleValue::Color(c)) => self.progress_fill_low = c,
            (ProgressFillMedium, StyleValue::Color(c)) => self.progress_fill_medium = c,
            (Padding, StyleValue::Scalar(s)) => self.padding = s,
            (Spacing, StyleValue::Scalar(s)) => self.spacing = s,
            (BorderRadius, StyleValue::Scalar(s)) => self.border_radius = s,
            (BorderWidth, StyleValue::Scalar(s)) => self.border_width = s,
            (FontSize, StyleValue::Scalar(s)) => self.font_size = s,
            (FontSizeTitle, StyleValue::Scalar(s)) => self.font_size_title = s,
            (ButtonHeight, StyleValue::Scalar(s)) => self.button_height = s,
            (InputHeight, StyleValue::Scalar(s)) => self.input_height = s,
            (k, v) => debug_assert!(
                false,
                "Theme::set shape mismatch for {k:?}: built-in key got {v:?}"
            ),
        }
    }

    /// Register a mod-defined style value under `name` (Teardown-style
    /// `register_style`). Sugar for `set(StyleKey::custom(name), value)`; read it
    /// back with [`style`](Self::style) or `get(StyleKey::custom(name))`.
    pub fn register_style(&mut self, name: &str, value: StyleValue) {
        self.set(StyleKey::custom(name), value);
    }

    /// Read a mod-defined style value by `name` (`None` if never registered).
    pub fn style(&self, name: &str) -> Option<StyleValue> {
        self.get(StyleKey::custom(name))
    }

    /// Create a text block with theme styling
    pub fn text(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size)
            .with_color(
                (self.text[0] * 255.0) as u8,
                (self.text[1] * 255.0) as u8,
                (self.text[2] * 255.0) as u8,
            )
            .with_font_opt(self.font.clone())
    }

    /// Create a title text block
    pub fn title(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size_title)
            .with_color(
                (self.text[0] * 255.0) as u8,
                (self.text[1] * 255.0) as u8,
                (self.text[2] * 255.0) as u8,
            )
            .with_font_opt(self.font.clone())
    }
}
