//! Headless offscreen render of all widgets → PNG for visual inspection.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test widget_gallery -- --ignored --nocapture
//! ```
//! Writes `test_output/widget_gallery.png`.
//!
//! Layout is driven by [`Flow`] — a left-to-right, wrapping grid of labeled
//! cells. Each preview reserves a cell (which draws its label) and gets back a
//! content `Rect` to draw into, so adding a widget is one `flow.cell(...)` call
//! plus the widget's own draw call — no hand-placed coordinates.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Button, Checkbox, ColumnWidth, DragCapture, DrawContext, DrawList, Dropdown, DropdownState,
    FocusState, ImageButton, ImageFit, InputState, LayerStack, ProgressBar, ScrollState,
    ScrollView, Slider, Table, TableCell, TableColumn, Tabs,
    TextAlign, TextBlock, TextInput, Theme, TooltipContent, TooltipLayer, UiContext, UiRenderer,
    UiState, SLIDER_SCRUBBER_ICON, SLIDER_TRACK_NINE_SLICE,
};

/// Convenience: build a DrawContext for a single draw call in the gallery.
fn ctx<'a>(
    list: &'a mut DrawList,
    focus: &'a mut FocusState,
    theme: &'a Theme,
    input: &'a InputState,
) -> DrawContext<'a> {
    DrawContext::new(list, focus, theme, input, W as f32, 600.0)
}

const W: u32 = 800;
const LABEL_H: f32 = 16.0;
const LABEL_SIZE: f32 = 11.0;
/// Rough advance width per character at `LABEL_SIZE`, used so a long label
/// reserves enough horizontal room to not collide with the next cell.
const LABEL_CHAR_W: f32 = 6.0;

/// A wrapping grid of labeled preview cells.
///
/// `cell` reserves a `w`×`h` content box, draws its label just above it, and
/// returns the content `Rect`. Cells flow left-to-right and wrap when they run
/// past `max_x`. `section` breaks to a new row and draws a header.
struct Flow {
    x0: f32,
    y0: f32,
    max_x: f32,
    cur_x: f32,
    cur_y: f32,
    row_h: f32,
    col_gap: f32,
    row_gap: f32,
}

impl Flow {
    fn new(x0: f32, y0: f32, max_x: f32) -> Self {
        Self {
            x0,
            y0,
            max_x,
            cur_x: x0,
            cur_y: y0,
            row_h: 0.0,
            col_gap: 22.0,
            row_gap: 16.0,
        }
    }

    /// Break to a new row and draw a section header.
    fn section(&mut self, list: &mut DrawList, title: &str) {
        if self.cur_x > self.x0 {
            self.cur_y += self.row_h;
        }
        // Extra gap above a header (except the very first one).
        if self.cur_y > self.y0 {
            self.cur_y += self.row_gap * 1.5;
        }
        self.cur_x = self.x0;
        self.row_h = 0.0;
        list.text(
            TextBlock::new(title, self.x0, self.cur_y)
                .with_size(15.0)
                .with_color(120, 180, 255),
        );
        self.cur_y += 24.0;
    }

    /// Reserve a labeled `w`×`h` content cell; returns the content rect.
    fn cell(&mut self, list: &mut DrawList, label: &str, w: f32, h: f32) -> Rect {
        // A cell is as wide as its content or its label, whichever is larger,
        // so labels never overlap the neighbouring cell.
        let cell_w = w.max(label.chars().count() as f32 * LABEL_CHAR_W);
        if self.cur_x + cell_w > self.max_x && self.cur_x > self.x0 {
            self.cur_x = self.x0;
            self.cur_y += self.row_h + self.row_gap;
            self.row_h = 0.0;
        }
        list.text(
            TextBlock::new(label, self.cur_x, self.cur_y)
                .with_size(LABEL_SIZE)
                .with_color(150, 160, 180),
        );
        let content = Rect::new(self.cur_x, self.cur_y + LABEL_H, w, h);
        self.cur_x += cell_w + self.col_gap;
        self.row_h = self.row_h.max(LABEL_H + h);
        content
    }

