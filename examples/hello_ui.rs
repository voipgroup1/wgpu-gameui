//! `hello_ui` — minimal end-to-end example for `UiRenderer`.
//!
//! Opens a window, builds a `DrawList` containing a panel (rounded rect),
//! a button-shaped quad, an icon (loaded from an in-memory checkerboard
//! sprite), a nine-slice frame, and some text. Renders all of it through
//! the single `UiRenderer::render` call.

use std::sync::Arc;

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    InputState, LayerStack, ScrollState, ScrollView, TextBlock, Theme, UiContext, UiRenderer,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

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

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    ui: UiRenderer,
    icon_sprite: u32,
    nine_slice_id: u32,
}

#[derive(Default)]
struct UiState {
    scroll: ScrollState,
    modal_open: bool,
    modal_button_was_clicked: bool,
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    input: InputState,
    theme: Theme,
    state: UiState,
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
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("request adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("hello_ui device"),
                ..Default::default()
            },
            None,
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
        };
        surface.configure(&device, &config);

        // Build UiRenderer with a fresh font system.
        let font_system = wgpu_gameui::shared_font_system();
        let mut ui = UiRenderer::new(&device, &queue, format, font_system);

        // Upload some sprites.
        let icon_pixels = checkerboard_pixels(
            CHECKER_SIZE,
            [220, 80, 80, 255],
            [40, 40, 40, 255],
        );
        let icon_sprite = ui.load_sprite_rgba8("icon", CHECKER_SIZE, CHECKER_SIZE, &icon_pixels);

        let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
        let frame_sprite = ui.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
        let nine_slice_id = ui.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);

        self.gpu = Some(Gpu {
            surface,
            device,
            queue,
            config,
            ui,
            icon_sprite,
            nine_slice_id,
        });

        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(window) = self.window.as_ref() else { return };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.ui.resize(&gpu.queue, gpu.config.width, gpu.config.height);
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_x = position.x as f32;
                self.input.mouse_y = position.y as f32;
                window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let pressed = state == ElementState::Pressed;
                    if pressed && !self.input.mouse_down {
                        self.input.mouse_clicked = true;
                    } else if !pressed && self.input.mouse_down {
                        self.input.mouse_released = true;
                    }
                    self.input.mouse_down = pressed;
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
            WindowEvent::RedrawRequested => {
                let frame = match gpu.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => {
                        gpu.surface.configure(&gpu.device, &gpu.config);
                        return;
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

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

                list.text(
                    TextBlock::new("hello_ui — wgpu-gameui", 80.0, 80.0)
                        .with_size(22.0)
                        .with_color(255, 255, 255),
                );
                list.text(
                    TextBlock::new("Click 'Open Modal' to demo the layer system", 80.0, 120.0)
                        .with_size(14.0)
                        .with_color(200, 210, 230),
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
                    &self.theme,
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
                                TextBlock::new(
                                    &format!("Row #{:02}", i),
                                    vp.x + 12.0,
                                    y + 4.0,
                                )
                                .with_size(14.0)
                                .with_color(200, 210, 230),
                            );
                        }
                    },
                );

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

                let mut encoder =
                    gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                }

                gpu.ui.render_layers(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &view,
                    (gpu.config.width, gpu.config.height),
                    &layers,
                );

                gpu.queue.submit(Some(encoder.finish()));
                frame.present();
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
    };
    event_loop.run_app(&mut app).expect("run app");
}
