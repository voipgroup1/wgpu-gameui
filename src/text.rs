//! Text rendering using glyphon.
//! Provides GPU-accelerated text rendering with proper font shaping.

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer as GlyphonRenderer, Viewport,
};

pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: GlyphonRenderer,
    width: u32,
    height: u32,
}

impl TextRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer = GlyphonRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
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

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
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

        // Create buffers for each text block
        let mut buffers: Vec<Buffer> = Vec::with_capacity(texts.len());
        for text in texts {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(text.font_size, text.line_height));
            buffer.set_size(&mut self.font_system, Some(text.max_width), None);
            buffer.set_text(
                &mut self.font_system,
                &text.content,
                Attrs::new().family(Family::SansSerif).color(text.color),
                Shaping::Advanced,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
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
                bounds: TextBounds {
                    left: text.x as i32,
                    top: text.y as i32,
                    right: (text.x + text.max_width) as i32,
                    bottom: (text.y + 2000.0) as i32,
                },
                default_color: text.color,
                custom_glyphs: &[],
            })
            .collect();

        // Prepare the renderer
        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
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

        self.renderer.render(&self.atlas, &self.viewport, &mut pass).unwrap();
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
}
