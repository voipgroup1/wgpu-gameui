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
mod blur;
mod glyph_msdf;
mod image_cache;
mod msdf_atlas;
#[cfg(feature = "phosphor-icons")]
mod phosphor;
mod ui_renderer;

pub use glyph_msdf::{GlyphMetrics, GlyphMsdf, generate_glyph_msdf};
pub use msdf_atlas::{DEFAULT_PX_RANGE, DEFAULT_REF_PX, GlyphTile, MsdfGlyphAtlas};

#[cfg(feature = "phosphor-icons")]
pub use phosphor::PhosphorIcon;
#[cfg(feature = "phosphor-icons")]
pub(crate) use phosphor::{PHOSPHOR_FONT_ID, phosphor_font_data, phosphor_glyph_id};

pub use atlas::{AtlasRegion, SpriteAtlas, SpriteId};
pub use blur::{Backdrop, BlurParams};
pub use image_cache::{ImageCache, ImageEntry, ImageError};
pub(crate) use ui_renderer::{ortho_matrix,ortho_matrix2};
pub use ui_renderer::{NineSliceMeta, UiRenderer};

pub use crate::widgets::NineSliceId;
