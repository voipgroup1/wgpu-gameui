//! GPU rendering for `DrawList`.
//!
//! See [`UiRenderer`] for the entry point. Internally this module owns:
//! * a colored-quad pipeline (consumes [`crate::Vertex`] directly)
//! * a textured-quad pipeline (icons and nine-slice tessellation)
//! * a dynamic [`SpriteAtlas`]
//! * a nine-slice metadata table
//! * a [`crate::TextRenderer`] for glyphon text
//!
//! `UiRenderer::render` consumes a `DrawList` and emits four sub-render-passes
//! in this order: nine-slices → colored quads → icons → text. This matches the
//! reference implementation in citybuilder.

mod atlas;
mod ui_renderer;

pub use atlas::{AtlasRegion, SpriteAtlas, SpriteId};
pub use ui_renderer::{NineSliceMeta, UiRenderer};

pub use crate::widgets::NineSliceId;
