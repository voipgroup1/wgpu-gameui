//! `hello_ui` — minimal end-to-end example for `UiRenderer`.
//!
//! Opens a window, builds a `DrawList` containing a panel (rounded rect),
//! a button-shaped quad, an icon (loaded from an in-memory checkerboard
//! sprite), a nine-slice frame, a decoded image (drawn full and cropped), a
//! line in a runtime-loaded custom font, center/right-aligned text, and some
//! text. Renders all of it through the single `UiRenderer::render` call.

use std::sync::Arc;
use std::time::Instant;
use wgpu::{CurrentSurfaceTexture, SurfaceColorSpace};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Button, ClickTracker, CursorIcon, CursorState, DragCapture, DragHandle, DragTracker,
    DrawContext, Dropdown, DropdownState, FocusState, FontHandle, InputState, LayerStack,
    ScrollState, ScrollView, StyleResolver, TextAlign, TextBlock, TextInput, Theme, UiContext,
    UiRenderer,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// Map the library's windowing-agnostic [`CursorIcon`] to winit's. This is the
/// one place the application bridges UI cursor requests to the windowing system.
fn to_winit_cursor(icon: CursorIcon) -> winit::window::CursorIcon {
    use winit::window::CursorIcon as W;
    match icon {
        CursorIcon::Default => W::Default,
        CursorIcon::Pointer => W::Pointer,
        CursorIcon::Text => W::Text,
        CursorIcon::Grab => W::Grab,
        CursorIcon::Grabbing => W::Grabbing,
        CursorIcon::ResizeHorizontal => W::EwResize,
        CursorIcon::ResizeVertical => W::NsResize,
        CursorIcon::NotAllowed => W::NotAllowed,
    }
}

const CHECKER_SIZE: u32 = 32;

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

/// Encode a small RGBA gradient as a PNG in memory, so the example can exercise
/// the runtime image-decode path (`UiRenderer::load_image_bytes`) the same way
/// Teardown's `UiImage(path)` loads an encoded file.
fn synth_png(w: u32, h: u32) -> Vec<u8> {
    use image::{ImageFormat, Rgba, RgbaImage};
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            img.put_pixel(x, y, Rgba([r, g, 160, 255]));
        }
    }
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("encode png");
    bytes
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

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    ui: UiRenderer,
    icon_sprite: u32,
    nine_slice_id: u32,
    image_sprite: u32,
    custom_font: FontHandle,
}

/// Focus ids for the two text inputs — any stable, unique `u64` works.
const TEXT_ID_A: u64 = 0;
const TEXT_ID_B: u64 = 1;
/// Focus-independent id for the dropdown.
const DROPDOWN_ID: u64 = 2;
/// Drag-capture id for the movable demo box.
const DRAG_BOX_ID: u64 = 3;
/// Focus ids for the two keyboard-operable demo buttons (Tab to reach them,
/// Space/Enter to activate).
const BTN_A_ID: u64 = 4;
const BTN_B_ID: u64 = 5;

const DROPDOWN_ITEMS: [&str; 5] = ["Fire", "Water", "Earth", "Air", "Aether"];

