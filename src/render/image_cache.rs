//! Decoded-image cache keyed by path/key, backed by the sprite atlas.
//!
//! Teardown's `UiImage(path)` / `UiImageBox` / `UiGetImageSize` load an encoded
//! image (PNG/JPEG) by path and draw it as a UI quad. The sprite atlas only
//! accepts raw RGBA8, so this module owns the decode + a `key -> sprite`
//! mapping. Decoding is done once per key; repeat loads return the cached
//! handle. The [`UiRenderer`](crate::UiRenderer) owns one of these and exposes
//! the public `load_image_*` / `image_size` / `has_image` / `unload_image` API.
//!
//! Note: [`SpriteAtlas`](super::SpriteAtlas) reclaims a removed sprite's slot
//! immediately (the pixel buffer is freed and the slot is recycled by the next
//! load), and shelf *fragmentation* is reclaimed by
//! [`UiRenderer::compact_atlas`](crate::UiRenderer::compact_atlas). So
//! `unload_image` no longer leaks atlas pixels — a long-running app that churns
//! one-off images should call `compact_atlas` periodically to keep the texture
//! from climbing toward its 4096² cap.

use std::collections::HashMap;

use super::SpriteId;

/// Metadata for a loaded image: its atlas handle and source pixel dimensions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ImageEntry {
    /// Atlas handle the decoded image was uploaded under.
    pub sprite: SpriteId,
    /// Source image width in pixels.
    pub width: u32,
    /// Source image height in pixels.
    pub height: u32,
}

/// Error from loading or decoding a UI image.
#[derive(Debug)]
pub enum ImageError {
    /// Reading the file from disk failed.
    Io(std::io::Error),
    /// Decoding the encoded bytes failed (unsupported format, corrupt data, …).
    Decode(image::ImageError),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Io(e) => write!(f, "image i/o error: {e}"),
            ImageError::Decode(e) => write!(f, "image decode error: {e}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImageError::Io(e) => Some(e),
            ImageError::Decode(e) => Some(e),
        }
    }
}

/// Maps image keys (file paths or explicit byte-load keys) to atlas sprites, so
/// repeat loads are free and callers can query image size without re-decoding.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<String, ImageEntry>,
}

impl ImageCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached entry for `key`, or `None` if it hasn't been loaded.
    pub fn get(&self, key: &str) -> Option<ImageEntry> {
        self.entries.get(key).copied()
    }

    /// Whether an entry is cached for `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Record the atlas entry for `key` (overwrites any existing mapping).
    pub fn insert(&mut self, key: &str, entry: ImageEntry) {
        self.entries.insert(key.to_string(), entry);
    }

    /// Drop the cache entry for `key`. Returns the removed entry, if any. This is
    /// a pure map remove — it does not touch the atlas; the caller (e.g.
    /// [`UiRenderer::unload_image`](crate::UiRenderer::unload_image)) is
    /// responsible for freeing the matching sprite slot.
    pub fn remove(&mut self, key: &str) -> Option<ImageEntry> {
        self.entries.remove(key)
    }
}

/// Decode encoded image bytes (PNG/JPEG/…) into `(width, height, rgba8)`.
pub fn decode_rgba8(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), image::ImageError> {
    let img = image::load_from_memory(bytes)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((w, h, rgba.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a tiny RGBA image to PNG in memory (test helper).
    fn synth_png(w: u32, h: u32) -> Vec<u8> {
        use image::{ImageFormat, Rgba, RgbaImage};
        let mut img = RgbaImage::new(w, h);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decode_roundtrips_a_synthesized_png() {
        let bytes = synth_png(3, 2);
        let (w, h, rgba) = decode_rgba8(&bytes).unwrap();
        assert_eq!((w, h), (3, 2));
        assert_eq!(rgba.len(), (3 * 2 * 4) as usize);
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_rgba8(&[0u8, 1, 2, 3, 4, 5]).is_err());
    }

    #[test]
    fn cache_insert_get_remove() {
        let mut cache = ImageCache::new();
        assert!(!cache.contains("a"));
        cache.insert(
            "a",
            ImageEntry {
                sprite: 7,
                width: 4,
                height: 8,
            },
        );
        assert!(cache.contains("a"));
        assert_eq!(cache.get("a").unwrap().sprite, 7);
        assert_eq!(cache.get("a").unwrap().width, 4);
        let removed = cache.remove("a").unwrap();
        assert_eq!(removed.height, 8);
        assert!(!cache.contains("a"));
    }

    /// Mirrors `UiRenderer::load_image_rgba8`: insert raw RGBA8 into the atlas
    /// *and* record the cache entry, then verify the decoded-image cache API
    /// (`has` via `contains` / `get` / `remove`) sees the key. This is the
    /// contract that distinguishes `load_image_rgba8` from `load_sprite_rgba8`
    /// (which registers only the atlas name and bypasses the cache).
    #[test]
    fn load_image_rgba8_registers_in_cache() {
        use crate::SpriteAtlas;
        let mut atlas = SpriteAtlas::new();
        let mut cache = ImageCache::new();

        // "Load" a 2x2 raw RGBA8 image under a key.
        let key = "notif_icon";
        let (w, h) = (2u32, 2u32);
        let rgba = vec![255u8; (w * h * 4) as usize];
        let sprite = atlas.insert(Some(key), w, h, &rgba);
        cache.insert(
            key,
            ImageEntry {
                sprite,
                width: w,
                height: h,
            },
        );

        // The cache sees it — the whole point of #1.
        assert!(cache.contains(key));
        assert_eq!(cache.get(key).unwrap().sprite, sprite);
        assert_eq!(cache.get(key).unwrap().width, w);
        assert_eq!(cache.get(key).unwrap().height, h);

        // Repeating the load is a cache fast-path (returns the same sprite).
        let again = cache.get(key).unwrap().sprite;
        assert_eq!(again, sprite);

        // Unloading drops the cache entry.
        cache.remove(key);
        assert!(!cache.contains(key));
    }
}
