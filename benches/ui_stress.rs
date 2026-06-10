//! Stress benchmarks for the UI render hot path.
//!
//! Answers "does the UI stay responsive when we draw 10k widgets a frame?" by
//! measuring the CPU cost of the per-frame pipeline at growing widget counts.
//! There is no culling in the library, so every widget is built and (for the
//! render benches) shaped/tessellated/encoded — i.e. these are worst-case,
//! everything-visible numbers.
//!
//! Run (needs a GPU adapter, like the `widget_gallery` test):
//! ```
//! DISPLAY=:0 cargo bench --bench ui_stress
//! ```
//!
//! What's measured is **CPU main-thread cost** — building the `DrawList`, then
//! `UiRenderer::render` (tessellation + MSDF text shaping + `queue.write_buffer`
//! + command encode + submit). GPU execution is async and not captured (that
//! would need timestamp queries); CPU stall is what makes a frame feel
//! unresponsive. Glyph MSDF generation is a one-time atlas cost absorbed by
//! Criterion's warmup, so steady-state samples reflect re-shaping + tessellation.
//!
//! Three groups:
//! - `drawlist_build` — CPU-only widget → `DrawList` (no GPU): geometry
//!   tessellation + `TextBlock`/string accumulation.
//! - `frame_render` — full `render()` of N chrome buttons (the real frame cost).
//! - `render_text_only` — N bare text blocks, identical vs unique labels, to
//!   attribute how much of `frame_render` is cosmic-text shaping.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Button, DrawList, FontSystemHandle, InputState, TextBlock, Theme, UiRenderer,
};

const W: u32 = 1920;
const H: u32 = 1080;

/// Counts for the GPU render benches.
const RENDER_COUNTS: &[usize] = &[100, 1_000, 10_000];
/// Counts for the CPU-only build bench (cheap enough to push higher).
const BUILD_COUNTS: &[usize] = &[100, 1_000, 10_000, 50_000];

/// Headless GPU + renderer, plus the shared font handle every `DrawList` must
/// use so it shapes against the same font DB the renderer does.
struct Harness {
    device: wgpu::Device,
    queue: wgpu::Queue,
    ui: UiRenderer,
    view: wgpu::TextureView,
    font_system: FontSystemHandle,
    // Keep the target alive for `view`.
    _target: wgpu::Texture,
}

impl Harness {
    fn new() -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("no GPU adapter available (run under DISPLAY=:0 with a GPU)");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("ui_stress device"),
                ..Default::default()
            },
            None,
        ))
        .expect("request device");

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let font_system = wgpu_gameui::shared_font_system();
        let ui = UiRenderer::new(&device, &queue, format, font_system.clone());

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui_stress target"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            device,
            queue,
            ui,
            view,
            font_system,
            _target: target,
        }
    }

    /// A fresh `DrawList` bound to the shared font system (avoids a per-call
    /// system-font-DB scan that `DrawList::new()` would trigger).
    fn draw_list(&self) -> DrawList {
        DrawList::with_font_system(self.font_system.clone())
    }

    /// Encode + submit one frame for `list`, draining finished GPU work without
    /// blocking on it (keeps the queue from backing up across Criterion iters).
    fn render_frame(&mut self, list: &DrawList) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        // Clear pass so the attachment is initialized each frame.
        {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.ui
            .render(&self.device, &self.queue, &mut encoder, &self.view, (W, H), list);
        self.queue.submit(Some(encoder.finish()));
        self.device.poll(wgpu::Maintain::Poll);
    }
}

/// Grid cell rect for widget `i`: a `cols`-wide grid of 64×28 cells. Widgets
/// past the target bounds are still fully built/shaped (no culling).
fn grid_rect(i: usize, cols: usize) -> Rect {
    let (cw, ch) = (64.0f32, 28.0f32);
    let col = (i % cols) as f32;
    let row = (i / cols) as f32;
    Rect::new(col * cw, row * ch, cw - 4.0, ch - 4.0)
}

fn cols_for(count: usize) -> usize {
    (count as f64).sqrt().ceil() as usize
}

/// Fill `list` with `count` chrome buttons in a grid.
fn build_buttons(list: &mut DrawList, count: usize, theme: &Theme, input: &InputState) {
    let cols = cols_for(count);
    for i in 0..count {
        Button::new("OK").draw(grid_rect(i, cols), list, theme, input);
    }
}

fn bench_drawlist_build(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let input = InputState::default();
    let mut list = harness.draw_list();

    let mut group = c.benchmark_group("drawlist_build");
    for &count in BUILD_COUNTS {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                build_buttons(&mut list, count, &theme, &input);
                std::hint::black_box(&list);
            });
        });
    }
    group.finish();
}

fn bench_frame_render(c: &mut Criterion) {
    let mut harness = Harness::new();
    let theme = Theme::default();
    let input = InputState::default();

    let mut group = c.benchmark_group("frame_render");
    // Render benches are heavier; trim sample size to keep wall time sane at 10k.
    group.sample_size(30);
    for &count in RENDER_COUNTS {
        // Build the DrawList once; the bench measures the render of it.
        let mut list = harness.draw_list();
        build_buttons(&mut list, count, &theme, &input);
        // Warm the atlas/buffers once before timing.
        harness.render_frame(&list);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, _| {
            b.iter(|| harness.render_frame(&list));
        });
    }
    group.finish();
}

fn bench_render_text_only(c: &mut Criterion) {
    let mut harness = Harness::new();

    let mut group = c.benchmark_group("render_text_only");
    group.sample_size(30);
    for &count in RENDER_COUNTS {
        for unique in [false, true] {
            let cols = cols_for(count);
            let mut list = harness.draw_list();
            for i in 0..count {
                let r = grid_rect(i, cols);
                let label = if unique {
                    format!("Btn {i}")
                } else {
                    "Btn".to_string()
                };
                list.text(TextBlock::new(&label, r.x, r.y).with_size(14.0));
            }
            harness.render_frame(&list);

            let kind = if unique { "unique" } else { "same" };
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(
                BenchmarkId::new(kind, count),
                &count,
                |b, _| b.iter(|| harness.render_frame(&list)),
            );
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_drawlist_build,
    bench_frame_render,
    bench_render_text_only
);
criterion_main!(benches);
