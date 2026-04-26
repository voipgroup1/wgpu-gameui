//! Core drawing types - vertices, draw commands, and the DrawList.

use crate::text::TextBlock;

/// A colored vertex for triangle-based rendering.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    pub fn new(x: f32, y: f32, color: [f32; 4]) -> Self {
        Self {
            position: [x, y],
            color,
        }
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
}

/// Draw list for collecting render commands.
///
/// All shapes are tessellated into triangles immediately when added.
/// The vertices can be rendered directly as a triangle list.
#[derive(Default)]
pub struct DrawList {
    pub vertices: Vec<Vertex>,
    pub texts: Vec<TextBlock>,
    pub icons: Vec<IconDraw>,
    pub nine_slices: Vec<NineSliceDraw>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.texts.clear();
        self.icons.clear();
        self.nine_slices.clear();
    }

    /// Add a single triangle.
    pub fn triangle(
        &mut self,
        p0: (f32, f32),
        p1: (f32, f32),
        p2: (f32, f32),
        color: [f32; 4],
    ) {
        self.vertices.push(Vertex::new(p0.0, p0.1, color));
        self.vertices.push(Vertex::new(p1.0, p1.1, color));
        self.vertices.push(Vertex::new(p2.0, p2.1, color));
    }

    /// Add a rectangle (2 triangles, 6 vertices).
    pub fn quad(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
        let x0 = x;
        let y0 = y;
        let x1 = x + width;
        let y1 = y + height;

        // First triangle: top-left, top-right, bottom-right
        self.vertices.push(Vertex::new(x0, y0, color));
        self.vertices.push(Vertex::new(x1, y0, color));
        self.vertices.push(Vertex::new(x1, y1, color));

        // Second triangle: bottom-right, bottom-left, top-left
        self.vertices.push(Vertex::new(x1, y1, color));
        self.vertices.push(Vertex::new(x0, y1, color));
        self.vertices.push(Vertex::new(x0, y0, color));
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
    pub fn text(&mut self, block: TextBlock) {
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
        });
    }
}
