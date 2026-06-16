//! Virtualized list / grid widget.
//!
//! [`List`] renders a flat collection of `count` items into a scroll viewport,
//! packing them as a single column (a list) or `N` columns (a grid via
//! [`List::columns`]). Only the items intersecting the viewport are drawn, so a
//! 10k-item collection costs the same as a screenful — the widget culls to the
//! visible index range exactly like [`Table`](super::Table), reusing
//! [`ScrollView`](super::ScrollView) for clipping / wheel / scrollbar drag.
//!
//! The widget owns no data: the caller passes the item `count` and a per-item
//! **content** closure `FnMut(&mut DrawList, Rect, ListItem)`. The list itself
//! draws the row background (selection / hover / zebra) and handles all
//! interaction (click-to-select, keyboard navigation); the closure only fills
//! the item rect with its content.
//!
//! Persistent state — scroll offset, the selected set, and the keyboard cursor —
//! lives in a caller-owned [`ListState`], matching the immediate-mode,
//! caller-owns-state style of the rest of the crate
//! ([`ScrollState`](super::ScrollState), [`TreeState`](super::TreeState)).
//!
//! Like [`Table`](super::Table) and [`ScrollView`](super::ScrollView), this is a
//! *scrollable* widget and so takes a raw `&mut InputState` (the `ScrollView`
//! consumes the wheel) rather than a [`DrawContext`](super::DrawContext), whose
//! `input` is shared-only.
//!
//! ```ignore
//! let mut state = ListState::new();
//! let items = ["Sword", "Shield", "Potion"];
//! let out = List::new()
//!     .with_item_height(22.0)
//!     .selection(SelectionMode::Multi)
//!     .focused(panel_has_focus)
//!     .draw(rect, items.len(), &mut state, list, &style, &mut input,
//!         |list, cell, it| {
//!             let color = if it.selected { [255, 255, 255] } else { [200, 210, 230] };
//!             list.text(TextBlock::new(items[it.index], cell.x + 6.0, cell.y + 3.0)
//!                 .with_color(color[0], color[1], color[2]));
//!         });
//! if let Some(i) = out.activated { launch(items[i]); }
//! ```

use std::collections::BTreeSet;

use crate::layout::Rect;
use crate::{InputState, StyleKey, StyleResolver};

use super::DrawList;
use super::scroll_view::{ScrollState, ScrollView};

/// How a [`List`] responds to clicks / keyboard selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// Items are never selected; the list only reports `clicked` / `hovered`.
    None,
    /// At most one selected item (default). Any click replaces the selection.
    #[default]
    Single,
    /// Multiple items: plain click replaces, `Ctrl`+click toggles, `Shift`+click
    /// selects the inclusive range from the anchor.
    Multi,
}

/// This frame's rising-edge navigation keys. Held keys stay `true` across frames
/// at the `InputState` level, so we derive a one-shot edge per press (mirrors the
/// `Tree` widget's nav handling).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NavKeys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    home: bool,
    end: bool,
    activate: bool,
}

impl NavKeys {
    fn raw(input: &InputState) -> Self {
        Self {
            up: input.nav.up,
            down: input.nav.down,
            left: input.nav.left,
            right: input.nav.right,
            // Home/End have no device-agnostic nav intent — they stay keyboard-only.
            home: input.key_home,
            end: input.key_end,
            activate: input.nav.confirm,
        }
    }
}

/// Caller-owned, persistent list state: scroll offset + selection + keyboard
/// cursor. Build once and thread the same `&mut ListState` every frame.
#[derive(Debug, Clone, Default)]
pub struct ListState {
    /// Scroll offset / content extent. Reused by the embedded [`ScrollView`].
    pub scroll: ScrollState,
    /// The selected item indices (ordered, so the caller can iterate cheaply).
    selected: BTreeSet<usize>,
    /// Origin of a `Shift`-range selection (the last plainly-clicked item).
    anchor: Option<usize>,
    /// The keyboard "current" item — the one arrow keys move and `Enter`
    /// activates. Distinct from `selected`: plain arrow nav replaces the
    /// selection with the cursor, `Shift`+arrow extends from `anchor`.
    cursor: Option<usize>,
    /// This frame's rising-edge nav keys.
    keys: NavKeys,
    /// Last frame's raw key state, for edge detection.
    prev_keys: NavKeys,
}

