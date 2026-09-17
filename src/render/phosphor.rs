//! Phosphor icon font (regular weight, MIT) — the vector source for the MSDF
//! icon atlas.
//!
//! Icons are real font glyphs (Phosphor ships its outlines in a TTF, mapped to
//! Private-Use-Area codepoints), so we render them through the *exact same* fdsm
//! MSDF generator and `ui_msdf.wgsl` shader that text uses — no stroke→fill
//! conversion, no new runtime deps. The only icon-specific knowledge lives here:
//! the vendored font bytes, the curated [`PhosphorIcon`] enum, and the
//! enum→codepoint→`GlyphId` resolution.
//!
//! Placement is *not* baseline-driven the way text is: an icon fits-and-centers
//! into a caller-given rect (see `text::fit_centered`), so the side-bearing /
//! line-box metrics that make text glyphs awkward to center don't apply.
//!
//! This whole module is gated behind the `phosphor-icons` feature.

use super::icon_font::{IconFontId, IconGlyph, icon_glyph};

/// The vendored Phosphor regular-weight font (MIT licensed — see
/// `assets/fonts/phosphor/LICENSE`).
pub(crate) const PHOSPHOR_TTF: &[u8] =
    include_bytes!("../../assets/fonts/phosphor/Phosphor-Regular.ttf");

/// The name Phosphor is registered under in the icon-font registry. Applications
/// may look it up with [`icon_font_id`](super::icon_font::icon_font_id), though
/// [`IconFontId::PHOSPHOR`] is the direct route.
pub(crate) fn phosphor_font_name() -> &'static str {
    "phosphor"
}

/// Declare the curated icon set once: the enum, its [`PhosphorIcon::ALL`] table,
/// the codepoints and the kebab-case names all expand from the same list, so a
/// new icon is a single line and the four can never drift apart.
macro_rules! icons {
    ($( $(#[$doc:meta])* $variant:ident = $cp:literal, $name:literal; )*) => {
        /// Curated set of Phosphor icons exposed by the library. Codepoints are
        /// the Phosphor regular-weight Private-Use-Area assignments; they are
        /// verified against the vendored font's cmap by the unit tests in this
        /// module.
        ///
        /// `#[non_exhaustive]` so we can grow the set without it being a
        /// breaking change.
        ///
        /// Names match Phosphor's own catalogue (<https://phosphoricons.com>),
        /// so [`from_name`](PhosphorIcon::from_name) accepts exactly what a
        /// designer would type — useful for data- or script-driven UI.
        #[non_exhaustive]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        pub enum PhosphorIcon {
            $( $(#[$doc])* $variant, )*
        }

        impl PhosphorIcon {
            /// Every variant, for prewarming the atlas, for `from_name`, and for tests.
            pub const ALL: &'static [PhosphorIcon] = &[ $( PhosphorIcon::$variant, )* ];

            /// The glyph's Private-Use-Area codepoint in the Phosphor regular font.
            pub fn codepoint(self) -> char {
                match self {
                    $( PhosphorIcon::$variant => $cp, )*
                }
            }

            /// The icon's canonical kebab-case Phosphor name (e.g. `"caret-up"`).
            pub fn name(self) -> &'static str {
                match self {
                    $( PhosphorIcon::$variant => $name, )*
                }
            }

            /// Look an icon up by its kebab-case Phosphor name. `None` for a name
            /// outside the curated set — callers decide whether that is a warning
            /// or a hard error.
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $( $name => Some(PhosphorIcon::$variant), )*
                    _ => None,
                }
            }
        }
    };
}

