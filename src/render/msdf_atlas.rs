//! Glyph MSDF atlas: lazily generates a multi-channel signed distance field for
//! each glyph on first sighting, packs it into a single CPU-side RGBA8 buffer with
//! a shelf packer, and hands out uv rects + placement metrics. The GPU texture is
//! owned by [`crate::render::UiRenderer`] (mirroring how [`super::atlas::SpriteAtlas`]
//! is driven) and re-uploaded from [`MsdfGlyphAtlas::build_pixel_buffer`] whenever
//! [`MsdfGlyphAtlas::take_dirty`] reports a change.
//!
//! ## Why a separate atlas from `SpriteAtlas`
//!
//! * **Format must be linear, not sRGB.** MSDF texels are distances, not colors —
//!   sampling them through an sRGB-decoding view would warp the field. The renderer
//!   creates this atlas's texture as `Rgba8Unorm` (linear) with `FilterMode::Linear`
//!   (MSDF *requires* bilinear).
//! * **No edge-replication halo.** A glyph's MSDF already carries a saturated
//!   "outside" margin (the padding baked in by [`super::glyph_msdf`]); a plain 1px
//!   zero gutter between tiles is enough to stop cross-tile bilinear bleed, since
//!   the gutter value (far-outside) matches the tile edge.
//!
//! ## Lazy generation & caching
//!
//! [`MsdfGlyphAtlas::glyph`] is the only entry point. It caches every lookup —
//! including misses (outline-less glyphs like space) as `None` — so generation
//! happens exactly once per `(font, glyph)` and never on a frame's hot path. Callers
//! should pre-warm the printable-ASCII set at init to avoid first-sighting hitches.

use std::collections::HashMap;

use ttf_parser::{Face, GlyphId};

use super::atlas::AtlasRegion;
use super::glyph_msdf::{GlyphMetrics, generate_glyph_msdf};

/// Initial atlas dimensions.
pub(crate) const INITIAL_MSDF_ATLAS_SIZE: u32 = 1024;
/// Maximum atlas dimensions before allocation fails.
pub(crate) const MAX_MSDF_ATLAS_SIZE: u32 = 4096;
/// Zero gutter (in pixels) reserved on each side of a tile to prevent cross-tile
/// bilinear bleed. No replication needed — see module docs.
const GLYPH_GUTTER: u32 = 1;

/// Default reference EM size (pixels) the distance fields are generated at. Higher
/// = crisper at large display sizes, more atlas space.
pub const DEFAULT_REF_PX: f32 = 40.0;
/// Default distance-ramp width (tile pixels). The shader scales screen-space AA by
/// this; also sets the tile padding and — crucially — the *effect reach*: outlines
/// and shadow/glow blur are only valid within `~(px_range/2) * (font_size/ref_px)`
/// screen px of the edge. `12 / 40` gives ~2.4px reach at a 16px UI font (and ~6px
/// at 40px), enough for 1–2px outlines, soft shadows, and small glow halos without
/// bloating tiles. Fill AA is unaffected by this value (it's computed in screen space).
pub const DEFAULT_PX_RANGE: f32 = 12.0;

/// A glyph's location and placement, handed back from [`MsdfGlyphAtlas::glyph`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphTile {
    /// Pixel rect of the tile content within the atlas.
    pub region: AtlasRegion,
    /// EM-space placement metrics (see [`GlyphMetrics`]).
    pub metrics: GlyphMetrics,
}

#[derive(Clone)]
struct Shelf {
    y: u32,
    /// Cell height (tile height + 2 * GLYPH_GUTTER).
    height: u32,
    cursor_x: u32,
}

#[derive(Clone)]
struct StoredGlyph {
    region: AtlasRegion,
    metrics: GlyphMetrics,
    /// RGBA8 pixels (region.w * region.h * 4). MSDF RGB with alpha forced to 255.
    rgba: Vec<u8>,
}

/// CPU-side glyph MSDF atlas. The GPU texture is owned and uploaded by the renderer.
pub struct MsdfGlyphAtlas {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    glyphs: Vec<StoredGlyph>,
    /// `(font_id, glyph_id)` → stored-glyph index, or `None` for outline-less glyphs
    /// (whitespace) and generation failures. Caches misses so we never retry.
    lookup: HashMap<(u64, u16), Option<u32>>,
    dirty: bool,
    ref_px: f32,
    px_range: f32,
}

