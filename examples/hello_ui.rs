//! `hello_ui` — minimal end-to-end example for `UiRenderer`.
//!
//! Opens a window, builds a `DrawList` containing a panel (rounded rect),
//! a button-shaped quad, an icon (loaded from an in-memory checkerboard
//! sprite), a nine-slice frame, and some text. Renders all of it through
//! the single `UiRenderer::render` call.

use std::sync::Arc;

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, TextBlock, UiContext, UiRenderer};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
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

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
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

                // Build draw list.
                let mut list = DrawList::new();
                // Background nine-slice panel.
                list.nine_slice_id(
                    gpu.nine_slice_id,
                    40.0,
                    40.0,
                    480.0,
                    260.0,
                    [1.0, 1.0, 1.0, 1.0],
                );
                // Inner rounded fill.
                list.rounded_rect(
                    Rect::new(60.0, 60.0, 440.0, 220.0),
                    8.0,
                    [0.12, 0.14, 0.20, 0.95],
                );
                // Pretend button.
                list.rounded_rect(
                    Rect::new(80.0, 220.0, 120.0, 40.0),
                    6.0,
                    [0.30, 0.55, 0.85, 1.0],
                );
                // Icon (sprite).
                list.icon_sprite(
                    gpu.icon_sprite,
                    220.0,
                    220.0,
                    40.0,
                    40.0,
                    [1.0, 1.0, 1.0, 1.0],
                );

                // Text overlay.
                list.text(
                    TextBlock::new("hello_ui — wgpu-gameui", 80.0, 80.0)
                        .with_size(22.0)
                        .with_color(255, 255, 255),
                );
                list.text(
                    TextBlock::new("Renderer draws panel + quads + icon + text", 80.0, 120.0)
                        .with_size(14.0)
                        .with_color(200, 210, 230),
                );

                // ---------------------------------------------------------------
                // UiContext demo: a panel built with push/translate/color, plus a
                // rotated badge showing the rotation surface.
                // ---------------------------------------------------------------
                {
                    let mut ui = UiContext::new(&mut list);

                    // Translate into the bottom-right area, then nest a panel.
                    ui.push();
                    ui.translate(560.0, 60.0);
                    ui.color(0.20, 0.85, 0.55, 1.0);
                    ui.rounded_rect(200.0, 80.0, 8.0, [1.0, 1.0, 1.0, 1.0]);
                    // Centered label inside that panel.
                    ui.push();
                    ui.translate(100.0, 40.0);
                    ui.center();
                    // Reset tint to white for the label.
                    ui.color(1.0, 1.0, 1.0, 1.0);
                    ui.text(
                        TextBlock::new("UiContext", 0.0, 0.0)
                            .with_size(20.0)
                            .with_max_width(200.0)
                            .with_color(20, 30, 30),
                    );
                    ui.pop();
                    ui.pop();

                    // Color-filter sub-tree: a half-alpha overlay quad.
                    ui.push();
                    ui.translate(560.0, 160.0);
                    ui.color_filter(1.0, 1.0, 1.0, 0.5);
                    ui.quad(200.0, 30.0, [0.30, 0.55, 0.85, 1.0]);
                    ui.pop();

                    // Rotated badge — demonstrates rotation through the affine
                    // stack. Origin shifts to the badge centre, then we rotate.
                    ui.push();
                    ui.translate(660.0, 240.0);
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

                gpu.ui.render(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &view,
                    (gpu.config.width, gpu.config.height),
                    &list,
                );

                gpu.queue.submit(Some(encoder.finish()));
                frame.present();
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
    };
    event_loop.run_app(&mut app).expect("run app");
}
