//! `UiRenderer` — single-call renderer consuming a `DrawList`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::TextRenderer;
use crate::layer::LayerStack;
use crate::layout::Rect;
use crate::render::atlas::{SpriteAtlas, SpriteId};
use crate::render::blur::{Backdrop, Blur, BlurParams};
use crate::render::image_cache::{ImageCache, ImageEntry, ImageError, decode_rgba8};
use crate::text::FontSystemHandle;
use crate::widgets::{
    ChromeInstance, CircleInstance, ColorCmd, DrawList, IconDraw, NineSliceDraw, NineSliceId,
    Vertex,
};

const SHADER: &str = include_str!("ui.wgsl");

/// Metadata describing a registered nine-slice resource.
#[derive(Clone, Debug)]
pub struct NineSliceMeta {
    /// Atlas sprite the nine-slice samples from.
    pub sprite: SpriteId,
    /// Border insets in source pixels: [left, top, right, bottom].
    pub border: [u32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

/// Per-instance icon/image record — matches `vs_icon` in `ui.wgsl`.
///
/// Icons, sprites, and cropped images all flow through this path. The four
/// world-space corners are baked in (the `DrawList` already applied the active
/// transform), so the vertex shader bilinearly interpolates them — rotation,
/// scale, and shear are handled with no fallback. Replaces re-tessellating 6
/// verts/icon into the textured soup + re-uploading it every frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
struct IconInstance {
    /// `[tl.x, tl.y, tr.x, tr.y]` (world space).
    c_tl_tr: [f32; 4],
    /// `[br.x, br.y, bl.x, bl.y]` (world space).
    c_br_bl: [f32; 4],
    /// Source UV rect `[u0, v0, u1, v1]`.
    uv_rect: [f32; 4],
    /// Tint (multiplied with the sampled texel).
    tint: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `flags[0] > 0.5`).
    clip: [f32; 4],
    /// `[clip_enabled, _pad, _pad, _pad]`.
    flags: [f32; 4],
}

/// Per-instance icon attributes — matches [`IconInstance`] / `vs_icon`
/// (location 0 is the base-mesh corner).
const ICON_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    1 => Float32x4, // c_tl_tr
    2 => Float32x4, // c_br_bl
    3 => Float32x4, // uv_rect
    4 => Float32x4, // tint
    5 => Float32x4, // clip
    6 => Float32x4, // flags
];

const COLOR_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32,
];

/// Base unit-quad vertex (one attribute: the corner in `[0,1]²`).
const CHROME_BASE_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,
];

/// Per-instance chrome attributes — matches [`ChromeInstance`] / `vs_chrome`.
const CHROME_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    1 => Float32x4, // rect
    2 => Float32x4, // bg
    3 => Float32x4, // border
    4 => Float32x4, // clip
    5 => Float32x4, // params (radius, thickness, clip_enabled, _pad)
];

/// Per-instance circle attributes — matches [`CircleInstance`] / `vs_circle`
/// (location 0 is the base-mesh corner).
const CIRCLE_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    1 => Float32x4, // center (cx, cy, radius, thickness)
    2 => Float32x4, // color
    3 => Float32x4, // clip
    4 => Float32x4, // params (clip_enabled, _, _, _)
];

/// Unit-quad corners (TL, TR, BR, BL) for the chrome base mesh.
const CHROME_BASE_VERTS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
/// Two triangles for the unit quad above.
const CHROME_BASE_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// Per-instance nine-slice record — matches `vs_nine_slice` in `ui.wgsl`.
///
/// Unlike [`ChromeInstance`] (built in the `DrawList`), this is built in the
/// renderer because it needs the registered nine-slice's atlas region + border,
/// which the `DrawList` doesn't know. The full affine is baked in (`lin` +
/// `translate` linear/translation parts) and applied to the local corner in the
/// vertex shader, so rotated/scaled nine-slices need no fallback — every panel,
/// transformed or not, is one instance. Replaces re-tessellating 54 verts/panel
/// into the textured soup each frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
struct NineSliceInstance {
    /// Affine linear part `[a, b, c, d]` (row-major, from [`Affine2`]).
    lin: [f32; 4],
    /// `[tx, ty, clip_enabled, _pad]`.
    translate: [f32; 4],
    /// Local-space `[x, y, w, h]` (pre-transform).
    origin_size: [f32; 4],
    /// Outer UV edges `[u0, v0, u3, v3]`.
    uv_outer: [f32; 4],
    /// Inner border-seam UVs `[u1, v1, u2, v2]`.
    uv_inner: [f32; 4],
    /// Border widths in screen px `[left, top, right, bottom]`.
    border: [f32; 4],
    /// Tint (multiplied with the sampled texel).
    tint: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `translate[2] > 0.5`).
    clip: [f32; 4],
}

/// Per-instance nine-slice attributes — matches [`NineSliceInstance`] /
/// `vs_nine_slice` (location 0 is the base-mesh corner).
const NINE_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    1 => Float32x4, // lin
    2 => Float32x4, // translate
    3 => Float32x4, // origin_size
    4 => Float32x4, // uv_outer
    5 => Float32x4, // uv_inner
    6 => Float32x4, // border
    7 => Float32x4, // tint
    8 => Float32x4, // clip
];

/// Detects the "freshly-constructed `DrawList`/`LayerStack` every frame" footgun.
///
/// The text-measure cache (and the shaped-glyph cache) lives **on the
/// `DrawList`**. A caller that builds a new list every frame — e.g.
/// `LayerStack::new()` inside the render loop — throws that cache away each frame,
/// so every label is re-shaped through glyphon on every frame. That is invisible
/// in correctness terms but can cost *milliseconds* (it made one HUD cost ~11ms a
/// frame instead of <1ms). The fix is always the same: build one list/stack once
/// and reuse it (`clear()` resets geometry while keeping the warm cache).
///
/// We can't see the cache directly from the renderer, but we can see identity:
/// each [`DrawList`] carries a unique, never-reused [`id`](DrawList::id). A reused
/// list reports the **same** id every frame; a per-frame-fresh list reports a
/// **brand-new** id every frame. So: remember the handful of ids seen recently,
/// and if the renderer is fed an id it has *never* seen for many consecutive
/// frames, the caller is rebuilding every frame — warn (once).
///
/// Patterns this deliberately does **not** flag: a single reused list (same id
/// forever), a small set of double-buffered/persistent stacks cycled round-robin
/// (their ids keep recurring), and the occasional rebuild on resize (a single
/// fresh id now and then never builds a long streak).
#[derive(Default)]
struct StaleListDetector {
    /// Bounded ring of recently observed `DrawList` ids (most-recent last). Large
    /// enough to cover any realistic set of persistent/double-buffered stacks.
    recent: VecDeque<u64>,
    /// Consecutive frames whose id had never been seen before.
    fresh_streak: u32,
    /// Latch: warn at most once per renderer (the advice is the same every time).
    warned: bool,
}

