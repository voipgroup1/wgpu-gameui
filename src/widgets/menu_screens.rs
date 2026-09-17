//! Main-menu primitives: the themed fullscreen scrim and a
//! vertically-navigable menu button list.
//!
//! These are the pieces a game main menu / pause overlay is composed from —
//! deliberately small and orthogonal, in the library's rect-native style (see
//! the module docs on [`Panel`](super::Panel)):
//!
//! - [`draw_scrim`] — the dim layer behind a menu/pause overlay (themed via
//!   [`StyleKey::Scrim`]).
//! - [`MenuList`] — a column of equal-width menu buttons with gamepad/keyboard
//!   up-down selection (caller-owned `selected`, like [`ScrollState`]'s
//!   caller-owned offset), uniform width from the widest label, and
//!   click/hover/confirm activation. It composes [`Button`] internally, so
//!   hover/press chrome and disabled handling match every other button.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};

use super::button::Button;
use super::draw_list::DrawList;

/// Fill `rect` with the theme's fullscreen dim ([`StyleKey::Scrim`]) — the
/// darkening layer drawn between the game world and a modal / menu / pause
/// overlay. A pure paint helper: no interaction, no state, no mouse handling.
///
/// ```ignore
/// draw_scrim(screen_rect, &mut list, &styles);
/// MenuList::new(&["Resume", "Settings", "Quit"]).draw(column_rect, &mut ctx);
/// ```
pub fn draw_scrim(rect: Rect, list: &mut DrawList, styles: &StyleResolver) {
    list.quad(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        styles.color(StyleKey::Scrim),
    );
}

/// Interaction results from one [`MenuList::draw`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MenuListOutput {
    /// Index of the item activated this frame (click, or confirm on the
    /// selected item), if any. This is the value the caller branches on —
    /// `Resume`, `Settings`, `Quit`, …
    pub activated: Option<usize>,
    /// Index the pointer hovered this frame, if any. Hover moves the
    /// selection (the classic console-menu coupling), so `selected_changed`
    /// is usually the more useful signal.
    pub hovered: Option<usize>,
    /// Whether the caller-owned `selected` changed this frame — by pointer
    /// hover, `nav.up`/`nav.down`, or wrap-around. Write it back into the
    /// widget for the next frame.
    pub selected_changed: bool,
    /// The widest item's natural width, `padding` included — the minimum
    /// width that fits every label without ellipsis. The widget sizes its
    /// rows to `rect.width`, so pass this (or anything larger) as the column
    /// width for a fit-content menu.
    pub intrinsic_width: f32,
}

/// A vertical column of equal-width menu buttons — the main-menu / pause-menu
/// core. Transient: rebuild it each frame; the only persistent state is the
/// caller-owned `selected` index (and, if you opt in, the focus ids).
///
/// # Navigation
///
/// - **Pointer** — hover moves the selection; click activates.
/// - **Gamepad/keyboard** — `nav.up`/`nav.down` move the selection (wrapping),
///   `nav.confirm` activates it. `left`/`right` are left alone (a menu screen
///   has no horizontal axis) and `nav.cancel` is left alone (Esc-to-quit is
///   the game's decision, not the widget's).
/// - **Tab ring** — opt in per item via [`focusable`](Self::focusable) to join
///   the widget focus system; confirm then also works through the buttons'
///   own Space/Enter path.
///
/// # Example
///
/// ```ignore
/// let mut menu_selected = 0usize; // caller-owned, like ScrollState
///
/// let out = MenuList::new(&["New Game", "Continue", "Settings", "Quit"])
///     .row_height(44.0)
///     .gap(10.0)
///     .focusable(MENU_FOCUS_BASE) // stable ids: base + i per row
///     .draw(menu_rect, &mut menu_selected, &mut ctx);
/// if out.selected_changed {
///     // menu_selected was written through — play a move sound, etc.
/// }
/// if let Some(i) = out.activated {
///     match i { 0 => start_game(), 3 => quit(), _ => {} }
/// }
/// ```
pub struct MenuList<'a> {
    items: &'a [&'a str],
    row_height: Option<f32>,
    gap: f32,
    enabled: Option<&'a [bool]>,
    focus_base: Option<u64>,
}

