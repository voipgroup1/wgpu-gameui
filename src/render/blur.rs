//! Backdrop blur — separable two-pass Gaussian over an **app-provided** scene
//! texture.
//!
//! The renderer otherwise never samples a framebuffer; blur is the one effect
//! that must. Rather than push a GPU texture handle into the (deliberately
//! GPU-agnostic) [`DrawList`], the app hands us the already-rendered scene as a
//! sampleable [`wgpu::TextureView`] and we blur a region of it straight into the
//! UI target. The intended call order for a pause/menu screen is:
//!
//! 1. render the game scene into a texture (with `TEXTURE_BINDING` usage),
//! 2. [`UiRenderer::blur_backdrop`](crate::UiRenderer::blur_backdrop) that scene
//!    into the UI surface over the panel region,
//! 3. render the UI panels on top with [`UiRenderer::render`](crate::UiRenderer::render).
//!
//! Pass A blurs horizontally from the scene into a (downsampled) intermediate;
//! pass B blurs vertically from the intermediate into the target.

use bytemuck::{Pod, Zeroable};

const SHADER: &str = include_str!("blur.wgsl");

/// The app-provided scene texture to blur behind UI panels.
pub struct Backdrop<'a> {
    /// A sampleable view of the rendered scene (needs `TEXTURE_BINDING` usage).
    pub view: &'a wgpu::TextureView,
    /// Physical-pixel dimensions of that texture.
    pub size: (u32, u32),
}

/// Backdrop-blur configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurParams {
    /// Blur strength, in source pixels. Larger = blurrier. Default `8.0`.
    pub radius: f32,
    /// Intermediate-resolution divisor (`1`/`2`/`4`). Higher is cheaper and
    /// gives a wider, softer blur for the same radius. Default `2`.
    pub downsample: u32,
    /// RGBA multiplied into the blurred output — use a darkening, semi-opaque
    /// value for a scrim. Default `[1, 1, 1, 1]` (passthrough, opaque).
    pub tint: [f32; 4],
}

