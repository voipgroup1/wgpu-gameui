//! Headless rendering — turn a [`DrawList`] into pixels without a window.
//!
//! Rendering a frame offscreen and writing it to a PNG is the only way to
//! actually *look* at a UI from a test, a CI job, or an agent that cannot open a
//! window. The recipe is short but has two steps that are easy to get wrong and
//! fail in confusing ways:
//!
//! 1. [`UiRenderer::render`] loads the existing attachment contents
//!    (`LoadOp::Load`) rather than clearing, so a target texture that is never
//!    cleared first renders over uninitialised memory.
//! 2. `copy_texture_to_buffer` requires each row to start on a 256-byte
//!    boundary, so the readback buffer is padded and the padding must be
//!    stripped or the image comes out sheared diagonally.
//!
//! [`capture_draw_list`] and [`capture_layers`] handle both. They need no async
//! runtime — `Device::poll(Maintain::Wait)` is synchronous — so they are always
//! available. Only [`HeadlessGpu`], which creates a device from nothing, needs a
//! blocking executor and lives behind the (default-on) `headless` feature.
//!
//! ```ignore
//! let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
//! let mut list = gpu.draw_list();
//! // ... draw into `list` ...
//! let pixels = gpu.capture(&list, (800, 600));
//! gpu.save_png("test_output/frame.png", &pixels, (800, 600)).unwrap();
//! ```

use std::path::Path;

use crate::layer::LayerStack;
use crate::render::UiRenderer;
use crate::widgets::DrawList;

/// Texture format used for offscreen capture. sRGB so colours match what the
/// swapchain shows on screen.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Round a row length up to wgpu's 256-byte `bytes_per_row` alignment.
fn padded_bytes_per_row(width: u32) -> u32 {
    (width * 4 + 255) & !255
}

/// Shared body of [`capture_draw_list`] / [`capture_layers`]. The caller declares
/// the frame boundary (`UiRenderer::begin_frame`) because the closure below holds
/// the unique borrow of `ui`.
fn capture_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: (u32, u32),
    clear: wgpu::Color,
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
) -> Vec<u8> {
    let (width, height) = size;
    assert!(width > 0 && height > 0, "capture size must be non-zero");

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gameui capture target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CAPTURE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = padded_bytes_per_row(width);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gameui capture readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gameui capture"),
    });

    // `UiRenderer::render` uses `LoadOp::Load`, so the target must be cleared by
    // a pass of our own first — without this the frame composites over
    // uninitialised memory.
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gameui capture clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
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

    draw(&mut encoder, &view);

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
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| {
        r.expect("failed to map capture readback buffer")
    });
    device.poll(wgpu::PollType::Poll);
    let data = slice.get_mapped_range().unwrap();

    // Strip the row padding; without this the image shears diagonally.
    let row_stride = (width * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * height as usize);
    for row in 0..height as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }
    pixels
}

/// Render `list` offscreen and read it back as tightly-packed RGBA8
/// (`width * 4` bytes per row, no padding).
///
/// `scale_factor` maps logical to physical pixels; pass `1.0` unless you are
/// reproducing a HiDPI window.
pub fn capture_draw_list(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    list: &DrawList,
    size: (u32, u32),
    scale_factor: f32,
    clear: wgpu::Color,
) -> Vec<u8> {
    ui.begin_frame();
    capture_with(device, queue, size, clear, |encoder, view| {
        ui.render(device, queue, encoder, view, size, scale_factor, list);
    })
}

/// [`capture_draw_list`] for a whole [`LayerStack`] — base list plus every
/// overlay layer, in z order.
pub fn capture_layers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    layers: &LayerStack,
    size: (u32, u32),
    scale_factor: f32,
    clear: wgpu::Color,
) -> Vec<u8> {
    ui.begin_frame();
    capture_with(device, queue, size, clear, |encoder, view| {
        ui.render_layers(device, queue, encoder, view, size, scale_factor, layers);
    })
}

/// Write tightly-packed RGBA8 pixels to `path` as a PNG, creating parent
/// directories as needed.
pub fn write_png(path: impl AsRef<Path>, pixels: &[u8], size: (u32, u32)) -> std::io::Result<()> {
    let (width, height) = size;
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        pixels.len(),
        expected,
        "expected {expected} bytes of RGBA for {width}x{height}, got {}",
        pixels.len()
    );

    let path = path.as_ref();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let img = image::RgbaImage::from_raw(width, height, pixels.to_vec())
        .expect("pixel buffer size already validated");
    img.save(path)
        .map_err(|e| std::io::Error::other(format!("failed to write {}: {e}", path.display())))
}

