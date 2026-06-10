//! Text input widget with selection, cursor navigation, and clipboard support.

use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;

/// Byte-position snapping: find the closest cursor position to `click_x`
/// by binary-searching the cursor-positions table.
fn closest_cursor_pos(positions: &[(usize, f32)], click_x: f32) -> usize {
    if positions.is_empty() {
        return 0;
    }
    // Binary search for the first x > click_x.
    match positions.binary_search_by(|&(_, x)| x.partial_cmp(&click_x).unwrap_or(std::cmp::Ordering::Equal)) {
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
    pub focused: bool,
    /// Byte index of the text cursor (insertion point) within `value`.
    pub cursor_pos: usize,
    /// When set, the text between `selection_start` and `cursor_pos` is selected.
    /// `None` means no active selection.
    pub selection_start: Option<usize>,
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
            focused: false,
            cursor_pos: 0,
            selection_start: None,
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
            focused: false,
            cursor_pos: 0,
            selection_start: None,
            clipboard_get: None,
            clipboard_set: None,
        }
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

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
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
            self.value.replace_range(self.cursor_pos..next.0 + next.1, "");
        }
    }

    /// Move cursor one grapheme-cluster left.
    fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.value[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_pos = prev;
        }
    }

    /// Move cursor one grapheme-cluster right.
    fn cursor_right(&mut self) {
        if self.cursor_pos < self.value.len() {
            let next = self.value[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i)
                .unwrap_or(self.value.len());
            self.cursor_pos = next;
        }
    }

    /// Move cursor to the beginning of the text.
    fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to the end of the text.
    fn cursor_end(&mut self) {
        self.cursor_pos = self.value.len();
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

    /// Process keyboard events for the text field. Called automatically by `draw()`.
    pub fn process_keyboard(&mut self, input: &InputState) {
        if !self.focused {
            return;
        }

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

        // Arrow keys (with possible Shift for selection extension).
        if input.key_left {
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
                self.cursor_left();
            } else {
                if self.selection_start.is_some() && self.cursor_pos != self.selection_start.unwrap() {
                    // Moving without Shift clears selection and jumps to the closer end.
                    let (s, _e) = self.selection_range().unwrap();
                    // Move to the leftmost end of the selection.
                    self.cursor_pos = s;
                } else {
                    self.cursor_left();
                }
                self.selection_start = None;
            }
            return;
        }

        if input.key_right {
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
                self.cursor_right();
            } else {
                if self.selection_start.is_some() && self.cursor_pos != self.selection_start.unwrap() {
                    let (_, e) = self.selection_range().unwrap();
                    self.cursor_pos = e;
                } else {
                    self.cursor_right();
                }
                self.selection_start = None;
            }
            return;
        }

        if input.key_home {
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
            } else {
                self.selection_start = None;
            }
            self.cursor_home();
            return;
        }

        if input.key_end {
            let was = self.cursor_pos;
            if input.shift_pressed {
                if self.selection_start.is_none() {
                    self.selection_start = Some(was);
                }
            } else {
                self.selection_start = None;
            }
            self.cursor_end();
            return;
        }

        // Backspace — delete before cursor or selection.
        if input.backspace_pressed {
            self.delete_before_cursor();
            return;
        }

        // Delete — delete after cursor or selection.
        if input.key_delete {
            self.delete_after_cursor();
            return;
        }

        // Text insertion.
        if !input.text_input.is_empty() {
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
    /// Processes keyboard input internally when `focused`. To wire up Ctrl+A/C/V/X,
    /// also call [`set_clipboard_get`](Self::set_clipboard_get) /
    /// [`set_clipboard_set`](Self::set_clipboard_set) before drawing.
    pub fn draw(&mut self, list: &mut DrawList, theme: &Theme, input: &InputState) -> bool {
        let hovered = input.is_hovered(self.x, self.y, self.width, self.height);
        let clicked = hovered && input.mouse_clicked;

        // ---- Click-to-position ----
        if clicked && self.focused {
            let click_x = input.mouse_x;
            let padding = theme.padding;
            let text_left = self.x + padding;
            let text_right = self.x + self.width - padding;

            if click_x >= text_left && click_x <= text_right {
                let local_x = click_x - text_left;
                let positions = list.text_cursor_positions(&self.value, theme.font_size, Some(self.width - padding * 2.0));
                let byte_pos = closest_cursor_pos(&positions, local_x);
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

        // Process keyboard events.
        self.process_keyboard(input);

        // ---- Draw background ----
        list.quad(
            self.x,
            self.y,
            self.width,
            self.height,
            theme.input_background,
        );

        // ---- Draw border ----
        let border = theme.border_width;
        let border_color = if self.focused {
            theme.input_focus_border
        } else if hovered {
            theme.accent
        } else {
            theme.input_border
        };

        list.quad(self.x, self.y, self.width, border, border_color);
        list.quad(
            self.x,
            self.y + self.height - border,
            self.width,
            border,
            border_color,
        );
        list.quad(self.x, self.y, border, self.height, border_color);
        list.quad(
            self.x + self.width - border,
            self.y,
            border,
            self.height,
            border_color,
        );

        // ---- Draw text content ----
        let text_x = self.x + theme.padding;
        let text_y = self.y + (self.height - theme.font_size) / 2.0;
        let text_max_w = self.width - theme.padding * 2.0;
        let line_height = theme.font_size;
        let (text_content, text_color) = if self.value.is_empty() {
            (&self.placeholder, theme.text_dim)
        } else {
            (&self.value, theme.text)
        };

        if self.focused && !self.value.is_empty() {
            // ---- Draw selection highlight ----
            if let Some((sel_start, sel_end)) = self.selection_range() {
                if sel_start < sel_end {
                    let positions = list.text_cursor_positions(&self.value, theme.font_size, Some(text_max_w));
                    let sel_x1 = positions
                        .iter()
                        .find(|&&(i, _)| i >= sel_start)
                        .map(|&(_, x)| x)
                        .unwrap_or(0.0);
                    let sel_x2 = positions
                        .iter()
                        .find(|&&(i, _)| i >= sel_end)
                        .map(|&(_, x)| x)
                        .unwrap_or(positions.last().map(|&(_, x)| x).unwrap_or(0.0));

                    list.quad(
                        text_x + sel_x1,
                        text_y,
                        sel_x2 - sel_x1,
                        line_height,
                        theme.accent, // selection highlight color
                    );
                }
            }
        }

        let text = TextBlock::new(text_content, text_x, text_y)
            .with_size(theme.font_size)
            .with_color(
                (text_color[0] * 255.0) as u8,
                (text_color[1] * 255.0) as u8,
                (text_color[2] * 255.0) as u8,
            )
            .with_max_width(text_max_w);
        list.text(text);

        // ---- Draw cursor ----
        if self.focused {
            let cursor_x = if self.value.is_empty() {
                text_x
            } else {
                let positions = list.text_cursor_positions(&self.value, theme.font_size, Some(text_max_w));
                let offset = positions
                    .iter()
                    .find(|&&(i, _)| i >= self.cursor_pos)
                    .map(|&(_, x)| x)
                    .unwrap_or(positions.last().map(|&(_, x)| x).unwrap_or(0.0));
                text_x + offset
            };
            list.quad(
                cursor_x,
                text_y,
                1.5,
                line_height,
                theme.text,
            );
        }

        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputState;

    fn make_input(val: &str) -> TextInput {
        TextInput::new(0.0, 0.0, 200.0, 24.0)
            .with_value(val.to_string())
            .with_focused(true)
    }

    fn fake_input() -> InputState {
        InputState::default()
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
        ti.cursor_left();
        assert_eq!(ti.cursor_pos, 1);
        ti.cursor_left();
        assert_eq!(ti.cursor_pos, 0);
        ti.cursor_left(); // no-op
        assert_eq!(ti.cursor_pos, 0);
        ti.cursor_right();
        assert_eq!(ti.cursor_pos, 1);
        ti.cursor_right();
        assert_eq!(ti.cursor_pos, 2);
        ti.cursor_right(); // no-op
        assert_eq!(ti.cursor_pos, 2);
    }

    #[test]
    fn home_end() {
        let mut ti = make_input("hello world");
        ti.cursor_pos = 5;
        ti.cursor_home();
        assert_eq!(ti.cursor_pos, 0);
        ti.cursor_end();
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
        ti.cursor_left();
        assert_eq!(ti.cursor_pos, 3, "left from end should land after X");

        // Move left again: skip 'X' (1 byte) → byte 2 (after 'é').
        ti.cursor_left();
        assert_eq!(ti.cursor_pos, 2, "left from X should land after é");

        // Move left again: skip 'é' (2 bytes) → byte 0.
        ti.cursor_left();
        assert_eq!(ti.cursor_pos, 0, "left from é should land at start");

        // Move right: skip 'é' (2 bytes) → byte 2.
        ti.cursor_right();
        assert_eq!(ti.cursor_pos, 2, "right from start should land after é");

        // Move right: skip 'X' (1 byte) → byte 3.
        ti.cursor_right();
        assert_eq!(ti.cursor_pos, 3, "right from é should land after X");

        // Move right: skip 'あ' (3 bytes) → byte 6.
        ti.cursor_right();
        assert_eq!(ti.cursor_pos, 6, "right from X should land at end");
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
}
