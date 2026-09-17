//! Declarative settings form — a schema of rows, the caller's values slice,
//! and the widget that draws them together.
//!
//! Define the fields once as a [`SettingsSpec`], keep a parallel
//! [`Vec<SettingValue>`] the caller owns, and let [`SettingsForm`] draw the
//! rows (label left, control right) and mutate the values in place. Sections
//! group rows under `Group` headers (or bare title text via
//! [`bare_headers`](SettingsForm::bare_headers)); every interactive row joins
//! the Tab focus ring, so keyboard/gamepad selection comes free (plus
//! [`ArrowFocusNav`](crate::ArrowFocusNav) for arrow-driven menus).
//!
//! Values are indexed by **field order** — sections take no slot:
//!
//! ```no_run
//! # use wgpu_gameui::{SettingsSpec, SettingValue, SettingsFormState, SettingsForm,
//! #     default_values, Binding, KeyCode};
//! # fn demo(rect: wgpu_gameui::layout::Rect, ctx: &mut wgpu_gameui::DrawContext) {
//! let spec = SettingsSpec::new()
//!     .section("Video")
//!     .toggle("VSync")
//!     .slider("Brightness", 0.0..=1.0)
//!     .section("Controls")
//!     .binding("Jump");
//! let mut values = default_values(&spec, &[(0, true.into()), (1, 0.5.into())]);
//! let mut state = SettingsFormState::new();
//! let out = SettingsForm::new(&spec).draw(&mut values, rect, &mut state, ctx);
//! if out.changed.contains(&0) { /* vsync flipped */ }
//! # let _ = out;
//! # }
//! ```

use crate::layout::Rect;
use crate::widgets::binding::{Binding, KeyCode, PadButton};
use crate::widgets::{DrawContext, Group};
use crate::{Dropdown, DropdownState, DragCapture, DragId, FocusId, Slider, StyleKey, Tone};

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

