//! MSDF text rendering.
//!
//! Shaping and layout still go through **cosmic-text** (via the `glyphon`
//! re-export), exactly as before — so wrapping and measurement are unchanged. What
//! changed is the *rasterize + atlas + GPU draw* stage: instead of glyphon's
//! grayscale-alpha glyph cache, each glyph is rendered from a **multi-channel
//! signed distance field** ([`crate::render::MsdfGlyphAtlas`]). This gives crisp
//! fill at any size and is the foundation for outline/shadow/glow effects
//! (Teardown `UiTextOutline`/`UiTextShadow` parity) added in later phases.
//!
//! [`TextRenderer`] is self-contained: it owns the MSDF atlas, a linear-sampled
//! `Rgba8Unorm` (NOT sRGB — the texels are distances, not colors) GPU texture, the
//! MSDF pipeline, and its own ortho uniform. [`TextRenderer::render`] shapes each
//! [`TextBlock`], emits one quad per glyph, lazily generates any unseen glyph into
//! the atlas, uploads the atlas if it changed, and draws — all in one call.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};

use crate::layout::Rect;
use crate::render::{ortho_matrix, GlyphTile, MsdfGlyphAtlas};

use glyphon::cosmic_text::{fontdb, Align as CosmicAlign, Wrap};
use glyphon::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, Weight};

const MSDF_SHADER: &str = include_str!("render/ui_msdf.wgsl");

/// Shared handle to a glyphon `FontSystem`.
///
/// Both `TextRenderer` and `TextMeasurer` hold the same handle so measured text widths
/// (used for layout) match rendered glyphs (used for output) — including any custom
/// fonts loaded into the system later.
pub type FontSystemHandle = Arc<Mutex<FontSystem>>;

/// Create a new shared `FontSystem` handle.
///
/// `FontSystem::new()` loads the host's system fonts (used for broad script /
/// emoji fallback). With the default `bundled-font` feature on, the bundled Noto
/// Sans faces are also embedded and registered as the default sans-serif (see
/// [`register_bundled_fonts`]), so unstyled text renders identically on every
/// machine rather than depending on which system font happens to be installed.
pub fn shared_font_system() -> FontSystemHandle {
    let handle = Arc::new(Mutex::new(FontSystem::new()));
    register_bundled_fonts(&handle);
    handle
}

/// Embed the bundled Noto Sans faces (regular/bold/italic/bold-italic) into `fs`
/// and register the family as the default sans-serif, so `Family::SansSerif` —
/// and any [`TextBlock`] without an explicit font — resolves to it
/// deterministically on every machine instead of an OS-dependent system font.
/// The bold/italic faces share the family name, so [`TextBlock::bold`] /
/// [`TextBlock::italic`] select them on the default font too.
///
/// Returns the family [`FontHandle`] (also usable directly via
/// [`TextBlock::with_font`]). With the `bundled-font` feature **disabled** this is
/// a no-op that returns `None` and the default stays the system sans-serif.
/// [`shared_font_system`] calls this for you; call it yourself only when you
/// construct a `FontSystem` by other means.
pub fn register_bundled_fonts(fs: &FontSystemHandle) -> Option<FontHandle> {
    #[cfg(feature = "bundled-font")]
    {
        // Load all four faces; they share the "Noto Sans" family so weight/style
        // selection picks the right one. Only the regular handle is returned.
        let regular = load_font_bytes(fs, notosans::REGULAR_TTF).ok()?;
        let _ = load_font_bytes(fs, notosans::BOLD_TTF);
        let _ = load_font_bytes(fs, notosans::ITALIC_TTF);
        let _ = load_font_bytes(fs, notosans::BOLD_ITALIC_TTF);
        {
            let mut guard = fs.lock().expect("FontSystem poisoned");
            guard
                .db_mut()
                .set_sans_serif_family(regular.family().to_string());
        }
        Some(regular)
    }
    #[cfg(not(feature = "bundled-font"))]
    {
        let _ = fs;
        None
    }
}

/// Handle to a font loaded into the shared [`FontSystem`], identified by its
/// family name.
///
/// cosmic-text's shaping selects fonts by family name only (`Family::Name`), so
/// a handle is just the family string. Obtain one from [`load_font_file`] /
/// [`load_font_bytes`] and pass it to [`TextBlock::with_font`] to shape a block
/// in that font. If two faces share a family name, the most recently loaded one
/// wins.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontHandle(pub String);

impl FontHandle {
    /// The font's family name (the cosmic-text selector).
    pub fn family(&self) -> &str {
        &self.0
    }
}

/// Horizontal alignment of multi-line text within its `max_width` layout box.
///
/// Alignment is relative to [`TextBlock::max_width`]; `Center`/`Right` only
/// produce a visible shift when `max_width` is wider than the longest line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlign {
    /// Lines start at the left edge of the box (default).
    #[default]
    Left,
    /// Lines are centered within `max_width`.
    Center,
    /// Lines are flushed to the right edge of `max_width`.
    Right,
}

/// How a [`TextBlock`] breaks lines when its content is wider than
/// [`max_width`](TextBlock::max_width).
///
/// The default, [`WordOrGlyph`](WrapMode::WordOrGlyph), is exactly the implicit
/// behaviour every block had before this knob existed (cosmic-text's `Buffer`
/// default), so leaving it unset changes nothing. Use [`None`](WrapMode::None)
/// for single-line fields that should overflow (and be clipped) rather than
/// wrap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WrapMode {
    /// Never wrap; the text stays on one line and overflows `max_width`
    /// (combine with [`TextBlock::with_clip`] to hide the overflow).
    None,
    /// Break between words; a single word too long to fit still overflows.
    Word,
    /// Break anywhere between glyphs.
    Glyph,
    /// Break between words, falling back to glyph breaks for a word too long to
    /// fit on a line by itself. Matches the pre-existing implicit behaviour.
    #[default]
    WordOrGlyph,
}

impl From<WrapMode> for Wrap {
    fn from(mode: WrapMode) -> Self {
        match mode {
            WrapMode::None => Wrap::None,
            WrapMode::Word => Wrap::Word,
            WrapMode::Glyph => Wrap::Glyph,
            WrapMode::WordOrGlyph => Wrap::WordOrGlyph,
        }
    }
}

