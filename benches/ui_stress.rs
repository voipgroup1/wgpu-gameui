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

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use wgpu_gameui::layout::{Anchor, LayoutResult, Positioned, Rect, Size, VStack};
use wgpu_gameui::{
    AnimSlot, AnimationState, Button, Checkbox, ColumnWidth, DragCapture, DrawContext, DrawList,
    Easing, FocusState, FontSystemHandle, Frame, InputState, KeyboardNav, List, ListItem,
    ListState, NumberInput, ScrollState, ScrollView, Slider, StyleResolver, Table, TableCell,
    TableColumn, TextBlock, TextInput, TextMeasurer, Theme, UiRenderer, UiState,
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
        let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

        // A registered nine-slice for the nine-slice render bench.
        let frame = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
        let frame_sprite = ui.load_sprite_rgba8(NINE_SLICE_KEY, 32, 32, &frame);
        ui.register_nine_slice(NINE_SLICE_KEY, frame_sprite, [4, 4, 4, 4]);

        // A registered sprite (resolved by name at draw time) for the icon bench.
        let icon = solid_with_border(32, [120, 200, 240, 255], [30, 60, 90, 255], 3);
        ui.load_sprite_rgba8(ICON_KEY, 32, 32, &icon);

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
        self.ui.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.view,
            (W, H),
            1.0,
            list,
        );
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
    let mut focus = FocusState::new();
    let mut ctx = DrawContext::new(list, &mut focus, theme, input, W as f32, H as f32);
    for i in 0..count {
        Button::new("OK").draw(grid_rect(i, cols), &mut ctx);
    }
}

/// Nine-slice resource key registered in the harness.
const NINE_SLICE_KEY: &str = "bench_frame";
/// Icon sprite key registered in the harness.
const ICON_KEY: &str = "bench_icon";

/// An `n×n`-pixel sprite: a `thickness`-px border of `border` around `fill`.
/// Matches the gallery's helper so the bench nine-slice has real border texels.
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

/// Fill `list` with `count` nine-slice panels in a grid.
fn build_nine_slices(list: &mut DrawList, count: usize) {
    let cols = cols_for(count);
    for i in 0..count {
        let r = grid_rect(i, cols);
        list.nine_slice(r.x, r.y, r.width, r.height, NINE_SLICE_KEY);
    }
}

/// Fill `list` with `count` icons (resolved by name) in a grid.
fn build_icons(list: &mut DrawList, count: usize) {
    let cols = cols_for(count);
    for i in 0..count {
        let r = grid_rect(i, cols);
        list.icon(ICON_KEY, r.x, r.y, r.width, r.height);
    }
}

/// The colored-soup primitives exercised by the primitive benches. Each is a
/// distinct tessellation shape (vertex/index count and CPU cost differ): a flat
/// quad (4 verts), a rounded rect (5 strip quads + 4×8 corner tris), a 4-quad
/// outline, a single line quad, and a 16–64-segment circle fan. These are the
/// last "tessellate N shapes into a soup + re-upload the whole soup each frame"
/// path in the renderer, so we measure them to know the cost before deciding
/// whether the instancing approach (chrome/nine-slice/icons) should carry over.
const PRIMITIVE_KINDS: &[&str] = &["rect", "rounded_rect", "rect_outline", "line", "circle"];

/// Fill `list` with `count` instances of one primitive `kind` in a grid.
fn build_primitive(list: &mut DrawList, kind: &str, count: usize) {
    let cols = cols_for(count);
    let color = [0.3, 0.55, 0.85, 1.0];
    for i in 0..count {
        let r = grid_rect(i, cols);
        match kind {
            "rect" => list.quad(r.x, r.y, r.width, r.height, color),
            "rounded_rect" => list.rounded_rect(r, 6.0, color),
            "rect_outline" => list.rect_outline(r, 2.0, color),
            "line" => list.line([r.x, r.y], [r.x + r.width, r.y + r.height], 2.0, color),
            "circle" => list.circle(
                (r.x + r.width * 0.5, r.y + r.height * 0.5),
                r.height * 0.5,
                color,
            ),
            other => unreachable!("unknown primitive kind {other}"),
        }
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
            group.bench_with_input(BenchmarkId::new(kind, count), &count, |b, _| {
                b.iter(|| harness.render_frame(&list))
            });
        }
    }
    group.finish();
}

