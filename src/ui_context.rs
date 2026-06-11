//! Teardown-style immediate-mode façade over `DrawList`.
//!
//! `UiContext` is a thin borrow over an existing `DrawList`. The transform and
//! tint stacks live on `DrawList` (so existing widget calls that take an
//! absolute `Rect` are transparently transform-aware); `UiContext` just adds
//! Teardown-flavoured verbs (`push`, `pop`, `translate`, `align`, `center`,
//! `color`, `color_filter`, `place_rect`) plus a per-stack-frame alignment.
//!
//! Pop is explicit. There is no `Drop`-based auto-pop, mirroring Teardown's
//! `UiPush`/`UiPop` semantics.

use crate::affine::Affine2;
use crate::layer::{LayerKind, LayerStack};
use crate::layout::Rect;
use crate::text::{FontHandle, TextBlock};
use crate::theme::Theme;
use crate::widgets::{
    Button, Checkbox, DragCapture, DragId, DrawContext, DropdownState, FocusId, FocusState,
    ScrollState, Slider, TextInput,
};
use crate::widgets::DrawList;
use crate::InputState;
use glyphon::{Style, Weight};
use std::collections::HashMap;

/// Horizontal alignment relative to the current origin.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignH {
    Left,
    Center,
    Right,
}

/// Vertical alignment relative to the current origin.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignV {
    Top,
    Middle,
    Bottom,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct AlignSpec {
    h: AlignH,
    v: AlignV,
}

/// A `UiWindow` frame: a world-space rect that redefines what `width`/`height`/
/// `center`/`middle` operate on (Teardown's `UiWindow`). Pushed by
/// [`UiContext::window_begin`], scoped to the enclosing `push`/`pop`.
#[derive(Copy, Clone, Debug, PartialEq)]
struct WindowFrame {
    rect: Rect,
}

impl AlignSpec {
    const DEFAULT: Self = Self {
        h: AlignH::Left,
        v: AlignV::Top,
    };

    /// Parse a Teardown-style align string. Accepts space-separated tokens in
    /// any order: `left|center|right` for horizontal and `top|middle|bottom`
    /// for vertical. Unknown tokens fall back to the previous component (and
    /// are returned in the second tuple element so the caller can surface a
    /// warning). Empty input returns `base` unchanged.
    fn parse(spec: &str, base: Self) -> (Self, Vec<String>) {
        let mut h = base.h;
        let mut v = base.v;
        let mut unknown = Vec::new();
        for token in spec.split_ascii_whitespace() {
            match token {
                "left" => h = AlignH::Left,
                "center" => h = AlignH::Center,
                "right" => h = AlignH::Right,
                "top" => v = AlignV::Top,
                "middle" | "center_v" => v = AlignV::Middle,
                "bottom" => v = AlignV::Bottom,
                other => unknown.push(other.to_string()),
            }
        }
        (Self { h, v }, unknown)
    }

    fn offset(&self, w: f32, h: f32) -> [f32; 2] {
        let x = match self.h {
            AlignH::Left => 0.0,
            AlignH::Center => -w * 0.5,
            AlignH::Right => -w,
        };
        let y = match self.v {
            AlignV::Top => 0.0,
            AlignV::Middle => -h * 0.5,
            AlignV::Bottom => -h,
        };
        [x, y]
    }
}

/// The current font selection — family, size, weight, and style — tracked on a
/// push/pop stack so Teardown's stateful `UiFont` verb (and bold/italic) scope
/// to their enclosing `UiPush`/`UiPop` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// Family handle, or `None` to fall back to the theme/bundled default.
    pub font: Option<FontHandle>,
    /// Font size in pixels.
    pub size: f32,
    /// Weight (e.g. `Weight::NORMAL` / `Weight::BOLD`).
    pub weight: Weight,
    /// Style (`Normal` / `Italic` / `Oblique`).
    pub style: Style,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            font: None,
            size: 16.0,
            weight: Weight::NORMAL,
            style: Style::Normal,
        }
    }
}

/// Caller-owned, frame-persistent state backing the interactive `UiContext`
/// verbs (`text_button`/`slider`/`checkbox`/`text_input`/…). Construct one per UI
/// surface and thread `&mut` into [`UiContext::interactive`] every frame, the
/// same way the crate already threads `DragCapture`/`FocusState`/`ScrollState`
/// into the raw widgets. Persists the bits an immediate-mode UI must remember
/// between frames: which draggable owns the pointer, which field has keyboard
/// focus, open dropdowns, scroll offsets, and per-field text-edit cursors.
#[derive(Default)]
pub struct UiState {
    /// Pointer-drag arbitration (sliders, scroll thumbs, …).
    pub drag: DragCapture,
    /// Keyboard-focus arbitration (text inputs, Tab ring).
    pub focus: FocusState,
    /// Open-dropdown / selection state for `Dropdown` widgets.
    pub dropdowns: DropdownState,
    /// Scroll offsets for `ScrollView` widgets.
    pub scroll: ScrollState,
    /// Persistent per-field text editors (cursor, selection), keyed by the
    /// `FocusId` passed to [`UiContext::text_input`]. The caller never touches
    /// these directly — the verb owns the cursor while the caller owns the
    /// `String`.
    text_inputs: HashMap<FocusId, TextInput>,
    /// Vertical gap inserted between auto-advanced verbs. Re-seeded from
    /// `theme.spacing` each frame by [`UiState::begin_frame`].
    pub item_gap: f32,
}

impl UiState {
    /// Fresh state. `item_gap` starts at 0 until the first
    /// [`begin_frame`](Self::begin_frame) seeds it from the theme.
    pub fn new() -> Self {
        Self::default()
    }

    /// Per-frame setup: arm focus navigation for this frame's Tab/Escape/click
    /// edges and seed the auto-advance gap from the theme. Call before building
    /// the frame's interactive verbs (mirrors [`InputState::end_frame`] timing).
    pub fn begin_frame(&mut self, input: &InputState, theme: &Theme) {
        self.focus.begin_frame(input);
        self.item_gap = theme.spacing;
    }

    /// Per-frame teardown: resolve focus navigation against the widgets
    /// registered this frame. Call after building the frame's verbs.
    pub fn end_frame(&mut self) {
        self.focus.end_frame(None);
    }
}

