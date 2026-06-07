//! `UiRenderer` — single-call renderer consuming a `DrawList`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::layer::LayerStack;
use crate::render::atlas::{SpriteAtlas, SpriteId};
use crate::render::image_cache::{decode_rgba8, ImageCache, ImageEntry, ImageError};
use crate::text::FontSystemHandle;
use crate::widgets::{DrawList, IconDraw, NineSliceDraw, NineSliceId, Vertex};
use crate::TextRenderer;

const SHADER: &str = include_str!("ui.wgsl");

/// Metadata describing a registered nine-slice resource.
#[derive(Clone, Debug)]
pub struct NineSliceMeta {
    pub sprite: SpriteId,
    /// Border insets in source pixels: [left, top, right, bottom].
    pub border: [u32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct TexVertex {
    position: [f32; 2],
    uv: [f32; 2],
    tint: [f32; 4],
    clip: [f32; 4],
    clip_enabled: f32,
}

impl TexVertex {
    fn new(pos: [f32; 2], uv: [f32; 2], tint: [f32; 4], clip: Option<[f32; 4]>) -> Self {
        let (clip_rect, enabled) = match clip {
            Some(r) => (r, 1.0),
            None => ([0.0; 4], 0.0),
        };
        Self {
            position: pos,
            uv,
            tint,
            clip: clip_rect,
            clip_enabled: enabled,
        }
    }
}

const TEX_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32,
];

const COLOR_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32,
];

/// Public renderer.
pub struct UiRenderer {
    // Pipelines
    color_pipeline: wgpu::RenderPipeline,
    tex_pipeline: wgpu::RenderPipeline,

    // Uniforms (shared by both pipelines via group(0))
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    // Atlas resources
    atlas: SpriteAtlas,
    texture: wgpu::Texture,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    current_atlas_size: u32,

    // Nine-slice registry
    nine_slices: Vec<NineSliceMeta>,
    nine_slice_names: HashMap<String, NineSliceId>,

    // Decoded-image cache (path/key -> atlas sprite + dimensions)
    image_cache: ImageCache,

    // Vertex buffers (grow as needed)
    color_vbo: wgpu::Buffer,
    color_ibo: wgpu::Buffer,
    color_vbo_capacity: u64,
    color_ibo_capacity: u64,

    tex_vbo: wgpu::Buffer,
    tex_vbo_capacity: u64,

    // Text
    text_renderer: TextRenderer,

    // De-duplicated set of names we've already warned about — prevents log spam
    // when a missing sprite key is referenced every frame. RefCell because
    // tessellate_* take &self.
    warned_missing: RefCell<HashSet<String>>,
}