impl MsdfGlyphAtlas {
    pub fn new() -> Self {
        Self::with_params(DEFAULT_REF_PX, DEFAULT_PX_RANGE)
    }

    pub fn with_params(ref_px: f32, px_range: f32) -> Self {
        Self {
            width: INITIAL_MSDF_ATLAS_SIZE,
            height: INITIAL_MSDF_ATLAS_SIZE,
            shelves: Vec::new(),
            next_shelf_y: 0,
            glyphs: Vec::new(),
            lookup: HashMap::new(),
            dirty: true,
            ref_px,
            px_range,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The distance-ramp width (tile pixels) the fields were generated with. The
    /// shader needs this to scale screen-space AA: `screen_px_range = px_range *
    /// font_size / ref_px`.
    pub fn px_range(&self) -> f32 {
        self.px_range
    }

    /// The reference EM size (pixels) the fields were generated at.
    pub fn ref_px(&self) -> f32 {
        self.ref_px
    }

    /// Look up (and lazily generate) the MSDF tile for a glyph.
    ///
    /// * `font_id` — a stable per-font key (the caller maps cosmic-text's
    ///   `fontdb::ID` to this).
    /// * `glyph_id` — the shaped glyph index.
    /// * `font_data` — the raw font face bytes (`cosmic_text::Font::data()`); only
    ///   touched on a cache miss.
    ///
    /// Returns `None` for outline-less glyphs (whitespace) — the caller advances the
    /// pen without emitting a quad. Generation happens at most once per `(font_id,
    /// glyph_id)`.
    pub fn glyph(&mut self, font_id: u64, glyph_id: u16, font_data: &[u8]) -> Option<GlyphTile> {
        if let Some(cached) = self.lookup.get(&(font_id, glyph_id)) {
            return cached.map(|idx| self.tile(idx));
        }

        let generated = Face::parse(font_data, 0).ok().and_then(|face| {
            generate_glyph_msdf(&face, GlyphId(glyph_id), self.ref_px, self.px_range)
        });

        match generated {
            Some(g) => {
                let region = self.pack(g.metrics.width_px, g.metrics.height_px);
                let idx = self.glyphs.len() as u32;
                self.glyphs.push(StoredGlyph {
                    region,
                    metrics: g.metrics,
                    rgba: rgb_to_rgba(&g.image),
                });
                self.dirty = true;
                self.lookup.insert((font_id, glyph_id), Some(idx));
                Some(GlyphTile {
                    region,
                    metrics: g.metrics,
                })
            }
            None => {
                self.lookup.insert((font_id, glyph_id), None);
                None
            }
        }
    }

    fn tile(&self, idx: u32) -> GlyphTile {
        let g = &self.glyphs[idx as usize];
        GlyphTile {
            region: g.region,
            metrics: g.metrics,
        }
    }

    /// Take the dirty flag; returns whether a GPU re-upload is needed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// Render the full atlas as a single packed RGBA8 buffer of `width*height*4`.
    /// Tiles are written into their content rect; gutters stay zero (far-outside),
    /// which is the correct neutral value for MSDF bilinear sampling.
    pub fn build_pixel_buffer(&self) -> Vec<u8> {
        let mut buf = vec![0u8; (self.width * self.height * 4) as usize];
        let stride = (self.width * 4) as usize;
        for g in &self.glyphs {
            let r = g.region;
            let row_bytes = (r.w * 4) as usize;
            for row in 0..r.h {
                let src_off = (row * r.w * 4) as usize;
                let dst_off = ((r.y + row) as usize) * stride + (r.x as usize) * 4;
                buf[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&g.rgba[src_off..src_off + row_bytes]);
            }
        }
        buf
    }

    fn pack(&mut self, w: u32, h: u32) -> AtlasRegion {
        loop {
            if let Some(r) = self.try_place(w, h) {
                return r;
            }
            if !self.try_grow() {
                panic!(
                    "glyph {}x{} doesn't fit in MSDF atlas at max size {}",
                    w, h, MAX_MSDF_ATLAS_SIZE
                );
            }
        }
    }

    fn try_place(&mut self, w: u32, h: u32) -> Option<AtlasRegion> {
        let cell_w = w + 2 * GLYPH_GUTTER;
        let cell_h = h + 2 * GLYPH_GUTTER;
        if cell_w > self.width {
            return None;
        }
        for shelf in &mut self.shelves {
            if shelf.height >= cell_h && shelf.cursor_x + cell_w <= self.width {
                let region = AtlasRegion {
                    x: shelf.cursor_x + GLYPH_GUTTER,
                    y: shelf.y + GLYPH_GUTTER,
                    w,
                    h,
                };
                shelf.cursor_x += cell_w;
                return Some(region);
            }
        }
        if self.next_shelf_y + cell_h <= self.height {
            let shelf = Shelf {
                y: self.next_shelf_y,
                height: cell_h,
                cursor_x: cell_w,
            };
            let region = AtlasRegion {
                x: GLYPH_GUTTER,
                y: shelf.y + GLYPH_GUTTER,
                w,
                h,
            };
            self.next_shelf_y += cell_h;
            self.shelves.push(shelf);
            return Some(region);
        }
        None
    }

    fn try_grow(&mut self) -> bool {
        let new_size = (self.width.max(self.height) * 2).min(MAX_MSDF_ATLAS_SIZE);
        if new_size == self.width && new_size == self.height {
            return false;
        }
        self.width = new_size;
        self.height = new_size;
        self.dirty = true;
        true
    }
}

impl Default for MsdfGlyphAtlas {
    fn default() -> Self {
        Self::new()
    }
}

/// Expand an MSDF `RgbImage` to RGBA8 with alpha forced to 255.
fn rgb_to_rgba(img: &image::RgbImage) -> Vec<u8> {
    let mut out = Vec::with_capacity((img.width() * img.height() * 4) as usize);
    for p in img.pixels() {
        out.extend_from_slice(&[p.0[0], p.0[1], p.0[2], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = notosans::REGULAR_TTF;

    #[test]
    fn glyph_generates_caches_and_packs() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;

        let t1 = atlas.glyph(1, a, FONT).expect("A tile");
        assert!(t1.region.w > 0 && t1.region.h > 0);
        // Second lookup is cached — same region, no new stored glyph.
        let before = atlas.glyphs.len();
        let t2 = atlas.glyph(1, a, FONT).expect("A tile cached");
        assert_eq!(t1, t2);
        assert_eq!(atlas.glyphs.len(), before, "cached lookup must not re-pack");
    }

    #[test]
    fn whitespace_is_cached_as_miss() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let space = face.glyph_index(' ').unwrap().0;
        assert!(atlas.glyph(1, space, FONT).is_none());
        // Cached as a miss — no stored glyph, and a repeat is still None.
        assert_eq!(atlas.glyphs.len(), 0);
        assert!(atlas.glyph(1, space, FONT).is_none());
        assert_eq!(atlas.lookup.len(), 1);
    }

    #[test]
    fn distinct_fonts_keyed_separately() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;
        let t_font1 = atlas.glyph(1, a, FONT).unwrap();
        let t_font2 = atlas.glyph(2, a, FONT).unwrap();
        // Same glyph id, different font key → two separate tiles.
        assert_ne!(t_font1.region, t_font2.region);
        assert_eq!(atlas.glyphs.len(), 2);
    }

    #[test]
    fn tiles_do_not_overlap_including_gutter() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let mut regions = Vec::new();
        for code in 0x21u8..=0x7e {
            let c = code as char;
            if let Some(g) = face.glyph_index(c)
                && let Some(t) = atlas.glyph(1, g.0, FONT)
            {
                regions.push(t.region);
            }
        }
        assert!(regions.len() > 90);
        for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                let a = regions[i];
                let b = regions[j];
                let overlap_x = a.x < b.x + b.w && b.x < a.x + a.w;
                let overlap_y = a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(
                    !(overlap_x && overlap_y),
                    "tiles {i} and {j} overlap: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn pixel_buffer_matches_atlas_size() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;
        atlas.glyph(1, a, FONT);
        let buf = atlas.build_pixel_buffer();
        assert_eq!(buf.len(), (atlas.width() * atlas.height() * 4) as usize);
        assert!(atlas.take_dirty());
        // Dirty consumed.
        assert!(!atlas.take_dirty());
    }
}