impl Default for BlurParams {
    fn default() -> Self {
        Self {
            radius: 8.0,
            downsample: 2,
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct BlurUniforms {
    out_rect: [f32; 4],
    in_uv: [f32; 4],
    dir_step: [f32; 2],
    radius: f32,
    _pad0: f32,
    tint: [f32; 4],
}

/// Cached intermediate render target (horizontal-pass output).
struct Intermediate {
    #[allow(dead_code)]
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// GPU resources for the separable-Gaussian blur. Constructed lazily on first
/// use so [`UiRenderer::new`](crate::UiRenderer) stays unchanged.
pub(crate) struct Blur {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    src_bgl: wgpu::BindGroupLayout,
    // One uniform buffer per pass: both passes are recorded into the same
    // encoder before submit, so a single shared buffer would let pass B's
    // contents clobber pass A's before the GPU runs either.
    uniform_a: wgpu::Buffer,
    uniform_b: wgpu::Buffer,
    uniform_bg_a: wgpu::BindGroup,
    uniform_bg_b: wgpu::BindGroup,
    inter: Option<Intermediate>,
    format: wgpu::TextureFormat,
}

impl Blur {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur uniform bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let src_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur source bgl"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur pipeline layout"),
            bind_group_layouts: &[&uniform_bgl, &src_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_blur"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_blur"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_size = std::mem::size_of::<BlurUniforms>() as u64;
        let uniform_a = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur uniform A"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur uniform B"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bg_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur uniform bg A"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_a.as_entire_binding(),
            }],
        });
        let uniform_bg_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur uniform bg B"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_b.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            sampler,
            src_bgl,
            uniform_a,
            uniform_b,
            uniform_bg_a,
            uniform_bg_b,
            inter: None,
            format,
        }
    }

    fn ensure_intermediate(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let need = match &self.inter {
            Some(i) => i.size != size,
            None => true,
        };
        if need {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("blur intermediate"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.inter = Some(Intermediate { tex, view, size });
        }
    }

    fn make_src_bg(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur source bg"),
            layout: &self.src_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Record the two blur passes into `encoder`. `region_phys` is the
    /// blur+placement rect in **physical** pixels of the target.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        backdrop: &Backdrop,
        region_phys: [f32; 4],
        viewport: (u32, u32),
        params: &BlurParams,
    ) {
        let ds = params.downsample.max(1);
        let inter_size = intermediate_size([region_phys[2], region_phys[3]], ds);
        self.ensure_intermediate(device, inter_size);
        let inter = self.inter.as_ref().expect("intermediate just ensured");

        // Pass A: horizontal blur, scene -> intermediate (full quad).
        let ua = BlurUniforms {
            out_rect: [-1.0, 1.0, 1.0, -1.0],
            in_uv: uv_rect(region_phys, backdrop.size),
            dir_step: [1.0 / backdrop.size.0.max(1) as f32, 0.0],
            radius: params.radius,
            _pad0: 0.0,
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        queue.write_buffer(&self.uniform_a, 0, bytemuck::bytes_of(&ua));

        // Pass B: vertical blur, intermediate -> target (region quad). The
        // radius is scaled down by `ds` because one intermediate texel spans
        // `ds` source pixels, keeping horizontal and vertical extents matched.
        let ub = BlurUniforms {
            out_rect: ndc_rect(region_phys, viewport),
            in_uv: [0.0, 0.0, 1.0, 1.0],
            dir_step: [0.0, 1.0 / inter_size.1.max(1) as f32],
            radius: params.radius / ds as f32,
            _pad0: 0.0,
            tint: params.tint,
        };
        queue.write_buffer(&self.uniform_b, 0, bytemuck::bytes_of(&ub));

        let src_bg_a = self.make_src_bg(device, backdrop.view);
        let src_bg_b = self.make_src_bg(device, &inter.view);

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur pass A (horizontal)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &inter.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.uniform_bg_a, &[]);
            rp.set_bind_group(1, &src_bg_a, &[]);
            rp.draw(0..4, 0..1);
        }
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur pass B (vertical)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.uniform_bg_b, &[]);
            rp.set_bind_group(1, &src_bg_b, &[]);
            rp.draw(0..4, 0..1);
        }
    }
}

/// Intermediate texture size for a region of `[w, h]` physical px at `downsample`.
/// Clamped to at least 1×1 so a tiny/degenerate region never makes a 0-sized
/// texture.
pub(crate) fn intermediate_size(region_wh: [f32; 2], downsample: u32) -> (u32, u32) {
    let ds = downsample.max(1) as f32;
    let w = (region_wh[0] / ds).round().max(1.0) as u32;
    let h = (region_wh[1] / ds).round().max(1.0) as u32;
    (w.max(1), h.max(1))
}

/// Map a physical-pixel `[x, y, w, h]` rect (top-left origin, y-down) to NDC
/// corners `[x0, y0(top), x1, y1(bottom)]` for the given viewport.
pub(crate) fn ndc_rect(region: [f32; 4], viewport: (u32, u32)) -> [f32; 4] {
    let w = viewport.0.max(1) as f32;
    let h = viewport.1.max(1) as f32;
    let x0 = 2.0 * region[0] / w - 1.0;
    let x1 = 2.0 * (region[0] + region[2]) / w - 1.0;
    let y0 = 1.0 - 2.0 * region[1] / h; // top edge
    let y1 = 1.0 - 2.0 * (region[1] + region[3]) / h; // bottom edge
    [x0, y0, x1, y1]
}

/// Map a physical-pixel `[x, y, w, h]` rect to UV corners `[u0, v0, u1, v1]`
/// within a source texture of `source` physical px.
pub(crate) fn uv_rect(region: [f32; 4], source: (u32, u32)) -> [f32; 4] {
    let w = source.0.max(1) as f32;
    let h = source.1.max(1) as f32;
    [
        region[0] / w,
        region[1] / h,
        (region[0] + region[2]) / w,
        (region[1] + region[3]) / h,
    ]
}

