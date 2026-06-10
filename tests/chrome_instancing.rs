//! Headless GPU parity test for the instanced SDF chrome path.
//!
//! `DrawList::chrome_rect` rasterizes a button's rounded background + border
//! from a signed distance field in one instanced draw, instead of tessellating
//! ~80 vertices into the colored soup. This test renders the same grid of
//! buttons two ways — once via `chrome_rect` (instanced), once via the immediate
//! `rounded_rect` + `rounded_rect_outline` primitives — reads both frames back,
//! and asserts the images match within a tolerance.
//!
//! They are *not* bit-exact: the SDF path anti-aliases its edges (a feature),
//! so the corners and 1px borders differ by design. We therefore allow a small
//! fraction of pixels to differ and require the bulk (flat interiors) to match.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test chrome_instancing -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, FontSystemHandle, UiRenderer};

const W: u32 = 480;
const H: u32 = 320;

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
    ui.render(device, queue, &mut encoder, &view, (W, H), list);
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

/// Lay out a 4×3 grid of 90×50 buttons.
fn button_rects() -> Vec<Rect> {
    let mut rects = Vec::new();
    for row in 0..3 {
        for col in 0..4 {
            rects.push(Rect::new(
                20.0 + col as f32 * 110.0,
                20.0 + row as f32 * 90.0,
                90.0,
                50.0,
            ));
        }
    }
    rects
}

const RADIUS: f32 = 8.0;
const THICKNESS: f32 = 2.0;
const BG: [f32; 4] = [0.20, 0.45, 0.75, 1.0];
const BORDER: [f32; 4] = [0.85, 0.85, 0.90, 1.0];

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_chrome_matches_immediate() {
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

    let rects = button_rects();

    // Instanced path.
    let mut instanced = DrawList::with_font_system(font_system.clone());
    for r in &rects {
        instanced.chrome_rect(*r, RADIUS, THICKNESS, BG, BORDER);
    }

    // Immediate path: exactly what `chrome_rect`'s fallback emits.
    let mut immediate = DrawList::with_font_system(font_system.clone());
    for r in &rects {
        immediate.rounded_rect(*r, RADIUS, BG);
        immediate.rounded_rect_outline(*r, RADIUS, THICKNESS, BORDER);
    }

    let img_inst = render_list(&device, &queue, &mut ui, &instanced);
    let img_imm = render_list(&device, &queue, &mut ui, &immediate);

    // Persist for eyeballing.
    std::fs::create_dir_all("test_output").ok();
    image::RgbaImage::from_raw(W, H, img_inst.clone())
        .and_then(|i| i.save("test_output/chrome_instanced.png").ok().map(|_| i));
    image::RgbaImage::from_raw(W, H, img_imm.clone())
        .and_then(|i| i.save("test_output/chrome_immediate.png").ok().map(|_| i));

    assert_eq!(img_inst.len(), img_imm.len());

    // Count pixels whose RGB differ beyond a tolerance. Edge AA + the 1px SDF
    // border are where the two paths legitimately diverge, so we allow a small
    // fraction. A non-trivial number of pixels must be drawn (sanity).
    let mut differing = 0usize;
    let mut drawn = 0usize;
    for (a, b) in img_inst.chunks_exact(4).zip(img_imm.chunks_exact(4)) {
        let da = (a[0] as i32 - b[0] as i32).abs()
            + (a[1] as i32 - b[1] as i32).abs()
            + (a[2] as i32 - b[2] as i32).abs();
        if da > 48 {
            differing += 1;
        }
        // "Drawn" = not the black clear color in the immediate reference.
        if a[0] as i32 + a[1] as i32 + a[2] as i32 > 30 {
            drawn += 1;
        }
    }
    let total = (W * H) as usize;
    assert!(drawn > total / 20, "too few pixels drawn ({drawn}/{total})");

    // Differences should be confined to AA edges/borders: well under 6% of all
    // pixels. (Each 90×50 button is ~4500px × 12 = 54k drawn; their ~1px rounded
    // outlines/corners are a few thousand px total.)
    let frac = differing as f64 / total as f64;
    assert!(
        frac < 0.06,
        "instanced vs immediate differ in {:.2}% of pixels (>{:.0}%) — not just AA edges",
        frac * 100.0,
        6.0
    );
}