#[derive(Default)]
struct UiState {
    scroll: ScrollState,
    modal_open: bool,
    modal_button_was_clicked: bool,
    text_input: TextInput,
    text_input2: TextInput,
    /// Single keyboard-focus owner shared by both text inputs. Tab / Shift-Tab
    /// cycle between them; clicking elsewhere or pressing Esc blurs.
    focus: FocusState,
    /// Single open-dropdown owner. Click to open, click an option / outside /
    /// Esc to close.
    dropdowns: DropdownState,
    /// Currently selected dropdown option index.
    dropdown_sel: usize,
    /// Activation count for the two keyboard-operable demo buttons, so the live
    /// example shows Space/Enter activation working.
    button_clicks: u32,
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    input: InputState,
    theme: Theme,
    state: UiState,
    /// Cross-frame drag detection feeding `input.is_dragging`/`drag_delta`.
    drag: DragTracker,
    /// Top-left of the draggable demo box.
    drag_box: Rect,
    /// Drag-ownership arbiter for the movable box (so it can't fight other
    /// draggables for a single pointer gesture).
    drag_capture: DragCapture,
    /// Double-click and hold detection feeding `input.mouse_double_clicked`/`mouse_held`.
    clicks: ClickTracker,
    /// App start time, for wall-clock timestamps fed to `ClickTracker::update`.
    start: Instant,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("hello_ui — wgpu-gameui")
            .with_inner_size(winit::dpi::LogicalSize::new(800.0, 480.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.window = Some(window.clone());

        // Set up wgpu.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("request adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("hello_ui device"),
                ..Default::default()
            }
        ))
        .expect("request device");

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        // Build UiRenderer with a fresh font system. Keep a clone of the handle
        // so we can load a custom font into the same `FontSystem` the renderer
        // shapes against.
        let font_system = wgpu_gameui::shared_font_system();
        let font_for_loading = font_system.clone();
        let mut ui = UiRenderer::new(&device, &queue, format, font_system);

        // Upload some sprites.
        let icon_pixels = checkerboard_pixels(CHECKER_SIZE, [220, 80, 80, 255], [40, 40, 40, 255]);
        let icon_sprite = ui.load_sprite_rgba8("icon", CHECKER_SIZE, CHECKER_SIZE, &icon_pixels);

        let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
        let frame_sprite = ui.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
        let nine_slice_id = ui.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);

        // Decode an encoded (PNG) image at runtime, exactly like `UiImage(path)`.
        let png = synth_png(64, 64);
        let image_sprite = ui
            .load_image_bytes("demo_gradient", &png)
            .expect("decode demo image");

        // Load a font from bytes and select it per-`TextBlock`.
        let custom_font = wgpu_gameui::load_font_bytes(&font_for_loading, notosans::REGULAR_TTF)
            .expect("load custom font");

        self.gpu = Some(Gpu {
            surface,
            device,
            queue,
            config,
            ui,
            icon_sprite,
            nine_slice_id,
            image_sprite,
            custom_font,
        });

        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(window) = self.window.as_ref() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.ui
                    .resize(&gpu.queue, gpu.config.width, gpu.config.height);
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_x = position.x as f32;
                self.input.mouse_y = position.y as f32;
                window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        if pressed && !self.input.mouse_down {
                            self.input.mouse_clicked = true;
                        } else if !pressed && self.input.mouse_down {
                            self.input.mouse_released = true;
                        }
                        self.input.mouse_down = pressed;
                    }
                    MouseButton::Right => {
                        if pressed && !self.input.mouse_right_down {
                            self.input.mouse_right_clicked = true;
                        } else if !pressed && self.input.mouse_right_down {
                            self.input.mouse_right_released = true;
                        }
                        self.input.mouse_right_down = pressed;
                    }
                    MouseButton::Middle => {
                        if pressed && !self.input.mouse_middle_down {
                            self.input.mouse_middle_clicked = true;
                        } else if !pressed && self.input.mouse_middle_down {
                            self.input.mouse_middle_released = true;
                        }
                        self.input.mouse_middle_down = pressed;
                    }
                    _ => {}
                }
                window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 20.0) as f32,
                };
                self.input.scroll_delta = dy;
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event: ref ke, .. } => {
                use winit::keyboard::{KeyCode, PhysicalKey};
                let pressed = ke.state == ElementState::Pressed;
                // Map physical keys to our input fields.
                if let PhysicalKey::Code(code) = ke.physical_key {
                    match code {
                        KeyCode::ArrowLeft => self.input.key_left = pressed,
                        KeyCode::ArrowRight => self.input.key_right = pressed,
                        KeyCode::ArrowUp => self.input.key_up = pressed,
                        KeyCode::ArrowDown => self.input.key_down = pressed,
                        KeyCode::Space => {
                            if pressed {
                                self.input.key_space = true;
                            }
                        }
                        KeyCode::Home => self.input.key_home = pressed,
                        KeyCode::End => self.input.key_end = pressed,
                        KeyCode::Delete => self.input.key_delete = pressed,
                        KeyCode::Tab => {
                            if pressed {
                                self.input.key_tab = true;
                            }
                        }
                        KeyCode::Escape => {
                            if pressed {
                                self.input.key_escape = true;
                            }
                        }
                        KeyCode::Backspace => {
                            if pressed {
                                self.input.backspace_pressed = true;
                            }
                        }
                        KeyCode::ShiftLeft | KeyCode::ShiftRight => {
                            self.input.shift_pressed = pressed;
                        }
                        KeyCode::ControlLeft | KeyCode::ControlRight => {
                            self.input.ctrl_pressed = pressed;
                        }
                        KeyCode::Enter => {
                            if pressed {
                                self.input.enter_pressed = true;
                            }
                        }
                        _ => {}
                    }
                }
                // Capture text from the key event (winit 0.30 replaces `ReceivedCharacter` with
                // `KeyEvent::text`).
                if pressed {
                    if let Some(ref text) = ke.text {
                        // Filter out control characters (Enter, Tab, etc.) — they're
                        // handled via physical key mappings above.
                        for c in text.chars() {
                            if !c.is_control() {
                                self.input.text_input.push(c);
                            }
                        }
                    }
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = match gpu.surface.get_current_texture() {
                    CurrentSurfaceTexture::Success(f) => f,
                    _ => {
                        gpu.surface.configure(&gpu.device, &gpu.config);
                        return;
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                // Classify the pointer gesture for this frame before any layer
                // derives its own input view.
                let t = self.start.elapsed().as_secs_f64();
                self.drag.update(&mut self.input);
                self.clicks.update(&mut self.input, t);

                // Per-frame cursor accumulator: hovered widgets request an icon
                // through their DrawContext; we apply the winner to the window
                // after drawing. Fresh each frame.
                let mut frame_cursor = CursorState::new();

                // Build a LayerStack so we can demo modal layers.
                let mut layers = LayerStack::new();

                // Push the modal first (if open) so layer dispatch sees the
                // full z-order when computing `input_for_base`.
                let modal_rect = Rect::new(220.0, 100.0, 360.0, 200.0);
                let modal_idx = if self.state.modal_open {
                    Some(layers.push_modal(modal_rect))
                } else {
                    None
                };

                // Translate raw keyboard edges into device-agnostic navigation
                // intents (`self.input.nav`) before any focus/tree/dropdown
                // `begin_frame` reads them. `KeyboardNav` is the default binding;
                // to also drive the UI from a controller, fold a gamepad snapshot
                // in here too, e.g.:
                //
                //     let pad = /* fill GamepadNav from gilrs/SDL each frame */;
                //     wgpu_gameui::map_gamepad(&mut self.input, &pad);
                //
                // (map_keyboard / map_gamepad OR into `nav`, so order is moot.)
                wgpu_gameui::map_keyboard(&mut self.input);

                // Establish the open dropdown's popup layer at frame-top (from
                // last frame's geometry) so `input_for_base` blocks clicks to
                // widgets under the open list — same as the modal above.
                self.state.dropdowns.begin_frame(&self.input);
                let dropdown_popup = self.state.dropdowns.push_open_layer(&mut layers);

                // Resolve input for the base layer. When a modal is open this
                // sets `mouse_consumed = true` so base widgets can't fire.
                let mut base_input = layers.input_for_base(&self.input);

                let list = layers.base_mut();

                // Background nine-slice panel.
                list.nine_slice_id(
                    gpu.nine_slice_id,
                    40.0,
                    40.0,
                    480.0,
                    260.0,
                    [1.0, 1.0, 1.0, 1.0],
                );
                list.rounded_rect(
                    Rect::new(60.0, 60.0, 440.0, 220.0),
                    8.0,
                    [0.12, 0.14, 0.20, 0.95],
                );

                // "Open Modal" button rect.
                let open_btn = Rect::new(80.0, 220.0, 160.0, 40.0);
                let btn_hovered = !base_input.mouse_consumed
                    && open_btn.contains(base_input.mouse_x, base_input.mouse_y);
                let btn_color = if btn_hovered {
                    [0.40, 0.65, 0.95, 1.0]
                } else {
                    [0.30, 0.55, 0.85, 1.0]
                };
                list.rounded_rect(open_btn, 6.0, btn_color);
                if btn_hovered && base_input.mouse_clicked {
                    self.state.modal_open = true;
                    self.state.modal_button_was_clicked = true;
                }

                list.icon_sprite(
                    gpu.icon_sprite,
                    260.0,
                    220.0,
                    40.0,
                    40.0,
                    [1.0, 1.0, 1.0, 1.0],
                );

                // Title: MSDF outline demo (white fill, dark 2px outline).
                list.text(
                    TextBlock::new("hello_ui — wgpu-gameui", 80.0, 80.0)
                        .with_size(22.0)
                        .with_color(255, 255, 255)
                        .with_outline(10, 12, 18, 255, 2.0),
                );
                // Subtitle: MSDF drop-shadow demo.
                list.text(
                    TextBlock::new("Click 'Open Modal' to demo the layer system", 80.0, 120.0)
                        .with_size(14.0)
                        .with_color(200, 210, 230)
                        .with_shadow(0, 0, 0, 180, 1.0, 1.0, 1.0),
                );
                list.text(
                    TextBlock::new("Open Modal", 110.0, 232.0)
                        .with_size(16.0)
                        .with_color(255, 255, 255),
                );

                // ---------- ScrollView demo ----------
                // 30 rows of overflowing content in a 200x240 viewport.
                let scroll_viewport = Rect::new(560.0, 60.0, 200.0, 240.0);
                list.rounded_rect(scroll_viewport, 6.0, [0.06, 0.07, 0.10, 1.0]);
                self.state.scroll.content_size = [180.0, 30.0 * 24.0];

                ScrollView::new(scroll_viewport).vertical_only().draw(
                    &mut self.state.scroll,
                    list,
                    &StyleResolver::new(&self.theme),
                    &mut base_input,
                    |list, vp| {
                        for i in 0..30usize {
                            let y = vp.y + i as f32 * 24.0;
                            let bg = if i % 2 == 0 {
                                [0.16, 0.18, 0.24, 1.0]
                            } else {
                                [0.10, 0.12, 0.18, 1.0]
                            };
                            list.quad(vp.x + 4.0, y + 2.0, vp.width - 12.0, 20.0, bg);
                            list.text(
                                TextBlock::new(&format!("Row #{:02}", i), vp.x + 12.0, y + 4.0)
                                    .with_size(14.0)
                                    .with_color(200, 210, 230),
                            );
                        }
                    },
                );

                // ---------- Text input demo ----------
                // Two inputs sharing one FocusState: click to focus, Tab /
                // Shift-Tab to cycle, click empty space or press Esc to blur.
                // Focus arbitration is resolved in `focus.end_frame()` below.
                {
                    self.state.focus.begin_frame(&base_input);

                    // Position the two text inputs.
                    self.state.text_input.x = 80.0;
                    self.state.text_input.y = 300.0;
                    self.state.text_input.width = 240.0;
                    self.state.text_input.height = 28.0;
                    self.state.text_input2.x = 80.0;
                    self.state.text_input2.y = 336.0;
                    self.state.text_input2.width = 240.0;
                    self.state.text_input2.height = 28.0;

                    // Background label for the inputs.
                    list.text(
                        TextBlock::new("Text Input (Tab to cycle):", 80.0, 280.0)
                            .with_size(12.0)
                            .with_color(180, 190, 210),
                    );
                    {
                        let mut ctx = DrawContext::new(
                            list,
                            &mut self.state.focus,
                            &self.theme,
                            &base_input,
                            gpu.config.width as f32,
                            gpu.config.height as f32,
                        )
                        .with_cursor(&mut frame_cursor);
                        self.state.text_input.draw(TEXT_ID_A, &mut ctx);
                        self.state.text_input2.draw(TEXT_ID_B, &mut ctx);
                    }

                    // Two keyboard-operable buttons sharing the same focus ring
                    // as the text inputs: Tab/Shift-Tab cycle through inputs and
                    // buttons; Space/Enter activates a focused button.
                    {
                        list.text(
                            TextBlock::new(
                                &format!(
                                    "Buttons (Tab to focus, Space/Enter to click): {} clicks",
                                    self.state.button_clicks
                                ),
                                80.0,
                                372.0,
                            )
                            .with_size(12.0)
                            .with_color(180, 190, 210),
                        );
                        let mut ctx = DrawContext::new(
                            list,
                            &mut self.state.focus,
                            &self.theme,
                            &base_input,
                            gpu.config.width as f32,
                            gpu.config.height as f32,
                        )
                        .with_cursor(&mut frame_cursor);
                        if Button::new("Click A")
                            .focusable(BTN_A_ID)
                            .draw(Rect::new(80.0, 390.0, 100.0, 28.0), &mut ctx)
                        {
                            self.state.button_clicks += 1;
                        }
                        if Button::new("Click B")
                            .focusable(BTN_B_ID)
                            .draw(Rect::new(190.0, 390.0, 100.0, 28.0), &mut ctx)
                        {
                            self.state.button_clicks += 1;
                        }
                    }

                    self.state.focus.end_frame(None);
                }

                // ---------- Dropdown demo ----------
                // The button draws inline here; the open option list is drawn
                // after the base scope (into the popup pushed at frame-top).
                {
                    list.text(
                        TextBlock::new("Dropdown:", 340.0, 280.0)
                            .with_size(12.0)
                            .with_color(180, 190, 210),
                    );
                    let dd_rect = Rect::new(340.0, 300.0, 150.0, 28.0);
                    let mut ctx = DrawContext::new(
                        list,
                        &mut self.state.focus,
                        &self.theme,
                        &base_input,
                        gpu.config.width as f32,
                        gpu.config.height as f32,
                    )
                    .with_cursor(&mut frame_cursor);
                    Dropdown::new(&DROPDOWN_ITEMS, self.state.dropdown_sel).draw(
                        DROPDOWN_ID,
                        dd_rect,
                        &mut self.state.dropdowns,
                        &mut ctx,
                    );
                }

                // ---------- Image + custom font + alignment demo ----------
                // Decoded image drawn at full size, then the same image cropped
                // to its top-left quarter (UV [0,0,0.5,0.5]) stretched to match.
                list.image(
                    gpu.image_sprite,
                    Rect::new(80.0, 320.0, 64.0, 64.0),
                    [1.0, 1.0, 1.0, 1.0],
                );
                list.image_cropped(
                    gpu.image_sprite,
                    Rect::new(152.0, 320.0, 64.0, 64.0),
                    [0.0, 0.0, 0.5, 0.5],
                    [1.0, 1.0, 1.0, 1.0],
                );
                // A line shaped in the runtime-loaded custom font.
                list.text(
                    TextBlock::new("Custom font: Noto Sans", 232.0, 322.0)
                        .with_size(16.0)
                        .with_color(255, 228, 160)
                        .with_font(gpu.custom_font.clone()),
                );
                // Center- and right-aligned lines within a 300px-wide box.
                list.text(
                    TextBlock::new("centered in 300px", 232.0, 348.0)
                        .with_size(14.0)
                        .with_color(200, 210, 230)
                        .with_max_width(300.0)
                        .with_align(TextAlign::Center),
                );
                list.text(
                    TextBlock::new("right-aligned in 300px", 232.0, 366.0)
                        .with_size(14.0)
                        .with_color(200, 210, 230)
                        .with_max_width(300.0)
                        .with_align(TextAlign::Right),
                );
                // Bold + italic, using the bundled default sans-serif (real faces).
                list.text(
                    TextBlock::new("Bold", 232.0, 388.0)
                        .with_size(16.0)
                        .with_color(255, 255, 255)
                        .bold(),
                );
                list.text(
                    TextBlock::new("Italic", 290.0, 388.0)
                        .with_size(16.0)
                        .with_color(255, 255, 255)
                        .italic(),
                );

                // A small UiContext font-stack snippet: push a bold 18px font,
                // draw a line via `text_line`, then pop back to the default.
                let stack_font = gpu.custom_font.clone();
                {
                    let mut ui = UiContext::new(list);
                    ui.push();
                    ui.translate(232.0, 410.0);

                    ui.font(stack_font, 18.0);
                    ui.bold(true);
                    ui.text_line("font-stack: pushed bold", [0.7, 0.9, 1.0, 1.0]);
                    ui.pop();
                }

                // ---------- Modal demo ----------
                if let Some(idx) = modal_idx {
                    // Resolve input for THIS layer.
                    let modal_input = layers.input_for_layer(idx, &self.input);

                    let m = &mut layers.layers_mut()[idx].list;
                    // Dim the background with a full-screen scrim.
                    m.quad(
                        0.0,
                        0.0,
                        gpu.config.width as f32,
                        gpu.config.height as f32,
                        [0.0, 0.0, 0.0, 0.55],
                    );
                    m.rounded_rect(modal_rect, 8.0, [0.18, 0.20, 0.26, 1.0]);
                    m.text(
                        TextBlock::new("Modal Dialog", modal_rect.x + 16.0, modal_rect.y + 16.0)
                            .with_size(20.0)
                            .with_color(255, 255, 255),
                    );
                    m.text(
                        TextBlock::new(
                            "Lower layers can't be hovered while this is open.",
                            modal_rect.x + 16.0,
                            modal_rect.y + 56.0,
                        )
                        .with_size(14.0)
                        .with_color(200, 210, 230)
                        .with_max_width(modal_rect.width - 32.0),
                    );
                    let close_btn = Rect::new(
                        modal_rect.x + modal_rect.width - 110.0,
                        modal_rect.y + modal_rect.height - 50.0,
                        90.0,
                        34.0,
                    );
                    let close_hover = close_btn.contains(modal_input.mouse_x, modal_input.mouse_y);
                    m.rounded_rect(
                        close_btn,
                        6.0,
                        if close_hover {
                            [0.40, 0.65, 0.95, 1.0]
                        } else {
                            [0.30, 0.55, 0.85, 1.0]
                        },
                    );
                    m.text(
                        TextBlock::new("Close", close_btn.x + 22.0, close_btn.y + 8.0)
                            .with_size(16.0)
                            .with_color(255, 255, 255),
                    );
                    if close_hover && modal_input.mouse_clicked {
                        self.state.modal_open = false;
                    }
                    layers.pop_layer();
                }

                // Deferred dropdown list (Popup layer above the base content).
                // Picking an option updates the selection and closes the menu.
                if let Some((id, idx)) = self.state.dropdowns.draw_open_layer(
                    &mut layers,
                    dropdown_popup,
                    &StyleResolver::new(&self.theme),
                    &self.input,
                ) {
                    if id == DROPDOWN_ID {
                        self.state.dropdown_sel = idx;
                    }
                }
                self.state.dropdowns.end_frame();

                // Bonus: rotated badge from the original demo, on the base layer
                // (built via UiContext::with_layers so it stays clipped to the
                // base list).
                {
                    let mut ui = UiContext::with_layers(&mut layers);
                    ui.push();
                    ui.translate(660.0, 320.0);
                    ui.rotate(15.0_f32.to_radians());
                    ui.center();
                    ui.rounded_rect(160.0, 40.0, 6.0, [0.95, 0.45, 0.30, 1.0]);
                    ui.pop();
                }

                // ---------- Draggable box demo (DragHandle + DragTracker) ----------
                // `DragHandle::bare()` arbitrates the grab through a shared
                // `DragCapture` (so it can't fight other draggables) and returns
                // the per-frame delta sourced from the `DragTracker`. We use the
                // bare handle as a pure hit-zone and keep the box's own visuals.
                {
                    let handle_rect = self.drag_box;
                    let out = {
                        let mut ctx = DrawContext::new(
                            layers.base_mut(),
                            &mut self.state.focus,
                            &self.theme,
                            &base_input,
                            gpu.config.width as f32,
                            gpu.config.height as f32,
                        )
                        .with_cursor(&mut frame_cursor);
                        DragHandle::bare().draw(
                            DRAG_BOX_ID,
                            &mut self.drag_capture,
                            handle_rect,
                            &mut ctx,
                        )
                    };
                    self.drag_box.x += out.delta[0];
                    self.drag_box.y += out.delta[1];

                    let color = if out.dragging {
                        [0.95, 0.75, 0.30, 1.0]
                    } else {
                        [0.30, 0.70, 0.55, 1.0]
                    };
                    let list = layers.base_mut();
                    list.rounded_rect(self.drag_box, 8.0, color);
                    list.text(
                        TextBlock::new("drag me", self.drag_box.x + 18.0, self.drag_box.y + 18.0)
                            .with_size(16.0)
                            .with_color(20, 24, 28),
                    );
                }

                // ---------- Double-click / hold demo (ClickTracker) ----------
                {
                    let demo_rect = Rect::new(340.0, 430.0, 150.0, 56.0);
                    let on_btn = !base_input.mouse_consumed
                        && demo_rect.contains(base_input.mouse_x, base_input.mouse_y);
                    let color = if on_btn && base_input.mouse_held {
                        [0.85, 0.30, 0.30, 1.0] // red = held
                    } else if on_btn && base_input.mouse_double_clicked {
                        [0.40, 0.80, 0.40, 1.0] // green = double-click
                    } else if on_btn {
                        [0.40, 0.55, 0.80, 1.0] // hover
                    } else {
                        [0.26, 0.38, 0.60, 1.0] // idle
                    };
                    let label = if on_btn && base_input.mouse_held {
                        "HELD"
                    } else if on_btn && base_input.mouse_double_clicked {
                        "DOUBLE!"
                    } else {
                        "dbl/hold"
                    };
                    let list = layers.base_mut();
                    list.rounded_rect(demo_rect, 8.0, color);
                    list.text(
                        TextBlock::new(label, demo_rect.x + 18.0, demo_rect.y + 18.0)
                            .with_size(16.0)
                            .with_color(230, 235, 245),
                    );
                }

                // Apply the cursor requested by whichever widget the pointer is
                // over this frame (Default if none asked).
                window.set_cursor(to_winit_cursor(frame_cursor.resolve()));

                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("hello_ui encoder"),
                        });

                // Clear the frame manually (UiRenderer always loads).
                {
                    let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                }

                gpu.ui.render_layers(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &view,
                    (gpu.config.width, gpu.config.height),
                    window.scale_factor() as f32,
                    &layers,
                );

                gpu.queue.submit(Some(encoder.finish()));
                //frame.present();
                gpu.queue.present(frame);
                self.input.end_frame();
                window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let _ = env_logger::try_init();
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App {
        window: None,
        gpu: None,
        input: InputState::default(),
        theme: Theme::default(),
        state: UiState::default(),
        drag: DragTracker::new(),
        drag_box: Rect::new(340.0, 360.0, 150.0, 56.0),
        drag_capture: DragCapture::new(),
        clicks: ClickTracker::new(),
        start: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run app");
}
