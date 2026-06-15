//! Typed, extensible styling: keyed style values, scoped overrides, and the
//! resolver widgets read through.
//!
//! The flat [`Theme`](crate::Theme) struct stays the source of built-in values;
//! this module adds three things on top of it:
//!
//! - [`StyleKey`] / [`StyleValue`] — a typed address space over every theme
//!   field plus a [`StyleKey::Custom`] namespace for mod-defined keys (so a
//!   custom widget can carry its own style without core changes).
//! - [`StyleOverlay`] — a caller-owned sparse set of overrides.
//! - [`StyleResolver`] — the single read path: overlay first, then theme. A
//!   scoped overlay thus recolors everything drawn under it **without cloning
//!   the theme**.

use std::collections::HashMap;

use crate::Theme;
use crate::text::TextBlock;

/// A single resolved style datum — either a color or a scalar.
///
/// Theme values are one of these two shapes (`[f32; 4]` RGBA colors or `f32`
/// sizes), so the keyed map is uniform without boxing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StyleValue {
    Color([f32; 4]),
    Scalar(f32),
}

impl StyleValue {
    /// The color, if this is a [`StyleValue::Color`].
    pub fn as_color(self) -> Option<[f32; 4]> {
        match self {
            StyleValue::Color(c) => Some(c),
            StyleValue::Scalar(_) => None,
        }
    }

    /// The scalar, if this is a [`StyleValue::Scalar`].
    pub fn as_scalar(self) -> Option<f32> {
        match self {
            StyleValue::Scalar(s) => Some(s),
            StyleValue::Color(_) => None,
        }
    }
}

/// 64-bit FNV-1a hash of `name`. Used to address [`StyleKey::Custom`] keys by
/// name without a global interner: the hash is a pure function, stable across
/// themes and runs, so `StyleKey::custom("x")` always denotes the same key.
const fn fnv1a64(name: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let bytes = name.as_bytes();
    let mut hash = OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// A typed address for a style value: one variant per built-in [`Theme`] field,
/// plus [`StyleKey::Custom`] for mod-defined keys.
///
/// Colors and scalars share the enum; [`StyleResolver::color`] /
/// [`StyleResolver::scalar`] pick the matching shape (a built-in key always
/// resolves to its declared shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StyleKey {
    // --- Colors ---
    Background,
    Panel,
    PanelBorder,
    Button,
    ButtonHover,
    ButtonPressed,
    ButtonBorder,
    InputBackground,
    InputBorder,
    InputFocusBorder,
    Text,
    TextDim,
    TextHighlight,
    Accent,
    Error,
    FocusRing,
    TabInactive,
    TabActive,
    TabHover,
    TabBorder,
    ProgressBackground,
    ProgressFill,
    ProgressFillLow,
    ProgressFillMedium,
    // --- Scalars ---
    Padding,
    Spacing,
    BorderRadius,
    BorderWidth,
    FontSize,
    FontSizeTitle,
    ButtonHeight,
    InputHeight,
    /// A mod-defined key, addressed by the FNV-1a hash of its name (see
    /// [`StyleKey::custom`]). Lives in [`Theme`]'s custom map / a [`StyleOverlay`].
    Custom(u64),
}

impl StyleKey {
    /// A custom key addressed by `name`. Equal names always produce the same
    /// key; distinct names (barring an astronomically unlikely 64-bit collision)
    /// produce distinct keys. No registration or global interner required.
    pub fn custom(name: &str) -> Self {
        StyleKey::Custom(fnv1a64(name))
    }

    /// Whether this key denotes a color (vs a scalar). Built-in keys have a
    /// fixed shape; `Custom` keys are shapeless (whatever value was stored), so
    /// this returns `false` for them.
    pub fn is_color(self) -> bool {
        use StyleKey::*;
        matches!(
            self,
            Background
                | Panel
                | PanelBorder
                | Button
                | ButtonHover
                | ButtonPressed
                | ButtonBorder
                | InputBackground
                | InputBorder
                | InputFocusBorder
                | Text
                | TextDim
                | TextHighlight
                | Accent
                | Error
                | FocusRing
                | TabInactive
                | TabActive
                | TabHover
                | TabBorder
                | ProgressBackground
                | ProgressFill
                | ProgressFillLow
                | ProgressFillMedium
        )
    }
}

/// A caller-owned sparse set of style overrides layered over a [`Theme`].
///
/// Build one, [`set`](Self::set) the keys you want to override, then hand it to
/// a widget via [`DrawContext::with_style`](crate::DrawContext::with_style) (or
/// a [`StyleResolver`]). Anything resolved finds the overlay value first and the
/// theme otherwise — so a single overlay can recolor a subtree without touching
/// or cloning the theme.
///
/// Backed by a `Vec` rather than a `HashMap`: override sets are tiny (a handful
/// of keys), so a linear scan is faster and allocation-lighter than hashing.
#[derive(Clone, Debug, Default)]
pub struct StyleOverlay {
    entries: Vec<(StyleKey, StyleValue)>,
}