/// One field in a [`SettingsSpec`]. Built by the spec's
/// [`toggle`](SettingsSpec::toggle) / [`slider`](SettingsSpec::slider) /
/// [`choice`](SettingsSpec::choice) / [`binding`](SettingsSpec::binding) /
/// [`action`](SettingsSpec::action) builders; the caller's `values` slice is
/// indexed by field order (sections are not fields).
#[derive(Debug, Clone)]
pub enum SettingField {
    /// A checkbox row; value `SettingValue::Bool`.
    Toggle(&'static str),
    /// A slider row; value `SettingValue::Number`, clamped to the range.
    Slider(&'static str, std::ops::RangeInclusive<f32>),
    /// A dropdown row; value `SettingValue::Choice(idx)`. Zero options draws
    /// as an inert row.
    Choice(&'static str, &'static [&'static str]),
    /// A rebindable control row; value `SettingValue::Binding`.
    Binding(&'static str),
    /// A button row — takes no value; activates like a menu item.
    Action(&'static str),
}

impl SettingField {
    /// The row label.
    pub fn label(&self) -> &'static str {
        match self {
            SettingField::Toggle(l)
            | SettingField::Slider(l, _)
            | SettingField::Choice(l, _)
            | SettingField::Binding(l)
            | SettingField::Action(l) => l,
        }
    }
}

/// Declarative form description: sections and the fields inside them. Build
/// once and reuse across frames; cloning is cheap.
#[derive(Debug, Clone, Default)]
pub struct SettingsSpec {
    /// Section headers, in order.
    sections: Vec<&'static str>,
    fields: Vec<SettingField>,
    /// Index into `fields` where each section starts (same length as
    /// `sections`).
    section_starts: Vec<usize>,
}

impl SettingsSpec {
    /// An empty spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new section with the given header. Fields added after this
    /// (until the next `section` call) belong to it.
    pub fn section(mut self, title: &'static str) -> Self {
        self.section_starts.push(self.fields.len());
        self.sections.push(title);
        self
    }

    /// Add a checkbox field; its value is `SettingValue::Bool`.
    pub fn toggle(mut self, label: &'static str) -> Self {
        self.fields.push(SettingField::Toggle(label));
        self
    }

    /// Add a slider field over `range`; its value is `SettingValue::Number`.
    pub fn slider(mut self, label: &'static str, range: std::ops::RangeInclusive<f32>) -> Self {
        self.fields.push(SettingField::Slider(label, range));
        self
    }

    /// Add a dropdown field over `options`; its value is
    /// `SettingValue::Choice(selected_index)`.
    pub fn choice(mut self, label: &'static str, options: &'static [&'static str]) -> Self {
        self.fields.push(SettingField::Choice(label, options));
        self
    }

    /// Add a rebindable control field; its value is `SettingValue::Binding`.
    pub fn binding(mut self, label: &'static str) -> Self {
        self.fields.push(SettingField::Binding(label));
        self
    }

    /// Add a button row (no value; activates like a menu item).
    pub fn action(mut self, label: &'static str) -> Self {
        self.fields.push(SettingField::Action(label));
        self
    }

    /// Number of fields (== the length the caller's `values` slice needs).
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// The field at `index` (by field order), if in range.
    pub fn field(&self, index: usize) -> Option<&SettingField> {
        self.fields.get(index)
    }

    /// Iterate all fields in order.
    pub fn fields(&self) -> impl Iterator<Item = &SettingField> {
        self.fields.iter()
    }

    /// `(section_title, field_index_range)` pairs in order. Fields added
    /// before the first `section` call come back under an untitled `""` group,
    /// so no field is ever dropped. Empty sections (a `section` call with no
    /// fields after it) yield empty ranges.
    pub fn section_ranges(&self) -> Vec<(&'static str, std::ops::Range<usize>)> {
        let mut out = Vec::with_capacity(self.sections.len() + 1);
        if self.section_starts.first().copied() != Some(0) && !self.fields.is_empty() {
            let first = self.section_starts.first().copied().unwrap_or(self.fields.len());
            out.push(("", 0..first));
        }
        for (si, &start) in self.section_starts.iter().enumerate() {
            let end = self
                .section_starts
                .get(si + 1)
                .copied()
                .unwrap_or(self.fields.len());
            out.push((self.sections[si], start..end));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// The caller-side value of one spec field, by field order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingValue {
    /// Checkbox state.
    Bool(bool),
    /// Slider value.
    Number(f32),
    /// Dropdown selection (index into the field's options).
    Choice(usize),
    /// Current input binding.
    Binding(Binding),
    /// Actions take no value; present so the slice stays parallel to the spec.
    None,
}

impl From<bool> for SettingValue {
    fn from(b: bool) -> Self {
        SettingValue::Bool(b)
    }
}
impl From<f32> for SettingValue {
    fn from(n: f32) -> Self {
        SettingValue::Number(n)
    }
}
impl From<usize> for SettingValue {
    fn from(i: usize) -> Self {
        SettingValue::Choice(i)
    }
}
impl From<Binding> for SettingValue {
    fn from(b: Binding) -> Self {
        SettingValue::Binding(b)
    }
}

impl SettingValue {
    /// The `Bool`, or `None` for a differently-typed value.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The number, or `None`.
    pub fn as_number(&self) -> Option<f32> {
        match self {
            SettingValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The choice index, or `None`.
    pub fn as_choice(&self) -> Option<usize> {
        match self {
            SettingValue::Choice(i) => Some(*i),
            _ => None,
        }
    }

    /// The binding, or `None`.
    pub fn as_binding(&self) -> Option<Binding> {
        match self {
            SettingValue::Binding(b) => Some(*b),
            _ => None,
        }
    }
}

/// Build a values slice matching `spec` from `(field_index, value)` defaults —
/// unspecified fields get their type's zero value, and mis-typed or
/// out-of-range entries are ignored, so the returned slice always has exactly
/// [`field_count`](SettingsSpec::field_count) entries that fit their fields.
pub fn default_values(
    spec: &SettingsSpec,
    defaults: &[(usize, SettingValue)],
) -> Vec<SettingValue> {
    spec.fields()
        .enumerate()
        .map(|(i, field)| {
            defaults
                .iter()
                .find(|(di, _)| *di == i)
                .map(|(_, v)| *v)
                .filter(|v| value_fits(field, v))
                .unwrap_or_else(|| match field {
                    SettingField::Toggle(_) => SettingValue::Bool(false),
                    SettingField::Slider(_, r) => SettingValue::Number(*r.start()),
                    SettingField::Choice(_, _) => SettingValue::Choice(0),
                    SettingField::Binding(_) => SettingValue::Binding(Binding::Key(KeyCode::Space)),
                    SettingField::Action(_) => SettingValue::None,
                })
        })
        .collect()
}

/// Whether `value` has the type `field` expects.
fn value_fits(field: &SettingField, value: &SettingValue) -> bool {
    matches!(
        (field, value),
        (SettingField::Toggle(_), SettingValue::Bool(_))
            | (SettingField::Slider(_, _), SettingValue::Number(_))
            | (SettingField::Choice(_, _), SettingValue::Choice(_))
            | (SettingField::Binding(_), SettingValue::Binding(_))
            | (SettingField::Action(_), _)
    )
}

// ---------------------------------------------------------------------------
// Form state + output
// ---------------------------------------------------------------------------

/// Caller-owned form state: slider drag capture, dropdown arbitration, the
/// listening binding row, and an ID base so several forms can share a surface.
#[derive(Debug)]
pub struct SettingsFormState {
    /// Shared slider drag capture.
    pub capture: DragCapture,
    /// Per-form ID base: field *i* uses `id_base + i + 1` (0 reserved).
    pub id_base: u64,
    /// The binding row listening for input: `(field_index, grace)`. `grace`
    /// is set when the listener starts and clears after one draw, so the
    /// click/Enter that armed the row can't capture itself.
    pub listening: Option<(usize, bool)>,
    /// Dropdown arbitration shared by all choice rows of this form. Drive it
    /// with [`begin_frame`](DropdownState::begin_frame) at frame-top and
    /// [`end_frame`](DropdownState::end_frame) at frame-bottom — or use the
    /// form's [`push_open_layer`](SettingsForm::push_open_layer) /
    /// [`draw_open_layer`](SettingsForm::draw_open_layer) helpers.
    pub dropdowns: DropdownState,
    /// This frame's gamepad snapshot, for binding capture. Set it once per
    /// frame from the same [`crate::GamepadNav`] the host feeds
    /// [`map_gamepad`](crate::map_gamepad); keyboard-only hosts leave it
    /// `None`.
    pub pad: Option<crate::GamepadNav>,
}

impl Default for SettingsFormState {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsFormState {
    /// Fresh state; IDs derive from base `0x5E77_0000`.
    pub fn new() -> Self {
        Self {
            capture: DragCapture::new(),
            id_base: 0x5E77_0000,
            listening: None,
            dropdowns: DropdownState::default(),
            pad: None,
        }
    }

    /// Distinct ID space for another form on the same surface.
    pub fn with_id_base(mut self, id_base: u64) -> Self {
        self.id_base = id_base;
        self
    }
}

/// What happened this frame. `changed`/`activated` hold **field indices**.
#[derive(Debug, Clone, Default)]
pub struct SettingsFormOutput {
    /// Fields whose value the form wrote this frame (slider drags, toggles,
    /// dropdown picks applied via
    /// [`draw_open_layer`](SettingsForm::draw_open_layer), captured bindings).
    pub changed: Vec<usize>,
    /// The action row that was clicked or confirm-activated, if any.
    pub activated: Option<usize>,
    /// The row currently listening for a key, for callers drawing hints.
    pub listening: Option<usize>,
}

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

/// Draw a [`SettingsSpec`] against a values slice. All persistence lives in
/// the caller-owned [`SettingsFormState`]; the form itself is per-frame
/// configuration borrowing the spec.
pub struct SettingsForm<'spec> {
    spec: &'spec SettingsSpec,
    /// Row height override; `None` → theme `InputHeight`.
    row_height: Option<f32>,
    /// Label column fraction of the row width; `None` sizes to the widest
    /// label per section (clamped 0.15..=0.75).
    label_frac: Option<f32>,
    /// Section headers as `Group` panels (default) or bare title text.
    group_headers: bool,
}

impl<'spec> SettingsForm<'spec> {
    /// A form for `spec`.
    pub fn new(spec: &'spec SettingsSpec) -> Self {
        Self {
            spec,
            row_height: None,
            label_frac: None,
            group_headers: true,
        }
    }

    /// Override the row height (default: theme `InputHeight`).
    pub fn row_height(mut self, px: f32) -> Self {
        self.row_height = Some(px.max(1.0));
        self
    }

    /// Fix the label column at `fraction` of the row width (clamped
    /// 0.15..=0.75). Default: sized to the widest label per section.
    pub fn label_fraction(mut self, fraction: f32) -> Self {
        self.label_frac = Some(fraction.clamp(0.15, 0.75));
        self
    }

    /// Section headers as bare title text instead of `Group` panels — for
    /// forms inside an existing panel where nested panels are too heavy.
    pub fn bare_headers(mut self) -> Self {
        self.group_headers = false;
        self
    }

    /// Height the form needs at `width`, from `ctx`'s theme — for sizing a
    /// scroll viewport or panel around the form before drawing it.
    pub fn intrinsic_height(&self, ctx: &DrawContext) -> f32 {
        let s = ctx.styles();
        let row_h = self.row_height.unwrap_or_else(|| s.scalar(StyleKey::InputHeight));
        let spacing = s.scalar(StyleKey::Spacing);
        let pad = s.scalar(StyleKey::Padding);
        let title = s.scalar(StyleKey::FontSize);
        let mut sections = self
            .spec
            .section_ranges()
            .into_iter()
            .filter(|(_, r)| !r.is_empty())
            .peekable();
        let mut h = 0.0;
        while let Some((_, range)) = sections.next() {
            if sections.peek().is_some() {
                h += spacing; // gap between sections
            }
            let n = range.len() as f32;
            if self.group_headers {
                // Group: header strip (title + 2·pad) + top gap (pad) + rows +
                // bottom gap (pad).
                h += title + 4.0 * pad + n * row_h + (n - 1.0) * spacing;
            } else {
                h += title + spacing + n * row_h + (n - 1.0) * spacing;
            }
        }
        h
    }

    /// Draw the form into `rect`, mutating `values` in place. Dropdown picks
    /// are NOT applied here — a choice row's open list is a popup layer; see
    /// [`draw_open_layer`](Self::draw_open_layer) for the host sequence.
    pub fn draw(
        &self,
        values: &mut [SettingValue],
        rect: Rect,
        state: &mut SettingsFormState,
        ctx: &mut DrawContext,
    ) -> SettingsFormOutput {
        let mut out = SettingsFormOutput::default();

        let s = ctx.styles();
        let row_h = self.row_height.unwrap_or_else(|| s.scalar(StyleKey::InputHeight));
        let spacing = s.scalar(StyleKey::Spacing);
        let pad = s.scalar(StyleKey::Padding);
        let title_size = s.scalar(StyleKey::FontSize);

        let mut y = rect.y;
        let mut sections = self
            .spec
            .section_ranges()
            .into_iter()
            .filter(|(_, r)| !r.is_empty())
            .peekable();
        while let Some((title, range)) = sections.next() {
            if sections.peek().is_some() {
                y += spacing; // gap between sections
            }
            let n = range.len();

            // ---- section block ----
            let block_h = if self.group_headers {
                title_size + 4.0 * pad + n as f32 * row_h + (n - 1) as f32 * spacing
            } else {
                title_size + spacing + n as f32 * row_h + (n - 1) as f32 * spacing
            };
            let block_rect = Rect::new(rect.x, y, rect.width, block_h);

            let inner: Rect;
            if self.group_headers {
                let g = Group::new(title).with_padding(pad);
                // `draw` paints body + header strip and returns the content
                // area (below the header, inset by the padding).
                inner = g.draw(block_rect, ctx.draw_list, &s);
            } else {
                let mut block = s.title_block(title, rect.x, y);
                let (_, lh) = ctx.draw_list.measure_block(&block);
                block.y += (title_size + spacing - lh) * 0.5;
                ctx.draw_list.text(block);
                inner = Rect::new(
                    rect.x,
                    y + title_size + spacing,
                    rect.width,
                    block_h - title_size - spacing,
                );
            }

            // ---- rows ----
            // Label column sized to the widest label in this section.
            let label_frac = self.label_frac.unwrap_or_else(|| {
                let mut w = 0.0f32;
                for f in &self.spec.fields[range.clone()] {
                    let block = s.text_block(f.label(), 0.0, 0.0);
                    let (lw, _) = ctx.draw_list.measure_block(&block);
                    w = w.max(lw);
                }
                ((w + pad * 2.0) / inner.width).clamp(0.15, 0.75)
            });
            let label_w = inner.width * label_frac;
            let control_w = (inner.width - label_w).max(40.0);

            let mut row_y = inner.y;
            for i in range {
                let row = Rect::new(inner.x, row_y, inner.width, row_h);
                row_y += row_h + spacing;
                let field = &self.spec.fields[i];

                // Action rows span the full row (the button *is* the row);
                // everything else splits label | control.
                let (label_rect, control_rect) = if matches!(field, SettingField::Action(_)) {
                    (row, row)
                } else {
                    (
                        Rect::new(row.x, row.y, label_w, row_h),
                        Rect::new(row.x + label_w, row.y, control_w, row_h),
                    )
                };
                if !matches!(field, SettingField::Action(_)) {
                    self.draw_label(field.label(), label_rect, ctx);
                }

                let Some(value) = values.get_mut(i) else {
                    continue;
                };
                let focus_id: FocusId = state.id_base.wrapping_add(i as u64 + 1);
                let mut row_out = RowOut::default();
                self.draw_control(i, field, value, control_rect, state, ctx, focus_id, &mut row_out);
                if row_out.changed {
                    out.changed.push(i);
                }
                if row_out.activated && out.activated.is_none() {
                    out.activated = Some(i);
                }
            }
            y = block_rect.y + block_h; // next section starts below this block
        }

        out.listening = state.listening.map(|(i, _)| i);
        out
    }

    /// Push the popup layer for an open dropdown at frame-top — between
    /// `state.dropdowns.begin_frame(&mut input)` and
    /// `layers.input_for_base(&input)` — so clicks under the open list are
    /// blocked this frame. Thin wrapper over
    /// [`DropdownState::push_open_layer`].
    pub fn push_open_layer(
        &self,
        state: &mut SettingsFormState,
        layers: &mut crate::LayerStack,
    ) -> Option<usize> {
        state.dropdowns.push_open_layer(layers)
    }

    /// Draw the open dropdown list for any choice row (opened this frame or
    /// still open from an earlier one) into its popup layer, applying the pick
    /// to `values`. Returns the field index whose `Choice` value changed.
    ///
    /// Host sequence, mirroring the Dropdown widget's own contract:
    ///
    /// 1. `state.dropdowns.begin_frame(&mut input);` — frame-top, before
    ///    `input_for_base` (an open list claims Escape/arrows/clicks).
    /// 2. `let popup = form.push_open_layer(&mut state, &mut layers);`
    /// 3. `let input = layers.input_for_base(&input);` — build the
    ///    `DrawContext` with this input.
    /// 4. Draw the form (and everything else on the base layer).
    /// 5. `if let Some(i) = form.draw_open_layer(&mut state, popup,
    ///    &mut values, &mut layers, &styles, &input) { … }`
    /// 6. `state.dropdowns.end_frame(&mut focus);` then
    ///    `focus.end_frame(…)`.
    ///
    /// `styles` is the resolver the base pass drew with — build it the same
    /// way (`StyleResolver::with_overlay_opt(theme, overlay)`, or just
    /// `ctx.styles()` while the `DrawContext` is at hand) so the popup list
    /// matches the form's theme, including per-screen overrides.
    pub fn draw_open_layer(
        &self,
        state: &mut SettingsFormState,
        popup: Option<usize>,
        values: &mut [SettingValue],
        layers: &mut crate::LayerStack,
        styles: &crate::StyleResolver,
        input: &crate::InputState,
    ) -> Option<usize> {
        let (id, idx) = state.dropdowns.draw_open_layer(layers, popup, styles, input)?;
        // Map the dropdown id back to its field: field i ⇔ id_base + i + 1.
        let field = (id.wrapping_sub(state.id_base).checked_sub(1)?) as usize;
        match self.spec.fields.get(field) {
            Some(SettingField::Choice(_, _)) => {}
            _ => return None,
        }
        if let Some(SettingValue::Choice(cur)) = values.get_mut(field) {
            if *cur != idx {
                *cur = idx;
                return Some(field);
            }
        }
        None
    }

    // -- label column --------------------------------------------------------
    fn draw_label(&self, label: &str, rect: Rect, ctx: &mut DrawContext) {
        let s = ctx.styles();
        let mut block = s.text_block(label, rect.x, rect.y);
        let (_, lh) = ctx.draw_list.measure_block(&block);
        block.y = rect.y + (rect.height - lh) * 0.5;
        ctx.draw_list.text(block);
    }

    // -- one control; reports its outcome into `row_out` ----------------------
    #[allow(clippy::too_many_arguments)]
    fn draw_control(
        &self,
        index: usize,
        field: &SettingField,
        value: &mut SettingValue,
        rect: Rect,
        state: &mut SettingsFormState,
        ctx: &mut DrawContext,
        focus_id: FocusId,
        row_out: &mut RowOut,
    ) {
        let id: DragId = state.id_base.wrapping_add(index as u64 + 1);
        match field {
            SettingField::Toggle(_) => {
                let on = value.as_bool().unwrap_or(false);
                // Empty label: the row label column already carries the text.
                let clicked = crate::Checkbox::new()
                    .focusable(focus_id)
                    .draw(on, "", rect, ctx);
                if clicked {
                    *value = SettingValue::Bool(!on);
                    row_out.changed = true;
                }
            }
            SettingField::Slider(_, range) => {
                let v = value
                    .as_number()
                    .unwrap_or(*range.start())
                    .clamp(*range.start(), *range.end());
                let out = Slider::new(*range.start(), *range.end())
                    .focusable(focus_id)
                    .with_value_display(false)
                    .draw(v, id, &mut state.capture, rect, ctx);
                if out.changed {
                    *value = SettingValue::Number(out.value);
                    row_out.changed = true;
                }
            }
            SettingField::Choice(_, options) => {
                if options.is_empty() {
                    return;
                }
                let sel = value.as_choice().unwrap_or(0).min(options.len() - 1);
                // The selection lands via draw_open_layer, not here.
                let _ = Dropdown::new(options, sel).draw(id, rect, &mut state.dropdowns, ctx);
            }
            SettingField::Binding(_) => {
                self.draw_binding_control(index, value, rect, state, ctx, focus_id, row_out);
            }
            SettingField::Action(label) => {
                let clicked = crate::Button::new(*label)
                    .tone(Tone::Default)
                    .focusable(focus_id)
                    .draw(rect, ctx);
                if clicked {
                    row_out.activated = true;
                }
            }
        }
    }

    // -- the listening rebind control -----------------------------------------
    #[allow(clippy::too_many_arguments)]
    fn draw_binding_control(
        &self,
        index: usize,
        value: &mut SettingValue,
        rect: Rect,
        state: &mut SettingsFormState,
        ctx: &mut DrawContext,
        focus_id: FocusId,
        row_out: &mut RowOut,
    ) {
        let listening_here = matches!(state.listening, Some((i, _)) if i == index);

        if listening_here {
            let grace = state.listening.map(|(_, g)| g).unwrap_or(false);
            if grace {
                // The arm click/Enter is still in this frame's input — clear
                // the grace so nothing captures itself.
                state.listening = Some((index, false));
            } else if ctx.input.key_escape {
                state.listening = None; // Escape always cancels
            } else if let Some(b) = Self::captured_binding(ctx.input, state.pad.as_ref()) {
                *value = SettingValue::Binding(b);
                state.listening = None;
                row_out.changed = true;
            }
            let _ = crate::Button::new("press a key… (Esc cancels)")
                .tone(Tone::Ghost)
                .focusable(focus_id)
                .draw(rect, ctx);
            return;
        }

        let label = value
            .as_binding()
            .map(|b| b.label())
            .unwrap_or_else(|| "—".to_string());
        let clicked = crate::Button::new(label.as_str())
            .tone(Tone::Default)
            .focusable(focus_id)
            .draw(rect, ctx);
        if clicked {
            // Moving the listener clears any previous one implicitly.
            state.listening = Some((index, true));
        }
    }

    /// The binding satisfied by this frame's input, if any. Scans in device
    /// order (named keys, chars, mouse, pad); `Escape` is excluded — it
    /// cancels listening instead.
    fn captured_binding(input: &crate::InputState, pad: Option<&crate::GamepadNav>) -> Option<Binding> {
        const KEYS: &[KeyCode] = &[
            KeyCode::Tab,
            KeyCode::Space,
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Delete,
        ];
        for k in KEYS {
            if k.pressed(input) {
                return Some(Binding::Key(*k));
            }
        }
        if !input.text_input.is_empty() {
            let c = input
                .text_input
                .chars()
                .find(|c| !c.is_control())
                .unwrap_or_else(|| input.text_input.chars().next().unwrap());
            return Some(Binding::Char {
                c,
                ctrl: input.ctrl_pressed,
                shift: input.shift_pressed,
                alt: input.alt_down,
            });
        }
        if input.mouse_clicked {
            return Some(Binding::MouseLeft);
        }
        if input.mouse_right_clicked {
            return Some(Binding::MouseRight);
        }
        if input.mouse_middle_clicked {
            return Some(Binding::MouseMiddle);
        }
        const PADS: &[PadButton] = &[
            PadButton::DpadUp,
            PadButton::DpadDown,
            PadButton::DpadLeft,
            PadButton::DpadRight,
            PadButton::South,
            PadButton::East,
            PadButton::LeftShoulder,
            PadButton::RightShoulder,
            PadButton::StickUp,
            PadButton::StickDown,
            PadButton::StickLeft,
            PadButton::StickRight,
        ];
        if let Some(pad) = pad {
            for p in PADS {
                if p.in_pad(pad) {
                    return Some(Binding::Pad(*p));
                }
            }
        }
        None
    }
}

/// Per-row outcome scratch passed from `draw` into `draw_control`.
#[derive(Default)]
struct RowOut {
    changed: bool,
    activated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DrawList;
    use crate::FocusState;
    use crate::InputState;
    use crate::style::StyleOverlay;

    const OPTIONS: &[&str] = &["Low", "Medium", "High"];

    fn spec() -> SettingsSpec {
        SettingsSpec::new()
            .section("Video")
            .toggle("VSync")
            .slider("Brightness", 0.0..=1.0)
            .choice("Quality", OPTIONS)
            .section("Controls")
            .binding("Jump")
            .action("Reset to defaults")
    }

    /// Draw one frame with the given input; returns (output, list).
    fn draw_with(
        values: &mut Vec<SettingValue>,
        state: &mut SettingsFormState,
        input: &InputState,
    ) -> (SettingsFormOutput, DrawList) {
        let spec = spec();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let mut overlay = StyleOverlay::new();
        overlay.set_scalar(StyleKey::ButtonHeight, 24.0);
        overlay.set_scalar(StyleKey::InputHeight, 24.0);
        overlay.set_scalar(StyleKey::Spacing, 4.0);
        let mut ctx = crate::DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0)
            .with_style(&overlay);
        let rect = Rect::new(0.0, 0.0, 400.0, 600.0);
        let out = SettingsForm::new(&spec).draw(values, rect, state, &mut ctx);
        (out, list)
    }

    fn fresh_values() -> Vec<SettingValue> {
        default_values(
            &spec(),
            &[(0, true.into()), (1, 0.5.into()), (2, 2usize.into())],
        )
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    /// The row band (top, bottom) of each non-action field, read from the
    /// drawn label texts — no hand-guessed geometry.
    fn label_bands() -> Vec<(usize, f32, f32)> {
        let spec = spec();
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        let (_, list) = draw_with(&mut values, &mut state, &InputState::default());
        let labels: Vec<&str> = spec
            .fields()
            .filter(|f| !matches!(f, SettingField::Action(_)))
            .map(|f| f.label())
            .collect();
        let mut texts = list
            .texts
            .iter()
            .filter(|t| labels.contains(&t.content.as_str()));
        let mut out = Vec::new();
        for (fi, f) in spec.fields().enumerate() {
            if matches!(f, SettingField::Action(_)) {
                continue;
            }
            if let Some(t) = texts.next() {
                out.push((fi, t.y, t.y + t.line_height));
            }
        }
        out
    }

    fn row_center_y(i: usize) -> f32 {
        let (_, top, bottom) = label_bands()
            .into_iter()
            .find(|(fi, _, _)| *fi == i)
            .expect("row band");
        (top + bottom) * 0.5
    }

    #[test]
    fn default_values_match_field_count_and_types() {
        let v = fresh_values();
        assert_eq!(v.len(), 5);
        assert_eq!(v[0], SettingValue::Bool(true));
        assert_eq!(v[1], SettingValue::Number(0.5));
        assert_eq!(v[2], SettingValue::Choice(2));
        assert!(matches!(v[3], SettingValue::Binding(_)));
        assert_eq!(v[4], SettingValue::None);
    }

    #[test]
    fn default_values_reject_mistyped_defaults() {
        // Index 0 is a toggle; a Number default doesn't fit and is dropped.
        let v = default_values(&spec(), &[(0, 0.5.into())]);
        assert_eq!(v[0], SettingValue::Bool(false), "fell back to the zero value");
        // Out-of-range index ignored; slice still complete.
        let v = default_values(&spec(), &[(99, true.into())]);
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn section_ranges_cover_every_field() {
        let spec = spec();
        let ranges = spec.section_ranges();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].0, "Video");
        assert_eq!(ranges[0].1, 0..3);
        assert_eq!(ranges[1].0, "Controls");
        assert_eq!(ranges[1].1, 3..5);
        // Fields before the first section get the "" group.
        let s = SettingsSpec::new().toggle("Orphan").section("S").toggle("A");
        let r = s.section_ranges();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("", 0..1));
        assert_eq!(r[1], ("S", 1..2));
    }

    #[test]
    fn toggle_click_flips_the_value() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        let y = row_center_y(0);
        // The checkbox hit square sits in the control column (labels are at
        // x=4; controls start right of the label split ≈ x=78).
        let (out, _) = draw_with(&mut values, &mut state, &click_at(100.0, y));
        assert!(
            out.changed.contains(&0),
            "toggle flipped; changed = {:?}",
            out.changed
        );
        assert_eq!(values[0], SettingValue::Bool(false), "started true, clicked to false");
    }

    #[test]
    fn binding_row_arms_on_click_and_ignores_the_arm_frame() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();

        let y = row_center_y(3);
        let (out, _) = draw_with(&mut values, &mut state, &click_at(200.0, y));
        assert_eq!(state.listening, Some((3, true)), "row 3 armed (grace set)");
        assert_eq!(out.listening, Some(3));

        // Next frame with no input: grace cleared, still listening, nothing
        // captured.
        let (out, _) = draw_with(&mut values, &mut state, &InputState::default());
        assert_eq!(state.listening, Some((3, false)), "grace cleared");
        assert!(!out.changed.contains(&3));

        // The frame after that, a key press captures.
        let mut input = InputState::default();
        input.key_space = true;
        let (out, _) = draw_with(&mut values, &mut state, &input);
        assert!(out.changed.contains(&3), "space captured");
        assert_eq!(values[3], SettingValue::Binding(Binding::Key(KeyCode::Space)));
        assert_eq!(state.listening, None, "listener cleared");
    }

    #[test]
    fn escape_cancels_listening_without_capturing() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        state.listening = Some((3, false)); // already armed, grace passed

        let mut input = InputState::default();
        input.key_escape = true;
        let (out, _) = draw_with(&mut values, &mut state, &input);
        assert_eq!(state.listening, None, "cancelled");
        assert!(!out.changed.contains(&3), "Escape never becomes a binding");
        assert_eq!(values[3].as_binding(), Some(Binding::Key(KeyCode::Space)));
    }

    #[test]
    fn char_capture_carries_modifiers() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        state.listening = Some((3, false));

        let mut input = InputState::default();
        input.text_input = "s".into();
        input.ctrl_pressed = true;
        let (out, _) = draw_with(&mut values, &mut state, &input);
        assert!(out.changed.contains(&3));
        assert_eq!(
            values[3],
            SettingValue::Binding(Binding::Char { c: 's', ctrl: true, shift: false, alt: false })
        );
    }

    #[test]
    fn pad_capture_uses_the_state_snapshot() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        state.listening = Some((3, false));
        state.pad = Some(crate::GamepadNav {
            dpad_up: true,
            ..Default::default()
        });
        let (out, _) = draw_with(&mut values, &mut state, &InputState::default());
        assert!(out.changed.contains(&3));
        assert_eq!(
            values[3],
            SettingValue::Binding(Binding::Pad(PadButton::DpadUp))
        );
    }

    #[test]
    fn action_row_reports_activated() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        // Action row spans the whole row — its band is one row + spacing below
        // the binding row's.
        let y = row_center_y(3) + 28.0;
        let (out, _) = draw_with(&mut values, &mut state, &click_at(200.0, y));
        assert_eq!(out.activated, Some(4), "action row clicked");
        assert!(out.changed.is_empty(), "actions never write values");
    }

    #[test]
    fn slider_drag_writes_clamped_numbers() {
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        // Press in the slider row's control area.
        let y = row_center_y(1);
        let (out, _) = draw_with(&mut values, &mut state, &click_at(300.0, y));
        assert!(
            state.capture.active().is_some() || out.changed.contains(&1),
            "slider claimed the drag or moved"
        );
    }

    #[test]
    fn intrinsic_height_matches_a_drawn_form() {
        let spec = spec();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let input = InputState::default();
        let mut overlay = StyleOverlay::new();
        overlay.set_scalar(StyleKey::ButtonHeight, 24.0);
        overlay.set_scalar(StyleKey::InputHeight, 24.0);
        overlay.set_scalar(StyleKey::Spacing, 4.0);
        let rect = Rect::new(0.0, 0.0, 400.0, 600.0);
        let h = {
            let ctx =
                crate::DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0)
                    .with_style(&overlay);
            SettingsForm::new(&spec).intrinsic_height(&ctx)
        };
        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx =
            crate::DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0)
                .with_style(&overlay);
        let _ = SettingsForm::new(&spec).draw(&mut values, rect, &mut state, &mut ctx);
        // The deepest drawn chrome bottom must not exceed the announced height.
        let max_y = list
            .chrome_instances
            .iter()
            .map(|c| c.rect[1] + c.rect[3])
            .fold(0.0f32, f32::max);
        assert!(
            max_y < h + 1.0,
            "chrome bottom {} exceeds intrinsic_height {}",
            max_y,
            h
        );
        assert!(h > 100.0, "suspiciously short height {}", h);
    }