impl StaleListDetector {
    /// How many distinct recent ids to remember. Comfortably exceeds any sane
    /// number of persistent stacks an app cycles between.
    const RECENT_CAP: usize = 16;
    /// Consecutive all-fresh frames before we conclude the caller rebuilds every
    /// frame. ~2s at 60fps — long enough that brief transients never trip it.
    const WARN_AFTER: u32 = 120;

    /// Record one rendered list id. Returns `true` exactly once — on the frame the
    /// footgun is first detected — so the caller can emit the warning.
    fn observe(&mut self, id: u64) -> bool {
        if self.recent.contains(&id) {
            // Seen before → the list is being reused. Reset the streak.
            self.fresh_streak = 0;
        } else {
            self.fresh_streak = self.fresh_streak.saturating_add(1);
            self.recent.push_back(id);
            if self.recent.len() > Self::RECENT_CAP {
                self.recent.pop_front();
            }
        }
        if !self.warned && self.fresh_streak >= Self::WARN_AFTER {
            self.warned = true;
            return true;
        }
        false
    }
}

/// Public renderer.
pub struct UiRenderer {
    // Pipelines
    color_pipeline: wgpu::RenderPipeline,
    icon_pipeline: wgpu::RenderPipeline,
    chrome_pipeline: wgpu::RenderPipeline,
    circle_pipeline: wgpu::RenderPipeline,
    nine_slice_pipeline: wgpu::RenderPipeline,

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

    // Vertex buffers (grow as needed). Each `draw_*` call bump-allocates its
    // slice at the running `*_offset` and advances it, so the many draws issued
    // per submit (nine-slices + icons per layer, every layer) occupy disjoint
    // regions instead of all aliasing offset 0 and reading the last write at
    // draw time. Offsets reset to 0 each frame in `prepare_frame`.
    color_vbo: wgpu::Buffer,
    color_ibo: wgpu::Buffer,
    color_vbo_capacity: u64,
    color_ibo_capacity: u64,
    color_vbo_offset: u64,
    color_ibo_offset: u64,

    // Instanced icons: reuses the chrome unit-quad base mesh + a growing
    // per-frame instance buffer (bump offset, reset in `prepare_frame`).
    icon_inst_buffer: wgpu::Buffer,
    icon_inst_capacity: u64,
    icon_inst_offset: u64,

    // Instanced chrome: a persistent unit-quad base mesh + a growing per-frame
    // instance buffer (bump offset like the others, reset in `prepare_frame`).
    chrome_base_vbo: wgpu::Buffer,
    chrome_base_ibo: wgpu::Buffer,
    chrome_inst_buffer: wgpu::Buffer,
    chrome_inst_capacity: u64,
    chrome_inst_offset: u64,

    // Instanced circles: reuses the chrome unit-quad base mesh + a growing
    // per-frame instance buffer (bump offset, reset in `prepare_frame`).
    circle_inst_buffer: wgpu::Buffer,
    circle_inst_capacity: u64,
    circle_inst_offset: u64,

    // Instanced nine-slice: reuses the chrome unit-quad base mesh + a growing
    // per-frame instance buffer (bump offset, reset in `prepare_frame`).
    nine_inst_buffer: wgpu::Buffer,
    nine_inst_capacity: u64,
    nine_inst_offset: u64,

    // Target color format (used for the lazily-built backdrop-blur pipeline).
    format: wgpu::TextureFormat,
    // Backdrop blur — built on first `blur_backdrop` call so `new` is unchanged.
    blur: Option<Blur>,

    // Text
    text_renderer: TextRenderer,

    // De-duplicated set of names we've already warned about — prevents log spam
    // when a missing sprite key is referenced every frame. RefCell because
    // tessellate_* take &self.
    warned_missing: RefCell<HashSet<String>>,

    // Detects callers that rebuild their DrawList/LayerStack every frame (cold
    // text-measure cache → per-frame reshaping). Fed the rendered list's id in
    // `render`/`render_layers`; warns once if it's always a brand-new list.
    stale_list: StaleListDetector,
}

impl UiRenderer {
    /// Build the renderer: compiles the UI shader, creates the pipelines,
    /// atlases, and bind groups, and wires up the text sub-renderer.
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