/// What `UiContext` is rendering into.
enum Backend<'a> {
    /// Plain draw list (no layer system; modal_begin/popup_begin will panic
    /// in debug if called).
    List(&'a mut DrawList),
    /// Full layer stack — modal_begin/popup_begin route here.
    Layers(&'a mut LayerStack),
}

impl<'a> Backend<'a> {
    fn list_mut(&mut self) -> &mut DrawList {
        match self {
            Backend::List(l) => l,
            Backend::Layers(s) => s.current_mut(),
        }
    }
}

/// Teardown-style façade over a `DrawList` or `LayerStack`. Owns no draw
/// state — borrows the backend for the duration of the build.
pub struct UiContext<'a> {
    backend: Backend<'a>,
    align_stack: Vec<AlignSpec>,
    /// Clip-stack depth recorded at each `push`, restored (`truncate_clip`) at the
    /// matching `pop` — makes `UiClipRect`/`UiWindow` clips scope to their
    /// push/pop frame, matching Teardown.
    clip_depth_stack: Vec<usize>,
    /// Active `UiWindow` frames. The top is the current window; empty means the
    /// full screen. Scoped to `push`/`pop` like clips.
    window_stack: Vec<WindowFrame>,
    /// `window_stack` length recorded at each `push`, restored at the matching
    /// `pop`.
    window_depth_stack: Vec<usize>,
    /// Stack of layer kinds still open — used by Drop debug_assert, by
    /// modal_end / popup_end to verify the caller closed the right kind, and
    /// to detect unbalanced begin/end pairs. Length == number of open layers.
    open_layer_kinds: Vec<LayerKind>,
    /// Names of unknown align tokens we've already warned about, to keep one
    /// typo from spamming the log every frame.
    warned_align_tokens: std::collections::HashSet<String>,
    /// Active font selection, scoped to `push`/`pop` like `align_stack`. The top
    /// is the current font; always at least one entry (`FontSpec::default()`).
    font_stack: Vec<FontSpec>,
    /// Per-frame input snapshot. `Some` only in interactive mode (constructed
    /// via [`UiContext::interactive`]/[`interactive_layers`](Self::interactive_layers)).
    input: Option<&'a InputState>,
    /// Caller-owned persistent widget state. `Some` only in interactive mode.
    state: Option<&'a mut UiState>,
    /// Active theme (colours, sizes, fonts). `Some` only in interactive mode —
    /// every stateful widget needs a `&Theme`.
    theme: Option<&'a Theme>,
}

impl<'a> UiContext<'a> {
    /// Wrap an existing `DrawList`. `modal_begin`/`popup_begin` will
    /// debug_assert when called on this variant — switch to
    /// [`UiContext::with_layers`] for full layer support.
    pub fn new(list: &'a mut DrawList) -> Self {
        Self {
            backend: Backend::List(list),
            align_stack: vec![AlignSpec::DEFAULT],
            clip_depth_stack: Vec::new(),
            window_stack: Vec::new(),
            window_depth_stack: Vec::new(),
            open_layer_kinds: Vec::new(),
            warned_align_tokens: std::collections::HashSet::new(),
            font_stack: vec![FontSpec::default()],
            input: None,
            state: None,
            theme: None,
        }
    }

    /// Wrap a `LayerStack`. Enables `modal_begin`/`popup_begin`.
    pub fn with_layers(layers: &'a mut LayerStack) -> Self {
        Self {
            backend: Backend::Layers(layers),
            align_stack: vec![AlignSpec::DEFAULT],
            clip_depth_stack: Vec::new(),
            window_stack: Vec::new(),
            window_depth_stack: Vec::new(),
            open_layer_kinds: Vec::new(),
            warned_align_tokens: std::collections::HashSet::new(),
            font_stack: vec![FontSpec::default()],
            input: None,
            state: None,
            theme: None,
        }
    }

    /// Wrap a `DrawList` in **interactive** mode: stateful verbs
    /// (`text_button`/`slider`/`checkbox`/`text_input`/…) become available, each
    /// reading `input`, mutating the caller-owned `state`, and laying out with
    /// `theme`. Draw-only verbs keep working too. Use
    /// [`interactive_layers`](Self::interactive_layers) for modal/popup support.
    pub fn interactive(
        list: &'a mut DrawList,
        input: &'a InputState,
        state: &'a mut UiState,
        theme: &'a Theme,
    ) -> Self {
        let mut ctx = Self::new(list);
        ctx.input = Some(input);
        ctx.state = Some(state);
        ctx.theme = Some(theme);
        ctx
    }

    /// Wrap a `LayerStack` in **interactive** mode (see
    /// [`interactive`](Self::interactive)), additionally enabling
    /// `modal_begin`/`popup_begin`.
    pub fn interactive_layers(
        layers: &'a mut LayerStack,
        input: &'a InputState,
        state: &'a mut UiState,
        theme: &'a Theme,
    ) -> Self {
        let mut ctx = Self::with_layers(layers);
        ctx.input = Some(input);
        ctx.state = Some(state);
        ctx.theme = Some(theme);
        ctx
    }

    /// Push transform + tint + align + clip/window scope (Teardown's `UiPush`).
    pub fn push(&mut self) {
        let clip_depth = self.backend.list_mut().clip_len();
        let window_depth = self.window_stack.len();
        let list = self.backend.list_mut();
        list.push_transform();
        list.push_tint();
        let top = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        self.align_stack.push(top);
        let font_top = self.font_stack.last().cloned().unwrap_or_default();
        self.font_stack.push(font_top);
        self.clip_depth_stack.push(clip_depth);
        self.window_depth_stack.push(window_depth);
    }

    /// Pop transform + tint + align + clip/window scope (Teardown's `UiPop`).
    ///
    /// Any `UiClipRect`/`UiWindow` set since the matching `push` is restored
    /// here, so clips don't leak past their frame.
    pub fn pop(&mut self) {
        let list = self.backend.list_mut();
        list.pop_transform();
        list.pop_tint();
        if self.align_stack.len() > 1 {
            self.align_stack.pop();
        }
        if self.font_stack.len() > 1 {
            self.font_stack.pop();
        }
        if let Some(depth) = self.clip_depth_stack.pop() {
            self.backend.list_mut().truncate_clip(depth);
        }
        if let Some(depth) = self.window_depth_stack.pop() {
            self.window_stack.truncate(depth);
        }
    }