/// Load a font from a TTF/OTF file into the shared `FontSystem`, returning a
/// [`FontHandle`] that selects it for [`TextBlock::with_font`].
///
/// After loading, drop any cached measurements ([`TextMeasurer::clear_cache`])
/// if the same family name replaced an earlier face.
pub fn load_font_file(
    fs: &FontSystemHandle,
    path: impl AsRef<Path>,
) -> std::io::Result<FontHandle> {
    let bytes = std::fs::read(path)?;
    load_font_bytes(fs, &bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Load a font from in-memory TTF/OTF bytes into the shared `FontSystem`.
///
/// Returns the family-name [`FontHandle`], or an error string if the bytes do
/// not parse into a face that exposes a family name. The family name is read
/// from the same `fontdb` that cosmic-text shapes against, so the returned
/// handle is guaranteed to resolve.
pub fn load_font_bytes(fs: &FontSystemHandle, bytes: &[u8]) -> Result<FontHandle, String> {
    let mut guard = fs.lock().expect("FontSystem poisoned");
    let db = guard.db_mut();
    let before = db.len();
    db.load_font_data(bytes.to_vec());
    // `load_font_data` appends one face per font in the data (one for a plain
    // TTF/OTF). Take the first newly added face's primary family name.
    db.faces()
        .nth(before)
        .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
        .map(FontHandle)
        .ok_or_else(|| "loaded font exposes no family name".to_string())
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct MsdfVertex {
    position: [f32; 2],
    uv: [f32; 2],
    fill: [f32; 4],
    clip: [f32; 4],
    clip_enabled: f32,
    /// Distance-ramp width of the field in atlas texels (constant per atlas).
    px_range: f32,
    /// Outline/glow color composited under the fill (a == 0 disables).
    outline: [f32; 4],
    /// Outline width in screen px (glyph grown outward by this much).
    outline_width: f32,
    /// Extra AA spread in screen px (soft shadows / glow); 0 = crisp.
    softness: f32,
}

const MSDF_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32,
    5 => Float32,
    6 => Float32x4,
    7 => Float32,
    8 => Float32,
];

pub struct TextRenderer {
    font_system: FontSystemHandle,

    // MSDF glyph atlas (CPU source of truth) + its GPU mirror.
    atlas: MsdfGlyphAtlas,
    texture: wgpu::Texture,
    sampler: wgpu::Sampler,
    atlas_bgl: wgpu::BindGroupLayout,
    atlas_bind_group: wgpu::BindGroup,
    current_atlas_size: u32,

    // Ortho projection (owned, sized from `resize`).
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    pipeline: wgpu::RenderPipeline,

    vbo: wgpu::Buffer,
    vbo_capacity: u64,
    /// Bytes already used in `vbo` this frame. Each `render` pass writes at this
    /// offset and advances it, so multiple text passes within one submit (e.g.
    /// base layer + tooltip layer) occupy disjoint regions instead of all
    /// aliasing offset 0 and reading the last-written data at draw time. Reset
    /// to 0 by [`begin_frame`](Self::begin_frame) each frame.
    vbo_offset: u64,

    /// Stable per-font keys for the atlas, assigned on first sighting. Decouples
    /// the atlas from cosmic-text's `fontdb::ID`.
    font_keys: HashMap<fontdb::ID, u64>,
    next_font_key: u64,

    /// Cross-frame shaped-layout cache. Keyed by everything that affects layout
    /// except position/color/clip/effects, so a block whose content and metrics
    /// are unchanged reuses its glyph layout instead of re-shaping every frame
    /// (the dominant cost in large text-heavy frames). Inner map is keyed by the
    /// content string so hits borrow `&str` with no allocation.
    shape_cache: HashMap<ShapeKey, HashMap<String, CachedShape>>,
    /// Monotonic frame counter stamped onto cache entries for working-set eviction.
    shape_frame: u64,

    width: u32,
    height: u32,
}

impl TextRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = shared_font_system();
        Self::with_font_system(device, queue, format, font_system)
    }

    /// Construct a `TextRenderer` reusing an existing shared `FontSystem`.
    pub fn with_font_system(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: FontSystemHandle,
    ) -> Self {
        let atlas = MsdfGlyphAtlas::new();

        // Uniform (group 0): ortho projection, matching the main UI pipelines.
        let uniform_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("msdf text uniform"),
                contents: bytemuck::cast_slice(&[ortho_matrix(1.0, 1.0)]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("msdf uniform bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("msdf uniform bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Atlas texture (group 1): linear filtering, linear (non-sRGB) format.
        let (texture, sampler, atlas_bgl, atlas_bind_group) =
            create_msdf_texture(device, atlas.width(), atlas.height());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("msdf text shader"),
            source: wgpu::ShaderSource::Wgsl(MSDF_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("msdf text pipeline layout"),
            bind_group_layouts: &[&uniform_bgl, &atlas_bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("msdf text pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_msdf"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MsdfVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MSDF_VERTEX_ATTRIBS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_msdf"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vbo_capacity = (4096 * std::mem::size_of::<MsdfVertex>()) as u64;
        let vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("msdf text vbo"),
            size: vbo_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            font_system,
            atlas,
            texture,
            sampler,
            atlas_bgl,
            atlas_bind_group,
            current_atlas_size: 0, // forces first upload
            uniform_buffer,
            uniform_bind_group,
            pipeline,
            vbo,
            vbo_capacity,
            vbo_offset: 0,
            font_keys: HashMap::new(),
            next_font_key: 0,
            shape_cache: HashMap::new(),
            shape_frame: 0,
            width: 1,
            height: 1,
        }
    }

    /// Get a clone of the shared font system handle.
    ///
    /// Use this to construct a `DrawList` / `TextMeasurer` that shares font state with
    /// this renderer.
    pub fn font_system_handle(&self) -> FontSystemHandle {
        Arc::clone(&self.font_system)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    /// Reset the per-frame vertex-buffer cursor. Call once at the start of each
    /// frame (before any [`render`](Self::render) pass) so the bump offset that
    /// keeps multiple text passes from aliasing starts fresh.
    pub fn begin_frame(&mut self) {
        self.vbo_offset = 0;
    }

    /// Drop the cross-frame shaped-text cache, forcing every block to re-shape on
    /// the next frame.
    ///
    /// The cache assumes the shared `FontSystem`'s font set is stable: a layout
    /// shaped once is reused for any later block with the same content + metrics +
    /// font + alignment. If you load a new font into the shared `FontSystem` after
    /// text has been rendered (e.g. [`load_font_bytes`]), call this so blocks that
    /// reference the new font re-shape against it — mirrors
    /// [`TextMeasurer::clear_cache`].
    pub fn clear_shape_cache(&mut self) {
        self.shape_cache.clear();
    }

    /// Measure text using cosmic-text's shaping/layout path without touching GPU state.
    pub fn measure(&mut self, text: &str, font_size: f32) -> (f32, f32) {
        let mut fs = self.font_system.lock().expect("FontSystem poisoned");
        measure_with_font_system(
            &mut fs,
            text,
            font_size,
            None,
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        )
    }

    /// Pre-generate the printable-ASCII glyph set into the atlas so the first
    /// frame that displays them doesn't hitch. Call once after construction.
    pub fn prewarm_ascii(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let ascii: String = (0x20u8..=0x7e).map(|c| c as char).collect();
        // Clone the handle so the guard borrows a local, not `self` (frees `self`
        // for `self.font_key`/`self.atlas`).
        let fs_handle = Arc::clone(&self.font_system);
        let mut fs = fs_handle.lock().expect("FontSystem poisoned");
        let mut buffer = Buffer::new(&mut fs, Metrics::new(self.atlas.ref_px(), self.atlas.ref_px()));
        buffer.set_text(
            &mut fs,
            &ascii,
            Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut fs, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let font_key = self.font_key(glyph.font_id);
                if let Some(font) = fs.get_font(glyph.font_id) {
                    self.atlas.glyph(font_key, glyph.glyph_id, font.data());
                }
            }
        }
        drop(fs);
        self.upload_atlas(device, queue);
    }

    fn font_key(&mut self, id: fontdb::ID) -> u64 {
        resolve_font_key(&mut self.font_keys, &mut self.next_font_key, id)
    }

    /// Build glyph quads for all text blocks, generating any unseen glyphs into the
    /// atlas. Returns the vertex list (6 verts/glyph, triangle list).
    fn build_vertices(&mut self, texts: &[TextBlock]) -> Vec<MsdfVertex> {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_shape").entered();

        let px_range = self.atlas.px_range();
        let ref_px = self.atlas.ref_px();
        self.shape_frame = self.shape_frame.wrapping_add(1);
        let frame = self.shape_frame;

        // First pass: resolve every block to a list of `GlyphPlacement`s, shaping
        // through cosmic-text only on a *cache miss*. The shaped relative layout
        // is keyed by everything that affects it (content + metrics + font +
        // align + ellipsize) and reused across frames, so unchanged labels skip
        // the expensive re-shape entirely. We resolve uv only *after* this pass,
        // because glyph generation can grow the atlas (changing the size uv
        // divides by) — pixel regions stay valid (top-left origin) but uv must use
        // the final size.
        let mut placements: Vec<GlyphPlacement> = Vec::new();

        for block in texts {
            if block.content.is_empty() {
                continue;
            }

            let key: ShapeKey = (
                block.font_size.to_bits(),
                block.line_height.to_bits(),
                block.max_width.to_bits(),
                family_hash(block.font.as_ref()),
                block.align,
                block.ellipsize,
                block.weight.0,
                style_disc(block.style),
                block.wrap,
            );

            // Fast path: a cached layout for this exact key + content. No
            // FontSystem lock, no shaping — every cached glyph is already in the
            // atlas (it never evicts), so `atlas.glyph` re-looks it up without
            // touching font data. We only stamp the frame for working-set eviction.
            if let Some(entry) = self
                .shape_cache
                .get_mut(&key)
                .and_then(|inner| inner.get_mut(&block.content))
            {
                entry.last_used = frame;
                append_placements(
                    &mut self.atlas,
                    &mut self.font_keys,
                    &mut self.next_font_key,
                    block,
                    &block.spans,
                    &entry.glyphs,
                    &mut placements,
                );
                continue;
            }

            // Miss: shape now, recording the relative layout for future frames.
            let fs_handle = Arc::clone(&self.font_system);
            let mut fs = fs_handle.lock().expect("FontSystem poisoned");

            let family = block
                .font
                .as_ref()
                .map(|h| Family::Name(h.family()))
                .unwrap_or(Family::SansSerif);

            // In ellipsis mode the block is a single line truncated to `max_width`
            // with a trailing '…'; otherwise it wraps at `max_width` as before.
            let truncated;
            let content: &str = if block.ellipsize {
                truncated = ellipsize_to_width(
                    &mut fs,
                    &block.content,
                    block.font_size,
                    block.line_height,
                    block.max_width,
                    family,
                    block.weight,
                    block.style,
                );
                &truncated
            } else {
                &block.content
            };

            let mut buffer = Buffer::new(&mut fs, Metrics::new(block.font_size, block.line_height));
            if block.ellipsize {
                buffer.set_wrap(&mut fs, Wrap::None);
                buffer.set_size(&mut fs, None, None);
            } else {
                buffer.set_wrap(&mut fs, block.wrap.into());
                buffer.set_size(&mut fs, Some(block.max_width), None);
            }
            buffer.set_text(
                &mut fs,
                content,
                Attrs::new()
                    .family(family)
                    .weight(block.weight)
                    .style(block.style)
                    .color(block.color),
                Shaping::Advanced,
            );
            // Horizontal alignment is set per buffer line before layout; Left is
            // cosmic-text's default so we only override for Center/Right.
            if let Some(align) = cosmic_align(block.align) {
                for line in buffer.lines.iter_mut() {
                    line.set_align(Some(align));
                }
            }
            buffer.shape_until_scroll(&mut fs, false);

            // Collect the relative layout. Whitespace / outline-less glyphs yield
            // no tile (atlas returns `None`) and are skipped — so every stored
            // glyph is guaranteed present in the atlas on later frames.
            let mut shaped: Vec<ShapedGlyph> = Vec::new();
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    let font_size = glyph.font_size;
                    let rel_x = glyph.x + font_size * glyph.x_offset;
                    let rel_y = run.line_y + glyph.y - font_size * glyph.y_offset;

                    let font_key =
                        resolve_font_key(&mut self.font_keys, &mut self.next_font_key, glyph.font_id);
                    let Some(font) = fs.get_font(glyph.font_id) else {
                        continue;
                    };
                    if self
                        .atlas
                        .glyph(font_key, glyph.glyph_id, font.data())
                        .is_none()
                    {
                        continue; // whitespace / outline-less
                    }
                    shaped.push(ShapedGlyph {
                        font_id: glyph.font_id,
                        glyph_id: glyph.glyph_id,
                        rel_x,
                        rel_y,
                        font_size,
                        byte_start: glyph.start as u32,
                    });
                }
            }
            drop(fs);

            let entry = self
                .shape_cache
                .entry(key)
                .or_default()
                .entry(block.content.clone())
                .or_insert(CachedShape {
                    glyphs: Vec::new(),
                    last_used: frame,
                });
            entry.glyphs = shaped;
            entry.last_used = frame;
            append_placements(
                &mut self.atlas,
                &mut self.font_keys,
                &mut self.next_font_key,
                block,
                &block.spans,
                &entry.glyphs,
                &mut placements,
            );
        }

        // Evict layouts that fell out of the visible working set once the cache
        // grows past its cap, so long-running UIs that cycle through many
        // distinct strings don't grow it without bound. Entries touched this
        // frame always survive.
        let total: usize = self.shape_cache.values().map(|inner| inner.len()).sum();
        if total > SHAPE_CACHE_MAX {
            for inner in self.shape_cache.values_mut() {
                inner.retain(|_, e| e.last_used == frame);
            }
            self.shape_cache.retain(|_, inner| !inner.is_empty());
        }

        // Second pass: resolve uv against the final atlas size and emit quads in
        // back-to-front sweeps so every glyph's shadow/glow sits behind ALL fills:
        //   1. shadows  2. glow  3. fill (+ outline)
        let (aw, ah) = (self.atlas.width(), self.atlas.height());
        let mut verts: Vec<MsdfVertex> = Vec::with_capacity(placements.len() * 6);

        for p in &placements {
            if let Some((color, offset, softness)) = p.shadow {
                // A shadow is a fill with widened AA; cap the blur to the field reach.
                let safe = field_reach(p.font_size, px_range, ref_px);
                push_glyph_quad(
                    &mut verts,
                    p,
                    aw,
                    ah,
                    px_range,
                    &QuadStyle {
                        fill: color,
                        outline: [0.0; 4],
                        outline_width: 0.0,
                        softness: softness.min(safe),
                        offset,
                    },
                );
            }
        }
        for p in &placements {
            if let Some((color, radius)) = p.glow {
                // The glow is a grown, soft, fill-less halo. Cap its band (width +
                // softness) to the field's valid reach so it follows the glyph instead
                // of filling the tile rectangle (graceful degradation at small sizes).
                let safe = field_reach(p.font_size, px_range, ref_px);
                let radius = radius.min(safe / 1.5);
                let softness = (radius * 0.5).max(0.5).min((safe - radius).max(0.0));
                push_glyph_quad(
                    &mut verts,
                    p,
                    aw,
                    ah,
                    px_range,
                    &QuadStyle {
                        fill: [0.0; 4],
                        outline: color,
                        outline_width: radius,
                        softness,
                        offset: [0.0, 0.0],
                    },
                );
            }
        }
        for p in &placements {
            let (outline, outline_width) = p.outline.unwrap_or(([0.0; 4], 0.0));
            // Cap the outline thickness to the field reach to avoid tile-fill artifacts.
            let safe = field_reach(p.font_size, px_range, ref_px);
            push_glyph_quad(
                &mut verts,
                p,
                aw,
                ah,
                px_range,
                &QuadStyle {
                    fill: p.fill,
                    outline,
                    outline_width: outline_width.min(safe),
                    softness: 0.0,
                    offset: [0.0, 0.0],
                },
            );
        }
        verts
    }

    /// (Re)upload the atlas pixels to the GPU if the CPU atlas changed. Recreates
    /// the texture + bind group when the atlas has grown.
    fn upload_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_atlas_upload").entered();
        if self.atlas.width() != self.current_atlas_size {
            let (texture, sampler, _bgl, bind_group) =
                create_msdf_texture_with_bgl(device, &self.atlas_bgl, self.atlas.width(), self.atlas.height());
            self.texture = texture;
            self.sampler = sampler;
            self.atlas_bind_group = bind_group;
            self.current_atlas_size = self.atlas.width();
            let _ = self.atlas.take_dirty();
            self.write_atlas_pixels(queue);
        } else if self.atlas.take_dirty() {
            self.write_atlas_pixels(queue);
        }
    }

    fn write_atlas_pixels(&self, queue: &wgpu::Queue) {
        let pixels = self.atlas.build_pixel_buffer();
        let w = self.atlas.width();
        let h = self.atlas.height();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Prepare and render text in a single call.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        texts: &[TextBlock],
    ) {
        if texts.is_empty() {
            return;
        }

        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_render").entered();

        // Keep the ortho uniform in sync with the current viewport.
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[ortho_matrix(self.width as f32, self.height as f32)]),
        );

        let verts = self.build_vertices(texts);
        // Glyph generation may have dirtied / grown the atlas — upload before drawing.
        self.upload_atlas(device, queue);

        if verts.is_empty() {
            return;
        }
        // Bump-allocate this pass's slice so it doesn't alias earlier passes in
        // the same submit (which would all read the last write at draw time).
        let vbytes = (verts.len() * std::mem::size_of::<MsdfVertex>()) as u64;
        let offset = self.ensure_vbo_capacity(device, vbytes);
        queue.write_buffer(&self.vbo, offset, bytemuck::cast_slice(&verts));
        self.vbo_offset = offset + vbytes;

        #[cfg(feature = "tracy")]
        let _pass_span = tracing::info_span!("gameui_text_pass").entered();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("msdf text pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vbo.slice(offset..));
        pass.draw(0..verts.len() as u32, 0..1);
    }

    /// Ensure `vbo` can hold `bytes` starting at the current frame offset, and
    /// return the byte offset to write/draw this pass at. Grows by allocating a
    /// fresh buffer when needed; earlier passes keep referencing the old buffer
    /// (held alive by the encoder), so their data stays valid.
    fn ensure_vbo_capacity(&mut self, device: &wgpu::Device, bytes: u64) -> u64 {
        let offset = self.vbo_offset;
        let needed = offset + bytes;
        if needed > self.vbo_capacity {
            self.vbo_capacity = needed.next_power_of_two();
            self.vbo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("msdf text vbo"),
                size: self.vbo_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        offset
    }
}

/// Convert a cosmic-text `Color` (sRGB u8 RGBA) to a normalized `[f32; 4]`. Matches
/// how the colored-quad pipeline treats `Vertex` colors (pass-through), so MSDF text
/// fill matches solid UI colors.
fn color_to_rgba(c: Color) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// One shaped glyph, **relative to the block origin**. The block's `x`/`y`,
/// color, clip and effects are re-applied per frame at emit time, so this layout
/// is identical for any block that shares the shaping key (content + metrics +
/// font + align + ellipsize) and can be cached across frames — skipping the
/// expensive cosmic-text re-shape that dominates large text-heavy frames.
#[derive(Clone)]
struct ShapedGlyph {
    /// cosmic-text font id; resolved to a stable atlas font key + tile each frame.
    font_id: fontdb::ID,
    glyph_id: u16,
    /// Pen x relative to `block.x`: `glyph.x + font_size * glyph.x_offset`.
    rel_x: f32,
    /// Baseline y relative to `block.y`: `run.line_y + glyph.y - font_size * glyph.y_offset`.
    rel_y: f32,
    /// Per-glyph font size (usually equals the block's, but cosmic-text reports
    /// it per glyph, so we preserve it).
    font_size: f32,
    /// Byte offset of this glyph's source character in the shaped content string.
    /// Used by [`append_placements`] to resolve per-span colour overrides from
    /// [`TextBlock::spans`].
    byte_start: u32,
}

/// A cached relative layout for one shaping key, with a frame stamp used to evict
/// entries that fall out of the visible working set.
struct CachedShape {
    glyphs: Vec<ShapedGlyph>,
    last_used: u64,
}

/// Outer cache key: everything that affects shaped layout *except* the content
/// string (which is the inner `HashMap` key, so hits borrow `&str` with no
/// allocation — mirrors [`TextMeasurer`]). Scalars are stored as bit patterns so
/// the key is `Hash + Eq`. The `(u16, u8)` are the font weight and the
/// [`style_disc`] style discriminant, so bold/italic variants cache and re-shape
/// independently of the regular face; the trailing [`WrapMode`] keys the wrap
/// policy so the same content at the same metrics caches separately per wrap.
type ShapeKey = (u32, u32, u32, u64, TextAlign, bool, u16, u8, WrapMode);

/// Stable discriminant for a cosmic-text [`Style`] so it can sit in a `Hash + Eq`
/// cache key: `Normal = 0`, `Italic = 1`, `Oblique = 2`.
fn style_disc(style: Style) -> u8 {
    match style {
        Style::Normal => 0,
        Style::Italic => 1,
        Style::Oblique => 2,
    }
}

/// Cap on cached shaping entries before stale ones are evicted. Text-heavy
/// screens (tables, logs) carry more distinct labels than the measurer's 4096,
/// so this is larger; eviction keeps only the current frame's working set.
const SHAPE_CACHE_MAX: usize = 8192;

/// Map a cosmic-text `fontdb::ID` to a stable atlas font key, assigning a fresh
/// one on first sighting. Free function (rather than a `&mut self` method) so the
/// `font_keys`/`next_font_key` fields can be borrowed disjointly from the rest of
/// the renderer during shaping.
fn resolve_font_key(
    font_keys: &mut HashMap<fontdb::ID, u64>,
    next_font_key: &mut u64,
    id: fontdb::ID,
) -> u64 {
    if let Some(k) = font_keys.get(&id) {
        return *k;
    }
    let k = *next_font_key;
    *next_font_key += 1;
    font_keys.insert(id, k);
    k
}

/// Resolve the fill colour for a single glyph given a set of [`TextSpan`]s.
///
/// Iterates the spans in order, tracking their cumulative byte position in the
/// concatenated text, and returns the `color` of the first span that contains
/// `byte_start`. Returns `None` when `spans` is empty or all spans have
/// `color: None` (caller should fall back to the block's global colour).
pub fn resolve_span_color(byte_start: u32, spans: &[TextSpan]) -> Option<[f32; 4]> {
    let mut offset = 0usize;
    for span in spans {
        let end = offset + span.text.len();
        if (byte_start as usize) < end {
            return span.color;
        }
        offset = end;
    }
    None
}

/// Turn a block's cached relative glyph layout into `GlyphPlacement`s, applying
/// the block's position, color, clip and effects. Shared by the cache-hit and
/// cache-miss paths so both produce identical output. Takes the atlas / font-key
/// fields by `&mut` (not `&mut self`) so the caller can hold a borrow into the
/// shape cache simultaneously. Every glyph here is already in the atlas, so the
/// `atlas.glyph` lookup needs no font data (`&[]`).
///
/// `spans` may be empty (plain mode); in that case all glyphs use the block's
/// global colour. When non-empty, per-glyph colour is resolved via
/// [`resolve_span_color`] and falls back to the block colour for spans with
/// `color: None`.
fn append_placements(
    atlas: &mut MsdfGlyphAtlas,
    font_keys: &mut HashMap<fontdb::ID, u64>,
    next_font_key: &mut u64,
    block: &TextBlock,
    spans: &[TextSpan],
    shaped: &[ShapedGlyph],
    out: &mut Vec<GlyphPlacement>,
) {
    let block_fill = color_to_rgba(block.color);
    let clip = block.clip.map(|c| [c.x, c.y, c.width, c.height]);
    let outline = block.outline.as_ref().map(|o| (color_to_rgba(o.color), o.width_px));
    let shadow = block
        .shadow
        .as_ref()
        .map(|s| (color_to_rgba(s.color), s.offset, s.softness));
    let glow = block.glow.as_ref().map(|g| (color_to_rgba(g.color), g.radius_px));

    for g in shaped {
        let font_key = resolve_font_key(font_keys, next_font_key, g.font_id);
        let Some(tile) = atlas.glyph(font_key, g.glyph_id, &[]) else {
            continue; // present on every later frame; defensive only
        };
        // Per-span colour override: only evaluated when spans are present.
        let fill = if spans.is_empty() {
            block_fill
        } else {
            resolve_span_color(g.byte_start, spans).unwrap_or(block_fill)
        };
        out.push(GlyphPlacement {
            tile,
            pen_x: block.x + g.rel_x,
            baseline_y: block.y + g.rel_y,
            font_size: g.font_size,
            clip,
            fill,
            outline,
            shadow,
            glow,
        });
    }
}

/// A glyph ready to be turned into quads, captured before uv resolution. Carries
/// the block's resolved effect parameters so the emit sweeps can build shadow /
/// glow / fill quads from one record.
struct GlyphPlacement {
    tile: GlyphTile,
    pen_x: f32,
    baseline_y: f32,
    font_size: f32,
    clip: Option<[f32; 4]>,
    fill: [f32; 4],
    /// (color, width_px)
    outline: Option<([f32; 4], f32)>,
    /// (color, offset, softness)
    shadow: Option<([f32; 4], [f32; 2], f32)>,
    /// (color, radius_px)
    glow: Option<([f32; 4], f32)>,
}

/// Maximum effect reach (screen px) a glyph's distance field supports at a given
/// font size, leaving 0.5px AA headroom. Effects (outline width, shadow/glow blur)
/// clamped to this never read past the field's valid range, so they follow the
/// glyph shape instead of filling the tile rectangle.
///
/// The field is valid for `±(px_range/2)` tile texels around the edge; one tile
/// texel maps to `font_size / ref_px` screen px.
fn field_reach(font_size: f32, px_range: f32, ref_px: f32) -> f32 {
    (0.5 * px_range * font_size / ref_px - 0.5).max(0.0)
}

/// Per-quad appearance for one emit sweep.
struct QuadStyle {
    fill: [f32; 4],
    outline: [f32; 4],
    outline_width: f32,
    softness: f32,
    /// Screen-space translation applied to the quad (drop-shadow offset).
    offset: [f32; 2],
}

/// Emit two triangles (6 verts) for one glyph's MSDF tile with the given style.
fn push_glyph_quad(
    out: &mut Vec<MsdfVertex>,
    p: &GlyphPlacement,
    atlas_w: u32,
    atlas_h: u32,
    px_range: f32,
    style: &QuadStyle,
) {
    let m = &p.tile.metrics;
    let font_size = p.font_size;
    let (ox, oy) = (style.offset[0], style.offset[1]);
    // Screen rect (y-down): top_em is above the baseline (positive), bottom_em below.
    let x0 = p.pen_x + m.left_em * font_size + ox;
    let x1 = p.pen_x + m.right_em * font_size + ox;
    let y0 = p.baseline_y - m.top_em * font_size + oy;
    let y1 = p.baseline_y - m.bottom_em * font_size + oy;

    let uv = p.tile.region.uv(atlas_w, atlas_h);
    let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);

    let (clip_rect, clip_on) = match p.clip {
        Some(c) => (c, 1.0),
        None => ([0.0; 4], 0.0),
    };

    let v = |x: f32, y: f32, u: f32, vv: f32| MsdfVertex {
        position: [x, y],
        uv: [u, vv],
        fill: style.fill,
        clip: clip_rect,
        clip_enabled: clip_on,
        px_range,
        outline: style.outline,
        outline_width: style.outline_width,
        softness: style.softness,
    };

    // TL, TR, BR / TL, BR, BL
    out.push(v(x0, y0, u0, v0));
    out.push(v(x1, y0, u1, v0));
    out.push(v(x1, y1, u1, v1));
    out.push(v(x0, y0, u0, v0));
    out.push(v(x1, y1, u1, v1));
    out.push(v(x0, y1, u0, v1));
}