        // Icon (instanced textured-quad) pipeline. Shares the ortho uniform
        // (group 0) AND the atlas texture (group 1). Two vertex buffers: the
        // chrome unit-quad base mesh + per-instance records.
        let icon_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui icon pipeline layout"),
            bind_group_layouts: &[&uniform_bgl, &texture_bgl],
            push_constant_ranges: &[],
        });

        let icon_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui icon pipeline"),
            layout: Some(&icon_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_icon"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &CHROME_BASE_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<IconInstance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ICON_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_icon"),
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

        // Chrome (instanced SDF rounded-rect) pipeline. Shares the ortho uniform
        // (group 0); no texture. Two vertex buffers: unit-quad base + instances.
        let chrome_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui chrome pipeline layout"),
                bind_group_layouts: &[&uniform_bgl],
                push_constant_ranges: &[],
            });

        let chrome_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui chrome pipeline"),
            layout: Some(&chrome_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_chrome"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &CHROME_BASE_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ChromeInstance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &CHROME_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_chrome"),
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

        // Circle (instanced SDF disc/ring) pipeline. Same shape as chrome —
        // ortho uniform (group 0), no texture, unit-quad base + instances — but
        // its fragment computes a circle SDF instead of a rounded-rect one.
        let circle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui circle pipeline"),
            layout: Some(&chrome_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_circle"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &CHROME_BASE_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<CircleInstance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &CIRCLE_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_circle"),
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

        // Nine-slice (instanced) pipeline. Shares the ortho uniform (group 0)
        // AND the atlas texture (group 1, like the textured path). Two vertex
        // buffers: the chrome unit-quad base mesh + per-instance records.
        let nine_slice_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui nine-slice pipeline layout"),
                bind_group_layouts: &[&uniform_bgl, &texture_bgl],
                push_constant_ranges: &[],
            });

        let nine_slice_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui nine-slice pipeline"),
            layout: Some(&nine_slice_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_nine_slice"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &CHROME_BASE_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<NineSliceInstance>()
                            as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &NINE_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_nine_slice"),
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

        let chrome_base_vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui chrome base vbo"),
            contents: bytemuck::cast_slice(&CHROME_BASE_VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let chrome_base_ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui chrome base ibo"),
            contents: bytemuck::cast_slice(&CHROME_BASE_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Initial dynamic buffers — sized for a typical frame; grow on demand.
        let color_vbo_capacity = (4096 * std::mem::size_of::<Vertex>()) as u64;
        let color_ibo_capacity = (8192 * std::mem::size_of::<u32>()) as u64;
        let icon_inst_capacity = (1024 * std::mem::size_of::<IconInstance>()) as u64;

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
        let icon_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui icon inst buffer"),
            size: icon_inst_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut text_renderer = TextRenderer::with_font_system(device, queue, format, font_system);
        // Generate the printable-ASCII MSDF set up front so the first frame that
        // shows text doesn't hitch on per-glyph generation.
        text_renderer.prewarm_ascii(device, queue);
        // Same for the curated Phosphor icon set.
        #[cfg(feature = "phosphor-icons")]
        text_renderer.prewarm_icons(device, queue);

        let current_atlas_size = atlas.width();

        let chrome_inst_capacity = (1024 * std::mem::size_of::<ChromeInstance>()) as u64;
        let chrome_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui chrome inst buffer"),
            size: chrome_inst_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let nine_inst_capacity = (256 * std::mem::size_of::<NineSliceInstance>()) as u64;
        let nine_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui nine-slice inst buffer"),
            size: nine_inst_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let circle_inst_capacity = (1024 * std::mem::size_of::<CircleInstance>()) as u64;
        let circle_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui circle inst buffer"),
            size: circle_inst_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            color_pipeline,
            icon_pipeline,
            chrome_pipeline,
            circle_pipeline,
            nine_slice_pipeline,
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
            color_vbo_offset: 0,
            color_ibo_offset: 0,
            icon_inst_buffer,
            icon_inst_capacity,
            icon_inst_offset: 0,
            chrome_base_vbo,
            chrome_base_ibo,
            chrome_inst_buffer,
            chrome_inst_capacity,
            chrome_inst_offset: 0,
            circle_inst_buffer,
            circle_inst_capacity,
            circle_inst_offset: 0,
            nine_inst_buffer,
            nine_inst_capacity,
            nine_inst_offset: 0,
            format,
            blur: None,
            text_renderer,
            warned_missing: RefCell::new(HashSet::new()),
            stale_list: StaleListDetector::default(),
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
    pub fn load_sprite_rgba8(&mut self, name: &str, w: u32, h: u32, pixels: &[u8]) -> SpriteId {
        self.atlas.insert(Some(name), w, h, pixels)
    }

    /// Look up a sprite id by name.
    pub fn sprite_id(&self, name: &str) -> Option<SpriteId> {
        self.atlas.id_for(name)
    }

    /// True if `key` resolves to a sprite already present in the atlas — a
    /// loaded image (`load_image_*`), an out-of-band sprite
    /// (`load_sprite_rgba8`), or any registered name. Broader than
    /// [`UiRenderer::has_image`] (which only sees the decoded-image cache); a
    /// caller draining deferred image-load requests uses this to skip keys that
    /// are already drawable (so it never tries to `fs::read` an out-of-band key).
    pub fn has_sprite(&self, key: &str) -> bool {
        self.atlas.id_for(key).is_some()
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
        let (w, h, rgba) = decode_rgba8(&bytes).map_err(ImageError::Decode)?;
        Ok(self.insert_image(&key, w, h, &rgba))
    }

    /// Decode and load an encoded image from in-memory bytes under an explicit
    /// `key` (e.g. an `include_bytes!` asset). Cached by `key` like
    /// [`UiRenderer::load_image_file`].
    pub fn load_image_bytes(&mut self, key: &str, bytes: &[u8]) -> Result<SpriteId, ImageError> {
        if let Some(entry) = self.image_cache.get(key) {
            return Ok(entry.sprite);
        }
        let (w, h, rgba) = decode_rgba8(bytes).map_err(ImageError::Decode)?;
        Ok(self.insert_image(key, w, h, &rgba))
    }

    /// Load an already-decoded RGBA8 image under an explicit `key`, skipping the
    /// decode step entirely. This is the decode-free sibling of
    /// [`UiRenderer::load_image_bytes`]: use it when the caller already holds the
    /// raw pixels (e.g. a notification daemon that rendered an icon into a
    /// buffer) to avoid a pointless RGBA→PNG→decode round-trip.
    ///
    /// Like the other `load_image_*` methods, the image is registered in the
    /// decoded-image cache, so [`has_image`](Self::has_image) /
    /// [`image_size`](Self::image_size) /
    /// [`unload_image`](Self::unload_image) all see it. (Contrast
    /// [`load_sprite_rgba8`](Self::load_sprite_rgba8), which registers only the
    /// atlas name and bypasses the cache.) Cached by `key`: a repeat load is free
    /// and returns the existing handle.
    pub fn load_image_rgba8(&mut self, key: &str, w: u32, h: u32, rgba: &[u8]) -> SpriteId {
        if let Some(entry) = self.image_cache.get(key) {
            return entry.sprite;
        }
        self.insert_image(key, w, h, rgba)
    }

    /// Insert decoded RGBA8 pixels into the atlas and record the cache entry.
    fn insert_image(&mut self, key: &str, w: u32, h: u32, rgba: &[u8]) -> SpriteId {
        let sprite = self.atlas.insert(Some(key), w, h, rgba);
        self.image_cache.insert(
            key,
            ImageEntry {
                sprite,
                width: w,
                height: h,
            },
        );
        sprite
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

    /// Drop the cache entry for an image `key` (next load re-decodes), and free
    /// the sprite's atlas slot so its pixels are reclaimed. Backs Teardown's
    /// `UiUnloadImage`.
    ///
    /// The freed slot is tombstoned and recycled by a later `load_*`; the
    /// GPU texture is re-uploaded without this sprite's pixels on the next
    /// render. Shelf *fragmentation* left by the removal is reclaimed lazily by
    /// [`compact_atlas`](Self::compact_atlas), which a long-running app can call
    /// on its own schedule (e.g. when [`atlas_size`](Self::atlas_size) approaches
    /// a threshold). Idempotent for an unknown key.
    pub fn unload_image(&mut self, key: &str) {
        if let Some(entry) = self.image_cache.remove(key) {
            self.atlas.remove(entry.sprite);
        }
    }

    /// Reclaim atlas shelf fragmentation left by [`unload_image`](Self::unload_image)
    /// / [`remove`](SpriteAtlas::remove) calls, repacking every live sprite into
    /// fresh contiguous shelves without changing any `SpriteId`. Safe to call at
    /// any time; idempotent when nothing has been removed.
    ///
    /// The texture dimensions are not shrunk (only the *packing* is tightened),
    /// so this prevents the atlas from climbing toward the 4096² cap under churn
    /// but does not release the peak texture size. Call it periodically — e.g. a
    /// daemon that loads and discards many one-off icons — to keep growth bounded.
    pub fn compact_atlas(&mut self) {
        self.atlas.compact();
    }

    /// Current atlas texture dimensions in pixels, as `(width, height)`. Monitor
    /// this to decide when to call [`compact_atlas`](Self::compact_atlas); the
    /// atlas grows (1024 → 2048 → 4096) only when a sprite doesn't fit, and
    /// panics past 4096².
    pub fn atlas_size(&self) -> (u32, u32) {
        (self.atlas.width(), self.atlas.height())
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

    /// Look up a registered nine-slice id by the name it was registered under.
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
        let view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
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
    /// Render a single `DrawList`.
    ///
    /// `viewport` is the **physical** render-target size in pixels. `scale_factor`
    /// is the display's logical→physical ratio (e.g. `1.0` on a standard display,
    /// `2.0` on a Retina/HiDPI output — pass the window's scale factor). The UI is
    /// laid out in **logical** pixels; the renderer divides the physical viewport
    /// by `scale_factor` when building its projection, so geometry stays the same
    /// logical size while text rasterizes against the higher-resolution
    /// framebuffer (the MSDF path sharpens automatically). Pass `1.0` to disable.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
        scale_factor: f32,
        draw_list: &DrawList,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render").entered();
        if self.stale_list.observe(draw_list.id()) {
            Self::warn_stale_list("render", "DrawList");
        }
        self.prepare_frame(device, queue, viewport, scale_factor);
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
        scale_factor: f32,
        layers: &LayerStack,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render_layers").entered();
        // Key the footgun detector on the BASE list only: modal/popup/tooltip
        // layers are legitimately transient (built per push), so their ids would
        // be false positives.
        if self.stale_list.observe(layers.base_id()) {
            Self::warn_stale_list("render_layers", "LayerStack");
        }
        self.prepare_frame(device, queue, viewport, scale_factor);
        self.render_one(device, queue, encoder, view, layers.base());
        for layer in layers.layers() {
            self.render_one(device, queue, encoder, view, &layer.list);
        }
    }

    /// Emit the once-per-renderer warning that the caller is feeding a
    /// freshly-constructed list/stack every frame (see [`StaleListDetector`]).
    fn warn_stale_list(method: &str, kind: &str) {
        log::warn!(
            "wgpu-gameui: `UiRenderer::{method}` has been called with a \
             freshly-constructed `{kind}` for {}+ consecutive frames. The \
             text-measure and shaped-glyph caches live ON the `DrawList`, so a \
             list rebuilt every frame can never warm them — every label is \
             re-shaped through glyphon every frame (this can cost milliseconds \
             per frame). Construct ONE `{kind}` and reuse it across frames, \
             calling `.clear()` each frame to reset geometry while keeping the \
             warm caches.",
            StaleListDetector::WARN_AFTER,
        );
    }

    /// Blur an **app-provided** scene texture into `target` over `region`, for
    /// menu/pause "frosted glass" backdrops.
    ///
    /// The renderer never samples its own framebuffer, so the app supplies the
    /// already-rendered scene as a sampleable [`Backdrop`] (a `TextureView` with
    /// `TEXTURE_BINDING` usage plus its physical size). This records a separable
    /// two-pass Gaussian into `encoder` and writes the blurred region straight
    /// into `target`; draw the UI panels afterwards so they sit crisp on top.
    ///
    /// `region` is in **logical** pixels (UI coordinates); `viewport` is the
    /// target's **physical** size and `scale_factor` the logical→physical ratio,
    /// matching [`render`](Self::render). A degenerate region is a no-op.
    ///
    /// Typical frame: render scene → `blur_backdrop(region)` → `render(panels)`.
    #[allow(clippy::too_many_arguments)]
    pub fn blur_backdrop(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        backdrop: &Backdrop,
        region: Rect,
        viewport: (u32, u32),
        scale_factor: f32,
        params: &BlurParams,
    ) {
        let scale = if scale_factor > 0.0 { scale_factor } else { 1.0 };
        let region_phys = [
            region.x * scale,
            region.y * scale,
            region.width * scale,
            region.height * scale,
        ];
        if region_phys[2] <= 0.0 || region_phys[3] <= 0.0 {
            return;
        }
        if self.blur.is_none() {
            self.blur = Some(Blur::new(device, self.format));
        }
        let blur = self.blur.as_mut().expect("blur just ensured");
        blur.run(
            device,
            queue,
            encoder,
            target,
            backdrop,
            region_phys,
            viewport,
            params,
        );
    }

    fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: (u32, u32),
        scale_factor: f32,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_prepare_frame").entered();
        // The framebuffer is `viewport` physical pixels, but the UI is laid out
        // in logical pixels. Project logical → NDC by dividing the physical size
        // by the scale factor: a logical point still maps to the same NDC, while
        // the GPU rasterizes onto the full physical target (text self-sharpens).
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let logical_w = viewport.0 as f32 / scale;
        let logical_h = viewport.1 as f32 / scale;
        let uniforms = Uniforms {
            view_proj: ortho_matrix(logical_w, logical_h),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
        self.text_renderer.resize(viewport.0, viewport.1);
        // Reset the per-frame bump cursors so this frame's draws start at 0.
        self.color_vbo_offset = 0;
        self.color_ibo_offset = 0;
        self.icon_inst_offset = 0;
        self.chrome_inst_offset = 0;
        self.circle_inst_offset = 0;
        self.nine_inst_offset = 0;
        self.text_renderer.begin_frame();
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

        // ---------- 1. Nine-slices (instanced) ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_nine_slices").entered();
            let instances = self.build_nine_slice_instances(&draw_list.nine_slices);
            if !instances.is_empty() {
                self.draw_nine_slices(device, queue, encoder, view, &instances);
            }
        }

        // ---------- 2. Colored quads (+ instanced chrome) ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_color_quads").entered();
            if draw_list.color_cmds.is_empty() {
                // No chrome instances recorded: original single-draw fast path.
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
            } else {
                self.draw_color_interleaved(device, queue, encoder, view, draw_list);
            }
        }

        // ---------- 3. Icons (instanced) ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_icons").entered();
            let instances = self.build_icon_instances(&draw_list.icons);
            if !instances.is_empty() {
                self.draw_icons(device, queue, encoder, view, &instances);
            }
        }

        // ---------- 3b. MSDF vector icons (Phosphor) ----------
        // After sprite icons, before text — icons sit under text just like sprite
        // icons do. Shares the text renderer's MSDF pipeline/uniform/vbo.
        #[cfg(feature = "phosphor-icons")]
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_icons_msdf").entered();
            self.text_renderer
                .render_icons(device, queue, encoder, view, &draw_list.icons_msdf);
        }

        // ---------- 4. Text ----------
        {
            #[cfg(feature = "tracy")]
            let _s = tracing::info_span!("gameui_text").entered();
            self.text_renderer
                .render(device, queue, encoder, view, &draw_list.texts);
        }
    }

    fn build_icon_instances(&self, icons: &[IconDraw]) -> Vec<IconInstance> {
        let aw = self.atlas.width();
        let ah = self.atlas.height();
        let mut out: Vec<IconInstance> = Vec::with_capacity(icons.len());

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

            out.push(build_icon_instance(icon.corners, uv, icon.tint, clip));
        }

        out
    }

    fn build_nine_slice_instances(&self, draws: &[NineSliceDraw]) -> Vec<NineSliceInstance> {
        let aw = self.atlas.width();
        let ah = self.atlas.height();
        let mut out: Vec<NineSliceInstance> = Vec::with_capacity(draws.len());

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

            out.push(build_nine_slice_instance(
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
            ));
        }

        out
    }

    /// Ensure the color vbo/ibo hold this pass's bytes at their running frame
    /// offsets; returns `(vbo_offset, ibo_offset)` to write/draw at. Grows by
    /// allocating a fresh buffer; earlier passes keep their old buffer (held by
    /// the encoder) so their data stays valid.
    fn ensure_color_capacity(
        &mut self,
        device: &wgpu::Device,
        verts: usize,
        indices: usize,
    ) -> (u64, u64) {
        let v_off = self.color_vbo_offset;
        let needed_v = v_off + (verts * std::mem::size_of::<Vertex>()) as u64;
        if needed_v > self.color_vbo_capacity {
            self.color_vbo_capacity = needed_v.next_power_of_two();
            self.color_vbo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui color vbo"),
                size: self.color_vbo_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let i_off = self.color_ibo_offset;
        let needed_i = i_off + (indices * std::mem::size_of::<u32>()) as u64;
        if needed_i > self.color_ibo_capacity {
            self.color_ibo_capacity = needed_i.next_power_of_two();
            self.color_ibo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui color ibo"),
                size: self.color_ibo_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        (v_off, i_off)
    }

    /// Ensure the icon instance buffer holds `count` instances at its running
    /// frame offset; returns the byte offset to write/draw at. Same grow
    /// semantics as [`ensure_chrome_capacity`].
    fn ensure_icon_capacity(&mut self, device: &wgpu::Device, count: usize) -> u64 {
        let off = self.icon_inst_offset;
        let needed = off + (count * std::mem::size_of::<IconInstance>()) as u64;
        if needed > self.icon_inst_capacity {
            self.icon_inst_capacity = needed.next_power_of_two();
            self.icon_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui icon inst buffer"),
                size: self.icon_inst_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        off
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
        let (v_off, i_off) = self.ensure_color_capacity(device, verts.len(), indices.len());
        queue.write_buffer(&self.color_vbo, v_off, bytemuck::cast_slice(verts));
        queue.write_buffer(&self.color_ibo, i_off, bytemuck::cast_slice(indices));
        self.color_vbo_offset = v_off + (verts.len() * std::mem::size_of::<Vertex>()) as u64;
        self.color_ibo_offset = i_off + (indices.len() * std::mem::size_of::<u32>()) as u64;

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
        // Index 0 maps to the first vertex of this pass's slice (base_vertex 0
        // + the sliced vertex buffer), so per-pass indices stay 0-based.
        pass.set_vertex_buffer(0, self.color_vbo.slice(v_off..));
        pass.set_index_buffer(self.color_ibo.slice(i_off..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
    }

    /// Ensure the chrome instance buffer holds `count` instances at its running
    /// frame offset; returns the byte offset to write/draw at. Same grow
    /// semantics as [`ensure_tex_capacity`].
    fn ensure_chrome_capacity(&mut self, device: &wgpu::Device, count: usize) -> u64 {
        let off = self.chrome_inst_offset;
        let needed = off + (count * std::mem::size_of::<ChromeInstance>()) as u64;
        if needed > self.chrome_inst_capacity {
            self.chrome_inst_capacity = needed.next_power_of_two();
            self.chrome_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui chrome inst buffer"),
                size: self.chrome_inst_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        off
    }

    /// Same grow semantics as [`ensure_chrome_capacity`], for circle instances.
    fn ensure_circle_capacity(&mut self, device: &wgpu::Device, count: usize) -> u64 {
        let off = self.circle_inst_offset;
        let needed = off + (count * std::mem::size_of::<CircleInstance>()) as u64;
        if needed > self.circle_inst_capacity {
            self.circle_inst_capacity = needed.next_power_of_two();
            self.circle_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui circle inst buffer"),
                size: self.circle_inst_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        off
    }

    /// Render the colored stage as an ordered interleave of soup index runs and
    /// instanced chrome / circle runs (see [`ColorCmd`]). Soup + instances are
    /// each uploaded once at their frame bump offsets, then a single render pass
    /// issues the draws in submission order so layering is preserved.
    fn draw_color_interleaved(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        draw_list: &DrawList,
    ) {
        let verts = &draw_list.vertices;
        let indices = &draw_list.indices;
        let instances = &draw_list.chrome_instances;
        let circles = &draw_list.circle_instances;

        // Upload the whole soup once. Soup indices are absolute, so binding the
        // full sliced vbo/ibo with base_vertex 0 lets each `Soup` sub-range draw
        // its slice directly.
        let (v_off, i_off) = if !indices.is_empty() {
            let (v_off, i_off) = self.ensure_color_capacity(device, verts.len(), indices.len());
            queue.write_buffer(&self.color_vbo, v_off, bytemuck::cast_slice(verts));
            queue.write_buffer(&self.color_ibo, i_off, bytemuck::cast_slice(indices));
            self.color_vbo_offset = v_off + (verts.len() * std::mem::size_of::<Vertex>()) as u64;
            self.color_ibo_offset = i_off + (indices.len() * std::mem::size_of::<u32>()) as u64;
            (v_off, i_off)
        } else {
            (0, 0)
        };

        // Upload all chrome instances once.
        let inst_off = if !instances.is_empty() {
            let off = self.ensure_chrome_capacity(device, instances.len());
            queue.write_buffer(
                &self.chrome_inst_buffer,
                off,
                bytemuck::cast_slice(instances),
            );
            self.chrome_inst_offset =
                off + (instances.len() * std::mem::size_of::<ChromeInstance>()) as u64;
            off
        } else {
            0
        };

        // Upload all circle instances once.
        let circle_off = if !circles.is_empty() {
            let off = self.ensure_circle_capacity(device, circles.len());
            queue.write_buffer(&self.circle_inst_buffer, off, bytemuck::cast_slice(circles));
            self.circle_inst_offset =
                off + (circles.len() * std::mem::size_of::<CircleInstance>()) as u64;
            off
        } else {
            0
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui color+chrome pass"),
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
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);

        // Draw a soup index sub-range with the color pipeline.
        let draw_soup = |pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>| {
            pass.set_pipeline(&self.color_pipeline);
            pass.set_vertex_buffer(0, self.color_vbo.slice(v_off..));
            pass.set_index_buffer(self.color_ibo.slice(i_off..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(range, 0, 0..1);
        };

        for cmd in &draw_list.color_cmds {
            match cmd {
                ColorCmd::Soup { indices } => draw_soup(&mut pass, indices.clone()),
                ColorCmd::Chrome { instances } => {
                    pass.set_pipeline(&self.chrome_pipeline);
                    pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
                    pass.set_vertex_buffer(1, self.chrome_inst_buffer.slice(inst_off..));
                    pass.set_index_buffer(
                        self.chrome_base_ibo.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.draw_indexed(0..CHROME_BASE_INDICES.len() as u32, 0, instances.clone());
                }
                ColorCmd::Circle { instances } => {
                    pass.set_pipeline(&self.circle_pipeline);
                    pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
                    pass.set_vertex_buffer(1, self.circle_inst_buffer.slice(circle_off..));
                    pass.set_index_buffer(
                        self.chrome_base_ibo.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.draw_indexed(0..CHROME_BASE_INDICES.len() as u32, 0, instances.clone());
                }
            }
        }

        // Soup appended after the last recorded command is the trailing run.
        let committed = draw_list.soup_committed_indices;
        let total = indices.len() as u32;
        if total > committed {
            draw_soup(&mut pass, committed..total);
        }
    }

    /// Draw all icons/images as a single instanced call: the chrome unit-quad
    /// base mesh + one [`IconInstance`] per icon. The vertex shader bilinearly
    /// interpolates the baked-in corners and the fragment samples the atlas.
    fn draw_icons(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        instances: &[IconInstance],
    ) {
        let off = self.ensure_icon_capacity(device, instances.len());
        queue.write_buffer(&self.icon_inst_buffer, off, bytemuck::cast_slice(instances));
        self.icon_inst_offset =
            off + (instances.len() * std::mem::size_of::<IconInstance>()) as u64;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui icon pass"),
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
        pass.set_pipeline(&self.icon_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.texture_bind_group, &[]);
        pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
        pass.set_vertex_buffer(1, self.icon_inst_buffer.slice(off..));
        pass.set_index_buffer(self.chrome_base_ibo.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(
            0..CHROME_BASE_INDICES.len() as u32,
            0,
            0..instances.len() as u32,
        );
    }

    /// Ensure the nine-slice instance buffer holds `count` instances at its
    /// running frame offset; returns the byte offset to write/draw at. Same grow
    /// semantics as [`ensure_chrome_capacity`].
    fn ensure_nine_capacity(&mut self, device: &wgpu::Device, count: usize) -> u64 {
        let off = self.nine_inst_offset;
        let needed = off + (count * std::mem::size_of::<NineSliceInstance>()) as u64;
        if needed > self.nine_inst_capacity {
            self.nine_inst_capacity = needed.next_power_of_two();
            self.nine_inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui nine-slice inst buffer"),
                size: self.nine_inst_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        off
    }

    /// Draw all nine-slice panels as a single instanced call: the chrome
    /// unit-quad base mesh + one [`NineSliceInstance`] per panel. The fragment
    /// remaps local coords → source UV (nine-region piecewise map) and samples
    /// the atlas.
    fn draw_nine_slices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        instances: &[NineSliceInstance],
    ) {
        let off = self.ensure_nine_capacity(device, instances.len());
        queue.write_buffer(&self.nine_inst_buffer, off, bytemuck::cast_slice(instances));
        self.nine_inst_offset =
            off + (instances.len() * std::mem::size_of::<NineSliceInstance>()) as u64;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui nine-slice pass"),
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
        pass.set_pipeline(&self.nine_slice_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.texture_bind_group, &[]);
        pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
        pass.set_vertex_buffer(1, self.nine_inst_buffer.slice(off..));
        pass.set_index_buffer(self.chrome_base_ibo.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(
            0..CHROME_BASE_INDICES.len() as u32,
            0,
            0..instances.len() as u32,
        );
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

/// Build one [`IconInstance`] from a quad's 4 world-space corners (TL, TR, BR,
/// BL — matching `Affine2::transform_rect_corners`), its source UV rect, tint,
/// and clip. The corners are baked in and bilinearly interpolated in `vs_icon`,
/// so any rotation/scale/shear from the active transform is preserved with no
/// fallback. Mirrors the per-vertex UV assignment the old tessellator did.
fn build_icon_instance(
    corners: [[f32; 2]; 4],
    uv: [f32; 4], // u0, v0, u1, v1
    tint: [f32; 4],
    clip: Option<[f32; 4]>,
) -> IconInstance {
    let (clip_rect, clip_enabled) = match clip {
        Some(r) => (r, 1.0),
        None => ([0.0; 4], 0.0),
    };
    let tl = corners[0];
    let tr = corners[1];
    let br = corners[2];
    let bl = corners[3];
    IconInstance {
        c_tl_tr: [tl[0], tl[1], tr[0], tr[1]],
        c_br_bl: [br[0], br[1], bl[0], bl[1]],
        uv_rect: uv,
        tint,
        clip: clip_rect,
        flags: [clip_enabled, 0.0, 0.0, 0.0],
    }
}

/// Build one [`NineSliceInstance`] from a panel's local rect, transform, tint,
/// clip, and the resolved atlas region + border. The 9-region UV map and the
/// border collapse are evaluated per-pixel in `fs_nine_slice`; here we only
/// resolve the outer/inner UV stops and pack the affine. Mirrors the UV math the
/// old `tessellate_nine_slice` did on the CPU.
#[allow(clippy::too_many_arguments)]
fn build_nine_slice_instance(
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
) -> NineSliceInstance {
    let bl = border[0] as f32;
    let bt = border[1] as f32;
    let br = border[2] as f32;
    let bb = border[3] as f32;

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

    let (clip_rect, clip_enabled) = match clip {
        Some(r) => (r, 1.0),
        None => ([0.0; 4], 0.0),
    };

    NineSliceInstance {
        lin: [transform.a, transform.b, transform.c, transform.d],
        translate: [transform.tx, transform.ty, clip_enabled, 0.0],
        origin_size: [x, y, w, h],
        uv_outer: [u0, v0, u3, v3],
        uv_inner: [u1, v1, u2, v2],
        border: [bl, bt, br, bb],
        tint,
        clip: clip_rect,
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

    // ---- StaleListDetector ----

    #[test]
    fn stale_detector_reused_list_never_warns() {
        // The correct pattern: one list reused every frame → same id forever.
        let mut d = StaleListDetector::default();
        for _ in 0..(StaleListDetector::WARN_AFTER * 3) {
            assert!(!d.observe(42), "reusing one list must never warn");
        }
        assert_eq!(d.fresh_streak, 0);
    }

    #[test]
    fn stale_detector_fresh_list_each_frame_warns_once() {
        // The footgun: a brand-new id every frame (LayerStack::new() in the loop).
        let mut d = StaleListDetector::default();
        let mut id = 1u64;
        let mut warnings = 0;
        for _ in 0..(StaleListDetector::WARN_AFTER * 2) {
            if d.observe(id) {
                warnings += 1;
            }
            id += 1; // never repeats — every frame is a fresh list
        }
        assert_eq!(warnings, 1, "must warn exactly once, not spam every frame");
    }

    #[test]
    fn stale_detector_warns_exactly_at_threshold() {
        let mut d = StaleListDetector::default();
        // WARN_AFTER-1 fresh frames: not yet.
        for id in 1..StaleListDetector::WARN_AFTER as u64 {
            assert!(!d.observe(id));
        }
        // The WARN_AFTER-th consecutive fresh frame trips it.
        assert!(d.observe(StaleListDetector::WARN_AFTER as u64));
    }

    #[test]
    fn stale_detector_double_buffered_stacks_dont_warn() {
        // Two persistent stacks alternating (a legitimate double-buffer): both ids
        // keep recurring, so neither is ever "never seen". Must not warn.
        let mut d = StaleListDetector::default();
        for i in 0..(StaleListDetector::WARN_AFTER * 4) {
            let id = if i % 2 == 0 { 7 } else { 9 };
            assert!(!d.observe(id), "alternating persistent stacks must not warn");
        }
    }

    #[test]
    fn stale_detector_occasional_rebuild_does_not_warn() {
        // A reused list that gets rebuilt now and then (e.g. on resize) produces a
        // lone fresh id occasionally — never a long enough streak to trip.
        let mut d = StaleListDetector::default();
        let mut id = 1u64;
        for frame in 0..(StaleListDetector::WARN_AFTER * 5) {
            if frame % 20 == 0 {
                id += 1; // rebuild: new id this frame, then reused for the next 19
            }
            assert!(!d.observe(id), "occasional rebuilds must not warn");
        }
    }

    // ---- DrawList / LayerStack identity (backs the detector) ----

    #[test]
    fn distinct_draw_lists_have_distinct_ids() {
        let a = DrawList::new();
        let b = DrawList::new();
        assert_ne!(a.id(), b.id(), "each DrawList must get a unique id");
    }

    #[test]
    fn clear_preserves_id() {
        let mut a = DrawList::new();
        let before = a.id();
        a.clear();
        assert_eq!(a.id(), before, "clear() must keep the same list identity");
    }

    #[test]
    fn layer_stack_base_id_matches_base_list() {
        let s = LayerStack::new();
        assert_eq!(s.base_id(), s.base().id());
        // A second stack is a distinct base list.
        let s2 = LayerStack::new();
        assert_ne!(s.base_id(), s2.base_id());
    }

    /// Replicate the vertex shaders' `vec4(pos, 0, 1) * view_proj` (row-vector ×
    /// matrix) to get the NDC of a pixel-space point under a given ortho.
    fn project(m: &[[f32; 4]; 4], x: f32, y: f32) -> (f32, f32) {
        let v = [x, y, 0.0, 1.0];
        let mut out = [0.0f32; 4];
        for (j, o) in out.iter_mut().enumerate() {
            *o = v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j] + v[3] * m[3][j];
        }
        (out[0], out[1])
    }

    #[test]
    fn ortho_maps_logical_corners_to_ndc() {
        let m = ortho_matrix(800.0, 600.0);
        // Top-left logical origin → NDC top-left (-1, +1); Y is down in pixels.
        let (x0, y0) = project(&m, 0.0, 0.0);
        assert!(
            (x0 + 1.0).abs() < 1e-5 && (y0 - 1.0).abs() < 1e-5,
            "{x0},{y0}"
        );
        // Bottom-right logical extent → NDC (+1, -1).
        let (x1, y1) = project(&m, 800.0, 600.0);
        assert!(
            (x1 - 1.0).abs() < 1e-5 && (y1 + 1.0).abs() < 1e-5,
            "{x1},{y1}"
        );
        // Center → origin.
        let (xc, yc) = project(&m, 400.0, 300.0);
        assert!(xc.abs() < 1e-5 && yc.abs() < 1e-5, "{xc},{yc}");
    }

    #[test]
    fn dpi_scale_is_logical_size_invariant() {
        // prepare_frame builds ortho from (physical / scale). Two (physical,
        // scale) pairs with the SAME logical size must project a logical point
        // identically — the scale factor only changes which framebuffer the
        // identical logical UI rasterizes onto.
        let logical = |w: f32, s: f32| ortho_matrix(w / s, w / s); // square for brevity
        let a = logical(800.0, 1.0); // physical 800, scale 1 → logical 800
        let b = logical(1600.0, 2.0); // physical 1600, scale 2 → logical 800
        for &(px, py) in &[(0.0, 0.0), (250.0, 700.0), (800.0, 800.0)] {
            let (ax, ay) = project(&a, px, py);
            let (bx, by) = project(&b, px, py);
            assert!(
                (ax - bx).abs() < 1e-6 && (ay - by).abs() < 1e-6,
                "logical point ({px},{py}) projected differently: a=({ax},{ay}) b=({bx},{by})"
            );
        }
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
    fn icon_instance_packs_corners_uv_clip() {
        // Corners in TL, TR, BR, BL order (as transform_rect_corners yields).
        let corners = [[10.0, 20.0], [110.0, 20.0], [110.0, 70.0], [10.0, 70.0]];
        let inst = build_icon_instance(
            corners,
            [0.1, 0.2, 0.3, 0.4],
            [1.0, 0.5, 0.25, 1.0],
            Some([1.0, 2.0, 3.0, 4.0]),
        );
        assert_eq!(inst.c_tl_tr, [10.0, 20.0, 110.0, 20.0]);
        assert_eq!(inst.c_br_bl, [110.0, 70.0, 10.0, 70.0]);
        assert_eq!(inst.uv_rect, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(inst.tint, [1.0, 0.5, 0.25, 1.0]);
        assert_eq!(inst.clip, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(inst.flags[0], 1.0); // clip enabled
    }

    #[test]
    fn icon_instance_clip_none_disables() {
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let inst = build_icon_instance(corners, [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        assert_eq!(inst.flags[0], 0.0); // clip disabled
        assert_eq!(inst.clip, [0.0; 4]);
    }

    #[test]
    fn nine_slice_instance_packs_affine_and_size() {
        let region = AtlasRegion {
            x: 0,
            y: 0,
            w: 32,
            h: 32,
        };
        // Translation-only transform: linear part is identity, translate carries
        // the offset. clip None => clip_enabled 0.
        let t = crate::affine::Affine2::translation(7.0, 11.0);
        let inst = build_nine_slice_instance(
            5.0,
            6.0,
            100.0,
            80.0,
            &t,
            [1.0; 4],
            None,
            region,
            [4, 4, 4, 4],
            64,
            64,
        );
        assert_eq!(inst.lin, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(inst.translate, [7.0, 11.0, 0.0, 0.0]);
        assert_eq!(inst.origin_size, [5.0, 6.0, 100.0, 80.0]);
        assert_eq!(inst.border, [4.0, 4.0, 4.0, 4.0]);
    }

    #[test]
    fn nine_slice_instance_uvs_match_border() {
        let region = AtlasRegion {
            x: 0,
            y: 0,
            w: 32,
            h: 32,
        };
        let identity = crate::affine::Affine2::IDENTITY;
        let inst = build_nine_slice_instance(
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
        // Outer UV origin = region origin (0,0); far edge = 32/64 = 0.5.
        approx4(inst.uv_outer, [0.0, 0.0, 0.5, 0.5]);
        // Inner seams: left/top = 8/64 = 0.125; right/bottom = (32-8)/64 = 0.375.
        approx4(inst.uv_inner, [0.125, 0.125, 0.375, 0.375]);
    }

    #[test]
    fn nine_slice_instance_records_affine_rotation_and_clip() {
        let region = AtlasRegion {
            x: 16,
            y: 16,
            w: 16,
            h: 16,
        };
        // A rotation injects nonzero off-diagonals — proving the full affine is
        // baked in (no axis-aligned fallback needed for nine-slices).
        let r = crate::affine::Affine2::rotation(std::f32::consts::FRAC_PI_2);
        let inst = build_nine_slice_instance(
            0.0,
            0.0,
            10.0,
            10.0,
            &r,
            [1.0; 4],
            Some([1.0, 2.0, 3.0, 4.0]),
            region,
            [2, 2, 2, 2],
            64,
            64,
        );
        assert!(inst.lin[1].abs() > 1e-3 || inst.lin[2].abs() > 1e-3);
        assert_eq!(inst.translate[2], 1.0); // clip_enabled
        assert_eq!(inst.clip, [1.0, 2.0, 3.0, 4.0]);
    }
}