    /// Shift the local origin (Teardown's `UiTranslate`).
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.backend.list_mut().translate(dx, dy);
    }

    /// Rotate the local coordinate frame (Teardown's `UiRotate` is in degrees;
    /// we take radians to match Rust convention. Use `f32::to_radians()` to
    /// convert from degrees at the call site).
    pub fn rotate(&mut self, angle_radians: f32) {
        self.backend.list_mut().rotate(angle_radians);
    }

    /// Non-uniform scale the local coordinate frame (Teardown's `UiScale`).
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.backend.list_mut().scale(sx, sy);
    }

    /// Set alignment for subsequent placement helpers (Teardown's `UiAlign`).
    pub fn align(&mut self, spec: &str) {
        let base = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let (new_spec, unknown) = AlignSpec::parse(spec, base);
        for token in unknown {
            if self.warned_align_tokens.insert(token.clone()) {
                log::warn!(
                    "wgpu-gameui: UiContext::align received unknown token '{}' \
                     (expected one of: left|center|right, top|middle|bottom) — \
                     ignoring",
                    token
                );
            }
        }
        if let Some(top) = self.align_stack.last_mut() {
            *top = new_spec;
        }
    }

    /// Shorthand for `align("center middle")` (Teardown's `UiCenter`).
    pub fn center(&mut self) {
        if let Some(top) = self.align_stack.last_mut() {
            *top = AlignSpec {
                h: AlignH::Center,
                v: AlignV::Middle,
            };
        }
    }

    /// Set the current font family and size (Teardown's `UiFont(path, size)`).
    /// Scoped to the enclosing `push`/`pop`.
    pub fn font(&mut self, font: FontHandle, size: f32) {
        if let Some(top) = self.font_stack.last_mut() {
            top.font = Some(font);
            top.size = size;
        }
    }

    /// Set just the current font size, leaving family/weight/style intact
    /// (Teardown's `UiFontSize`).
    pub fn font_size(&mut self, size: f32) {
        if let Some(top) = self.font_stack.last_mut() {
            top.size = size;
        }
    }

    /// Set just the current font family, leaving size/weight/style intact.
    pub fn font_family(&mut self, font: FontHandle) {
        if let Some(top) = self.font_stack.last_mut() {
            top.font = Some(font);
        }
    }

    /// Toggle bold (`Weight::BOLD` when `on`, else `Weight::NORMAL`).
    pub fn bold(&mut self, on: bool) {
        if let Some(top) = self.font_stack.last_mut() {
            top.weight = if on { Weight::BOLD } else { Weight::NORMAL };
        }
    }

    /// Set an explicit font weight.
    pub fn font_weight(&mut self, weight: Weight) {
        if let Some(top) = self.font_stack.last_mut() {
            top.weight = weight;
        }
    }

    /// Toggle italic (`Style::Italic` when `on`, else `Style::Normal`).
    pub fn italic(&mut self, on: bool) {
        if let Some(top) = self.font_stack.last_mut() {
            top.style = if on { Style::Italic } else { Style::Normal };
        }
    }

    /// Set an explicit font style (`Normal` / `Italic` / `Oblique`).
    pub fn font_style(&mut self, style: Style) {
        if let Some(top) = self.font_stack.last_mut() {
            top.style = style;
        }
    }

    /// The current font selection (clone of the stack top).
    pub fn current_font(&self) -> FontSpec {
        self.font_stack.last().cloned().unwrap_or_default()
    }

    /// Draw a single line of text using the current font stack (size, family,
    /// weight, style), honoring align/transform like [`text`](Self::text). The
    /// line's box for alignment is the font size × line-height of the active
    /// `FontSpec`.
    pub fn text_line(&mut self, text: &str, color: [f32; 4]) {
        let spec = self.current_font();
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
        let block = TextBlock::new(text, 0.0, 0.0)
            .with_size(spec.size)
            .with_rgba(to_u8(color[0]), to_u8(color[1]), to_u8(color[2]), to_u8(color[3]))
            .with_font_opt(spec.font)
            .with_weight(spec.weight)
            .with_style(spec.style);
        self.text_block(block);
    }

    /// Replace the current tint (Teardown's `UiColor`).
    pub fn color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.backend.list_mut().set_tint([r, g, b, a]);
    }

    /// Multiply into the current tint (Teardown's `UiColorFilter`).
    pub fn color_filter(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.backend.list_mut().multiply_tint([r, g, b, a]);
    }

    /// Return the current world-space cursor position (origin of the local
    /// frame after all active transforms).
    pub fn cursor(&mut self) -> [f32; 2] {
        self.backend
            .list_mut()
            .current_transform()
            .transform_point([0.0, 0.0])
    }

    /// Compute the world-space rect for a widget of the given local size at
    /// the current origin under the active alignment, then transform through
    /// the active affine.
    pub fn place_rect(&mut self, width: f32, height: f32) -> Rect {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(width, height);
        let local = Rect::new(ox, oy, width, height);
        self.backend
            .list_mut()
            .current_transform()
            .transform_rect_aabb(local)
    }

    /// Clip subsequent drawing to a `w`×`h` rect at the current origin
    /// (Teardown's `UiClipRect`). When `inherit` is true the new clip is
    /// intersected with the active clip; otherwise it replaces it. Scoped to the
    /// enclosing `push`/`pop`.
    pub fn clip_rect(&mut self, w: f32, h: f32, inherit: bool) {
        let local = Rect::new(0.0, 0.0, w, h);
        let list = self.backend.list_mut();
        if inherit {
            list.push_clip(local);
        } else {
            list.push_clip_exact(local);
        }
    }

    /// Begin a `w`×`h` window at the current origin (Teardown's `UiWindow`).
    /// Subsequent `width`/`height`/`center`/`middle` operate in the window's
    /// size. When `clip` is true the window also clips its contents (see
    /// [`clip_rect`](Self::clip_rect) for the `inherit` semantics). Scoped to the
    /// enclosing `push`/`pop`.
    pub fn window_begin(&mut self, w: f32, h: f32, clip: bool, inherit: bool) {
        let rect = self
            .backend
            .list_mut()
            .current_transform()
            .transform_rect_aabb(Rect::new(0.0, 0.0, w, h));
        self.window_stack.push(WindowFrame { rect });
        if clip {
            self.clip_rect(w, h, inherit);
        }
    }

    /// The current `UiWindow` rect in world space, or `None` when no window is
    /// active (full screen).
    pub fn current_window_rect(&self) -> Option<Rect> {
        self.window_stack.last().map(|w| w.rect)
    }

    /// The active clip rect in world space, or `None` when nothing is clipped.
    pub fn current_clip(&mut self) -> Option<Rect> {
        self.backend.list_mut().current_clip()
    }

    /// True when the world-space point `(x, y)` is inside the active clip region
    /// (always true when nothing is clipped). Teardown's `UiIsInClipRegion`.
    pub fn is_in_clip_region(&mut self, x: f32, y: f32) -> bool {
        match self.backend.list_mut().current_clip() {
            Some(c) => x >= c.x && x <= c.x + c.width && y >= c.y && y <= c.y + c.height,
            None => true,
        }
    }

    /// True when a `w`×`h` rect at the current origin lies fully outside the
    /// active clip region (never, when nothing is clipped). Teardown's
    /// `UiIsRectFullyClipped`.
    pub fn is_rect_fully_clipped(&mut self, w: f32, h: f32) -> bool {
        let list = self.backend.list_mut();
        let world = list
            .current_transform()
            .transform_rect_aabb(Rect::new(0.0, 0.0, w, h));
        match list.current_clip() {
            Some(c) => c.intersection(world).is_none(),
            None => false,
        }
    }

    /// Draw a colored quad of the given size at the aligned origin.
    pub fn quad(&mut self, w: f32, h: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend.list_mut().quad(ox, oy, w, h, color);
    }

    /// Draw a rounded rect of the given size at the aligned origin.
    pub fn rounded_rect(&mut self, w: f32, h: f32, radius: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend
            .list_mut()
            .rounded_rect(Rect::new(ox, oy, w, h), radius, color);
    }

    /// Draw a rectangle outline of the given size at the aligned origin
    /// (Teardown's `UiRectOutline`). The outer edge is flush with the aligned
    /// box; the border grows inward.
    pub fn rect_outline(&mut self, w: f32, h: f32, thickness: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend
            .list_mut()
            .rect_outline(Rect::new(ox, oy, w, h), thickness, color);
    }

    /// Draw a rounded-rectangle outline of the given size at the aligned origin
    /// (Teardown's `UiRoundedRectOutline`).
    pub fn rounded_rect_outline(
        &mut self,
        w: f32,
        h: f32,
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend
            .list_mut()
            .rounded_rect_outline(Rect::new(ox, oy, w, h), radius, thickness, color);
    }

    /// Draw a filled circle of the given radius at the aligned origin
    /// (Teardown's `UiCircle`). The circle occupies a `2r×2r` box for alignment,
    /// so the default `left top` align puts the *box corner* at the origin and
    /// `center middle` puts the *center* at the origin.
    pub fn circle(&mut self, radius: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(radius * 2.0, radius * 2.0);
        self.backend
            .list_mut()
            .circle((ox + radius, oy + radius), radius, color);
    }

    /// Draw a circle outline of the given radius/thickness at the aligned origin
    /// (Teardown's `UiCircleOutline`). Aligned like [`circle`](Self::circle).
    pub fn circle_outline(&mut self, radius: f32, thickness: f32, color: [f32; 4]) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(radius * 2.0, radius * 2.0);
        self.backend
            .list_mut()
            .circle_outline((ox + radius, oy + radius), radius, thickness, color);
    }

    /// The current per-axis scale factors of the active transform (Teardown's
    /// `UiGetScale`). Derived from the basis-vector lengths of the active
    /// affine, so `UiScale(2, 3)` reports `(2, 3)` and a rotation reports the
    /// unchanged scale.
    pub fn current_scale(&mut self) -> (f32, f32) {
        let m = self.backend.list_mut().current_transform();
        let sx = (m.a * m.a + m.c * m.c).sqrt();
        let sy = (m.b * m.b + m.d * m.d).sqrt();
        (sx, sy)
    }

    /// Draw an atlas image (by key) of the given size at the aligned origin.
    /// The key is resolved against the renderer's sprite atlas at render time, so
    /// the sprite need only exist by the time [`crate::UiRenderer::render`] runs.
    pub fn icon(&mut self, key: &str, w: f32, h: f32) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let [ox, oy] = align.offset(w, h);
        self.backend.list_mut().icon(key, ox, oy, w, h);
    }

    /// Draw a pre-built [`TextBlock`] whose origin honours align/transform.
    /// (The auto-advancing string verb is [`text`](Self::text).)
    pub fn text_block(&mut self, mut block: TextBlock) {
        let align = *self.align_stack.last().unwrap_or(&AlignSpec::DEFAULT);
        let w = block.max_width;
        let h = block.line_height;
        let [ox, oy] = align.offset(w, h);
        block.x += ox;
        block.y += oy;
        self.backend.list_mut().text(block);
    }

    /// Direct access to the underlying `DrawList` (the currently active layer's
    /// list when running on a `LayerStack`).
    pub fn list(&mut self) -> &mut DrawList {
        self.backend.list_mut()
    }

    // ------------------------------------------------------------------
    // Interactive verbs (require `UiContext::interactive[_layers]`)
    // ------------------------------------------------------------------

    /// Validate interactive mode and return the copied-out `&InputState` +
    /// `&Theme` refs (both are `Copy` `Option<&_>`, so this borrows nothing of
    /// `self`). Returns `None` after a `debug_assert!` when called on a draw-only
    /// context, so verbs degrade to a no-op in release instead of panicking —
    /// mirroring `modal_begin` on a `DrawList` backend.
    fn interactive_refs(&self) -> Option<(&'a InputState, &'a Theme)> {
        match (self.input, self.theme) {
            (Some(i), Some(t)) => Some((i, t)),
            _ => {
                debug_assert!(
                    false,
                    "interactive UiContext verb called on a draw-only context; \
                     construct via UiContext::interactive(list, input, state, theme)"
                );
                None
            }
        }
    }

    /// Un-apply the active transform from a placed world rect (and map the mouse
    /// the same way) so a raw widget — which re-applies the active transform via
    /// the `DrawList` — lands back at `world` on screen, and its
    /// `rect.contains(mouse)` hit test matches the on-screen position. `inv` is
    /// the inverse of the active affine.
    fn localize(inv: Affine2, world: Rect, input: &InputState) -> (Rect, InputState) {
        let local = inv.transform_rect_aabb(world);
        let [mx, my] = inv.transform_point([input.mouse_x, input.mouse_y]);
        let mut li = input.clone();
        li.mouse_x = mx;
        li.mouse_y = my;
        (local, li)
    }

    /// Advance the vertical layout cursor by `height` plus the current
    /// `item_gap` (Teardown-style stacking). No-op gap when state is absent.
    fn advance(&mut self, height: f32) {
        let gap = self.state.as_ref().map_or(0.0, |s| s.item_gap);
        self.backend.list_mut().translate(0.0, height + gap);
    }

    /// Default widget width: the inner width of the active `UiWindow`, else
    /// 200px. Used when a verb's width argument is `None`.
    fn default_field_width(&self) -> f32 {
        self.current_window_rect().map_or(200.0, |r| r.width)
    }

    /// Draw a line of text in the current theme text colour using the active
    /// font stack, then advance the layout cursor by the font size. The
    /// auto-advancing companion to [`text_block`](Self::text_block) /
    /// [`text_line`](Self::text_line).
    pub fn text(&mut self, label: &str) {
        let color = self.theme.map_or([1.0, 1.0, 1.0, 1.0], |t| t.text);
        let size = self.current_font().size;
        self.text_line(label, color);
        self.advance(size);
    }

    /// Draw a chrome text button and report whether it was clicked this frame.
    /// `w`/`h` default to [`default_field_width`](Self::default_field_width) /
    /// `theme.button_height`. Auto-advances by the button height.
    pub fn text_button(&mut self, label: &str, w: Option<f32>, h: Option<f32>) -> bool {
        let (input, theme) = match self.interactive_refs() {
            Some(v) => v,
            None => return false,
        };
        let width = w.unwrap_or_else(|| self.default_field_width());
        let height = h.unwrap_or(theme.button_height);
        let world = self.place_rect(width, height);
        let inv = self.backend.list_mut().current_transform().inverse();
        let (local, local_input) = Self::localize(inv, world, input);
        let clicked = {
            let list = self.backend.list_mut();
            let state = self.state.as_mut().expect("text_button requires interactive state");
            let mut ctx = DrawContext::new(list, &mut state.focus, theme, &local_input, 0.0, 0.0);
            Button::draw_at(label, local, true, &mut ctx)
        };
        self.advance(height);
        clicked
    }

    /// Draw a slider for `value` in `[min, max]` and return the (possibly
    /// updated) value. `id` is a stable per-slider [`DragId`]. `w` defaults to
    /// [`default_field_width`](Self::default_field_width); height is
    /// `theme.input_height`. Auto-advances by the height.
    pub fn slider(&mut self, id: DragId, value: f32, min: f32, max: f32, w: Option<f32>) -> f32 {
        let (input, theme) = match self.interactive_refs() {
            Some(v) => v,
            None => return value,
        };
        let width = w.unwrap_or_else(|| self.default_field_width());
        let height = theme.input_height;
        let world = self.place_rect(width, height);
        let inv = self.backend.list_mut().current_transform().inverse();
        let (local, local_input) = Self::localize(inv, world, input);
        let new_value = {
            // Disjoint field borrows: `self.backend` (list) and `self.state`'s
            // `drag`/`focus` fields (the slider needs the drag arbiter; the
            // DrawContext carries focus even though the slider registers none).
            let list = self.backend.list_mut();
            let state = match self.state.as_mut() {
                Some(s) => s,
                None => {
                    debug_assert!(false, "UiContext::slider requires interactive state");
                    return value;
                }
            };
            let mut ctx = DrawContext::new(list, &mut state.focus, theme, &local_input, 0.0, 0.0);
            Slider::new(min, max)
                .draw(value, id, &mut state.drag, local, &mut ctx)
                .value
        };
        self.advance(height);
        new_value
    }

    /// Draw a checkbox with `label` and current `checked` state; return the new
    /// checked state (toggles on click). The widget is stateless, so no id is
    /// needed. Auto-advances by `max(font_size, 20)`.
    pub fn checkbox(&mut self, label: &str, checked: bool) -> bool {
        let (input, theme) = match self.interactive_refs() {
            Some(v) => v,
            None => return checked,
        };
        let height = theme.font_size.max(20.0);
        // The checkbox box is fitted to rect height; give the row enough width
        // for the box plus the label area (default field width).
        let width = self.default_field_width();
        let world = self.place_rect(width, height);
        let inv = self.backend.list_mut().current_transform().inverse();
        let (local, local_input) = Self::localize(inv, world, input);
        let toggled = {
            let list = self.backend.list_mut();
            let state = self.state.as_mut().expect("checkbox requires interactive state");
            let mut ctx = DrawContext::new(list, &mut state.focus, theme, &local_input, 0.0, 0.0);
            Checkbox::new().draw(checked, label, local, &mut ctx)
        };
        self.advance(height);
        if toggled {
            !checked
        } else {
            checked
        }
    }

    /// Draw an atlas image (by key) of size `w`×`h` at the aligned origin and
    /// advance by `h`. The auto-advancing companion to [`icon`](Self::icon);
    /// needs no input/state, so it works in draw-only mode too.
    pub fn image_box(&mut self, key: &str, w: f32, h: f32) {
        self.icon(key, w, h);
        self.advance(h);
    }

    /// Draw a single-line text input bound to the caller's `buffer`. Persists
    /// the edit cursor/selection in `UiState` keyed by `id`; syncs the caller's
    /// `&mut String` in (external changes win) and out (edits are written back).
    /// Returns whether the text changed this frame. `w` defaults to
    /// [`default_field_width`](Self::default_field_width); height is
    /// `theme.input_height`. Auto-advances by the height.
    pub fn text_input(
        &mut self,
        id: FocusId,
        buffer: &mut String,
        placeholder: &str,
        w: Option<f32>,
    ) -> bool {
        let (input, theme) = match self.interactive_refs() {
            Some(v) => v,
            None => return false,
        };
        let width = w.unwrap_or_else(|| self.default_field_width());
        let height = theme.input_height;
        let world = self.place_rect(width, height);
        let inv = self.backend.list_mut().current_transform().inverse();
        let (local, local_input) = Self::localize(inv, world, input);
        let changed = {
            // Disjoint field borrows: `self.backend` (list) and `self.state`.
            let list = self.backend.list_mut();
            let state = match self.state.as_mut() {
                Some(s) => s,
                None => {
                    debug_assert!(false, "UiContext::text_input requires interactive state");
                    return false;
                }
            };
            // Touch two `UiState` fields at once.
            let UiState {
                text_inputs, focus, ..
            } = &mut **state;
            let ti = text_inputs.entry(id).or_insert_with(|| {
                let mut t = TextInput::new(local.x, local.y, local.width, local.height);
                t.value = buffer.clone();
                t.cursor_pos = t.value.len();
                t
            });
            // Keep geometry + placeholder synced to this frame's placed rect.
            ti.x = local.x;
            ti.y = local.y;
            ti.width = local.width;
            ti.height = local.height;
            ti.placeholder.clear();
            ti.placeholder.push_str(placeholder);
            // External changes to the caller's buffer win over our cached value.
            if ti.value != *buffer {
                ti.value = buffer.clone();
                if ti.cursor_pos > ti.value.len() {
                    ti.cursor_pos = ti.value.len();
                }
                ti.selection_start = None;
            }
            let before = ti.value.clone();
            let mut ctx = DrawContext::new(list, focus, theme, &local_input, 0.0, 0.0);
            ti.draw(id, &mut ctx);
            let changed = ti.value != before;
            if changed {
                buffer.clear();
                buffer.push_str(&ti.value);
            }
            changed
        };
        self.advance(height);
        changed
    }

    /// Open a modal layer covering `rect`. Subsequent draw calls go to the
    /// modal layer until `modal_end` is called. Lower layers receive
    /// `mouse_consumed = true` for input dispatch.
    ///
    /// Calling this on a `UiContext::new(DrawList)` (no layers) hits a
    /// `debug_assert!` — switch to `UiContext::with_layers` for modal support.
    pub fn modal_begin(&mut self, rect: Rect) {
        match &mut self.backend {
            Backend::List(_) => {
                debug_assert!(
                    false,
                    "UiContext::modal_begin requires a LayerStack backend; \
                     construct via UiContext::with_layers(...)"
                );
            }
            Backend::Layers(s) => {
                s.push_modal(rect);
                self.open_layer_kinds.push(LayerKind::Modal);
            }
        }
    }

    /// Close the most recent modal layer. Debug-asserts that the most-recent
    /// open layer was opened with `modal_begin`.
    pub fn modal_end(&mut self) {
        self.close_layer(LayerKind::Modal);
    }

    /// Open a popup layer with bounding `rect`. Clicks inside `rect` are
    /// captured (lower layers see `mouse_consumed`); clicks outside fall
    /// through.
    pub fn popup_begin(&mut self, rect: Rect) {
        match &mut self.backend {
            Backend::List(_) => {
                debug_assert!(
                    false,
                    "UiContext::popup_begin requires a LayerStack backend; \
                     construct via UiContext::with_layers(...)"
                );
            }
            Backend::Layers(s) => {
                s.push_popup(rect);
                self.open_layer_kinds.push(LayerKind::Popup);
            }
        }
    }

    /// Close the most recent popup layer. Debug-asserts that the most-recent
    /// open layer was opened with `popup_begin`.
    pub fn popup_end(&mut self) {
        self.close_layer(LayerKind::Popup);
    }

    fn close_layer(&mut self, expected: LayerKind) {
        match &mut self.backend {
            Backend::List(_) => {
                debug_assert!(
                    false,
                    "UiContext::*_end called on a UiContext that has no layer backend"
                );
            }
            Backend::Layers(s) => {
                let top = self.open_layer_kinds.last().copied();
                // Pop *before* asserting so a kind-mismatch panic doesn't
                // turn into a double-panic via Drop's balance check.
                if !self.open_layer_kinds.is_empty() {
                    s.pop_layer();
                    self.open_layer_kinds.pop();
                }
                debug_assert!(
                    top.is_some(),
                    "UiContext::*_end called with no open layer"
                );
                debug_assert!(
                    top == Some(expected),
                    "UiContext layer kind mismatch: expected to close a {:?}, but the most-recent open layer is a {:?}",
                    expected,
                    top
                );
            }
        }
    }
}