impl StyleOverlay {
    /// An empty overlay (resolves to the theme for every key).
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Override `key` with `value`. Replaces any existing entry for `key`.
    /// Returns `&mut self` for chaining.
    pub fn set(&mut self, key: StyleKey, value: StyleValue) -> &mut Self {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.entries.push((key, value));
        }
        self
    }

    /// Convenience for `set(key, StyleValue::Color(c))`.
    pub fn set_color(&mut self, key: StyleKey, c: [f32; 4]) -> &mut Self {
        self.set(key, StyleValue::Color(c))
    }

    /// Convenience for `set(key, StyleValue::Scalar(s))`.
    pub fn set_scalar(&mut self, key: StyleKey, s: f32) -> &mut Self {
        self.set(key, StyleValue::Scalar(s))
    }

    /// The overridden value for `key`, if any.
    pub fn get(&self, key: StyleKey) -> Option<StyleValue> {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    }

    /// Whether the overlay has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop all overrides.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The single style read path: an optional [`StyleOverlay`] layered over a
/// [`Theme`]. Borrows both; holds no state and is cheap to construct on demand.
///
/// Resolution precedence is **overlay → theme**. Built-in keys always resolve
/// (the theme has a value for each); `Custom` keys resolve only if set on the
/// overlay or registered on the theme, so read those with [`color_or`](Self::color_or)
/// / [`scalar_or`](Self::scalar_or).
#[derive(Clone, Copy)]
pub struct StyleResolver<'a> {
    theme: &'a Theme,
    overlay: Option<&'a StyleOverlay>,
}

impl<'a> StyleResolver<'a> {
    /// A resolver over `theme` with no overrides.
    pub fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            overlay: None,
        }
    }

    /// A resolver over `theme` with `overlay` taking precedence.
    pub fn with_overlay(theme: &'a Theme, overlay: &'a StyleOverlay) -> Self {
        Self {
            theme,
            overlay: Some(overlay),
        }
    }

    /// A resolver over `theme` with an optional overlay (convenience for call
    /// sites that hold an `Option<&StyleOverlay>`).
    pub fn with_overlay_opt(theme: &'a Theme, overlay: Option<&'a StyleOverlay>) -> Self {
        Self { theme, overlay }
    }

    /// The theme this resolver reads from (for non-style fields like `font`).
    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Resolve `key` to its value: overlay first, then theme. `None` only for a
    /// `Custom` key that's set in neither.
    pub fn get(&self, key: StyleKey) -> Option<StyleValue> {
        if let Some(v) = self.overlay.and_then(|o| o.get(key)) {
            return Some(v);
        }
        self.theme.get(key)
    }

    /// Resolve a color key. Built-in color keys always resolve; a missing/
    /// mismatched value falls back to opaque magenta as a loud "unset" sentinel
    /// (only reachable by misusing a `Custom`/scalar key here — use
    /// [`color_or`](Self::color_or) for those).
    pub fn color(&self, key: StyleKey) -> [f32; 4] {
        self.color_or(key, [1.0, 0.0, 1.0, 1.0])
    }

    /// Resolve a color key, falling back to `default` when unset or non-color.
    pub fn color_or(&self, key: StyleKey, default: [f32; 4]) -> [f32; 4] {
        self.get(key).and_then(StyleValue::as_color).unwrap_or(default)
    }

    /// Resolve a scalar key. Built-in scalar keys always resolve; otherwise `0.0`
    /// (use [`scalar_or`](Self::scalar_or) for `Custom` keys).
    pub fn scalar(&self, key: StyleKey) -> f32 {
        self.scalar_or(key, 0.0)
    }

    /// Resolve a scalar key, falling back to `default` when unset or non-scalar.
    pub fn scalar_or(&self, key: StyleKey, default: f32) -> f32 {
        self.get(key)
            .and_then(StyleValue::as_scalar)
            .unwrap_or(default)
    }

    /// A body [`TextBlock`] styled through the resolver: [`FontSize`](StyleKey::FontSize)
    /// size, [`Text`](StyleKey::Text) color, and the theme font. The overlay-aware
    /// counterpart of [`Theme::text`](crate::Theme::text).
    pub fn text_block(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        let c = self.color(StyleKey::Text);
        TextBlock::new(content, x, y)
            .with_size(self.scalar(StyleKey::FontSize))
            .with_color(
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
            )
            .with_font_opt(self.theme.font.clone())
    }

    /// A title [`TextBlock`] styled through the resolver: [`FontSizeTitle`](StyleKey::FontSizeTitle)
    /// size, [`Text`](StyleKey::Text) color, and the theme font. The overlay-aware
    /// counterpart of [`Theme::title`](crate::Theme::title).
    pub fn title_block(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        let c = self.color(StyleKey::Text);
        TextBlock::new(content, x, y)
            .with_size(self.scalar(StyleKey::FontSizeTitle))
            .with_color(
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
            )
            .with_font_opt(self.theme.font.clone())
    }
}