    #[test]
    fn theme_override_resizes_rows() {
        // Theming contract: every metric the form reads goes through the
        // context's style resolution, so a menu-look overlay (bigger rows,
        // bigger gaps) changes the drawn form without touching the spec or
        // the widget.
        let mut overlay = StyleOverlay::new();
        overlay.set_scalar(StyleKey::InputHeight, 48.0);
        overlay.set_scalar(StyleKey::Spacing, 12.0);

        let mut values = fresh_values();
        let mut state = SettingsFormState::new();
        let spec = spec();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let input = InputState::default();
        let mut ctx = crate::DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0)
            .with_style(&overlay);
        let out = SettingsForm::new(&spec).draw(
            &mut values,
            Rect::new(0.0, 0.0, 400.0, 600.0),
            &mut state,
            &mut ctx,
        );
        assert!(out.changed.is_empty());
        // The first label sits vertically centered in a 48px row: its baseline
        // is far lower than the default-theme draw's (rows were 24px).
        let vsync_y = list.texts.iter().find(|t| t.content == "VSync").expect("label").y;
        let mut default_list = DrawList::new();
        let mut default_focus = FocusState::new();
        let default_input = InputState::default();
        let mut default_ctx = crate::DrawContext::new(
            &mut default_list,
            &mut default_focus,
            &theme,
            &default_input,
            800.0,
            600.0,
        );
        let _ = SettingsForm::new(&spec).draw(
            &mut values,
            Rect::new(0.0, 0.0, 400.0, 600.0),
            &mut SettingsFormState::new(),
            &mut default_ctx,
        );
        let default_y = default_list
            .texts
            .iter()
            .find(|t| t.content == "VSync")
            .expect("default label")
            .y;
        assert!(
            vsync_y > default_y + 6.0,
            "overlay rows (label y {}) should be taller than default ({} )",
            vsync_y,
            default_y
        );
    }

    #[test]
    fn values_shorter_than_the_spec_do_not_panic() {
        let mut values = vec![SettingValue::Bool(true)];
        let mut state = SettingsFormState::new();
        let (out, _) = draw_with(&mut values, &mut state, &InputState::default());
        assert!(out.changed.is_empty());
    }

    #[test]
    fn open_layer_maps_the_pick_back_to_its_field() {
        let spec = spec();
        let mut state = SettingsFormState::new();
        let mut values = fresh_values();
        // Seed the dropdown as open on field 2 (id_base + 3), button at the
        // real control-column spot.
        state
            .dropdowns
            .open_for_test(state.id_base + 3, Rect::new(78.0, 85.0, 318.0, 24.0), OPTIONS, 1);
        let mut layers = crate::LayerStack::new();
        let theme = crate::Theme::default();
        let styles = crate::StyleResolver::new(&theme);

        // A row click on option 1 (list opens below the button; rows are 28px).
        let mut input = InputState {
            mouse_x: 200.0,
            mouse_y: 85.0 + 24.0 + 2.0 + 28.0 + 14.0, // second list row's band
            mouse_clicked: true,
            ..Default::default()
        };
        state.dropdowns.begin_frame(&mut input);
        let changed = SettingsForm::new(&spec).draw_open_layer(
            &mut state,
            None,
            &mut values,
            &mut layers,
            &styles,
            &input,
        );
        assert_eq!(changed, Some(2), "field 2's choice changed");
        assert_eq!(values[2], SettingValue::Choice(1));
        // Closing on pick: a second frame reports no change.
        state.dropdowns.begin_frame(&mut input);
        let changed = SettingsForm::new(&spec).draw_open_layer(
            &mut state,
            None,
            &mut values,
            &mut layers,
            &styles,
            &input,
        );
        assert_eq!(changed, None);
    }
}
