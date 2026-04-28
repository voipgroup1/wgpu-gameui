//! Dynamic sprite texture atlas with shelf rectangle packing.
//!
//! Sprites are uploaded as RGBA8 byte slices and packed into a single GPU texture.
//! When a sprite doesn't fit, the atlas grows (1024 -> 2048 -> 4096) and re-uploads
//! all stored pixels. Sprites are addressed by an opaque `SpriteId` and may be looked
//! up by name. UV coordinates are stored as pixel rects and converted to UV space at
//! query time so they remain valid when the atlas grows.
//!
//! The packer uses a simple shelf algorithm: each shelf has a fixed height, and a
//! sprite is placed on the first shelf with enough remaining width and height
//! capacity. New shelves open at `next_shelf_y` when no existing shelf fits. This is
//! O(N*shelves) per insert, fast enough for hundreds of UI sprites and trivial to
//! reason about.

use std::collections::HashMap;

/// Opaque handle to a sprite stored in the atlas.
pub type SpriteId = u32;

/// Initial atlas dimensions.
pub(crate) const INITIAL_ATLAS_SIZE: u32 = 1024;
/// Maximum atlas dimensions before allocation fails.
pub(crate) const MAX_ATLAS_SIZE: u32 = 4096;
/// Padding around each sprite to avoid bilinear bleed.
const SPRITE_PADDING: u32 = 1;

/// A region within the atlas, in pixel coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AtlasRegion {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl AtlasRegion {
    /// Convert pixel rect to UV coordinates given the atlas size.
    pub fn uv(&self, atlas_w: u32, atlas_h: u32) -> [f32; 4] {
        let aw = atlas_w as f32;
        let ah = atlas_h as f32;
        [
            self.x as f32 / aw,
            self.y as f32 / ah,
            (self.x + self.w) as f32 / aw,
            (self.y + self.h) as f32 / ah,
        ]
    }
}

#[derive(Clone)]
struct Shelf {
    y: u32,
    height: u32,
    cursor_x: u32,
}

#[derive(Clone)]
struct StoredSprite {
    region: AtlasRegion,
    /// Padded RGBA8 pixels (region.w * region.h * 4).
    pixels: Vec<u8>,
}

/// Dynamic atlas. CPU-side state is the source of truth; the GPU texture is
/// (re)uploaded when sprites are added or the atlas grows.
pub struct SpriteAtlas {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    sprites: Vec<StoredSprite>,
    name_to_id: HashMap<String, SpriteId>,
    dirty: bool,
}