// ---- selection primitives (single source of truth) ----------------------
//
// These operate on the destructured field borrows so the draw loop (which can't
// hold `&mut ListState` while `ScrollView` borrows `state.scroll`) and the
// public methods share identical semantics.

fn sel_one(
    sel: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: &mut Option<usize>,
    i: usize,
) {
    sel.clear();
    sel.insert(i);
    *anchor = Some(i);
    *cursor = Some(i);
}

fn sel_toggle(
    sel: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: &mut Option<usize>,
    i: usize,
) {
    if !sel.remove(&i) {
        sel.insert(i);
    }
    *anchor = Some(i);
    *cursor = Some(i);
}

fn sel_range(
    sel: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: &mut Option<usize>,
    a: usize,
    b: usize,
) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    sel.clear();
    for k in lo..=hi {
        sel.insert(k);
    }
    *anchor = Some(a);
    *cursor = Some(b);
}

impl ListState {
    /// A fresh state: nothing selected, scrolled to the top.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when item `i` is currently selected.
    pub fn is_selected(&self, i: usize) -> bool {
        self.selected.contains(&i)
    }

    /// Iterate the selected indices in ascending order.
    pub fn selected(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected.iter().copied()
    }

    /// How many items are selected.
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// The keyboard cursor (current) item, if any.
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// Move the keyboard cursor without changing the selection.
    pub fn set_cursor(&mut self, i: usize) {
        self.cursor = Some(i);
    }

    /// Select exactly `i`, replacing any prior selection.
    pub fn select_one(&mut self, i: usize) {
        sel_one(&mut self.selected, &mut self.anchor, &mut self.cursor, i);
    }

    /// Toggle `i` in the selection (multi-select).
    pub fn toggle(&mut self, i: usize) {
        sel_toggle(&mut self.selected, &mut self.anchor, &mut self.cursor, i);
    }

    /// Select the inclusive range `a..=b` (order-independent), replacing the
    /// prior selection. The anchor becomes `a`, the cursor `b`.
    pub fn select_range(&mut self, a: usize, b: usize) {
        sel_range(&mut self.selected, &mut self.anchor, &mut self.cursor, a, b);
    }

    /// Clear the selection (cursor and anchor are preserved).
    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Select every item in `0..count`.
    pub fn select_all(&mut self, count: usize) {
        self.selected.clear();
        self.selected.extend(0..count);
    }
}

/// Per-item info handed to the content closure so it can style by state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListItem {
    /// The item's index in `0..count`.
    pub index: usize,
    /// True if this item is in the selected set.
    pub selected: bool,
    /// True if the pointer is over this item this frame.
    pub hovered: bool,
    /// True if this item is the keyboard cursor.
    pub cursor: bool,
}

/// Interaction results from one [`List::draw`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListOutput {
    /// The item clicked this frame, if any.
    pub clicked: Option<usize>,
    /// The item activated this frame (double-relevant for keyboard: `Enter` /
    /// `Space` on the cursor).
    pub activated: Option<usize>,
    /// The item under the pointer this frame, if any.
    pub hovered: Option<usize>,
    /// True if the pointer is over the list's content area (for scroll routing).
    pub mouse_over_content: bool,
}

/// A virtualized list / grid. Transient — rebuild it each frame; all persistent
/// state lives in the caller-owned [`ListState`].
pub struct List {
    item_height: Option<f32>,
    columns: usize,
    col_gap: f32,
    row_gap: f32,
    selection: SelectionMode,
    focused: bool,
    zebra: bool,
}

