//! Registry of icon fonts — the vendored Phosphor set plus any the application
//! supplies.
//!
//! An *icon font* is an ordinary TTF whose glyphs are monochrome symbols mapped
//! to Private-Use-Area codepoints. The library already renders such glyphs
//! through [`MsdfGlyphAtlas`](crate::render::MsdfGlyphAtlas), which is keyed by
//! `(font_id, glyph_id)` — so a second font needs no new atlas, pipeline, bind
//! group or draw call. It only needs an id, its bytes reachable at MSDF
//! generation time, and a cmap reachable when a draw command is built.
//!
//! That is all this module is.
//!
//! ## Why `&'static [u8]`
//!
//! A [`ttf_parser::Face`] borrows the bytes it was parsed from. Keeping a parsed
//! face in a process-wide registry therefore requires the bytes to outlive it,
//! and `'static` is the honest way to say so. It also matches how fonts actually
//! ship: `include_bytes!` in the application binary, exactly as Phosphor is
//! vendored here. To register a font read from disk at runtime, leak it —
//! `Box::leak(bytes.into_boxed_slice())` — which is not a real leak, since an
//! icon font is loaded once and lives for the process.
//!
//! ## Resolve once, draw many
//!
//! [`icon_glyph`] takes the registry's read lock. Call it at startup and keep the
//! returned [`IconGlyph`] handles — they are plain `Copy` integers. Draw commands
//! carry a resolved `IconGlyph`, so the registry is never touched on the per-frame
//! draw path.
//!
//! This whole module is gated behind the `phosphor-icons` feature.

use std::sync::{OnceLock, RwLock};

use ttf_parser::Face;

use super::phosphor::{PHOSPHOR_TTF, phosphor_font_name};

/// Opaque handle to a registered icon font, from [`register_icon_font`].
///
/// The numeric value is the font's index in the registry and doubles as its
/// `font_id` in the MSDF atlas. Ids start at 0 and are never reused.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IconFontId(pub(crate) u32);

impl IconFontId {
    /// The built-in Phosphor font. Always registered, always id 0.
    pub const PHOSPHOR: IconFontId = IconFontId(0);

    /// The font's index, which is also its MSDF-atlas `font_id`.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// A glyph resolved against a specific icon font — what a draw command carries.
///
/// Obtained from [`icon_glyph`] (or, for the built-in set,
/// [`PhosphorIcon::glyph`](crate::PhosphorIcon::glyph)). Resolving is a cmap
/// lookup behind a lock, so do it once and keep the handle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconGlyph {
    /// Which registered font the glyph belongs to.
    pub font: IconFontId,
    /// The glyph's index within that font.
    pub glyph_id: u16,
}

/// One registered font: its lookup name, its bytes, and its parsed face.
struct IconFontEntry {
    name: &'static str,
    data: &'static [u8],
    /// Parsed once so `icon_glyph` is a cmap lookup rather than a re-parse.
    /// Sound because `data` is `'static`.
    face: Face<'static>,
}

/// The process-wide registry. Phosphor occupies index 0 (installed by
/// [`registry`] on first touch), application fonts follow in registration order.
fn registry() -> &'static RwLock<Vec<IconFontEntry>> {
    static FONTS: OnceLock<RwLock<Vec<IconFontEntry>>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let face = Face::parse(PHOSPHOR_TTF, 0).expect("parse vendored Phosphor font");
        RwLock::new(vec![IconFontEntry {
            name: phosphor_font_name(),
            data: PHOSPHOR_TTF,
            face,
        }])
    })
}

/// Register an icon font under `name` and return its handle.
///
/// **Idempotent**: registering a name that is already present returns the
/// existing handle without re-parsing, so an application can call this from
/// several entry points (or once per window) without accumulating fonts.
///
/// Returns `None` if `data` is not a parseable font face.
///
/// ```ignore
/// static SHIP_ICONS: &[u8] = include_bytes!("../assets/ship-icons.ttf");
/// let font = register_icon_font("ship-icons", SHIP_ICONS).expect("valid TTF");
/// let anchor = icon_glyph(font, '\u{e001}').expect("anchor glyph");
/// ```
pub fn register_icon_font(name: &'static str, data: &'static [u8]) -> Option<IconFontId> {
    // Fast path: already registered. Taking the read lock first keeps repeat
    // calls from serialising on the writer.
    if let Some(existing) = icon_font_id(name) {
        return Some(existing);
    }
    let face = Face::parse(data, 0).ok()?;
    let mut fonts = registry().write().ok()?;
    // Re-check under the write lock: two threads can pass the read-lock check.
    if let Some(i) = fonts.iter().position(|f| f.name == name) {
        return Some(IconFontId(i as u32));
    }
    let id = IconFontId(fonts.len() as u32);
    fonts.push(IconFontEntry { name, data, face });
    Some(id)
}

/// Look a registered icon font up by name. Mirrors
/// [`UiRenderer::sprite_id`](crate::UiRenderer::sprite_id).
pub fn icon_font_id(name: &str) -> Option<IconFontId> {
    let fonts = registry().read().ok()?;
    fonts
        .iter()
        .position(|f| f.name == name)
        .map(|i| IconFontId(i as u32))
}

