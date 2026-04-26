//! Core drawing types - vertices, draw commands, and the DrawList.

use crate::layout::Rect;
use crate::text::{FontSystemHandle, TextBlock, TextMeasurer};

pub(crate) const ROUNDED_RECT_CORNER_SEGMENTS: usize = 8;

/// A colored vertex for triangle-based rendering.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    pub clip_enabled: f32,
}

impl Vertex {
    pub fn new(x: f32, y: f32, color: [f32; 4]) -> Self {
        Self {
            position: [x, y],
            color,
            clip: [0.0; 4],
            clip_enabled: 0.0,
        }
    }

    pub fn with_clip(mut self, clip: Option<Rect>) -> Self {
        if let Some(clip) = clip {
            self.clip = [clip.x, clip.y, clip.width, clip.height];
            self.clip_enabled = 1.0;
        }
        self
    }
}

/// A textured quad command (e.g. an icon from a texture atlas).
#[derive(Clone, Debug)]
pub struct IconDraw {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Key identifying the icon (typically a file path).
    pub icon_key: String,
    pub clip: Option<Rect>,
}

/// A nine-slice textured panel draw command.
#[derive(Clone, Debug)]
pub struct NineSliceDraw {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Key identifying which nine-slice texture to use (e.g., "panel2").
    pub texture_key: String,
    pub clip: Option<Rect>,
}

/// Draw list for collecting render commands.
///
/// All shapes are tessellated into triangles immediately when added.
/// The vertices can be rendered directly as a triangle list.
#[derive(Default)]
pub struct DrawList {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub texts: Vec<TextBlock>,
    pub icons: Vec<IconDraw>,
    pub nine_slices: Vec<NineSliceDraw>,
    pub(crate) text_measurer: TextMeasurer,
    clip_stack: Vec<Rect>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a `DrawList` whose measurer shares the given `FontSystem`.
    ///
    /// Use this together with `TextRenderer::font_system_handle()` so measured text
    /// widths match what gets rendered to screen.
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self {
            text_measurer: TextMeasurer::with_font_system(font_system),
            ..Self::default()
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.texts.clear();
        self.icons.clear();
        self.nine_slices.clear();
        self.clip_stack.clear();
    }

    /// Measure text using glyphon's shaping/layout path.
    ///
    /// Pass `max_width = None` for unconstrained single-line measurement, or
    /// `Some(w)` to let glyphon wrap and report the resulting multi-line height.
    pub fn measure_text(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.text_measurer.measure(text, font_size, max_width)
    }

    /// Push a clipping rectangle. Nested clips are intersected with the current clip.
    pub fn push_clip(&mut self, rect: Rect) {
        let clip = match self.current_clip() {
            Some(current) => current
                .intersection(rect)
                .unwrap_or_else(|| Rect::new(rect.x, rect.y, 0.0, 0.0)),
            None => rect,
        };
        self.clip_stack.push(clip);
    }

    /// Pop the current clipping rectangle.
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    /// Return the active clipping rectangle.
    pub fn current_clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    fn vertex(&self, x: f32, y: f32, color: [f32; 4]) -> Vertex {
        Vertex::new(x, y, color).with_clip(self.current_clip())
    }