impl Default for List {
    fn default() -> Self {
        Self {
            item_height: None,
            columns: 1,
            col_gap: 0.0,
            row_gap: 0.0,
            selection: SelectionMode::Single,
            focused: false,
            zebra: false,
        }
    }
}

impl List {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fixed pixel height of every item row. Defaults to a theme-derived value.
    pub fn with_item_height(mut self, h: f32) -> Self {
        self.item_height = Some(h);
        self
    }

    /// Number of columns: `1` (default) is a list, `>1` packs a grid
    /// left-to-right, top-to-bottom.
    pub fn columns(mut self, n: usize) -> Self {
        self.columns = n.max(1);
        self
    }

    /// Gap between columns and between rows, in pixels.
    pub fn with_gap(mut self, col: f32, row: f32) -> Self {
        self.col_gap = col.max(0.0);
        self.row_gap = row.max(0.0);
        self
    }

    /// Selection behavior. Defaults to [`SelectionMode::Single`].
    pub fn selection(mut self, mode: SelectionMode) -> Self {
        self.selection = mode;
        self
    }

    /// Whether this list currently holds keyboard focus. Only a focused list
    /// reacts to arrow / Home / End / Enter keys (so background lists don't
    /// hijack navigation).
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Alternate-row background tint (only meaningful for a single-column list).
    pub fn with_zebra(mut self, zebra: bool) -> Self {
        self.zebra = zebra;
        self
    }