fn create_msdf_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::Sampler, wgpu::BindGroupLayout, wgpu::BindGroup) {
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("msdf atlas bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let (texture, sampler, _bgl, bg) = create_msdf_texture_with_bgl(device, &bgl, width, height);
    (texture, sampler, bgl, bg)
}

fn create_msdf_texture_with_bgl(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::Sampler, (), wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("msdf atlas texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Linear (NOT sRGB): MSDF texels are distances, not colors.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("msdf atlas bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (texture, sampler, (), bg)
}

/// Maximum number of cached text measurements before the cache is flushed.
///
/// Dynamic strings (an FPS counter, a coordinate readout) change every frame and
/// would otherwise grow the cache without bound. When the cache reaches this many
/// entries it is cleared wholesale; static labels are simply re-measured once and
/// re-cached on the next frame. 4096 short entries is a few hundred KB at most.
const MEASURE_CACHE_CAP: usize = 4096;

/// CPU-side glyphon text measurer for layout and widget construction.
///
/// Shaping a string through glyphon to obtain its dimensions is not free, and most
/// UI text is static across frames (labels, button captions). [`TextMeasurer`] caches
/// `(text, font_size, max_width) -> (width, height)` so repeated measurements of the
/// same string are a hash lookup instead of a re-shape.
///
/// The cache assumes the underlying `FontSystem`'s font set does not change after the
/// first measurement (true for the system-font default). If fonts are loaded into the
/// shared `FontSystem` after measuring, call [`TextMeasurer::clear_cache`] to drop
/// stale metrics.
pub struct TextMeasurer {
    font_system: FontSystemHandle,
    /// Keyed by quantized `(font_size_bits, max_width_bits, family_hash, weight,
    /// style_disc)` so the inner `HashMap<String, _>` can be probed with a
    /// borrowed `&str` — no key allocation on a cache hit, only on a miss when we
    /// insert. `family_hash` is 0 for the default font; different fonts/weights/
    /// styles have different advances so all three must be part of the key.
    cache: HashMap<(u32, Option<u32>, u64, u16, u8, WrapMode), HashMap<String, (f32, f32)>>,
    cache_entries: usize,
}

impl TextMeasurer {
    /// Create a measurer with its own private `FontSystem`.
    ///
    /// Prefer [`TextMeasurer::with_font_system`] when a `TextRenderer` already exists,
    /// so measured widths match rendered glyphs.
    pub fn new() -> Self {
        Self {
            font_system: shared_font_system(),
            cache: HashMap::new(),
            cache_entries: 0,
        }
    }

    /// Create a measurer that shares its `FontSystem` with another component (typically
    /// a `TextRenderer`).
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self {
            font_system,
            cache: HashMap::new(),
            cache_entries: 0,
        }
    }

    /// Get a clone of the shared font system handle.
    pub fn font_system_handle(&self) -> FontSystemHandle {
        Arc::clone(&self.font_system)
    }

    /// Drop all cached measurements.
    ///
    /// Call this if the shared `FontSystem`'s font set changes after measuring, so the
    /// next measurement re-shapes against the new fonts.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.cache_entries = 0;
    }

    /// Measure text using glyphon's shaping/layout path, with a result cache.
    ///
    /// `max_width` constrains the shaping width; pass `None` for unconstrained
    /// single-line measurement, or `Some(w)` to let glyphon wrap the text and report
    /// the resulting multi-line height.
    ///
    /// On a cache hit this performs only a hash lookup and does not lock the
    /// `FontSystem`. On a miss it shapes the text (mutating glyphon's font system cache)
    /// and stores the result; it never touches any GPU renderer, atlas, or swash state.
    pub fn measure(&mut self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_with_font(text, font_size, max_width, None)
    }

    /// Like [`measure`](Self::measure), but shapes `text` in a specific font.
    ///
    /// Pass `None` for the default (system sans-serif). Different fonts have
    /// different glyph advances, so the font is part of the cache key — measuring
    /// the same string under two fonts caches two results.
    pub fn measure_with_font(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        font: Option<&FontHandle>,
    ) -> (f32, f32) {
        self.measure_styled(
            text,
            font_size,
            max_width,
            font,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        )
    }

    /// Like [`measure_with_font`](Self::measure_with_font), but also selects a
    /// font `weight`, `style`, and wrap policy. Bold/italic faces have different
    /// advances and the wrap policy changes line breaks, so each
    /// `(font, weight, style, wrap)` combination is a distinct cache entry — a
    /// measurement under this path matches a [`TextBlock`] rendered with the same
    /// font/weight/style/wrap.
    pub fn measure_styled(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        font: Option<&FontHandle>,
        weight: Weight,
        style: Style,
        wrap: WrapMode,
    ) -> (f32, f32) {
        let key = (
            font_size.to_bits(),
            max_width.map(f32::to_bits),
            family_hash(font),
            weight.0,
            style_disc(style),
            wrap,
        );

        if let Some(inner) = self.cache.get(&key) {
            if let Some(&dims) = inner.get(text) {
                return dims;
            }
        }

        let dims = {
            let mut fs = self.font_system.lock().expect("FontSystem poisoned");
            measure_with_font_system(
                &mut fs,
                text,
                font_size,
                max_width,
                font.map(|h| h.family()),
                weight,
                style,
                wrap,
            )
        };

        // Bound memory: dynamic strings (FPS, coordinates) would grow the cache
        // forever. Flush wholesale when full — static labels re-cache next frame.
        if self.cache_entries >= MEASURE_CACHE_CAP {
            self.clear_cache();
        }
        self.cache
            .entry(key)
            .or_default()
            .insert(text.to_string(), dims);
        self.cache_entries += 1;

        dims
    }
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a font handle into the `TextMeasurer` cache key. `None` (the default
/// font) hashes to 0; a named font hashes its family. Two different family names
/// colliding on a 64-bit hash is astronomically unlikely and only the cost is a
/// rare stale measurement, so a plain `DefaultHasher` is fine here.
fn family_hash(font: Option<&FontHandle>) -> u64 {
    use std::hash::{Hash, Hasher};
    match font {
        None => 0,
        Some(h) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            h.0.hash(&mut hasher);
            hasher.finish()
        }
    }
}

/// Map our [`TextAlign`] to cosmic-text's `Align`, returning `None` for the
/// default (`Left`) so callers can skip the per-line override.
fn cosmic_align(align: TextAlign) -> Option<CosmicAlign> {
    match align {
        TextAlign::Left => None,
        TextAlign::Center => Some(CosmicAlign::Center),
        TextAlign::Right => Some(CosmicAlign::Right),
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_with_font_system(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    max_width: Option<f32>,
    family_name: Option<&str>,
    weight: Weight,
    style: Style,
    wrap: WrapMode,
) -> (f32, f32) {
    let line_height = font_size * 1.25;
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    let shape_width = max_width.unwrap_or(f32::MAX / 4.0);
    buffer.set_wrap(font_system, wrap.into());
    buffer.set_size(font_system, Some(shape_width), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(
        font_system,
        text,
        Attrs::new().family(family).weight(weight).style(style),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    let mut width = 0.0f32;
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height += run.line_height;
    }

    if text.is_empty() {
        (0.0, line_height)
    } else {
        (width, height.max(line_height))
    }
}

/// Compute per-character cursor x-positions for the given text.
///
/// Returns a `Vec<(usize, f32)>` where each entry maps a byte index in `text`
/// to its x-offset (in screen pixels) from the left edge. The first entry is
/// always `(0, 0.0)` and the last is `(text.len(), total_width)`. For a single
/// line of text, these are monotonically increasing.
///
/// Use this to implement click-to-position (binary-search on the x values) and
/// to draw the text cursor / selection highlights at the correct x position
/// for a given byte offset.
///
/// `family_name` selects the font; pass `None` for the default sans-serif.
pub fn text_cursor_positions(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    family_name: Option<&str>,
) -> Vec<(usize, f32)> {
    let mut positions: Vec<(usize, f32)> = Vec::with_capacity(text.len().saturating_add(1));
    if text.is_empty() {
        positions.push((0, 0.0));
        return positions;
    }

    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(font_system, Some(max_width), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(
        font_system,
        text,
        Attrs::new().family(family),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    // Each cluster boundary (byte index) maps to the glyph's x-position.
    // We iterate in layout-run order (top-to-bottom for multi-line) and
    // record the x for each glyph's start. The last position is the total
    // width of the longest line.
    positions.push((0, 0.0));

    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let start_idx = glyph.start as usize;
            // Only record the first time we see each byte index.
            if start_idx > positions.last().map(|(i, _)| *i).unwrap_or(0) {
                positions.push((start_idx, glyph.x));
            }
        }
        // After each run, record the line end position.
        let line_end_x = run.line_w;
        // Find the last byte index of this run.
        if let Some(last_glyph) = run.glyphs.last() {
            let end_idx = last_glyph.end as usize;
            if end_idx > positions.last().map(|(i, _)| *i).unwrap_or(0) {
                positions.push((end_idx, line_end_x));
            }
        }
    }

    // Ensure the final byte index is always present.
    let last_byte = text.len();
    if positions.last().map(|(i, _)| *i).unwrap_or(0) < last_byte {
        // Estimate: use the last position's x plus an average char width.
        let last_x = positions.last().map(|(_, x)| *x).unwrap_or(0.0);
        positions.push((last_byte, last_x));
    }

    positions
}

/// Truncate `content` to a single line that fits within `max_width`, appending a
/// trailing `'…'`. Returns `content` unchanged when it already fits.
///
/// Shapes with no wrapping and reads the laid-out glyph positions to find the
/// byte cutoff, so it costs at most two extra shaping passes (the content and the
/// ellipsis) and only for blocks that actually overflow.
#[allow(clippy::too_many_arguments)]
fn ellipsize_to_width(
    fs: &mut FontSystem,
    content: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    family: Family,
    weight: Weight,
    style: Style,
) -> String {
    if content.is_empty() || !max_width.is_finite() || max_width <= 0.0 {
        return content.to_string();
    }
    let metrics = Metrics::new(font_size, line_height);
    let attrs = || Attrs::new().family(family).weight(weight).style(style);

    // Shape the full content on a single line.
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_wrap(fs, Wrap::None);
    buffer.set_size(fs, None, None);
    buffer.set_text(fs, content, attrs(), Shaping::Advanced);
    buffer.shape_until_scroll(fs, false);

    let full_w = buffer.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max);
    if full_w <= max_width {
        return content.to_string();
    }

    // Width of the ellipsis at this size/family, reserved at the right edge.
    let mut ell = Buffer::new(fs, metrics);
    ell.set_wrap(fs, Wrap::None);
    ell.set_size(fs, None, None);
    ell.set_text(fs, "…", attrs(), Shaping::Advanced);
    ell.shape_until_scroll(fs, false);
    let ellipsis_w = ell.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max);

    let budget = max_width - ellipsis_w;
    if budget <= 0.0 {
        return "…".to_string();
    }

    // Largest byte offset whose glyph still fits within the budget. Take the max
    // over all fitting glyphs (shaping/ligatures need not be end-ordered).
    let mut cut = 0usize;
    for run in buffer.layout_runs() {
        for g in run.glyphs {
            if g.x + g.w <= budget {
                cut = cut.max(g.end);
            }
        }
    }
    let cut = cut.min(content.len());
    let mut s = content[..cut].trim_end().to_string();
    s.push('…');
    s
}

/// A run of text within a [`TextBlock`] with optional per-span colour and
/// underline overrides.
///
/// **V1 constraint**: spans must share the block's global font attributes
/// (size, weight, style, family) — only colour and underline may vary. Mixed
/// font-size / weight spans require a `set_rich_text` shaping path and are
/// deferred to a future version.
#[derive(Debug, Clone, Default)]
pub struct TextSpan {
    /// The text content of this span.
    pub text: String,
    /// Per-span fill colour as `[r, g, b, a]` in `0.0..=1.0`. `None` →
    /// inherit the block's colour.
    pub color: Option<[f32; 4]>,
    /// Underline colour in `[r, g, b, a]` (same range). `None` → no underline.
    /// The underline rect is emitted as a coloured soup quad at
    /// [`DrawList::text`] time so it renders beneath the MSDF glyphs.
    pub underline: Option<[f32; 4]>,
}

/// Crisp outline drawn around glyphs, composited under the fill. Maps to
/// Teardown's `UiTextOutline(r, g, b, a, thickness)`.
#[derive(Clone, Copy, Debug)]
pub struct TextOutline {
    pub color: Color,
    /// Outline thickness in screen pixels.
    pub width_px: f32,
}

/// Drop shadow drawn offset behind the text. Maps to Teardown's
/// `UiTextShadow(r, g, b, a, distance, blur)`.
#[derive(Clone, Copy, Debug)]
pub struct TextShadow {
    pub color: Color,
    /// Screen-space offset `[dx, dy]`.
    pub offset: [f32; 2],
    /// Edge softness (blur) in screen pixels.
    pub softness: f32,
}

/// Soft colored halo around glyphs (a wide, soft, fill-less outline).
#[derive(Clone, Copy, Debug)]
pub struct TextGlow {
    pub color: Color,
    /// Halo radius in screen pixels.
    pub radius_px: f32,
}

/// A block of text to render.
#[derive(Clone)]
pub struct TextBlock {
    pub content: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub max_width: f32,
    pub color: Color,
    pub clip: Option<Rect>,
    /// Optional crisp outline (off by default).
    pub outline: Option<TextOutline>,
    /// Optional drop shadow (off by default).
    pub shadow: Option<TextShadow>,
    /// Optional soft glow halo (off by default).
    pub glow: Option<TextGlow>,
    /// Font to shape this block in. `None` = the default system sans-serif.
    pub font: Option<FontHandle>,
    /// Horizontal alignment within `max_width` (default `Left`).
    pub align: TextAlign,
    /// Single-line ellipsis mode: when `true`, the block is laid out on one line
    /// (no wrapping) and truncated with a trailing `'…'` if it would exceed
    /// `max_width`. When `false` (default) the block wraps at `max_width`.
    pub ellipsize: bool,
    /// Font weight selector (default [`Weight::NORMAL`]). Picks the matching face
    /// (e.g. [`Weight::BOLD`]) from the block's family during shaping; cosmic-text
    /// selects a real face and does **not** synthesize faux-bold when absent.
    pub weight: Weight,
    /// Font style selector (default [`Style::Normal`]). [`Style::Italic`] /
    /// [`Style::Oblique`] pick the matching face from the family when present.
    pub style: Style,
    /// Inline text spans for per-run colour and underline overrides. When
    /// non-empty, `content` is derived from the concatenation of span texts at
    /// draw time and need not be set by the caller. All spans must share the
    /// block's global font attributes (see [`TextSpan`]).
    pub spans: Vec<TextSpan>,
    /// Line-wrapping policy when the content exceeds `max_width` (default
    /// [`WrapMode::WordOrGlyph`], matching the historical implicit behaviour).
    /// Ignored in `ellipsize` mode, which always lays out on a single line.
    pub wrap: WrapMode,
}

impl TextBlock {
    pub fn new(content: impl Into<String>, x: f32, y: f32) -> Self {
        Self {
            content: content.into(),
            x,
            y,
            font_size: 16.0,
            line_height: 20.0,
            max_width: 800.0,
            color: Color::rgb(255, 255, 255),
            clip: None,
            outline: None,
            shadow: None,
            glow: None,
            font: None,
            align: TextAlign::Left,
            ellipsize: false,
            weight: Weight::NORMAL,
            style: Style::Normal,
            spans: Vec::new(),
            wrap: WrapMode::default(),
        }
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self.line_height = size * 1.25;
        self
    }

    pub fn with_max_width(mut self, width: f32) -> Self {
        self.max_width = width;
        self
    }

    pub fn with_color(mut self, r: u8, g: u8, b: u8) -> Self {
        self.color = Color::rgb(r, g, b);
        self
    }

    pub fn with_rgba(mut self, r: u8, g: u8, b: u8, a: u8) -> Self {
        self.color = Color::rgba(r, g, b, a);
        self
    }

    pub fn with_clip(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Add a crisp outline of `width_px` screen pixels in the given color.
    pub fn with_outline(mut self, r: u8, g: u8, b: u8, a: u8, width_px: f32) -> Self {
        self.outline = Some(TextOutline {
            color: Color::rgba(r, g, b, a),
            width_px,
        });
        self
    }

    /// Add a drop shadow offset by `(dx, dy)` screen px with `softness` px blur.
    ///
    /// The `(r, g, b, a, dx, dy, softness)` shape mirrors Teardown's
    /// `UiTextShadow(r, g, b, a, distance, blur)` for a direct binding mapping.
    #[allow(clippy::too_many_arguments)]
    pub fn with_shadow(mut self, r: u8, g: u8, b: u8, a: u8, dx: f32, dy: f32, softness: f32) -> Self {
        self.shadow = Some(TextShadow {
            color: Color::rgba(r, g, b, a),
            offset: [dx, dy],
            softness,
        });
        self
    }

    /// Add a soft glow halo of `radius_px` screen pixels.
    pub fn with_glow(mut self, r: u8, g: u8, b: u8, a: u8, radius_px: f32) -> Self {
        self.glow = Some(TextGlow {
            color: Color::rgba(r, g, b, a),
            radius_px,
        });
        self
    }

    /// Shape this block in `font` (from [`load_font_file`] / [`load_font_bytes`])
    /// instead of the default system sans-serif.
    pub fn with_font(mut self, font: FontHandle) -> Self {
        self.font = Some(font);
        self
    }

    /// Set the horizontal alignment within `max_width`. `Center`/`Right` only
    /// shift visibly when `max_width` exceeds the longest line.
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Enable single-line ellipsis: lay the text out on one line and truncate it
    /// with a trailing `'…'` if it would exceed [`Self::max_width`]. Use this for
    /// labels that must stay inside a fixed-width box (set `max_width` to that
    /// box's inner width).
    pub fn with_ellipsis(mut self) -> Self {
        self.ellipsize = true;
        self
    }

    /// Shape this block in `font` when `font` is `Some`, leaving the current font
    /// unchanged on `None`. Lets callers thread an optional theme font through
    /// without a branch at every call site.
    pub fn with_font_opt(mut self, font: Option<FontHandle>) -> Self {
        if let Some(f) = font {
            self.font = Some(f);
        }
        self
    }

    /// Render this block bold (shorthand for `with_weight(Weight::BOLD)`).
    pub fn bold(mut self) -> Self {
        self.weight = Weight::BOLD;
        self
    }

    /// Render this block italic (shorthand for `with_style(Style::Italic)`).
    pub fn italic(mut self) -> Self {
        self.style = Style::Italic;
        self
    }

    /// Select a specific font weight (e.g. `Weight::BOLD`, `Weight(500)`).
    pub fn with_weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Select a specific font style (`Style::Normal`/`Italic`/`Oblique`).
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Replace the block's text with the given inline spans. The display
    /// `content` is derived automatically as the concatenation of all span
    /// texts at draw time. See [`TextSpan`] for the V1 constraint (all spans
    /// must share the block's global font attributes).
    pub fn with_spans(mut self, spans: Vec<TextSpan>) -> Self {
        self.spans = spans;
        self
    }

    /// Set the line-wrapping policy (default [`WrapMode::WordOrGlyph`]). Use
    /// [`WrapMode::None`] to keep the text on one line and overflow `max_width`
    /// (pair with [`with_clip`](Self::with_clip) to hide the overflow).
    pub fn with_wrap(mut self, wrap: WrapMode) -> Self {
        self.wrap = wrap;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        color_to_rgba, cosmic_align, ellipsize_to_width, field_reach, load_font_bytes,
        measure_with_font_system, resolve_span_color, shared_font_system, text_cursor_positions,
        FontHandle, MsdfVertex, TextAlign, TextBlock, TextMeasurer, TextRenderer, TextSpan,
        WrapMode,
    };
    use glyphon::{Attrs, Buffer, Color, Family, Metrics, Shaping, Style, Weight};

    // ---- TextSpan / resolve_span_color ----

    fn red() -> [f32; 4] {
        [1.0, 0.0, 0.0, 1.0]
    }
    fn green() -> [f32; 4] {
        [0.0, 1.0, 0.0, 1.0]
    }
    fn blue() -> [f32; 4] {
        [0.0, 0.0, 1.0, 1.0]
    }

    #[test]
    fn resolve_span_color_picks_correct_span_for_each_byte() {
        // Spans: "Hello" (red) | " " (no color) | "World" (blue)
        // Bytes: 0..5            5..6              6..11
        let spans = vec![
            TextSpan { text: "Hello".into(), color: Some(red()), underline: None },
            TextSpan { text: " ".into(), color: None, underline: None },
            TextSpan { text: "World".into(), color: Some(blue()), underline: None },
        ];
        // First span: bytes 0–4
        assert_eq!(resolve_span_color(0, &spans), Some(red()));
        assert_eq!(resolve_span_color(4, &spans), Some(red()));
        // Second span: byte 5, color None
        assert_eq!(resolve_span_color(5, &spans), None);
        // Third span: bytes 6–10
        assert_eq!(resolve_span_color(6, &spans), Some(blue()));
        assert_eq!(resolve_span_color(10, &spans), Some(blue()));
    }

    #[test]
    fn resolve_span_color_empty_spans_returns_none() {
        assert_eq!(resolve_span_color(0, &[]), None);
    }

    #[test]
    fn resolve_span_color_all_no_color_returns_none() {
        let spans = vec![
            TextSpan { text: "abc".into(), color: None, underline: None },
            TextSpan { text: "def".into(), color: None, underline: None },
        ];
        assert_eq!(resolve_span_color(0, &spans), None);
        assert_eq!(resolve_span_color(3, &spans), None);
    }

    #[test]
    fn resolve_span_color_multibyte_utf8_boundary() {
        // "café" is 5 bytes (c-a-f-é where é = 2 bytes)
        let spans = vec![
            TextSpan { text: "café".into(), color: Some(red()), underline: None },
            TextSpan { text: "!".into(), color: Some(green()), underline: None },
        ];
        // 'é' is at byte offset 3 (0xc3 0xa9), so byte 3 and 4 are in first span
        assert_eq!(resolve_span_color(3, &spans), Some(red()));
        assert_eq!(resolve_span_color(4, &spans), Some(red()));
        // '!' is at byte offset 5
        assert_eq!(resolve_span_color(5, &spans), Some(green()));
    }

    // ---- TextBlock::with_spans ----

    #[test]
    fn with_spans_derives_content_from_span_texts() {
        let block = TextBlock::new("", 0.0, 0.0).with_spans(vec![
            TextSpan { text: "Hello".into(), color: None, underline: None },
            TextSpan { text: " ".into(), color: None, underline: None },
            TextSpan { text: "World".into(), color: None, underline: None },
        ]);
        // Content is derived by DrawList::text at draw time, not in the builder.
        // The builder just stores the spans; the content field is the caller's
        // responsibility or derived from spans at draw time.
        assert_eq!(block.spans.len(), 3);
        assert_eq!(block.spans[0].text, "Hello");
        assert_eq!(block.spans[2].text, "World");
    }

    #[test]
    fn with_spans_empty_vec_is_plain_mode() {
        let block = TextBlock::new("Hello", 0.0, 0.0).with_spans(vec![]);
        assert!(block.spans.is_empty());
        assert_eq!(block.content, "Hello");
    }

    #[test]
    fn color_to_rgba_normalizes_channels() {
        let c = Color::rgba(255, 128, 0, 64);
        let v = color_to_rgba(c);
        assert!((v[0] - 1.0).abs() < 1e-6);
        assert!((v[1] - 128.0 / 255.0).abs() < 1e-6);
        assert!((v[2] - 0.0).abs() < 1e-6);
        assert!((v[3] - 64.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn field_reach_scales_with_font_size() {
        // Reach grows linearly with font size and is zero (clamped) for tiny fonts.
        let small = field_reach(8.0, 12.0, 40.0);
        let large = field_reach(40.0, 12.0, 40.0);
        assert!(large > small);
        // At 40px with px_range 12 / ref 40: 0.5*12*40/40 - 0.5 = 5.5 px.
        assert!((large - 5.5).abs() < 1e-4);
        // Never negative.
        assert_eq!(field_reach(0.5, 12.0, 40.0).max(0.0), field_reach(0.5, 12.0, 40.0));
        assert!(field_reach(0.1, 12.0, 40.0) >= 0.0);
    }

    #[test]
    fn effect_builders_are_opt_in_and_set_fields() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert!(plain.outline.is_none() && plain.shadow.is_none() && plain.glow.is_none());

        let styled = TextBlock::new("x", 0.0, 0.0)
            .with_outline(0, 0, 0, 255, 2.0)
            .with_shadow(10, 20, 30, 200, 1.0, 2.0, 1.5)
            .with_glow(80, 180, 255, 255, 3.0);
        let o = styled.outline.unwrap();
        assert_eq!(o.width_px, 2.0);
        let s = styled.shadow.unwrap();
        assert_eq!(s.offset, [1.0, 2.0]);
        assert_eq!(s.softness, 1.5);
        let g = styled.glow.unwrap();
        assert_eq!(g.radius_px, 3.0);
    }

    #[test]
    fn measures_text_with_glyphon_layout() {
        let mut measurer = TextMeasurer::new();
        let (hello_width, hello_height) = measurer.measure("Hello", 16.0, None);
        assert!(hello_width > 0.0);
        assert!(hello_height > 0.0);

        let font_size = 16.0;
        let (m_width, _) = measurer.measure("M", font_size, None);
        let approximate_width = "M".len() as f32 * font_size * 0.5;
        assert!((m_width - approximate_width).abs() > f32::EPSILON);
    }

    #[test]
    fn measure_with_max_width_wraps_to_multiple_lines() {
        let mut measurer = TextMeasurer::new();
        let long = "The quick brown fox jumps over the lazy dog repeatedly each morning.";
        let (_, h_unwrapped) = measurer.measure(long, 14.0, None);
        let (_, h_wrapped) = measurer.measure(long, 14.0, Some(80.0));
        assert!(h_wrapped > h_unwrapped);
    }

    /// Helper: number of laid-out lines = height / line_height (font_size*1.25).
    fn line_count(measurer: &mut TextMeasurer, text: &str, size: f32, w: f32, wrap: WrapMode) -> u32 {
        let (_, h) =
            measurer.measure_styled(text, size, Some(w), None, Weight::NORMAL, Style::Normal, wrap);
        (h / (size * 1.25)).round() as u32
    }

    #[test]
    fn wrap_mode_controls_line_count() {
        let mut m = TextMeasurer::new();
        let size = 14.0;
        let w = 70.0;

        // A multi-word string narrower than its natural width: every wrapping
        // mode breaks it; `None` keeps it on one line. (Word- vs glyph-packing
        // line *counts* are font-dependent, so we don't compare those two here —
        // see the unbreakable-word case below for that distinction.)
        let words = "alpha beta gamma delta epsilon";
        assert_eq!(
            line_count(&mut m, words, size, w, WrapMode::None),
            1,
            "Wrap::None must stay on a single line",
        );
        assert!(line_count(&mut m, words, size, w, WrapMode::Word) > 1, "Word should wrap");
        assert!(
            line_count(&mut m, words, size, w, WrapMode::WordOrGlyph) > 1,
            "WordOrGlyph should wrap",
        );
        assert!(line_count(&mut m, words, size, w, WrapMode::Glyph) > 1, "Glyph should wrap");

        // A single word with no break opportunities: `Word` cannot break it (it
        // overflows on one line) while `Glyph`/`WordOrGlyph` break mid-word.
        // This is the font-independent Word-vs-Glyph distinction.
        let long_word = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            line_count(&mut m, long_word, size, w, WrapMode::Word),
            1,
            "Word wrap cannot split a single long word — it overflows on one line",
        );
        assert!(
            line_count(&mut m, long_word, size, w, WrapMode::Glyph) > 1,
            "Glyph wrap must break a long word across lines",
        );
        assert!(
            line_count(&mut m, long_word, size, w, WrapMode::WordOrGlyph) > 1,
            "WordOrGlyph falls back to glyph breaks for a too-long word",
        );
    }

    #[test]
    fn wrap_mode_default_is_word_or_glyph() {
        // The TextBlock default and the bare `measure` default must agree.
        assert_eq!(WrapMode::default(), WrapMode::WordOrGlyph);
        let block = TextBlock::new("x", 0.0, 0.0);
        assert_eq!(block.wrap, WrapMode::WordOrGlyph);
        let with = block.with_wrap(WrapMode::None);
        assert_eq!(with.wrap, WrapMode::None);

        // `measure` (no wrap arg) must match `measure_styled` with the default,
        // proving the convenience path forwards the same policy.
        let mut m = TextMeasurer::new();
        let text = "alpha beta gamma delta epsilon";
        let bare = m.measure(text, 14.0, Some(70.0));
        let styled = m.measure_styled(
            text,
            14.0,
            Some(70.0),
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        );
        assert_eq!(bare, styled);
    }

    #[test]
    fn wrap_mode_is_part_of_measure_cache_key() {
        // Same content/metrics, different wrap → distinct cached results (None
        // stays one line, Glyph wraps), so the wrap must be in the key.
        let mut m = TextMeasurer::new();
        let text = "alpha beta gamma delta epsilon";
        let (_, h_none) = m.measure_styled(
            text, 14.0, Some(70.0), None, Weight::NORMAL, Style::Normal, WrapMode::None,
        );
        let (_, h_glyph) = m.measure_styled(
            text, 14.0, Some(70.0), None, Weight::NORMAL, Style::Normal, WrapMode::Glyph,
        );
        assert!(h_glyph > h_none, "distinct wrap modes must not collide in the cache");
        // Re-measuring None still returns the one-line height (key really splits).
        let (_, h_none2) = m.measure_styled(
            text, 14.0, Some(70.0), None, Weight::NORMAL, Style::Normal, WrapMode::None,
        );
        assert_eq!(h_none, h_none2);
    }

    #[test]
    fn cache_returns_identical_results_on_repeat() {
        let mut measurer = TextMeasurer::new();
        let first = measurer.measure("Cached label", 16.0, None);
        // Second call must hit the cache and return the exact same dimensions.
        let second = measurer.measure("Cached label", 16.0, None);
        assert_eq!(first, second);
    }

    #[test]
    fn cache_keys_on_font_size_and_max_width() {
        let mut measurer = TextMeasurer::new();
        let small = measurer.measure("Hello", 12.0, None);
        let large = measurer.measure("Hello", 24.0, None);
        // Different font sizes are distinct cache entries with distinct metrics.
        assert!(large.0 > small.0);
        assert!(large.1 > small.1);
        // Re-measuring each still returns its own cached value.
        assert_eq!(measurer.measure("Hello", 12.0, None), small);
        assert_eq!(measurer.measure("Hello", 24.0, None), large);
    }

    #[test]
    fn clear_cache_forces_remeasure_without_changing_result() {
        let mut measurer = TextMeasurer::new();
        let before = measurer.measure("Persistent", 18.0, None);
        measurer.clear_cache();
        let after = measurer.measure("Persistent", 18.0, None);
        assert_eq!(before, after);
    }

    #[test]
    fn load_font_bytes_returns_family_name() {
        let fs = shared_font_system();
        let handle = load_font_bytes(&fs, notosans::REGULAR_TTF).expect("load noto");
        assert!(!handle.family().is_empty());
        assert!(
            handle.family().to_lowercase().contains("noto"),
            "expected a Noto family, got {:?}",
            handle.family()
        );
    }

    #[test]
    fn load_font_bytes_rejects_garbage() {
        let fs = shared_font_system();
        assert!(load_font_bytes(&fs, &[0u8, 1, 2, 3, 4, 5, 6, 7]).is_err());
    }

    #[test]
    fn loaded_font_is_actually_selected_during_shaping() {
        // The real proof that `with_font` works: shape a string selecting the
        // loaded family and confirm cosmic-text resolved glyphs to *that* face.
        let fs = shared_font_system();
        let handle = load_font_bytes(&fs, notosans::REGULAR_TTF).unwrap();
        let mut guard = fs.lock().unwrap();
        let mut buffer = Buffer::new(&mut guard, Metrics::new(20.0, 25.0));
        buffer.set_text(
            &mut guard,
            "Ag",
            Attrs::new().family(Family::Name(handle.family())),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut guard, false);
        let font_id = buffer.layout_runs().next().unwrap().glyphs[0].font_id;
        let info = guard.db().face(font_id).expect("resolved face exists");
        let family = info
            .families
            .first()
            .map(|(n, _)| n.as_str())
            .unwrap_or("");
        assert_eq!(family, handle.family());
    }

    #[test]
    fn measure_with_font_is_cached_per_font() {
        // Default and custom-font measurements live under distinct cache keys and
        // each round-trips. (We don't assert the metrics differ — the system
        // default sans-serif may itself be Noto on some hosts.)
        let fs = shared_font_system();
        let handle = load_font_bytes(&fs, notosans::REGULAR_TTF).unwrap();
        let mut measurer = TextMeasurer::with_font_system(fs);
        let default = measurer.measure("Hello world", 16.0, None);
        let noto = measurer.measure_with_font("Hello world", 16.0, None, Some(&handle));
        assert!(default.0 > 0.0 && noto.0 > 0.0);
        // Re-measuring each returns its own cached value (proves keys are distinct
        // when the metrics happen to coincide, and stable when they don't).
        assert_eq!(measurer.measure("Hello world", 16.0, None), default);
        assert_eq!(
            measurer.measure_with_font("Hello world", 16.0, None, Some(&handle)),
            noto
        );
    }

    #[test]
    fn font_and_align_defaults_and_builders() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert!(plain.font.is_none());
        assert_eq!(plain.align, TextAlign::Left);

        let styled = TextBlock::new("x", 0.0, 0.0)
            .with_font(FontHandle("Noto Sans".to_string()))
            .with_align(TextAlign::Center);
        assert_eq!(styled.font.as_ref().unwrap().family(), "Noto Sans");
        assert_eq!(styled.align, TextAlign::Center);
    }

    #[test]
    fn alignment_shifts_leftmost_glyph_within_max_width() {
        // Mirrors the shaping in `build_vertices`: a short line in a wide box
        // moves rightward under Center then Right. Asserting on cosmic-text's
        // per-glyph x (which the renderer adds to `block.x`) keeps this GPU-free.
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let mut leftmost = |align: TextAlign| -> f32 {
            let mut buffer = Buffer::new(&mut guard, Metrics::new(16.0, 20.0));
            buffer.set_size(&mut guard, Some(400.0), None);
            buffer.set_text(
                &mut guard,
                "short",
                Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
            );
            if let Some(a) = cosmic_align(align) {
                for line in buffer.lines.iter_mut() {
                    line.set_align(Some(a));
                }
            }
            buffer.shape_until_scroll(&mut guard, false);
            buffer.layout_runs().next().unwrap().glyphs[0].x
        };
        let left = leftmost(TextAlign::Left);
        let center = leftmost(TextAlign::Center);
        let right = leftmost(TextAlign::Right);
        assert!(center > left, "center {center} should exceed left {left}");
        assert!(right > center, "right {right} should exceed center {center}");
    }

    #[test]
    fn with_ellipsis_is_opt_in() {
        assert!(!TextBlock::new("x", 0.0, 0.0).ellipsize);
        assert!(TextBlock::new("x", 0.0, 0.0).with_ellipsis().ellipsize);
    }

    #[test]
    fn weight_and_style_defaults_and_builders() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert_eq!(plain.weight, Weight::NORMAL);
        assert_eq!(plain.style, Style::Normal);

        assert_eq!(TextBlock::new("x", 0.0, 0.0).bold().weight, Weight::BOLD);
        assert_eq!(TextBlock::new("x", 0.0, 0.0).italic().style, Style::Italic);
        assert_eq!(
            TextBlock::new("x", 0.0, 0.0).with_weight(Weight(500)).weight,
            Weight(500)
        );
        assert_eq!(
            TextBlock::new("x", 0.0, 0.0).with_style(Style::Oblique).style,
            Style::Oblique
        );
    }

    #[test]
    fn with_font_opt_only_applies_some() {
        let none = TextBlock::new("x", 0.0, 0.0).with_font_opt(None);
        assert!(none.font.is_none());
        let some = TextBlock::new("x", 0.0, 0.0)
            .with_font_opt(Some(FontHandle("Noto Sans".to_string())));
        assert_eq!(some.font.as_ref().unwrap().family(), "Noto Sans");
        // Some over an existing font replaces it; None leaves it untouched.
        let kept = TextBlock::new("x", 0.0, 0.0)
            .with_font(FontHandle("A".into()))
            .with_font_opt(None);
        assert_eq!(kept.font.as_ref().unwrap().family(), "A");
    }

    #[test]
    fn style_disc_is_stable() {
        assert_eq!(super::style_disc(Style::Normal), 0);
        assert_eq!(super::style_disc(Style::Italic), 1);
        assert_eq!(super::style_disc(Style::Oblique), 2);
    }

    #[test]
    fn bold_measures_wider_than_regular() {
        // Load the regular + bold Noto faces (same "Noto Sans" family); the
        // weight selects between them at shape time. GPU-free — measurer only.
        let fs = shared_font_system();
        let regular = load_font_bytes(&fs, notosans::REGULAR_TTF).unwrap();
        let _bold = load_font_bytes(&fs, notosans::BOLD_TTF).unwrap();
        let mut m = TextMeasurer::with_font_system(fs);
        let text = "The quick brown fox jumps";
        let (rw, _) =
            m.measure_styled(text, 18.0, None, Some(&regular), Weight::NORMAL, Style::Normal, WrapMode::default());
        let (bw, _) =
            m.measure_styled(text, 18.0, None, Some(&regular), Weight::BOLD, Style::Normal, WrapMode::default());
        assert!(rw > 0.0 && bw > 0.0);
        assert!(bw > rw, "bold width {bw} should exceed regular {rw}");
        // Distinct cache entries keyed by weight: re-measuring regular still
        // returns the regular width (proves weight is part of the key).
        let (rw2, _) =
            m.measure_styled(text, 18.0, None, Some(&regular), Weight::NORMAL, Style::Normal, WrapMode::default());
        assert_eq!(rw2, rw);
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn bundled_font_is_default_sans_serif() {
        // `shared_font_system` registers the bundled Noto faces + sets the
        // default sans-serif, so `Family::SansSerif` resolves to Noto.
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();

        let mut buffer = Buffer::new(&mut guard, Metrics::new(18.0, 22.0));
        buffer.set_text(
            &mut guard,
            "Ag",
            Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut guard, false);
        let font_id = buffer.layout_runs().next().unwrap().glyphs[0].font_id;
        let fam = guard
            .db()
            .face(font_id)
            .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
            .unwrap_or_default();
        assert!(
            fam.to_lowercase().contains("noto"),
            "default sans-serif should be the bundled Noto family, got {fam:?}"
        );

        // Bold weight selects a heavier face from the same bundled family.
        let mut bold = Buffer::new(&mut guard, Metrics::new(18.0, 22.0));
        bold.set_text(
            &mut guard,
            "Ag",
            Attrs::new().family(Family::SansSerif).weight(Weight::BOLD),
            Shaping::Advanced,
        );
        bold.shape_until_scroll(&mut guard, false);
        let bold_id = bold.layout_runs().next().unwrap().glyphs[0].font_id;
        let bold_weight = guard.db().face(bold_id).map(|f| f.weight.0).unwrap_or(0);
        assert!(
            bold_weight >= 600,
            "bold weight should select a bold face (>=600), got {bold_weight}"
        );
    }

    #[test]
    fn ellipsize_leaves_fitting_text_unchanged() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        // A wide budget the short string easily fits within.
        let out = ellipsize_to_width(
            &mut guard,
            "short",
            16.0,
            20.0,
            1000.0,
            Family::SansSerif,
            Weight::NORMAL,
            Style::Normal,
        );
        assert_eq!(out, "short");
    }

    #[test]
    fn ellipsize_truncates_overflowing_text_with_ellipsis() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let long = "a_very_long_object_name_that_will_not_fit";
        let max_width = 80.0;
        let out = ellipsize_to_width(
            &mut guard,
            long,
            14.0,
            18.0,
            max_width,
            Family::SansSerif,
            Weight::NORMAL,
            Style::Normal,
        );
        assert_ne!(out, long, "overflowing text should be truncated");
        assert!(out.ends_with('…'), "truncated text should end with an ellipsis");
        assert!(out.chars().count() < long.chars().count());
        // The truncated line (incl. the ellipsis) must fit the budget.
        let (w, _) = measure_with_font_system(
            &mut guard,
            &out,
            14.0,
            None,
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        );
        assert!(w <= max_width, "ellipsized width {w} must fit {max_width}");
    }

    #[test]
    fn ellipsize_degenerate_budget_returns_just_ellipsis() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        // A budget too small for even one glyph + the ellipsis.
        let out = ellipsize_to_width(
            &mut guard,
            "anything",
            14.0,
            18.0,
            2.0,
            Family::SansSerif,
            Weight::NORMAL,
            Style::Normal,
        );
        assert_eq!(out, "…");
    }

    // ---- text_cursor_positions tests ----

    #[test]
    fn cursor_positions_empty_text() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let pos = text_cursor_positions(&mut guard, "", 16.0, 20.0, 800.0, None);
        assert_eq!(pos, &[(0, 0.0)]);
    }

    #[test]
    fn cursor_positions_has_origin_first() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let pos = text_cursor_positions(&mut guard, "Hi", 16.0, 20.0, 800.0, None);
        assert_eq!(pos.first(), Some(&(0, 0.0)));
    }

    #[test]
    fn cursor_positions_last_is_text_len() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let text = "Hello";
        let pos = text_cursor_positions(&mut guard, text, 16.0, 20.0, 800.0, None);
        assert_eq!(pos.last().map(|(i, _)| *i), Some(text.len()));
    }

    #[test]
    fn cursor_positions_monotonically_increasing() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let text = "The quick brown fox";
        let pos = text_cursor_positions(&mut guard, text, 16.0, 20.0, 800.0, None);
        for pair in pos.windows(2) {
            assert!(
                pair[0].1 <= pair[1].1,
                "position regressed: {:?} -> {:?}",
                pair[0],
                pair[1]
            );
            assert!(
                pair[0].0 <= pair[1].0,
                "byte index regressed: {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn cursor_positions_last_matches_measure_width() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let text = "Hello World";
        let font_size = 16.0;
        let max_width = 800.0;
        let pos = text_cursor_positions(&mut guard, text, font_size, font_size * 1.25, max_width, None);

        let (total_w, _) = measure_with_font_system(
            &mut guard,
            text,
            font_size,
            None,
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        );
        let final_x = pos.last().map(|(_, x)| *x).unwrap_or(0.0);
        // The final x-position should approximate the measured width.
        assert!(
            (final_x - total_w).abs() < 2.0,
            "final x {final_x} differs from measured width {total_w} by >2px"
        );
    }

    #[test]
    fn cursor_positions_multibyte_utf8() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        // "é" is 2 bytes (U+00E9), "あ" is 3 bytes (U+3042).
        // Positions should be recorded at the correct *byte* boundaries:
        //   "éXあ" → bytes: [0..2) = é, [2..3) = X, [3..6) = あ
        let text = "éXあ";
        let pos = text_cursor_positions(&mut guard, text, 16.0, 20.0, 800.0, None);

        // We should have a position for byte 0, byte 2 (after é), byte 3 (after X),
        // and byte 6 (end of あ).
        let indices: Vec<usize> = pos.iter().map(|(i, _)| *i).collect();
        assert!(
            indices.contains(&0),
            "should have position at byte 0, got {indices:?}"
        );
        assert!(
            indices.contains(&2),
            "should have position at byte 2 (after é), got {indices:?}"
        );
        assert!(
            indices.contains(&3),
            "should have position at byte 3 (after X), got {indices:?}"
        );
        assert!(
            indices.contains(&6),
            "should have position at byte 6 (end), got {indices:?}"
        );
        assert_eq!(pos.last().map(|(i, _)| *i), Some(6));
    }

    // ----- Shaped-text cache (GPU-gated, like the chrome parity test) -----

    /// Build a headless `TextRenderer` + a loaded Noto font handle, or `None` if
    /// no GPU adapter is available (so the `#[ignore]`d tests below no-op safely).
    fn headless_renderer() -> Option<(wgpu::Device, wgpu::Queue, TextRenderer, FontHandle)> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("text-cache test device"),
                ..Default::default()
            },
            None,
        ))
        .ok()?;
        let fs = shared_font_system();
        let font = load_font_bytes(&fs, notosans::REGULAR_TTF).expect("load noto");
        let renderer = TextRenderer::with_font_system(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            fs,
        );
        Some((device, queue, renderer, font))
    }

    /// Total cached shaping entries across all outer keys.
    fn cache_total(r: &TextRenderer) -> usize {
        r.shape_cache.values().map(|inner| inner.len()).sum()
    }

    /// Reinterpret a vertex slice as raw bytes for exact-equality comparison
    /// (`MsdfVertex` is `Pod` but not `PartialEq`).
    fn vbytes(v: &[MsdfVertex]) -> &[u8] {
        bytemuck::cast_slice(v)
    }

    fn label(text: &str, font: &FontHandle) -> TextBlock {
        TextBlock::new(text, 40.0, 40.0)
            .with_size(18.0)
            .with_font(font.clone())
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn shape_cache_reuses_layout_and_matches_cold() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let blocks = [label("Cached label", &font)];

        // Cold: shapes and records exactly one entry.
        let cold = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1, "one shaping entry recorded");
        assert!(!cold.is_empty(), "non-empty glyph geometry");

        // Warm: served from the cache, byte-identical output, still one entry.
        let warm = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1, "warm frame adds no new entries");
        assert_eq!(vbytes(&cold), vbytes(&warm), "cache must not change pixels");
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn moving_block_reuses_cache_and_shifts() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let a = r.build_vertices(&[label("Move me", &font)]);
        let mut moved = label("Move me", &font);
        moved.x += 100.0;
        moved.y += 50.0;
        let b = r.build_vertices(&[moved]);

        // Same key + content => still a single cache entry (a hit, not a re-shape).
        assert_eq!(cache_total(&r), 1, "moved block reuses the cached layout");
        assert_eq!(a.len(), b.len());
        for (va, vb) in a.iter().zip(b.iter()) {
            assert!((vb.position[0] - va.position[0] - 100.0).abs() < 1e-3);
            assert!((vb.position[1] - va.position[1] - 50.0).abs() < 1e-3);
            // Everything but position is unchanged: compare uv + fill.
            assert_eq!(va.uv, vb.uv);
            assert_eq!(va.fill, vb.fill);
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn distinct_attributes_create_distinct_entries() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Each variant differs in exactly one keyed attribute, so each is a miss.
        let base = label("Same", &font);
        let mut bigger = label("Same", &font);
        bigger.font_size = 24.0;
        let mut narrower = label("Same", &font);
        narrower.max_width = 20.0;
        let mut centered = label("Same", &font);
        centered.align = TextAlign::Center;
        let mut clipped_ellipsis = label("Same", &font);
        clipped_ellipsis.ellipsize = true;
        let other_content = label("Different", &font);
        let default_font = label("Same", &font).with_max_width(800.0);
        let mut default_font = default_font;
        default_font.font = None; // None vs Noto => different family hash
        let bold_variant = label("Same", &font).bold();
        let italic_variant = label("Same", &font).italic();

        r.build_vertices(&[
            base,
            bigger,
            narrower,
            centered,
            clipped_ellipsis,
            other_content,
            default_font,
            bold_variant,
            italic_variant,
        ]);
        assert_eq!(
            cache_total(&r),
            9,
            "each distinct (content|size|width|align|ellipsize|font|weight|style) is its own entry"
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn eviction_prunes_to_working_set() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Frame 1: overflow the cap with distinct strings (same key, distinct
        // content). All are used this frame, so the post-frame eviction retains
        // them (nothing is stale yet).
        let many: Vec<TextBlock> = (0..=super::SHAPE_CACHE_MAX)
            .map(|i| label(&format!("e{i}"), &font))
            .collect();
        r.build_vertices(&many);
        assert!(
            cache_total(&r) > super::SHAPE_CACHE_MAX,
            "frame 1 keeps the whole working set"
        );

        // Frame 2: a tiny working set. The cache is still over cap, so everything
        // not touched this frame is evicted, leaving just the live entry.
        r.build_vertices(&[label("e0", &font)]);
        assert_eq!(cache_total(&r), 1, "stale entries pruned to the working set");
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn clear_shape_cache_forces_reshape_with_identical_result() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let blocks = [label("Reshape", &font)];
        let before = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1);

        r.clear_shape_cache();
        assert_eq!(cache_total(&r), 0, "clear empties the cache");

        let after = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1, "re-shaped and re-cached");
        assert_eq!(vbytes(&before), vbytes(&after), "re-shape is identical");
    }
}
