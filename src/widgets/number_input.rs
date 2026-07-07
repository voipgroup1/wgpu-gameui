//! Number input / spin box: a numeric [`TextInput`] with parse + validation,
//! range clamping, and step controls (+/- buttons, mouse wheel, and Up/Down
//! arrows while focused).
//!
//! The widget wraps a caller-owned [`TextInput`] (so it inherits cursor,
//! selection, and clipboard editing for free) and a `f64` value that is the
//! app's source of truth. Each frame:
//!
//! 1. Step controls (buttons / wheel / arrows) adjust the value first.
//! 2. When the field is **not** focused (or was just stepped), its text is
//!    rewritten from the value — so external changes show and the text
//!    canonicalises the instant editing ends.
//! 3. The text field draws and processes keystrokes.
//! 4. While focused, the text is sanitised to numeric characters and parsed;
//!    a successful parse updates the (clamped) value live. Mid-edit states
//!    that don't parse (`""`, `"-"`, `"1."`) leave the value untouched so the
//!    user can keep typing. `Enter` canonicalises the text from the value.
//!
//! Integer fields are just `decimals == 0` (the default).

use crate::layout::Rect;
#[cfg(feature = "phosphor-icons")]
use crate::StyleKey;

use super::{Button, DrawContext, FocusId, TextInput};

/// Format `value` for display with `decimals` fractional digits. `decimals == 0`
/// renders a plain integer (rounded). Non-finite values render as `"0"`.
fn format_value(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    if decimals == 0 {
        format!("{}", value.round() as i64)
    } else {
        format!("{:.*}", decimals, value)
    }
}

/// Strip `s` down to the characters that can appear in a base-10 number we will
/// `parse::<f64>()`: ASCII digits, a single leading `-`, and (only when
/// `allow_decimal`) a single `.`. Everything else is dropped.
///
/// Returns the cleaned string and the remapped cursor byte index (the count of
/// kept characters at or before the old `cursor`). All kept characters are
/// ASCII, so byte and char offsets coincide in the output.
fn sanitize_numeric(s: &str, cursor: usize, allow_decimal: bool) -> (String, usize) {
    let mut out = String::with_capacity(s.len());
    let mut new_cursor = 0usize;
    let mut seen_dot = false;
    let mut byte = 0usize;
    for c in s.chars() {
        let keep = match c {
            '0'..='9' => true,
            '-' => out.is_empty(),
            '.' => allow_decimal && !seen_dot,
            _ => false,
        };
        if keep {
            if c == '.' {
                seen_dot = true;
            }
            out.push(c);
        }
        byte += c.len_utf8();
        // Everything up to and including this char sits at or before the cursor.
        if byte <= cursor && keep {
            new_cursor = out.len();
        }
    }
    if cursor >= s.len() {
        new_cursor = out.len();
    }
    (out, new_cursor)
}

/// Output from drawing a [`NumberInput`].
pub struct NumberOutput {
    /// The current value after this frame's interaction, always within
    /// `[min, max]`.
    pub value: f64,
    /// Whether `value` differs from the value passed into `draw` this frame
    /// (including a clamp of an out-of-range input). Callers store it back when
    /// `true`.
    pub changed: bool,
}

/// A numeric spin box: editable text plus +/- step buttons, wheel stepping, and
/// arrow-key stepping while focused.
///
/// # Example
/// ```ignore
/// // `ti` is a caller-owned `TextInput` persisted across frames (e.g. in a
/// // `HashMap<FocusId, TextInput>`), `value` a persisted `f64`.
/// let out = NumberInput::new()
///     .with_range(0.0, 100.0)
///     .with_step(5.0)
///     .draw(value, id, &mut ti, rect, &mut ctx);
/// if out.changed {
///     value = out.value;
/// }
/// ```
pub struct NumberInput {
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    step_buttons: bool,
    wheel_step: bool,
    arrow_step: bool,
    /// Optional display formatter overriding the default (`format_value`), e.g.
    /// zero-padding `7` → `"07"` for an hour field.
    formatter: Option<fn(f64) -> String>,
    /// Optional parse-back from edited text. When set, the default numeric
    /// `sanitize`/`parse` path is bypassed and this owns validation.
    parser: Option<fn(&str) -> Option<f64>>,
}

