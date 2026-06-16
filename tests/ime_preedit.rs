//! Headless offscreen render of a focused, *composing* `TextInput` → PNG.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! DISPLAY=:0 cargo test -p wgpu-gameui --test ime_preedit -- --ignored --nocapture
//! ```
//! Writes `test_output/ime_preedit.png`. The field should show the inline
//! preedit string underlined (IME composition convention), with the field's
//! committed value spliced around the insertion point.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawContext, DrawList, FocusState, InputState, TextInput, Theme, UiRenderer};

const W: u32 = 512;
const H: u32 = 128;

fn gpu_setup() -> Option<(wgpu::Device, wgpu::Queue, wgpu::TextureFormat)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("ime test device"),
            ..Default::default()
        },
        None,
    ))
    .ok()?;
    Some((device, queue, wgpu::TextureFormat::Rgba8UnormSrgb))
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_focused_composing_text_input() {
    let Some((device, queue, format)) = gpu_setup() else {
        eprintln!("no GPU adapter — skipping ime_preedit test");
        return;
    };

    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ime target"),
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

    let theme = Theme::default();

    // A focused text field that is composing: the IME has delivered the preedit
    // "ンゴ" but it is not yet committed into the value "ab|cd".
    let mut input = InputState::default();
    input.preedit = "ンゴ".to_string();

    let mut focus = FocusState::new();
    const FIELD_ID: u64 = 0;
    focus.focus(FIELD_ID);
    focus.begin_frame(&input);

    let mut list = DrawList::with_font_system(font_system.clone());

    // Field rect, leaving margin so the underline band sits well inside the image.
    let field = Rect {
        x: 24.0,
        y: 40.0,
        width: 460.0,
        height: 40.0,
    };
    let mut input_widget =
        TextInput::new(field.x, field.y, field.width, field.height).with_value("abcd");
    // Place the caret between "ab" and "cd" (byte 2) so the preedit splices in
    // the middle: "ab" + preedit(underlined) + "cd".
    input_widget.cursor_pos = 2;
    {
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, W as f32, H as f32);
        input_widget.draw(FIELD_ID, &mut ctx);
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
    img.save("test_output/ime_preedit.png").expect("save png");
    eprintln!("wrote test_output/ime_preedit.png");

    // (a) Text actually rendered (some pixels differ from the clear colour).
    let clear = [13u8, 15, 20];
    let rendered = img.pixels().any(|p| {
        let d = (p.0[0] as i32 - clear[0] as i32).abs()
            + (p.0[1] as i32 - clear[1] as i32).abs()
            + (p.0[2] as i32 - clear[2] as i32).abs();
        d > 30
    });
    assert!(
        rendered,
        "no pixels rendered — composing TextInput produced an empty frame"
    );

    // (b) The underline is a thin SOLID horizontal bar beneath the preedit run,
    // unlike the sparse glyph pixels above it. Scan rows in the lower half of
    // the field for the longest contiguous run of near-text-coloured pixels;
    // a real underline produces a run much longer than any glyph stroke.
    //
    // theme.text ≈ [0.9, 0.9, 0.95] → bright, low chroma. Match bright pixels.
    let is_textish = |p: &image::Rgba<u8>| p.0[0] > 150 && p.0[1] > 150 && p.0[2] > 150;

    let field_x0 = field.x as u32;
    let field_x1 = (field.x + field.width).min(W as f32) as u32;
    // Lower portion of the field — the underline sits just below the baseline.
    let y_lo = (field.y + field.height * 0.45) as u32;
    let y_hi = (field.y + field.height).min(H as f32) as u32;

    let mut best_run = 0u32;
    for y in y_lo..y_hi {
        let mut run = 0u32;
        for x in field_x0..field_x1 {
            if is_textish(img.get_pixel(x, y)) {
                run += 1;
                best_run = best_run.max(run);
            } else {
                run = 0;
            }
        }
    }

    // An underline under a two-glyph preedit at ~28px is comfortably > 20px wide
    // of solid colour. Glyph strokes break up far sooner.
    assert!(
        best_run >= 16,
        "no solid horizontal underline run found in the field's lower half \
         (longest run = {best_run}px) — IME preedit underline may not be rendering",
    );
}
