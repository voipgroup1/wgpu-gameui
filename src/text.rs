//! Text rendering using glyphon.
//! Provides GPU-accelerated text rendering with proper font shaping.

use std::sync::{Arc, Mutex};

use crate::layout::Rect;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer as GlyphonRenderer, Viewport,
};

/// Shared handle to a glyphon `FontSystem`.
///
/// Both `TextRenderer` and `TextMeasurer` hold the same handle so measured text widths
/// (used for layout) match rendered glyphs (used for output) — including any custom
/// fonts loaded into the system later.
pub type FontSystemHandle = Arc<Mutex<FontSystem>>;

/// Create a new shared `FontSystem` handle.
pub fn shared_font_system() -> FontSystemHandle {
    Arc::new(Mutex::new(FontSystem::new()))
}

pub struct TextRenderer {
    font_system: FontSystemHandle,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: GlyphonRenderer,
    width: u32,
    height: u32,
}

impl TextRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = shared_font_system();
        Self::with_font_system(device, queue, format, font_system)
    }

    /// Construct a `TextRenderer` reusing an existing shared `FontSystem`.
    pub fn with_font_system(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: FontSystemHandle,
    ) -> Self {
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            GlyphonRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let viewport = Viewport::new(device, &cache);

        Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            width: 1,
            height: 1,
        }
    }

    /// Get a clone of the shared font system handle.
    ///
    /// Use this to construct a `DrawList` / `TextMeasurer` that shares font state with
    /// this renderer.
    pub fn font_system_handle(&self) -> FontSystemHandle {
        Arc::clone(&self.font_system)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Measure text using glyphon's shaping/layout path without preparing GPU atlas state.
    ///
    /// This mutates the internal font system cache as glyphon shapes text, but does not
    /// mutate rendered atlas or renderer state.
    pub fn measure(&mut self, text: &str, font_size: f32) -> (f32, f32) {
        let mut fs = self.font_system.lock().expect("FontSystem poisoned");
        measure_with_font_system(&mut fs, text, font_size, None)
    }

    /// Prepare and render text in a single call.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        texts: &[TextBlock],
    ) {
        if texts.is_empty() {
            return;
        }

        // Update viewport
        self.viewport.update(
            queue,
            Resolution {
                width: self.width,
                height: self.height,
            },
        );

        let mut fs = self.font_system.lock().expect("FontSystem poisoned");

        // Create buffers for each text block
        let mut buffers: Vec<Buffer> = Vec::with_capacity(texts.len());
        for text in texts {
            let mut buffer = Buffer::new(&mut *fs, Metrics::new(text.font_size, text.line_height));
            buffer.set_size(&mut *fs, Some(text.max_width), None);
            buffer.set_text(
                &mut *fs,
                &text.content,
                Attrs::new().family(Family::SansSerif).color(text.color),
                Shaping::Advanced,
            );
            buffer.shape_until_scroll(&mut *fs, false);
            buffers.push(buffer);
        }

        // Build text areas referencing the buffers
        let text_areas: Vec<TextArea> = texts
            .iter()
            .zip(buffers.iter())
            .map(|(text, buffer)| TextArea {
                buffer,
                left: text.x,
                top: text.y,
                scale: 1.0,
                bounds: text.bounds(),
                default_color: text.color,
                custom_glyphs: &[],
            })
            .collect();

        // Prepare the renderer
        self.renderer
            .prepare(
                device,
                queue,
                &mut *fs,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .unwrap();

        // Render
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Text Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        self.renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .unwrap();
    }
}

/// CPU-side glyphon text measurer for layout and widget construction.
pub struct TextMeasurer {
    font_system: FontSystemHandle,
}

impl TextMeasurer {
    /// Create a measurer with its own private `FontSystem`.
    ///
    /// Prefer [`TextMeasurer::with_font_system`] when a `TextRenderer` already exists,
    /// so measured widths match rendered glyphs.
    pub fn new() -> Self {
        Self {
            font_system: shared_font_system(),
        }
    }