/// Resolve `ch` to a glyph in `font`, or `None` if the font is unregistered or
/// has no glyph for that codepoint.
///
/// Takes the registry's read lock — resolve at startup and keep the handle
/// rather than calling this per frame.
pub fn icon_glyph(font: IconFontId, ch: char) -> Option<IconGlyph> {
    let fonts = registry().read().ok()?;
    let entry = fonts.get(font.0 as usize)?;
    let glyph_id = entry.face.glyph_index(ch)?.0;
    Some(IconGlyph { font, glyph_id })
}

/// The raw bytes `font` was registered with, or `None` if it is unregistered.
///
/// Exposed so a caller can feed a registered font to
/// [`MsdfGlyphAtlas::glyph`](crate::render::MsdfGlyphAtlas::glyph) directly —
/// e.g. to compare a new icon font's glyph metrics against Phosphor's without
/// standing up a GPU.
pub fn icon_font_data(font: IconFontId) -> Option<&'static [u8]> {
    let fonts = registry().read().ok()?;
    fonts.get(font.0 as usize).map(|f| f.data)
}

/// Every registered font's bytes, handed to
/// [`MsdfGlyphAtlas::glyph`](crate::render::MsdfGlyphAtlas::glyph) for MSDF
/// generation on a cache miss, indexed by [`IconFontId::index`].
///
/// The render/prewarm paths call this **once per batch** and then index the
/// returned `Vec` per icon: the bytes are `&'static`, so the lock is released
/// before any MSDF generation happens. An id past the end of the `Vec` is an
/// unregistered font, and its icons are skipped.
pub(crate) fn icon_font_snapshot() -> Vec<&'static [u8]> {
    match registry().read() {
        Ok(fonts) => fonts.iter().map(|f| f.data).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second, distinct face to register. Any parseable TTF works; the bundled
    /// Noto face is `'static` and always present under the default features.
    #[cfg(feature = "bundled-font")]
    fn second_font() -> &'static [u8] {
        notosans::REGULAR_TTF
    }

    #[test]
    fn phosphor_is_always_font_zero() {
        assert_eq!(IconFontId::PHOSPHOR.index(), 0);
        assert!(
            !icon_font_snapshot().is_empty(),
            "Phosphor must be registered without any explicit call"
        );
        assert_eq!(icon_font_snapshot()[0], PHOSPHOR_TTF);
    }

    /// The snapshot is what the render pass indexes per icon, so its ordering
    /// must be the id ordering — not merely "contains the right bytes".
    #[cfg(feature = "bundled-font")]
    #[test]
    fn the_snapshot_is_indexed_by_font_id() {
        let font = register_icon_font("test-snapshot-order", second_font()).expect("valid font");
        let snap = icon_font_snapshot();
        assert_eq!(snap[font.index() as usize], second_font());
        assert_eq!(snap[IconFontId::PHOSPHOR.index() as usize], PHOSPHOR_TTF);
    }

    #[test]
    fn phosphor_glyphs_resolve_through_the_registry() {
        let g = icon_glyph(IconFontId::PHOSPHOR, crate::PhosphorIcon::Gear.codepoint())
            .expect("gear resolves");
        assert_eq!(g.font, IconFontId::PHOSPHOR);
        assert_ne!(g.glyph_id, 0, "resolved to .notdef");
    }

    #[test]
    fn unknown_codepoint_is_none() {
        // Phosphor's cmap covers a PUA range plus a couple of control-range
        // entries, so pick something far outside it: U+2603 SNOWMAN.
        assert_eq!(icon_glyph(IconFontId::PHOSPHOR, '\u{2603}'), None);
    }

    #[test]
    fn unregistered_font_resolves_to_none_rather_than_panicking() {
        let bogus = IconFontId(9999);
        assert_eq!(icon_glyph(bogus, 'a'), None);
        assert!(icon_font_snapshot().get(bogus.index() as usize).is_none());
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn registering_is_idempotent_by_name() {
        let a = register_icon_font("test-idempotent", second_font()).expect("valid font");
        let b = register_icon_font("test-idempotent", second_font()).expect("valid font");
        assert_eq!(a, b, "re-registering a name must reuse the handle");
        assert_eq!(icon_font_id("test-idempotent"), Some(a));
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn a_registered_font_is_keyed_apart_from_phosphor() {
        let font = register_icon_font("test-distinct", second_font()).expect("valid font");
        assert_ne!(font, IconFontId::PHOSPHOR);
        // 'A' exists in a text font but not in Phosphor's PUA-only cmap, so the
        // two fonts genuinely resolve different things under the same char.
        assert!(icon_glyph(font, 'A').is_some(), "text font has 'A'");
        assert_eq!(icon_glyph(IconFontId::PHOSPHOR, 'A'), None);
    }

    #[test]
    fn garbage_bytes_are_rejected_not_panicked_on() {
        static GARBAGE: &[u8] = b"this is definitely not a font";
        assert_eq!(register_icon_font("test-garbage", GARBAGE), None);
        assert_eq!(icon_font_id("test-garbage"), None);
    }

    #[test]
    fn unknown_font_name_is_none() {
        assert_eq!(icon_font_id("no-such-font"), None);
    }
}
