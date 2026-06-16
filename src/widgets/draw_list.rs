//! Core drawing types - vertices, draw commands, and the DrawList.

use crate::affine::Affine2;
use crate::layout::Rect;
use crate::render::SpriteId;
#[cfg(feature = "phosphor-icons")]
use crate::render::{PhosphorIcon, phosphor_glyph_id};
use crate::text::{FontHandle, FontSystemHandle, FontVMetrics, TextBlock, TextMeasurer, Underline};

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
///
/// Carries pre-transformed corners in TL, TR, BR, BL order so rotated/scaled
/// sprites tessellate correctly.
///
/// `sprite` is the resolved atlas handle. When `None`, the renderer falls back
/// to looking up `icon_key` in the atlas at render time (slightly slower; one
/// `HashMap<String, SpriteId>` lookup per icon per frame). Prefer resolving the
/// sprite once at registration via [`DrawList::icon_sprite`].
#[derive(Clone, Debug)]
pub struct IconDraw {
    /// Pre-transformed corners, TL/TR/BR/BL.
    pub corners: [[f32; 2]; 4],
    /// Pre-resolved atlas handle, if known.
    pub sprite: Option<SpriteId>,
    /// Name fallback for late-resolved sprites.
    pub icon_key: String,
    /// Multiplied with sampled atlas color. Default white.
    pub tint: [f32; 4],
    pub clip: Option<Rect>,
    /// Optional normalized source sub-rect `[u0, v0, u1, v1]` (0..1 within the
    /// sprite) for cropped draws. `None` draws the whole sprite. Resolved
    /// against the atlas region at render time.
    pub src: Option<[f32; 4]>,
}

/// A single instanced "chrome" rect (button background + border) for the SDF
/// rounded-rect pipeline.
///
/// Field layout matches the per-instance vertex attributes in `ui.wgsl`
/// (`vs_chrome`), so a `&[ChromeInstance]` uploads straight to the instance
/// buffer with no repacking. All geometry is computed in the fragment shader
/// from these values, which is why thousands of identical-shape buttons collapse
/// to one base mesh + N small records instead of re-tessellating ~80 verts each.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChromeInstance {
    /// World-space rect: `[x, y, w, h]` (transform already baked in).
    pub rect: [f32; 4],
    /// Fill (background) color, tint already applied.
    pub bg: [f32; 4],
    /// Border color, tint already applied.
    pub border: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `params[2] > 0.5`).
    pub clip: [f32; 4],
    /// `[corner_radius, border_thickness, clip_enabled, _pad]`.
    pub params: [f32; 4],
}

/// A single instanced circle (filled disc or ring outline) for the SDF circle
/// pipeline.
///
/// Field layout matches the per-instance vertex attributes in `ui.wgsl`
/// (`vs_circle`), so a `&[CircleInstance]` uploads straight to the instance
/// buffer. The fragment computes the disc/ring from a signed distance, so a
/// smooth anti-aliased circle of any size is one base mesh + one small record
/// instead of a re-tessellated 16-64-segment fan every frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CircleInstance {
    /// `[center_x, center_y, radius, thickness]` (transform already baked in).
    /// `thickness <= 0` is a filled disc; `> 0` is a ring centered on `radius`.
    pub center: [f32; 4],
    /// Color, tint already applied.
    pub color: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `params[0] > 0.5`).
    pub clip: [f32; 4],
    /// `[clip_enabled, _pad, _pad, _pad]`.
    pub params: [f32; 4],
}

/// One entry in a [`DrawList`]'s ordered color-stage command stream.
///
/// The colored-quad stage is no longer a single soup draw: chrome rects are
/// instanced and must interleave with surrounding soup geometry in submission
/// order (a hover overlay quad drawn *over* a button, a panel *under* it). The
/// renderer walks these in order, drawing each soup index sub-range with the
/// color pipeline and each chrome instance sub-range with the instanced chrome
/// pipeline. When no `chrome_rect` is ever called the stream stays empty and the
/// renderer keeps its original single-draw fast path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ColorCmd {
    /// Draw soup index positions `start..end` (absolute into `indices`).
    Soup { indices: std::ops::Range<u32> },
    /// Draw chrome instances `start..end` (into `chrome_instances`).
    Chrome { instances: std::ops::Range<u32> },
    /// Draw circle instances `start..end` (into `circle_instances`).
    Circle { instances: std::ops::Range<u32> },
}

/// Opaque handle to a registered nine-slice resource.
pub type NineSliceId = u32;

/// A nine-slice textured panel draw command.
///
/// Carries the local-space rect plus the affine transform that maps local
/// space to world (screen) space. Tessellation computes the 9 sub-rect
/// corners in local space and runs each through `transform`.
#[derive(Clone, Debug)]
pub struct NineSliceDraw {
    /// Local-space rect (pre-transform).
    pub local: Rect,
    /// Affine to apply to each corner during tessellation.
    pub transform: Affine2,
    /// Pre-resolved nine-slice handle.
    pub nine_slice: Option<NineSliceId>,
    /// Name fallback for late resolution.
    pub texture_key: String,
    /// Multiplied with sampled color. Default white.
    pub tint: [f32; 4],
    pub clip: Option<Rect>,
}

/// A vector icon drawn through the MSDF icon atlas (Phosphor).
///
/// Like [`NineSliceDraw`], this stores the **local rect + transform** rather than
/// pre-baked corners: the renderer fits-and-centers the glyph tile inside `local`
/// (so placement is bearing-independent) and then transforms the resulting quad
/// corners, giving rotation/scale support for free.
#[cfg(feature = "phosphor-icons")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconMsdf {
    /// Local-space rect (pre-transform) the icon is fit-centered into.
    pub local: Rect,
    /// Affine applied to each fitted quad corner during tessellation.
    pub transform: Affine2,
    /// Resolved Phosphor glyph index (from [`PhosphorIcon`] at push time).
    pub glyph_id: u16,
    /// Multiplied with the sampled field's fill color. Default white.
    pub tint: [f32; 4],
    pub clip: Option<Rect>,
}

/// Draw list for collecting render commands.
///
/// Owns a transform stack and a tint stack: every primitive method consults
/// the top of both stacks at push time so widgets that already take absolute
/// `Rect`s remain transform-aware without code changes.
pub struct DrawList {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub texts: Vec<TextBlock>,
    pub icons: Vec<IconDraw>,
    pub nine_slices: Vec<NineSliceDraw>,
    /// MSDF vector icons (Phosphor), rendered by the text renderer's icon pass.
    #[cfg(feature = "phosphor-icons")]
    pub icons_msdf: Vec<IconMsdf>,
    /// Instanced chrome rects (button backgrounds/borders, plus rect/rounded-rect
    /// fills and outlines). Drawn by the chrome pipeline; interleaved with soup
    /// geometry via [`DrawList::color_cmds`].
    pub chrome_instances: Vec<ChromeInstance>,
    /// Instanced circles (filled discs + ring outlines). Drawn by the circle
    /// SDF pipeline; interleaved with soup/chrome via [`DrawList::color_cmds`].
    pub circle_instances: Vec<CircleInstance>,
    /// Ordered color-stage command stream (soup runs interleaved with chrome
    /// instance runs). Empty unless [`DrawList::chrome_rect`] was used, in which
    /// case the renderer falls back to a single soup draw.
    pub(crate) color_cmds: Vec<ColorCmd>,
    /// Count of soup index positions already committed to a `Soup` command. Soup
    /// appended after the last command is the implicit trailing run.
    pub(crate) soup_committed_indices: u32,
    pub(crate) text_measurer: TextMeasurer,
    clip_stack: Vec<Rect>,
    transform_stack: Vec<Affine2>,
    tint_stack: Vec<[f32; 4]>,
    /// Logged-once flag for "tried to draw rotated text" — glyphon does not
    /// support rotation, so we silently render axis-aligned.
    text_rotation_warned: bool,
}