/// A self-contained offscreen GPU: adapter, device, queue, and a [`UiRenderer`],
/// created without a window.
///
/// This is the one-call path for a test or tool that just wants pixels. Behind
/// the default-on `headless` feature, which pulls in a blocking executor for
/// adapter/device creation — everything else in this module works without it.
#[cfg(feature = "headless")]
pub struct HeadlessGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    ui: UiRenderer,
    font_system: crate::text::FontSystemHandle,
}

#[cfg(feature = "headless")]
impl HeadlessGpu {
    /// Create an offscreen GPU context, or `None` when no adapter is available
    /// (headless CI without a software rasteriser, for instance).
    ///
    /// Returning `None` rather than panicking lets a test skip cleanly:
    /// `let Some(gpu) = HeadlessGpu::new() else { return };`
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("gameui headless device"),
                ..Default::default()
            },
            None,
        ))
        .ok()?;
        let font_system = crate::text::shared_font_system();
        let ui = UiRenderer::new(&device, &queue, CAPTURE_FORMAT, font_system.clone());
        Some(Self {
            device,
            queue,
            ui,
            font_system,
        })
    }

    /// The underlying device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The underlying queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The renderer, for loading sprites/images or registering nine-slices.
    pub fn renderer(&mut self) -> &mut UiRenderer {
        &mut self.ui
    }

    /// A fresh [`DrawList`] sharing this context's font system, so measured text
    /// matches what gets rendered.
    pub fn draw_list(&self) -> DrawList {
        DrawList::with_font_system(self.font_system.clone())
    }

    /// A fresh [`LayerStack`] sharing this context's font system.
    pub fn layer_stack(&self) -> LayerStack {
        LayerStack::with_font_system(self.font_system.clone())
    }

    /// Render `list` and read the pixels back, on a transparent background.
    pub fn capture(&mut self, list: &DrawList, size: (u32, u32)) -> Vec<u8> {
        self.capture_on(list, size, wgpu::Color::TRANSPARENT)
    }

    /// [`capture`](Self::capture) with an explicit clear colour.
    pub fn capture_on(&mut self, list: &DrawList, size: (u32, u32), clear: wgpu::Color) -> Vec<u8> {
        capture_draw_list(
            &self.device,
            &self.queue,
            &mut self.ui,
            list,
            size,
            1.0,
            clear,
        )
    }

    /// Render a whole [`LayerStack`] and read the pixels back.
    pub fn capture_layers(
        &mut self,
        layers: &LayerStack,
        size: (u32, u32),
        clear: wgpu::Color,
    ) -> Vec<u8> {
        capture_layers(
            &self.device,
            &self.queue,
            &mut self.ui,
            layers,
            size,
            1.0,
            clear,
        )
    }

    /// Render `list` straight to a PNG file.
    pub fn save_png(
        &mut self,
        path: impl AsRef<Path>,
        list: &DrawList,
        size: (u32, u32),
        clear: wgpu::Color,
    ) -> std::io::Result<()> {
        let pixels = self.capture_on(list, size, clear);
        write_png(path, &pixels, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_padding_rounds_up_to_256() {
        assert_eq!(
            padded_bytes_per_row(64),
            256,
            "64px = 256B, already aligned"
        );
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(512), 2048, "512px = 2048B, aligned");
        assert_eq!(padded_bytes_per_row(800), 3328, "800px = 3200B -> 3328B");
    }

    #[test]
    fn write_png_round_trips_pixels() {
        let dir = std::env::temp_dir().join("wgpu_gameui_capture_test");
        let path = dir.join("solid.png");
        let _ = std::fs::remove_file(&path);

        // 2x2 opaque red.
        let pixels: Vec<u8> = std::iter::repeat_n([255u8, 0, 0, 255], 4)
            .flatten()
            .collect();
        write_png(&path, &pixels, (2, 2)).expect("write png");

        let decoded = image::open(&path).expect("read back").to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.as_raw(), &pixels);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[should_panic(expected = "expected 16 bytes of RGBA")]
    fn write_png_rejects_a_mismatched_buffer() {
        let path = std::env::temp_dir().join("wgpu_gameui_capture_bad.png");
        write_png(path, &[0u8; 4], (2, 2)).ok();
    }
}
