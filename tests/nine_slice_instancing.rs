//! Headless GPU parity test for the instanced nine-slice path.
//!
//! `DrawList::nine_slice` now rasterizes a panel from a single instanced draw:
//! one unit-quad base mesh + one `NineSliceInstance`, with the fragment shader
//! remapping local coords into the source UV via the classic nine-region
//! piecewise-linear map. This replaces re-tessellating 9 quads (54 verts) per
//! panel into the textured soup every frame.
//!
//! This test renders the same grid of panels two ways — once via the instanced
//! `nine_slice` path, once via 9 individually-cropped `image_cropped` quads
//! (which reproduce, region-for-region, what the old CPU tessellator emitted) —
//! reads both frames back, and asserts the images match within a tolerance.
//! They are not bit-exact: linear filtering at the region seams diverges a hair
//! between a single spanning quad and 9 abutting quads, so a small fraction of
//! pixels (the seams) may differ.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test nine_slice_instancing -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, FontSystemHandle, UiRenderer};

const W: u32 = 480;
const H: u32 = 320;

/// Sprite dimensions + nine-slice border (source px).
const SPRITE: u32 = 32;
const BORDER: u32 = 8;

/// A `SPRITE×SPRITE` sprite painted with 9 distinct colors, one per nine-slice
/// source region (corners / edges / center), so any UV-mapping error between the
/// two render paths shows up as a color mismatch rather than hiding in a flat
/// fill.
fn nine_region_sprite() -> Vec<u8> {
    // Region color per (row, col) in the 3×3 nine-slice grid.
    const COLORS: [[u8; 4]; 9] = [
        [220, 40, 40, 255],
        [40, 220, 40, 255],
        [40, 40, 220, 255],
        [220, 220, 40, 255],
        [220, 40, 220, 255],
        [40, 220, 220, 255],
        [255, 140, 0, 255],
        [140, 0, 255, 255],
        [120, 120, 120, 255],
    ];
    let b = BORDER;
    let s = SPRITE;
    let mut out = vec![0u8; (s * s * 4) as usize];
    for y in 0..s {
        let row = if y < b {
            0
        } else if y < s - b {
            1
        } else {
            2
        };
        for x in 0..s {
            let col = if x < b {
                0
            } else if x < s - b {
                1
            } else {
                2
            };
            let c = COLORS[row * 3 + col];
            let idx = ((y * s + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

/// Render a single `DrawList` to an RGBA image (tightly packed, W*4 per row).
fn render_list(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    list: &DrawList,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("parity target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = (W * 4 + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parity readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    ui.render(device, queue, &mut encoder, &view, (W, H), 1.0, list);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    let row_stride = (W * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * H as usize);
    for row in 0..H as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }
    pixels
}

/// Lay out a 3×3 grid of 120×80 panels (each comfortably larger than 2×border,
/// so no middle-region collapse).
fn panel_rects() -> Vec<Rect> {
    let mut rects = Vec::new();
    for row in 0..3 {
        for col in 0..3 {
            rects.push(Rect::new(
                20.0 + col as f32 * 150.0,
                20.0 + row as f32 * 100.0,
                120.0,
                80.0,
            ));
        }
    }
    rects
}

/// Emit the 9 cropped-image quads that reproduce the old CPU nine-slice
/// tessellation for one panel `r`.
fn reference_nine_slice(list: &mut DrawList, sprite: wgpu_gameui::render::SpriteId, r: Rect) {
    let b = BORDER as f32;
    let s = SPRITE as f32;
    // Screen-space region stops.
    let xs = [r.x, r.x + b, r.x + r.width - b, r.x + r.width];
    let ys = [r.y, r.y + b, r.y + r.height - b, r.y + r.height];
    // Normalized within-sprite UV stops.
    let us = [0.0, b / s, (s - b) / s, 1.0];
    let vs = [0.0, b / s, (s - b) / s, 1.0];

    for row in 0..3 {
        for col in 0..3 {
            let dest = Rect::new(
                xs[col],
                ys[row],
                xs[col + 1] - xs[col],
                ys[row + 1] - ys[row],
            );
            let src_uv = [us[col], vs[row], us[col + 1], vs[row + 1]];
            list.image_cropped(sprite, dest, src_uv, [1.0; 4]);
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_nine_slice_matches_immediate() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("parity device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system: FontSystemHandle = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    let pixels = nine_region_sprite();
    let sprite = ui.load_sprite_rgba8("parity_frame", SPRITE, SPRITE, &pixels);
    let id = ui.register_nine_slice("parity_frame", sprite, [BORDER, BORDER, BORDER, BORDER]);

    let rects = panel_rects();

    // Instanced path.
    let mut instanced = DrawList::with_font_system(font_system.clone());
    for r in &rects {
        instanced.nine_slice_id(id, r.x, r.y, r.width, r.height, [1.0; 4]);
    }

    // Reference path: 9 cropped image quads per panel.
    let mut immediate = DrawList::with_font_system(font_system.clone());
    for r in &rects {
        reference_nine_slice(&mut immediate, sprite, *r);
    }

    let img_inst = render_list(&device, &queue, &mut ui, &instanced);
    let img_imm = render_list(&device, &queue, &mut ui, &immediate);

    // Persist for eyeballing.
    std::fs::create_dir_all("test_output").ok();
    image::RgbaImage::from_raw(W, H, img_inst.clone()).and_then(|i| {
        i.save("test_output/nine_slice_instanced.png")
            .ok()
            .map(|_| i)
    });
    image::RgbaImage::from_raw(W, H, img_imm.clone()).and_then(|i| {
        i.save("test_output/nine_slice_immediate.png")
            .ok()
            .map(|_| i)
    });

    assert_eq!(img_inst.len(), img_imm.len());

    let mut differing = 0usize;
    let mut drawn = 0usize;
    for (a, b) in img_inst.chunks_exact(4).zip(img_imm.chunks_exact(4)) {
        let da = (a[0] as i32 - b[0] as i32).abs()
            + (a[1] as i32 - b[1] as i32).abs()
            + (a[2] as i32 - b[2] as i32).abs();
        if da > 48 {
            differing += 1;
        }
        if a[0] as i32 + a[1] as i32 + a[2] as i32 > 30 {
            drawn += 1;
        }
    }
    let total = (W * H) as usize;
    assert!(drawn > total / 20, "too few pixels drawn ({drawn}/{total})");

    // Differences confined to the region seams (linear-filter divergence between
    // one spanning quad and 9 abutting quads): well under 6% of all pixels.
    let frac = differing as f64 / total as f64;
    assert!(
        frac < 0.06,
        "instanced vs immediate nine-slice differ in {:.2}% of pixels (>6%) — not just seams",
        frac * 100.0
    );
}