impl<'a> Drop for UiContext<'a> {
    /// Surfaces unbalanced `push`/`pop` calls in debug builds.
    fn drop(&mut self) {
        debug_assert_eq!(
            self.align_stack.len(),
            1,
            "UiContext dropped with {} unbalanced push/pop pair(s) on the align stack",
            self.align_stack.len() - 1
        );
        debug_assert_eq!(
            self.font_stack.len(),
            1,
            "UiContext dropped with {} unbalanced push/pop pair(s) on the font stack",
            self.font_stack.len() - 1
        );
        debug_assert_eq!(
            self.open_layer_kinds.len(),
            0,
            "UiContext dropped with {} unbalanced modal_begin/end or popup_begin/end pair(s)",
            self.open_layer_kinds.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn align_left_top_at_origin() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        let r = ui.place_rect(10.0, 20.0);
        assert_eq!(r, Rect::new(0.0, 0.0, 10.0, 20.0));
    }

    #[test]
    fn align_center_middle_centers_rect() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("center middle");
        let r = ui.place_rect(10.0, 10.0);
        assert_eq!(r, Rect::new(-5.0, -5.0, 10.0, 10.0));
    }

    #[test]
    fn align_right_bottom_offsets_rect() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("right bottom");
        let r = ui.place_rect(10.0, 20.0);
        assert_eq!(r, Rect::new(-10.0, -20.0, 10.0, 20.0));
    }

    #[test]
    fn translate_then_place_shifts() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.translate(100.0, 50.0);
        let r = ui.place_rect(10.0, 10.0);
        assert_eq!(r, Rect::new(100.0, 50.0, 10.0, 10.0));
    }

    #[test]
    fn scale_doubles_size_under_translate_only() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.scale(2.0, 2.0);
        let r = ui.place_rect(10.0, 10.0);
        assert!(approx(r.width, 20.0));
        assert!(approx(r.height, 20.0));
    }

    #[test]
    fn color_replaces_tint() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.color(0.5, 0.5, 0.5, 1.0);
        ui.color(0.25, 0.25, 0.25, 1.0);
        assert_eq!(ui.list().current_tint(), [0.25, 0.25, 0.25, 1.0]);
    }

    #[test]
    fn color_filter_multiplies_tint() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.color(0.5, 0.5, 0.5, 1.0);
        ui.color_filter(0.5, 0.5, 0.5, 1.0);
        let t = ui.list().current_tint();
        assert!(approx(t[0], 0.25));
        assert!(approx(t[1], 0.25));
        assert!(approx(t[2], 0.25));
        assert!(approx(t[3], 1.0));
    }

    #[test]
    fn push_pop_balances_align_too() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("center middle");
        ui.push();
        ui.align("right bottom");
        let r1 = ui.place_rect(10.0, 10.0);
        assert_eq!(r1, Rect::new(-10.0, -10.0, 10.0, 10.0));
        ui.pop();
        let r2 = ui.place_rect(10.0, 10.0);
        assert_eq!(r2, Rect::new(-5.0, -5.0, 10.0, 10.0));
    }

    #[test]
    fn cursor_returns_world_origin() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.translate(7.0, 11.0);
        ui.scale(2.0, 2.0);
        ui.translate(3.0, 4.0);
        let c = ui.cursor();
        // local (0,0) -> scale -> (0,0) -> translate(3,4) -> ... but that's
        // local-side. Composed: translate(7,11) * scale(2,2) * translate(3,4)
        // applied to (0,0) is translate(7,11) * scale(2,2) of (3,4) = (7+6, 11+8).
        assert!(approx(c[0], 13.0));
        assert!(approx(c[1], 19.0));
    }

    #[test]
    fn center_is_shorthand_for_center_middle() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.center();
        let r = ui.place_rect(10.0, 10.0);
        assert_eq!(r, Rect::new(-5.0, -5.0, 10.0, 10.0));
    }

    #[test]
    fn quad_via_context_uses_align() {
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::new(&mut list);
            ui.translate(100.0, 100.0);
            ui.center();
            ui.quad(20.0, 20.0, [1.0, 1.0, 1.0, 1.0]);
        }
        // A translate-only quad records one chrome instance; its world rect
        // origin should be at (100 - 10, 100 - 10) = (90, 90).
        assert_eq!(list.chrome_instances.len(), 1);
        assert_eq!(
            [list.chrome_instances[0].rect[0], list.chrome_instances[0].rect[1]],
            [90.0, 90.0]
        );
    }

    #[test]
    fn rect_outline_via_context_uses_align() {
        // Under center-middle, a 20×20 outline at origin 100,100 has its top
        // strip's first vertex at the box top-left (90, 90).
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::new(&mut list);
            ui.translate(100.0, 100.0);
            ui.center();
            ui.rect_outline(20.0, 20.0, 2.0, [1.0, 1.0, 1.0, 1.0]);
        }
        // A translate-only outline records one chrome (stroke) instance; its
        // world rect origin is the box top-left (90, 90).
        assert_eq!(list.chrome_instances.len(), 1);
        assert_eq!(
            [list.chrome_instances[0].rect[0], list.chrome_instances[0].rect[1]],
            [90.0, 90.0]
        );
    }

    #[test]
    fn circle_via_context_centers_under_center_middle() {
        // center-middle: the circle's center sits at the origin. A translate-only
        // circle records one SDF instance whose center is at the origin.
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::new(&mut list);
            ui.translate(50.0, 60.0);
            ui.center();
            ui.circle(10.0, [1.0, 1.0, 1.0, 1.0]);
        }
        assert_eq!(list.circle_instances.len(), 1);
        assert_eq!(
            [list.circle_instances[0].center[0], list.circle_instances[0].center[1]],
            [50.0, 60.0]
        );
    }

    #[test]
    fn circle_via_context_left_top_offsets_by_radius() {
        // left-top (default): the 2r box corner is at the origin, so the center
        // is at origin + (r, r).
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::new(&mut list);
            ui.translate(50.0, 60.0);
            ui.circle(10.0, [1.0, 1.0, 1.0, 1.0]);
        }
        assert_eq!(list.circle_instances.len(), 1);
        assert_eq!(
            [list.circle_instances[0].center[0], list.circle_instances[0].center[1]],
            [60.0, 70.0]
        );
    }

    #[test]
    fn current_scale_reports_axis_factors() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.scale(2.0, 3.0);
        let (sx, sy) = ui.current_scale();
        assert!(approx(sx, 2.0));
        assert!(approx(sy, 3.0));
    }

    #[test]
    fn current_scale_unaffected_by_rotation() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.rotate(0.7);
        let (sx, sy) = ui.current_scale();
        assert!(approx(sx, 1.0));
        assert!(approx(sy, 1.0));
    }

    #[test]
    fn clip_rect_scoped_to_push_pop() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        assert_eq!(ui.current_clip(), None);
        ui.push();
        ui.clip_rect(100.0, 50.0, false);
        assert_eq!(ui.current_clip(), Some(Rect::new(0.0, 0.0, 100.0, 50.0)));
        ui.pop();
        // The clip is gone once its frame closes.
        assert_eq!(ui.current_clip(), None);
    }

    #[test]
    fn clip_rect_inherit_intersects_parent() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.push();
        ui.clip_rect(50.0, 50.0, false); // parent
        ui.push();
        ui.clip_rect(100.0, 100.0, true); // inherit → intersected down to 50×50
        assert_eq!(ui.current_clip(), Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
        ui.pop();
        ui.pop();
    }

    #[test]
    fn clip_rect_no_inherit_replaces_parent() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.push();
        ui.clip_rect(50.0, 50.0, false); // parent
        ui.push();
        ui.clip_rect(100.0, 100.0, false); // replace → larger than parent
        assert_eq!(ui.current_clip(), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
        ui.pop();
        // Parent clip restored.
        assert_eq!(ui.current_clip(), Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
        ui.pop();
    }

    #[test]
    fn window_begin_sets_current_window_and_clips() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.translate(200.0, 100.0);
        ui.push();
        ui.window_begin(400.0, 200.0, true, false);
        assert_eq!(
            ui.current_window_rect(),
            Some(Rect::new(200.0, 100.0, 400.0, 200.0))
        );
        assert_eq!(ui.current_clip(), Some(Rect::new(200.0, 100.0, 400.0, 200.0)));
        ui.pop();
        assert_eq!(ui.current_window_rect(), None);
        assert_eq!(ui.current_clip(), None);
    }

    #[test]
    fn is_rect_fully_clipped_outside_region() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.push();
        ui.clip_rect(50.0, 50.0, false);
        // A rect at the origin overlaps the clip.
        assert!(!ui.is_rect_fully_clipped(10.0, 10.0));
        // Translate well outside the clip, then test.
        ui.translate(500.0, 500.0);
        assert!(ui.is_rect_fully_clipped(10.0, 10.0));
        ui.pop();
        // No clip → never fully clipped.
        assert!(!ui.is_rect_fully_clipped(10.0, 10.0));
    }

    #[test]
    fn align_unknown_token_is_collected() {
        let base = AlignSpec::DEFAULT;
        let (spec, unknown) = AlignSpec::parse("center wibble bottom", base);
        assert_eq!(spec.h, AlignH::Center);
        assert_eq!(spec.v, AlignV::Bottom);
        assert_eq!(unknown, vec!["wibble".to_string()]);
    }

    #[test]
    fn modal_begin_routes_draws_to_modal_layer() {
        let mut layers = LayerStack::new();
        {
            let mut ui = UiContext::with_layers(&mut layers);
            ui.quad(10.0, 10.0, [1.0; 4]); // base
            ui.modal_begin(Rect::new(0.0, 0.0, 50.0, 50.0));
            ui.quad(20.0, 20.0, [1.0; 4]); // routed to modal
            ui.modal_end();
            ui.quad(5.0, 5.0, [1.0; 4]); // base again
        }
        // Translate-only quads record chrome instances: base got 2, modal got 1.
        assert_eq!(layers.base().chrome_instances.len(), 2);
        assert_eq!(layers.layers().len(), 1);
        assert_eq!(layers.layers()[0].list.chrome_instances.len(), 1);
    }

    #[test]
    fn nested_modal_popup_balanced() {
        let mut layers = LayerStack::new();
        {
            let mut ui = UiContext::with_layers(&mut layers);
            ui.modal_begin(Rect::new(0.0, 0.0, 200.0, 200.0));
            ui.popup_begin(Rect::new(50.0, 50.0, 50.0, 50.0));
            ui.popup_end();
            ui.modal_end();
        }
        assert!(!layers.has_active_layer());
        assert_eq!(layers.layers().len(), 2);
    }

    #[test]
    #[should_panic]
    fn modal_begin_on_drawlist_only_panics_in_debug() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.modal_begin(Rect::new(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    #[should_panic(expected = "unbalanced modal_begin")]
    fn unbalanced_modal_drop_panics_in_debug() {
        // Box the LayerStack so we can leak it on panic-unwind to avoid a
        // double-panic from its own balance assertion.
        let mut layers = Box::new(LayerStack::new());
        let layers_ptr: *mut LayerStack = &mut *layers;
        // SAFETY: forget the box to prevent its Drop firing during unwind.
        std::mem::forget(layers);
        // SAFETY: still pointing at valid memory we won't touch after the
        // panic; the test process tears down regardless.
        let layers_ref = unsafe { &mut *layers_ptr };
        let mut ui = UiContext::with_layers(layers_ref);
        ui.modal_begin(Rect::new(0.0, 0.0, 1.0, 1.0));
        // Drop of `ui` fires the debug_assert.
    }

    #[test]
    #[should_panic(expected = "layer kind mismatch")]
    fn popup_begin_followed_by_modal_end_panics_in_debug() {
        let mut layers = LayerStack::new();
        let mut ui = UiContext::with_layers(&mut layers);
        ui.popup_begin(Rect::new(0.0, 0.0, 1.0, 1.0));
        ui.modal_end(); // wrong kind -> debug_assert; layer still popped
    }

    #[test]
    fn font_defaults_to_default_spec() {
        let mut list = DrawList::new();
        let ui = UiContext::new(&mut list);
        assert_eq!(ui.current_font(), FontSpec::default());
    }

    #[test]
    fn font_verbs_mutate_stack_top() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.font(FontHandle("Noto Sans".into()), 24.0);
        ui.bold(true);
        ui.italic(true);
        let f = ui.current_font();
        assert_eq!(f.font, Some(FontHandle("Noto Sans".into())));
        assert_eq!(f.size, 24.0);
        assert_eq!(f.weight, Weight::BOLD);
        assert_eq!(f.style, Style::Italic);
        // Independent setters leave the rest intact.
        ui.font_size(12.0);
        ui.bold(false);
        let f = ui.current_font();
        assert_eq!(f.size, 12.0);
        assert_eq!(f.weight, Weight::NORMAL);
        assert_eq!(f.style, Style::Italic); // unchanged
        assert_eq!(f.font, Some(FontHandle("Noto Sans".into()))); // unchanged
    }

    #[test]
    fn push_pop_scopes_font_too() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.font(FontHandle("Base".into()), 16.0);
        ui.push();
        ui.font(FontHandle("Inner".into()), 32.0);
        ui.bold(true);
        let inner = ui.current_font();
        assert_eq!(inner.font, Some(FontHandle("Inner".into())));
        assert_eq!(inner.size, 32.0);
        assert_eq!(inner.weight, Weight::BOLD);
        ui.pop();
        let outer = ui.current_font();
        assert_eq!(outer.font, Some(FontHandle("Base".into())));
        assert_eq!(outer.size, 16.0);
        assert_eq!(outer.weight, Weight::NORMAL);
    }

    #[test]
    fn text_line_carries_font_stack_attributes() {
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::new(&mut list);
            ui.font(FontHandle("Noto Sans".into()), 28.0);
            ui.bold(true);
            ui.italic(true);
            ui.text_line("hi", [1.0, 0.0, 0.0, 1.0]);
        }
        assert_eq!(list.texts.len(), 1);
        let block = &list.texts[0];
        assert_eq!(block.font, Some(FontHandle("Noto Sans".into())));
        assert_eq!(block.font_size, 28.0);
        assert_eq!(block.weight, Weight::BOLD);
        assert_eq!(block.style, Style::Italic);
    }

    #[test]
    #[should_panic(expected = "unbalanced push/pop pair(s) on the font stack")]
    fn unbalanced_font_stack_drop_panics_in_debug() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        // Push the font stack without a matching pop by leaking an extra entry.
        ui.font_stack.push(FontSpec::default());
        // Re-balance the align stack so only the font-stack assert can fire.
        // (push() also grows align_stack; here we touched font_stack directly,
        // so align_stack is still balanced.)
        drop(ui);
    }

    #[test]
    fn align_call_warns_once_per_unknown_token() {
        // Same unknown token across multiple align() calls should be deduped.
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.align("typo");
        ui.align("typo");
        assert_eq!(ui.warned_align_tokens.len(), 1);
        ui.align("other_typo");
        assert_eq!(ui.warned_align_tokens.len(), 2);
    }

    // ---- Interactive verbs (P0-B) ----

    fn click_at(x: f32, y: f32) -> InputState {
        let mut i = InputState::default();
        i.mouse_x = x;
        i.mouse_y = y;
        i.mouse_down = true;
        i.mouse_clicked = true;
        i
    }

    #[test]
    fn draw_only_ctx_has_no_interactive_fields() {
        let mut list = DrawList::new();
        let ui = UiContext::new(&mut list);
        assert!(ui.input.is_none() && ui.state.is_none() && ui.theme.is_none());
    }

    #[test]
    fn interactive_ctx_sets_all_three_fields() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut state = UiState::new();
        let mut list = DrawList::new();
        let ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
        assert!(ui.input.is_some() && ui.state.is_some() && ui.theme.is_some());
    }

    #[test]
    #[should_panic(expected = "draw-only context")]
    fn interactive_verb_on_drawonly_panics_in_debug() {
        let mut list = DrawList::new();
        let mut ui = UiContext::new(&mut list);
        ui.text_button("nope", None, None);
    }

    #[test]
    fn localize_roundtrips_under_translate() {
        let inv = Affine2::translation(100.0, 50.0).inverse();
        let world = Rect::new(110.0, 60.0, 20.0, 10.0);
        let mut input = InputState::default();
        input.mouse_x = 115.0;
        input.mouse_y = 64.0;
        let (local, li) = UiContext::localize(inv, world, &input);
        assert!(approx(local.x, 10.0) && approx(local.y, 10.0));
        assert!(approx(li.mouse_x, 15.0) && approx(li.mouse_y, 14.0));
        // The mouse is inside the world rect, so it's inside the localized rect.
        assert!(local.contains(li.mouse_x, li.mouse_y));
    }

    #[test]
    fn localize_roundtrips_under_scale() {
        let t = Affine2::scale(2.0, 2.0);
        let inv = t.inverse();
        let world = t.transform_rect_aabb(Rect::new(5.0, 5.0, 10.0, 10.0)); // (10,10,20,20)
        let mut input = InputState::default();
        input.mouse_x = 12.0; // inside world
        input.mouse_y = 12.0;
        let (local, li) = UiContext::localize(inv, world, &input);
        assert!(approx(local.x, 5.0) && approx(local.width, 10.0));
        assert!(local.contains(li.mouse_x, li.mouse_y));
    }

    #[test]
    fn text_button_reports_click_inside_and_not_outside() {
        let theme = Theme::default();
        // Inside.
        {
            let input = click_at(10.0, 10.0);
            let mut state = UiState::new();
            let mut list = DrawList::new();
            let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
            assert!(ui.text_button("OK", Some(100.0), Some(30.0)));
        }
        // Outside.
        {
            let input = click_at(500.0, 500.0);
            let mut state = UiState::new();
            let mut list = DrawList::new();
            let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
            assert!(!ui.text_button("OK", Some(100.0), Some(30.0)));
        }
    }

    #[test]
    fn checkbox_toggles_on_click() {
        let theme = Theme::default();
        let input = click_at(5.0, 5.0); // inside the box (fitted to row height)
        let mut state = UiState::new();
        let mut list = DrawList::new();
        let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
        assert!(ui.checkbox("on", false)); // false -> toggled -> true
    }

    #[test]
    fn checkbox_no_toggle_when_clicked_away() {
        let theme = Theme::default();
        let input = click_at(2000.0, 2000.0);
        let mut state = UiState::new();
        let mut list = DrawList::new();
        let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
        assert!(!ui.checkbox("on", false)); // unchanged
    }

    #[test]
    fn slider_drags_and_updates_value() {
        let theme = Theme::default();
        let mut input = InputState::default();
        input.mouse_x = 95.0; // near the right end of a 100px track
        input.mouse_y = theme.input_height / 2.0;
        input.mouse_down = true;
        input.mouse_clicked = true;
        let mut state = UiState::new();
        let v = {
            let mut list = DrawList::new();
            let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
            ui.slider(7, 0.0, 0.0, 1.0, Some(100.0))
        };
        assert!(v > 0.5, "value should rise toward the right: {v}");
        assert!(state.drag.is_active(7), "the slider should own the drag");
    }

    #[test]
    fn slider_noop_when_clicked_away() {
        let theme = Theme::default();
        let mut input = InputState::default();
        input.mouse_x = 5000.0;
        input.mouse_y = 5000.0;
        input.mouse_down = true;
        input.mouse_clicked = true;
        let mut state = UiState::new();
        let v = {
            let mut list = DrawList::new();
            let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
            ui.slider(7, 0.42, 0.0, 1.0, Some(100.0))
        };
        assert!(approx(v, 0.42));
        assert!(!state.drag.is_active(7));
    }

    #[test]
    fn text_input_edits_caller_buffer() {
        let theme = Theme::default();
        let mut state = UiState::new();
        let mut buffer = String::from("ab");
        // Frame 1: click inside to take focus.
        let input1 = click_at(5.0, 5.0);
        state.begin_frame(&input1, &theme);
        {
            let mut list = DrawList::new();
            let mut ui = UiContext::interactive(&mut list, &input1, &mut state, &theme);
            ui.text_input(1, &mut buffer, "", Some(150.0));
        }
        state.end_frame();
        // Frame 2: type a character while focused.
        let mut input2 = InputState::default();
        input2.text_input = "c".to_string();
        state.begin_frame(&input2, &theme);
        let changed = {
            let mut list = DrawList::new();
            let mut ui = UiContext::interactive(&mut list, &input2, &mut state, &theme);
            ui.text_input(1, &mut buffer, "", Some(150.0))
        };
        state.end_frame();
        assert!(changed, "typing should report a change");
        assert_eq!(buffer.len(), 3);
        assert!(buffer.contains('c'));
    }

    #[test]
    fn verbs_auto_advance_cursor() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut state = UiState::new();
        state.begin_frame(&input, &theme); // seeds item_gap = theme.spacing
        let mut list = DrawList::new();
        let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
        let c0 = ui.cursor();
        ui.text_button("A", Some(100.0), Some(30.0));
        let c1 = ui.cursor();
        assert!(approx(c1[1], c0[1] + 30.0 + theme.spacing));
        ui.text_button("B", Some(100.0), Some(30.0));
        let c2 = ui.cursor();
        assert!(approx(c2[1], c1[1] + 30.0 + theme.spacing));
        // Horizontal cursor unchanged by a vertical stack.
        assert!(approx(c2[0], c0[0]));
    }

    #[test]
    fn text_verb_advances_by_font_size() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut state = UiState::new();
        state.begin_frame(&input, &theme);
        let mut list = DrawList::new();
        {
            let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
            ui.font_size(24.0);
            let c0 = ui.cursor();
            ui.text("hello");
            let c1 = ui.cursor();
            assert!(approx(c1[1], c0[1] + 24.0 + theme.spacing));
        }
        assert_eq!(list.texts.len(), 1);
    }

    #[test]
    fn default_field_width_uses_window_then_fallback() {
        let theme = Theme::default();
        let input = InputState::default();
        let mut state = UiState::new();
        let mut list = DrawList::new();
        let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
        // No window: fallback 200.
        assert!(approx(ui.default_field_width(), 200.0));
        ui.push();
        ui.window_begin(360.0, 100.0, false, false);
        assert!(approx(ui.default_field_width(), 360.0));
        ui.pop();
        assert!(approx(ui.default_field_width(), 200.0));
    }
}
