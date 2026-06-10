//! Headless offscreen render of all widgets → PNG for visual inspection.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test widget_gallery -- --ignored --nocapture
//! ```
//! Writes `test_output/widget_gallery.png`.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Checkbox, ColumnWidth, DragCapture, ImageButton, ImageFit, InputState, LayerStack, ProgressBar,
    ScrollState, ScrollView, Slider, Table, TableCell, TableColumn, Tabs, TextAlign, TextBlock,
    TextInput, Theme, TooltipContent, TooltipLayer, UiRenderer,
};

const W: u32 = 800;
const H: u32 = 950;

fn checkerboard_pixels(size: u32, a: [u8; 4], b: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let cell = ((x / 4) + (y / 4)) % 2;
            let c = if cell == 0 { a } else { b };
            out.extend_from_slice(&c);
        }
    }
    out
}

fn solid_with_border(size: u32, fill: [u8; 4], border: [u8; 4], thickness: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let on_border = x < thickness
                || y < thickness
                || x >= size - thickness
                || y >= size - thickness;
            let c = if on_border { border } else { fill };
            let idx = ((y * size + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_widget_gallery() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter available");

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("gallery device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    // Upload test sprites.
    let icon_pixels = checkerboard_pixels(32, [220, 80, 80, 255], [40, 40, 40, 255]);
    let icon_sprite = ui.load_sprite_rgba8("icon", 32, 32, &icon_pixels);
    let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
    let frame_sprite = ui.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
    let nine_slice_id = ui.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gallery target"),
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

    // bytes_per_row must be 256-aligned for wgpu copy.
    let row_stride = W * 4;
    let bytes_per_row = (row_stride + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // Use a LayerStack so we can demo tooltip layers.
    let mut layers = LayerStack::new();
    let theme = Theme::default();
    let mut input = InputState::default();
    input.mouse_x = 200.0;
    input.mouse_y = 100.0;

    // =====================================================================
    // Draw all base-layer widgets in a scope so the `list` borrow on `layers`
    // is released before we add tooltip layers.
    // =====================================================================
    {
    let list = layers.base_mut();

    // =====================================================================
    // Title
    // =====================================================================
    list.text(
        TextBlock::new("wgpu-gameui Widget Gallery", 20.0, 16.0)
            .with_size(24.0)
            .with_color(255, 255, 255),
    );

    // =====================================================================
    // Row 1: Basic primitives (y ~ 50)
    // =====================================================================
    list.rounded_rect(Rect::new(20.0, 50.0, 120.0, 50.0), 8.0, [0.25, 0.40, 0.65, 1.0]);
    list.text(
        TextBlock::new("RoundedRect", 28.0, 65.0)
            .with_size(12.0)
            .with_color(255, 255, 255),
    );

    list.line([160.0, 60.0], [260.0, 90.0], 3.0, [0.95, 0.65, 0.25, 1.0]);

    list.circle((320.0, 75.0), 25.0, [0.30, 0.70, 0.40, 1.0]);
    list.circle_outline((390.0, 75.0), 25.0, 3.0, [0.70, 0.30, 0.40, 1.0]);

    list.nine_slice_id(nine_slice_id, 460.0, 55.0, 60.0, 40.0, [1.0; 4]);
    list.icon_sprite(icon_sprite, 550.0, 60.0, 32.0, 32.0, [1.0; 4]);
    list.image(icon_sprite, Rect::new(600.0, 60.0, 32.0, 32.0), [1.0; 4]);

    // =====================================================================
    // Row 2: Text styles (y ~ 120)
    // =====================================================================
    list.text(
        TextBlock::new("Normal 16px text — The quick brown fox jumps", 20.0, 120.0)
            .with_size(16.0)
            .with_color(200, 210, 230),
    );
    list.text(
        TextBlock::new("20px white + dark outline", 20.0, 150.0)
            .with_size(20.0)
            .with_color(255, 255, 255)
            .with_outline(10, 12, 18, 255, 2.0),
    );
    list.text(
        TextBlock::new("Shadowed text (offset 1,1)", 20.0, 185.0)
            .with_size(16.0)
            .with_color(200, 220, 255)
            .with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0),
    );
    list.text(
        TextBlock::new("Glow effect", 320.0, 185.0)
            .with_size(24.0)
            .with_color(255, 255, 200)
            .with_glow(80, 200, 255, 255, 3.0),
    );
    list.text(
        TextBlock::new("Right-aligned", 480.0, 220.0)
            .with_size(14.0)
            .with_color(180, 190, 210)
            .with_max_width(150.0)
            .with_align(TextAlign::Right),
    );
    list.text(
        TextBlock::new("Centered in 150px", 480.0, 240.0)
            .with_size(14.0)
            .with_color(180, 190, 210)
            .with_max_width(150.0)
            .with_align(TextAlign::Center),
    );

    // =====================================================================
    // Row 3: Button, Checkbox, ProgressBar, Slider (y ~ 280)
    // =====================================================================
    let btn_rect = Rect::new(20.0, 280.0, 100.0, 32.0);
    list.rounded_rect(btn_rect, 6.0, [0.30, 0.55, 0.85, 1.0]);
    list.text(
        TextBlock::new("Button", 44.0, 287.0)
            .with_size(14.0)
            .with_color(255, 255, 255),
    );

    // Checkboxes: draw(&self, checked, label, rect, list, theme, input)
    let cb = Checkbox::new();
    cb.draw(false, "Option A", Rect::new(140.0, 285.0, 100.0, 20.0), &mut *list, &theme, &input);
    cb.draw(true, "Option B (checked)", Rect::new(140.0, 310.0, 160.0, 20.0), &mut *list, &theme, &input);

    // ProgressBar: draw(&self, rect, list, theme)
    let pb = ProgressBar::new(0.65);
    pb.draw(Rect::new(320.0, 285.0, 150.0, 20.0), &mut *list, &theme);

    // Slider: draw(&self, value, id, capture, rect, list, theme, input)
    let slider_val = 40.0;
    let mut capture = DragCapture::default();
    let slider = Slider::new(0.0, 100.0);
    slider.draw(
        slider_val,
        0,
        &mut capture,
        Rect::new(300.0, 330.0, 150.0, 20.0),
        &mut *list,
        &theme,
        &input,
    );
    list.text(
        TextBlock::new(&format!("Slider val: {:.0}", slider_val), 470.0, 332.0)
            .with_size(12.0)
            .with_color(200, 210, 230),
    );

    // =====================================================================
    // Row 4: Tabs (y ~ 380)
    // =====================================================================
    let tabs = Tabs::new(&["Tab A", "Tab B", "Tab C"]);
    let _output = tabs.draw(
        Rect::new(20.0, 380.0, 300.0, 30.0),
        0,
        &mut *list,
        &theme,
        &mut input,
    );

    // =====================================================================
    // Row 5: TextInput (y ~ 420)
    // =====================================================================
    {
        let mut ti = TextInput::new(20.0, 425.0, 220.0, 28.0)
            .with_value("Hello, wgpu-gameui!")
            .with_focused(true);
        let _ = ti.draw(&mut *list, &theme, &input);
    }
    {
        let mut ti = TextInput::new(260.0, 425.0, 200.0, 28.0)
            .with_placeholder("Placeholder...");
        let _ = ti.draw(&mut *list, &theme, &input);
    }

    // =====================================================================
    // Row 6: ScrollView (y ~ 470)
    // =====================================================================
    let scroll_viewport = Rect::new(20.0, 485.0, 180.0, 100.0);
    list.rounded_rect(scroll_viewport, 4.0, [0.06, 0.07, 0.10, 1.0]);
    let mut scroll_state = ScrollState::default();
    scroll_state.content_size = [160.0, 300.0];
    ScrollView::new(scroll_viewport)
        .vertical_only()
        .draw(
            &mut scroll_state,
            &mut *list,
            &theme,
            &mut input,
            |list, vp| {
                for i in 0..12usize {
                    let y = vp.y + i as f32 * 22.0;
                    let bg = if i % 2 == 0 {
                        [0.16, 0.18, 0.24, 1.0]
                    } else {
                        [0.10, 0.12, 0.18, 1.0]
                    };
                    list.quad(vp.x + 4.0, y + 2.0, vp.width - 12.0, 18.0, bg);
                    list.text(
                        TextBlock::new(&format!("Item #{:02}", i), vp.x + 10.0, y + 3.0)
                            .with_size(12.0)
                            .with_color(180, 190, 210),
                    );
                }
            },
        );

    // =====================================================================
    // Row 7: Table (y ~ 480 alongside scrollview)
    // =====================================================================
    {
        let columns = &[
            TableColumn::new("Name", ColumnWidth::Fixed(100.0)),
            TableColumn::new("Score", ColumnWidth::Fixed(60.0)),
            TableColumn::new("Status", ColumnWidth::Flex(1.0)),
        ];
        let rows = vec![
            vec![
                TableCell::new("Alice"),
                TableCell::new("95"),
                TableCell::new("Pass"),
            ],
            vec![
                TableCell::new("Bob"),
                TableCell::new("72"),
                TableCell::new("Pass"),
            ],
            vec![
                TableCell::new("Charlie"),
                TableCell::new("48"),
                TableCell::new("Fail"),
            ],
        ];
        let table_rect = Rect::new(220.0, 485.0, 270.0, 80.0);
        list.rounded_rect(table_rect, 4.0, [0.06, 0.07, 0.10, 1.0]);
        Table::new(columns).draw(
            table_rect,
            &rows,
            &mut ScrollState::default(),
            &mut *list,
            &theme,
            &mut input,
        );
    }

    // =====================================================================
    // Row 8: Rect outline (y ~ 595)
    // =====================================================================
    list.rect_outline(Rect::new(20.0, 600.0, 100.0, 30.0), 2.0, [0.70, 0.30, 0.40, 1.0]);
    list.text(
        TextBlock::new("Rect outline", 28.0, 606.0)
            .with_size(12.0)
            .with_color(200, 210, 230),
    );

    // ImageButton: chrome / bare / disabled variants (draw returns clicked).
    ImageButton::sprite(icon_sprite)
        .fit(ImageFit::Contain)
        .natural_size(32.0, 32.0)
        .draw(Rect::new(200.0, 595.0, 40.0, 40.0), &mut *list, &theme, &input);
    ImageButton::sprite(icon_sprite)
        .bare()
        .fit(ImageFit::Contain)
        .natural_size(32.0, 32.0)
        .draw(Rect::new(250.0, 595.0, 40.0, 40.0), &mut *list, &theme, &input);
    ImageButton::sprite(icon_sprite)
        .enabled(false)
        .fit(ImageFit::Contain)
        .natural_size(32.0, 32.0)
        .draw(Rect::new(300.0, 595.0, 40.0, 40.0), &mut *list, &theme, &input);
    list.text(
        TextBlock::new("ImageButton: chrome / bare / disabled", 200.0, 640.0)
            .with_size(11.0)
            .with_color(180, 190, 210),
    );

    // =====================================================================
    // Row 9: Labels / title helpers (y ~ 645)
    // =====================================================================
    list.text(
        TextBlock::new("label_at + title_at (panel helpers)", 20.0, 645.0)
            .with_size(14.0)
            .with_color(180, 190, 210),
    );
    wgpu_gameui::label_at(
        &mut *list,
        &theme,
        "Label: some value",
        Rect::new(20.0, 665.0, 200.0, 20.0),
    );
    wgpu_gameui::title_at(
        &mut *list,
        &theme,
        "Title: Section Header",
        Rect::new(20.0, 690.0, 250.0, 22.0),
    );
    wgpu_gameui::label_centered_at(
        &mut *list,
        &theme,
        "Centered Label",
        Rect::new(300.0, 665.0, 160.0, 20.0),
    );

    // =====================================================================
    // Row 10: Image cropped (y ~ 730)
    // =====================================================================
    list.image_cropped(
        icon_sprite,
        Rect::new(20.0, 735.0, 40.0, 40.0),
        [0.0, 0.0, 0.5, 0.5],
        [1.0; 4],
    );
    list.text(
        TextBlock::new("Cropped (TL quarter)", 68.0, 748.0)
            .with_size(11.0)
            .with_color(180, 190, 210),
    );

    // =====================================================================
    // Row 11: Tooltip layer (y ~ 790)
    // =====================================================================
    } // drop `list` borrow on layers before tooltip layer ops
    {
        let list = layers.base_mut();
        list.rounded_rect(Rect::new(20.0, 800.0, 120.0, 24.0), 4.0, [0.25, 0.30, 0.40, 1.0]);
        list.text(
            TextBlock::new("Hoverable region", 26.0, 803.0)
                .with_size(11.0)
                .with_color(200, 210, 230),
        );
    } // drop `list` so we can borrow layers mutably again

    // Build and draw tooltip layer (no concurrent `list` borrow on layers).
    {
        let mut tooltip = TooltipLayer::new();
        tooltip.register(
            Rect::new(20.0, 800.0, 120.0, 24.0),
            TooltipContent::text("This is a tooltip!"),
        );
        let mut tip_input = InputState::default();
        tip_input.mouse_x = 50.0;
        tip_input.mouse_y = 810.0;
        tooltip.tick(999.0, &tip_input);
        tooltip.draw_into_layers(&mut layers, &tip_input, &theme, 50.0, 810.0);
    }

    // =====================================================================
    // Render
    // =====================================================================
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

    ui.render_layers(
        &device,
        &queue,
        &mut encoder,
        &view,
        (W, H),
        &layers,
    );

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

    // De-pad: the GPU buffer rows are 256-aligned (`bytes_per_row`), but a
    // tightly-packed RGBA image expects `row_stride` (W*4) per row. Copy each
    // row's real bytes, dropping the alignment padding — otherwise every row
    // drifts by the padding amount and the image shears diagonally.
    let row_stride = (W * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * H as usize);
    for row in 0..H as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }

    std::fs::create_dir_all("test_output").unwrap();
    let img = image::RgbaImage::from_raw(W, H, pixels).expect("image from raw");
    img.save("test_output/widget_gallery.png").expect("save png");
    eprintln!("wrote test_output/widget_gallery.png");

    // Sanity: at least some pixels are not the clear color.
    let clear = [13u8, 15, 20];
    let drew = img.pixels().any(|p| {
        let d = (p.0[0] as i32 - clear[0] as i32).abs()
            + (p.0[1] as i32 - clear[1] as i32).abs()
            + (p.0[2] as i32 - clear[2] as i32).abs();
        d > 30
    });
    assert!(
        drew,
        "no widget pixels rendered — pipeline produced an empty frame"
    );
}
