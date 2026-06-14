//! Headless offscreen render of MSDF text → PNG, for visual parity inspection.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test msdf_render -- --ignored --nocapture
//! ```
//! Writes `test_output/msdf_render.png`. Eyeball it to confirm fill crispness and
//! glyph placement across sizes before declaring fill parity.

use wgpu_gameui::{DrawList, TextBlock, UiRenderer};

const W: u32 = 512;
const H: u32 = 256;

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_text_to_png() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter available");

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("msdf test device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("msdf target"),
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

    // Readback buffer (bytes_per_row must be 256-aligned; W*4 = 2048 already is).
    let bytes_per_row = W * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut list = DrawList::with_font_system(font_system.clone());
    list.text(
        TextBlock::new("Ag", 16.0, 4.0)
            .with_size(64.0)
            .with_color(255, 255, 255),
    );
    // Outlined (white fill, black 2px outline).
    list.text(
        TextBlock::new("Outlined", 150.0, 20.0)
            .with_size(40.0)
            .with_color(255, 255, 255)
            .with_outline(0, 0, 0, 255, 2.5),
    );
    list.text(
        TextBlock::new("Hello World 0123", 16.0, 90.0)
            .with_size(28.0)
            .with_color(255, 255, 255),
    );
    // Drop shadow.
    list.text(
        TextBlock::new("Shadowed text", 300.0, 88.0)
            .with_size(24.0)
            .with_color(255, 255, 255)
            .with_shadow(0, 0, 0, 200, 2.0, 2.0, 1.5),
    );
    list.text(
        TextBlock::new("The quick brown fox jumps", 16.0, 140.0)
            .with_size(18.0)
            .with_color(210, 220, 240),
    );
    // Glow.
    list.text(
        TextBlock::new("Glowing", 360.0, 135.0)
            .with_size(26.0)
            .with_color(255, 255, 255)
            .with_glow(80, 180, 255, 255, 2.5),
    );
    list.text(
        TextBlock::new("small 12px: the quick brown fox.,!?@#", 16.0, 180.0)
            .with_size(12.0)
            .with_color(255, 230, 150),
    );
    // Small outlined text — checks effect reach at UI font sizes.
    list.text(
        TextBlock::new("14px outlined", 16.0, 210.0)
            .with_size(14.0)
            .with_color(255, 255, 255)
            .with_outline(20, 20, 20, 255, 1.5),
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("encoder"),
    });
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.06,
                        b: 0.08,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    ui.render(&device, &queue, &mut encoder, &view, (W, H), 1.0, &list);

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

    std::fs::create_dir_all("test_output").unwrap();
    let img = image::RgbaImage::from_raw(W, H, data.to_vec()).expect("image from raw");
    img.save("test_output/msdf_render.png").expect("save png");
    eprintln!("wrote test_output/msdf_render.png");

    // Sanity: at least some pixels are not the clear color (text actually drew).
    let clear = [13u8, 15, 20];
    let drew = img.pixels().any(|p| {
        let d = (p.0[0] as i32 - clear[0] as i32).abs()
            + (p.0[1] as i32 - clear[1] as i32).abs()
            + (p.0[2] as i32 - clear[2] as i32).abs();
        d > 30
    });
    assert!(
        drew,
        "no text pixels rendered — pipeline produced an empty frame"
    );
}