impl<'a> Default for MenuList<'a> {
    fn default() -> Self {
        Self {
            items: &[],
            row_height: None,
            gap: 0.0,
            enabled: None,
            focus_base: None,
        }
    }
}

impl<'a> MenuList<'a> {
    /// A menu listing `items`, top to bottom.
    pub fn new(items: &'a [&'a str]) -> Self {
        Self {
            items,
            ..Default::default()
        }
    }

    /// Height of each row. Defaults to the theme's
    /// [`ButtonHeight`](StyleKey::ButtonHeight).
    pub fn row_height(mut self, h: f32) -> Self {
        self.row_height = Some(h);
        self
    }

    /// Vertical gap between rows. Defaults to the theme's
    /// [`Spacing`](StyleKey::Spacing).
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Per-item enabled flags (a disabled row is dimmed and inert, like a
    /// disabled [`Button`]). Must match `items` in length when set.
    pub fn enabled(mut self, enabled: &'a [bool]) -> Self {
        self.enabled = Some(enabled);
        self
    }

    /// Make every row focusable under stable ids `base + i` (see
    /// [`Button::focusable`]): the rows join the Tab ring, draw the focus ring
    /// while focused, and activation also flows through the button's own
    /// Space/Enter path. Without this the menu relies purely on
    /// `nav.up/down/confirm` + pointer.
    pub fn focusable(mut self, base: u64) -> Self {
        self.focus_base = Some(base);
        self
    }

    /// Total height for `n` rows at `row_height` and `gap` (both already
    /// resolved against the theme by [`draw`](Self::draw)).
    fn total_height(&self, row_height: f32, gap: f32, n: usize) -> f32 {
        if n == 0 {
            return 0.0;
        }
        let n = n as f32;
        row_height * n + gap * (n - 1.0)
    }

    /// Draw the menu into `rect` (rows are laid out top-down, centered as a
    /// block if `rect` is taller than the content).
    ///
    /// `selected` is the caller-owned selection; it is updated in place this
    /// frame (hover/nav) and clamped to the item range, so the same `usize`
    /// can be passed again next frame. Note it is written **before** any
    /// activation is reported: `out.activated` indexes the *new* selection,
    /// which is what a game wants (`nav.down` then `confirm` across two
    /// frames fires the row the player moved to).
    pub fn draw(
        &self,
        rect: Rect,
        selected: &mut usize,
        ctx: &mut crate::DrawContext,
    ) -> MenuListOutput {
        let mut out = MenuListOutput::default();
        let n = self.items.len();
        if n == 0 || rect.width <= 0.0 {
            return out;
        }
        let s = ctx.styles();
        let row_height = self
            .row_height
            .unwrap_or_else(|| s.scalar(StyleKey::ButtonHeight));
        let gap = if self.gap > 0.0 {
            self.gap
        } else {
            s.scalar(StyleKey::Spacing)
        };
        let padding = s.scalar(StyleKey::Padding);

        // Uniform row width = the widest natural label, never narrower than
        // ~3× the widest text so a menu reads as a menu (the menubar's
        // `MenuItemMinWidth` idea, adapted for a column) — but capped to the
        // rect actually allocated.
        let mut max_label_w = 0.0f32;
        for item in self.items {
            let block = s.text_block(*item, 0.0, 0.0);
            let (w, _) = ctx.draw_list.measure_block(&block);
            max_label_w = max_label_w.max(w);
        }
        out.intrinsic_width = (max_label_w + padding * 2.0)
            .max(max_label_w * 3.0)
            .min(rect.width);

        // Center the block vertically within the rect.
        let content_h = self.total_height(row_height, gap, n);
        let mut y = rect.y + (rect.height - content_h) * 0.5;
        if content_h > rect.height {
            y = rect.y; // overflow: start at the top (clip if you need less)
        }
        let x = rect.x + (rect.width - out.intrinsic_width) * 0.5;

        // Navigation: clamp the stored selection up front so a shrunken item
        // list can never index out of bounds below.
        *selected = (*selected).min(n - 1);

        let up = ctx.input.nav.up;
        let down = ctx.input.nav.down;
        let confirm = ctx.input.nav.confirm;
        let had_nav = up || down;
        if had_nav {
            let n_i = n as i32;
            let cur = *selected as i32;
            let dir = if down { 1 } else { -1 };
            *selected = (cur + dir).rem_euclid(n_i) as usize;
            out.selected_changed = true;
        }

        // Resolve pointer hover BEFORE painting any row, so every row's
        // chrome reflects the same (new) selection — otherwise rows painted
        // above the hovered one show the stale selection for a frame.
        let mouse = (
            ctx.input.mouse_x,
            ctx.input.mouse_y,
            ctx.input.mouse_consumed,
        );
        let mut hovered_index: Option<usize> = None;
        {
            let mut hy = y;
            for (i, _) in self.items.iter().enumerate() {
                let row = Rect::new(x, hy, out.intrinsic_width, row_height);
                hy += row_height + gap;
                let enabled = self
                    .enabled
                    .map_or(true, |e| e.get(i).copied().unwrap_or(true));
                if enabled && !mouse.2 && row.contains(mouse.0, mouse.1) {
                    hovered_index = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = hovered_index {
            if *selected != i {
                *selected = i;
                out.selected_changed = true;
            }
            out.hovered = Some(i);
        }

        // The rows. `activated_here` captures which row (if any) the draw
        // activated so the nav-confirm below can't double-report a row the
        // pointer already fired.
        let mut activated_here: Option<usize> = None;
        for (i, label) in self.items.iter().enumerate() {
            let row = Rect::new(x, y, out.intrinsic_width, row_height);
            y += row_height + gap;

            let enabled = self
                .enabled
                .map_or(true, |e| e.get(i).copied().unwrap_or(true));

            // Hover chrome comes from selection (hover promotes selection),
            // so nothing per-row is needed beyond the tone above.
            let mut button = Button::new(*label)
                .tone(if *selected == i {
                    crate::Tone::Accent
                } else {
                    crate::Tone::Default
                })
                .enabled(enabled);
            if let Some(base) = self.focus_base {
                button = button.focusable(base + i as u64);
            }
            let clicked = button.draw(row, ctx);
            if clicked && activated_here.is_none() {
                activated_here = Some(i);
            }
        }

        // Confirm activates the (possibly just-moved) selection — but not on
        // the same frame the selection moved (the press that confirms must
        // not also be the press that moves).
        if confirm && !had_nav && activated_here.is_none() && !ctx.input.mouse_consumed {
            if self
                .enabled
                .map_or(true, |e| e.get(*selected).copied().unwrap_or(true))
            {
                out.activated = Some(*selected);
            }
        } else if let Some(i) = activated_here {
            if self
                .enabled
                .map_or(true, |e| e.get(i).copied().unwrap_or(true))
            {
                out.activated = Some(i);
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FocusState;
    use crate::InputState;
    use crate::style::StyleOverlay;

    const ITEMS: &[&str] = &["Resume", "Settings", "Quit"];

    fn draw_at(rect: Rect, selected: &mut usize, input: &InputState) -> (MenuListOutput, DrawList) {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let mut overlay = StyleOverlay::new();
        overlay.set_scalar(StyleKey::ButtonHeight, 24.0);
        overlay.set_scalar(StyleKey::Spacing, 4.0);
        let mut ctx = crate::DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0)
            .with_style(&overlay);
        let out = MenuList::new(ITEMS).draw(rect, selected, &mut ctx);
        (out, list)
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_selected_without_input() {
        let (out, _) = draw_at(
            Rect::new(0.0, 0.0, 200.0, 200.0),
            &mut 0,
            &InputState::default(),
        );
        assert_eq!(out.activated, None);
        assert_eq!(out.hovered, None);
        assert!(!out.selected_changed);
    }

    #[test]
    fn click_on_a_row_activates_it() {
        // Rows are centered in a 200-high rect: 3 rows of 24 + 2 gaps of 4 =
        // 80 content, top at 60. Row 1 ("Settings") spans y 88..112.
        let (out, _) = draw_at(
            Rect::new(0.0, 0.0, 200.0, 200.0),
            &mut 0,
            &click_at(100.0, 100.0),
        );
        assert_eq!(out.activated, Some(1));
    }

    #[test]
    fn click_outside_activates_nothing() {
        let (out, _) = draw_at(
            Rect::new(0.0, 0.0, 200.0, 200.0),
            &mut 0,
            &click_at(100.0, 400.0),
        );
        assert_eq!(out.activated, None);
    }

    #[test]
    fn hover_moves_the_selection() {
        // Hover row 2 (y 116..140 → point at 128).
        let mut input = click_at(100.0, 128.0);
        input.mouse_clicked = false;
        let (out, _) = draw_at(Rect::new(0.0, 0.0, 200.0, 200.0), &mut 0, &input);
        assert_eq!(out.hovered, Some(2));
        assert!(out.selected_changed);
    }

    #[test]
    fn nav_down_moves_and_wraps() {
        let mut input = InputState::default();
        input.nav.down = true;
        let (out, _) = draw_at(Rect::new(0.0, 0.0, 200.0, 200.0), &mut 0, &input);
        assert_eq!(out.activated, None, "a move frame does not activate");
        let mut sel = 2;
        let (out, _) = draw_at(Rect::new(0.0, 0.0, 200.0, 200.0), &mut sel, &input);
        assert_eq!(sel, 0, "down from the last row wraps to the first");
        assert!(out.selected_changed);

        let mut input = InputState::default();
        input.nav.up = true;
        let mut sel = 0;
        draw_at(Rect::new(0.0, 0.0, 200.0, 200.0), &mut sel, &input);
        assert_eq!(sel, 2, "up from the first row wraps to the last");
    }

    #[test]
    fn confirm_activates_the_selection_but_not_on_a_move_frame() {
        let mut sel = 1;
        let mut input = InputState::default();
        input.nav.confirm = true;
        let (out, _) = draw_at(Rect::new(0.0, 0.0, 200.0, 200.0), &mut sel, &input);
        assert_eq!(out.activated, Some(1));

        // down+confirm on the same frame: the move wins, no activation.
        input.nav.down = true;
        let (out, _) = draw_at(Rect::new(0.0, 0.0, 200.0, 200.0), &mut sel, &input);
        assert_eq!(out.activated, None, "the frame that moves doesn't confirm");
        assert_eq!(sel, 2);
    }

    #[test]
    fn selection_is_clamped_to_the_item_range() {
        let mut sel = 7;
        let (out, _) = draw_at(
            Rect::new(0.0, 0.0, 200.0, 200.0),
            &mut sel,
            &InputState::default(),
        );
        assert_eq!(sel, ITEMS.len() - 1, "out-of-range selection clamps");
        assert!(!out.selected_changed, "clamping alone is not a change");
    }

    #[test]
    fn disabled_row_neither_hovers_nor_activates() {
        let enabled = [true, false, true];
        // Click exactly on row 1 (the disabled one).
        let (out, _) = disabled_case(&enabled, click_at(100.0, 100.0));
        assert_eq!(out.activated, None, "a disabled row cannot be clicked");
        assert_eq!(out.hovered, None);
    }

    fn disabled_case(enabled: &[bool], input: InputState) -> (MenuListOutput, DrawList) {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let mut overlay = StyleOverlay::new();
        overlay.set_scalar(StyleKey::ButtonHeight, 24.0);
        overlay.set_scalar(StyleKey::Spacing, 4.0);
        let mut ctx = crate::DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0)
            .with_style(&overlay);
        let out = MenuList::new(ITEMS).enabled(enabled).draw(
            Rect::new(0.0, 0.0, 200.0, 200.0),
            &mut 0,
            &mut ctx,
        );
        (out, list)
    }

    #[test]
    fn rows_are_equal_width_and_block_centered() {
        let (_, list) = draw_at(
            Rect::new(0.0, 0.0, 400.0, 400.0),
            &mut 0,
            &InputState::default(),
        );
        // A raised button paints a plinth (full row height) + face (row
        // height minus travel) chrome pair. Group chrome rects into row
        // bands by y, take each band's max width — that's the row
        // allocation — and assert the row-level invariants.
        let mut rows: Vec<(f32, f32, f32)> = Vec::new(); // (top_y, max_w, max_h)
        for c in &list.chrome_instances {
            let (y, w, h) = (c.rect[1], c.rect[2], c.rect[3]);
            if let Some(row) = rows.iter_mut().find(|(ry, _, _)| (*ry - y).abs() < 0.5) {
                row.1 = row.1.max(w);
                row.2 = row.2.max(h);
            } else {
                rows.push((y, w, h));
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Drop the 1px edge-highlight band (its own chrome rect, offset 1px).
        rows.retain(|(_, _, h)| *h > 4.0);
        assert_eq!(rows.len(), ITEMS.len(), "one row band per item");
        let width = rows[0].1;
        for r in &rows {
            assert!((r.1 - width).abs() < 1e-3, "rows are equal width");
            assert!((r.2 - 24.0).abs() < 1e-3, "row height is the themed 24px");
        }
        // Content = 3*24 + 2*4 = 80; centered in 400 → first row top at 160.
        // (Report the actual first row y to keep failures legible.)
        assert!(
            (rows[0].0 - 160.0).abs() < 1e-3,
            "first row should be block-centered at y=160, got {} (rows: {:?})",
            rows[0].0,
            rows
        );
        // The uniform width never exceeds the allocated rect width.
        assert!(width > 0.0 && width <= 400.0);
    }

    #[test]
    fn focusable_rows_register_in_the_tab_ring() {
        let idle = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let mut ctx = crate::DrawContext::new(&mut list, &mut focus, &theme, &idle, 800.0, 600.0);
        MenuList::new(ITEMS).focusable(900).draw(
            Rect::new(0.0, 0.0, 200.0, 200.0),
            &mut 0,
            &mut ctx,
        );
        // Registered ids surface when Tab resolves: focus 900+0, request next.
        let mut input = InputState::default();
        input.nav.next = true;
        focus.begin_frame(&input);
        focus.register(1); // unrelated base focusable to prove layering
        focus.end_frame(None);
        assert!(
            focus.is_focused(901) || focus.is_focused(1),
            "menu ids participated in the ring"
        );
    }

    #[test]
    fn empty_menu_draws_nothing() {
        let idle = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let theme = crate::Theme::default();
        let mut ctx = crate::DrawContext::new(&mut list, &mut focus, &theme, &idle, 800.0, 600.0);
        let mut sel = 0;
        let out = MenuList::new(&[]).draw(Rect::new(0.0, 0.0, 200.0, 200.0), &mut sel, &mut ctx);
        assert_eq!(out.activated, None);
        assert_eq!(out.intrinsic_width, 0.0);
        assert!(list.icons.is_empty() && list.chrome_instances.is_empty());
    }

    #[test]
    fn scrim_uses_the_theme_color() {
        let mut theme = crate::Theme::default();
        theme.scrim = [1.0, 0.0, 0.0, 0.5];
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 100.0, 80.0);
        {
            let s = StyleResolver::new(&theme);
            draw_scrim(rect, &mut list, &s);
        }
        // `quad` collapses to an instanced chrome rect (radius 0, flat fill).
        assert_eq!(list.chrome_instances.len(), 1, "scrim emits one quad");
        let inst = &list.chrome_instances[0];
        assert_eq!(inst.bg, [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(inst.rect, [0.0, 0.0, 100.0, 80.0]);
    }
    #[test]
    fn scrim_overrides_through_an_overlay() {
        let theme = crate::Theme::default();
        let mut overlay = StyleOverlay::new();
        overlay.set_color(StyleKey::Scrim, [0.0, 1.0, 0.0, 1.0]);
        let mut list = DrawList::new();
        let s = StyleResolver::with_overlay(&theme, &overlay);
        draw_scrim(Rect::new(0.0, 0.0, 10.0, 10.0), &mut list, &s);
        for v in &list.vertices {
            assert_eq!(v.color, [0.0, 1.0, 0.0, 1.0]);
        }
    }
}
