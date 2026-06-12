//! Headless offscreen render of a focused, multi-line `TextInput` → PNG.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! DISPLAY=:0 cargo test -p wgpu-gameui --test multiline_text_input -- --ignored --nocapture
//! ```
//! Writes `test_output/multiline_text_input.png`. The field should show several
//! text lines (hard newlines + one wrapped line) with a caret, and — when the
//! caret is driven to the bottom of a value taller than the box — a non-zero
//! `scroll_offset` (autoscroll engaged).

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawContext, DrawList, FocusState, InputState, TextInput, Theme, UiRenderer};

// W * 4 must be a multiple of COPY_BYTES_PER_ROW_ALIGNMENT (256), so W is a
// multiple of 64.
const W: u32 = 384;
const H: u32 = 160;

fn gpu_setup() -> Option<(wgpu::Device, wgpu::Queue, wgpu::TextureFormat)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("multiline test device"),
            ..Default::default()
        },
        None,
    ))
    .ok()?;
    Some((device, queue, wgpu::TextureFormat::Rgba8UnormSrgb))
}

/// Render the given field once into an RGBA image.
fn render_field(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    format: wgpu::TextureFormat,
    font_system: &wgpu_gameui::FontSystemHandle,
    field: &mut TextInput,
    focus: &mut FocusState,
    field_id: u64,
    input: &InputState,
) -> image::RgbaImage {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("multiline target"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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

    let theme = Theme::default();
    focus.begin_frame(input);
    let mut list = DrawList::with_font_system(font_system.clone());
    {
        let mut ctx = DrawContext::new(&mut list, focus, &theme, input, W as f32, H as f32);
        field.draw(field_id, &mut ctx);
    }

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.06, b: 0.08, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    ui.render(device, queue, &mut encoder, &view, (W, H), 1.0, &list);
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
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    image::RgbaImage::from_raw(W, H, data.to_vec()).expect("image from raw")
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_focused_multiline_text_input() {
    let Some((device, queue, format)) = gpu_setup() else {
        eprintln!("no GPU adapter — skipping multiline_text_input test");
        return;
    };

    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    const FIELD_ID: u64 = 0;
    let field_rect = Rect { x: 16.0, y: 16.0, width: 320.0, height: 100.0 };

    // A focused multiline field with two hard newlines and a long line that
    // wraps at the field width.
    let mut field = TextInput::new(field_rect.x, field_rect.y, field_rect.width, field_rect.height)
        .with_multiline(true)
        .with_value("line one\nsecond line\nthis third line is long enough to wrap onto another row");
    field.cursor_pos = 0; // caret at the very top first

    let mut focus = FocusState::new();
    focus.focus(FIELD_ID);

    let input = InputState::default();
    let img = render_field(
        &device, &queue, &mut ui, format, &font_system, &mut field, &mut focus, FIELD_ID, &input,
    );

    std::fs::create_dir_all("test_output").unwrap();
    img.save("test_output/multiline_text_input.png").expect("save png");
    eprintln!("wrote test_output/multiline_text_input.png");

    // (a) Text rendered across MULTIPLE vertical bands — count rows (inside the
    // field) that contain bright text pixels, then require ≥3 distinct bands
    // separated by gaps (one band per visual line). Inset past the bright focus
    // border (its vertical sides would otherwise mark every row), and require a
    // handful of bright pixels per row so a stray edge pixel isn't a "band".
    let is_textish = |p: &image::Rgba<u8>| p.0[0] > 150 && p.0[1] > 150 && p.0[2] > 150;
    let x0 = (field_rect.x + 8.0) as u32;
    let x1 = (field_rect.x + field_rect.width - 8.0).min(W as f32) as u32;
    let y0 = (field_rect.y + 4.0) as u32;
    let y1 = (field_rect.y + field_rect.height - 4.0).min(H as f32) as u32;

    let mut bands = 0u32;
    let mut in_band = false;
    for y in y0..y1 {
        let count = (x0..x1).filter(|&x| is_textish(img.get_pixel(x, y))).count();
        let row_has_text = count >= 4;
        if row_has_text && !in_band {
            bands += 1;
            in_band = true;
        } else if !row_has_text {
            in_band = false;
        }
    }
    assert!(
        bands >= 3,
        "expected ≥3 text bands (multi-line render); found {bands}",
    );

    // (b) Autoscroll engages when the caret is driven to the bottom of a value
    // taller than the box. Type enough newlines to exceed the field height, then
    // confirm scroll_offset became positive.
    let mut typed = InputState::default();
    typed.enter_pressed = true;
    // Drive the caret to the end first.
    field.cursor_pos = field.value.len();
    // A handful of Enter presses (each one frame) to push the caret below the box.
    for _ in 0..6 {
        let _ = render_field(
            &device, &queue, &mut ui, format, &font_system, &mut field, &mut focus, FIELD_ID,
            &typed,
        );
    }
    assert!(
        field.scroll_offset > 0.0,
        "autoscroll should engage once the caret falls below the box (scroll_offset = {})",
        field.scroll_offset,
    );
}