fn bench_nine_slice(c: &mut Criterion) {
    let mut harness = Harness::new();

    let mut group = c.benchmark_group("nine_slice");
    group.sample_size(30);
    for &count in RENDER_COUNTS {
        let mut list = harness.draw_list();
        build_nine_slices(&mut list, count);
        // Warm the atlas/buffers once before timing.
        harness.render_frame(&list);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, _| {
            b.iter(|| harness.render_frame(&list));
        });
    }
    group.finish();
}

fn bench_icons(c: &mut Criterion) {
    let mut harness = Harness::new();

    let mut group = c.benchmark_group("icons");
    group.sample_size(30);
    for &count in RENDER_COUNTS {
        let mut list = harness.draw_list();
        build_icons(&mut list, count);
        // Warm the atlas/buffers once before timing.
        harness.render_frame(&list);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, _| {
            b.iter(|| harness.render_frame(&list));
        });
    }
    group.finish();
}

/// CPU-only tessellation cost: build `count` of each primitive into the soup
/// (no GPU). This is the work the soup pays every frame that instancing would
/// eliminate, isolated from upload/encode.
fn bench_primitives_build(c: &mut Criterion) {
    let harness = Harness::new();
    let mut list = harness.draw_list();

    let mut group = c.benchmark_group("primitives_build");
    for &kind in PRIMITIVE_KINDS {
        for &count in BUILD_COUNTS {
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(BenchmarkId::new(kind, count), &count, |b, &count| {
                b.iter(|| {
                    list.clear();
                    build_primitive(&mut list, kind, count);
                    std::hint::black_box(&list);
                });
            });
        }
    }
    group.finish();
}

/// Full per-frame render cost of `count` of each primitive: tessellation +
/// whole-soup `queue.write_buffer` + encode + submit (the real frame cost of
/// the colored-soup path, with no chrome interleaving).
fn bench_primitives_render(c: &mut Criterion) {
    let mut harness = Harness::new();

    let mut group = c.benchmark_group("primitives_render");
    group.sample_size(30);
    for &kind in PRIMITIVE_KINDS {
        for &count in RENDER_COUNTS {
            let mut list = harness.draw_list();
            build_primitive(&mut list, kind, count);
            // Warm the buffers once before timing.
            harness.render_frame(&list);

            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(BenchmarkId::new(kind, count), &count, |b, _| {
                b.iter(|| harness.render_frame(&list));
            });
        }
    }
    group.finish();
}

/// Benchmark for the layout system: building and laying out a 10k-child stack.
///
/// CPU-only — no GPU needed. Builds a VStack tree once, then measures the
/// `layout()` call at growing sizes.
fn bench_layout(c: &mut Criterion) {
    let counts: &[usize] = &[100, 1_000, 10_000, 50_000];

    let mut group = c.benchmark_group("layout_resolve");
    for &count in counts {
        // Build the tree once: a single VStack with N children.
        let mut stack = VStack::new(2.0).with_padding(4.0);
        for i in 0..count {
            // Mix: mostly Fixed, every 10th is Fill, every 23rd is Fit-ish.
            if i % 10 == 0 {
                stack = stack.child_fill(60.0);
            } else if i % 23 == 0 {
                stack = stack.child_percent(0.05, 60.0);
            } else {
                stack = stack.child(20.0, 60.0);
            }
        }
        // Wrap in a Positioned so it's a full LayoutNode tree.
        let tree = Positioned::new(
            Anchor::TopLeft { offset: (0.0, 0.0) },
            Size::fixed(600.0, (count as f32 * 22.0).max(200.0)),
            stack,
        );

        group.throughput(Throughput::Elements(count as u64));
        // Fresh allocation per call (the `layout()` convenience path).
        group.bench_with_input(BenchmarkId::new("alloc", count), &count, |b, _| {
            b.iter(|| {
                let result = tree.layout_screen(1920.0, 1080.0);
                std::hint::black_box(&result);
            });
        });
        // Reused caller-owned buffer (the `layout_into` hot-loop path): the
        // per-frame Vec allocation is paid once, then amortized away.
        group.bench_with_input(BenchmarkId::new("reuse", count), &count, |b, _| {
            let mut buf = LayoutResult::default();
            b.iter(|| {
                tree.layout_screen_into(1920.0, 1080.0, &mut buf);
                std::hint::black_box(&buf);
            });
        });
    }
    group.finish();
}