impl Default for NumberInput {
    fn default() -> Self {
        Self::new()
    }
}

impl NumberInput {
    /// An unbounded integer spin box with step `1`.
    pub fn new() -> Self {
        Self {
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            step: 1.0,
            decimals: 0,
            step_buttons: true,
            wheel_step: true,
            arrow_step: true,
            formatter: None,
            parser: None,
        }
    }

    /// Clamp the value to `[min, max]` (inclusive).
    pub fn with_range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Amount added/subtracted per step (button click, wheel notch, arrow key).
    pub fn with_step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Number of fractional digits shown (and whether `.` is an accepted input
    /// character). `0` (default) is an integer field.
    pub fn with_decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    /// Show the +/- step buttons on the right (default `true`). When `false`,
    /// the whole rect is the editable field.
    pub fn with_step_buttons(mut self, on: bool) -> Self {
        self.step_buttons = on;
        self
    }

    /// Step on mouse-wheel while hovered **and** focused (default `true`). The
    /// focus requirement keeps the wheel from hijacking page scrolling unless
    /// the user is actively editing the field.
    pub fn with_wheel_step(mut self, on: bool) -> Self {
        self.wheel_step = on;
        self
    }

    /// Step on Up/Down arrows while focused (default `true`). Single-line
    /// `TextInput` ignores Up/Down, so this never collides with text editing.
    pub fn with_arrow_step(mut self, on: bool) -> Self {
        self.arrow_step = on;
        self
    }

    /// Custom display formatter, overriding the default (`format_value`).
    ///
    /// Use this for fixed-width fields the default formatter can't express —
    /// e.g. zero-padding an hour field: `format!("{:02}", value.round() as i64)`
    /// renders `7` as `"07"`. The formatter is applied whenever the value owns
    /// the text (not editing, just stepped, or on `Enter`).
    ///
    /// Pair with [`with_parser`](Self::with_parser) if the displayed text is not
    /// trivially `f64`-parseable; with only a formatter set, the default
    /// sanitize + `parse::<f64>()` path still applies (which is correct for
    /// zero-padded numeric output like `"07"`).
    pub fn with_formatter(mut self, formatter: fn(f64) -> String) -> Self {
        self.formatter = Some(formatter);
        self
    }

    /// Custom parse-back from edited text. When set, the default numeric
    /// [`sanitize`](sanitize_numeric) + `parse::<f64>()` path is **bypassed** —
    /// this parser owns validation, returning `None` to leave the value
    /// untouched mid-edit (mirroring the default's handling of partial states
    /// like `""` or `"-"`).
    ///
    /// Only needed when the display text is not directly `f64`-parseable (e.g. a
    /// formatter that injects units or separators). For zero-padded numeric
    /// fields (`"07"`), the default parse handles it — formatter alone suffices.
    pub fn with_parser(mut self, parser: fn(&str) -> Option<f64>) -> Self {
        self.parser = Some(parser);
        self
    }

    fn clamp(&self, v: f64) -> f64 {
        v.clamp(self.min, self.max)
    }

    /// Format `value` for display: custom formatter if set, else the default.
    fn display_value(&self, value: f64) -> String {
        match self.formatter {
            Some(f) => f(value),
            None => format_value(value, self.decimals),
        }
    }

    /// Parse edited text back to a value: custom parser if set, else default
    /// `f64` parse. `None` for a partial/non-parseable edit.
    fn parse_value(&self, text: &str) -> Option<f64> {
        match self.parser {
            Some(p) => p(text),
            None => text.parse::<f64>().ok(),
        }
    }

    /// Draw the spin box. `ti` carries the persistent text-edit state; its
    /// geometry is overwritten with the (button-adjusted) field rect each frame.
    pub fn draw(
        &self,
        value: f64,
        id: FocusId,
        ti: &mut TextInput,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> NumberOutput {
        let original = value;
        let mut value = self.clamp(value);
        let allow_decimal = self.decimals > 0;

        // Reserve a square-ish right column for the +/- buttons.
        let btn_w = if self.step_buttons {
            rect.height.min(20.0)
        } else {
            0.0
        };
        let field_w = (rect.width - btn_w).max(0.0);
        let field_rect = Rect::new(rect.x, rect.y, field_w, rect.height);

        // Snapshot the scalar input state we need before any `&mut ctx` borrows
        // (Button::draw_at / ti.draw take `ctx` mutably).
        let was_focused = ctx.focus.is_focused(id);
        let mouse_x = ctx.input.mouse_x;
        let mouse_y = ctx.input.mouse_y;
        let scroll_delta = ctx.input.scroll_delta;
        let scroll_consumed = ctx.input.scroll_consumed;
        let mouse_consumed = ctx.input.mouse_consumed;
        let key_up = ctx.input.key_up;
        let key_down = ctx.input.key_down;
        let enter_pressed = ctx.input.enter_pressed;

        let mut stepped = false;

        // 1a. Wheel stepping (hovered + focused so it doesn't eat page scroll).
        if self.wheel_step
            && was_focused
            && !scroll_consumed
            && scroll_delta != 0.0
            && field_rect.contains(mouse_x, mouse_y)
        {
            value = self.clamp(value + self.step * scroll_delta.signum() as f64);
            stepped = true;
        }

        // 1b. Arrow stepping while focused.
        if self.arrow_step && was_focused {
            if key_up {
                value = self.clamp(value + self.step);
                stepped = true;
            }
            if key_down {
                value = self.clamp(value - self.step);
                stepped = true;
            }
        }

        // 1c. +/- buttons (top = increment, bottom = decrement). Buttons don't
        // register focus, so clicking one keeps the field unfocused and the
        // not-focused branch below reformats the text from the new value.
        if self.step_buttons && btn_w > 0.0 {
            let half = (rect.height / 2.0).floor();
            let up_rect = Rect::new(rect.x + field_w, rect.y, btn_w, half);
            let down_rect = Rect::new(rect.x + field_w, rect.y + half, btn_w, rect.height - half);
            let can_click = !mouse_consumed;
            // Square corners so the steppers sit flush against the field and each
            // other without rounded inner edges.
            #[cfg(feature = "phosphor-icons")]
            {
                // Vector +/- icons centred in each stepper. The Button draws only
                // chrome (empty label); the icon is overlaid on top. The icon
                // placement is em-scaled (1 em → the button's smaller dimension),
                // which already leaves ~25% margin, so no extra inset is needed —
                // and crucially the minus renders as a short bar the width of the
                // plus's arm, not stretched to the (wider) button.
                let s = ctx.styles();
                let tint = s.color(StyleKey::Text);
                if Button::new("").with_radius(0.0).draw(up_rect, ctx) && can_click {
                    value = self.clamp(value + self.step);
                    stepped = true;
                }
                super::Icon::new(crate::render::PhosphorIcon::Plus)
                    .tint(tint)
                    .draw(up_rect, ctx.draw_list);
                if Button::new("").with_radius(0.0).draw(down_rect, ctx) && can_click {
                    value = self.clamp(value - self.step);
                    stepped = true;
                }
                super::Icon::new(crate::render::PhosphorIcon::Minus)
                    .tint(tint)
                    .draw(down_rect, ctx.draw_list);
            }
            // Text fallback when the icon font is compiled out. The label is
            // centred by Button. Use the typographic MINUS SIGN (U+2212), drawn
            // on the same math axis as "+" — the ASCII hyphen-minus sits low and
            // looks bottom-aligned next to the centred plus.
            #[cfg(not(feature = "phosphor-icons"))]
            {
                if Button::new("+").with_radius(0.0).draw(up_rect, ctx) && can_click {
                    value = self.clamp(value + self.step);
                    stepped = true;
                }
                if Button::new("\u{2212}")
                    .with_radius(0.0)
                    .draw(down_rect, ctx)
                    && can_click
                {
                    value = self.clamp(value - self.step);
                    stepped = true;
                }
            }
        }

        // 2. When not editing (or just stepped), the value owns the text.
        if !was_focused || stepped {
            let formatted = self.display_value(value);
            if ti.value != formatted {
                ti.value = formatted;
                ti.cursor_pos = ti.value.len();
                ti.selection_start = None;
            }
        }

        // 3. Draw the editable field at the reserved sub-rect.
        ti.x = field_rect.x;
        ti.y = field_rect.y;
        ti.width = field_rect.width;
        ti.height = field_rect.height;
        ti.multiline = false;
        ti.draw(id, ctx);

        // 4. While focused, sanitise + parse the edited text into the value.
        //    A custom parser owns validation entirely (sanitize is bypassed);
        //    otherwise the default sanitize + parse path applies.
        if ctx.focus.is_focused(id) {
            if self.parser.is_none() {
                let (clean, cur) = sanitize_numeric(&ti.value, ti.cursor_pos, allow_decimal);
                if clean != ti.value {
                    ti.value = clean;
                    ti.cursor_pos = cur.min(ti.value.len());
                    if ti.selection_start.is_some_and(|s| s > ti.value.len()) {
                        ti.selection_start = None;
                    }
                }
            }
            if let Some(parsed) = self.parse_value(&ti.value) {
                value = self.clamp(parsed);
            }
            // Enter canonicalises the displayed text from the (clamped) value.
            if enter_pressed {
                ti.value = self.display_value(value);
                ti.cursor_pos = ti.value.len();
                ti.selection_start = None;
            }
        }

        NumberOutput {
            value,
            changed: value != original,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn draw_number(
        ni: &NumberInput,
        value: f64,
        id: FocusId,
        ti: &mut TextInput,
        rect: Rect,
        focus: &mut FocusState,
        input: &InputState,
    ) -> NumberOutput {
        let mut list = DrawList::new();
        let theme = Theme::default();
        let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
        ni.draw(value, id, ti, rect, &mut ctx)
    }

    fn rect() -> Rect {
        // 120px wide, 24px tall at the origin. With step buttons the field is
        // the left (120 - 24) = 96px; the button column is the rightmost 24px.
        Rect::new(0.0, 0.0, 120.0, 24.0)
    }

    // ---- helpers ----

    #[test]
    fn format_value_integer_and_decimals() {
        assert_eq!(format_value(3.0, 0), "3");
        assert_eq!(format_value(3.4, 0), "3"); // rounds
        assert_eq!(format_value(3.6, 0), "4");
        assert_eq!(format_value(-2.0, 0), "-2");
        assert_eq!(format_value(1.5, 1), "1.5");
        assert_eq!(format_value(1.0, 2), "1.00");
        assert_eq!(format_value(f64::NAN, 0), "0");
        assert_eq!(format_value(f64::INFINITY, 2), "0");
    }

    #[test]
    fn sanitize_strips_non_numeric_keeps_one_dot_and_leading_minus() {
        // Letters dropped; one dot kept; minus only at the front.
        let (s, _) = sanitize_numeric("a1b2.3c", 7, true);
        assert_eq!(s, "12.3");
        let (s, _) = sanitize_numeric("1.2.3", 5, true);
        assert_eq!(s, "1.23", "second dot dropped");
        let (s, _) = sanitize_numeric("--5-", 4, true);
        assert_eq!(s, "-5", "only a single leading minus survives");
        let (s, _) = sanitize_numeric("3-4", 3, true);
        assert_eq!(s, "34", "interior minus dropped");
    }

    #[test]
    fn sanitize_disallows_dot_for_integer_fields() {
        let (s, _) = sanitize_numeric("1.5", 3, false);
        assert_eq!(s, "15");
    }

    #[test]
    fn sanitize_remaps_cursor_past_dropped_chars() {
        // "a12" with cursor after the 'a' (byte 1) → "12" with cursor at 0.
        let (s, cur) = sanitize_numeric("a12", 1, true);
        assert_eq!(s, "12");
        assert_eq!(cur, 0, "cursor moves back over the dropped leading 'a'");
        // cursor at end stays at end.
        let (s, cur) = sanitize_numeric("a12", 3, true);
        assert_eq!(s, "12");
        assert_eq!(cur, 2);
    }

    // ---- interaction ----

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    #[test]
    fn plus_button_increments_and_clamps_to_max() {
        let ni = NumberInput::new().with_range(0.0, 10.0).with_step(3.0);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        // Up button is the top half of the 24px column at x in [96,120), y in [0,12).
        let input = click_at(108.0, 6.0);
        let out = draw_number(&ni, 9.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 10.0, "9 + 3 clamps to max 10");
        assert!(out.changed);
        assert_eq!(ti.value, "10", "field text reformatted from the value");
    }

    #[test]
    fn minus_button_decrements_and_clamps_to_min() {
        let ni = NumberInput::new().with_range(0.0, 10.0).with_step(3.0);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        // Down button is the bottom half: y in [12,24).
        let input = click_at(108.0, 18.0);
        let out = draw_number(&ni, 2.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 0.0, "2 - 3 clamps to min 0");
        assert!(out.changed);
    }

    #[test]
    fn arrow_up_down_step_only_when_focused() {
        let ni = NumberInput::new().with_range(0.0, 100.0).with_step(5.0);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();

        let mut up = InputState::default();
        up.key_up = true;
        // Unfocused: arrows do nothing.
        let out = draw_number(&ni, 50.0, 0, &mut ti, rect(), &mut focus, &up);
        assert_eq!(out.value, 50.0, "arrows are inert when not focused");

        // Focused: arrow up steps by +5, down by -5.
        focus.focus(0);
        let out = draw_number(&ni, 50.0, 0, &mut ti, rect(), &mut focus, &up);
        assert_eq!(out.value, 55.0);

        let mut down = InputState::default();
        down.key_down = true;
        let out = draw_number(&ni, 50.0, 0, &mut ti, rect(), &mut focus, &down);
        assert_eq!(out.value, 45.0);
    }

    #[test]
    fn wheel_steps_when_focused_and_hovered() {
        let ni = NumberInput::new().with_range(0.0, 100.0).with_step(2.0);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        let mut input = InputState::default();
        input.scroll_delta = 1.0; // scroll up
        input.mouse_x = 40.0; // over the field
        input.mouse_y = 12.0;
        let out = draw_number(&ni, 10.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 12.0, "wheel up steps +2");

        input.scroll_delta = -3.0; // any negative magnitude = one notch down
        let out = draw_number(&ni, 10.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 8.0);
    }

    #[test]
    fn typed_text_parses_into_value_and_clamps() {
        let ni = NumberInput::new().with_range(0.0, 100.0);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        // Simulate the user having typed "999" into the focused field.
        ti.value = "999".to_string();
        ti.cursor_pos = 3;
        let input = InputState::default();
        let out = draw_number(&ni, 0.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 100.0, "parsed 999 clamps to max 100");
        assert_eq!(ti.value, "999", "text is NOT reformatted mid-edit");
    }

    #[test]
    fn partial_edits_do_not_change_value() {
        let ni = NumberInput::new().with_decimals(2);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        let input = InputState::default();
        // These don't parse as f64, so the value is held while the user types.
        // (Note "1." *does* parse to 1.0 in Rust, so it is not listed here.)
        for partial in ["", "-", "."] {
            ti.value = partial.to_string();
            ti.cursor_pos = ti.value.len();
            let out = draw_number(&ni, 7.0, 0, &mut ti, rect(), &mut focus, &input);
            assert_eq!(out.value, 7.0, "{partial:?} leaves the value untouched");
            assert_eq!(ti.value, partial, "{partial:?} stays editable");
        }
    }

    #[test]
    fn enter_canonicalises_text_from_value() {
        let ni = NumberInput::new().with_range(0.0, 100.0).with_decimals(1);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        ti.value = "3.50000".to_string();
        ti.cursor_pos = ti.value.len();
        let mut input = InputState::default();
        input.enter_pressed = true;
        let out = draw_number(&ni, 0.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 3.5);
        assert_eq!(ti.value, "3.5", "Enter reformats from the value");
    }

    #[test]
    fn unfocused_field_reflects_external_value() {
        let ni = NumberInput::new().with_decimals(1);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        ti.value = "garbage".to_string();
        let mut focus = FocusState::new(); // not focused
        let input = InputState::default();
        let out = draw_number(&ni, 4.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 4.0);
        assert_eq!(
            ti.value, "4.0",
            "unfocused text is rewritten from the value"
        );
    }

    #[test]
    fn out_of_range_input_is_clamped_and_reported_changed() {
        let ni = NumberInput::new().with_range(0.0, 10.0);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        let input = InputState::default();
        let out = draw_number(&ni, 50.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 10.0);
        assert!(
            out.changed,
            "an out-of-range input clamps and reports changed"
        );
    }

    // ---- formatter / parser ----

    /// Zero-pad an hour field to two digits. The default sanitize + parse path
    /// still applies (formatter-only), so typed `"9"` parses to 9 and displays
    /// `"09"` once the value owns the text.
    fn hour_formatter(v: f64) -> String {
        format!("{:02}", v.round() as i64)
    }

    #[test]
    fn formatter_zero_pads_display_when_not_editing() {
        let ni = NumberInput::new()
            .with_range(0.0, 23.0)
            .with_formatter(hour_formatter);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new(); // not focused
        let input = InputState::default();
        let out = draw_number(&ni, 7.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 7.0);
        assert_eq!(ti.value, "07", "formatter zero-pads single-digit hour");
    }

    #[test]
    fn formatter_keeps_default_parse_for_zero_padded_numeric() {
        // Formatter-only: the default parse path handles "07" → 7.0 fine.
        let ni = NumberInput::new()
            .with_range(0.0, 23.0)
            .with_formatter(hour_formatter);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        ti.value = "09".to_string(); // user typed zero-padded value
        ti.cursor_pos = ti.value.len();
        let input = InputState::default();
        let out = draw_number(&ni, 0.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 9.0, "default parse handles zero-padded input");
    }

    #[test]
    fn formatter_round_trips_via_enter_canonicalization() {
        let ni = NumberInput::new()
            .with_range(0.0, 23.0)
            .with_formatter(hour_formatter);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        ti.value = "9".to_string();
        ti.cursor_pos = ti.value.len();
        let mut input = InputState::default();
        input.enter_pressed = true;
        let out = draw_number(&ni, 0.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 9.0);
        assert_eq!(ti.value, "09", "Enter re-canonicalises through the formatter");
    }

    /// A toy custom format ("3h" / "12h") paired with a matching parser, to
    /// exercise the parser-owns-validation path that bypasses sanitize.
    fn hours_with_unit(v: f64) -> String {
        format!("{}h", v.round() as i64)
    }

    fn parse_hours_with_unit(s: &str) -> Option<f64> {
        let s = s.strip_suffix('h').unwrap_or(s);
        s.trim().parse::<f64>().ok()
    }

    #[test]
    fn custom_parser_bypasses_sanitize_and_owns_validation() {
        // Without the parser, sanitize would strip the 'h' and mangle the text.
        // With the parser, "3h" parses cleanly to 3.0.
        let ni = NumberInput::new()
            .with_range(0.0, 23.0)
            .with_formatter(hours_with_unit)
            .with_parser(parse_hours_with_unit);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        ti.value = "3h".to_string();
        ti.cursor_pos = ti.value.len();
        let input = InputState::default();
        let out = draw_number(&ni, 0.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 3.0, "custom parser extracts the numeric value");
        assert_eq!(ti.value, "3h", "text is not mangled by sanitize");
    }

    #[test]
    fn custom_parser_none_leaves_value_untouched() {
        // A parser returning None (e.g. partial edit) leaves the value as-is,
        // mirroring the default path's handling of "" or "-".
        let ni = NumberInput::new()
            .with_range(0.0, 23.0)
            .with_parser(parse_hours_with_unit);
        let mut ti = TextInput::new(0.0, 0.0, 96.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(0);
        ti.value = "-".to_string(); // doesn't parse
        ti.cursor_pos = ti.value.len();
        let input = InputState::default();
        let out = draw_number(&ni, 5.0, 0, &mut ti, rect(), &mut focus, &input);
        assert_eq!(out.value, 5.0, "parser None holds the value mid-edit");
    }
}