    /// The y just below all content drawn so far.
    fn bottom(&self) -> f32 {
        self.cur_y + self.row_h
    }
}

fn solid_with_border(size: u32, fill: [u8; 4], border: [u8; 4], thickness: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let on_border =
                x < thickness || y < thickness || x >= size - thickness || y >= size - thickness;
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

    // Real PNG art from assets/ — decoded through the same `load_image_file`
    // path the game uses, so the gallery doubles as a smoke test for it.
    let load = |ui: &mut UiRenderer, name: &str| {
        ui.load_image_file(format!("assets/{name}.png"))
            .unwrap_or_else(|e| panic!("load assets/{name}.png: {e:?}"))
    };
    let duck = load(&mut ui, "rubberduck");
    let ball = load(&mut ui, "soccerball");
    let car = load(&mut ui, "toycar");
    let board = load(&mut ui, "skateboard");
    let snow = load(&mut ui, "snowflake");
    let suitcase = load(&mut ui, "suitcase");

    // Synthetic sprites for primitives that show off tinting / nine-slice.
    let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
    let frame_sprite = ui.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
    let nine_slice_id = ui.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);

    // Slider assets: it draws its track via the `SLIDER_TRACK_NINE_SLICE` key
    // ("track") and its knob via the `SLIDER_SCRUBBER_ICON` key. Register
    // placeholders so the slider shows in the gallery (the game ships real art).
    let track_pixels = solid_with_border(16, [40, 42, 55, 255], [90, 95, 120, 255], 2);
    let track_sprite = ui.load_sprite_rgba8("slider_track", 16, 16, &track_pixels);
    ui.register_nine_slice(SLIDER_TRACK_NINE_SLICE, track_sprite, [4, 4, 4, 4]);
    let knob_pixels = solid_with_border(16, [200, 205, 220, 255], [110, 115, 140, 255], 2);
    ui.load_sprite_rgba8(SLIDER_SCRUBBER_ICON, 16, 16, &knob_pixels);

    let mut layers = LayerStack::new();
    let theme = Theme::default();
    let mut input = InputState::default();

    // Focus owner for the text inputs. Seed the first as focused so the rendered
    // PNG shows a caret; `begin_frame`/`end_frame` bracket the draws below.
    let mut focus = FocusState::new();
    focus.focus(0);
    focus.begin_frame(&input);

    // Dropdown owner, seeded OPEN so the PNG shows the floating option list.
    const DROPDOWN_ID: u64 = 100;
    const DROPDOWN_ITEMS: [&str; 4] = ["Red", "Green", "Blue", "Alpha"];
    let mut dropdowns = DropdownState::new();

    // Reserved by the flow inside the scope below, used afterwards. The scope
    // runs unconditionally, so deferred init is sound (and avoids a dead store).
    let tooltip_rect;
    let content_bottom;

    // =====================================================================
    // Build the base layer. The `list` borrow on `layers` is released at the
    // end of this scope so we can add tooltip layers afterwards.
    // =====================================================================
    {
        let list = layers.base_mut();

        list.text(
            TextBlock::new("wgpu-gameui Widget Gallery", 20.0, 16.0)
                .with_size(24.0)
                .with_color(255, 255, 255),
        );

        let mut flow = Flow::new(20.0, 56.0, (W as f32) - 20.0);

        // ---- Primitives -------------------------------------------------
        flow.section(list, "Primitives");

        let r = flow.cell(list, "Rounded rect", 120.0, 44.0);
        list.rounded_rect(r, 8.0, [0.25, 0.40, 0.65, 1.0]);

        let r = flow.cell(list, "Line", 90.0, 44.0);
        list.line(
            [r.x, r.y + r.height],
            [r.x + r.width, r.y],
            3.0,
            [0.95, 0.65, 0.25, 1.0],
        );

        let r = flow.cell(list, "Circle", 50.0, 50.0);
        list.circle(
            (r.x + r.width / 2.0, r.y + r.height / 2.0),
            r.width / 2.0,
            [0.30, 0.70, 0.40, 1.0],
        );

        let r = flow.cell(list, "Circle outline", 50.0, 50.0);
        list.circle_outline(
            (r.x + r.width / 2.0, r.y + r.height / 2.0),
            r.width / 2.0,
            3.0,
            [0.70, 0.30, 0.40, 1.0],
        );

        // Rect outline routes through the SDF chrome instance (transparent fill,
        // border-only band) when translate-only.
        let r = flow.cell(list, "Rect outline", 120.0, 44.0);
        list.rect_outline(r, 2.0, [0.55, 0.75, 0.95, 1.0]);

        // Rotated rounded rect: a non-translate transform falls back to the soup
        // tessellator, proving the rounded-rect primitive still draws correctly
        // off-axis (instanced fast path is translate-only).
        let r = flow.cell(list, "Rounded rect (rotated)", 120.0, 44.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.rounded_rect(
            Rect::new(-r.width / 2.0, -r.height / 2.0, r.width, r.height),
            8.0,
            [0.25, 0.40, 0.65, 1.0],
        );
        list.pop_transform();

        let r = flow.cell(list, "Nine-slice", 64.0, 44.0);
        list.nine_slice_id(nine_slice_id, r.x, r.y, r.width, r.height, [1.0; 4]);

        // Rotated nine-slice: unlike chrome, the instanced nine-slice bakes the
        // full affine into the instance (UV mapping is local-space), so rotation
        // is exact with no fallback — strictly more capable than the old soup.
        let r = flow.cell(list, "Nine-slice (rotated)", 64.0, 44.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.nine_slice_id(
            nine_slice_id,
            -r.width / 2.0,
            -r.height / 2.0,
            r.width,
            r.height,
            [1.0; 4],
        );
        list.pop_transform();

        let r = flow.cell(list, "Icon sprite", 40.0, 40.0);
        list.icon_sprite(ball, r.x, r.y, r.width, r.height, [1.0; 4]);

        let r = flow.cell(list, "Image", 40.0, 40.0);
        list.image(car, r, [1.0; 4]);

        let r = flow.cell(list, "Image (cropped)", 40.0, 40.0);
        list.image_cropped(snow, r, [0.0, 0.0, 0.5, 0.5], [1.0; 4]);

        let r = flow.cell(list, "Rect outline", 100.0, 32.0);
        list.rect_outline(r, 2.0, [0.70, 0.30, 0.40, 1.0]);

        // ---- Text -------------------------------------------------------
        flow.section(list, "Text");

        let r = flow.cell(list, "Plain", 190.0, 20.0);
        list.text(
            TextBlock::new("The quick brown fox", r.x, r.y)
                .with_size(16.0)
                .with_color(200, 210, 230),
        );

        let r = flow.cell(list, "Outline", 130.0, 24.0);
        list.text(
            TextBlock::new("Outlined", r.x, r.y)
                .with_size(20.0)
                .with_color(255, 255, 255)
                .with_outline(10, 12, 18, 255, 2.0),
        );

        let r = flow.cell(list, "Shadow", 120.0, 20.0);
        list.text(
            TextBlock::new("Shadowed", r.x, r.y)
                .with_size(16.0)
                .with_color(200, 220, 255)
                .with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0),
        );

        let r = flow.cell(list, "Glow", 120.0, 26.0);
        list.text(
            TextBlock::new("Glow", r.x, r.y)
                .with_size(24.0)
                .with_color(255, 255, 200)
                .with_glow(80, 200, 255, 255, 3.0),
        );

        let r = flow.cell(list, "Align right", 150.0, 20.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("Right", r.x, r.y + 2.0)
                .with_size(14.0)
                .with_color(180, 190, 210)
                .with_max_width(r.width)
                .with_align(TextAlign::Right),
        );

        let r = flow.cell(list, "Align center", 150.0, 20.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("Center", r.x, r.y + 2.0)
                .with_size(14.0)
                .with_color(180, 190, 210)
                .with_max_width(r.width)
                .with_align(TextAlign::Center),
        );

        // ---- Fonts ------------------------------------------------------
        // The bundled Noto Sans family (registered by `shared_font_system`)
        // resolves the default sans-serif and provides real bold/italic faces.
        flow.section(list, "Fonts");

        let r = flow.cell(list, "Regular", 130.0, 24.0);
        list.text(
            TextBlock::new("Regular", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235),
        );

        let r = flow.cell(list, "Bold", 130.0, 24.0);
        list.text(
            TextBlock::new("Bold", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .bold(),
        );

        let r = flow.cell(list, "Italic", 130.0, 24.0);
        list.text(
            TextBlock::new("Italic", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .italic(),
        );

        let r = flow.cell(list, "Bold Italic", 140.0, 24.0);
        list.text(
            TextBlock::new("Bold Italic", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .bold()
                .italic(),
        );

        // ---- Interactive verbs (UiContext) ------------------------------
        // The crate-side stateful façade: each verb places + localizes the raw
        // widget and auto-advances a vertical cursor. Rendered at rest (the
        // static InputState isn't interacting), so this just eyeballs layout +
        // crispness of the verb stack.
        flow.section(list, "Interactive verbs (UiContext)");
        {
            let r = flow.cell(list, "Stacked verbs", 200.0, 168.0);
            let mut vstate = UiState::new();
            vstate.begin_frame(&input, &theme);
            let mut buf = String::from("editable");
            {
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                ui.text("text() label");
                ui.text_button("text_button()", Some(200.0), None);
                let _ = ui.slider(0, 0.6, 0.0, 1.0, Some(200.0));
                let _ = ui.checkbox("checkbox()", true);
                let _ = ui.text_input(1, &mut buf, "type…", Some(200.0));
            }
            vstate.end_frame();
        }

        // ---- Widgets ----------------------------------------------------
        flow.section(list, "Widgets");

        let r = flow.cell(list, "Button", 100.0, 32.0);
        Button::draw_at("Button", r, true, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Button (bare)", 100.0, 32.0);
        Button::new("Bare").bare().draw(r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Button (disabled)", 110.0, 32.0);
        Button::draw_at("Disabled", r, false, &mut ctx(list, &mut focus, &theme, &input));

        let cb = Checkbox::new();
        let r = flow.cell(list, "Checkbox", 120.0, 20.0);
        cb.draw(false, "Off", r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Checkbox (checked)", 120.0, 20.0);
        cb.draw(true, "On", r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Progress bar", 150.0, 20.0);
        ProgressBar::new(0.65).draw(r, list, &theme);

        let r = flow.cell(list, "Slider", 160.0, 24.0);
        let mut capture = DragCapture::default();
        Slider::new(0.0, 100.0).draw(40.0, 0, &mut capture, r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Tabs", 240.0, 30.0);
        Tabs::new(&["Tab A", "Tab B", "Tab C"]).draw(r, 0, list, &theme, &input);

        let r = flow.cell(list, "Text input", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_value("Hello, wgpu-gameui!")
            .draw(0, &mut focus, list, &theme, &input);

        let r = flow.cell(list, "Text input (empty)", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_placeholder("Placeholder...")
            .draw(1, &mut focus, list, &theme, &input);

        // Dropdown, seeded open: the floating list (drawn after the base scope)
        // renders above whatever cells sit below it.
        let r = flow.cell(list, "Dropdown (open)", 160.0, 28.0);
        dropdowns.open_for_test(DROPDOWN_ID, r, &DROPDOWN_ITEMS, 2);
        Dropdown::new(&DROPDOWN_ITEMS, 2).draw(
            DROPDOWN_ID,
            r,
            &mut dropdowns,
            &mut DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0),
        );

        let r = flow.cell(list, "Scroll view", 180.0, 100.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        let mut scroll_state = ScrollState::default();
        scroll_state.content_size = [160.0, 300.0];
        ScrollView::new(r).vertical_only().draw(
            &mut scroll_state,
            list,
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
        let r = flow.cell(list, "Table", 270.0, 88.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        Table::new(columns).draw(r, &rows, &mut ScrollState::default(), list, &theme, &mut input);

        let r = flow.cell(list, "Image button", 40.0, 40.0);
        ImageButton::sprite(duck)
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &theme, &input);

        let r = flow.cell(list, "Image button (bare)", 40.0, 40.0);
        ImageButton::sprite(board)
            .bare()
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &theme, &input);

        let r = flow.cell(list, "Image button (disabled)", 40.0, 40.0);
        ImageButton::sprite(suitcase)
            .enabled(false)
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &theme, &input);

        // ---- Instanced chrome (SDF rounded-rect) ------------------------
        // Every `Button` already routes its background+border through the
        // instanced `chrome_rect` path; this section makes the batching
        // explicit (a strip of same-shape buttons collapses to one base mesh +
        // N instances) and shows the rotated-transform fallback still renders.
        flow.section(list, "Instanced chrome");

        for i in 0..6 {
            let r = flow.cell(list, "", 70.0, 30.0);
            Button::new(format!("#{i}")).draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

        // Rotated chrome: `chrome_rect` can't express a rotation as a single
        // axis-aligned instance, so it falls back to immediate tessellation.
        let r = flow.cell(list, "Rotated (fallback)", 80.0, 40.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.chrome_rect(
            Rect::new(-r.width / 2.0, -r.height / 2.0, r.width, r.height),
            8.0,
            2.0,
            [0.30, 0.55, 0.35, 1.0],
            [0.80, 0.90, 0.80, 1.0],
        );
        list.pop_transform();

        // Tooltip target last: its popup floats down-and-right into the empty
        // headroom below, overlapping no other widget.
        let r = flow.cell(list, "Tooltip target", 120.0, 24.0);
        list.rounded_rect(r, 4.0, [0.25, 0.30, 0.40, 1.0]);
        list.text(
            TextBlock::new("Hover me", r.x + 8.0, r.y + 5.0)
                .with_size(12.0)
                .with_color(200, 210, 230),
        );
        tooltip_rect = r;

        // Leave headroom below the last row for the tooltip popup.
        content_bottom = flow.bottom() + 70.0;
    }

    // Size the target to the laid-out content first, so the tooltip layer
    // knows the real screen height (it flips the popup up/left near the edges).
    let h = (content_bottom.ceil() as u32).max(64);

    // Floating dropdown list (Popup layer above the base content).
    {
        let popup = dropdowns.push_open_layer(&mut layers);
        dropdowns.draw_open_layer(&mut layers, popup, &theme, &InputState::default());
    }

    // Tooltip layer, hovering the reserved target.
    {
        let mut tooltip = TooltipLayer::new();
        tooltip.register(tooltip_rect, TooltipContent::text("This is a tooltip!"));
        let mut tip_input = InputState::default();
        tip_input.mouse_x = tooltip_rect.x + tooltip_rect.width / 2.0;
        tip_input.mouse_y = tooltip_rect.y + tooltip_rect.height / 2.0;
        tooltip.tick(999.0, &tip_input);
        tooltip.draw_into_layers(&mut layers, &tip_input, &theme, W as f32, h as f32);
    }

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gallery target"),
        size: wgpu::Extent3d {
            width: W,
            height: h,
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
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

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

    ui.render_layers(&device, &queue, &mut encoder, &view, (W, h), 1.0, &layers);

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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: h,
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
    let mut pixels = Vec::with_capacity(row_stride * h as usize);
    for row in 0..h as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }

    std::fs::create_dir_all("test_output").unwrap();
    let img = image::RgbaImage::from_raw(W, h, pixels).expect("image from raw");
    img.save("test_output/widget_gallery.png").expect("save png");
    eprintln!("wrote test_output/widget_gallery.png ({W}x{h})");

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
