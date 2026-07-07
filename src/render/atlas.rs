//! Dynamic sprite texture atlas with shelf rectangle packing.
//!
//! Sprites are uploaded as RGBA8 byte slices and packed into a single GPU texture.
//! When a sprite doesn't fit, the atlas grows (1024 -> 2048 -> 4096) and re-uploads
//! all stored pixels. Sprites are addressed by an opaque `SpriteId` and may be looked
//! up by name. UV coordinates are stored as pixel rects and converted to UV space at
//! query time so they remain valid when the atlas grows.
//!
//! ## Bilinear bleed prevention
//!
//! Each sprite occupies a `(w + 2) x (h + 2)` cell in the atlas, with the sprite
//! content placed at offset `(1, 1)` inside the cell. The 1-pixel halo around every
//! sprite is filled with replicated edge pixels at upload time (see
//! [`SpriteAtlas::build_pixel_buffer`]) so bilinear sampling at the sprite's
//! boundary reads `(edge, edge)` instead of `(edge, transparent_gutter)`. Without
//! this, every non-corner sprite would visibly darken / fade at its edges under
//! [`wgpu::FilterMode::Linear`] sampling.
//!
//! ## Algorithm
//!
//! Shelf packer: each shelf has a fixed height, a sprite is placed on the first
//! shelf with enough remaining width and height capacity. New shelves open at
//! `next_shelf_y` when no existing shelf fits. O(N * shelves) per insert, fast
//! enough for hundreds of UI sprites and trivial to reason about.

use std::collections::HashMap;

/// Opaque handle to a sprite stored in the atlas.
pub type SpriteId = u32;

/// Initial atlas dimensions.
pub(crate) const INITIAL_ATLAS_SIZE: u32 = 1024;
/// Maximum atlas dimensions before allocation fails.
pub(crate) const MAX_ATLAS_SIZE: u32 = 4096;
/// Halo (replicated-edge gutter) width on each side of a sprite, in pixels.
const SPRITE_HALO: u32 = 1;

/// A region within the atlas, in pixel coordinates. Refers to the *content* rect
/// — the surrounding 1-pixel halo is implicit and never sampled directly.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AtlasRegion {
    /// X offset of the content rect within the atlas, in pixels.
    pub x: u32,
    /// Y offset of the content rect within the atlas, in pixels.
    pub y: u32,
    /// Content width in pixels.
    pub w: u32,
    /// Content height in pixels.
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
    /// Cell height (sprite height + 2 * SPRITE_HALO).
    height: u32,
    cursor_x: u32,
}

#[derive(Clone)]
struct StoredSprite {
    region: AtlasRegion,
    /// Sprite RGBA8 pixels (region.w * region.h * 4) — content only, no halo.
    pixels: Vec<u8>,
    /// The name this sprite was inserted under, if any. Kept so [`SpriteAtlas::remove`]
    /// can clear the [`name_to_id`](SpriteAtlas::name_to_id) entry in O(1) without a
    /// reverse index.
    name: Option<String>,
}

/// Dynamic atlas. CPU-side state is the source of truth; the GPU texture is
/// (re)uploaded when sprites are added or the atlas grows.
pub struct SpriteAtlas {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    /// Indexed by `SpriteId`. `None` slots are **tombstones** — sprites that were
    /// [`remove`](Self::remove)d. The slot index is the sprite's permanent id, so
    /// removing a sprite never shifts or renumbers the others (live `SpriteId`s
    /// held elsewhere stay valid). Tombstone slots are recycled by [`insert`](Self::insert)
    /// via [`free_list`](Self::free_list "structfield"), and their shelf footprint
    /// is reclaimed by [`compact`](Self::compact).
    sprites: Vec<Option<StoredSprite>>,
    /// Tombstoned slot indices available for reuse by [`insert`](Self::insert),
    /// keeping the `sprites` `Vec` from growing without bound under churn.
    free_list: Vec<SpriteId>,
    name_to_id: HashMap<String, SpriteId>,
    dirty: bool,
}

