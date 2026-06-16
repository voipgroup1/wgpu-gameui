//! Text input widget with selection, cursor navigation, and clipboard support.

use std::borrow::Cow;

use crate::InputState;
use crate::StyleKey;
use crate::layout::Rect;
use crate::text::{
    CaretPos, TextBlock, TextSpan, VisualGlyph, WrapMode, byte_at_point, byte_on_adjacent_line,
    caret_for_byte, selection_rects, visual_caret_neighbor,
};

use super::{DrawContext, FocusId};

/// Snap a byte index to the nearest char boundary at or below it, clamped to
/// `s.len()`. Guards the `value[..cursor]` slice in [`compose_preedit`] against
/// a `cursor_pos` that lands inside a multi-byte UTF-8 sequence.
fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Build the inline-composition display for a focused field that is composing
/// an IME preedit.
///
/// Returns `(display_string, spans, caret_byte_in_display)` where:
/// - `display_string` = `value[..cursor] + preedit + value[cursor..]`
/// - `spans` = the before / preedit / after runs, with empty before/after
///   segments omitted so we never emit zero-length spans. Only the preedit span
///   carries `underline` (and `color: None`, so it inherits the block's normal
///   colour — the standard IME convention of normal-coloured underlined text).
/// - `caret_byte_in_display` = `cursor + (preedit_cursor.start | preedit.len())`,
///   i.e. the caret sits inside the preedit where the IME asked, or at its end.
///
/// `cursor_pos` is clamped and snapped to a char boundary; `preedit_cursor`'s
/// start is likewise snapped to a preedit char boundary.
fn compose_preedit(
    value: &str,
    cursor_pos: usize,
    preedit: &str,
    preedit_cursor: Option<[usize; 2]>,
    underline: [f32; 4],
) -> (String, Vec<TextSpan>, usize) {
    let cursor = floor_char_boundary(value, cursor_pos);
    let before = &value[..cursor];
    let after = &value[cursor..];

    let mut display = String::with_capacity(value.len() + preedit.len());
    display.push_str(before);
    display.push_str(preedit);
    display.push_str(after);

    let mut spans = Vec::with_capacity(3);
    if !before.is_empty() {
        spans.push(TextSpan {
            text: before.to_string(),
            color: None,
            underline: None,
        });
    }
    spans.push(TextSpan {
        text: preedit.to_string(),
        color: None,
        underline: Some(underline),
    });
    if !after.is_empty() {
        spans.push(TextSpan {
            text: after.to_string(),
            color: None,
            underline: None,
        });
    }

    // Caret within the preedit: the IME's cursor start, snapped to a boundary,
    // else the preedit end. Display-space byte = cursor + that offset.
    let ime_caret = preedit_cursor
        .map(|[start, _]| floor_char_boundary(preedit, start))
        .unwrap_or(preedit.len());
    let caret_byte = cursor + ime_caret;

    (display, spans, caret_byte)
}

/// Byte-position snapping: find the closest cursor position to `click_x`
/// by binary-searching the cursor-positions table.
fn closest_cursor_pos(positions: &[(usize, f32)], click_x: f32) -> usize {
    if positions.is_empty() {
        return 0;
    }
    // Binary search for the first x > click_x.
    match positions
        .binary_search_by(|&(_, x)| x.partial_cmp(&click_x).unwrap_or(std::cmp::Ordering::Equal))
    {
        Ok(i) => positions[i].0,
        Err(0) => positions[0].0,
        Err(i) if i >= positions.len() => positions[positions.len() - 1].0,
        Err(i) => {
            // Between positions[i-1] and positions[i] — pick the nearer one.
            let (_, x_left) = positions[i - 1];
            let (_, x_right) = positions[i];
            if (click_x - x_left).abs() <= (x_right - click_x).abs() {
                positions[i - 1].0
            } else {
                positions[i].0
            }
        }
    }
}

/// The `(first_byte, last_byte)` of the visual line containing `byte`, from a
/// [`CaretPos`] layout. Used for line-relative Home/End in multiline mode.
/// Falls back to `(byte, byte)` for an empty layout.
fn line_bounds(layout: &[CaretPos], byte: usize) -> (usize, usize) {
    if layout.is_empty() {
        return (byte, byte);
    }
    let line = caret_for_byte(layout, byte).line;
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for p in layout.iter().filter(|p| p.line == line) {
        lo = lo.min(p.byte);
        hi = hi.max(p.byte);
    }
    if lo == usize::MAX {
        (byte, byte)
    } else {
        (lo, hi)
    }
}

/// The `(first_byte, last_byte)` byte range of a specific visual `line`.
fn line_byte_range(layout: &[CaretPos], line: usize) -> (usize, usize) {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for p in layout.iter().filter(|p| p.line == line) {
        lo = lo.min(p.byte);
        hi = hi.max(p.byte);
    }
    if lo == usize::MAX { (0, 0) } else { (lo, hi) }
}

/// The caret x on visual `line` at (or just after) `byte`, falling back to the
/// last caret x on that line.
fn line_x_for_byte(layout: &[CaretPos], line: usize, byte: usize) -> f32 {
    let mut last = 0.0;
    for p in layout.iter().filter(|p| p.line == line) {
        if p.byte >= byte {
            return p.x;
        }
        last = p.x;
    }
    last
}

/// A text input widget with selection, cursor navigation, and optional clipboard integration.
///
/// Supports:
/// - Click-to-position the cursor
/// - Shift+click to extend selection
/// - Arrow keys (Left/Right) to move cursor
/// - Shift+Left/Shift+Right to extend selection
/// - Home/End to jump to start/end
/// - Ctrl+A to select all
/// - Backspace/Delete (selection-aware)
/// - Selection highlight rendering
///
/// # Clipboard
///
/// To enable Ctrl+X/C/V, set [`clipboard_get`](Self::set_clipboard_get) and
/// [`clipboard_set`](Self::set_clipboard_set) closures, or implement the
/// [`Clipboard`](crate::Clipboard) trait and call
/// [`set_clipboard`](Self::set_clipboard).
pub struct TextInput {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub value: String,
    pub placeholder: String,
    /// Byte index of the text cursor (insertion point) within `value`.
    pub cursor_pos: usize,
    /// When set, the text between `selection_start` and `cursor_pos` is selected.
    /// `None` means no active selection.
    pub selection_start: Option<usize>,
    /// Multi-line (textarea) mode: `Enter` inserts a newline, the value wraps to
    /// the field width, Up/Down navigate lines, and the field clips + autoscrolls
    /// vertically to keep the caret visible. Single-line (`false`) is unchanged.
    pub multiline: bool,
    /// Vertical scroll offset in pixels (multiline only). Maintained by autoscroll
    /// in [`draw`](Self::draw) so the caret line stays inside the box.
    pub scroll_offset: f32,
    /// Sticky horizontal column (pixels) for Up/Down navigation. Seeded from the
    /// caret's x on the first vertical move and cleared by any horizontal
    /// move/edit, so a run of Up/Down keeps the original column.
    desired_caret_x: Option<f32>,
    /// Password masking: when `Some(ch)`, every value character is *displayed*
    /// (and measured) as `ch`, while `value` keeps the real plaintext. Masking
    /// implies single-line behaviour (a `multiline` flag is ignored while masked)
    /// and suppresses inline IME preedit so the composition can't leak. `None`
    /// renders normally. Set via [`password`](Self::password) /
    /// [`with_mask`](Self::with_mask).
    pub mask: Option<char>,
    /// Base paragraph direction for the field's content (default
    /// [`Auto`](crate::TextDirection::Auto)). Bidi reordering of mixed scripts is
    /// automatic regardless; this forces the base direction (and hence the
    /// reading-start alignment + caret home edge) for direction-neutral content.
    /// Set via [`with_direction`](Self::with_direction).
    pub direction: crate::TextDirection,
    /// Clipboard getter — returns the current clipboard contents.
    clipboard_get: Option<Box<dyn FnMut() -> String>>,
    /// Clipboard setter — writes text to the clipboard.
    clipboard_set: Option<Box<dyn FnMut(String)>>,
}

impl Default for TextInput {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 24.0,
            value: String::new(),
            placeholder: String::new(),
            cursor_pos: 0,
            selection_start: None,
            multiline: false,
            scroll_offset: 0.0,
            desired_caret_x: None,
            mask: None,
            direction: crate::TextDirection::Auto,
            clipboard_get: None,
            clipboard_set: None,
        }
    }
}