/// CPU-only text measurement: cache-hit vs cache-miss at growing string counts.
///
/// Both paths use identical short strings so the comparison is apples-to-apples.
/// Cache-hit re-measures the same strings every iteration (hash lookup, no
/// shaping). Cache-miss measures a fresh unique string each time (FontSystem lock
/// + cosmic-text shape + insert), with the TextMeasurer cache cleared between
/// iterations so every call is a genuine miss.
///
/// Note: cosmic-text has its own internal glyph cache that survives
/// `TextMeasurer::clear_cache()`, so "miss" times represent a warm-glyph-cache
/// miss (the glyphs are already rasterized), not a cold start.
fn bench_text_shape(c: &mut Criterion) {
    let mut measurer = TextMeasurer::new();
    let counts: &[usize] = &[100, 1_000, 10_000];
    const FONT_SIZE: f32 = 14.0;

    let mut group = c.benchmark_group("text_shape");
    for &count in counts {
        // Use identical string patterns for both paths.
        let hit_strings: Vec<String> =
            (0..count).map(|i| format!("item{i:05}")).collect();

        // Pre-populate the cache so the hit path hits.
        for s in &hit_strings {
            measurer.measure(s, FONT_SIZE, None);
        }

        // --- cache hit: all strings already in cache --------------------
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("cache_hit", count), &count, |b, _| {
            b.iter(|| {
                for s in &hit_strings {
                    std::hint::black_box(measurer.measure(s, FONT_SIZE, None));
                }
            });
        });

        // --- cache miss: cache cleared, entirely disjoint string set -----
        measurer.clear_cache();
        group.bench_with_input(BenchmarkId::new("cache_miss", count), &count, |b, &count| {
            b.iter(|| {
                measurer.clear_cache();
                for i in 0..count {
                    // Offset by count so these never collide with hit_strings.
                    std::hint::black_box(measurer.measure(
                        &format!("miss{i:05}x{count:05}"),
                        FONT_SIZE,
                        None,
                    ));
                }
            });
        });
    }
    group.finish();
}

/// CPU-only build of the four main interactive widget types through `DrawContext`.
///
/// Measures the per-widget cost of the `draw()` call — style lookups, geometry
/// tessellation, focus registration, and interaction edge detection — in
/// isolation (no GPU render, no UI context overhead).
fn bench_interactive_widgets(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let input = InputState::default();
    let mut list = harness.draw_list();
    let counts: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("interactive_widgets");

    // --- Slider ----------------------------------------------------------
    for &count in counts {
        let slider = Slider::new(0.0, 1.0).with_step(0.01);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("slider", count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut focus = FocusState::new();
                let mut capture = DragCapture::new();
                let mut ctx = DrawContext::new(
                    &mut list, &mut focus, &theme, &input, W as f32, H as f32,
                );
                for i in 0..count {
                    let r = grid_rect(i, cols_for(count));
                    std::hint::black_box(slider.draw(
                        0.5,
                        i as u64, // DragId
                        &mut capture,
                        r,
                        &mut ctx,
                    ));
                }
            });
        });
    }

    // --- Checkbox --------------------------------------------------------
    for &count in counts {
        let cb = Checkbox::new();
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("checkbox", count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut focus = FocusState::new();
                let mut ctx = DrawContext::new(
                    &mut list, &mut focus, &theme, &input, W as f32, H as f32,
                );
                for i in 0..count {
                    let r = grid_rect(i, cols_for(count));
                    std::hint::black_box(cb.draw(i % 2 == 0, "X", r, &mut ctx));
                }
            });
        });
    }

    // --- TextInput -------------------------------------------------------
    for &count in counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("text_input", count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut focus = FocusState::new();
                let mut ctx = DrawContext::new(
                    &mut list, &mut focus, &theme, &input, W as f32, H as f32,
                );
                // Fresh TextInputs each iter — they carry internal cursor state
                // but we want the pure draw cost, not state-accumulation effects.
                let mut tis: Vec<TextInput> = (0..count)
                    .map(|i| {
                        let r = grid_rect(i, cols_for(count));
                        TextInput::new(r.x, r.y, r.width, r.height)
                    })
                    .collect();
                for (i, ti) in tis.iter_mut().enumerate() {
                    std::hint::black_box(ti.draw(i as u64, &mut ctx));
                }
            });
        });
    }

    // --- NumberInput -----------------------------------------------------
    for &count in counts {
        let ni = NumberInput::new().with_range(0.0, 100.0).with_step(1.0);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("number_input", count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut focus = FocusState::new();
                let mut ctx = DrawContext::new(
                    &mut list, &mut focus, &theme, &input, W as f32, H as f32,
                );
                let mut tis: Vec<TextInput> = (0..count)
                    .map(|i| {
                        let r = grid_rect(i, cols_for(count));
                        TextInput::new(r.x, r.y, r.width, r.height)
                    })
                    .collect();
                for (i, ti) in tis.iter_mut().enumerate() {
                    let r = grid_rect(i, cols_for(count));
                    std::hint::black_box(ni.draw(50.0, i as u64, ti, r, &mut ctx));
                }
            });
        });
    }

    group.finish();
}