icons! {
    /// Upward-pointing caret (e.g. collapse / increment).
    CaretUp = '\u{e13c}', "caret-up";
    /// Downward-pointing caret (e.g. expand / decrement).
    CaretDown = '\u{e136}', "caret-down";
    /// Plus sign (add / increment).
    Plus = '\u{e3d4}', "plus";
    /// Minus sign (remove / decrement).
    Minus = '\u{e32a}', "minus";
    /// Checkmark (confirm / enabled state).
    Check = '\u{e182}', "check";
    /// Cross (close / cancel).
    X = '\u{e4f6}', "x";
    /// Filled right-pointing play control.
    Play = '\u{e3d0}', "play";
    /// Open eye (reveal / visible).
    Eye = '\u{e220}', "eye";
    /// Crossed-out eye (hide / hidden).
    EyeSlash = '\u{e224}', "eye-slash";
    /// Trash can (delete).
    Trash = '\u{e4a6}', "trash";
    /// Simple pencil (edit).
    PencilSimple = '\u{e3b4}', "pencil-simple";
    /// Gear (settings).
    Gear = '\u{e270}', "gear";
    /// Eraser (rub out / remove).
    Eraser = '\u{e21e}', "eraser";
    /// Paint brush (paint / recolour).
    PaintBrush = '\u{e6f0}', "paint-brush";
    /// Tilted paint bucket (fill).
    PaintBucket = '\u{e392}', "paint-bucket";
    /// Artist's palette (colour choice).
    Palette = '\u{e6c8}', "palette";
    /// Row of colour swatches.
    Swatches = '\u{e5b8}', "swatches";
    /// Isometric cube (3D volume).
    Cube = '\u{e1da}', "cube";
    /// Wireframe cube (transparent volume).
    CubeTransparent = '\u{ec7c}', "cube-transparent";
    /// Shaded sphere (3D ball).
    Sphere = '\u{ee66}', "sphere";
    /// Small filled dot (a single unit).
    Dot = '\u{ecde}', "dot";
    /// Circle outline.
    Circle = '\u{e18a}', "circle";
    /// Square outline.
    Square = '\u{e45e}', "square";
    /// Triangle outline (a wedge in cross-section).
    Triangle = '\u{e4b0}', "triangle";
    /// Geometric angle mark (a corner).
    Angle = '\u{e7bc}', "angle";
    /// Diamond outline (a bevelled solid).
    Diamond = '\u{e1ec}', "diamond";
    /// Circular arrow, clockwise (rotate forwards).
    ArrowClockwise = '\u{e036}', "arrow-clockwise";
    /// Circular arrow, counter-clockwise (rotate backwards).
    ArrowCounterClockwise = '\u{e038}', "arrow-counter-clockwise";
    /// Ruler (measurement / size).
    Ruler = '\u{e6b8}', "ruler";
}

impl PhosphorIcon {
    /// Resolve this icon to a [`IconGlyph`] in the registered Phosphor font.
    ///
    /// `None` only if the codepoint isn't in the vendored font's cmap, which the
    /// unit tests below rule out for the whole curated set — so callers may
    /// treat this as infallible in practice and simply skip drawing.
    pub fn glyph(self) -> Option<IconGlyph> {
        icon_glyph(IconFontId::PHOSPHOR, self.codepoint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_resolves_to_a_real_glyph() {
        for &icon in PhosphorIcon::ALL {
            let g = icon
                .glyph()
                .unwrap_or_else(|| panic!("{icon:?} ({:?}) not in cmap", icon.codepoint()));
            assert_eq!(g.font, IconFontId::PHOSPHOR);
            assert_ne!(g.glyph_id, 0, "{icon:?} resolved to the .notdef glyph");
        }
    }

    #[test]
    fn distinct_icons_have_distinct_glyphs() {
        let mut seen = std::collections::HashSet::new();
        for &icon in PhosphorIcon::ALL {
            let g = icon.glyph().unwrap();
            assert!(
                seen.insert(g.glyph_id),
                "{icon:?} shares a glyph id with another icon"
            );
        }
    }

    #[test]
    fn names_round_trip_through_from_name() {
        for &icon in PhosphorIcon::ALL {
            assert_eq!(
                PhosphorIcon::from_name(icon.name()),
                Some(icon),
                "{icon:?} did not round-trip via its name {:?}",
                icon.name()
            );
        }
    }

    /// `from_name` is the lookup a script-driven UI hits; two icons sharing a
    /// name would silently shadow one another in the match arm.
    #[test]
    fn names_are_unique_and_kebab_case() {
        let mut seen = std::collections::HashSet::new();
        for &icon in PhosphorIcon::ALL {
            let name = icon.name();
            assert!(seen.insert(name), "{name:?} is used by two icons");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{name:?} is not kebab-case"
            );
        }
    }

    #[test]
    fn unknown_name_is_none() {
        assert_eq!(PhosphorIcon::from_name("definitely-not-an-icon"), None);
        assert_eq!(
            PhosphorIcon::from_name("Gear"),
            None,
            "lookup is by kebab name, not variant"
        );
    }
}