impl Default for DrawList {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texts: Vec::new(),
            icons: Vec::new(),
            nine_slices: Vec::new(),
            #[cfg(feature = "phosphor-icons")]
            icons_msdf: Vec::new(),
            chrome_instances: Vec::new(),
            circle_instances: Vec::new(),
            color_cmds: Vec::new(),
            soup_committed_indices: 0,
            text_measurer: TextMeasurer::default(),
            clip_stack: Vec::new(),
            transform_stack: vec![Affine2::IDENTITY],
            tint_stack: vec![[1.0, 1.0, 1.0, 1.0]],
            text_rotation_warned: false,
        }
    }
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a `DrawList` whose measurer shares the given `FontSystem`.
    ///
    /// Use this together with `TextRenderer::font_system_handle()` so measured text
    /// widths match what gets rendered to screen.
    ///
    /// IMPORTANT: this builds every field explicitly rather than via
    /// `..Self::default()`. Struct-update syntax would fully evaluate
    /// `Self::default()` first — which constructs a throwaway
    /// `TextMeasurer::default()` whose `FontSystem::new()` scans the entire
    /// system font database (multiple milliseconds) — only to immediately
    /// overwrite and drop it. Callers that build a `DrawList` per frame would
    /// pay that font-DB scan every frame. Constructing fields directly with the
    /// caller-supplied (shared) font system avoids the wasted scan entirely.
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texts: Vec::new(),
            icons: Vec::new(),
            nine_slices: Vec::new(),
            #[cfg(feature = "phosphor-icons")]
            icons_msdf: Vec::new(),
            chrome_instances: Vec::new(),
            circle_instances: Vec::new(),
            color_cmds: Vec::new(),
            soup_committed_indices: 0,
            text_measurer: TextMeasurer::with_font_system(font_system),
            clip_stack: Vec::new(),
            transform_stack: vec![Affine2::IDENTITY],
            tint_stack: vec![[1.0, 1.0, 1.0, 1.0]],
            text_rotation_warned: false,
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.texts.clear();
        self.icons.clear();
        self.nine_slices.clear();
        #[cfg(feature = "phosphor-icons")]
        self.icons_msdf.clear();
        self.chrome_instances.clear();
        self.circle_instances.clear();
        self.color_cmds.clear();
        self.soup_committed_indices = 0;
        self.clip_stack.clear();
        self.transform_stack.clear();
        self.transform_stack.push(Affine2::IDENTITY);
        self.tint_stack.clear();
        self.tint_stack.push([1.0, 1.0, 1.0, 1.0]);
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

    /// Per-font vertical metrics for optical (cap-height) centring, for the given
    /// font at Normal weight/style (the only combination widget labels centre).
    /// Cached per font. Exposed mainly so debug tooling can draw the band; most
    /// callers want [`Self::vcentered_text_y`].
    pub fn font_vmetrics(&mut self, font: Option<&FontHandle>) -> FontVMetrics {
        self.text_measurer
            .vmetrics(font, glyphon::Weight::NORMAL, glyphon::Style::Normal)
    }

    /// Top `y` for a single-line text block of `font_size` so the label `text` is
    /// *optically* centred over the span `[top, top + height]`, using the font's
    /// real metrics.
    ///
    /// This is the font-aware counterpart of [`crate::text::vcentered_line_y`]:
    /// where that centres the em box (ascent+descent, biased low by the empty
    /// descent space), this centres the band the label's visual mass occupies.
    /// Which band that is depends on the text — labels with lowercase letters
    /// centre on the **x-height** body, all-caps/numeric labels on the taller
    /// **cap-height** band, and labels containing **CJK** on the ideographic ink
    /// centre (see [`FontVMetrics::center_offset_ratio`]) — so text reads as
    /// centred across scripts and case. Pass the same `font` the block renders
    /// with. Degrades to em-box centring when the metrics are unavailable.
    pub fn vcentered_text_y(
        &mut self,
        top: f32,
        height: f32,
        font_size: f32,
        font: Option<&FontHandle>,
        text: &str,
    ) -> f32 {
        let m = self.font_vmetrics(font);
        top + height / 2.0 - font_size * m.visual_center_ratio(text)
    }

    /// Compute per-character cursor x-positions for the given text.
    ///
    /// Returns a `Vec<(usize, f32)>` mapping byte indices in `text` to their
    /// x-offset (pixels) from the left edge. Use this for click-to-position
    /// cursor placement and selection highlight rendering.
    ///
    /// `max_width` constrains the layout width (glyphon may wrap). Pass
    /// `font_size * 0.0` for single-line mode with effectively infinite width.
    pub fn text_cursor_positions(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> Vec<(usize, f32)> {
        let handle = self.text_measurer.font_system_handle();
        let mut fs = handle.lock().expect("FontSystem poisoned");
        let mw = max_width.unwrap_or(f32::MAX / 4.0);
        let lh = font_size * 1.25;
        crate::text::text_cursor_positions(&mut fs, text, font_size, lh, mw, None)
    }

    /// Line-aware caret layout for the given text — the multi-line counterpart of
    /// [`Self::text_cursor_positions`]. Returns one [`crate::text::CaretPos`] per
    /// cluster boundary, preserving per-visual-line geometry (line index, top y,
    /// height) needed for vertical navigation, per-line selection, and
    /// click-to-place hit testing in a multi-line `TextInput`.
    ///
    /// `wrap` controls line breaking (use [`crate::text::WrapMode::None`] for
    /// single-line, [`crate::text::WrapMode::WordOrGlyph`] for a textarea).
    /// `max_width` constrains the layout width; pass `None` for effectively
    /// infinite width (single-line).
    pub fn text_caret_layout(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        wrap: crate::text::WrapMode,
        direction: crate::text::TextDirection,
    ) -> Vec<crate::text::CaretPos> {
        let handle = self.text_measurer.font_system_handle();
        let mut fs = handle.lock().expect("FontSystem poisoned");
        let mw = max_width.unwrap_or(f32::MAX / 4.0);
        let lh = font_size * 1.25;
        crate::text::text_caret_layout(&mut fs, text, font_size, lh, mw, wrap, None, direction)
    }

    /// Visual-order glyph layout for bidi-aware editing — the source for
    /// [`crate::text::selection_rects`] and [`crate::text::visual_caret_neighbor`].
    /// Same shaping parameters as [`Self::text_caret_layout`].
    pub fn text_visual_layout(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        wrap: crate::text::WrapMode,
        direction: crate::text::TextDirection,
    ) -> Vec<crate::text::VisualGlyph> {
        let handle = self.text_measurer.font_system_handle();
        let mut fs = handle.lock().expect("FontSystem poisoned");
        let mw = max_width.unwrap_or(f32::MAX / 4.0);
        let lh = font_size * 1.25;
        crate::text::text_visual_layout(&mut fs, text, font_size, lh, mw, wrap, None, direction)
    }

    // ---- Clip stack ----

    /// Push a clipping rectangle. Nested clips are intersected with the current clip.
    ///
    /// **Note:** when the active transform has rotation or shear, the rect is
    /// transformed to its AABB before being intersected; clipping is therefore
    /// approximate (over-clips along the diagonal) under rotation. Document the
    /// limitation rather than silently drawing wrong.
    pub fn push_clip(&mut self, rect: Rect) {
        let world_rect = self.current_transform().transform_rect_aabb(rect);
        let clip = match self.current_clip() {
            Some(current) => current
                .intersection(world_rect)
                .unwrap_or_else(|| Rect::new(world_rect.x, world_rect.y, 0.0, 0.0)),
            None => world_rect,
        };
        self.clip_stack.push(clip);
    }

    /// Push a clipping rectangle **without** intersecting the parent clip — the
    /// new clip *replaces* whatever was active (Teardown's `UiClipRect`/`UiWindow`
    /// with `inherit = false`). The rect is still transformed to its world-space
    /// AABB by the active transform, with the same rotation caveat as
    /// [`push_clip`].
    pub fn push_clip_exact(&mut self, rect: Rect) {
        let world_rect = self.current_transform().transform_rect_aabb(rect);
        self.clip_stack.push(world_rect);
    }

    /// Pop the current clipping rectangle.
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    /// Number of clips currently on the stack. Used to scope clips to a
    /// push/pop frame (record the depth on push, [`truncate_clip`] back on pop).
    pub fn clip_len(&self) -> usize {
        self.clip_stack.len()
    }

    /// Drop clips until the stack is `len` deep (no-op if already ≤ `len`).
    pub fn truncate_clip(&mut self, len: usize) {
        self.clip_stack.truncate(len);
    }

    /// Return the active clipping rectangle (in world / screen space).
    pub fn current_clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    // ---- Transform stack ----

    /// Push the current transform onto the stack (the new top is a clone of
    /// the old top, matching Teardown's `UiPush`).
    pub fn push_transform(&mut self) {
        let top = *self.transform_stack.last().unwrap_or(&Affine2::IDENTITY);
        self.transform_stack.push(top);
    }

    /// Pop the top transform. Refuses to pop below 1 entry (the implicit
    /// identity at the base of the stack).
    pub fn pop_transform(&mut self) {
        if self.transform_stack.len() > 1 {
            self.transform_stack.pop();
        }
    }

    /// Return the current (top) transform.
    pub fn current_transform(&self) -> Affine2 {
        *self.transform_stack.last().unwrap_or(&Affine2::IDENTITY)
    }

    /// Post-multiply the current transform by a translation.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.compose_top(&Affine2::translation(dx, dy));
    }

    /// Post-multiply the current transform by a rotation about the local origin.
    pub fn rotate(&mut self, angle_radians: f32) {
        self.compose_top(&Affine2::rotation(angle_radians));
    }

    /// Post-multiply the current transform by a non-uniform scale.
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.compose_top(&Affine2::scale(sx, sy));
    }

    fn compose_top(&mut self, m: &Affine2) {
        if let Some(top) = self.transform_stack.last_mut() {
            *top = top.compose(m);
        }
    }

    // ---- Tint stack ----

    /// Push the current tint onto the stack (clone of top).
    pub fn push_tint(&mut self) {
        let top = *self.tint_stack.last().unwrap_or(&[1.0, 1.0, 1.0, 1.0]);
        self.tint_stack.push(top);
    }

    /// Pop the top tint. Refuses to pop below 1 entry.
    pub fn pop_tint(&mut self) {
        if self.tint_stack.len() > 1 {
            self.tint_stack.pop();
        }
    }

    /// Replace the current tint (Teardown's `UiColor` semantics).
    pub fn set_tint(&mut self, rgba: [f32; 4]) {
        if let Some(top) = self.tint_stack.last_mut() {
            *top = rgba;
        }
    }

    /// Multiply the current tint by `rgba` (Teardown's `UiColorFilter` semantics).
    pub fn multiply_tint(&mut self, rgba: [f32; 4]) {
        if let Some(top) = self.tint_stack.last_mut() {
            top[0] *= rgba[0];
            top[1] *= rgba[1];
            top[2] *= rgba[2];
            top[3] *= rgba[3];
        }
    }

    /// Return the current (top) tint.
    pub fn current_tint(&self) -> [f32; 4] {
        *self.tint_stack.last().unwrap_or(&[1.0, 1.0, 1.0, 1.0])
    }

    /// Combine an input color with the current tint.
    fn apply_tint(&self, color: [f32; 4]) -> [f32; 4] {
        let t = self.current_tint();
        [
            color[0] * t[0],
            color[1] * t[1],
            color[2] * t[2],
            color[3] * t[3],
        ]
    }

    /// Build a colored vertex by transforming local position through the current
    /// affine and multiplying the input color by the current tint.
    fn vertex(&self, x: f32, y: f32, color: [f32; 4]) -> Vertex {
        let world = self.current_transform().transform_point([x, y]);
        let tinted = self.apply_tint(color);
        Vertex::new(world[0], world[1], tinted).with_clip(self.current_clip())
    }

    // ---- Primitives ----

    /// Add a single triangle.
    pub fn triangle(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), color: [f32; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.push(self.vertex(p0.0, p0.1, color));
        self.vertices.push(self.vertex(p1.0, p1.1, color));
        self.vertices.push(self.vertex(p2.0, p2.1, color));
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Add a filled rectangle.
    ///
    /// Fast path (translation-only transform): records a single fill-only SDF
    /// chrome instance (radius 0) instead of two soup triangles — so thousands of
    /// rects collapse to small per-instance records the renderer rasterizes,
    /// with no per-frame soup re-tessellation/re-upload. Under any rotation/scale/
    /// shear it falls back to soup geometry ([`DrawList::quad_soup`]) so the rect
    /// still transforms correctly.
    pub fn quad(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        if self.current_transform().is_translate_only() {
            self.fill_rect_instance(Rect::new(x, y, width, height), 0.0, color);
        } else {
            self.quad_soup(x, y, width, height, color);
        }
    }

    /// Tessellate a filled rectangle into the vertex soup (2 triangles, 4
    /// vertices). The fallback path for [`DrawList::quad`] under non-translation
    /// transforms, and the building block for the other soup primitives.
    fn quad_soup(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
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

    /// Add a filled rectangle with a distinct color per corner, in
    /// `[top_left, top_right, bottom_right, bottom_left]` order — the GPU
    /// interpolates linearly across the two triangles, giving a gradient fill
    /// (used by the color picker's SV square / hue / alpha bars).
    ///
    /// Always soup geometry (a gradient can't use the instanced-chrome fast
    /// path), and like [`quad_soup`](Self::quad_soup) it honors the current
    /// transform + tint via [`vertex`](Self::vertex). No-op on non-positive size.
    pub fn quad_gradient(&mut self, rect: Rect, colors: [[f32; 4]; 4]) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;
        let base = self.vertices.len() as u32;

        // Same winding as `quad_soup`: TL, TR, BR, BL.
        self.vertices.push(self.vertex(x0, y0, colors[0]));
        self.vertices.push(self.vertex(x1, y0, colors[1]));
        self.vertices.push(self.vertex(x1, y1, colors[2]));
        self.vertices.push(self.vertex(x0, y1, colors[3]));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    /// Fill `rect` with a linear gradient from `start` to `end` along `angle`
    /// (radians; `0` = left→right, `π/2` = top→bottom, increasing clockwise in
    /// screen space where +y points down).
    ///
    /// Exact for any angle: a linear color ramp is an affine function of
    /// position, which the GPU's bilinear corner interpolation reproduces
    /// precisely — so this is just [`quad_gradient`](Self::quad_gradient) with
    /// the four corner colors projected onto the gradient axis. No-op on
    /// non-positive size. For the cardinal directions prefer the cheaper
    /// [`horizontal_gradient`](Self::horizontal_gradient) /
    /// [`vertical_gradient`](Self::vertical_gradient).
    pub fn linear_gradient(&mut self, rect: Rect, start: [f32; 4], end: [f32; 4], angle: f32) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let (s, c) = angle.sin_cos();
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;
        // Project each corner (TL, TR, BR, BL) onto the gradient direction, then
        // normalize to [0,1] across the projected extent → per-corner colors.
        let proj = [
            x0 * c + y0 * s,
            x1 * c + y0 * s,
            x1 * c + y1 * s,
            x0 * c + y1 * s,
        ];
        let min = proj.iter().copied().fold(f32::INFINITY, f32::min);
        let max = proj.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let span = (max - min).max(f32::EPSILON);
        let colors = proj.map(|p| lerp_color(start, end, (p - min) / span));
        self.quad_gradient(rect, colors);
    }

    /// Fill `rect` with a horizontal gradient (`left` edge → `right` edge).
    pub fn horizontal_gradient(&mut self, rect: Rect, left: [f32; 4], right: [f32; 4]) {
        self.quad_gradient(rect, [left, right, right, left]);
    }

    /// Fill `rect` with a vertical gradient (`top` edge → `bottom` edge).
    pub fn vertical_gradient(&mut self, rect: Rect, top: [f32; 4], bottom: [f32; 4]) {
        self.quad_gradient(rect, [top, top, bottom, bottom]);
    }

    /// Fill `rect` with a radial gradient: `inner` at the center fading to
    /// `outer` toward the edges, as a triangle fan of `segments` wedges (clamped
    /// to ≥ 3). The fan radius reaches the rect's farthest corner so the whole
    /// rect is filled, and the geometry is clipped to `rect`. Honors the current
    /// transform/tint/clip like the other soup primitives. No-op on non-positive
    /// size.
    ///
    /// The fade is circular (equal in x and y), so for a non-square rect the
    /// iso-color rings are circles centered in the rect, not ellipses.
    pub fn radial_gradient(&mut self, rect: Rect, inner: [f32; 4], outer: [f32; 4], segments: u32) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let segments = segments.max(3);
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        // Circumradius whose inscribed circle (apothem) still reaches the
        // farthest corner, so the N-gon fully covers the rect before clipping.
        let half_diag = (rect.width * rect.width + rect.height * rect.height).sqrt() * 0.5;
        let r = half_diag / (std::f32::consts::PI / segments as f32).cos();

        self.push_clip(rect);
        let base = self.vertices.len() as u32;
        self.vertices.push(self.vertex(cx, cy, inner));
        for i in 0..segments {
            let theta = std::f32::consts::TAU * (i as f32) / (segments as f32);
            let (s, c) = theta.sin_cos();
            self.vertices
                .push(self.vertex(cx + r * c, cy + r * s, outer));
        }
        for i in 0..segments {
            let a = base + 1 + i;
            let b = base + 1 + ((i + 1) % segments);
            self.indices.extend_from_slice(&[base, a, b]);
        }
        self.pop_clip();
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

        // Compute offsets in local space; transform happens inside `vertex()`.
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

    /// Add a rounded rectangle. Geometry is built in local space and
    /// transformed at vertex push time, so a rotated transform produces a
    /// rotated rounded rect.
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
        if radius <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            self.quad(rect.x, rect.y, rect.width, rect.height, color);
            return;
        }

        // Fast path: one fill-only SDF instance (the shader clamps the radius and
        // rasterizes anti-aliased corners) instead of 5 strip quads + 4×8 corner
        // triangles into the soup. Falls back to tessellation under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.fill_rect_instance(rect, radius, color);
            return;
        }

        let radius = radius.min(rect.width * 0.5).min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;

        // Center quad — fully inset rect, untouched by corner arcs.
        self.quad(
            x0 + radius,
            y0 + radius,
            rect.width - radius * 2.0,
            rect.height - radius * 2.0,
            color,
        );
        // Top side strip
        self.quad(x0 + radius, y0, rect.width - radius * 2.0, radius, color);
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

    /// Add a rectangle outline of the given `thickness`, drawn flush *inside*
    /// `rect` (the outer edge of the border coincides with `rect`, the border
    /// grows inward). Mirrors Teardown's `UiRectOutline(w, h, thickness)`.
    ///
    /// Built from four edge quads so it transforms (rotation/scale/clip/tint)
    /// exactly like [`DrawList::quad`].
    pub fn rect_outline(&mut self, rect: Rect, thickness: f32, color: [f32; 4]) {
        if thickness <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        // Fast path: one outline-only SDF instance (radius 0, transparent fill)
        // instead of four edge quads. Falls back to soup under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.stroke_rect_instance(rect, 0.0, thickness, color);
            return;
        }
        // Clamp so an over-thick border degenerates to a filled rect instead of
        // overlapping itself / inverting the inner strips.
        let t = thickness.min(rect.width * 0.5).min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;

        // Top and bottom run the full width.
        self.quad(x0, y0, rect.width, t, color);
        self.quad(x0, rect.y + rect.height - t, rect.width, t, color);

        // Left and right fill only the gap between the top/bottom strips so the
        // corners are not double-covered.
        let inner_h = rect.height - 2.0 * t;
        if inner_h > 0.0 {
            self.quad(x0, y0 + t, t, inner_h, color);
            self.quad(x1 - t, y0 + t, t, inner_h, color);
        }
    }

    /// Add a rounded-rectangle outline of the given `thickness`, tracing the
    /// same boundary as [`DrawList::rounded_rect`] (outer edge flush with
    /// `rect`, border grows inward). Mirrors Teardown's
    /// `UiRoundedRectOutline(w, h, radius, thickness)`.
    pub fn rounded_rect_outline(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        if thickness <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        if radius <= 0.0 {
            self.rect_outline(rect, thickness, color);
            return;
        }

        // Fast path: one outline-only SDF instance (rounded, transparent fill)
        // instead of two edge quads + four corner arcs. The shader clamps radius
        // and thickness. Falls back to soup tessellation under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.stroke_rect_instance(rect, radius, thickness, color);
            return;
        }

        let radius = radius.min(rect.width * 0.5).min(rect.height * 0.5);
        let t = thickness
            .min(radius)
            .min(rect.width * 0.5)
            .min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;

        // Straight edges between the corner tangent points, inset inward by `t`.
        let span_w = rect.width - radius * 2.0;
        let span_h = rect.height - radius * 2.0;
        if span_w > 0.0 {
            self.quad(x0 + radius, y0, span_w, t, color); // top
            self.quad(x0 + radius, y1 - t, span_w, t, color); // bottom
        }
        if span_h > 0.0 {
            self.quad(x0, y0 + radius, t, span_h, color); // left
            self.quad(x1 - t, y0 + radius, t, span_h, color); // right
        }

        // Corner arcs (outer radius = `radius`, inner = radius - t) using the
        // same angular ranges as `rounded_rect` so the stroke follows the fill.
        let inner = (radius - t).max(0.0);
        let seg = ROUNDED_RECT_CORNER_SEGMENTS;
        self.stroked_arc(
            (x0 + radius, y0 + radius),
            inner,
            radius,
            std::f32::consts::PI,
            std::f32::consts::PI * 1.5,
            seg,
            color,
        );
        self.stroked_arc(
            (x1 - radius, y0 + radius),
            inner,
            radius,
            std::f32::consts::PI * 1.5,
            std::f32::consts::TAU,
            seg,
            color,
        );
        self.stroked_arc(
            (x1 - radius, y1 - radius),
            inner,
            radius,
            0.0,
            std::f32::consts::FRAC_PI_2,
            seg,
            color,
        );
        self.stroked_arc(
            (x0 + radius, y1 - radius),
            inner,
            radius,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            seg,
            color,
        );
    }

    /// Draw a rounded-rect "chrome" panel (button background + border) via the
    /// **instanced SDF pipeline** when possible, rather than tessellating ~80
    /// vertices into the soup every frame.
    ///
    /// Fast path (active transform is translation-only): records a single
    /// [`ChromeInstance`] — world rect, tinted `bg`/`border`, current clip,
    /// `radius`/`thickness` — that the renderer rasterizes from a signed
    /// distance field. Thousands of same-shape buttons collapse to one base
    /// mesh + N instances, with anti-aliased corners for free.
    ///
    /// Fallback (any rotation/scale/shear in the transform): defers to the
    /// immediate [`DrawList::rounded_rect`] + [`DrawList::rounded_rect_outline`]
    /// so a transformed chrome rect still renders correctly. The SDF instance
    /// carries only an axis-aligned world rect, so non-translation transforms
    /// can't be expressed as a single instance.
    ///
    /// Ordering with surrounding soup geometry is preserved: each call flushes
    /// any pending soup into a command before recording its instance.
    pub fn chrome_rect(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        bg: [f32; 4],
        border: [f32; 4],
    ) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }

        let m = self.current_transform();
        if !m.is_translate_only() {
            // Fallback: build it out of the immediate primitives, which already
            // run every vertex through the active transform.
            if radius > 0.0 {
                self.rounded_rect(rect, radius, bg);
            } else {
                self.quad(rect.x, rect.y, rect.width, rect.height, bg);
            }
            if thickness > 0.0 {
                self.rounded_rect_outline(rect, radius, thickness, border);
            }
            return;
        }

        // Fast path: one instance carrying both fill and border.
        self.push_chrome_instance(rect, radius, thickness, bg, border);
    }

    /// Record one SDF chrome instance (fill + border) for a translation-only
    /// rect, preserving draw order: any soup appended since the last command is
    /// flushed into a `Soup` command first, then this instance extends (or
    /// starts) the trailing `Chrome` run. The caller must already have checked
    /// `is_translate_only()`.
    fn push_chrome_instance(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        bg: [f32; 4],
        border: [f32; 4],
    ) {
        self.flush_soup();

        let m = self.current_transform();
        let (clip, clip_enabled) = match self.current_clip() {
            Some(c) => ([c.x, c.y, c.width, c.height], 1.0),
            None => ([0.0; 4], 0.0),
        };
        let inst = ChromeInstance {
            rect: [rect.x + m.tx, rect.y + m.ty, rect.width, rect.height],
            bg: self.apply_tint(bg),
            border: self.apply_tint(border),
            clip,
            params: [radius, thickness, clip_enabled, 0.0],
        };
        let idx = self.chrome_instances.len() as u32;
        self.chrome_instances.push(inst);

        match self.color_cmds.last_mut() {
            Some(ColorCmd::Chrome { instances }) if instances.end == idx => {
                instances.end = idx + 1;
            }
            _ => self.color_cmds.push(ColorCmd::Chrome {
                instances: idx..idx + 1,
            }),
        }
    }

    /// Record a fill-only SDF rect instance (border == fill so the anti-aliased
    /// edge stays the fill color, no border ring). Backs the translation-only
    /// fast path of [`DrawList::quad`] / [`DrawList::rounded_rect`].
    fn fill_rect_instance(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
        self.push_chrome_instance(rect, radius, 0.0, color, color);
    }

    /// Record an outline-only SDF rect instance (transparent fill so only the
    /// border band renders). Backs the translation-only fast path of
    /// [`DrawList::rect_outline`] / [`DrawList::rounded_rect_outline`].
    fn stroke_rect_instance(&mut self, rect: Rect, radius: f32, thickness: f32, color: [f32; 4]) {
        let transparent = [color[0], color[1], color[2], 0.0];
        self.push_chrome_instance(rect, radius, thickness, transparent, color);
    }

    /// Record one SDF circle instance for a translation-only circle. Same
    /// ordering contract as [`DrawList::push_chrome_instance`] but into the
    /// circle instance buffer / `Circle` runs. `thickness <= 0` is a filled
    /// disc; `> 0` is a ring centered on `radius`. The caller must already have
    /// checked `is_translate_only()`.
    fn push_circle_instance(
        &mut self,
        center: (f32, f32),
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        self.flush_soup();

        let m = self.current_transform();
        let (clip, clip_enabled) = match self.current_clip() {
            Some(c) => ([c.x, c.y, c.width, c.height], 1.0),
            None => ([0.0; 4], 0.0),
        };
        let inst = CircleInstance {
            center: [center.0 + m.tx, center.1 + m.ty, radius, thickness],
            color: self.apply_tint(color),
            clip,
            params: [clip_enabled, 0.0, 0.0, 0.0],
        };
        let idx = self.circle_instances.len() as u32;
        self.circle_instances.push(inst);

        match self.color_cmds.last_mut() {
            Some(ColorCmd::Circle { instances }) if instances.end == idx => {
                instances.end = idx + 1;
            }
            _ => self.color_cmds.push(ColorCmd::Circle {
                instances: idx..idx + 1,
            }),
        }
    }

    /// Commit soup geometry appended since the last command into a `Soup`
    /// command, so a following chrome instance draws after it. No-op if nothing
    /// new was appended.
    fn flush_soup(&mut self) {
        let total = self.indices.len() as u32;
        if total > self.soup_committed_indices {
            self.color_cmds.push(ColorCmd::Soup {
                indices: self.soup_committed_indices..total,
            });
            self.soup_committed_indices = total;
        }
    }

    /// Emit a thick arc band between `inner` and `outer` radius from
    /// `start_angle` to `end_angle` as a strip of `segments` quads (two
    /// triangles each).
    fn stroked_arc(
        &mut self,
        center: (f32, f32),
        inner: f32,
        outer: f32,
        start_angle: f32,
        end_angle: f32,
        segments: usize,
        color: [f32; 4],
    ) {
        for i in 0..segments {
            let t0 = i as f32 / segments as f32;
            let t1 = (i + 1) as f32 / segments as f32;
            let a0 = start_angle + (end_angle - start_angle) * t0;
            let a1 = start_angle + (end_angle - start_angle) * t1;
            let (c0, s0) = (a0.cos(), a0.sin());
            let (c1, s1) = (a1.cos(), a1.sin());
            let i0 = (center.0 + c0 * inner, center.1 + s0 * inner);
            let o0 = (center.0 + c0 * outer, center.1 + s0 * outer);
            let i1 = (center.0 + c1 * inner, center.1 + s1 * inner);
            let o1 = (center.0 + c1 * outer, center.1 + s1 * outer);
            self.triangle(i0, o0, o1, color);
            self.triangle(i0, o1, i1, color);
        }
    }

    /// Number of segments to approximate a circle of the given radius — enough
    /// for the curve to read as smooth without exploding vertex counts.
    fn circle_segments(radius: f32) -> usize {
        ((radius * 0.5).ceil() as usize).clamp(16, 64)
    }

    /// Add a filled circle, centered at `center`. Mirrors Teardown's
    /// `UiCircle(radius)`. Built as a triangle fan so it transforms like the
    /// other primitives.
    pub fn circle(&mut self, center: (f32, f32), radius: f32, color: [f32; 4]) {
        if radius <= 0.0 {
            return;
        }
        // Fast path: one SDF disc instance (smooth at any radius) instead of a
        // 16-64-segment fan. Falls back to the fan under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.push_circle_instance(center, radius, 0.0, color);
            return;
        }
        let segs = Self::circle_segments(radius);
        for i in 0..segs {
            let a0 = std::f32::consts::TAU * i as f32 / segs as f32;
            let a1 = std::f32::consts::TAU * (i + 1) as f32 / segs as f32;
            let p0 = (center.0 + a0.cos() * radius, center.1 + a0.sin() * radius);
            let p1 = (center.0 + a1.cos() * radius, center.1 + a1.sin() * radius);
            self.triangle(center, p0, p1, color);
        }
    }

    /// Add a circle outline of the given `thickness`, centered on the path at
    /// `radius` (the band spans `radius ± thickness/2`). Mirrors Teardown's
    /// `UiCircleOutline(radius, thickness)`.
    pub fn circle_outline(
        &mut self,
        center: (f32, f32),
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        if radius <= 0.0 || thickness <= 0.0 {
            return;
        }
        // Fast path: one SDF ring instance instead of a stroked-arc band.
        // Falls back to the band tessellation under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.push_circle_instance(center, radius, thickness, color);
            return;
        }
        let half = thickness * 0.5;
        let inner = (radius - half).max(0.0);
        let outer = radius + half;
        let segs = Self::circle_segments(outer);
        self.stroked_arc(
            center,
            inner,
            outer,
            0.0,
            std::f32::consts::TAU,
            segs,
            color,
        );
    }

    /// Add text. The block's origin is transformed through the current
    /// affine; uniform scale (the geometric mean of the X and Y axis basis
    /// lengths, i.e. `sqrt(|det|)`) is applied to font_size, line_height and
    /// max_width. Under non-uniform scale this picks the "average" zoom so a
    /// 2x-by-1x stretch becomes ~1.41x text rather than picking only one axis.
    /// Rotation/shear is **not supported** by the text pipeline (glyphs are
    /// emitted as axis-aligned MSDF quads) — when the transform has any
    /// rotation we log a one-shot warning and render axis-aligned.
    pub fn text(&mut self, mut block: TextBlock) {
        let m = self.current_transform();
        if !m.is_axis_aligned() && !self.text_rotation_warned {
            log::warn!(
                "wgpu-gameui: TextBlock pushed under a rotated/sheared transform — \
                 text will render axis-aligned (MSDF text pipeline limitation)"
            );
            self.text_rotation_warned = true;
        }

        // When span mode is active, derive the display content from the
        // concatenated span texts so the shape cache and cursor-position calls
        // below all operate on the same string.
        if !block.spans.is_empty() {
            block.content = block.spans.iter().map(|s| s.text.as_str()).collect();
        }

        // Apply tint to the block colour and to per-span colour/underline
        // overrides. Tint is colour-only, so we do it before the position
        // transform (order w.r.t. position doesn't matter here, but doing it
        // early lets the underline quads below use the already-tinted colour).
        let tint = self.current_tint();
        if tint != [1.0, 1.0, 1.0, 1.0] {
            // glyphon::Color is RGBA8; multiply per-channel via the public accessors.
            let r = block.color.r() as f32 / 255.0;
            let g = block.color.g() as f32 / 255.0;
            let b = block.color.b() as f32 / 255.0;
            let a = block.color.a() as f32 / 255.0;
            let nr = (r * tint[0]).clamp(0.0, 1.0);
            let ng = (g * tint[1]).clamp(0.0, 1.0);
            let nb = (b * tint[2]).clamp(0.0, 1.0);
            let na = (a * tint[3]).clamp(0.0, 1.0);
            block.color = glyphon::Color::rgba(
                (nr * 255.0).round() as u8,
                (ng * 255.0).round() as u8,
                (nb * 255.0).round() as u8,
                (na * 255.0).round() as u8,
            );
            // Tint per-span colour and underline overrides with the same factor.
            for span in &mut block.spans {
                if let Some(c) = &mut span.color {
                    c[0] = (c[0] * tint[0]).clamp(0.0, 1.0);
                    c[1] = (c[1] * tint[1]).clamp(0.0, 1.0);
                    c[2] = (c[2] * tint[2]).clamp(0.0, 1.0);
                    c[3] = (c[3] * tint[3]).clamp(0.0, 1.0);
                }
                // Only an explicit underline colour needs tinting here; an
                // inheriting underline reads the already-tinted span/block colour
                // at emission time.
                if let Underline::Color(c) = &mut span.underline {
                    c[0] = (c[0] * tint[0]).clamp(0.0, 1.0);
                    c[1] = (c[1] * tint[1]).clamp(0.0, 1.0);
                    c[2] = (c[2] * tint[2]).clamp(0.0, 1.0);
                    c[3] = (c[3] * tint[3]).clamp(0.0, 1.0);
                }
            }
        }

        // Emit underline rects for spans that have `underline` set, BEFORE
        // transforming block.x/block.y. We use the original (pre-transform,
        // pre-scale) font_size and position, so that `self.quad()` can apply
        // the active transform uniformly — matching exactly what the text
        // pipeline does. Soup geometry draws before text glyphs, so the
        // underlines naturally appear beneath the MSDF rendering.
        if block
            .spans
            .iter()
            .any(|s| !matches!(s.underline, Underline::None))
        {
            let positions =
                self.text_cursor_positions(&block.content, block.font_size, Some(block.max_width));
            // Sit the underline just below the baseline so it clears the letter
            // bottoms. `baseline_ratio` (~1.0 of the em) locates the baseline
            // below the block top; the old flat `0.9` sat *above* it, cutting
            // through the glyph bottoms. The small extra gap drops it into the
            // descender zone, font-metric-relative so it scales with any face.
            let vm = self.font_vmetrics(block.font.as_ref());
            let underline_y = block.y + block.font_size * (vm.baseline_ratio + 0.12);
            let thickness = (block.font_size * 0.07).max(1.0);
            // The block colour (already tinted above), as the fallback for an
            // inheriting underline on a span with no colour of its own.
            let block_rgba = [
                block.color.r() as f32 / 255.0,
                block.color.g() as f32 / 255.0,
                block.color.b() as f32 / 255.0,
                block.color.a() as f32 / 255.0,
            ];
            let mut span_byte = 0usize;
            for span in &block.spans {
                // Inherit → the span's text colour (or the block colour); Color →
                // the explicit (tinted) override; None → no underline.
                let ul_color = match span.underline {
                    Underline::None => None,
                    Underline::Inherit => Some(span.color.unwrap_or(block_rgba)),
                    Underline::Color(c) => Some(c),
                };
                if let Some(ul_color) = ul_color {
                    let x_start = span_cursor_x(&positions, span_byte);
                    let end_byte = span_byte + span.text.len();
                    let x_end = span_cursor_x(&positions, end_byte);
                    if x_end > x_start {
                        self.quad(
                            block.x + x_start,
                            underline_y,
                            x_end - x_start,
                            thickness,
                            ul_color,
                        );
                    }
                }
                span_byte += span.text.len();
            }
        }

        // Transform origin.
        let origin = m.transform_point([block.x, block.y]);
        block.x = origin[0];
        block.y = origin[1];

        // Apply uniform-ish scale: geometric mean of the two basis lengths,
        // which equals sqrt(|det|). This handles non-uniform axis-aligned
        // scale gracefully (picks the average zoom instead of dropping a
        // dimension).
        let scale = m.uniform_scale();
        if scale > 0.0 && (scale - 1.0).abs() > 1e-6 {
            block.font_size *= scale;
            block.line_height *= scale;
            block.max_width *= scale;
        }

        if let Some(clip) = self.current_clip() {
            let natural_bounds = Rect::new(block.x, block.y, block.max_width, 2000.0);
            let text_bounds = block.clip.unwrap_or(natural_bounds);
            block.clip = text_bounds
                .intersection(clip)
                .or_else(|| Some(Rect::new(clip.x, clip.y, 0.0, 0.0)));
        }
        self.texts.push(block);
    }

    /// Add a vector [`PhosphorIcon`], fit-centered into `rect` and rendered crisp
    /// at any size through the MSDF icon atlas. `tint` multiplies the fill (use
    /// `[1.0; 4]` for the icon's natural color). Honors the current transform,
    /// tint stack, and clip. No-op for a zero-area rect or an unresolvable glyph.
    #[cfg(feature = "phosphor-icons")]
    pub fn icon_msdf(&mut self, rect: Rect, icon: PhosphorIcon, tint: [f32; 4]) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let Some(glyph_id) = phosphor_glyph_id(icon) else {
            return;
        };
        self.icons_msdf.push(IconMsdf {
            local: rect,
            transform: self.current_transform(),
            glyph_id,
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
        });
    }

    /// Add a textured icon by name. The renderer will resolve `icon_key` against
    /// its `SpriteAtlas` at render time.
    pub fn icon(&mut self, icon_key: &str, x: f32, y: f32, width: f32, height: f32) {
        let corners = self
            .current_transform()
            .transform_rect_corners(Rect::new(x, y, width, height));
        self.icons.push(IconDraw {
            corners,
            sprite: None,
            icon_key: icon_key.to_string(),
            tint: self.current_tint(),
            clip: self.current_clip(),
            src: None,
        });
    }

    /// Add a textured icon by pre-resolved sprite handle, with optional tint.
    /// Cheaper than [`DrawList::icon`] — no per-frame name lookup.
    pub fn icon_sprite(
        &mut self,
        sprite: SpriteId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        tint: [f32; 4],
    ) {
        let corners = self
            .current_transform()
            .transform_rect_corners(Rect::new(x, y, width, height));
        self.icons.push(IconDraw {
            corners,
            sprite: Some(sprite),
            icon_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
            src: None,
        });
    }

    /// Draw a loaded image sprite filling `dest`, tinted by `tint`. Equivalent
    /// to [`DrawList::icon_sprite`] with a `Rect` destination. Backs Teardown's
    /// `UiImage(path)` / `UiFillImage`.
    pub fn image(&mut self, sprite: SpriteId, dest: Rect, tint: [f32; 4]) {
        self.push_image(sprite, dest, None, tint);
    }

    /// Draw a cropped region of a loaded image sprite into `dest`. `src_uv` is a
    /// normalized `[u0, v0, u1, v1]` sub-rect (0..1) within the sprite. Backs
    /// Teardown's `UiImage(path, x0, y0, x1, y1)`.
    pub fn image_cropped(
        &mut self,
        sprite: SpriteId,
        dest: Rect,
        src_uv: [f32; 4],
        tint: [f32; 4],
    ) {
        self.push_image(sprite, dest, Some(src_uv), tint);
    }

    fn push_image(&mut self, sprite: SpriteId, dest: Rect, src: Option<[f32; 4]>, tint: [f32; 4]) {
        let corners = self.current_transform().transform_rect_corners(dest);
        self.icons.push(IconDraw {
            corners,
            sprite: Some(sprite),
            icon_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
            src,
        });
    }

    /// Add a nine-slice textured panel by name.
    pub fn nine_slice(&mut self, x: f32, y: f32, width: f32, height: f32, texture_key: &str) {
        self.nine_slices.push(NineSliceDraw {
            local: Rect::new(x, y, width, height),
            transform: self.current_transform(),
            nine_slice: None,
            texture_key: texture_key.to_string(),
            tint: self.current_tint(),
            clip: self.current_clip(),
        });
    }

    /// Add a nine-slice panel by pre-resolved handle.
    pub fn nine_slice_id(
        &mut self,
        id: NineSliceId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        tint: [f32; 4],
    ) {
        self.nine_slices.push(NineSliceDraw {
            local: Rect::new(x, y, width, height),
            transform: self.current_transform(),
            nine_slice: Some(id),
            texture_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
        });
    }
}