/// CPU cost of processing key events through `TextInput` — the hot path of text
/// editing (typing, backspace, delete, arrow keys, Ctrl+A).
///
/// Each iteration creates N fresh `TextInput`s, feeds each one simulated
/// key-downs, then draws them. This isolates the edit-path cost separate from
/// pure draw.
fn bench_text_input_edit(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let mut list = harness.draw_list();
    let counts: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("text_input_edit");
    for &count in counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut focus = FocusState::new();
                // One key-down edge per field: Backspace (trims last char).
                let mut input = InputState::default();
                input.backspace_pressed = true;
                let mut ctx = DrawContext::new(
                    &mut list, &mut focus, &theme, &input, W as f32, H as f32,
                );
                // Fresh TextInputs each iteration with a starting value so
                // backspace hit is consistent.
                let mut tis: Vec<TextInput> = (0..count)
                    .map(|i| {
                        let r = grid_rect(i, cols_for(count));
                        TextInput::new(r.x, r.y, r.width, r.height)
                            .with_value(format!("item {i}"))
                    })
                    .collect();
                for (i, ti) in tis.iter_mut().enumerate() {
                    std::hint::black_box(ti.draw(i as u64, &mut ctx));
                }
            });
        });
    }
    group.finish();
}

/// CPU-only cost of drawing N `ScrollView` regions, each wrapping a simple
/// content closure (a handful of quads). Measures the scroll-clip + transform +
/// scrollbar overhead independent of the content.
fn bench_scroll_view(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let mut list = harness.draw_list();
    let counts: &[usize] = &[100, 1_000, 10_000];
    let style = StyleResolver::new(&theme);

    let mut group = c.benchmark_group("scroll_view");
    for &count in counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut input = InputState::default();
                for i in 0..count {
                    let mut state = ScrollState::new();
                    state.content_size = [200.0, 500.0]; // scrollable content
                    let r = Rect::new(
                        (i * 64) as f32 % (W as f32 - 200.0),
                        (i / 10) as f32 * 160.0,
                        200.0,
                        150.0,
                    );
                    let sv = ScrollView::new(r);
                    sv.draw(&mut state, &mut list, &style, &mut input, |list, inner| {
                        // Simulate a small content payload: 5 colored quads.
                        list.quad(inner.x, inner.y, inner.width, 30.0, [0.2, 0.3, 0.8, 1.0]);
                        list.quad(inner.x, inner.y + 60.0, inner.width, 30.0, [0.3, 0.6, 0.3, 1.0]);
                        list.quad(inner.x, inner.y + 120.0, inner.width, 30.0, [0.8, 0.3, 0.2, 1.0]);
                        list.quad(inner.x, inner.y + 250.0, inner.width, 30.0, [0.5, 0.3, 0.7, 1.0]);
                        list.quad(inner.x, inner.y + 400.0, inner.width, 30.0, [0.2, 0.7, 0.7, 1.0]);
                    });
                }
                std::hint::black_box(&list);
            });
        });
    }
    group.finish();
}

/// CPU-only cost of drawing one virtualized `List` with `count` logical items
/// (the widget draws only the visible subset). Measures the culling + selection +
/// scroll interaction cost, not the item closure (which is a no-op label).
fn bench_list_virtual(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let mut list = harness.draw_list();
    let style = StyleResolver::new(&theme);
    let counts: &[usize] = &[1_000, 10_000, 100_000];

    let mut group = c.benchmark_group("list_virtual");
    for &count in counts {
        let lw = List::new();
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                list.clear();
                let mut state = ListState::default();
                let mut input = InputState::default();
                lw.draw(
                    Rect::new(0.0, 0.0, 300.0, 600.0),
                    count,
                    &mut state,
                    &mut list,
                    &style,
                    &mut input,
                    |list, rect, item: ListItem| {
                        // Minimal closure: one text label per visible row.
                        list.text(TextBlock::new(
                            &format!("Item {}", item.index),
                            rect.x,
                            rect.y,
                        ));
                    },
                );
                std::hint::black_box(&list);
            });
        });
    }
    group.finish();
}