impl SpriteAtlas {
    /// Create an empty atlas at the initial size, flagged dirty for first upload.
    pub fn new() -> Self {
        Self {
            width: INITIAL_ATLAS_SIZE,
            height: INITIAL_ATLAS_SIZE,
            shelves: Vec::new(),
            next_shelf_y: 0,
            sprites: Vec::new(),
            free_list: Vec::new(),
            name_to_id: HashMap::new(),
            dirty: true,
        }
    }

    /// Current atlas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Current atlas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The content region for a sprite id, or `None` if the id is unknown (out
    /// of range) or the slot has been [`remove`](Self::remove)d (tombstoned).
    pub fn region(&self, id: SpriteId) -> Option<AtlasRegion> {
        self.sprites
            .get(id as usize)
            .and_then(|slot| slot.as_ref().map(|s| s.region))
    }

    /// Look up a sprite id by the name it was inserted under. Returns `None` for
    /// a name whose sprite was [`remove`](Self::remove)d.
    pub fn id_for(&self, name: &str) -> Option<SpriteId> {
        self.name_to_id.get(name).copied()
    }

    /// Insert a sprite. Returns its new id. Panics if the sprite cannot fit even
    /// after growing to MAX_ATLAS_SIZE — UI atlases shouldn't hit that.
    ///
    /// A previously [`remove`](Self::remove)d slot is recycled if one is
    /// available, so the `SpriteId` space does not grow without bound under
    /// load-then-unload churn.
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