impl TextInput {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            value: String::new(),
            placeholder: String::new(),
            cursor_pos: 0,
            selection_start: None,
            multiline: false,
            scroll_offset: 0.0,
            desired_caret_x: None,
            mask: None,
            direction: crate::TextDirection::Auto,
            clipboard_get: None,
            clipboard_set: None,
        }
    }

    /// Enable multi-line (textarea) mode. See [`multiline`](Self::multiline).
    pub fn with_multiline(mut self, multiline: bool) -> Self {
        self.multiline = multiline;
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self.cursor_pos = self.value.len();
        self
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Make this a password field: characters are displayed as a bullet (`•`)
    /// while [`value`](Self::value) keeps the real text. See [`mask`](Self::mask).
    pub fn password(mut self) -> Self {
        self.mask = Some('•');
        self
    }

    /// Mask displayed characters with `ch` (e.g. `'*'`). See [`mask`](Self::mask).
    pub fn with_mask(mut self, ch: char) -> Self {
        self.mask = Some(ch);
        self
    }

    /// Force the base paragraph direction of the field's content. See
    /// [`direction`](Self::direction). Leaving this `Auto` still edits RTL text
    /// correctly (the base direction is auto-detected from the content); set it
    /// to pin the direction for neutral content or a consistently-RTL field.
    pub fn with_direction(mut self, direction: crate::TextDirection) -> Self {
        self.direction = direction;
        self
    }

    // ---- Masking (display vs. value) ----

    /// The string actually drawn and measured: the plaintext `value`, or — when
    /// masked — one mask glyph per value character. Placeholder text is never
    /// masked (it's handled separately at draw time).
    fn display_value(&self) -> Cow<'_, str> {
        match self.mask {
            Some(ch) => Cow::Owned(std::iter::repeat_n(ch, self.value.chars().count()).collect()),
            None => Cow::Borrowed(&self.value),
        }
    }

    /// Map a byte offset in `value` to the matching byte offset in the displayed
    /// (masked) string. Identity when unmasked. Relies on the 1:1 char mapping
    /// between value and mask glyphs.
    fn value_to_display_byte(&self, byte: usize) -> usize {
        match self.mask {
            None => byte,
            Some(ch) => {
                let clamped = byte.min(self.value.len());
                self.value[..clamped].chars().count() * ch.len_utf8()
            }
        }
    }

    /// Inverse of [`value_to_display_byte`](Self::value_to_display_byte): map a
    /// byte offset in the displayed string back to a `value` byte offset.
    fn display_to_value_byte(&self, display_byte: usize) -> usize {
        match self.mask {
            None => display_byte,
            Some(ch) => {
                let char_index = display_byte / ch.len_utf8();
                self.value
                    .char_indices()
                    .nth(char_index)
                    .map(|(i, _)| i)
                    .unwrap_or(self.value.len())
            }
        }
    }

    /// Set the clipboard getter closure (e.g. `|| arboard::Clipboard::new().unwrap().get_text().unwrap_or_default()`).
    pub fn set_clipboard_get(&mut self, f: impl FnMut() -> String + 'static) {
        self.clipboard_get = Some(Box::new(f));
    }

    /// Set the clipboard setter closure (e.g. `|t| { let _ = arboard::Clipboard::new().unwrap().set_text(t); }`).
    pub fn set_clipboard_set(&mut self, f: impl FnMut(String) + 'static) {
        self.clipboard_set = Some(Box::new(f));
    }

    // ---- Selection helpers ----

    /// The byte range of the current selection `(start, end)` with start <= end.
    /// Returns `None` when there is no selection.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_start.map(|s| {
            if s <= self.cursor_pos {
                (s, self.cursor_pos)
            } else {
                (self.cursor_pos, s)
            }
        })
    }

    /// Return the selected text, if any.
    pub fn selected_text(&self) -> Option<&str> {
        self.selection_range()
            .map(|(start, end)| &self.value[start..end])
    }

    /// Delete the current selection, if any, and return the deleted text.
    /// Returns `None` if there was no selection.
    pub fn delete_selection(&mut self) -> Option<String> {
        let range = self.selection_range()?;
        let deleted = self.value[range.0..range.1].to_string();
        self.value.replace_range(range.0..range.1, "");
        self.cursor_pos = range.0;
        self.selection_start = None;
        Some(deleted)
    }

    /// Delete one character before the cursor (Backspace), or the selection if active.
    fn delete_before_cursor(&mut self) {
        if self.selection_start.is_some() {
            self.delete_selection();
        } else if self.cursor_pos > 0 {
            let prev = self.value[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, c)| (i, c.len_utf8()))
                .unwrap_or((0, 0));
            self.value.replace_range(prev.0..self.cursor_pos, "");
            self.cursor_pos = prev.0;
        }
    }

    /// Delete one character after the cursor (Delete), or the selection if active.
    fn delete_after_cursor(&mut self) {
        if self.selection_start.is_some() {
            self.delete_selection();
        } else if self.cursor_pos < self.value.len() {
            let next = self.value[self.cursor_pos..]
                .char_indices()
                .next()
                .map(|(i, c)| (self.cursor_pos + i, c.len_utf8()))
                .unwrap_or((self.cursor_pos, 0));
            self.value
                .replace_range(self.cursor_pos..next.0 + next.1, "");
        }
    }

    /// Move the cursor one step to the **visual left**.
    ///
    /// When a visual glyph layout is available (`vis` non-empty — focused,
    /// unmasked fields), the caret steps one glyph cell to the left *on screen*
    /// regardless of run direction, so the key matches the physical arrow in
    /// bidi text (macOS/Windows/GTK behaviour). With no layout (masked fields,
    /// or callers driving the widget without geometry) it falls back to a
    /// logical previous-grapheme step — identical to the historical behaviour
    /// and correct for the LTR/masked case where visual == logical.
    fn cursor_left(&mut self, vis: &[VisualGlyph]) {
        if !vis.is_empty() {
            self.cursor_pos = visual_caret_neighbor(vis, self.cursor_pos, -1);
            return;
        }
        if self.cursor_pos > 0 {
            let prev = self.value[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_pos = prev;
        }
    }

    /// Move the cursor one step to the **visual right**. See [`cursor_left`] for
    /// the visual-vs-logical contract.
    ///
    /// [`cursor_left`]: Self::cursor_left
    fn cursor_right(&mut self, vis: &[VisualGlyph]) {
        if !vis.is_empty() {
            self.cursor_pos = visual_caret_neighbor(vis, self.cursor_pos, 1);
            return;
        }
        if self.cursor_pos < self.value.len() {
            let next = self.value[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i)
                .unwrap_or(self.value.len());
            self.cursor_pos = next;
        }
    }

    /// Select all text.
    fn select_all(&mut self) {
        if !self.value.is_empty() {
            self.selection_start = Some(0);
            self.cursor_pos = self.value.len();
        }
    }

    /// Cut: copy selection to clipboard then delete it.
    fn cut(&mut self) {
        let text = self.selected_text().map(|s| s.to_string());
        if let Some(ref text) = text {
            if let Some(ref mut set) = self.clipboard_set {
                set(text.clone());
            }
        }
        if text.is_some() {
            self.delete_selection();
        }
    }

    /// Copy: copy selection to clipboard.
    fn copy(&mut self) {
        let text = self.selected_text().map(|s| s.to_string());
        if let Some(ref text) = text {
            if let Some(ref mut set) = self.clipboard_set {
                set(text.clone());
            }
        }
    }

    /// Paste: replace selection (or insert at cursor) with clipboard contents.
    fn paste(&mut self) {
        let clip = self
            .clipboard_get
            .as_mut()
            .map(|get| get())
            .unwrap_or_default();
        if clip.is_empty() {
            return;
        }
        // Delete any active selection first.
        if self.selection_start.is_some() {
            self.delete_selection();
        }
        self.value.insert_str(self.cursor_pos, &clip);
        self.cursor_pos += clip.len();
        self.selection_start = None;
    }

    // ---- Input processing ----

    /// Process keyboard events for the text field. Called by `draw()` only when
    /// the widget holds focus; callers driving it directly are responsible for
    /// the focus gate.
    ///
    /// This is the single-line / no-layout entry point: vertical navigation and
    /// line-relative Home/End (which need laid-out line geometry) are inert. For
    /// a multi-line field, call [`process_keyboard_with_layout`] with the field's
    /// [`CaretPos`] layout instead (which `draw` does automatically).
    ///
    /// [`process_keyboard_with_layout`]: Self::process_keyboard_with_layout
    pub fn process_keyboard(&mut self, input: &InputState) {
        self.handle_keyboard(input, &[], &[]);
    }

    /// Like [`process_keyboard`](Self::process_keyboard) but with the field's
    /// laid-out caret geometry, enabling multi-line vertical navigation (Up/Down
    /// with a sticky column) and line-relative Home/End. The `layout` reflects
    /// the value as laid out *this* frame (pre-edit); edits re-layout next frame
    /// — a one-frame latency that only affects the Up/Down target line.
    pub fn process_keyboard_with_layout(&mut self, input: &InputState, layout: &[CaretPos]) {
        self.handle_keyboard(input, layout, &[]);
    }

    /// Full keyboard handler. `layout` is the line-aware [`CaretPos`] geometry
    /// (vertical nav / line Home-End); `vis` is the visual-order glyph layout
    /// used for bidi-correct Left/Right caret movement. `draw` supplies both;
    /// the public entry points pass empty slices for the parts they can't build.
    fn handle_keyboard(&mut self, input: &InputState, layout: &[CaretPos], vis: &[VisualGlyph]) {
        // Masking forces single-line semantics (Enter submits, Up/Down inert).
        let multiline = self.multiline && self.mask.is_none();
        // Check for Ctrl shortcuts via text_input.
        // When Ctrl is held, `ReceivedCharacter` may send ASCII control codes
        // (0x01 = Ctrl+A, 0x03 = Ctrl+C, 0x16 = Ctrl+V, 0x18 = Ctrl+X)
        // OR the lowercase letter depending on the platform/backend.
        let ctrl_char: Option<char> = if input.ctrl_pressed {
            input.text_input.chars().next()
        } else {
            None
        };

        // Map ASCII control codes to their letter equivalents.
        let ctrl_letter = ctrl_char.and_then(|c| {
            if ('\x01'..='\x1a').contains(&c) {
                Some((c as u8 + b'a' - 1) as char)
            } else {
                let lower = c.to_ascii_lowercase();
                if ('a'..='z').contains(&lower) {
                    Some(lower)
                } else {
                    None
                }
            }
        });

        // Ctrl+A — Select All
        if ctrl_letter == Some('a') {
            self.select_all();
            return;
        }
        // Ctrl+X — Cut
        if ctrl_letter == Some('x') {
            self.cut();
            return;
        }
        // Ctrl+C — Copy
        if ctrl_letter == Some('c') {
            self.copy();
            return;
        }
        // Ctrl+V — Paste
        if ctrl_letter == Some('v') {
            self.paste();
            return;
        }

        // Enter — insert a newline in multiline mode. (Single-line fields leave
        // `enter_pressed` for the caller to read as a submit signal.)
        if input.enter_pressed && multiline {
            if self.selection_start.is_some() {
                self.delete_selection();
            }
            self.value.insert(self.cursor_pos, '\n');
            self.cursor_pos += 1;
            self.selection_start = None;
            self.desired_caret_x = None;
            return;
        }

        // Up/Down — vertical line navigation (multiline only; needs layout).
        if multiline && (input.key_up || input.key_down) && !layout.is_empty() {
            let dir = if input.key_up { -1 } else { 1 };
            // Seed the sticky column from the current caret on the first vertical
            // move; subsequent moves keep the original column.
            let desired = self
                .desired_caret_x
                .unwrap_or_else(|| caret_for_byte(layout, self.cursor_pos).x);
            let was = self.cursor_pos;
            let target = byte_on_adjacent_line(layout, self.cursor_pos, dir, desired);
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
            } else {
                self.selection_start = None;
            }
            self.cursor_pos = target;
            self.desired_caret_x = Some(desired);
            return;
        }

        // Arrow keys (with possible Shift for selection extension).
        if input.key_left {
            self.desired_caret_x = None;
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
                self.cursor_left(vis);
            } else {
                if self.selection_start.is_some()
                    && self.cursor_pos != self.selection_start.unwrap()
                {
                    // Moving without Shift collapses the selection to its
                    // logical start. (In bidi text the logical start may not be
                    // the visual-left edge; collapsing visually is a documented
                    // v1 limitation alongside boundary affinity.)
                    let (s, _e) = self.selection_range().unwrap();
                    self.cursor_pos = s;
                } else {
                    self.cursor_left(vis);
                }
                self.selection_start = None;
            }
            return;
        }

        if input.key_right {
            self.desired_caret_x = None;
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
                self.cursor_right(vis);
            } else {
                if self.selection_start.is_some()
                    && self.cursor_pos != self.selection_start.unwrap()
                {
                    let (_, e) = self.selection_range().unwrap();
                    self.cursor_pos = e;
                } else {
                    self.cursor_right(vis);
                }
                self.selection_start = None;
            }
            return;
        }

        if input.key_home {
            self.desired_caret_x = None;
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
            } else {
                self.selection_start = None;
            }
            // Multiline: jump to the start of the current visual line; single-line:
            // document start.
            self.cursor_pos = if multiline && !layout.is_empty() {
                line_bounds(layout, self.cursor_pos).0
            } else {
                0
            };
            return;
        }

        if input.key_end {
            self.desired_caret_x = None;
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
            } else {
                self.selection_start = None;
            }
            // Multiline: jump to the end of the current visual line; single-line:
            // document end.
            self.cursor_pos = if multiline && !layout.is_empty() {
                line_bounds(layout, self.cursor_pos).1
            } else {
                self.value.len()
            };
            return;
        }

        // Backspace — delete before cursor or selection.
        if input.backspace_pressed {
            self.desired_caret_x = None;
            self.delete_before_cursor();
            return;
        }

        // Delete — delete after cursor or selection.
        if input.key_delete {
            self.desired_caret_x = None;
            self.delete_after_cursor();
            return;
        }

        // Text insertion.
        if !input.text_input.is_empty() {
            self.desired_caret_x = None;
            // If there's a selection, delete it first.
            if self.selection_start.is_some() {
                self.delete_selection();
            }
            self.value.insert_str(self.cursor_pos, &input.text_input);
            self.cursor_pos += input.text_input.len();
            self.selection_start = None;
        }
    }

    /// Draw the input, handle text changes, and return true if clicked (to request focus).
    ///
    /// Focus is arbitrated by the caller-owned [`FocusState`]: the widget
    /// [`register`](FocusState::register)s itself in the Tab ring each frame and
    /// [`request`](FocusState::request)s focus when clicked. Keyboard input is
    /// processed only while this widget is the focus owner, so multiple inputs
    /// can no longer all type at once. To wire up Ctrl+A/C/V/X, also call
    /// [`set_clipboard_get`](Self::set_clipboard_get) /
    /// [`set_clipboard_set`](Self::set_clipboard_set) before drawing.
    pub fn draw(&mut self, id: FocusId, ctx: &mut DrawContext) -> bool {
        // I-beam cursor while hovering the field (before borrowing ctx fields).
        if ctx.input.is_hovered(self.x, self.y, self.width, self.height) {
            ctx.request_cursor(crate::CursorIcon::Text);
        }
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        let focus = &mut *ctx.focus;
        let input = ctx.input;
        // Join the Tab ring for this frame.
        focus.register(id);

        let hovered = input.is_hovered(self.x, self.y, self.width, self.height);
        let clicked = hovered && input.mouse_clicked;

        // Clicking takes focus immediately (same-frame caret).
        if clicked {
            focus.request(id);
        }
        let focused = focus.is_focused(id);

        // Masking forces single-line behaviour (passwords never wrap), so all the
        // multiline branches below key off this effective flag, not `self.multiline`.
        let multiline = self.multiline && self.mask.is_none();

        // ---- Geometry & layout policy ----
        let padding = s.scalar(StyleKey::Padding);
        let text_x = self.x + padding;
        let text_max_w = self.width - padding * 2.0;
        // Single-line never wraps (a long value overflows + clips); multiline
        // wraps to the field width.
        let wrap = if multiline {
            WrapMode::WordOrGlyph
        } else {
            WrapMode::None
        };
        // Laid-out line height — matches `text_caret_layout` / `TextBlock::with_size`
        // (font_size * 1.25). Single-line keeps font_size for the selection/caret
        // quad height (unchanged behaviour).
        let line_height = if multiline {
            s.scalar(StyleKey::FontSize) * 1.25
        } else {
            s.scalar(StyleKey::FontSize)
        };
        // Top-aligned for multiline, vertically centred for single-line. Unlike
        // static labels (which pick the band from their text), an editable field
        // must centre on a *content-independent* band so the text doesn't hop up
        // and down as the user types capitals vs lowercase — pin it to the
        // x-height band (the common typing case).
        let text_top = if multiline {
            self.y + padding
        } else {
            let m = list.font_vmetrics(s.theme().font.as_ref());
            let font_size = s.scalar(StyleKey::FontSize);
            self.y + self.height / 2.0 - font_size * m.baseline_ratio
                + font_size * m.x_ratio / 2.0
        };
        let inner_rect = Rect::new(
            text_x,
            self.y + padding,
            text_max_w,
            (self.height - padding * 2.0).max(0.0),
        );

        // Caret layout for the *current* value (multiline+focused only), built
        // BEFORE keyboard/click so vertical nav, line Home/End, and click hit
        // testing see this frame's geometry (one-frame edit latency — see
        // `process_keyboard_with_layout`).
        let nav_layout: Vec<CaretPos> = if multiline && focused {
            list.text_caret_layout(
                &self.value,
                s.scalar(StyleKey::FontSize),
                Some(text_max_w),
                wrap,
                self.direction,
            )
        } else {
            Vec::new()
        };

        // Visual-order glyph layout for bidi-correct Left/Right caret movement,
        // built BEFORE keyboard handling (same one-frame-latency contract as
        // `nav_layout`). Only built for focused, single-line, *unmasked* fields:
        // masked bullets are LTR (visual == logical, so the logical fallback in
        // `cursor_left`/`cursor_right` is exact); multiline keeps the existing
        // logical movement + left-edge caret rendering so it stays coherent
        // (single-line is the bidi-precise editing surface — multiline RTL caret
        // edge-precision is a documented limitation, like boundary affinity).
        let move_vis: Vec<VisualGlyph> = if focused
            && !multiline
            && self.mask.is_none()
            && !self.value.is_empty()
        {
            list.text_visual_layout(
                &self.value,
                s.scalar(StyleKey::FontSize),
                Some(text_max_w),
                wrap,
                self.direction,
            )
        } else {
            Vec::new()
        };

        // ---- Click-to-position ----
        if clicked && focused {
            if multiline {
                // Hit-test against the laid-out lines, accounting for the scroll
                // offset and top-left text origin.
                let local_x = input.mouse_x - text_x;
                let local_y = input.mouse_y - text_top + self.scroll_offset;
                let byte_pos = byte_at_point(&nav_layout, local_x, local_y);
                if input.shift_pressed {
                    if self.selection_start.is_none() {
                        self.selection_start = Some(self.cursor_pos);
                    }
                } else {
                    self.selection_start = None;
                }
                self.cursor_pos = byte_pos;
                self.desired_caret_x = None;
            } else {
                let click_x = input.mouse_x;
                let text_left = self.x + padding;
                let text_right = self.x + self.width - padding;

                if click_x >= text_left && click_x <= text_right {
                    let local_x = click_x - text_left;
                    // Measure against the masked display, then map the picked
                    // display byte back to a real value byte.
                    let display = self.display_value();
                    let positions = list.text_cursor_positions(
                        &display,
                        s.scalar(StyleKey::FontSize),
                        Some(text_max_w),
                    );
                    let byte_pos = self.display_to_value_byte(closest_cursor_pos(&positions, local_x));
                    if input.shift_pressed {
                        // Extend selection.
                        if self.selection_start.is_none() {
                            self.selection_start = Some(self.cursor_pos);
                        }
                    } else {
                        self.selection_start = None;
                    }
                    self.cursor_pos = byte_pos;
                } else if click_x < text_left {
                    self.cursor_pos = 0;
                    if !input.shift_pressed {
                        self.selection_start = None;
                    }
                } else {
                    self.cursor_pos = self.value.len();
                    if !input.shift_pressed {
                        self.selection_start = None;
                    }
                }
            }
        }

        // Process keyboard events only while focused.
        if focused {
            self.handle_keyboard(input, &nav_layout, &move_vis);
        }

        // A live IME composition is shown inline (underlined) and supersedes the
        // value selection; commit reuses the normal `text_input` path so the
        // preedit itself is display-only and never mutates `self.value`. Masked
        // (password) fields suppress the inline preedit so it can't leak.
        let composing = focused && !input.preedit.is_empty() && self.mask.is_none();

        // ---- Draw background + border ----
        // Honor `theme.border_radius` so text inputs match the other inputs
        // (Dropdown/Button/Checkbox); `rounded_rect` falls back to a hard
        // rectangle when the radius is 0, so a rectangular theme still works.
        let frame = Rect::new(self.x, self.y, self.width, self.height);
        let radius = s.scalar(StyleKey::BorderRadius);
        let border = s.scalar(StyleKey::BorderWidth);
        let border_color = if focused {
            s.color(StyleKey::InputFocusBorder)
        } else if hovered {
            s.color(StyleKey::Accent)
        } else {
            s.color(StyleKey::InputBorder)
        };

        list.rounded_rect(frame, radius, s.color(StyleKey::InputBackground));
        list.rounded_rect_outline(frame, radius, border, border_color);

        // Render-time caret layout (multiline+focused), reflecting any edits made
        // this frame — used for autoscroll, per-line selection, and the caret.
        let render_layout: Vec<CaretPos> = if multiline && focused {
            list.text_caret_layout(
                &self.value,
                s.scalar(StyleKey::FontSize),
                Some(text_max_w),
                wrap,
                self.direction,
            )
        } else {
            Vec::new()
        };

        // ---- Autoscroll to keep the caret line visible (multiline) ----
        if multiline && focused {
            let inner_h = (self.height - padding * 2.0).max(0.0);
            let caret = caret_for_byte(&render_layout, self.cursor_pos);
            let caret_top = caret.line_top;
            let caret_h = if caret.line_height > 0.0 {
                caret.line_height
            } else {
                line_height
            };
            let caret_bottom = caret_top + caret_h;
            if caret_top < self.scroll_offset {
                self.scroll_offset = caret_top;
            } else if caret_bottom > self.scroll_offset + inner_h {
                self.scroll_offset = caret_bottom - inner_h;
            }
            if self.scroll_offset < 0.0 {
                self.scroll_offset = 0.0;
            }
        }

        // ---- Resolve drawn text ----
        // The drawn string is the masked display (plaintext when unmasked); the
        // placeholder is shown unmasked when the value is empty. Built after the
        // autoscroll mutation above so the borrow it holds on `self` doesn't clash.
        let display = self.display_value();
        let (text_content, text_color): (&str, [f32; 4]) = if self.value.is_empty() {
            (&self.placeholder, s.color(StyleKey::TextDim))
        } else {
            (&display, s.color(StyleKey::Text))
        };

        // Multiline content is clipped to the inner box (so wrapped + scrolled
        // lines, the selection, and the caret never spill past the field).
        if multiline {
            list.push_clip(inner_rect);
        }

        // The vertical shift applied to text/selection/caret in multiline mode.
        let scroll = if multiline {
            self.scroll_offset
        } else {
            0.0
        };

        // Render-time visual-order glyph layout of the *display* string for
        // single-line fields, shared by the bidi selection fill and the
        // edge-correct caret below. Masked bullets lay out LTR, so this collapses
        // to the historical single-span / left-edge behaviour for passwords.
        let caret_vis: Vec<VisualGlyph> =
            if focused && !composing && !multiline && !self.value.is_empty() {
                list.text_visual_layout(
                    &display,
                    s.scalar(StyleKey::FontSize),
                    Some(text_max_w),
                    wrap,
                    self.direction,
                )
            } else {
                Vec::new()
            };

        // ---- Draw selection highlight ----
        if focused && !composing && !self.value.is_empty() {
            if let Some((sel_start, sel_end)) = self.selection_range() {
                if sel_start < sel_end {
                    if multiline {
                        let start_line = caret_for_byte(&render_layout, sel_start).line;
                        let end_line = caret_for_byte(&render_layout, sel_end).line;
                        for line in start_line..=end_line {
                            let (line_lo, line_hi) = line_byte_range(&render_layout, line);
                            let a = sel_start.max(line_lo);
                            let b = sel_end.min(line_hi);
                            if b < a {
                                continue;
                            }
                            let x1 = line_x_for_byte(&render_layout, line, a);
                            // Selection that continues onto the next line fills to
                            // the inner edge (shows the newline is selected).
                            let x2 = if sel_end > line_hi {
                                text_max_w
                            } else {
                                line_x_for_byte(&render_layout, line, b)
                            };
                            let lt = render_layout
                                .iter()
                                .find(|p| p.line == line)
                                .map(|p| p.line_top)
                                .unwrap_or(0.0);
                            list.quad(
                                text_x + x1,
                                text_top - scroll + lt,
                                (x2 - x1).max(1.0),
                                line_height,
                                s.color(StyleKey::Accent),
                            );
                        }
                    } else {
                        // Single-line: resolve bidi-correct visual rectangles.
                        // A logically-contiguous selection can map to several
                        // disjoint visual spans across an LTR↔RTL boundary, so
                        // we draw one quad per returned rect. Selection bytes are
                        // mapped to display bytes first (identity when unmasked;
                        // masked bullets are LTR so this yields a single span).
                        let ds = self.value_to_display_byte(sel_start);
                        let de = self.value_to_display_byte(sel_end);
                        for r in selection_rects(&caret_vis, ds, de) {
                            list.quad(
                                text_x + r.x,
                                text_top,
                                r.w.max(1.0),
                                line_height,
                                s.color(StyleKey::Accent),
                            );
                        }
                    }
                }
            }
        }

        // A composing field splices the underlined preedit into the value at
        // the caret and renders it as spans; otherwise it's the plain value (or
        // placeholder). `composed` is owned, so it is reused for the caret below.
        let composed = if composing {
            Some(compose_preedit(
                &self.value,
                self.cursor_pos,
                &input.preedit,
                input.preedit_cursor,
                s.color(StyleKey::Text), // underline colour matches the text
            ))
        } else {
            None
        };

        let block_y = text_top - scroll;
        if let Some((_display, spans, _caret)) = &composed {
            let text_c = s.color(StyleKey::Text);
            let text = TextBlock::new("", text_x, block_y)
                .with_size(s.scalar(StyleKey::FontSize))
                .with_wrap(wrap)
                .with_color(
                    (text_c[0] * 255.0) as u8,
                    (text_c[1] * 255.0) as u8,
                    (text_c[2] * 255.0) as u8,
                )
                .with_max_width(text_max_w)
                .with_direction(self.direction)
                .with_spans(spans.clone());
            list.text(text);
        } else {
            let text = TextBlock::new(text_content, text_x, block_y)
                .with_size(s.scalar(StyleKey::FontSize))
                .with_wrap(wrap)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_max_width(text_max_w)
                .with_direction(self.direction);
            list.text(text);
        }

        // ---- Draw cursor ----
        if focused {
            if multiline && composed.is_none() {
                let caret = caret_for_byte(&render_layout, self.cursor_pos);
                let caret_h = if caret.line_height > 0.0 {
                    caret.line_height
                } else {
                    line_height
                };
                let caret_x = text_x + caret.x;
                let caret_y = text_top - scroll + caret.line_top;
                list.quad(caret_x, caret_y, 1.5, caret_h, s.color(StyleKey::Text));
                // Tell the windowing layer a text field is focused (so it can
                // enable IME) and where to anchor the IME candidate window.
                focus.request_ime(Rect::new(caret_x, caret_y, 1.5, caret_h));
            } else {
                let cursor_x = if let Some((display, _spans, caret_byte)) = &composed {
                    // Caret position is measured on the composed display string so
                    // it sits inside the preedit where the IME asked.
                    let positions = list.text_cursor_positions(
                        display,
                        s.scalar(StyleKey::FontSize),
                        Some(text_max_w),
                    );
                    let offset = positions
                        .iter()
                        .find(|&&(i, _)| i >= *caret_byte)
                        .map(|&(_, x)| x)
                        .unwrap_or(positions.last().map(|&(_, x)| x).unwrap_or(0.0));
                    text_x + offset
                } else if self.value.is_empty() {
                    text_x
                } else {
                    // Edge-correct caret position from the visual layout: in RTL
                    // (or bidi) content the caret for a byte sits on the cell edge
                    // where that byte logically begins — the *right* edge of an
                    // RTL cell — which the left-edge `text_cursor_positions` table
                    // gets wrong. `caret_vis` is over the display, so the value
                    // byte is mapped first (identity unmasked; masked = LTR).
                    let caret_disp = self.value_to_display_byte(self.cursor_pos);
                    crate::text::visual_caret_pos(&caret_vis, caret_disp)
                        .map(|c| text_x + c.x)
                        .unwrap_or(text_x)
                };
                list.quad(cursor_x, block_y, 1.5, line_height, s.color(StyleKey::Text));
                // See the multiline branch: declare IME focus + caret anchor.
                focus.request_ime(Rect::new(cursor_x, block_y, 1.5, line_height));
            }
        }

        if multiline {
            list.pop_clip();
        }

        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn make_input(val: &str) -> TextInput {
        TextInput::new(0.0, 0.0, 200.0, 24.0).with_value(val.to_string())
    }

    /// Draw a text input through a throwaway `DrawContext`. Scoping the context
    /// to this call drops its `&mut FocusState` borrow before the caller asserts
    /// on focus state.
    fn draw_input(
        ti: &mut TextInput,
        id: FocusId,
        focus: &mut FocusState,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
    ) -> bool {
        let mut ctx = DrawContext::new(list, focus, theme, input, 800.0, 600.0);
        ti.draw(id, &mut ctx)
    }

    fn fake_input() -> InputState {
        InputState::default()
    }

    #[test]
    fn focused_field_requests_ime_unfocused_does_not() {
        let mut ti = TextInput::new(5.0, 6.0, 120.0, 24.0).with_value("hi".to_string());
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = InputState::default();

        // Unfocused frame: the field must not request IME.
        focus.begin_frame(&input);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &input);
        assert_eq!(
            focus.ime_request(),
            None,
            "an unfocused text field must not enable IME"
        );

        // Focused frame: it requests IME with a caret rect anchored in the field.
        focus.focus(0);
        focus.begin_frame(&input);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &input);
        let req = focus
            .ime_request()
            .expect("a focused text field must request IME");
        assert!(req.x >= 5.0, "caret x within the field");
        assert!(req.y >= 6.0, "caret y within the field");
        assert!(req.height > 0.0, "caret has a height for IME anchoring");
    }

    #[test]
    fn hover_requests_text_cursor() {
        use crate::{CursorIcon, CursorState};
        let mut ti = TextInput::new(5.0, 6.0, 120.0, 24.0).with_value("hi".to_string());
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();

        // Pointer over the field → I-beam.
        let hover = InputState {
            mouse_x: 30.0,
            mouse_y: 12.0,
            ..Default::default()
        };
        let mut cursor = CursorState::new();
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &hover, 800.0, 600.0)
                .with_cursor(&mut cursor);
            ti.draw(0, &mut ctx);
        }
        assert_eq!(cursor.resolve(), CursorIcon::Text);

        // Pointer elsewhere → no request.
        let away = InputState {
            mouse_x: 500.0,
            mouse_y: 500.0,
            ..Default::default()
        };
        let mut cursor = CursorState::new();
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &away, 800.0, 600.0)
                .with_cursor(&mut cursor);
            ti.draw(0, &mut ctx);
        }
        assert_eq!(cursor.resolve(), CursorIcon::Default);
    }

    #[test]
    fn multiline_focused_field_requests_ime() {
        let mut ti = TextInput::new(0.0, 0.0, 120.0, 80.0)
            .with_multiline(true)
            .with_value("line one\nline two".to_string());
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = InputState::default();

        focus.focus(0);
        focus.begin_frame(&input);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &input);
        assert!(
            focus.ime_request().is_some(),
            "a focused multiline field requests IME"
        );
    }

    // ---- Password / masking ----

    #[test]
    fn password_builder_sets_bullet_mask() {
        assert_eq!(TextInput::new(0.0, 0.0, 100.0, 24.0).password().mask, Some('•'));
        assert_eq!(
            TextInput::new(0.0, 0.0, 100.0, 24.0).with_mask('*').mask,
            Some('*')
        );
        assert_eq!(make_input("x").mask, None, "unmasked by default");
    }

    #[test]
    fn display_value_masks_per_char_but_value_is_plaintext() {
        let ti = make_input("secret").password();
        assert_eq!(ti.value, "secret", "value stays plaintext");
        assert_eq!(&*ti.display_value(), "••••••", "display is one bullet per char");
        // Unmasked borrows the value unchanged.
        let plain = make_input("secret");
        assert_eq!(&*plain.display_value(), "secret");
    }

    #[test]
    fn masked_byte_mapping_roundtrips_multibyte() {
        // "aé☃" — a=1 byte, é=2 bytes, ☃=3 bytes (value len 6); 3 chars.
        let ti = make_input("aé☃").with_mask('*'); // '*' is 1 byte
        // value byte → display byte (char index * 1).
        assert_eq!(ti.value_to_display_byte(0), 0);
        assert_eq!(ti.value_to_display_byte(1), 1); // after 'a' → char 1
        assert_eq!(ti.value_to_display_byte(3), 2); // after 'é' → char 2
        assert_eq!(ti.value_to_display_byte(6), 3); // end → char 3
        // display byte → value byte.
        assert_eq!(ti.display_to_value_byte(0), 0);
        assert_eq!(ti.display_to_value_byte(1), 1);
        assert_eq!(ti.display_to_value_byte(2), 3);
        assert_eq!(ti.display_to_value_byte(3), 6);
    }

    #[test]
    fn masked_byte_mapping_with_multibyte_mask_glyph() {
        // '•' is 3 bytes; two value chars → display "••" (6 bytes).
        let ti = make_input("hi").password();
        assert_eq!(ti.value_to_display_byte(1), 3, "char 1 → 1 * 3 bytes");
        assert_eq!(ti.value_to_display_byte(2), 6);
        assert_eq!(ti.display_to_value_byte(3), 1);
        assert_eq!(ti.display_to_value_byte(6), 2);
    }

    #[test]
    fn masked_field_renders_bullets_not_plaintext() {
        let mut ti = make_input("hunter2").password();
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();
        let input = InputState::default();
        focus.begin_frame(&input);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &input);
        let drawn = &list.texts.last().expect("a text block was drawn").content;
        assert_eq!(drawn, "•••••••", "renders one bullet per char");
        assert!(!drawn.contains("hunter2"), "plaintext must never be drawn");
    }

    #[test]
    fn masked_field_forces_single_line_enter() {
        // multiline + mask: Enter must NOT insert a newline (passwords are single-line).
        let mut ti = make_input("pw").with_multiline(true).password();
        ti.cursor_pos = 1;
        let mut ev = fake_input();
        ev.enter_pressed = true;
        ti.process_keyboard(&ev);
        assert_eq!(ti.value, "pw", "masked field ignores multiline newline insertion");
    }

    #[test]
    fn masked_editing_operates_on_plaintext() {
        // Typing and backspace mutate the real value, not the mask.
        let mut ti = make_input("").password();
        let mut ev = fake_input();
        ev.text_input = "ab".to_string();
        ti.process_keyboard(&ev);
        assert_eq!(ti.value, "ab");
        let mut bs = fake_input();
        bs.backspace_pressed = true;
        ti.process_keyboard(&bs);
        assert_eq!(ti.value, "a");
        assert_eq!(&*ti.display_value(), "•");
    }

    #[test]
    fn initial_cursor_at_end() {
        let ti = make_input("hello");
        assert_eq!(ti.cursor_pos, 5);
        assert_eq!(ti.selection_start, None);
    }

    #[test]
    fn empty_input_cursor_at_zero() {
        let ti = make_input("");
        assert_eq!(ti.cursor_pos, 0);
    }

    #[test]
    fn delete_selection_works() {
        let mut ti = make_input("abcdef");
        ti.selection_start = Some(1);
        ti.cursor_pos = 4;
        let deleted = ti.delete_selection();
        assert_eq!(deleted.as_deref(), Some("bcd"));
        assert_eq!(ti.value, "aef");
        assert_eq!(ti.cursor_pos, 1);
        assert_eq!(ti.selection_start, None);
    }

    #[test]
    fn backspace_deletes_before_cursor() {
        let mut ti = make_input("abc");
        ti.cursor_pos = 2;
        ti.delete_before_cursor();
        assert_eq!(ti.value, "ac");
        assert_eq!(ti.cursor_pos, 1);
    }

    #[test]
    fn backspace_deletes_selection_first() {
        let mut ti = make_input("abcdef");
        ti.selection_start = Some(1);
        ti.cursor_pos = 4;
        ti.delete_before_cursor();
        assert_eq!(ti.value, "aef");
        assert_eq!(ti.cursor_pos, 1);
    }

    #[test]
    fn delete_after_cursor_works() {
        let mut ti = make_input("abc");
        ti.cursor_pos = 1;
        ti.delete_after_cursor();
        assert_eq!(ti.value, "ac");
        assert_eq!(ti.cursor_pos, 1);
    }

    #[test]
    fn cursor_left_right() {
        let mut ti = make_input("hi");
        assert_eq!(ti.cursor_pos, 2);
        ti.cursor_left(&[]);
        assert_eq!(ti.cursor_pos, 1);
        ti.cursor_left(&[]);
        assert_eq!(ti.cursor_pos, 0);
        ti.cursor_left(&[]); // no-op
        assert_eq!(ti.cursor_pos, 0);
        ti.cursor_right(&[]);
        assert_eq!(ti.cursor_pos, 1);
        ti.cursor_right(&[]);
        assert_eq!(ti.cursor_pos, 2);
        ti.cursor_right(&[]); // no-op
        assert_eq!(ti.cursor_pos, 2);
    }

    #[test]
    fn home_end() {
        // Single-line Home/End jump to document start/end via the keyboard path.
        let mut ti = make_input("hello world");
        ti.cursor_pos = 5;
        let mut home = fake_input();
        home.key_home = true;
        ti.process_keyboard(&home);
        assert_eq!(ti.cursor_pos, 0);
        let mut end = fake_input();
        end.key_end = true;
        ti.process_keyboard(&end);
        assert_eq!(ti.cursor_pos, 11);
    }

    #[test]
    fn select_all_works() {
        let mut ti = make_input("hello");
        ti.select_all();
        assert_eq!(ti.selection_start, Some(0));
        assert_eq!(ti.cursor_pos, 5);
        assert_eq!(ti.selected_text(), Some("hello"));
    }

    #[test]
    fn selection_range_ordered() {
        let mut ti = make_input("abcdef");
        ti.selection_start = Some(4);
        ti.cursor_pos = 1;
        let range = ti.selection_range();
        assert_eq!(range, Some((1, 4)));
    }

    #[test]
    fn text_insertion_deletes_selection() {
        let mut ti = make_input("hello");
        ti.selection_start = Some(1);
        ti.cursor_pos = 4;
        let mut input = fake_input();
        input.text_input = "XY".to_string();
        ti.process_keyboard(&input);
        assert_eq!(ti.value, "hXYo");
        assert_eq!(ti.cursor_pos, 3);
        assert_eq!(ti.selection_start, None);
    }

    #[test]
    fn ctrl_a_selects_all() {
        let mut ti = make_input("hello world");
        let mut input = fake_input();
        input.ctrl_pressed = true;
        input.text_input = "a".to_string();
        ti.process_keyboard(&input);
        assert_eq!(ti.selection_start, Some(0));
        assert_eq!(ti.cursor_pos, 11);
    }

    #[test]
    fn shift_arrow_extends_selection() {
        let mut ti = make_input("abcd");
        // Start with cursor at 2, press shift+left.
        ti.cursor_pos = 2;
        let mut input = fake_input();
        input.shift_pressed = true;
        input.key_left = true;
        ti.process_keyboard(&input);
        assert_eq!(ti.selection_start, Some(2));
        assert_eq!(ti.cursor_pos, 1);

        // Now press shift+left again.
        let mut input2 = fake_input();
        input2.shift_pressed = true;
        input2.key_left = true;
        ti.process_keyboard(&input2);
        assert_eq!(ti.cursor_pos, 0);

        // Without shift, arrow should clear selection and jump to the right end.
        let mut input3 = fake_input();
        input3.key_right = true;
        ti.process_keyboard(&input3);
        assert_eq!(ti.selection_start, None);
        assert_eq!(ti.cursor_pos, 2);
    }

    #[test]
    fn cursor_moves_by_char_not_byte_multibyte() {
        // "é" is 2 UTF-8 bytes, "あ" is 3 bytes.
        let mut ti = make_input("éXあ");
        // Cursor starts at end (byte 6: 2 + 1 + 3).
        assert_eq!(ti.cursor_pos, 6);

        // Move left: should skip "あ" (3 bytes) → byte 3 (after 'X').
        ti.cursor_left(&[]);
        assert_eq!(ti.cursor_pos, 3, "left from end should land after X");

        // Move left again: skip 'X' (1 byte) → byte 2 (after 'é').
        ti.cursor_left(&[]);
        assert_eq!(ti.cursor_pos, 2, "left from X should land after é");

        // Move left again: skip 'é' (2 bytes) → byte 0.
        ti.cursor_left(&[]);
        assert_eq!(ti.cursor_pos, 0, "left from é should land at start");

        // Move right: skip 'é' (2 bytes) → byte 2.
        ti.cursor_right(&[]);
        assert_eq!(ti.cursor_pos, 2, "right from start should land after é");

        // Move right: skip 'X' (1 byte) → byte 3.
        ti.cursor_right(&[]);
        assert_eq!(ti.cursor_pos, 3, "right from é should land after X");

        // Move right: skip 'あ' (3 bytes) → byte 6.
        ti.cursor_right(&[]);
        assert_eq!(ti.cursor_pos, 6, "right from X should land at end");
    }

    // ---- RTL / bidi editing ----

    #[test]
    fn with_direction_sets_field_direction_default_auto() {
        let rtl = TextInput::new(0.0, 0.0, 200.0, 24.0).with_direction(crate::TextDirection::Rtl);
        assert_eq!(rtl.direction, crate::TextDirection::Rtl);
        assert_eq!(
            TextInput::new(0.0, 0.0, 10.0, 10.0).direction,
            crate::TextDirection::Auto,
            "fields default to auto direction"
        );
    }

    #[test]
    fn focused_arrows_move_visually_for_ltr() {
        // Visual Left/Right must coincide with logical prev/next for LTR content
        // — a regression guard that the new visual-movement wiring leaves the
        // common case untouched. Movement runs inside `draw` (which builds the
        // visual layout), so we drive it through focused frames.
        let mut ti = make_input("hello");
        ti.cursor_pos = 5;
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();
        focus.focus(0);

        let mut left = fake_input();
        left.key_left = true;
        focus.begin_frame(&left);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &left);
        assert_eq!(ti.cursor_pos, 4, "visual-left from end steps back one char");

        let mut right = fake_input();
        right.key_right = true;
        focus.begin_frame(&right);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &right);
        assert_eq!(ti.cursor_pos, 5, "visual-right returns to the end");
    }

    #[test]
    fn focused_arrow_moves_caret_in_rtl_value() {
        // An RTL value lays out right-to-left; cosmic assigns bidi levels
        // regardless of which faces are installed, so visual movement engages.
        // The logical end of RTL text is the visual *left* edge, so a visual
        // Right step moves rightward into the text. We assert only that the
        // caret moves (the exact byte depends on the shaped run), confirming the
        // visual-movement path is wired for RTL content.
        let mut ti = make_input("אבג"); // three Hebrew letters
        let start = ti.cursor_pos; // logical end of the value
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();
        focus.focus(0);

        let mut right = fake_input();
        right.key_right = true;
        focus.begin_frame(&right);
        draw_input(&mut ti, 0, &mut focus, &mut list, &theme, &right);
        assert_ne!(
            ti.cursor_pos, start,
            "a visual arrow moves the caret in RTL text"
        );
    }

    #[test]
    fn active_selection_adds_a_fill_quad() {
        // The bidi selection-rect path must still draw a highlight for a plain
        // contiguous selection. Compare vertex counts with and without an active
        // selection on an otherwise identical focused field.
        let theme = Theme::default();
        let mut focus = FocusState::new();
        focus.focus(0);

        let mut plain = make_input("hello");
        plain.cursor_pos = 5;
        let mut l0 = DrawList::new();
        focus.begin_frame(&fake_input());
        draw_input(&mut plain, 0, &mut focus, &mut l0, &theme, &fake_input());
        // Filled quads take the instanced-chrome fast path under a translate-only
        // transform, so the selection rectangle lands in `chrome_instances`.
        let base = l0.chrome_instances.len();

        let mut sel = make_input("hello");
        sel.cursor_pos = 4;
        sel.selection_start = Some(1);
        let mut l1 = DrawList::new();
        focus.begin_frame(&fake_input());
        draw_input(&mut sel, 0, &mut focus, &mut l1, &theme, &fake_input());
        assert!(
            l1.chrome_instances.len() > base,
            "an active selection adds at least one fill quad ({} vs {})",
            l1.chrome_instances.len(),
            base
        );
    }

    #[test]
    fn cut_with_clipboard() {
        let copied = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let mut ti = make_input("hello world");
        ti.selection_start = Some(0);
        ti.cursor_pos = 5; // select "hello"
        ti.set_clipboard_set({
            let copied = copied.clone();
            move |t| *copied.borrow_mut() = t
        });
        ti.cut();
        assert_eq!(ti.value, " world");
        assert_eq!(ti.cursor_pos, 0);
        assert_eq!(ti.selection_start, None);
        assert_eq!(*copied.borrow(), "hello");
    }

    #[test]
    fn copy_with_clipboard() {
        let copied = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let mut ti = make_input("hello world");
        ti.selection_start = Some(6);
        ti.cursor_pos = 11; // select "world"
        ti.set_clipboard_set({
            let copied = copied.clone();
            move |t| *copied.borrow_mut() = t
        });
        ti.copy();
        // Value unchanged after copy.
        assert_eq!(ti.value, "hello world");
        assert_eq!(ti.selection_start, Some(6));
        assert_eq!(ti.cursor_pos, 11);
        assert_eq!(*copied.borrow(), "world");
    }

    #[test]
    fn paste_with_clipboard() {
        let mut ti = make_input("heo");
        ti.cursor_pos = 2; // between 'e' and 'o'
        ti.set_clipboard_get(|| "ll".to_string());
        ti.paste();
        assert_eq!(ti.value, "hello");
        assert_eq!(ti.cursor_pos, 4);
    }

    #[test]
    fn paste_replaces_selection() {
        let mut ti = make_input("hello world");
        ti.selection_start = Some(6);
        ti.cursor_pos = 11; // select "world"
        ti.set_clipboard_get(|| "there".to_string());
        ti.paste();
        assert_eq!(ti.value, "hello there");
        assert_eq!(ti.cursor_pos, 11);
        assert_eq!(ti.selection_start, None);
    }

    #[test]
    fn ctrl_x_cut_with_clipboard() {
        let copied = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let mut ti = make_input("abcdef");
        ti.selection_start = Some(1);
        ti.cursor_pos = 4; // select "bcd"
        ti.set_clipboard_set({
            let copied = copied.clone();
            move |t| *copied.borrow_mut() = t
        });
        let mut input = fake_input();
        input.ctrl_pressed = true;
        // Ctrl+X: the 'x' may come as ASCII control code 0x18 or as the letter 'x'.
        input.text_input = "\x18".to_string();
        ti.process_keyboard(&input);
        assert_eq!(ti.value, "aef");
        assert_eq!(*copied.borrow(), "bcd");
    }

    #[test]
    fn ctrl_c_copy_with_clipboard() {
        let copied = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let mut ti = make_input("abcdef");
        ti.selection_start = Some(2);
        ti.cursor_pos = 5; // select "cde"
        ti.set_clipboard_set({
            let copied = copied.clone();
            move |t| *copied.borrow_mut() = t
        });
        let mut input = fake_input();
        input.ctrl_pressed = true;
        input.text_input = "\x03".to_string(); // Ctrl+C ASCII code
        ti.process_keyboard(&input);
        assert_eq!(ti.value, "abcdef"); // unchanged
        assert_eq!(*copied.borrow(), "cde");
    }

    #[test]
    fn ctrl_v_paste_with_clipboard() {
        let mut ti = make_input("helo");
        ti.cursor_pos = 3; // between 'l' and 'o'
        ti.set_clipboard_get(|| "l".to_string());
        let mut input = fake_input();
        input.ctrl_pressed = true;
        input.text_input = "\x16".to_string(); // Ctrl+V ASCII code
        ti.process_keyboard(&input);
        assert_eq!(ti.value, "hello");
        assert_eq!(ti.cursor_pos, 4);
    }

    /// Clicking one input then the other (across frames) hands focus to exactly
    /// one — the old `focused: bool` bug let multiple inputs activate at once.
    #[test]
    fn two_inputs_cannot_both_be_focused() {
        let mut a = TextInput::new(0.0, 0.0, 100.0, 24.0);
        let mut b = TextInput::new(0.0, 40.0, 100.0, 24.0);
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let theme = Theme::default();

        // A frame in which the mouse clicks at (x, y).
        let click_at = |x: f32, y: f32| InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..Default::default()
        };

        // Frame 1: click input A.
        let input = click_at(10.0, 10.0);
        focus.begin_frame(&input);
        draw_input(&mut a, 0, &mut focus, &mut list, &theme, &input);
        draw_input(&mut b, 1, &mut focus, &mut list, &theme, &input);
        focus.end_frame(None);
        assert!(focus.is_focused(0));
        assert!(!focus.is_focused(1));

        // Frame 2: click input B — focus moves, A loses it.
        let input = click_at(10.0, 50.0);
        focus.begin_frame(&input);
        draw_input(&mut a, 0, &mut focus, &mut list, &theme, &input);
        draw_input(&mut b, 1, &mut focus, &mut list, &theme, &input);
        focus.end_frame(None);
        assert!(!focus.is_focused(0));
        assert!(focus.is_focused(1));

        // Frame 3: click empty space — both blur.
        let input = click_at(500.0, 500.0);
        focus.begin_frame(&input);
        draw_input(&mut a, 0, &mut focus, &mut list, &theme, &input);
        draw_input(&mut b, 1, &mut focus, &mut list, &theme, &input);
        focus.end_frame(None);
        assert!(!focus.is_focused(0));
        assert!(!focus.is_focused(1));
        assert_eq!(focus.focused(), None);
    }

    // ---- Multi-line behaviour ----

    /// Build a `[CaretPos]` layout matching what a multiline field lays out, so
    /// keyboard tests can exercise vertical nav / line Home-End off-screen.
    fn caret_layout(text: &str, max_width: f32) -> Vec<CaretPos> {
        let fsh = crate::text::shared_font_system();
        let mut fs = fsh.lock().unwrap();
        crate::text::text_caret_layout(
            &mut fs,
            text,
            16.0,
            20.0,
            max_width,
            WrapMode::WordOrGlyph,
            None,
            crate::text::TextDirection::Auto,
        )
    }

    #[test]
    fn enter_inserts_newline_only_when_multiline() {
        // Single-line: Enter is left for the caller (no value change).
        let mut single = make_input("ab");
        single.cursor_pos = 1;
        let mut ev = fake_input();
        ev.enter_pressed = true;
        single.process_keyboard(&ev);
        assert_eq!(
            single.value, "ab",
            "single-line Enter must not insert a newline"
        );

        // Multiline: Enter inserts '\n' at the cursor and advances.
        let mut multi = make_input("ab").with_multiline(true);
        multi.cursor_pos = 1;
        let mut ev2 = fake_input();
        ev2.enter_pressed = true;
        multi.process_keyboard(&ev2);
        assert_eq!(multi.value, "a\nb");
        assert_eq!(multi.cursor_pos, 2);
    }

    #[test]
    fn up_down_move_across_lines() {
        // Two short lines; the layout is unambiguous.
        let mut ti = make_input("foo\nbar").with_multiline(true);
        let layout = caret_layout("foo\nbar", 1000.0);
        // Put the cursor at the end of line 0 ("foo", byte 3).
        ti.cursor_pos = 3;
        // Down → land on line 1.
        let mut down = fake_input();
        down.key_down = true;
        ti.process_keyboard_with_layout(&down, &layout);
        assert_eq!(
            caret_for_byte(&layout, ti.cursor_pos).line,
            1,
            "down moves to line 1"
        );
        // Up → back to line 0.
        let mut up = fake_input();
        up.key_up = true;
        ti.process_keyboard_with_layout(&up, &layout);
        assert_eq!(
            caret_for_byte(&layout, ti.cursor_pos).line,
            0,
            "up returns to line 0"
        );
    }

    #[test]
    fn up_at_top_and_down_at_bottom_are_noops() {
        let mut ti = make_input("foo\nbar").with_multiline(true);
        let layout = caret_layout("foo\nbar", 1000.0);
        ti.cursor_pos = 1; // line 0
        let mut up = fake_input();
        up.key_up = true;
        ti.process_keyboard_with_layout(&up, &layout);
        assert_eq!(
            caret_for_byte(&layout, ti.cursor_pos).line,
            0,
            "up at top stays on line 0"
        );

        ti.cursor_pos = 5; // line 1 ("bar")
        let mut down = fake_input();
        down.key_down = true;
        ti.process_keyboard_with_layout(&down, &layout);
        assert_eq!(
            caret_for_byte(&layout, ti.cursor_pos).line,
            1,
            "down at bottom stays on line 1"
        );
    }

    #[test]
    fn shift_down_extends_selection() {
        let mut ti = make_input("foo\nbar").with_multiline(true);
        let layout = caret_layout("foo\nbar", 1000.0);
        ti.cursor_pos = 1;
        let mut ev = fake_input();
        ev.key_down = true;
        ev.shift_pressed = true;
        ti.process_keyboard_with_layout(&ev, &layout);
        assert_eq!(
            ti.selection_start,
            Some(1),
            "shift+down anchors the selection at the old caret"
        );
        assert!(ti.cursor_pos > 1, "caret moved down");
    }

    #[test]
    fn desired_caret_x_sticks_then_resets_on_horizontal_move() {
        // A long line then a short line then a long line. Moving down from a far
        // column on line 0 should keep the column across the short middle line.
        let text = "abcdefghij\nx\nabcdefghij";
        let mut ti = make_input(text).with_multiline(true);
        let layout = caret_layout(text, 1000.0);
        // Place the caret near the end of line 0 (byte 10, the '\n').
        ti.cursor_pos = 10;
        let start_x = caret_for_byte(&layout, ti.cursor_pos).x;

        let mut down = fake_input();
        down.key_down = true;
        ti.process_keyboard_with_layout(&down, &layout);
        // Now on the short line (line 1) — clamped near its end.
        assert_eq!(caret_for_byte(&layout, ti.cursor_pos).line, 1);

        // Down again — sticky column should land near the original x on line 2,
        // NOT clamped to the short middle line's end.
        ti.process_keyboard_with_layout(&down, &layout);
        let landed = caret_for_byte(&layout, ti.cursor_pos);
        assert_eq!(landed.line, 2);
        assert!(
            (landed.x - start_x).abs() < 30.0,
            "sticky column preserved across the short line (landed x {} vs start {})",
            landed.x,
            start_x,
        );

        // A horizontal move clears the sticky column.
        let mut left = fake_input();
        left.key_left = true;
        ti.process_keyboard_with_layout(&left, &layout);
        assert!(
            ti.desired_caret_x.is_none(),
            "horizontal move resets the sticky column"
        );
    }

    #[test]
    fn home_end_are_line_relative_in_multiline() {
        let text = "hello\nworld";
        let mut ti = make_input(text).with_multiline(true);
        let layout = caret_layout(text, 1000.0);
        // Cursor in the middle of line 1 ("world", byte 8).
        ti.cursor_pos = 8;
        let mut home = fake_input();
        home.key_home = true;
        ti.process_keyboard_with_layout(&home, &layout);
        assert_eq!(ti.cursor_pos, 6, "Home → start of line 1 (absolute byte 6)");
        let mut end = fake_input();
        end.key_end = true;
        ti.process_keyboard_with_layout(&end, &layout);
        assert_eq!(ti.cursor_pos, 11, "End → end of line 1 (absolute byte 11)");
    }

    #[test]
    fn line_bounds_finds_visual_line_extent() {
        let text = "ab\ncd";
        let layout = caret_layout(text, 1000.0);
        // Byte 4 is on line 1 ("cd", bytes 3..5).
        let (lo, hi) = line_bounds(&layout, 4);
        assert_eq!(lo, 3);
        assert_eq!(hi, 5);
    }

    // ---- IME preedit composition (compose_preedit) ----

    const UL: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    #[test]
    fn compose_preedit_empty_value_single_underlined_span() {
        let (display, spans, caret) = compose_preedit("", 0, "ㄓㄨ", None, UL);
        assert_eq!(display, "ㄓㄨ");
        assert_eq!(spans.len(), 1, "no before/after segments → one span");
        assert_eq!(spans[0].text, "ㄓㄨ");
        assert_eq!(spans[0].underline, Some(UL));
        assert_eq!(spans[0].color, None, "preedit inherits the block colour");
        // Caret at end of preedit (no preedit_cursor): 'ㄓㄨ' is 6 bytes.
        assert_eq!(caret, 6);
    }

    #[test]
    fn compose_preedit_caret_mid_value_splices_in_order() {
        // value "abXYZ", caret after "ab" (byte 2), preedit "QQ".
        let (display, spans, caret) = compose_preedit("abXYZ", 2, "QQ", None, UL);
        assert_eq!(display, "abQQXYZ");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "ab");
        assert_eq!(spans[0].underline, None);
        assert_eq!(spans[1].text, "QQ");
        assert_eq!(spans[1].underline, Some(UL));
        assert_eq!(spans[2].text, "XYZ");
        assert_eq!(spans[2].underline, None);
        // Caret at end of preedit: 2 (before) + 2 (preedit) = 4.
        assert_eq!(caret, 4);
    }

    #[test]
    fn compose_preedit_uses_preedit_cursor_start() {
        // preedit_cursor [1,1] → caret one byte into the preedit.
        let (_display, _spans, caret) = compose_preedit("ab", 2, "QQ", Some([1, 1]), UL);
        assert_eq!(caret, 2 + 1);
    }

    #[test]
    fn compose_preedit_at_value_start_omits_before_span() {
        let (display, spans, caret) = compose_preedit("xyz", 0, "PRE", None, UL);
        assert_eq!(display, "PRExyz");
        assert_eq!(spans.len(), 2, "empty before segment is omitted");
        assert_eq!(spans[0].text, "PRE");
        assert_eq!(spans[0].underline, Some(UL));
        assert_eq!(spans[1].text, "xyz");
        assert_eq!(caret, 3);
    }

    #[test]
    fn compose_preedit_at_value_end_omits_after_span() {
        let (display, spans, _caret) = compose_preedit("xyz", 3, "PRE", None, UL);
        assert_eq!(display, "xyzPRE");
        assert_eq!(spans.len(), 2, "empty after segment is omitted");
        assert_eq!(spans[0].text, "xyz");
        assert_eq!(spans[1].text, "PRE");
        assert_eq!(spans[1].underline, Some(UL));
    }

    #[test]
    fn compose_preedit_multibyte_value_and_preedit_on_boundaries() {
        // value "あい" (each 3 bytes), caret after first char (byte 3),
        // preedit "ん" (3 bytes).
        let (display, spans, caret) = compose_preedit("あい", 3, "ん", None, UL);
        assert_eq!(display, "あんい");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "あ");
        assert_eq!(spans[1].text, "ん");
        assert_eq!(spans[2].text, "い");
        assert_eq!(caret, 6, "3 (before) + 3 (preedit)");
    }

    #[test]
    fn compose_preedit_cursor_past_end_is_clamped() {
        // A cursor_pos beyond the value length is clamped to value.len().
        let (display, _spans, caret) = compose_preedit("ab", 999, "Q", None, UL);
        assert_eq!(display, "abQ");
        assert_eq!(caret, 3);
    }

    #[test]
    fn floor_char_boundary_snaps_into_multibyte() {
        // "é" = bytes [0,1]; index 1 is mid-char → snaps to 0.
        assert_eq!(floor_char_boundary("é", 1), 0);
        assert_eq!(floor_char_boundary("é", 2), 2);
        assert_eq!(floor_char_boundary("é", 99), 2, "clamped to len");
    }
}