    /// Add a single triangle.
    pub fn triangle(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), color: [f32; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.push(self.vertex(p0.0, p0.1, color));
        self.vertices.push(self.vertex(p1.0, p1.1, color));
        self.vertices.push(self.vertex(p2.0, p2.1, color));
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Add a rectangle (2 triangles, 4 vertices, 6 indices).
    pub fn quad(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
        let x0 = x;
        let y0 = y;
        let x1 = x + width;
        let y1 = y + height;
        let base = self.vertices.len() as u32;

        self.vertices.push(self.vertex(x0, y0, color));
        self.vertices.push(self.vertex(x1, y0, color));
        self.vertices.push(self.vertex(x1, y1, color));
        self.vertices.push(self.vertex(x0, y1, color));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    /// Add a thick line segment as a quad.
    pub fn line(&mut self, p0: [f32; 2], p1: [f32; 2], thickness: f32, color: [f32; 4]) {
        let dx = p1[0] - p0[0];
        let dy = p1[1] - p0[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON || thickness <= 0.0 {
            return;
        }

        let half = thickness * 0.5;
        let ox = -dy / len * half;
        let oy = dx / len * half;
        let base = self.vertices.len() as u32;

        self.vertices
            .push(self.vertex(p0[0] + ox, p0[1] + oy, color));
        self.vertices
            .push(self.vertex(p1[0] + ox, p1[1] + oy, color));
        self.vertices
            .push(self.vertex(p1[0] - ox, p1[1] - oy, color));
        self.vertices
            .push(self.vertex(p0[0] - ox, p0[1] - oy, color));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    /// Add connected thick line segments without joins or caps.
    pub fn polyline(&mut self, points: &[[f32; 2]], thickness: f32, color: [f32; 4]) {
        for segment in points.windows(2) {
            self.line(segment[0], segment[1], thickness, color);
        }
    }

    /// Add a rounded rectangle.
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
        if radius <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            self.quad(rect.x, rect.y, rect.width, rect.height, color);
            return;
        }

        let radius = radius.min(rect.width * 0.5).min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;

        // Zero-overlap decomposition: center quad + 4 side strips + 4 corner fans.
        // Every pixel inside the rounded rect is covered by exactly one triangle, so
        // semi-transparent fills don't double-blend at the corners.

        // Center quad — fully inset rect, untouched by corner arcs.
        self.quad(
            x0 + radius,
            y0 + radius,
            rect.width - radius * 2.0,
            rect.height - radius * 2.0,
            color,
        );
        // Top side strip
        self.quad(
            x0 + radius,
            y0,
            rect.width - radius * 2.0,
            radius,
            color,
        );
        // Bottom side strip
        self.quad(
            x0 + radius,
            y1 - radius,
            rect.width - radius * 2.0,
            radius,
            color,
        );
        // Left side strip
        self.quad(x0, y0 + radius, radius, rect.height - radius * 2.0, color);
        // Right side strip
        self.quad(
            x1 - radius,
            y0 + radius,
            radius,
            rect.height - radius * 2.0,
            color,
        );

        self.rounded_corner(
            (x0 + radius, y0 + radius),
            radius,
            std::f32::consts::PI,
            std::f32::consts::PI * 1.5,
            color,
        );
        self.rounded_corner(
            (x1 - radius, y0 + radius),
            radius,
            std::f32::consts::PI * 1.5,
            std::f32::consts::TAU,
            color,
        );
        self.rounded_corner(
            (x1 - radius, y1 - radius),
            radius,
            0.0,
            std::f32::consts::FRAC_PI_2,
            color,
        );
        self.rounded_corner(
            (x0 + radius, y1 - radius),
            radius,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            color,
        );
    }

    fn rounded_corner(
        &mut self,
        center: (f32, f32),
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        color: [f32; 4],
    ) {
        for i in 0..ROUNDED_RECT_CORNER_SEGMENTS {
            let t0 = i as f32 / ROUNDED_RECT_CORNER_SEGMENTS as f32;
            let t1 = (i + 1) as f32 / ROUNDED_RECT_CORNER_SEGMENTS as f32;
            let a0 = start_angle + (end_angle - start_angle) * t0;
            let a1 = start_angle + (end_angle - start_angle) * t1;
            let p0 = (center.0 + a0.cos() * radius, center.1 + a0.sin() * radius);
            let p1 = (center.0 + a1.cos() * radius, center.1 + a1.sin() * radius);
            self.triangle(center, p0, p1, color);
        }
    }

    /// Add a filled convex polygon using fan triangulation from centroid.
    /// Points should be in order (clockwise or counter-clockwise).
    pub fn filled_polygon(&mut self, points: &[(f32, f32)], color: [f32; 4]) {
        if points.len() < 3 {
            return;
        }

        // Calculate centroid
        let mut cx = 0.0;
        let mut cy = 0.0;
        for &(x, y) in points {
            cx += x;
            cy += y;
        }
        cx /= points.len() as f32;
        cy /= points.len() as f32;

        // Fan triangulation: create triangle from centroid to each edge
        for i in 0..points.len() {
            let p0 = points[i];
            let p1 = points[(i + 1) % points.len()];
            self.triangle((cx, cy), p0, p1, color);
        }
    }

    /// Add text.
    pub fn text(&mut self, mut block: TextBlock) {
        if let Some(clip) = self.current_clip() {
            let natural_bounds = Rect::new(block.x, block.y, block.max_width, 2000.0);
            let text_bounds = block.clip.unwrap_or(natural_bounds);
            block.clip = text_bounds
                .intersection(clip)
                .or_else(|| Some(Rect::new(clip.x, clip.y, 0.0, 0.0)));
        }
        self.texts.push(block);
    }

    /// Add a textured icon.
    pub fn icon(&mut self, icon_key: &str, x: f32, y: f32, width: f32, height: f32) {
        self.icons.push(IconDraw {
            x,
            y,
            width,
            height,
            icon_key: icon_key.to_string(),
            clip: self.current_clip(),
        });
    }

    /// Add a nine-slice textured panel.
    pub fn nine_slice(&mut self, x: f32, y: f32, width: f32, height: f32, texture_key: &str) {
        self.nine_slices.push(NineSliceDraw {
            x,
            y,
            width,
            height,
            texture_key: texture_key.to_string(),
            clip: self.current_clip(),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::Rect;

    use super::DrawList;

    #[test]
    fn rounded_rect_emits_plausible_geometry() {
        let mut list = DrawList::new();
        list.rounded_rect(Rect::new(0.0, 0.0, 100.0, 40.0), 6.0, [1.0, 1.0, 1.0, 1.0]);

        assert!(list.vertices.len() > 12);
        assert!(list.indices.len() > 18);
    }

    #[test]
    fn line_emits_quad_geometry() {
        let mut list = DrawList::new();
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0, 1.0, 1.0, 1.0]);

        assert_eq!(list.vertices.len(), 4);
        assert_eq!(list.indices.len(), 6);
    }

    #[test]
    fn clip_stack_marks_emitted_commands() {
        let mut list = DrawList::new();
        let clip = Rect::new(10.0, 20.0, 30.0, 40.0);

        list.push_clip(clip);
        list.quad(0.0, 0.0, 100.0, 100.0, [1.0, 1.0, 1.0, 1.0]);
        list.text(crate::text::TextBlock::new("clipped", 0.0, 0.0));
        list.icon("icon", 0.0, 0.0, 10.0, 10.0);
        list.pop_clip();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0, 1.0, 1.0, 1.0]);

        assert_eq!(list.vertices[0].clip_enabled, 1.0);
        assert_eq!(list.vertices[0].clip, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(list.texts[0].clip, Some(Rect::new(10.0, 20.0, 30.0, 40.0)));
        assert_eq!(list.icons[0].clip, Some(clip));
        assert_eq!(list.vertices[4].clip_enabled, 0.0);
    }
}