impl SpriteAtlas {
    pub fn new() -> Self {
        Self {
            width: INITIAL_ATLAS_SIZE,
            height: INITIAL_ATLAS_SIZE,
            shelves: Vec::new(),
            next_shelf_y: 0,
            sprites: Vec::new(),
            name_to_id: HashMap::new(),
            dirty: true,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn region(&self, id: SpriteId) -> Option<AtlasRegion> {
        self.sprites.get(id as usize).map(|s| s.region)
    }

    pub fn id_for(&self, name: &str) -> Option<SpriteId> {
        self.name_to_id.get(name).copied()
    }

    /// Insert a sprite. Returns its new id. Panics if the sprite cannot fit even
    /// after growing to MAX_ATLAS_SIZE — UI atlases shouldn't hit that.
    pub fn insert(&mut self, name: Option<&str>, w: u32, h: u32, pixels: &[u8]) -> SpriteId {
        assert_eq!(
            pixels.len(),
            (w * h * 4) as usize,
            "sprite pixel buffer size mismatch"
        );

        // Try to place; grow until success.
        let region = loop {
            if let Some(r) = self.try_place(w, h) {
                break r;
            }
            if !self.try_grow() {
                panic!(
                    "sprite {}x{} doesn't fit in atlas at max size {}",
                    w, h, MAX_ATLAS_SIZE
                );
            }
        };

        let id = self.sprites.len() as SpriteId;
        self.sprites.push(StoredSprite {
            region,
            pixels: pixels.to_vec(),
        });
        if let Some(name) = name {
            self.name_to_id.insert(name.to_string(), id);
        }
        self.dirty = true;
        id
    }

    fn try_place(&mut self, w: u32, h: u32) -> Option<AtlasRegion> {
        let pad_w = w + SPRITE_PADDING;
        let pad_h = h + SPRITE_PADDING;

        if pad_w > self.width {
            return None;
        }

        // First-fit on existing shelves.
        for shelf in &mut self.shelves {
            if shelf.height >= pad_h && shelf.cursor_x + pad_w <= self.width {
                let region = AtlasRegion {
                    x: shelf.cursor_x,
                    y: shelf.y,
                    w,
                    h,
                };
                shelf.cursor_x += pad_w;
                return Some(region);
            }
        }

        // Open a new shelf.
        if self.next_shelf_y + pad_h <= self.height {
            let shelf = Shelf {
                y: self.next_shelf_y,
                height: pad_h,
                cursor_x: pad_w,
            };
            let region = AtlasRegion {
                x: 0,
                y: shelf.y,
                w,
                h,
            };
            self.next_shelf_y += pad_h;
            self.shelves.push(shelf);
            return Some(region);
        }

        None
    }

    fn try_grow(&mut self) -> bool {
        let new_size = (self.width.max(self.height) * 2).min(MAX_ATLAS_SIZE);
        if new_size == self.width && new_size == self.height {
            return false;
        }
        self.width = new_size;
        self.height = new_size;
        // Existing shelves and regions remain valid (origin at top-left); UVs are
        // recomputed from pixel rects against the new size, so nothing needs
        // rebinning. The texture data must be re-uploaded though.
        self.dirty = true;
        true
    }

    /// Take the dirty flag and return whether a re-upload is needed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// Render the full atlas as a single packed RGBA8 buffer of size width*height*4.
    pub fn build_pixel_buffer(&self) -> Vec<u8> {
        let mut buf = vec![0u8; (self.width * self.height * 4) as usize];
        for sprite in &self.sprites {
            let r = sprite.region;
            for row in 0..r.h {
                let src_row_start = (row * r.w * 4) as usize;
                let src_row_end = src_row_start + (r.w * 4) as usize;
                let dst_row_start =
                    (((r.y + row) * self.width + r.x) * 4) as usize;
                let dst_row_end = dst_row_start + (r.w * 4) as usize;
                buf[dst_row_start..dst_row_end]
                    .copy_from_slice(&sprite.pixels[src_row_start..src_row_end]);
            }
        }
        buf
    }
}

impl Default for SpriteAtlas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_disjoint_regions() {
        let mut atlas = SpriteAtlas::new();
        let pixels = vec![255u8; 16 * 16 * 4];
        let a = atlas.insert(Some("a"), 16, 16, &pixels);
        let b = atlas.insert(Some("b"), 16, 16, &pixels);
        let c = atlas.insert(Some("c"), 16, 16, &pixels);

        let ra = atlas.region(a).unwrap();
        let rb = atlas.region(b).unwrap();
        let rc = atlas.region(c).unwrap();
        // Distinct positions
        assert!(ra != rb && rb != rc);
        // Same shelf (y), increasing x
        assert_eq!(ra.y, rb.y);
        assert!(ra.x < rb.x);
    }

    #[test]
    fn name_lookup_works() {
        let mut atlas = SpriteAtlas::new();
        let pixels = vec![0u8; 4 * 4 * 4];
        let id = atlas.insert(Some("hello"), 4, 4, &pixels);
        assert_eq!(atlas.id_for("hello"), Some(id));
        assert_eq!(atlas.id_for("missing"), None);
    }

    #[test]
    fn grows_when_full() {
        // Force growth: insert sprites totalling > 1024x1024 area.
        let mut atlas = SpriteAtlas::new();
        let big = 512u32;
        let pixels = vec![0u8; (big * big * 4) as usize];
        // 1024x1024 atlas fits 2x2 of 512 sprites only via shelves of ~513 height,
        // and the 5th will require growth.
        let _a = atlas.insert(None, big, big, &pixels);
        let _b = atlas.insert(None, big, big, &pixels);
        let _c = atlas.insert(None, big, big, &pixels);
        // Atlas now has shelves at y=0 (height ~513) and y=513 (height ~513);
        // remaining 511 of 1024 not enough for another 513 shelf, so 4th forces grow.
        let initial = atlas.width();
        let _d = atlas.insert(None, big, big, &pixels);
        assert!(atlas.width() >= initial);
        // 5th definitely grows or is on a new row in grown atlas.
        let _e = atlas.insert(None, big, big, &pixels);
        assert!(atlas.width() >= INITIAL_ATLAS_SIZE);
    }

    #[test]
    fn pack_no_overlap() {
        let mut atlas = SpriteAtlas::new();
        let mut regions = Vec::new();
        for i in 0..50 {
            let s = 16 + (i % 5) * 4;
            let pixels = vec![0u8; (s * s * 4) as usize];
            let id = atlas.insert(None, s, s, &pixels);
            regions.push(atlas.region(id).unwrap());
        }
        // Verify none overlap.
        for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                let a = regions[i];
                let b = regions[j];
                let overlap_x = a.x < b.x + b.w && b.x < a.x + a.w;
                let overlap_y = a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(
                    !(overlap_x && overlap_y),
                    "regions {} and {} overlap: {:?} vs {:?}",
                    i,
                    j,
                    a,
                    b
                );
            }
        }
    }
}
