//! GPU rendering for `DrawList`.
//!
//! See [`UiRenderer`] for the entry point. Internally this module owns:
//! * a colored-quad pipeline (consumes [`crate::Vertex`] directly)
//! * a textured-quad pipeline (icons and nine-slice tessellation)
//! * a dynamic [`SpriteAtlas`]
//! * a nine-slice metadata table
//! * a [`crate::TextRenderer`] for MSDF text (cosmic-text shaping + fdsm glyph atlas)
//!
//! `UiRenderer::render` consumes a `DrawList` and emits four sub-render-passes
//! in this order: nine-slices → colored quads → icons → text. This matches the
//! reference implementation in citybuilder.

mod atlas;
mod glyph_msdf;
mod image_cache;
mod msdf_atlas;
mod ui_renderer;

pub use glyph_msdf::{generate_glyph_msdf, GlyphMetrics, GlyphMsdf};
pub use msdf_atlas::{GlyphTile, MsdfGlyphAtlas, DEFAULT_PX_RANGE, DEFAULT_REF_PX};

pub use atlas::{AtlasRegion, SpriteAtlas, SpriteId};
pub use image_cache::{ImageCache, ImageEntry, ImageError};
pub use ui_renderer::{NineSliceMeta, UiRenderer};
pub(crate) use ui_renderer::ortho_matrix;

pub use crate::widgets::NineSliceId;