    /// Create a measurer that shares its `FontSystem` with another component (typically
    /// a `TextRenderer`).
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self { font_system }
    }

    /// Get a clone of the shared font system handle.
    pub fn font_system_handle(&self) -> FontSystemHandle {
        Arc::clone(&self.font_system)
    }

    /// Measure text using glyphon's shaping/layout path.
    ///
    /// `max_width` constrains the shaping width; pass `None` for unconstrained
    /// single-line measurement, or `Some(w)` to let glyphon wrap the text and report
    /// the resulting multi-line height.
    ///
    /// This mutates glyphon's font system cache while shaping; it does not touch any GPU
    /// renderer, atlas, or swash cache state.
    pub fn measure(&mut self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        let mut fs = self.font_system.lock().expect("FontSystem poisoned");
        measure_with_font_system(&mut fs, text, font_size, max_width)
    }
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

fn measure_with_font_system(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    max_width: Option<f32>,
) -> (f32, f32) {
    let line_height = font_size * 1.25;
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    let shape_width = max_width.unwrap_or(f32::MAX / 4.0);
    buffer.set_size(font_system, Some(shape_width), None);
    buffer.set_text(
        font_system,
        text,
        Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    let mut width = 0.0f32;
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height += run.line_height;
    }

    if text.is_empty() {
        (0.0, line_height)
    } else {
        (width, height.max(line_height))
    }
}

/// A block of text to render.
#[derive(Clone)]
pub struct TextBlock {
    pub content: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub max_width: f32,
    pub color: Color,
    pub clip: Option<Rect>,
}

impl TextBlock {
    pub fn new(content: impl Into<String>, x: f32, y: f32) -> Self {
        Self {
            content: content.into(),
            x,
            y,
            font_size: 16.0,
            line_height: 20.0,
            max_width: 800.0,
            color: Color::rgb(255, 255, 255),
            clip: None,
        }
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self.line_height = size * 1.25;
        self
    }

    pub fn with_max_width(mut self, width: f32) -> Self {
        self.max_width = width;
        self
    }

    pub fn with_color(mut self, r: u8, g: u8, b: u8) -> Self {
        self.color = Color::rgb(r, g, b);
        self
    }

    pub fn with_rgba(mut self, r: u8, g: u8, b: u8, a: u8) -> Self {
        self.color = Color::rgba(r, g, b, a);
        self
    }

    pub fn with_clip(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self
    }

    fn bounds(&self) -> TextBounds {
        if let Some(clip) = self.clip {
            TextBounds {
                left: clip.x as i32,
                top: clip.y as i32,
                right: (clip.x + clip.width) as i32,
                bottom: (clip.y + clip.height) as i32,
            }
        } else {
            TextBounds {
                left: self.x as i32,
                top: self.y as i32,
                right: (self.x + self.max_width) as i32,
                bottom: (self.y + 2000.0) as i32,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextMeasurer;

    #[test]
    fn measures_text_with_glyphon_layout() {
        let mut measurer = TextMeasurer::new();
        let (hello_width, hello_height) = measurer.measure("Hello", 16.0, None);
        assert!(hello_width > 0.0);
        assert!(hello_height > 0.0);

        let font_size = 16.0;
        let (m_width, _) = measurer.measure("M", font_size, None);
        let approximate_width = "M".len() as f32 * font_size * 0.5;
        assert!((m_width - approximate_width).abs() > f32::EPSILON);
    }

    #[test]
    fn measure_with_max_width_wraps_to_multiple_lines() {
        let mut measurer = TextMeasurer::new();
        let long = "The quick brown fox jumps over the lazy dog repeatedly each morning.";
        let (_, h_unwrapped) = measurer.measure(long, 14.0, None);
        let (_, h_wrapped) = measurer.measure(long, 14.0, Some(80.0));
        assert!(h_wrapped > h_unwrapped);
    }
}