impl UiRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: FontSystemHandle,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        // Uniforms (bind group 0)
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui uniform buffer"),
            contents: bytemuck::cast_slice(&[Uniforms {
                view_proj: ortho_matrix(1.0, 1.0),
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui uniform bgl"),
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
            label: Some("ui uniform bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Atlas + texture
        let atlas = SpriteAtlas::new();
        let (texture, sampler, texture_bgl, texture_bind_group) =
            create_atlas_texture(device, atlas.width(), atlas.height());

        // Color pipeline (no texture binding)
        let color_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui color pipeline layout"),
                bind_group_layouts: &[&uniform_bgl],
                push_constant_ranges: &[],
            });

        let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui color pipeline"),
            layout: Some(&color_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_color"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &COLOR_VERTEX_ATTRIBS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_color"),
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

        // Textured pipeline (uses texture bind group at slot 1)
        let tex_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui tex pipeline layout"),
            bind_group_layouts: &[&uniform_bgl, &texture_bgl],
            push_constant_ranges: &[],
        });

        let tex_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui tex pipeline"),
            layout: Some(&tex_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_tex"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TexVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &TEX_VERTEX_ATTRIBS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_tex"),
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

        // Initial dynamic buffers — sized for a typical frame; grow on demand.
        let color_vbo_capacity = (4096 * std::mem::size_of::<Vertex>()) as u64;
        let color_ibo_capacity = (8192 * std::mem::size_of::<u32>()) as u64;
        let tex_vbo_capacity = (4096 * std::mem::size_of::<TexVertex>()) as u64;

        let color_vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui color vbo"),
            size: color_vbo_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let color_ibo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui color ibo"),
            size: color_ibo_capacity,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tex_vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui tex vbo"),
            size: tex_vbo_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut text_renderer =
            TextRenderer::with_font_system(device, queue, format, font_system);
        // Generate the printable-ASCII MSDF set up front so the first frame that
        // shows text doesn't hitch on per-glyph generation.
        text_renderer.prewarm_ascii(device, queue);

        let current_atlas_size = atlas.width();

        Self {
            color_pipeline,
            tex_pipeline,
            uniform_buffer,
            uniform_bind_group,
            atlas,
            texture,
            texture_bind_group_layout: texture_bgl,
            texture_bind_group,
            sampler,
            current_atlas_size,
            nine_slices: Vec::new(),
            nine_slice_names: HashMap::new(),
            image_cache: ImageCache::new(),
            color_vbo,
            color_ibo,
            color_vbo_capacity,
            color_ibo_capacity,
            tex_vbo,
            tex_vbo_capacity,
            text_renderer,
            warned_missing: RefCell::new(HashSet::new()),
        }
    }

    fn warn_missing(&self, kind: &str, key: &str) {
        let composite = format!("{kind}:{key}");
        let mut set = self.warned_missing.borrow_mut();
        if set.insert(composite) {
            log::warn!(
                "wgpu-gameui: {} '{}' referenced but not registered — draw skipped",
                kind,
                key
            );
        }
    }

    /// Load a sprite from raw RGBA8 bytes into the atlas.
    ///
    /// The pixels are buffered CPU-side; the atlas texture is re-uploaded lazily
    /// in [`UiRenderer::render`] (or eagerly via [`UiRenderer::flush_atlas`])
    /// so back-to-back loads coalesce into a single GPU upload.
    pub fn load_sprite_rgba8(
        &mut self,
        name: &str,
        w: u32,
        h: u32,
        pixels: &[u8],
    ) -> SpriteId {
        self.atlas.insert(Some(name), w, h, pixels)
    }

    /// Look up a sprite id by name.
    pub fn sprite_id(&self, name: &str) -> Option<SpriteId> {
        self.atlas.id_for(name)
    }

    /// Decode and load an encoded image (PNG/JPEG) from disk, returning an atlas
    /// sprite handle. Cached by path: a repeat load of the same path is free and
    /// returns the existing handle (no re-decode). Backs Teardown's `UiImage`.
    pub fn load_image_file(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<SpriteId, ImageError> {
        let key = path.as_ref().to_string_lossy().into_owned();
        if let Some(entry) = self.image_cache.get(&key) {
            return Ok(entry.sprite);
        }
        let bytes = std::fs::read(path.as_ref()).map_err(ImageError::Io)?;
        self.decode_and_cache(&key, &bytes)
    }

    /// Decode and load an encoded image from in-memory bytes under an explicit
    /// `key` (e.g. an `include_bytes!` asset). Cached by `key` like
    /// [`UiRenderer::load_image_file`].
    pub fn load_image_bytes(&mut self, key: &str, bytes: &[u8]) -> Result<SpriteId, ImageError> {
        if let Some(entry) = self.image_cache.get(key) {
            return Ok(entry.sprite);
        }
        self.decode_and_cache(key, bytes)
    }

    fn decode_and_cache(&mut self, key: &str, bytes: &[u8]) -> Result<SpriteId, ImageError> {
        let (w, h, rgba) = decode_rgba8(bytes).map_err(ImageError::Decode)?;
        let sprite = self.atlas.insert(Some(key), w, h, &rgba);
        self.image_cache.insert(
            key,
            ImageEntry {
                sprite,
                width: w,
                height: h,
            },
        );
        Ok(sprite)
    }

    /// Pixel dimensions of a loaded image, if `key` has been loaded. Backs
    /// Teardown's `UiGetImageSize`.
    pub fn image_size(&self, key: &str) -> Option<(u32, u32)> {
        self.image_cache.get(key).map(|e| (e.width, e.height))
    }

    /// Whether an image `key` has been loaded. Backs Teardown's `UiHasImage`.
    pub fn has_image(&self, key: &str) -> bool {
        self.image_cache.contains(key)
    }

    /// Drop the cache entry for an image `key` (next load re-decodes). Backs
    /// Teardown's `UiUnloadImage`. Note: the atlas slot is not reclaimed.
    pub fn unload_image(&mut self, key: &str) {
        self.image_cache.remove(key);
    }

    /// Register a nine-slice resource referencing an existing sprite.
    pub fn register_nine_slice(
        &mut self,
        name: &str,
        sprite: SpriteId,
        border: [u32; 4],
    ) -> NineSliceId {
        let id = self.nine_slices.len() as NineSliceId;
        self.nine_slices.push(NineSliceMeta { sprite, border });
        self.nine_slice_names.insert(name.to_string(), id);
        id
    }

    pub fn nine_slice_id(&self, name: &str) -> Option<NineSliceId> {
        self.nine_slice_names.get(name).copied()
    }

    /// Notify the text sub-renderer of viewport changes.
    pub fn resize(&mut self, _queue: &wgpu::Queue, width: u32, height: u32) {
        self.text_renderer.resize(width, height);
    }

    /// Force-upload pending atlas changes to the GPU. Called automatically by
    /// `render()`, exposed for callers that want to control timing.
    pub fn flush_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_flush_atlas").entered();
        if self.atlas.width() != self.current_atlas_size {
            self.texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ui atlas texture"),
                size: wgpu::Extent3d {
                    width: self.atlas.width(),
                    height: self.atlas.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.texture_bind_group = self.create_texture_bg(device);
            self.current_atlas_size = self.atlas.width();
            // Force a full upload after grow.
            let _ = self.atlas.take_dirty();
            self.upload_atlas_pixels(queue);
        } else if self.atlas.take_dirty() {
            self.upload_atlas_pixels(queue);
        }
    }

    fn create_texture_bg(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        let view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui atlas bg"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn upload_atlas_pixels(&self, queue: &wgpu::Queue) {
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

    /// Render the entire DrawList in one call.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
        draw_list: &DrawList,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render").entered();
        self.prepare_frame(device, queue, viewport);
        self.render_one(device, queue, encoder, view, draw_list);
    }

    /// Render a `LayerStack`: base list first, then each layer in push order.
    /// Each layer goes through the full 4-pass pipeline so a higher-z layer's
    /// quads correctly overlap a lower-z layer's text/icons.
    pub fn render_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
        layers: &LayerStack,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render_layers").entered();
        self.prepare_frame(device, queue, viewport);
        self.render_one(device, queue, encoder, view, layers.base());
        for layer in layers.layers() {
            self.render_one(device, queue, encoder, view, &layer.list);
        }
    }

    fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: (u32, u32),
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_prepare_frame").entered();
        let uniforms = Uniforms {
            view_proj: ortho_matrix(viewport.0 as f32, viewport.1 as f32),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
        self.text_renderer.resize(viewport.0, viewport.1);
        self.flush_atlas(device, queue);
    }

    fn render_one(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        draw_list: &DrawList,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render_one").entered();

        // Layering, bottom→top: nine-slice backgrounds, colored quads (panels,
        // rounded-rect fills, sliders, custom shapes), icons, text. Each
        // requires its own pass because consecutive layers swap pipelines or
        // change vertex formats.

        // ---------- 1. Nine-slices ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_nine_slices").entered();
            let nine_slice_verts = self.tessellate_nine_slices(&draw_list.nine_slices);
            if !nine_slice_verts.is_empty() {
                self.draw_textured(device, queue, encoder, view, &nine_slice_verts);
            }
        }

        // ---------- 2. Colored quads ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_color_quads").entered();
            if !draw_list.vertices.is_empty() && !draw_list.indices.is_empty() {
                self.draw_color(
                    device,
                    queue,
                    encoder,
                    view,
                    &draw_list.vertices,
                    &draw_list.indices,
                );
            }
        }

        // ---------- 3. Icons ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_icons").entered();
            let icon_verts = self.tessellate_icons(&draw_list.icons);
            if !icon_verts.is_empty() {
                self.draw_textured(device, queue, encoder, view, &icon_verts);
            }
        }

        // ---------- 4. Text ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_text").entered();
            self.text_renderer
                .render(device, queue, encoder, view, &draw_list.texts);
        }
    }

    fn tessellate_icons(&self, icons: &[IconDraw]) -> Vec<TexVertex> {
        let aw = self.atlas.width();
        let ah = self.atlas.height();
        let mut out: Vec<TexVertex> = Vec::with_capacity(icons.len() * 6);

        for icon in icons {
            let id = match icon.sprite {
                Some(id) => id,
                None => match self.atlas.id_for(&icon.icon_key) {
                    Some(id) => id,
                    None => {
                        self.warn_missing("sprite", &icon.icon_key);
                        continue;
                    }
                },
            };
            let region = match self.atlas.region(id) {
                Some(r) => r,
                None => continue,
            };
            let uv = apply_crop_uv(region.uv(aw, ah), icon.src);
            let clip = icon.clip.map(|c| [c.x, c.y, c.width, c.height]);

            push_textured_quad_corners(
                &mut out,
                icon.corners,
                [uv[0], uv[1], uv[2], uv[3]],
                icon.tint,
                clip,
            );
        }

        out
    }

    fn tessellate_nine_slices(&self, draws: &[NineSliceDraw]) -> Vec<TexVertex> {
        let aw = self.atlas.width();
        let ah = self.atlas.height();
        let mut out: Vec<TexVertex> = Vec::with_capacity(draws.len() * 54);

        for draw in draws {
            let id = match draw.nine_slice {
                Some(id) => id,
                None => match self.nine_slice_names.get(&draw.texture_key) {
                    Some(id) => *id,
                    None => {
                        self.warn_missing("nine-slice", &draw.texture_key);
                        continue;
                    }
                },
            };
            let meta = match self.nine_slices.get(id as usize) {
                Some(m) => m,
                None => continue,
            };
            let region = match self.atlas.region(meta.sprite) {
                Some(r) => r,
                None => continue,
            };

            tessellate_nine_slice(
                &mut out,
                draw.local.x,
                draw.local.y,
                draw.local.width,
                draw.local.height,
                &draw.transform,
                draw.tint,
                draw.clip.map(|c| [c.x, c.y, c.width, c.height]),
                region,
                meta.border,
                aw,
                ah,
            );
        }

        out
    }

    fn ensure_color_capacity(
        &mut self,
        device: &wgpu::Device,
        verts: usize,
        indices: usize,
    ) {
        let needed_v = (verts * std::mem::size_of::<Vertex>()) as u64;
        if needed_v > self.color_vbo_capacity {
            self.color_vbo_capacity = needed_v.next_power_of_two();
            self.color_vbo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui color vbo"),
                size: self.color_vbo_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let needed_i = (indices * std::mem::size_of::<u32>()) as u64;
        if needed_i > self.color_ibo_capacity {
            self.color_ibo_capacity = needed_i.next_power_of_two();
            self.color_ibo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui color ibo"),
                size: self.color_ibo_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    fn ensure_tex_capacity(&mut self, device: &wgpu::Device, verts: usize) {
        let needed = (verts * std::mem::size_of::<TexVertex>()) as u64;
        if needed > self.tex_vbo_capacity {
            self.tex_vbo_capacity = needed.next_power_of_two();
            self.tex_vbo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui tex vbo"),
                size: self.tex_vbo_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    fn draw_color(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        verts: &[Vertex],
        indices: &[u32],
    ) {
        self.ensure_color_capacity(device, verts.len(), indices.len());
        queue.write_buffer(&self.color_vbo, 0, bytemuck::cast_slice(verts));
        queue.write_buffer(&self.color_ibo, 0, bytemuck::cast_slice(indices));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui color pass"),
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
        pass.set_pipeline(&self.color_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.color_vbo.slice(..));
        pass.set_index_buffer(self.color_ibo.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
    }

    fn draw_textured(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        verts: &[TexVertex],
    ) {
        self.ensure_tex_capacity(device, verts.len());
        queue.write_buffer(&self.tex_vbo, 0, bytemuck::cast_slice(verts));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui tex pass"),
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
        pass.set_pipeline(&self.tex_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.texture_bind_group, &[]);
        pass.set_vertex_buffer(0, self.tex_vbo.slice(..));
        pass.draw(0..verts.len() as u32, 0..1);
    }
}

fn create_atlas_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (
    wgpu::Texture,
    wgpu::Sampler,
    wgpu::BindGroupLayout,
    wgpu::BindGroup,
) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui atlas texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ui atlas bgl"),
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
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ui atlas bg"),
        layout: &bgl,
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
    (texture, sampler, bgl, bg)
}

pub(crate) fn ortho_matrix(width: f32, height: f32) -> [[f32; 4]; 4] {
    // Top-left origin; positive Y down. Matches DrawList coordinate system.
    let (l, r, t, b) = (0.0, width, 0.0, height);
    [
        [2.0 / (r - l), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (t - b), 0.0, 0.0],
        [0.0, 0.0, 0.5, 0.0],
        [-(r + l) / (r - l), -(t + b) / (t - b), 0.5, 1.0],
    ]
}

/// Resolve an optional normalized within-sprite crop against a sprite's full
/// atlas UV rect `[u0, v0, u1, v1]`. `src` components are 0..1 fractions of the
/// sprite; `None` returns the full rect unchanged.
fn apply_crop_uv(full: [f32; 4], src: Option<[f32; 4]>) -> [f32; 4] {
    match src {
        Some([u0, v0, u1, v1]) => {
            let span_u = full[2] - full[0];
            let span_v = full[3] - full[1];
            [
                full[0] + u0 * span_u,
                full[1] + v0 * span_v,
                full[0] + u1 * span_u,
                full[1] + v1 * span_v,
            ]
        }
        None => full,
    }
}

/// Emit two triangles for a quad whose 4 corners are given in TL, TR, BR, BL
/// order (matching `Affine2::transform_rect_corners`). Used for icons that may
/// have been rotated/scaled by the active transform.
fn push_textured_quad_corners(
    out: &mut Vec<TexVertex>,
    corners: [[f32; 2]; 4],
    uv: [f32; 4], // u0, v0, u1, v1
    tint: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
    let tl = corners[0];
    let tr = corners[1];
    let br = corners[2];
    let bl = corners[3];

    out.push(TexVertex::new(tl, [u0, v0], tint, clip));
    out.push(TexVertex::new(tr, [u1, v0], tint, clip));
    out.push(TexVertex::new(br, [u1, v1], tint, clip));

    out.push(TexVertex::new(br, [u1, v1], tint, clip));
    out.push(TexVertex::new(bl, [u0, v1], tint, clip));
    out.push(TexVertex::new(tl, [u0, v0], tint, clip));
}

#[allow(clippy::too_many_arguments)]
fn tessellate_nine_slice(
    out: &mut Vec<TexVertex>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    transform: &crate::affine::Affine2,
    tint: [f32; 4],
    clip: Option<[f32; 4]>,
    region: crate::render::AtlasRegion,
    border: [u32; 4],
    atlas_w: u32,
    atlas_h: u32,
) {
    let bl = border[0] as f32;
    let bt = border[1] as f32;
    let br = border[2] as f32;
    let bb = border[3] as f32;

    // Screen X columns: x0..x1 = left border, x2..x3 = right border.
    let x0 = x;
    let mut x1 = x + bl;
    let mut x2 = x + w - br;
    let x3 = x + w;
    if x1 > x2 {
        // Panel narrower than borders: collapse middle column.
        let mid = (x1 + x2) * 0.5;
        x1 = mid;
        x2 = mid;
    }

    let y0 = y;
    let mut y1 = y + bt;
    let mut y2 = y + h - bb;
    let y3 = y + h;
    if y1 > y2 {
        let mid = (y1 + y2) * 0.5;
        y1 = mid;
        y2 = mid;
    }

    // UVs: convert source-pixel borders against atlas dimensions.
    let aw = atlas_w as f32;
    let ah = atlas_h as f32;
    let u0 = region.x as f32 / aw;
    let u1 = (region.x as f32 + bl) / aw;
    let u2 = ((region.x + region.w) as f32 - br) / aw;
    let u3 = (region.x + region.w) as f32 / aw;
    let v0 = region.y as f32 / ah;
    let v1 = (region.y as f32 + bt) / ah;
    let v2 = ((region.y + region.h) as f32 - bb) / ah;
    let v3 = (region.y + region.h) as f32 / ah;

    let xs = [x0, x1, x2, x3];
    let ys = [y0, y1, y2, y3];
    let us = [u0, u1, u2, u3];
    let vs = [v0, v1, v2, v3];

    for row in 0..3 {
        for col in 0..3 {
            let px0 = xs[col];
            let px1 = xs[col + 1];
            let py0 = ys[row];
            let py1 = ys[row + 1];

            if (px1 - px0).abs() < 0.001 || (py1 - py0).abs() < 0.001 {
                continue;
            }

            let tu0 = us[col];
            let tu1 = us[col + 1];
            let tv0 = vs[row];
            let tv1 = vs[row + 1];

            let tl = transform.transform_point([px0, py0]);
            let tr = transform.transform_point([px1, py0]);
            let br = transform.transform_point([px1, py1]);
            let bl = transform.transform_point([px0, py1]);

            out.push(TexVertex::new(tl, [tu0, tv0], tint, clip));
            out.push(TexVertex::new(tr, [tu1, tv0], tint, clip));
            out.push(TexVertex::new(br, [tu1, tv1], tint, clip));

            out.push(TexVertex::new(br, [tu1, tv1], tint, clip));
            out.push(TexVertex::new(bl, [tu0, tv1], tint, clip));
            out.push(TexVertex::new(tl, [tu0, tv0], tint, clip));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::AtlasRegion;

    fn approx4(a: [f32; 4], b: [f32; 4]) {
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-5, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn crop_uv_none_is_identity() {
        let full = [0.25, 0.5, 0.75, 1.0];
        approx4(apply_crop_uv(full, None), full);
    }

    #[test]
    fn crop_uv_maps_into_region() {
        // Sprite occupies [0.2,0.4]..[0.6,0.8] of the atlas. Crop its centre
        // quarter [0.25,0.25]..[0.75,0.75] -> a centred sub-rect of the region.
        let full = [0.2, 0.4, 0.6, 0.8];
        let got = apply_crop_uv(full, Some([0.25, 0.25, 0.75, 0.75]));
        // span_u = 0.4, span_v = 0.4
        approx4(got, [0.3, 0.5, 0.5, 0.7]);
    }

    #[test]
    fn nine_slice_emits_nine_quads() {
        let mut out = Vec::new();
        let region = AtlasRegion {
            x: 0,
            y: 0,
            w: 32,
            h: 32,
        };
        let identity = crate::affine::Affine2::IDENTITY;
        tessellate_nine_slice(
            &mut out,
            0.0,
            0.0,
            100.0,
            80.0,
            &identity,
            [1.0; 4],
            None,
            region,
            [4, 4, 4, 4],
            64,
            64,
        );
        // 9 quads, 6 vertices each
        assert_eq!(out.len(), 54);
    }

    #[test]
    fn nine_slice_corner_uvs_match_border() {
        let mut out = Vec::new();
        let region = AtlasRegion {
            x: 0,
            y: 0,
            w: 32,
            h: 32,
        };
        let identity = crate::affine::Affine2::IDENTITY;
        tessellate_nine_slice(
            &mut out,
            0.0,
            0.0,
            100.0,
            80.0,
            &identity,
            [1.0; 4],
            None,
            region,
            [8, 8, 8, 8],
            64,
            64,
        );
        // First triangle of first quad (top-left corner). Its UV at (0,0) must
        // be region origin; second vertex (right edge of TL corner) must match
        // border-left UV: 8/64 = 0.125.
        let first = out[0];
        assert_eq!(first.uv, [0.0, 0.0]);
        let second = out[1];
        assert!((second.uv[0] - 8.0 / 64.0).abs() < 1e-6);
        assert!((second.uv[1] - 0.0).abs() < 1e-6);
    }
}
