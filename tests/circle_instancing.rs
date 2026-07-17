//! Headless GPU golden-pixel test for the instanced SDF circle path.
//!
//! `DrawList::circle` / `circle_outline` rasterize a disc (or ring) from a
//! signed distance field in one instanced draw, instead of tessellating a
//! triangle fan into the colored soup. There is no axis-aligned soup reference
//! to diff against (the public primitives always instance under a translate-only
//! transform), so this is a *geometric* golden test: it renders a filled disc
//! and a ring and samples known points (center / inside / on-ring / outside) to
//! confirm the SDF produces a circle of the right radius and band.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test circle_instancing -- --ignored
//! ```

use wgpu_gameui::{DrawList, FontSystemHandle, UiRenderer};

const W: u32 = 240;
const H: u32 = 240;

/// Render a single `DrawList` to an RGBA image (tightly packed, W*4 per row).
fn render_list(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    list: &DrawList,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("circle target"),
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
        label: Some("circle readback"),
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
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
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
    device.poll(wgpu::PollType::Poll);
    let data = slice.get_mapped_range().unwrap();

    let row_stride = (W * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * H as usize);
    for row in 0..H as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }
    pixels
}

/// Sample the RGB of one pixel.
fn px(img: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2]]
}

fn is_red(p: [u8; 3]) -> bool {
    p[0] > 180 && p[1] < 70 && p[2] < 70
}
fn is_green(p: [u8; 3]) -> bool {
    p[1] > 150 && p[0] < 70 && p[2] < 70
}
fn is_black(p: [u8; 3]) -> bool {
    p[0] < 30 && p[1] < 30 && p[2] < 30
}

fn new_renderer() -> (wgpu::Device, wgpu::Queue, UiRenderer, FontSystemHandle) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("circle device"),
            ..Default::default()
        }
    ))
    .expect("request device");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system: FontSystemHandle = wgpu_gameui::shared_font_system();
    let ui = UiRenderer::new(&device, &queue, format, font_system.clone());
    (device, queue, ui, font_system)
}

const CX: f32 = 120.0;
const CY: f32 = 120.0;
const R: f32 = 60.0;
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_circle_fill_is_a_disc() {
    let (device, queue, mut ui, font_system) = new_renderer();

    let mut list = DrawList::with_font_system(font_system.clone());
    list.circle((CX, CY), R, RED);
    let img = render_list(&device, &queue, &mut ui, &list);

    std::fs::create_dir_all("test_output").ok();
    image::RgbaImage::from_raw(W, H, img.clone())
        .and_then(|i| i.save("test_output/circle_fill.png").ok().map(|_| i));

    // Center is filled.
    assert!(
        is_red(px(&img, CX as u32, CY as u32)),
        "center should be red"
    );
    // Well inside the radius is filled.
    assert!(
        is_red(px(&img, (CX + R * 0.5) as u32, CY as u32)),
        "inside radius should be red"
    );
    // Well outside the radius is background.
    assert!(
        is_black(px(&img, (CX + R + 12.0) as u32, CY as u32)),
        "outside radius should be background"
    );
    // The corner of the bounding box is outside the disc → background.
    assert!(
        is_black(px(&img, (CX + R - 2.0) as u32, (CY + R - 2.0) as u32)),
        "bounding-box corner should be background (disc, not square)"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_circle_outline_is_a_ring() {
    let (device, queue, mut ui, font_system) = new_renderer();

    let thickness = 10.0_f32;
    let mut list = DrawList::with_font_system(font_system.clone());
    list.circle_outline((CX, CY), R, thickness, GREEN);
    let img = render_list(&device, &queue, &mut ui, &list);

    std::fs::create_dir_all("test_output").ok();
    image::RgbaImage::from_raw(W, H, img.clone())
        .and_then(|i| i.save("test_output/circle_outline.png").ok().map(|_| i));

    // Center is hollow.
    assert!(
        is_black(px(&img, CX as u32, CY as u32)),
        "ring center should be background"
    );
    // On the ring (at radius R) is filled.
    assert!(
        is_green(px(&img, (CX + R) as u32, CY as u32)),
        "ring at radius should be green"
    );
    // Well inside the ring band is hollow.
    assert!(
        is_black(px(&img, (CX + R - thickness - 8.0) as u32, CY as u32)),
        "inside the band should be background"
    );
    // Well outside the ring band is hollow.
    assert!(
        is_black(px(&img, (CX + R + thickness) as u32, CY as u32)),
        "outside the band should be background"
    );
}
