//! Headless offscreen render of span-coloured text and underlines → PNG.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! DISPLAY=:0 cargo test -p wgpu-gameui --test span_text -- --ignored --nocapture
//! ```
//! Writes `test_output/span_text.png`. Eyeball it to confirm:
//! - "RED" appears in red, "WHITE" in white, "BLUE" in blue (top row).
//! - The second row shows a yellow underline beneath an underlined word.

use wgpu_gameui::{DrawList, TextBlock, TextSpan, UiRenderer};

const W: u32 = 512;
const H: u32 = 256;

fn gpu_setup() -> Option<(wgpu::Device, wgpu::Queue, wgpu::TextureFormat)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("span test device"),
            ..Default::default()
        },
        None,
    ))
    .ok()?;
    Some((device, queue, wgpu::TextureFormat::Rgba8UnormSrgb))
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_span_colours_and_underline() {
    let Some((device, queue, format)) = gpu_setup() else {
        eprintln!("no GPU adapter — skipping span_text test");
        return;
    };

    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("span target"),
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

    let bytes_per_row = W * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut list = DrawList::with_font_system(font_system.clone());

    // Row 1: three colour spans — "RED" (red), " | " (white), "BLUE" (blue).
    list.text(
        TextBlock::new("", 16.0, 10.0)
            .with_size(32.0)
            .with_color(255, 255, 255)
            .with_spans(vec![
                TextSpan {
                    text: "RED".into(),
                    color: Some([1.0, 0.0, 0.0, 1.0]),
                    underline: None,
                },
                TextSpan {
                    text: " | ".into(),
                    color: Some([1.0, 1.0, 1.0, 1.0]),
                    underline: None,
                },
                TextSpan {
                    text: "BLUE".into(),
                    color: Some([0.0, 0.3, 1.0, 1.0]),
                    underline: None,
                },
            ]),
    );

    // Row 2: underline — "normal " (white, no underline) + "underlined" (white,
    // yellow underline) + " text" (white, no underline).
    list.text(
        TextBlock::new("", 16.0, 80.0)
            .with_size(28.0)
            .with_color(255, 255, 255)
            .with_spans(vec![
                TextSpan {
                    text: "normal ".into(),
                    color: None,
                    underline: None,
                },
                TextSpan {
                    text: "underlined".into(),
                    color: None,
                    underline: Some([1.0, 0.9, 0.0, 1.0]), // yellow underline
                },
                TextSpan {
                    text: " text".into(),
                    color: None,
                    underline: None,
                },
            ]),
    );

    // Row 3: plain block (no spans) — regression check that span-free blocks
    // still render with their original colour.
    list.text(
        TextBlock::new("Plain (no spans)", 16.0, 150.0)
            .with_size(24.0)
            .with_color(200, 220, 200),
    );

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });
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
    img.save("test_output/span_text.png").expect("save png");
    eprintln!("wrote test_output/span_text.png");

    // Sanity: at least some pixels differ from the clear colour (text actually rendered).
    let clear = [13u8, 15, 20];
    let rendered = img.pixels().any(|p| {
        let d = (p.0[0] as i32 - clear[0] as i32).abs()
            + (p.0[1] as i32 - clear[1] as i32).abs()
            + (p.0[2] as i32 - clear[2] as i32).abs();
        d > 30
    });
    assert!(
        rendered,
        "no pixels rendered — span text pipeline produced an empty frame"
    );

    // The first row contains red text: verify that somewhere in the top half
    // of the image (rows 0–60) there is a pixel with R channel clearly dominant.
    let has_red_dominant = (0..60_u32)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .any(|(x, y)| {
            let p = img.get_pixel(x, y);
            let r = p.0[0] as i32;
            let g = p.0[1] as i32;
            let b = p.0[2] as i32;
            r > 60 && r > g + 30 && r > b + 30
        });
    assert!(
        has_red_dominant,
        "no red-dominant pixel found in the top rows — colour spans may not be working"
    );

    // The underline row (y ≈ 100–115) should have yellow pixels (R≈G≫B).
    // We look for any pixel with R>100, G>80, B<50 in that band.
    let has_yellow_underline =
        (95..120_u32)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .any(|(x, y)| {
                let p = img.get_pixel(x, y);
                p.0[0] > 100 && p.0[1] > 70 && (p.0[2] as i32) < 50
            });
    assert!(
        has_yellow_underline,
        "no yellow underline pixels found — underline may not be rendering"
    );
}