    /// Draw the list and return interaction results.
    pub fn draw<F>(
        &self,
        rect: Rect,
        count: usize,
        state: &mut ListState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        mut item: F,
    ) -> ListOutput
    where
        F: FnMut(&mut DrawList, Rect, ListItem),
    {
        let item_h = self
            .item_height
            .unwrap_or(style.scalar(StyleKey::FontSize) + 10.0)
            .max(1.0);
        let cols = self.columns.max(1);
        let row_pitch = item_h + self.row_gap;

        // --- Geometry / content extent -----------------------------------
        let rows_total = count.div_ceil(cols);
        let content_h = if rows_total == 0 {
            0.0
        } else {
            rows_total as f32 * item_h + (rows_total - 1) as f32 * self.row_gap
        };
        state.scroll.content_size = [rect.width, content_h];

        // --- Keyboard nav (edge detection runs every frame; we only *act*
        //     on edges when focused so prev_keys never goes stale) ----------
        let raw = NavKeys::raw(input);
        state.keys = NavKeys {
            up: raw.up && !state.prev_keys.up,
            down: raw.down && !state.prev_keys.down,
            left: raw.left && !state.prev_keys.left,
            right: raw.right && !state.prev_keys.right,
            home: raw.home && !state.prev_keys.home,
            end: raw.end && !state.prev_keys.end,
            activate: raw.activate && !state.prev_keys.activate,
        };
        state.prev_keys = raw;

        let selectable = self.selection != SelectionMode::None;
        let mut activated = None;

        if self.focused && count > 0 {
            let k = state.keys;
            let last = count - 1;
            let base = state.cursor;
            let mut next = base.unwrap_or(0);
            // First key press lands on the first item rather than stepping off it.
            if base.is_some() {
                if k.down {
                    next = (next + cols).min(last);
                } else if k.up {
                    next = next.saturating_sub(cols);
                }
                if cols > 1 {
                    if k.right {
                        next = (next + 1).min(last);
                    } else if k.left {
                        next = next.saturating_sub(1);
                    }
                }
            }
            if k.home {
                next = 0;
            } else if k.end {
                next = last;
            }

            let moved = k.up || k.down || k.left || k.right || k.home || k.end;
            if moved {
                if selectable {
                    if self.selection == SelectionMode::Multi && input.shift_pressed {
                        let a = state.anchor.unwrap_or(next);
                        sel_range(
                            &mut state.selected,
                            &mut state.anchor,
                            &mut state.cursor,
                            a,
                            next,
                        );
                    } else {
                        sel_one(
                            &mut state.selected,
                            &mut state.anchor,
                            &mut state.cursor,
                            next,
                        );
                    }
                } else {
                    state.cursor = Some(next);
                }
                // Auto-scroll the cursor into view.
                let cursor_row = next / cols;
                let cell_top = cursor_row as f32 * row_pitch;
                let cell_bottom = cell_top + item_h;
                if cell_top < state.scroll.offset[1] {
                    state.scroll.offset[1] = cell_top;
                } else if cell_bottom > state.scroll.offset[1] + rect.height {
                    state.scroll.offset[1] = cell_bottom - rect.height;
                }
                state.scroll.clamp([rect.width, rect.height]);
            }

            if k.activate {
                if let Some(c) = state.cursor {
                    activated = Some(c);
                }
            }
        }

        // --- Snapshot input before ScrollView borrows it mutably ----------
        let mouse_x = input.mouse_x;
        let mouse_y = input.mouse_y;
        let mouse_clicked = input.mouse_clicked;
        let ctrl = input.ctrl_pressed;
        let shift = input.shift_pressed;
        let scroll_y = state.scroll.offset[1];
        let mouse_over_content = rect.contains(mouse_x, mouse_y) && !input.mouse_consumed;

        // Disjoint field borrows so the content closure can mutate selection
        // while ScrollView holds `&mut state.scroll`.
        let ListState {
            scroll,
            selected,
            anchor,
            cursor,
            ..
        } = state;
        let cursor_now = *cursor;

        let mut clicked = None;
        let mut hovered = None;
        let selection_mode = self.selection;
        let zebra = self.zebra;
        let col_gap = self.col_gap;

        ScrollView::new(rect)
            .vertical_only()
            .draw(scroll, list, style, input, |list, vp| {
                let cell_w = if cols == 1 {
                    vp.width
                } else {
                    ((vp.width - (cols - 1) as f32 * col_gap) / cols as f32).max(1.0)
                };
                let first_row = (scroll_y / row_pitch).floor().max(0.0) as usize;
                let visible_rows = (vp.height / row_pitch).ceil() as usize + 1;

                for row in first_row..(first_row + visible_rows) {
                    let world_y = vp.y + row as f32 * row_pitch;
                    let screen_y = world_y - scroll_y;
                    for col in 0..cols {
                        let idx = row * cols + col;
                        if idx >= count {
                            break;
                        }
                        let cell_x = vp.x + col as f32 * (cell_w + col_gap);
                        let cell = Rect::new(cell_x, world_y, cell_w, item_h);

                        // Hit-test in screen space; gaps are dead zones.
                        let over_cell = mouse_over_content
                            && mouse_x >= cell_x
                            && mouse_x < cell_x + cell_w
                            && mouse_y >= screen_y
                            && mouse_y < screen_y + item_h
                            && mouse_y >= vp.y
                            && mouse_y < vp.y + vp.height;

                        if over_cell {
                            hovered = Some(idx);
                            if mouse_clicked {
                                clicked = Some(idx);
                                if selectable {
                                    match selection_mode {
                                        SelectionMode::Multi if ctrl => {
                                            sel_toggle(selected, anchor, cursor, idx)
                                        }
                                        SelectionMode::Multi if shift => {
                                            let a = anchor.unwrap_or(idx);
                                            sel_range(selected, anchor, cursor, a, idx)
                                        }
                                        _ => sel_one(selected, anchor, cursor, idx),
                                    }
                                } else {
                                    *cursor = Some(idx);
                                }
                            }
                        }

                        let is_selected = selected.contains(&idx);
                        let bg = if is_selected {
                            style.color(StyleKey::Accent)
                        } else if over_cell {
                            style.color(StyleKey::ButtonHover)
                        } else if zebra && cols == 1 && idx % 2 == 1 {
                            let mut c = style.color(StyleKey::Panel);
                            c[0] *= 1.12;
                            c[1] *= 1.12;
                            c[2] *= 1.12;
                            c
                        } else {
                            [0.0, 0.0, 0.0, 0.0]
                        };
                        if bg[3] > 0.0 {
                            list.quad(cell.x, cell.y, cell.width, cell.height, bg);
                        }

                        item(
                            list,
                            cell,
                            ListItem {
                                index: idx,
                                selected: is_selected,
                                hovered: over_cell,
                                cursor: cursor_now == Some(idx),
                            },
                        );
                    }
                }
            });

        ListOutput {
            clicked,
            activated,
            hovered,
            mouse_over_content,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..InputState::default()
        }
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    /// Drive one frame; return the output.
    fn frame(
        list_w: &List,
        rect: Rect,
        count: usize,
        st: &mut ListState,
        input: &mut InputState,
    ) -> ListOutput {
        // Tests set raw key edges; map them into `nav` intents (which the list
        // reads) the same way a real frame's `NavMap` would.
        crate::map_keyboard(input);
        let mut dl = DrawList::new();
        let th = theme();
        list_w.draw(
            rect,
            count,
            st,
            &mut dl,
            &StyleResolver::new(&th),
            input,
            |_, _, _| {},
        )
    }

    #[test]
    fn content_size_list_and_grid() {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);

        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0),
            rect,
            10,
            &mut st,
            &mut idle(),
        );
        assert_eq!(st.scroll.content_size[1], 200.0, "10 rows * 20px");

        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0).columns(4),
            rect,
            10,
            &mut st,
            &mut idle(),
        );
        // ceil(10/4) = 3 rows.
        assert_eq!(st.scroll.content_size[1], 60.0, "3 grid rows * 20px");
    }

    #[test]
    fn content_size_includes_row_gap() {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);
        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0).with_gap(0.0, 5.0),
            rect,
            4,
            &mut st,
            &mut idle(),
        );
        // 4*20 + 3*5
        assert_eq!(st.scroll.content_size[1], 95.0);
    }

    #[test]
    fn virtualizes_only_visible_items() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut st = ListState::new();
        let mut dl = DrawList::new();
        let th = theme();
        let mut input = idle();
        let mut calls = 0usize;
        List::new().with_item_height(20.0).draw(
            rect,
            1000,
            &mut st,
            &mut dl,
            &StyleResolver::new(&th),
            &mut input,
            |_, _, _| calls += 1,
        );
        // ~5 visible rows + 1 slack; nowhere near 1000.
        assert!(calls < 20, "expected a screenful, drew {calls}");
        assert!(calls >= 5, "expected the visible rows, drew {calls}");
    }

    #[test]
    fn single_click_selects() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        let mut input = click_at(50.0, 10.0); // row 0
        let out = frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut input,
        );
        assert_eq!(out.clicked, Some(0));
        assert!(st.is_selected(0));
        assert_eq!(st.selected_count(), 1);
    }

    #[test]
    fn single_select_replaces() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut click_at(50.0, 10.0),
        );
        frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut click_at(50.0, 50.0),
        ); // row 2
        assert!(!st.is_selected(0));
        assert!(st.is_selected(2));
        assert_eq!(st.selected_count(), 1);
    }

    #[test]
    fn multi_ctrl_click_toggles() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new()
            .with_item_height(20.0)
            .selection(SelectionMode::Multi);
        let mut st = ListState::new();
        // plain click row 1
        frame(&w, rect, 5, &mut st, &mut click_at(50.0, 30.0));
        // ctrl-click row 3 -> adds
        let mut ctrl_click = click_at(50.0, 70.0);
        ctrl_click.ctrl_pressed = true;
        frame(&w, rect, 5, &mut st, &mut ctrl_click);
        assert!(st.is_selected(1) && st.is_selected(3));
        assert_eq!(st.selected_count(), 2);
        // ctrl-click row 1 again -> removes
        let mut ctrl_click = click_at(50.0, 30.0);
        ctrl_click.ctrl_pressed = true;
        frame(&w, rect, 5, &mut st, &mut ctrl_click);
        assert!(!st.is_selected(1) && st.is_selected(3));
        assert_eq!(st.selected_count(), 1);
    }

    #[test]
    fn multi_shift_click_selects_range() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new()
            .with_item_height(20.0)
            .selection(SelectionMode::Multi);
        let mut st = ListState::new();
        frame(&w, rect, 8, &mut st, &mut click_at(50.0, 30.0)); // anchor row 1
        let mut shift_click = click_at(50.0, 90.0); // row 4
        shift_click.shift_pressed = true;
        frame(&w, rect, 8, &mut st, &mut shift_click);
        let sel: Vec<usize> = st.selected().collect();
        assert_eq!(sel, vec![1, 2, 3, 4]);
    }

    #[test]
    fn keyboard_down_moves_cursor_and_selects() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();

        // First Down lands on item 0.
        let mut down = idle();
        down.key_down = true;
        frame(&w, rect, 10, &mut st, &mut down);
        assert_eq!(st.cursor(), Some(0));
        assert!(st.is_selected(0));

        // Release, then Down again -> item 1 (edge detection).
        frame(&w, rect, 10, &mut st, &mut idle());
        let mut down = idle();
        down.key_down = true;
        frame(&w, rect, 10, &mut st, &mut down);
        assert_eq!(st.cursor(), Some(1));
        assert!(st.is_selected(1) && !st.is_selected(0));
    }

    #[test]
    fn keyboard_autoscrolls_cursor_into_view() {
        let rect = Rect::new(0.0, 0.0, 100.0, 60.0); // 3 rows visible
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();
        // Step down well past the viewport (release between presses for edges).
        for _ in 0..6 {
            let mut down = idle();
            down.key_down = true;
            frame(&w, rect, 20, &mut st, &mut down);
            frame(&w, rect, 20, &mut st, &mut idle());
        }
        assert!(st.cursor().unwrap() >= 4);
        assert!(st.scroll.offset[1] > 0.0, "cursor scrolled into view");
    }

    #[test]
    fn keyboard_enter_activates_cursor() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();
        let mut down = idle();
        down.key_down = true;
        frame(&w, rect, 10, &mut st, &mut down); // cursor -> 0
        frame(&w, rect, 10, &mut st, &mut idle());
        let mut enter = idle();
        enter.enter_pressed = true;
        let out = frame(&w, rect, 10, &mut st, &mut enter);
        assert_eq!(out.activated, Some(0));
    }

    #[test]
    fn grid_hit_test_resolves_row_col() {
        // width 100, 4 cols, no gap -> cell_w 25.
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).columns(4);
        let mut st = ListState::new();
        // col 2 (x 60 in [50,75)), row 1 (y 30 in [20,40)) -> idx 1*4+2 = 6.
        let out = frame(&w, rect, 16, &mut st, &mut click_at(60.0, 30.0));
        assert_eq!(out.clicked, Some(6));
        assert!(st.is_selected(6));
    }

    #[test]
    fn grid_gap_is_dead_zone() {
        // width 100, 4 cols, 10px col gap -> cell_w (100-30)/4 = 17.5.
        // Cell 0 = [0,17.5); gap = [17.5,27.5). Click at x=20 hits the gap.
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new()
            .with_item_height(20.0)
            .columns(4)
            .with_gap(10.0, 0.0);
        let mut st = ListState::new();
        let out = frame(&w, rect, 16, &mut st, &mut click_at(20.0, 5.0));
        assert_eq!(out.clicked, None);
        assert_eq!(st.selected_count(), 0);
    }

    #[test]
    fn mouse_consumed_suppresses_interaction() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        let mut input = click_at(50.0, 10.0);
        input.mouse_consumed = true;
        let out = frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut input,
        );
        assert_eq!(out.clicked, None);
        assert_eq!(out.hovered, None);
        assert_eq!(st.selected_count(), 0);
    }
}