        let stored = StoredSprite {
            region,
            pixels: pixels.to_vec(),
            name: name.map(|n| n.to_string()),
        };
        // Reuse a tombstoned slot if one is free; otherwise append. Either way the
        // slot index is the sprite's stable id.
        let id = if let Some(recycled) = self.free_list.pop() {
            self.sprites[recycled as usize] = Some(stored);
            recycled
        } else {
            let id = self.sprites.len() as SpriteId;
            self.sprites.push(Some(stored));
            id
        };
        if let Some(name) = name {
            self.name_to_id.insert(name.to_string(), id);
        }
        self.dirty = true;
        id
    }

    /// Drop a sprite by id, tombstoning its slot so the index can be recycled by
    /// a later [`insert`](Self::insert) and its shelf footprint reclaimed by
    /// [`compact`](Self::compact). Frees the CPU-side pixel buffer immediately;
    /// the GPU texture is re-uploaded (without this sprite's pixels) on the next
    /// render because this sets the dirty flag.
    ///
    /// Returns `true` if the sprite was present and removed, `false` if the id is
    /// out of range or already a tombstone. The sprite's name (if any) is cleared
    /// from the name map, so [`id_for`](Self::id_for) no longer resolves it.
    ///
    /// **SpriteId stability:** this never shifts or renumbers other sprites. Any
    /// live `SpriteId` issued before this call remains valid; only the removed id
    /// becomes a tombstone (its [`region`](Self::region) now returns `None`).
    pub fn remove(&mut self, id: SpriteId) -> bool {
        let Some(slot) = self.sprites.get_mut(id as usize) else {
            return false;
        };
        let Some(stored) = slot.take() else {
            return false; // already a tombstone
        };
        if let Some(name) = &stored.name {
            // Only clear the map entry if it still points at us (a name reuse via
            // insert would have overwritten it to a different id).
            if self.name_to_id.get(name).copied() == Some(id) {
                self.name_to_id.remove(name);
            }
        }
        self.free_list.push(id);
        self.dirty = true;
        true
    }

    /// Reclaim shelf fragmentation left by [`remove`](Self::remove) by repacking
    /// every live sprite into fresh contiguous shelves, preserving each sprite's
    /// `SpriteId` (index). Tombstoned slots are skipped and keep their index (a
    /// later [`insert`](Self::insert) recycles them).
    ///
    /// Safe to call at any time: atlas regions are pixel rects re-derived into
    /// UVs every render frame, and a dirty atlas triggers a full GPU re-upload,
    /// so reassigning regions here is picked up automatically — exactly the
    /// invariant [`try_grow`](Self::try_grow "method") already relies on. Idempotent
    /// when there are no tombstones.
    pub fn compact(&mut self) {
        // Nothing to reclaim if nothing was ever removed.
        if self.free_list.is_empty() {
            return;
        }
        // Snapshot the live sprites (index + dims) up front: repacking mutates
        // `self` (the shelves), which would alias a live iterator over
        // `self.sprites`. Owned data here, no outstanding borrow in the loop.
        let live: Vec<(SpriteId, u32, u32)> = self
            .sprites
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                slot.as_ref().map(|s| (i as SpriteId, s.region.w, s.region.h))
            })
            .collect();

        self.shelves.clear();
        self.next_shelf_y = 0;
        for (id, w, h) in live {
            let region = loop {
                if let Some(r) = self.try_place(w, h) {
                    break r;
                }
                if !self.try_grow() {
                    // Shouldn't happen — the live set fit before compaction — but
                    // guard rather than silently drop a sprite.
                    panic!(
                        "sprite {}x{} doesn't fit during atlas compaction at max size {}",
                        w, h, MAX_ATLAS_SIZE
                    );
                }
            };
            // The slot is still live (compaction doesn't tombstone anything).
            self.sprites[id as usize].as_mut().unwrap().region = region;
        }
        self.dirty = true;
    }

    fn try_place(&mut self, w: u32, h: u32) -> Option<AtlasRegion> {
        // Each sprite reserves a (w + 2*halo) x (h + 2*halo) cell so its 1px
        // replicated-edge halo can sit inside the cell without colliding with the
        // neighbours' halos.
        let cell_w = w + 2 * SPRITE_HALO;
        let cell_h = h + 2 * SPRITE_HALO;

        if cell_w > self.width {
            return None;
        }

        // First-fit on existing shelves.
        for shelf in &mut self.shelves {
            if shelf.height >= cell_h && shelf.cursor_x + cell_w <= self.width {
                let region = AtlasRegion {
                    x: shelf.cursor_x + SPRITE_HALO,
                    y: shelf.y + SPRITE_HALO,
                    w,
                    h,
                };
                shelf.cursor_x += cell_w;
                return Some(region);
            }
        }

        // Open a new shelf.
        if self.next_shelf_y + cell_h <= self.height {
            let shelf = Shelf {
                y: self.next_shelf_y,
                height: cell_h,
                cursor_x: cell_w,
            };
            let region = AtlasRegion {
                x: SPRITE_HALO,
                y: shelf.y + SPRITE_HALO,
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
    /// Each sprite is written into its content rect *and* replicated 1 pixel into
    /// the surrounding halo (top/bottom rows, left/right columns, and the four
    /// corner pixels) so bilinear sampling at the content edge reads the sprite's
    /// own colour, not the neighbouring transparent gutter.
    pub fn build_pixel_buffer(&self) -> Vec<u8> {
        let mut buf = vec![0u8; (self.width * self.height * 4) as usize];
        let stride = (self.width * 4) as usize;

        let put = |buf: &mut [u8], x: u32, y: u32, src: &[u8]| {
            if x >= self.width || y >= self.height {
                return;
            }
            let off = (y as usize) * stride + (x as usize) * 4;
            buf[off..off + 4].copy_from_slice(src);
        };

        for slot in &self.sprites {
            let Some(sprite) = slot else {
                continue; // tombstone — contributes no pixels
            };
            let r = sprite.region;
            let row_bytes = (r.w * 4) as usize;

            // Content
            for row in 0..r.h {
                let src_off = (row * r.w * 4) as usize;
                let dst_off = ((r.y + row) as usize) * stride + (r.x as usize) * 4;
                buf[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&sprite.pixels[src_off..src_off + row_bytes]);
            }

            // Top/bottom edge replication (covers the row above / below the content).
            if r.y >= 1 {
                let src_off = 0usize;
                let dst_off = ((r.y - 1) as usize) * stride + (r.x as usize) * 4;
                buf[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&sprite.pixels[src_off..src_off + row_bytes]);
            }
            if r.y + r.h < self.height {
                let last_row = r.h - 1;
                let src_off = (last_row * r.w * 4) as usize;
                let dst_off = ((r.y + r.h) as usize) * stride + (r.x as usize) * 4;
                buf[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&sprite.pixels[src_off..src_off + row_bytes]);
            }

            // Left / right edge replication (column-by-column).
            for row in 0..r.h {
                let left_src = (row * r.w * 4) as usize;
                let right_src = (row * r.w * 4 + (r.w - 1) * 4) as usize;
                if r.x >= 1 {
                    put(
                        &mut buf,
                        r.x - 1,
                        r.y + row,
                        &sprite.pixels[left_src..left_src + 4],
                    );
                }
                if r.x + r.w < self.width {
                    put(
                        &mut buf,
                        r.x + r.w,
                        r.y + row,
                        &sprite.pixels[right_src..right_src + 4],
                    );
                }
            }

            // Four corners — replicate the matching corner pixel into the
            // diagonal halo cell so bilinear at the sprite corner reads
            // (corner, corner, corner, corner).
            let tl = 0usize;
            let tr = ((r.w - 1) * 4) as usize;
            let bl = ((r.h - 1) * r.w * 4) as usize;
            let br = ((r.h - 1) * r.w * 4 + (r.w - 1) * 4) as usize;
            if r.x >= 1 && r.y >= 1 {
                put(&mut buf, r.x - 1, r.y - 1, &sprite.pixels[tl..tl + 4]);
            }
            if r.x + r.w < self.width && r.y >= 1 {
                put(&mut buf, r.x + r.w, r.y - 1, &sprite.pixels[tr..tr + 4]);
            }
            if r.x >= 1 && r.y + r.h < self.height {
                put(&mut buf, r.x - 1, r.y + r.h, &sprite.pixels[bl..bl + 4]);
            }
            if r.x + r.w < self.width && r.y + r.h < self.height {
                put(&mut buf, r.x + r.w, r.y + r.h, &sprite.pixels[br..br + 4]);
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
        // With 512x512 sprites at INITIAL_ATLAS_SIZE=1024, cell height is 514, so
        // only one shelf fits. The 2nd insert is forced to grow.
        let mut atlas = SpriteAtlas::new();
        let big = 512u32;
        let pixels = vec![0u8; (big * big * 4) as usize];
        let _a = atlas.insert(None, big, big, &pixels);
        assert_eq!(atlas.width(), INITIAL_ATLAS_SIZE);
        let _b = atlas.insert(None, big, big, &pixels);
        // Second insert should have triggered a grow.
        assert!(
            atlas.width() > INITIAL_ATLAS_SIZE,
            "atlas should have grown after second 512x512 insert (was {})",
            atlas.width()
        );
    }

    #[test]
    fn pack_no_overlap() {
        // Stress test: many small sprites of mixed sizes. Verifies regions are
        // disjoint AND have at least 1 pixel of separation in every direction
        // (the halo gutter), which is the real invariant the packer must
        // maintain to prevent bilinear bleed across sprites.
        let mut atlas = SpriteAtlas::new();
        let mut regions = Vec::new();
        for i in 0..50 {
            let s = 16 + (i % 5) * 4;
            let pixels = vec![0u8; (s * s * 4) as usize];
            let id = atlas.insert(None, s, s, &pixels);
            regions.push(atlas.region(id).unwrap());
        }
        for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                let a = regions[i];
                let b = regions[j];
                // Treat each region as inflated by 1 pixel in all directions
                // (i.e. its halo). Inflated rects must NOT overlap.
                let ax0 = a.x.saturating_sub(1);
                let ay0 = a.y.saturating_sub(1);
                let ax1 = a.x + a.w + 1;
                let ay1 = a.y + a.h + 1;
                let bx0 = b.x.saturating_sub(1);
                let by0 = b.y.saturating_sub(1);
                let bx1 = b.x + b.w + 1;
                let by1 = b.y + b.h + 1;
                let overlap_x = ax0 < bx1 && bx0 < ax1;
                let overlap_y = ay0 < by1 && by0 < ay1;
                assert!(
                    !(overlap_x && overlap_y),
                    "regions {} and {} (incl. halo) overlap: {:?} vs {:?}",
                    i,
                    j,
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn halo_is_replicated_at_upload() {
        // Place two sprites of distinctive solid colours adjacent on the same
        // shelf. Verify that the gutter pixel between them carries each
        // sprite's edge colour (NOT zeroes), so bilinear at the boundary won't
        // bleed transparency.
        let mut atlas = SpriteAtlas::new();
        let red = [255u8, 0, 0, 255];
        let green = [0u8, 255, 0, 255];
        let red_pixels: Vec<u8> = red.repeat(8 * 8);
        let green_pixels: Vec<u8> = green.repeat(8 * 8);
        let r_id = atlas.insert(Some("red"), 8, 8, &red_pixels);
        let g_id = atlas.insert(Some("green"), 8, 8, &green_pixels);
        let buf = atlas.build_pixel_buffer();
        let stride = (atlas.width() * 4) as usize;
        let read = |x: u32, y: u32| -> [u8; 4] {
            let off = (y as usize) * stride + (x as usize) * 4;
            [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]
        };

        let r = atlas.region(r_id).unwrap();
        let g = atlas.region(g_id).unwrap();

        // Right halo of red: column at r.x + r.w should be solid red (any row in range).
        for row in 0..r.h {
            assert_eq!(
                read(r.x + r.w, r.y + row),
                red,
                "red right halo not replicated at ({}, {})",
                r.x + r.w,
                r.y + row
            );
        }
        // Left halo of green: column at g.x - 1 should be solid green.
        assert!(g.x >= 1);
        for row in 0..g.h {
            assert_eq!(
                read(g.x - 1, g.y + row),
                green,
                "green left halo not replicated at ({}, {})",
                g.x - 1,
                g.y + row
            );
        }
        // Top halo of red — first row above sprite should be solid red.
        assert!(r.y >= 1);
        for col in 0..r.w {
            assert_eq!(read(r.x + col, r.y - 1), red);
        }
    }

    #[test]
    fn remove_tombstones_slot_and_clears_name() {
        let mut atlas = SpriteAtlas::new();
        let pixels = vec![0u8; 8 * 8 * 4];
        let a = atlas.insert(Some("a"), 8, 8, &pixels);
        let b = atlas.insert(Some("b"), 8, 8, &pixels);

        // Removing `a` tombstones its slot and clears its name.
        assert!(atlas.remove(a));
        assert_eq!(atlas.region(a), None, "tombstoned slot region is None");
        assert_eq!(
            atlas.id_for("a"),
            None,
            "name map entry for removed sprite is cleared"
        );
        // The other sprite is untouched — its id still resolves.
        let rb = atlas.region(b);
        assert!(rb.is_some(), "unrelated sprite id stays valid");
        assert_eq!(atlas.id_for("b"), Some(b));

        // Removing an out-of-range id or an already-tombstoned slot is a no-op.
        assert!(!atlas.remove(a), "re-removing a tombstone is a no-op");
        assert!(!atlas.remove(9999), "out-of-range id is a no-op");
    }

    #[test]
    fn slot_reuse_after_remove() {
        let mut atlas = SpriteAtlas::new();
        let pixels = vec![1u8; 8 * 8 * 4];
        let a = atlas.insert(Some("a"), 8, 8, &pixels);

        assert!(atlas.remove(a));

        // The next insert recycles `a`'s tombstoned slot rather than appending a
        // new index — so the SpriteId space does not grow under churn.
        let b = atlas.insert(Some("b"), 8, 8, &pixels);
        assert_eq!(b, a, "tombstoned slot index is recycled");
        assert!(atlas.region(b).is_some());
        assert_eq!(atlas.id_for("b"), Some(b));
        assert_eq!(atlas.id_for("a"), None, "old name not resurrected");
    }

    #[test]
    fn compact_preserves_ids_and_reclaims_shelf_footprint() {
        // 256x256 sprites have a 258x258 cell; at the initial 1024 atlas only
        // floor(1024/258) = 3 shelves fit, so a 4th forces a grow to 2048.
        let mut atlas = SpriteAtlas::new();
        let big = 256u32;
        let pixels = vec![0u8; (big * big * 4) as usize];
        let ids: Vec<_> = (0..4).map(|_| atlas.insert(None, big, big, &pixels)).collect();
        let footprint_before = atlas.next_shelf_y;

        // Remove two of the four, leaving shelf fragmentation (the shelf cursor
        // doesn't rewind on remove).
        assert!(atlas.remove(ids[0]));
        assert!(atlas.remove(ids[2]));
        let live: Vec<_> = [ids[1], ids[3]].into_iter().collect();

        atlas.compact();

        // All live ids still resolve (index stability — the whole point), with
        // valid, disjoint regions.
        let regions: Vec<_> = live.iter().map(|id| atlas.region(*id).unwrap()).collect();
        for r in &regions {
            assert_eq!(r.w, big);
            assert_eq!(r.h, big);
        }
        for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                let a = regions[i];
                let b = regions[j];
                let separate = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y
                    || b.y + b.h <= a.y;
                assert!(separate, "compacted regions overlap: {a:?} vs {b:?}");
            }
        }
        // The two remaining sprites now pack onto a single shelf, so the shelf
        // footprint (next_shelf_y) shrank dramatically — this is the reclamation.
        assert!(
            atlas.next_shelf_y < footprint_before,
            "compaction should reclaim shelf footprint: {} -> {}",
            footprint_before,
            atlas.next_shelf_y
        );
        assert_eq!(
            atlas.next_shelf_y, 258,
            "two 258-tall cells pack onto one shelf after compaction"
        );
    }

    #[test]
    fn compact_is_noop_without_tombstones() {
        let mut atlas = SpriteAtlas::new();
        let pixels = vec![0u8; 8 * 8 * 4];
        let id = atlas.insert(Some("x"), 8, 8, &pixels);
        let region_before = atlas.region(id).unwrap();
        // No tombstones → compaction does nothing (and must not corrupt state).
        atlas.compact();
        assert_eq!(
            atlas.region(id).unwrap(),
            region_before,
            "compaction with no tombstones leaves regions untouched"
        );
    }

    #[test]
    fn build_pixel_buffer_skips_tombstones() {
        // A removed sprite must not bleed its stale pixels into the rebuilt
        // atlas texture: its old content area should be zeroed after re-upload.
        let mut atlas = SpriteAtlas::new();
        let red = [255u8, 0, 0, 255];
        let red_pixels: Vec<u8> = red.repeat(8 * 8);
        let id = atlas.insert(Some("red"), 8, 8, &red_pixels);
        let region = atlas.region(id).unwrap();

        assert!(atlas.remove(id));
        let buf = atlas.build_pixel_buffer();
        let stride = (atlas.width() * 4) as usize;
        let read = |x: u32, y: u32| -> [u8; 4] {
            let off = (y as usize) * stride + (x as usize) * 4;
            [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]
        };
        // The content area where red lived is now transparent (zeroed).
        assert_eq!(read(region.x, region.y), [0, 0, 0, 0]);
        assert_eq!(
            read(region.x + region.w - 1, region.y + region.h - 1),
            [0, 0, 0, 0]
        );
    }
}
