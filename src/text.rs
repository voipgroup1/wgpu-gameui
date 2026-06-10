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
use glyphon::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping};

const MSDF_SHADER: &str = include_str!("render/ui_msdf.wgsl");

/// Shared handle to a glyphon `FontSystem`.
///
/// Both `TextRenderer` and `TextMeasurer` hold the same handle so measured text widths
/// (used for layout) match rendered glyphs (used for output) — including any custom
/// fonts loaded into the system later.
pub type FontSystemHandle = Arc<Mutex<FontSystem>>;

/// Create a new shared `FontSystem` handle.
pub fn shared_font_system() -> FontSystemHandle {
    Arc::new(Mutex::new(FontSystem::new()))
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    /// Lines start at the left edge of the box (default).
    #[default]
    Left,
    /// Lines are centered within `max_width`.
    Center,
    /// Lines are flushed to the right edge of `max_width`.
    Right,
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

    /// Stable per-font keys for the atlas, assigned on first sighting. Decouples
    /// the atlas from cosmic-text's `fontdb::ID`.
    font_keys: HashMap<fontdb::ID, u64>,
    next_font_key: u64,

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
            font_keys: HashMap::new(),
            next_font_key: 0,
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

    /// Measure text using cosmic-text's shaping/layout path without touching GPU state.
    pub fn measure(&mut self, text: &str, font_size: f32) -> (f32, f32) {
        let mut fs = self.font_system.lock().expect("FontSystem poisoned");
        measure_with_font_system(&mut fs, text, font_size, None, None)
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
        if let Some(k) = self.font_keys.get(&id) {
            return *k;
        }
        let k = self.next_font_key;
        self.next_font_key += 1;
        self.font_keys.insert(id, k);
        k
    }

    /// Build glyph quads for all text blocks, generating any unseen glyphs into the
    /// atlas. Returns the vertex list (6 verts/glyph, triangle list).
    fn build_vertices(&mut self, texts: &[TextBlock]) -> Vec<MsdfVertex> {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_shape").entered();

        let fs_handle = Arc::clone(&self.font_system);
        let mut fs = fs_handle.lock().expect("FontSystem poisoned");
        let px_range = self.atlas.px_range();
        let ref_px = self.atlas.ref_px();

        // First pass: shape every block and generate any unseen glyphs, collecting
        // per-glyph placements. We resolve uv only *after* this pass, because glyph
        // generation can grow the atlas (changing the size uv divides by) — pixel
        // regions stay valid (top-left origin) but uv must use the final size.
        let mut placements: Vec<GlyphPlacement> = Vec::new();

        for block in texts {
            if block.content.is_empty() {
                continue;
            }
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
                buffer.set_size(&mut fs, Some(block.max_width), None);
            }
            buffer.set_text(
                &mut fs,
                content,
                Attrs::new().family(family).color(block.color),
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

            let default_fill = color_to_rgba(block.color);
            let clip = block.clip.map(|c| [c.x, c.y, c.width, c.height]);
            let outline = block.outline.as_ref().map(|o| (color_to_rgba(o.color), o.width_px));
            let shadow = block
                .shadow
                .as_ref()
                .map(|s| (color_to_rgba(s.color), s.offset, s.softness));
            let glow = block.glow.as_ref().map(|g| (color_to_rgba(g.color), g.radius_px));

            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    let font_size = glyph.font_size;
                    // Pen origin on the baseline, in screen space (mirrors
                    // cosmic-text's `glyph.physical((left, run.line_y), 1.0)`).
                    let pen_x = block.x + glyph.x + font_size * glyph.x_offset;
                    let baseline_y = block.y + run.line_y + glyph.y - font_size * glyph.y_offset;

                    let font_key = self.font_key(glyph.font_id);
                    let Some(font) = fs.get_font(glyph.font_id) else {
                        continue;
                    };
                    let Some(tile) = self.atlas.glyph(font_key, glyph.glyph_id, font.data()) else {
                        continue; // whitespace / outline-less
                    };

                    let fill = glyph.color_opt.map(color_to_rgba).unwrap_or(default_fill);
                    placements.push(GlyphPlacement {
                        tile,
                        pen_x,
                        baseline_y,
                        font_size,
                        clip,
                        fill,
                        outline,
                        shadow,
                        glow,
                    });
                }
            }
        }
        drop(fs);

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
        self.ensure_vbo_capacity(device, verts.len());
        queue.write_buffer(&self.vbo, 0, bytemuck::cast_slice(&verts));

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
        pass.set_vertex_buffer(0, self.vbo.slice(..));
        pass.draw(0..verts.len() as u32, 0..1);
    }

    fn ensure_vbo_capacity(&mut self, device: &wgpu::Device, verts: usize) {
        let needed = (verts * std::mem::size_of::<MsdfVertex>()) as u64;
        if needed > self.vbo_capacity {
            self.vbo_capacity = needed.next_power_of_two();
            self.vbo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("msdf text vbo"),
                size: self.vbo_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
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
    /// Keyed by quantized `(font_size_bits, max_width_bits, family_hash)` so the
    /// inner `HashMap<String, _>` can be probed with a borrowed `&str` — no key
    /// allocation on a cache hit, only on a miss when we insert. `family_hash`
    /// is 0 for the default font; different fonts have different advances so the
    /// font must be part of the key.
    cache: HashMap<(u32, Option<u32>, u64), HashMap<String, (f32, f32)>>,
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
        let key = (
            font_size.to_bits(),
            max_width.map(f32::to_bits),
            family_hash(font),
        );

        if let Some(inner) = self.cache.get(&key) {
            if let Some(&dims) = inner.get(text) {
                return dims;
            }
        }

        let dims = {
            let mut fs = self.font_system.lock().expect("FontSystem poisoned");
            measure_with_font_system(&mut fs, text, font_size, max_width, font.map(|h| h.family()))
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

fn measure_with_font_system(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    max_width: Option<f32>,
    family_name: Option<&str>,
) -> (f32, f32) {
    let line_height = font_size * 1.25;
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    let shape_width = max_width.unwrap_or(f32::MAX / 4.0);
    buffer.set_size(font_system, Some(shape_width), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(
        font_system,
        text,
        Attrs::new().family(family),
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
fn ellipsize_to_width(
    fs: &mut FontSystem,
    content: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    family: Family,
) -> String {
    if content.is_empty() || !max_width.is_finite() || max_width <= 0.0 {
        return content.to_string();
    }
    let metrics = Metrics::new(font_size, line_height);

    // Shape the full content on a single line.
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_wrap(fs, Wrap::None);
    buffer.set_size(fs, None, None);
    buffer.set_text(fs, content, Attrs::new().family(family), Shaping::Advanced);
    buffer.shape_until_scroll(fs, false);

    let full_w = buffer.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max);
    if full_w <= max_width {
        return content.to_string();
    }

    // Width of the ellipsis at this size/family, reserved at the right edge.
    let mut ell = Buffer::new(fs, metrics);
    ell.set_wrap(fs, Wrap::None);
    ell.set_size(fs, None, None);
    ell.set_text(fs, "…", Attrs::new().family(family), Shaping::Advanced);
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
}

#[cfg(test)]
mod tests {
    use super::{
        color_to_rgba, cosmic_align, ellipsize_to_width, field_reach, load_font_bytes,
        measure_with_font_system, shared_font_system, text_cursor_positions, FontHandle, TextAlign,
        TextBlock, TextMeasurer,
    };
    use glyphon::{Attrs, Buffer, Color, Family, Metrics, Shaping};

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
    fn ellipsize_leaves_fitting_text_unchanged() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        // A wide budget the short string easily fits within.
        let out = ellipsize_to_width(&mut guard, "short", 16.0, 20.0, 1000.0, Family::SansSerif);
        assert_eq!(out, "short");
    }

    #[test]
    fn ellipsize_truncates_overflowing_text_with_ellipsis() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        let long = "a_very_long_object_name_that_will_not_fit";
        let max_width = 80.0;
        let out = ellipsize_to_width(&mut guard, long, 14.0, 18.0, max_width, Family::SansSerif);
        assert_ne!(out, long, "overflowing text should be truncated");
        assert!(out.ends_with('…'), "truncated text should end with an ellipsis");
        assert!(out.chars().count() < long.chars().count());
        // The truncated line (incl. the ellipsis) must fit the budget.
        let (w, _) = measure_with_font_system(&mut guard, &out, 14.0, None, None);
        assert!(w <= max_width, "ellipsized width {w} must fit {max_width}");
    }

    #[test]
    fn ellipsize_degenerate_budget_returns_just_ellipsis() {
        let fs = shared_font_system();
        let mut guard = fs.lock().unwrap();
        // A budget too small for even one glyph + the ellipsis.
        let out = ellipsize_to_width(&mut guard, "anything", 14.0, 18.0, 2.0, Family::SansSerif);
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

        let (total_w, _) = measure_with_font_system(&mut guard, text, font_size, None, None);
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
}
