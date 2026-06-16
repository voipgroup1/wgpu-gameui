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

use std::sync::OnceLock;

use ttf_parser::Face;

/// The vendored Phosphor regular-weight font (MIT licensed — see
/// `assets/fonts/phosphor/LICENSE`).
pub(crate) const PHOSPHOR_TTF: &[u8] =
    include_bytes!("../../assets/fonts/phosphor/Phosphor-Regular.ttf");

/// Reserved font key for the icon atlas. Text font keys are assigned at runtime
/// starting from 0, so a reserved high constant can never collide — even if the
/// text and icon atlases were ever merged.
pub(crate) const PHOSPHOR_FONT_ID: u64 = u64::MAX;

/// Curated set of Phosphor icons exposed by the library. Codepoints are the
/// Phosphor regular-weight Private-Use-Area assignments; they are verified
/// against the vendored font's cmap by the unit tests in this module.
///
/// `#[non_exhaustive]` so we can grow the set without it being a breaking change.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PhosphorIcon {
    /// Upward-pointing caret (e.g. collapse / increment).
    CaretUp,
    /// Downward-pointing caret (e.g. expand / decrement).
    CaretDown,
    /// Plus sign (add / increment).
    Plus,
    /// Minus sign (remove / decrement).
    Minus,
    /// Checkmark (confirm / enabled state).
    Check,
    /// Cross (close / cancel).
    X,
    /// Open eye (reveal / visible).
    Eye,
    /// Crossed-out eye (hide / hidden).
    EyeSlash,
    /// Trash can (delete).
    Trash,
    /// Simple pencil (edit).
    PencilSimple,
    /// Gear (settings).
    Gear,
}

impl PhosphorIcon {
    /// Every variant, for prewarming the atlas and for tests.
    pub(crate) const ALL: &'static [PhosphorIcon] = &[
        PhosphorIcon::CaretUp,
        PhosphorIcon::CaretDown,
        PhosphorIcon::Plus,
        PhosphorIcon::Minus,
        PhosphorIcon::Check,
        PhosphorIcon::X,
        PhosphorIcon::Eye,
        PhosphorIcon::EyeSlash,
        PhosphorIcon::Trash,
        PhosphorIcon::PencilSimple,
        PhosphorIcon::Gear,
    ];

    /// The glyph's Private-Use-Area codepoint in the Phosphor regular font.
    pub fn codepoint(self) -> char {
        match self {
            PhosphorIcon::CaretUp => '\u{e13c}',
            PhosphorIcon::CaretDown => '\u{e136}',
            PhosphorIcon::Plus => '\u{e3d4}',
            PhosphorIcon::Minus => '\u{e32a}',
            PhosphorIcon::Check => '\u{e182}',
            PhosphorIcon::X => '\u{e4f6}',
            PhosphorIcon::Eye => '\u{e220}',
            PhosphorIcon::EyeSlash => '\u{e224}',
            PhosphorIcon::Trash => '\u{e4a6}',
            PhosphorIcon::PencilSimple => '\u{e3b4}',
            PhosphorIcon::Gear => '\u{e270}',
        }
    }
}

/// The parsed Phosphor face, kept alive for the program's lifetime so glyph-id
/// resolution doesn't re-parse the font on every call. The `'static` font bytes
/// make this sound.
fn phosphor_face() -> &'static Face<'static> {
    static FACE: OnceLock<Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| Face::parse(PHOSPHOR_TTF, 0).expect("parse vendored Phosphor font"))
}

/// Resolve an icon to its glyph index in the Phosphor font, or `None` if the
/// codepoint isn't present (should never happen for the curated set — the tests
/// guard it).
pub(crate) fn phosphor_glyph_id(icon: PhosphorIcon) -> Option<u16> {
    phosphor_face().glyph_index(icon.codepoint()).map(|g| g.0)
}

/// The raw Phosphor font bytes, handed to [`crate::render::MsdfGlyphAtlas::glyph`]
/// for MSDF generation on a cache miss.
pub(crate) fn phosphor_font_data() -> &'static [u8] {
    PHOSPHOR_TTF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_resolves_to_a_real_glyph() {
        for &icon in PhosphorIcon::ALL {
            let gid = phosphor_glyph_id(icon)
                .unwrap_or_else(|| panic!("{icon:?} ({:?}) not in cmap", icon.codepoint()));
            assert_ne!(gid, 0, "{icon:?} resolved to the .notdef glyph");
        }
    }

    #[test]
    fn distinct_icons_have_distinct_glyphs() {
        let mut seen = std::collections::HashSet::new();
        for &icon in PhosphorIcon::ALL {
            let gid = phosphor_glyph_id(icon).unwrap();
            assert!(
                seen.insert(gid),
                "{icon:?} shares a glyph id with another icon"
            );
        }
    }
}