/// Return the x-pixel offset of the cursor at `byte_pos` in the positions
/// table returned by [`DrawList::text_cursor_positions`]. Falls back to `0.0`
/// if the byte position is not present (shouldn't happen for well-formed span
/// data, but the function is cheap enough to not warrant a panic).
fn span_cursor_x(positions: &[(usize, f32)], byte_pos: usize) -> f32 {
    positions
        .iter()
        .find(|(b, _)| *b == byte_pos)
        .map(|(_, x)| *x)
        .unwrap_or(0.0)
}

/// Component-wise linear interpolation between two RGBA colors at `t ∈ [0,1]`.
fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

#[cfg(test)]
mod tests {
    use crate::affine::Affine2;
    use crate::layout::Rect;

    use super::DrawList;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn rounded_rect_emits_chrome_instance() {
        // Translate-only rounded fills record one SDF chrome instance (radius in
        // params[0], thickness 0 = fill), not tessellated soup.
        let mut list = DrawList::new();
        list.rounded_rect(Rect::new(0.0, 0.0, 100.0, 40.0), 6.0, [1.0, 1.0, 1.0, 1.0]);

        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instances.len(), 1);
        let inst = list.chrome_instances[0];
        assert_eq!(inst.rect, [0.0, 0.0, 100.0, 40.0]);
        assert!(approx(inst.params[0], 6.0)); // radius
        assert!(approx(inst.params[1], 0.0)); // thickness (fill)
    }

    #[test]
    fn rect_outline_emits_stroke_instance() {
        let mut list = DrawList::new();
        list.rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 2.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instances.len(), 1);
        let inst = list.chrome_instances[0];
        assert!(approx(inst.params[0], 0.0)); // radius (square corners)
        assert!(approx(inst.params[1], 2.0)); // thickness
        assert!(approx(inst.bg[3], 0.0)); // transparent fill: only the border band draws
    }

    #[test]
    fn rect_outline_degenerates_to_two_quads_when_thick() {
        // Under rotation the outline falls back to the soup tessellator, where a
        // thickness >= half height collapses the inner strip to top+bottom only.
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        list.rect_outline(Rect::new(0.0, 0.0, 100.0, 10.0), 50.0, [1.0; 4]);
        assert!(list.chrome_instances.is_empty());
        assert_eq!(list.vertices.len(), 8); // two quads
    }

    #[test]
    fn rect_outline_zero_thickness_draws_nothing() {
        let mut list = DrawList::new();
        list.rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 0.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert!(list.chrome_instances.is_empty());
    }

    #[test]
    fn rounded_rect_outline_emits_stroke_instance() {
        let mut list = DrawList::new();
        list.rounded_rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 8.0, 2.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instances.len(), 1);
        let inst = list.chrome_instances[0];
        assert!(approx(inst.params[0], 8.0)); // radius
        assert!(approx(inst.params[1], 2.0)); // thickness
        assert!(approx(inst.bg[3], 0.0)); // transparent fill
    }

    #[test]
    fn rounded_rect_outline_zero_radius_falls_back_to_rect_outline() {
        let mut rounded = DrawList::new();
        rounded.rounded_rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 0.0, 2.0, [1.0; 4]);
        let mut plain = DrawList::new();
        plain.rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 2.0, [1.0; 4]);
        // Both produce an identical square-cornered stroke instance.
        assert_eq!(rounded.chrome_instances.len(), 1);
        assert_eq!(rounded.chrome_instances, plain.chrome_instances);
    }

    #[test]
    fn circle_emits_instance() {
        // Translate-only circle fills record one SDF circle instance.
        let mut list = DrawList::new();
        list.circle((50.0, 50.0), 20.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.circle_instances.len(), 1);
        let inst = list.circle_instances[0];
        assert_eq!(inst.center, [50.0, 50.0, 20.0, 0.0]); // cx, cy, radius, thickness(fill)
    }

    #[test]
    fn circle_emits_fan_within_radius_under_rotation() {
        // Under rotation the circle falls back to the soup fan. Centered at the
        // origin (rotation's fixed point) so the within-radius check still holds.
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        let r = 20.0;
        list.circle((0.0, 0.0), r, [1.0; 4]);
        assert!(list.circle_instances.is_empty());
        assert!(!list.vertices.is_empty());
        for v in &list.vertices {
            let d = (v.position[0] * v.position[0] + v.position[1] * v.position[1]).sqrt();
            assert!(d <= r + 1e-3);
        }
    }

    #[test]
    fn circle_outline_band_spans_radius_under_rotation() {
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        let (r, t) = (20.0, 4.0);
        list.circle_outline((0.0, 0.0), r, t, [1.0; 4]);
        // Vertices sit on the inner or outer ring: distance in [r - t/2, r + t/2].
        assert!(list.circle_instances.is_empty());
        let lo = r - t * 0.5 - 1e-3;
        let hi = r + t * 0.5 + 1e-3;
        for v in &list.vertices {
            let d = (v.position[0] * v.position[0] + v.position[1] * v.position[1]).sqrt();
            assert!(
                d >= lo && d <= hi,
                "vertex dist {d} outside band [{lo},{hi}]"
            );
        }
    }

    #[test]
    fn circle_outline_emits_instance() {
        let mut list = DrawList::new();
        list.circle_outline((0.0, 0.0), 20.0, 4.0, [1.0; 4]);
        assert_eq!(list.circle_instances.len(), 1);
        let inst = list.circle_instances[0];
        assert_eq!(inst.center, [0.0, 0.0, 20.0, 4.0]); // thickness carries the band width
    }

    #[test]
    fn circle_zero_radius_draws_nothing() {
        let mut list = DrawList::new();
        list.circle((0.0, 0.0), 0.0, [1.0; 4]);
        list.circle_outline((0.0, 0.0), 0.0, 2.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert!(list.circle_instances.is_empty());
    }

    #[test]
    fn outline_primitives_respect_transform() {
        // Translate-only outlines bake the translated rect into the instance.
        let mut list = DrawList::new();
        list.translate(100.0, 50.0);
        list.rect_outline(Rect::new(0.0, 0.0, 10.0, 10.0), 2.0, [1.0; 4]);
        assert_eq!(list.chrome_instances[0].rect, [100.0, 50.0, 10.0, 10.0]);
    }

    #[test]
    fn line_emits_quad_geometry() {
        let mut list = DrawList::new();
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0, 1.0, 1.0, 1.0]);

        assert_eq!(list.vertices.len(), 4);
        assert_eq!(list.indices.len(), 6);
    }

    #[test]
    fn icon_helper_pushes_one_command() {
        let mut list = DrawList::new();
        list.icon("foo", 1.0, 2.0, 16.0, 16.0);
        assert_eq!(list.icons.len(), 1);
        assert_eq!(list.icons[0].icon_key, "foo");
        assert_eq!(list.icons[0].sprite, None);
        assert_eq!(list.icons[0].tint, [1.0, 1.0, 1.0, 1.0]);
        // TL and BR corners under identity match input rect.
        assert_eq!(list.icons[0].corners[0], [1.0, 2.0]);
        assert_eq!(list.icons[0].corners[2], [17.0, 18.0]);
    }

    #[test]
    fn icon_sprite_helper_resolves_id_and_tint() {
        let mut list = DrawList::new();
        list.icon_sprite(7, 0.0, 0.0, 24.0, 24.0, [0.5, 0.6, 0.7, 1.0]);
        assert_eq!(list.icons.len(), 1);
        assert_eq!(list.icons[0].sprite, Some(7));
        assert_eq!(list.icons[0].tint, [0.5, 0.6, 0.7, 1.0]);
        assert!(list.icons[0].icon_key.is_empty());
        assert_eq!(list.icons[0].src, None);
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn icon_msdf_records_glyph_tint_transform_and_clip() {
        use crate::render::PhosphorIcon;
        let mut list = DrawList::new();
        list.push_transform();
        list.translate(40.0, 60.0);
        list.set_tint([1.0, 1.0, 1.0, 0.5]);
        list.icon_msdf(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            PhosphorIcon::Plus,
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(list.icons_msdf.len(), 1);
        let rec = list.icons_msdf[0];
        // Glyph resolved to a real (non-notdef) id.
        assert_ne!(rec.glyph_id, 0);
        // Tint is multiplied by the active tint stack (alpha 1.0 * 0.5).
        assert_eq!(rec.tint, [1.0, 0.0, 0.0, 0.5]);
        // The translate transform is carried (origin maps to (40, 60)).
        let o = rec.transform.transform_point([0.0, 0.0]);
        assert!((o[0] - 40.0).abs() < 1e-4 && (o[1] - 60.0).abs() < 1e-4);
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn icon_msdf_skips_zero_rect() {
        use crate::render::PhosphorIcon;
        let mut list = DrawList::new();
        list.icon_msdf(Rect::new(0.0, 0.0, 0.0, 20.0), PhosphorIcon::X, [1.0; 4]);
        assert!(list.icons_msdf.is_empty());
    }

    #[test]
    fn image_and_image_cropped_set_src() {
        let mut list = DrawList::new();
        list.image(3, Rect::new(0.0, 0.0, 32.0, 32.0), [1.0, 1.0, 1.0, 1.0]);
        list.image_cropped(
            4,
            Rect::new(0.0, 0.0, 16.0, 16.0),
            [0.0, 0.0, 0.5, 0.5],
            [1.0, 1.0, 1.0, 1.0],
        );
        assert_eq!(list.icons.len(), 2);
        assert_eq!(list.icons[0].sprite, Some(3));
        assert_eq!(list.icons[0].src, None);
        assert_eq!(list.icons[1].sprite, Some(4));
        assert_eq!(list.icons[1].src, Some([0.0, 0.0, 0.5, 0.5]));
    }

    #[test]
    fn clip_stack_marks_emitted_commands() {
        let mut list = DrawList::new();
        let clip = Rect::new(10.0, 20.0, 30.0, 40.0);

        list.push_clip(clip);
        // Translate-only quads record chrome instances carrying the active clip.
        list.quad(0.0, 0.0, 100.0, 100.0, [1.0, 1.0, 1.0, 1.0]);
        list.text(crate::text::TextBlock::new("clipped", 0.0, 0.0));
        list.icon("icon", 0.0, 0.0, 10.0, 10.0);
        list.pop_clip();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0, 1.0, 1.0, 1.0]);

        assert_eq!(list.chrome_instances[0].params[2], 1.0); // clip_enabled
        assert_eq!(list.chrome_instances[0].clip, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(list.texts[0].clip, Some(Rect::new(10.0, 20.0, 30.0, 40.0)));
        assert_eq!(list.icons[0].clip, Some(clip));
        assert_eq!(list.icons[0].icon_key, "icon");
        assert_eq!(list.icons[0].tint, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(list.chrome_instances[1].params[2], 0.0); // clip disabled
    }

    #[test]
    fn push_clip_intersects_but_exact_replaces() {
        let mut list = DrawList::new();
        // Parent clip.
        list.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
        // Intersecting child: a larger rect is clipped down to the parent.
        list.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(list.current_clip(), Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
        list.pop_clip();
        // Exact child: replaces the parent, even when larger.
        list.push_clip_exact(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(list.current_clip(), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn clip_len_and_truncate_scope() {
        let mut list = DrawList::new();
        assert_eq!(list.clip_len(), 0);
        let base = list.clip_len();
        list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        list.push_clip_exact(Rect::new(0.0, 0.0, 5.0, 5.0));
        assert_eq!(list.clip_len(), 2);
        list.truncate_clip(base);
        assert_eq!(list.clip_len(), 0);
        assert_eq!(list.current_clip(), None);
    }

    #[test]
    fn underline_inherit_uses_span_then_block_colour() {
        use crate::text::{TextBlock, TextSpan, Underline};

        // Three underlined spans: Inherit-with-span-colour, Inherit-without
        // (falls back to block colour), and an explicit Colour override. Each
        // underline lands as a thin translate-only quad in chrome_instances,
        // carrying its resolved colour in `bg`.
        let span_red = [1.0, 0.0, 0.0, 1.0];
        let override_yellow = [1.0, 0.9, 0.0, 1.0];
        let mut list = DrawList::with_font_system(crate::shared_font_system());
        list.text(
            TextBlock::new("", 18.0, 10.0)
                .with_size(24.0)
                .with_color(0, 200, 0) // block green
                .with_spans(vec![
                    TextSpan {
                        text: "red ".into(),
                        color: Some(span_red),
                        underline: Underline::Inherit,
                    },
                    TextSpan {
                        text: "green ".into(),
                        color: None,
                        underline: Underline::Inherit,
                    },
                    TextSpan {
                        text: "yellow".into(),
                        color: None,
                        underline: Underline::Color(override_yellow),
                    },
                ]),
        );

        let block_green = [0.0, 200.0 / 255.0, 0.0, 1.0];
        let has_colour = |want: [f32; 4]| {
            list.chrome_instances.iter().any(|c| {
                c.bg.iter()
                    .zip(want.iter())
                    .all(|(a, b)| (a - b).abs() < 1e-3)
            })
        };
        assert!(has_colour(span_red), "inherit underline should use span colour");
        assert!(
            has_colour(block_green),
            "inherit underline w/o span colour should fall back to block colour"
        );
        assert!(
            has_colour(override_yellow),
            "explicit Colour underline should use the override"
        );
    }

    // ---- Transform/tint stack tests ----

    #[test]
    fn quad_under_translate() {
        // Translate-only quads bake the translated rect into a chrome instance.
        let mut list = DrawList::new();
        list.translate(100.0, 50.0);
        list.quad(0.0, 0.0, 10.0, 20.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instances[0].rect, [100.0, 50.0, 10.0, 20.0]);
    }

    #[test]
    fn quad_gradient_assigns_corner_colors() {
        let mut list = DrawList::new();
        let tl = [1.0, 0.0, 0.0, 1.0];
        let tr = [0.0, 1.0, 0.0, 1.0];
        let br = [0.0, 0.0, 1.0, 1.0];
        let bl = [1.0, 1.0, 0.0, 1.0];
        list.quad_gradient(Rect::new(0.0, 0.0, 10.0, 20.0), [tl, tr, br, bl]);
        // Four soup vertices in TL, TR, BR, BL order, each carrying its color.
        assert_eq!(list.vertices.len(), 4);
        assert_eq!(list.vertices[0].position, [0.0, 0.0]);
        assert_eq!(list.vertices[0].color, tl);
        assert_eq!(list.vertices[1].position, [10.0, 0.0]);
        assert_eq!(list.vertices[1].color, tr);
        assert_eq!(list.vertices[2].position, [10.0, 20.0]);
        assert_eq!(list.vertices[2].color, br);
        assert_eq!(list.vertices[3].position, [0.0, 20.0]);
        assert_eq!(list.vertices[3].color, bl);
        // Two triangles → 6 indices.
        assert_eq!(list.indices, vec![0, 1, 2, 2, 3, 0]);
    }

    #[test]
    fn quad_gradient_zero_size_is_noop() {
        let mut list = DrawList::new();
        list.quad_gradient(Rect::new(0.0, 0.0, 0.0, 20.0), [[1.0; 4]; 4]);
        list.quad_gradient(Rect::new(0.0, 0.0, 20.0, 0.0), [[1.0; 4]; 4]);
        assert!(list.vertices.is_empty());
        assert!(list.indices.is_empty());
    }

    #[test]
    fn horizontal_gradient_sets_left_right_corners() {
        let mut list = DrawList::new();
        let left = [1.0, 0.0, 0.0, 1.0];
        let right = [0.0, 0.0, 1.0, 1.0];
        list.horizontal_gradient(Rect::new(0.0, 0.0, 10.0, 20.0), left, right);
        // TL=left, TR=right, BR=right, BL=left.
        assert_eq!(list.vertices[0].color, left);
        assert_eq!(list.vertices[1].color, right);
        assert_eq!(list.vertices[2].color, right);
        assert_eq!(list.vertices[3].color, left);
    }

    #[test]
    fn vertical_gradient_sets_top_bottom_corners() {
        let mut list = DrawList::new();
        let top = [1.0, 0.0, 0.0, 1.0];
        let bottom = [0.0, 0.0, 1.0, 1.0];
        list.vertical_gradient(Rect::new(0.0, 0.0, 10.0, 20.0), top, bottom);
        // TL=top, TR=top, BR=bottom, BL=bottom.
        assert_eq!(list.vertices[0].color, top);
        assert_eq!(list.vertices[1].color, top);
        assert_eq!(list.vertices[2].color, bottom);
        assert_eq!(list.vertices[3].color, bottom);
    }

    #[test]
    fn linear_gradient_angle_zero_matches_horizontal() {
        let start = [1.0, 0.0, 0.0, 1.0];
        let end = [0.0, 1.0, 0.0, 1.0];
        let rect = Rect::new(3.0, 5.0, 10.0, 20.0);

        let mut a = DrawList::new();
        a.linear_gradient(rect, start, end, 0.0);
        let mut b = DrawList::new();
        b.horizontal_gradient(rect, start, end);

        for (va, vb) in a.vertices.iter().zip(b.vertices.iter()) {
            for k in 0..4 {
                assert!(
                    (va.color[k] - vb.color[k]).abs() < 1e-5,
                    "angle 0 should equal horizontal gradient"
                );
            }
        }
    }

    #[test]
    fn linear_gradient_quarter_turn_matches_vertical() {
        let start = [1.0, 0.0, 0.0, 1.0];
        let end = [0.0, 1.0, 0.0, 1.0];
        let rect = Rect::new(3.0, 5.0, 10.0, 20.0);

        let mut a = DrawList::new();
        a.linear_gradient(rect, start, end, std::f32::consts::FRAC_PI_2);
        let mut b = DrawList::new();
        b.vertical_gradient(rect, start, end);

        for (va, vb) in a.vertices.iter().zip(b.vertices.iter()) {
            for k in 0..4 {
                assert!(
                    (va.color[k] - vb.color[k]).abs() < 1e-5,
                    "angle π/2 should equal vertical gradient"
                );
            }
        }
    }

    #[test]
    fn linear_gradient_zero_size_is_noop() {
        let mut list = DrawList::new();
        list.linear_gradient(Rect::new(0.0, 0.0, 0.0, 20.0), [1.0; 4], [0.0; 4], 0.7);
        assert!(list.vertices.is_empty());
        assert!(list.indices.is_empty());
    }

    #[test]
    fn radial_gradient_builds_fan_with_center_and_ring() {
        let mut list = DrawList::new();
        let inner = [1.0, 1.0, 1.0, 1.0];
        let outer = [0.0, 0.0, 0.0, 0.0];
        let segments = 8;
        list.radial_gradient(Rect::new(0.0, 0.0, 40.0, 40.0), inner, outer, segments);

        // 1 center vertex + `segments` ring vertices.
        assert_eq!(list.vertices.len() as u32, 1 + segments);
        // Center color = inner; ring colors = outer.
        assert_eq!(list.vertices[0].color, inner);
        for v in &list.vertices[1..] {
            assert_eq!(v.color, outer);
        }
        // One triangle per wedge → 3 * segments indices, every triangle shares
        // the center vertex (index 0).
        assert_eq!(list.indices.len() as u32, 3 * segments);
        for tri in list.indices.chunks(3) {
            assert_eq!(tri[0], 0, "each wedge fans from the center vertex");
        }
    }

    #[test]
    fn radial_gradient_clamps_segments_and_zero_size_is_noop() {
        let mut list = DrawList::new();
        // segments < 3 is clamped up to 3 (a triangle).
        list.radial_gradient(Rect::new(0.0, 0.0, 10.0, 10.0), [1.0; 4], [0.0; 4], 1);
        assert_eq!(list.vertices.len(), 1 + 3);

        let mut empty = DrawList::new();
        empty.radial_gradient(Rect::new(0.0, 0.0, 0.0, 10.0), [1.0; 4], [0.0; 4], 16);
        assert!(empty.vertices.is_empty());
        assert!(empty.indices.is_empty());
    }

    #[test]
    fn quad_under_translate_then_scale() {
        let mut list = DrawList::new();
        list.translate(10.0, 20.0);
        list.scale(2.0, 3.0);
        list.quad(0.0, 0.0, 5.0, 5.0, [1.0; 4]);
        // local (0,0) -> scale -> (0,0) -> translate -> (10,20)
        assert_eq!(list.vertices[0].position, [10.0, 20.0]);
        // local (5,5) -> scale -> (10,15) -> translate -> (20,35)
        assert_eq!(list.vertices[2].position, [20.0, 35.0]);
    }

    #[test]
    fn rounded_rect_under_rotation_is_not_axis_aligned() {
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4); // 45 degrees
        list.rounded_rect(Rect::new(10.0, 10.0, 20.0, 20.0), 4.0, [1.0; 4]);

        // After 45° rotation, no two distinct vertices should share an x or y
        // by accident (other than coincidentally). Check that at least some
        // vertices have non-zero Y *and* non-zero X — i.e. the geometry isn't
        // collapsed into an axis-aligned box.
        let mut has_offdiag = false;
        for v in &list.vertices {
            if v.position[0].abs() > 0.001 && v.position[1].abs() > 0.001 {
                // Distance from origin should match local distance from origin
                // (rotation is rigid). For the corner at local (30,30), that
                // distance is sqrt(1800) ~= 42.43.
                let d = (v.position[0] * v.position[0] + v.position[1] * v.position[1]).sqrt();
                if d > 5.0 {
                    has_offdiag = true;
                }
            }
        }
        assert!(
            has_offdiag,
            "rotated rounded rect should have off-axis vertices"
        );
    }

    #[test]
    fn color_multiplies_with_tint() {
        let mut list = DrawList::new();
        list.set_tint([0.5, 0.5, 0.5, 1.0]);
        list.quad(0.0, 0.0, 10.0, 10.0, [0.4, 0.6, 0.8, 1.0]);
        // Tint is baked into the chrome instance's bg color (fill: border == bg).
        let inst = list.chrome_instances[0];
        assert!(approx(inst.bg[0], 0.2));
        assert!(approx(inst.bg[1], 0.3));
        assert!(approx(inst.bg[2], 0.4));
        assert!(approx(inst.bg[3], 1.0));
    }

    #[test]
    fn push_pop_restores_transform() {
        let mut list = DrawList::new();
        list.translate(10.0, 20.0);
        list.push_transform();
        list.translate(5.0, 5.0);
        assert_eq!(list.current_transform(), Affine2::translation(15.0, 25.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::translation(10.0, 20.0));
    }

    #[test]
    fn push_pop_restores_tint() {
        let mut list = DrawList::new();
        list.set_tint([0.5, 0.5, 0.5, 1.0]);
        list.push_tint();
        list.multiply_tint([0.5, 0.5, 0.5, 1.0]);
        assert_eq!(list.current_tint(), [0.25, 0.25, 0.25, 1.0]);
        list.pop_tint();
        assert_eq!(list.current_tint(), [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn pop_when_at_base_does_not_underflow() {
        let mut list = DrawList::new();
        list.pop_transform();
        list.pop_transform();
        list.pop_tint();
        list.pop_tint();
        assert_eq!(list.current_transform(), Affine2::IDENTITY);
        assert_eq!(list.current_tint(), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn nested_push_pop_balances() {
        let mut list = DrawList::new();
        list.push_transform();
        list.translate(1.0, 0.0);
        list.push_transform();
        list.translate(2.0, 0.0);
        list.push_transform();
        list.translate(4.0, 0.0);
        assert_eq!(list.current_transform(), Affine2::translation(7.0, 0.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::translation(3.0, 0.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::translation(1.0, 0.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::IDENTITY);
    }

    #[test]
    fn icon_corners_transformed_under_scale() {
        let mut list = DrawList::new();
        list.scale(2.0, 2.0);
        list.icon("foo", 5.0, 5.0, 10.0, 10.0);
        let c = list.icons[0].corners;
        assert_eq!(c[0], [10.0, 10.0]);
        assert_eq!(c[2], [30.0, 30.0]);
    }

    // ---- chrome_rect (instanced SDF) tests ----

    #[test]
    fn chrome_rect_fast_path_records_one_instance() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(10.0, 20.0, 80.0, 30.0),
            6.0,
            2.0,
            [0.1, 0.2, 0.3, 1.0],
            [0.4, 0.5, 0.6, 1.0],
        );
        // One instance, one Chrome command, no soup geometry.
        assert_eq!(list.chrome_instances.len(), 1);
        assert_eq!(
            list.color_cmds,
            vec![super::ColorCmd::Chrome { instances: 0..1 }]
        );
        assert!(list.vertices.is_empty());
        let inst = list.chrome_instances[0];
        assert_eq!(inst.rect, [10.0, 20.0, 80.0, 30.0]);
        assert_eq!(inst.bg, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(inst.border, [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(inst.params, [6.0, 2.0, 0.0, 0.0]); // radius, thickness, no clip
    }

    #[test]
    fn chrome_rect_bakes_translation_into_world_rect() {
        let mut list = DrawList::new();
        list.translate(100.0, 50.0);
        list.chrome_rect(
            Rect::new(5.0, 5.0, 20.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        assert_eq!(list.chrome_instances[0].rect, [105.0, 55.0, 20.0, 10.0]);
    }

    #[test]
    fn chrome_rect_consecutive_calls_batch_into_one_run() {
        let mut list = DrawList::new();
        for i in 0..4 {
            list.chrome_rect(
                Rect::new(i as f32 * 10.0, 0.0, 8.0, 8.0),
                4.0,
                1.0,
                [1.0; 4],
                [0.0; 4],
            );
        }
        assert_eq!(list.chrome_instances.len(), 4);
        // All four collapse into a single contiguous Chrome run.
        assert_eq!(
            list.color_cmds,
            vec![super::ColorCmd::Chrome { instances: 0..4 }]
        );
    }

    #[test]
    fn chrome_rect_interleaves_with_soup_in_order() {
        let mut list = DrawList::new();
        // soup, chrome, soup, chrome. `line` stays in the soup (it is not
        // instanced), so it produces genuine Soup runs to interleave with chrome.
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0; 4]); // 6 indices
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0; 4]); // 6 more indices
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        assert_eq!(
            list.color_cmds,
            vec![
                super::ColorCmd::Soup { indices: 0..6 },
                super::ColorCmd::Chrome { instances: 0..1 },
                super::ColorCmd::Soup { indices: 6..12 },
                super::ColorCmd::Chrome { instances: 1..2 },
            ]
        );
        // Trailing soup (after the last command) is implicit: committed cursor
        // sits at the last flush, anything past it is the trailing run.
        assert_eq!(list.soup_committed_indices, 12);
        assert_eq!(list.indices.len(), 12);
    }

    #[test]
    fn chrome_rect_trailing_soup_left_uncommitted() {
        let mut list = DrawList::new();
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0; 4]); // soup after chrome
        // The trailing line is NOT in a command; the renderer draws
        // indices[committed..total] as the trailing run.
        assert_eq!(
            list.color_cmds,
            vec![super::ColorCmd::Chrome { instances: 0..1 }]
        );
        assert_eq!(list.soup_committed_indices, 0);
        assert_eq!(list.indices.len(), 6);
    }

    #[test]
    fn chrome_rect_falls_back_to_soup_under_rotation() {
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        list.chrome_rect(
            Rect::new(0.0, 0.0, 40.0, 20.0),
            6.0,
            2.0,
            [1.0; 4],
            [0.5; 4],
        );
        // No instance recorded; geometry went into the soup, transformed.
        assert!(list.chrome_instances.is_empty());
        assert!(list.color_cmds.is_empty());
        assert!(!list.vertices.is_empty());
    }

    #[test]
    fn chrome_rect_applies_tint() {
        let mut list = DrawList::new();
        list.set_tint([0.5, 0.5, 0.5, 1.0]);
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            1.0,
            [0.4, 0.6, 0.8, 1.0],
            [0.2, 0.2, 0.2, 1.0],
        );
        let inst = list.chrome_instances[0];
        assert!(approx(inst.bg[0], 0.2) && approx(inst.bg[1], 0.3) && approx(inst.bg[2], 0.4));
        assert!(approx(inst.border[0], 0.1));
    }

    #[test]
    fn chrome_rect_records_active_clip() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(5.0, 6.0, 30.0, 40.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        let inst = list.chrome_instances[0];
        assert_eq!(inst.clip, [5.0, 6.0, 30.0, 40.0]);
        assert_eq!(inst.params[2], 1.0); // clip_enabled
    }

    #[test]
    fn chrome_rect_zero_size_draws_nothing() {
        let mut list = DrawList::new();
        list.chrome_rect(Rect::new(0.0, 0.0, 0.0, 10.0), 4.0, 1.0, [1.0; 4], [0.0; 4]);
        assert!(list.chrome_instances.is_empty());
        assert!(list.color_cmds.is_empty());
    }

    #[test]
    fn clear_resets_chrome_state() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        assert!(!list.chrome_instances.is_empty());
        assert!(!list.color_cmds.is_empty());
        list.clear();
        assert!(list.chrome_instances.is_empty());
        assert!(list.color_cmds.is_empty());
        assert_eq!(list.soup_committed_indices, 0);
    }

    #[test]
    fn nine_slice_carries_transform() {
        let mut list = DrawList::new();
        list.translate(50.0, 60.0);
        list.scale(2.0, 2.0);
        list.nine_slice_id(0, 0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        let n = &list.nine_slices[0];
        // Local (0,0) -> world (50,60); local (10,10) -> (70,80).
        let tl = n.transform.transform_point([0.0, 0.0]);
        let br = n.transform.transform_point([10.0, 10.0]);
        assert!(approx(tl[0], 50.0) && approx(tl[1], 60.0));
        assert!(approx(br[0], 70.0) && approx(br[1], 80.0));
    }
}