/// Unnormalized Gaussian weight at integer tap `i` for the given `sigma`.
///
/// This mirrors the kernel the WGSL fragment shader computes; it exists so the
/// blur's weighting math can be checked headlessly (see tests). The GPU path
/// computes its own weights, so this is only referenced from tests.
#[cfg(test)]
fn gaussian_weight(i: f32, sigma: f32) -> f32 {
    let s = sigma.max(0.0001);
    (-(i * i) / (2.0 * s * s)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params() {
        let p = BlurParams::default();
        assert_eq!(p.radius, 8.0);
        assert_eq!(p.downsample, 2);
        assert_eq!(p.tint, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn intermediate_size_divides_and_clamps() {
        assert_eq!(intermediate_size([200.0, 100.0], 2), (100, 50));
        assert_eq!(intermediate_size([200.0, 100.0], 4), (50, 25));
        assert_eq!(intermediate_size([200.0, 100.0], 1), (200, 100));
        // downsample 0 is treated as 1 (no divide-by-zero).
        assert_eq!(intermediate_size([200.0, 100.0], 0), (200, 100));
        // Degenerate region never yields a 0-sized texture.
        assert_eq!(intermediate_size([0.0, 0.0], 4), (1, 1));
        assert_eq!(intermediate_size([3.0, 3.0], 8), (1, 1));
    }

    #[test]
    fn ndc_rect_maps_corners_with_y_flip() {
        // Full-viewport region maps to the full NDC cube.
        let full = ndc_rect([0.0, 0.0, 800.0, 600.0], (800, 600));
        assert!((full[0] - -1.0).abs() < 1e-6); // x0 left
        assert!((full[1] - 1.0).abs() < 1e-6); // y0 top (+1)
        assert!((full[2] - 1.0).abs() < 1e-6); // x1 right
        assert!((full[3] - -1.0).abs() < 1e-6); // y1 bottom (-1)

        // A centered quarter-region: top is above center, bottom below.
        let r = ndc_rect([200.0, 150.0, 400.0, 300.0], (800, 600));
        assert!((r[0] - -0.5).abs() < 1e-6);
        assert!((r[1] - 0.5).abs() < 1e-6); // y0 top
        assert!((r[2] - 0.5).abs() < 1e-6);
        assert!((r[3] - -0.5).abs() < 1e-6); // y1 bottom
        assert!(r[1] > r[3], "top NDC y must be greater than bottom");
    }

    #[test]
    fn uv_rect_maps_subrect() {
        let uv = uv_rect([100.0, 50.0, 200.0, 100.0], (400, 200));
        assert!((uv[0] - 0.25).abs() < 1e-6);
        assert!((uv[1] - 0.25).abs() < 1e-6);
        assert!((uv[2] - 0.75).abs() < 1e-6);
        assert!((uv[3] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn gaussian_weights_are_symmetric_and_peak_at_center() {
        let sigma = 4.0;
        assert!((gaussian_weight(0.0, sigma) - 1.0).abs() < 1e-9);
        assert!((gaussian_weight(-3.0, sigma) - gaussian_weight(3.0, sigma)).abs() < 1e-12);
        assert!(gaussian_weight(1.0, sigma) > gaussian_weight(5.0, sigma));
        assert!(gaussian_weight(5.0, sigma) > 0.0);
    }

    #[test]
    fn gaussian_normalized_kernel_sums_to_one() {
        let sigma = 3.0;
        let mut sum = 0.0;
        for i in -16..=16 {
            sum += gaussian_weight(i as f32, sigma);
        }
        // Normalizing divides each weight by `sum`, so the normalized kernel
        // sums to exactly 1 by construction; just check `sum` is sane (>0).
        assert!(sum > 1.0);
        let normalized: f32 = (-16..=16)
            .map(|i| gaussian_weight(i as f32, sigma) / sum)
            .sum();
        assert!((normalized - 1.0).abs() < 1e-6);
    }
}