/// All built-in color keys, paired with the value the default theme stores —
/// used by the round-trip test and handy for tooling.
#[cfg(test)]
pub(crate) const COLOR_KEYS: &[StyleKey] = &[
    StyleKey::Background,
    StyleKey::Panel,
    StyleKey::PanelBorder,
    StyleKey::Button,
    StyleKey::ButtonHover,
    StyleKey::ButtonPressed,
    StyleKey::ButtonBorder,
    StyleKey::InputBackground,
    StyleKey::InputBorder,
    StyleKey::InputFocusBorder,
    StyleKey::Text,
    StyleKey::TextDim,
    StyleKey::TextHighlight,
    StyleKey::Accent,
    StyleKey::Error,
    StyleKey::FocusRing,
    StyleKey::TabInactive,
    StyleKey::TabActive,
    StyleKey::TabHover,
    StyleKey::TabBorder,
    StyleKey::ProgressBackground,
    StyleKey::ProgressFill,
    StyleKey::ProgressFillLow,
    StyleKey::ProgressFillMedium,
];

#[cfg(test)]
pub(crate) const SCALAR_KEYS: &[StyleKey] = &[
    StyleKey::Padding,
    StyleKey::Spacing,
    StyleKey::BorderRadius,
    StyleKey::BorderWidth,
    StyleKey::FontSize,
    StyleKey::FontSizeTitle,
    StyleKey::ButtonHeight,
    StyleKey::InputHeight,
];

/// Internal helper for [`Theme`]'s custom map type (kept here so the key/value
/// types live together).
pub(crate) type CustomStyles = HashMap<u64, StyleValue>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_value_accessors() {
        assert_eq!(StyleValue::Color([1.0, 2.0, 3.0, 4.0]).as_color(), Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(StyleValue::Color([1.0, 2.0, 3.0, 4.0]).as_scalar(), None);
        assert_eq!(StyleValue::Scalar(7.0).as_scalar(), Some(7.0));
        assert_eq!(StyleValue::Scalar(7.0).as_color(), None);
    }

    #[test]
    fn custom_key_is_stable_and_name_sensitive() {
        assert_eq!(StyleKey::custom("widget.bg"), StyleKey::custom("widget.bg"));
        assert_ne!(StyleKey::custom("widget.bg"), StyleKey::custom("widget.fg"));
        // Built-in keys are not colored-coded as custom.
        assert!(StyleKey::Accent.is_color());
        assert!(!StyleKey::Padding.is_color());
    }

    #[test]
    fn overlay_set_get_and_replace() {
        let mut o = StyleOverlay::new();
        assert!(o.is_empty());
        o.set_color(StyleKey::Button, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(o.get(StyleKey::Button), Some(StyleValue::Color([0.1, 0.2, 0.3, 1.0])));
        // Replace, not duplicate.
        o.set_color(StyleKey::Button, [0.9, 0.9, 0.9, 1.0]);
        assert_eq!(o.entries.len(), 1);
        assert_eq!(o.get(StyleKey::Button), Some(StyleValue::Color([0.9, 0.9, 0.9, 1.0])));
        o.clear();
        assert!(o.is_empty());
    }

    #[test]
    fn resolver_precedence_overlay_then_theme() {
        let theme = Theme::default();
        // No overlay: built-in resolves to the theme field.
        let r = StyleResolver::new(&theme);
        assert_eq!(r.color(StyleKey::Accent), theme.accent);
        assert_eq!(r.scalar(StyleKey::Padding), theme.padding);

        // With overlay: the override wins, other keys still come from the theme.
        let mut o = StyleOverlay::new();
        o.set_color(StyleKey::Accent, [0.0, 0.0, 0.0, 1.0]);
        let r = StyleResolver::with_overlay(&theme, &o);
        assert_eq!(r.color(StyleKey::Accent), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(r.color(StyleKey::Button), theme.button);
    }

    #[test]
    fn resolver_custom_key_uses_default_when_unset() {
        let theme = Theme::default();
        let r = StyleResolver::new(&theme);
        let key = StyleKey::custom("missing");
        assert_eq!(r.color_or(key, [0.5, 0.5, 0.5, 1.0]), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(r.scalar_or(key, 3.0), 3.0);
    }
}