/// CPU-only cost of drawing a `Table` with a fixed column layout and `count`
/// rows, each with a few cells. Measures header + row chrome + scroll overhead.
fn bench_table(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let mut list = harness.draw_list();
    let style = StyleResolver::new(&theme);
    let row_counts: &[usize] = &[100, 1_000, 10_000];

    let columns = [
        TableColumn::new("Name", ColumnWidth::Flex(0.4)),
        TableColumn::new("Age", ColumnWidth::Fixed(60.0)),
        TableColumn::new("Score", ColumnWidth::Flex(0.3)),
        TableColumn::new("Rank", ColumnWidth::Flex(0.3)),
    ];

    let mut group = c.benchmark_group("table");
    for &rows in row_counts {
        // Build the cell grid once.
        let data: Vec<Vec<TableCell>> = (0..rows)
            .map(|i| {
                vec![
                    TableCell::new(format!("Item {i}")),
                    TableCell::new(format!("{}", 20 + (i % 50) as u32)),
                    TableCell::new(format!("{:.1}", (i as f64 * 7.3) % 100.0)),
                    TableCell::new(format!("#{}", i + 1)),
                ]
            })
            .collect();
        let table = Table::new(&columns);
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_with_input(BenchmarkId::from_parameter(rows), &rows, |b, _| {
            b.iter(|| {
                list.clear();
                let mut scroll = ScrollState::new();
                let mut input = InputState::default();
                table.draw(
                    Rect::new(0.0, 0.0, 600.0, 400.0),
                    &data,
                    &mut scroll,
                    &mut list,
                    &style,
                    &mut input,
                );
                std::hint::black_box(&list);
            });
        });
    }
    group.finish();
}

/// Overhead of the `Frame::run` / `UiContext` interactive façade vs raw widgets.
///
/// Builds N `text_button` + `checkbox` + `slider` + `text_input` verbs through
/// the auto-advancing facade. Measures the `UiState::begin_frame`/`end_frame`
/// cost plus the per-verb `UiContext` indirection.
fn bench_ui_context_frame(c: &mut Criterion) {
    let harness = Harness::new();
    let theme = Theme::default();
    let counts: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("ui_context_frame");
    for &count in counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut list = harness.draw_list();
            b.iter(|| {
                list.clear();
                let mut input = InputState::default();
                let mut state = UiState::new();
                Frame::new(&mut state, &mut input, &theme, &KeyboardNav)
                    .run(&mut list, |ui| {
                        for i in 0..count {
                            ui.text_button(&format!("Btn {i}"), None, None);
                            ui.checkbox("X", i % 2 == 0);
                            ui.slider(i as u64, 0.5, 0.0, 1.0, None);
                            let mut buf = format!("field {i}");
                            ui.text_input(i as u64, &mut buf, "placeholder", None);
                        }
                    });
                std::hint::black_box(&list);
            });
        });
    }
    group.finish();
}

/// CPU cost of `AnimationState::tick` + `animate_color` for N unique widget IDs.
///
/// Simulates a frame where N widgets each request an eased color transition
/// (the common pattern for hover/press feedback). Each `animate_color` call
/// does a hash lookup + lerp; `tick` reaps stale entries.
fn bench_animation(c: &mut Criterion) {
    let counts: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("animation");
    for &count in counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut anim = AnimationState::new();
            // First frame: settle all targets (no transition).
            anim.tick(0.016);
            for i in 0..count {
                anim.animate_color(
                    i as u64,
                    AnimSlot::Bg,
                    [0.3, 0.5, 0.8, 1.0],
                    0.12,
                    Easing::EaseOut,
                );
            }
            b.iter(|| {
                anim.tick(0.016);
                for i in 0..count {
                    // Alternate between two target colors so the transition
                    // is always in-flight (re-basing each iteration).
                    let target = if i % 2 == 0 {
                        [0.3, 0.5, 0.8, 1.0]
                    } else {
                        [0.8, 0.5, 0.3, 1.0]
                    };
                    std::hint::black_box(anim.animate_color(
                        i as u64,
                        AnimSlot::Bg,
                        target,
                        0.12,
                        Easing::EaseOut,
                    ));
                }
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_drawlist_build,
    bench_frame_render,
    bench_render_text_only,
    bench_nine_slice,
    bench_icons,
    bench_primitives_build,
    bench_primitives_render,
    bench_layout,
    bench_text_shape,
    bench_interactive_widgets,
    bench_text_input_edit,
    bench_scroll_view,
    bench_list_virtual,
    bench_table,
    bench_ui_context_frame,
    bench_animation,
);
criterion_main!(benches);
