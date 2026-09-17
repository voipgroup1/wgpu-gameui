//! Radio button group widget.
//!
//! A mutually-exclusive set of options where exactly one is selected. The
//! selected index is **caller-owned** (passed in by value); [`RadioGroup::draw`]
//! returns `Some(new_index)` when the selection changed this frame (by click or
//! keyboard), else `None`.
//!
//! Like [`Checkbox`](super::Checkbox), each option is drawn from vector
//! primitives using [`Theme`](crate::Theme) colors, so it renders correctly with zero atlas
//! assets — a radio dot is never blank.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{StyleKey, StyleResolver};

use super::{DrawContext, DrawList, FocusId};

/// Gap (px) between an option's radio dot and its label. Matches the
/// checkbox box→label gap so the two widgets align.
const LABEL_GAP: f32 = 6.0;

/// A group of radio buttons — pick exactly one of `options`.
///
/// The whole group is a single Tab stop (one [`FocusId`]); arrow keys move the
/// selection *within* the group while it holds focus, matching native radio
/// semantics. Layout is vertical by default; [`horizontal`](Self::horizontal)
/// lays the options left-to-right.
///
/// # Example
/// ```ignore
/// // `selected` is caller-owned (e.g. stored in your app state).
/// if let Some(i) = RadioGroup::new(&["Low", "Medium", "High"])
///     .focusable(QUALITY)
///     .draw(selected, rect, &mut ctx)
/// {
///     selected = i;
/// }
/// ```
#[derive(Clone)]
pub struct RadioGroup<'a> {
    options: &'a [&'a str],
    /// When set, the group joins the Tab ring under this id, draws a focus ring
    /// on the selected option, and moves the selection with the arrow keys.
    focus_id: Option<FocusId>,
    /// Lay options left-to-right instead of top-to-bottom.
    horizontal: bool,
    /// Gap between options (px). Defaults to [`Theme::spacing`](crate::Theme::spacing).
    spacing: Option<f32>,
}

impl<'a> RadioGroup<'a> {
    /// A vector-drawn radio group over `options` (no atlas assets required).
    pub fn new(options: &'a [&'a str]) -> Self {
        Self {
            options,
            focus_id: None,
            horizontal: false,
            spacing: None,
        }
    }

    /// Make the group keyboard-focusable under `id`: it joins the Tab ring,
    /// draws a focus ring on the selected option, and moves the selection with
    /// Up/Down (vertical) or Left/Right (horizontal) while focused. Clicking an
    /// option also moves focus to the group.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// Lay the options left-to-right instead of stacked top-to-bottom.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Override the gap (px) between options. Defaults to [`Theme::spacing`](crate::Theme::spacing).
    pub fn spacing(mut self, gap: f32) -> Self {
        self.spacing = Some(gap);
        self
    }

    /// Diameter of the radio dot for `theme` — fitted to the label line height,
    /// matching the checkbox box size so the two widgets align in a column.
    fn diameter(s: &StyleResolver) -> f32 {
        s.scalar(StyleKey::FontSize).max(20.0)
    }

    /// Compute the interactive cell rect for option `i`. Cells tile the group
    /// along the layout axis; each is the click target for its option.
    ///
    /// Returns `None` for out-of-range `i`. Horizontal cell widths are measured
    /// from the label text (via `list.measure_text`), so this needs `&mut
    /// DrawList`.
    fn cell_rect(
        &self,
        i: usize,
        rect: Rect,
        s: &StyleResolver,
        list: &mut DrawList,
    ) -> Option<Rect> {
        if i >= self.options.len() {
            return None;
        }
        let diameter = Self::diameter(s);
        let gap = self.spacing.unwrap_or_else(|| s.scalar(StyleKey::Spacing));
        if self.horizontal {
            // Walk left-to-right, summing each prior option's measured width.
            let mut x = rect.x;
            for opt in &self.options[..i] {
                x += self.h_cell_width(opt, diameter, s, list) + gap;
            }
            let w = self.h_cell_width(self.options[i], diameter, s, list);
            Some(Rect::new(x, rect.y, w, rect.height))
        } else {
            let row_h = diameter;
            let y = rect.y + i as f32 * (row_h + gap);
            Some(Rect::new(rect.x, y, rect.width, row_h))
        }
    }

    /// The size this group needs to lay its options out without spilling, as
    /// `(width, height)`. Pass the result straight to [`draw`](Self::draw), or
    /// use it as the minimum when the surrounding layout has room to spare.
    ///
    /// A group's extent is not something a caller can derive: it depends on the
    /// dot diameter (fitted to the theme font size), the inter-option gap, and —
    /// horizontally — the *shaped* width of every label. Guessing it is how a
    /// group ends up 8px short of its last option.
    ///
    /// - **Vertical** (the default): `n` rows of one dot each, so the height is
    ///   `n·diameter + (n−1)·gap` and the width is the widest dot+label row.
    /// - **Horizontal**: one row, so the width is the sum of the option cells
    ///   plus the gaps between them, and the height is one dot.
    pub fn measure(&self, list: &mut DrawList, s: &StyleResolver) -> (f32, f32) {
        let diameter = Self::diameter(s);
        let gap = self.spacing.unwrap_or_else(|| s.scalar(StyleKey::Spacing));
        let n = self.options.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        let cells = self
            .options
            .iter()
            .map(|o| self.h_cell_width(o, diameter, s, list));
        if self.horizontal {
            let total: f32 = cells.sum::<f32>() + gap * (n - 1) as f32;
            (total, diameter)
        } else {
            let widest = cells.fold(0.0_f32, f32::max);
            (widest, n as f32 * diameter + gap * (n - 1) as f32)
        }
    }

    /// Width of a horizontal option cell: dot + gap + measured label width.
    fn h_cell_width(
        &self,
        label: &str,
        diameter: f32,
        s: &StyleResolver,
        list: &mut DrawList,
    ) -> f32 {
        let label_w = if label.is_empty() {
            0.0
        } else {
            LABEL_GAP
                + list
                    .measure_text(label, s.scalar(StyleKey::FontSize), None)
                    .0
        };
        diameter + label_w
    }

    /// Draw the group with the given `selected` index. Returns `Some(i)` if the
    /// selection changed to option `i` this frame (click or keyboard), else
    /// `None`.
    pub fn draw(&self, selected: usize, rect: Rect, ctx: &mut DrawContext) -> Option<usize> {
        ctx.push_debug_scope_rect("RadioGroup", rect);
        let input = ctx.input;
        let s = ctx.styles();
        let diameter = Self::diameter(&s);
        let radius = diameter * 0.4;
        let inner_radius = radius * 0.5;
        let border = s.scalar(StyleKey::BorderWidth).max(1.0).min(radius);

        // Honor layer capture so a group under a modal/popup ignores clicks
        // meant for the overlay.
        let mouse_live = !input.mouse_consumed;

        let mut result: Option<usize> = None;
        let mut focus_circle: Option<(f32, f32)> = None;

        for i in 0..self.options.len() {
            let cell = match self.cell_rect(i, rect, &s, ctx.draw_list) {
                Some(c) => c,
                None => continue,
            };
            let hovered = mouse_live && cell.contains(input.mouse_x, input.mouse_y);
            if hovered && input.mouse_clicked && i != selected {
                result = Some(i);
            }

            let cx = cell.x + diameter / 2.0;
            let cy = cell.y + cell.height / 2.0;
            if i == selected {
                focus_circle = Some((cx, cy));
            }

            let list = &mut *ctx.draw_list;
            // 4a: unselected = sunken dark trough with a black edge; selected =
            // the accent face with a dark on-accent dot.
            if i == selected {
                list.circle((cx, cy), radius, s.color(StyleKey::Accent));
                list.circle_outline((cx, cy), radius, border, [0.0, 0.0, 0.0, 0.5]);
                list.circle((cx, cy), inner_radius, s.color(StyleKey::OnAccent));
            } else {
                list.circle((cx, cy), radius, s.color(StyleKey::InputBackground));
                list.circle_outline((cx, cy), radius, border, [0.0, 0.0, 0.0, 0.6]);
            }
            // Hover highlight over the whole cell.
            if hovered && i != selected {
                list.quad(
                    cell.x,
                    cell.y,
                    cell.width,
                    cell.height,
                    [1.0, 1.0, 1.0, 0.06],
                );
            }

            // Label to the right of the dot.
            let label = self.options[i];
            if !label.is_empty() {
                let text_x = cell.x + diameter + LABEL_GAP;
                let font_size = s.scalar(StyleKey::FontSize);
                let text_y = list.vcentered_text_y(
                    cell.y,
                    cell.height,
                    font_size,
                    s.theme().font.as_ref(),
                    label,
                );
                let c = s.color(StyleKey::Text);
                let text = TextBlock::new(label, text_x, text_y)
                    .with_size(font_size)
                    .with_color(
                        (c[0] * 255.0) as u8,
                        (c[1] * 255.0) as u8,
                        (c[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone());
                list.text(text);
            }
        }

        // Keyboard focus + arrow navigation (opt-in via `focusable`).
        if let Some(id) = self.focus_id {
            ctx.register_focus(id);
            if result.is_some() {
                ctx.focus.request(id);
            }
            if ctx.focus.is_focused(id) {
                // Arrow keys move the selection along the layout axis (clamped,
                // no wrap). The selection base is the pending click result if
                // one happened this frame, else the incoming `selected`.
                let base = result.unwrap_or(selected);
                let (prev, next) = if self.horizontal {
                    (input.nav.left, input.nav.right)
                } else {
                    (input.nav.up, input.nav.down)
                };
                let n = self.options.len();
                if n > 0 {
                    if prev && base > 0 {
                        result = Some(base - 1);
                    } else if next && base + 1 < n {
                        result = Some(base + 1);
                    }
                }
                // Focus ring hugs the selected option's dot.
                if let Some((cx, cy)) = focus_circle {
                    ctx.draw_list.circle_outline(
                        (cx, cy),
                        radius + 3.0,
                        2.0,
                        s.color(StyleKey::FocusRing),
                    );
                }
            }
        }

        ctx.pop_debug_scope();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    const OPTS: [&str; 3] = ["Low", "Medium", "High"];

    fn theme() -> Theme {
        Theme::default()
    }

    /// Draw a group into a fresh context and return the populated draw list plus
    /// the selection result.
    fn draw_group(
        group: &RadioGroup,
        selected: usize,
        rect: Rect,
        input: &InputState,
    ) -> (DrawList, Option<usize>) {
        draw_group_focus(group, selected, rect, input, &mut FocusState::new())
    }

    /// Draw with an explicitly seeded focus state.
    fn draw_group_focus(
        group: &RadioGroup,
        selected: usize,
        rect: Rect,
        input: &InputState,
        focus: &mut FocusState,
    ) -> (DrawList, Option<usize>) {
        let mut list = DrawList::new();
        let theme = theme();
        let result = {
            let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
            group.draw(selected, rect, &mut ctx)
        };
        (list, result)
    }

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
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

    fn rect() -> Rect {
        // Tall enough for 3 vertical rows (row_h 20 + spacing each).
        Rect::new(0.0, 0.0, 160.0, 120.0)
    }

    /// Center of vertical option `i`'s row (for click targeting).
    fn row_center(i: usize) -> (f32, f32) {
        let th = theme();
        let diameter = RadioGroup::diameter(&StyleResolver::new(&th));
        let gap = th.spacing;
        let y = i as f32 * (diameter + gap) + diameter / 2.0;
        (5.0, y)
    }

    /// The measured size must actually contain every option — this is the whole
    /// point of the method, and the gallery shipped a hand-guessed height that
    /// cut the third row off until layout inspection caught it.
    #[test]
    fn measured_size_contains_every_option() {
        for horizontal in [false, true] {
            let mut g = RadioGroup::new(&OPTS);
            if horizontal {
                g = g.horizontal();
            }
            let th = theme();
            let s = StyleResolver::new(&th);
            let mut list = DrawList::new();
            let (w, h) = g.measure(&mut list, &s);
            let rect = Rect::new(10.0, 10.0, w, h);

            let last = g
                .cell_rect(OPTS.len() - 1, rect, &s, &mut list)
                .expect("last option has a cell");
            assert!(
                last.right() <= rect.right() + 0.01 && last.bottom() <= rect.bottom() + 0.01,
                "horizontal={horizontal}: last cell {last:?} escapes measured {rect:?}"
            );
        }
    }

    /// Growing the gap has to grow the measured extent, or the measurement is
    /// just the default theme's answer wearing a method signature.
    #[test]
    fn measured_size_tracks_the_configured_gap() {
        let th = theme();
        let s = StyleResolver::new(&th);
        let mut list = DrawList::new();
        let tight = RadioGroup::new(&OPTS).spacing(0.0).measure(&mut list, &s);
        let loose = RadioGroup::new(&OPTS).spacing(20.0).measure(&mut list, &s);
        // Two gaps between three options.
        assert!(
            (loose.1 - tight.1 - 40.0).abs() < 0.01,
            "vertical height should grow by 2 x 20px: {tight:?} -> {loose:?}"
        );
        assert_eq!(tight.0, loose.0, "vertical width is gap-independent");
    }

    #[test]
    fn empty_group_measures_to_nothing() {
        let th = theme();
        let s = StyleResolver::new(&th);
        let mut list = DrawList::new();
        let empty: [&str; 0] = [];
        assert_eq!(RadioGroup::new(&empty).measure(&mut list, &s), (0.0, 0.0));
    }

    #[test]
    fn vector_group_emits_geometry_no_icons() {
        let g = RadioGroup::new(&OPTS);
        let (list, _) = draw_group(&g, 0, rect(), &input_at(-1.0, -1.0));
        assert!(
            !list.circle_instances.is_empty(),
            "vector radios must emit circle geometry"
        );
        assert!(list.icons.is_empty(), "vector path must not queue icons");
    }

    #[test]
    fn selected_option_adds_inner_dot() {
        let g = RadioGroup::new(&OPTS);
        let idle = input_at(-1.0, -1.0);
        // Three options: each contributes a fill disc + an outline ring. The
        // selected one adds a third disc (the accent dot), so selecting any
        // option yields strictly more circle instances than selecting none
        // would — compare against a one-option group with no valid selection.
        let (sel, _) = draw_group(&g, 1, rect(), &idle);
        let none = RadioGroup::new(&OPTS);
        let (unsel, _) = draw_group(&none, 99, rect(), &idle); // out-of-range = nothing selected
        assert!(
            sel.circle_instances.len() > unsel.circle_instances.len(),
            "a selected option should add the inner accent dot"
        );
    }

    #[test]
    fn click_selects_option() {
        let g = RadioGroup::new(&OPTS);
        let (x, y) = row_center(2);
        let (_, r) = draw_group(&g, 0, rect(), &click_at(x, y));
        assert_eq!(r, Some(2), "clicking option 2 selects it");
    }

    #[test]
    fn click_already_selected_returns_none() {
        let g = RadioGroup::new(&OPTS);
        let (x, y) = row_center(1);
        let (_, r) = draw_group(&g, 1, rect(), &click_at(x, y));
        assert_eq!(r, None, "clicking the already-selected option is a no-op");
    }

    #[test]
    fn click_outside_returns_none() {
        let g = RadioGroup::new(&OPTS);
        let (_, r) = draw_group(&g, 0, rect(), &click_at(500.0, 500.0));
        assert_eq!(r, None);
    }

    #[test]
    fn consumed_mouse_suppresses_selection() {
        let g = RadioGroup::new(&OPTS);
        let (x, y) = row_center(2);
        let mut input = click_at(x, y);
        input.mouse_consumed = true;
        let (_, r) = draw_group(&g, 0, rect(), &input);
        assert_eq!(r, None);
    }

    // ---- Keyboard navigation ----

    fn arrows(up: bool, down: bool) -> InputState {
        let mut s = InputState {
            key_up: up,
            key_down: down,
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..Default::default()
        };
        crate::map_keyboard(&mut s); // arrows → nav directional
        s
    }

    fn arrows_h(left: bool, right: bool) -> InputState {
        let mut s = InputState {
            key_left: left,
            key_right: right,
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..Default::default()
        };
        crate::map_keyboard(&mut s); // arrows → nav directional
        s
    }

    #[test]
    fn arrow_down_moves_selection_when_focused() {
        let g = RadioGroup::new(&OPTS).focusable(1);
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, r) = draw_group_focus(&g, 0, rect(), &arrows(false, true), &mut focus);
        assert_eq!(r, Some(1));
    }

    #[test]
    fn arrow_up_moves_selection_when_focused() {
        let g = RadioGroup::new(&OPTS).focusable(1);
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, r) = draw_group_focus(&g, 2, rect(), &arrows(true, false), &mut focus);
        assert_eq!(r, Some(1));
    }

    #[test]
    fn arrows_clamp_at_ends() {
        let g = RadioGroup::new(&OPTS).focusable(1);
        let mut focus = FocusState::new();
        focus.focus(1);
        // Up at first option: clamp (no change, no wrap).
        let (_, r) = draw_group_focus(&g, 0, rect(), &arrows(true, false), &mut focus);
        assert_eq!(r, None, "up at the top must not wrap");
        // Down at last option: clamp.
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, r) = draw_group_focus(&g, 2, rect(), &arrows(false, true), &mut focus);
        assert_eq!(r, None, "down at the bottom must not wrap");
    }

    #[test]
    fn arrows_ignored_when_not_focused() {
        let g = RadioGroup::new(&OPTS).focusable(1);
        let mut focus = FocusState::new(); // not focused
        let (_, r) = draw_group_focus(&g, 0, rect(), &arrows(false, true), &mut focus);
        assert_eq!(r, None, "arrows do nothing without focus");
    }

    #[test]
    fn arrows_ignored_without_focusable() {
        let g = RadioGroup::new(&OPTS); // not focusable
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, r) = draw_group_focus(&g, 0, rect(), &arrows(false, true), &mut focus);
        assert_eq!(r, None);
    }

    #[test]
    fn horizontal_uses_left_right() {
        let g = RadioGroup::new(&OPTS).horizontal().focusable(1);
        let hrect = Rect::new(0.0, 0.0, 400.0, 24.0);
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, r) = draw_group_focus(&g, 0, hrect, &arrows_h(false, true), &mut focus);
        assert_eq!(r, Some(1));
        // Up/Down do nothing in horizontal mode.
        let mut focus = FocusState::new();
        focus.focus(1);
        let (_, r) = draw_group_focus(&g, 0, hrect, &arrows(false, true), &mut focus);
        assert_eq!(r, None, "vertical arrows are inert in horizontal mode");
    }

    #[test]
    fn click_requests_focus() {
        let g = RadioGroup::new(&OPTS).focusable(7);
        let mut focus = FocusState::new();
        let (x, y) = row_center(1);
        let _ = draw_group_focus(&g, 0, rect(), &click_at(x, y), &mut focus);
        assert!(focus.is_focused(7), "clicking a focusable group focuses it");
    }

    #[test]
    fn focus_ring_only_when_focused() {
        let g = RadioGroup::new(&OPTS).focusable(1);
        let idle = input_at(-1.0, -1.0);
        let (unfocused, _) = draw_group_focus(&g, 0, rect(), &idle, &mut FocusState::new());
        let mut focus = FocusState::new();
        focus.focus(1);
        let (focused, _) = draw_group_focus(&g, 0, rect(), &idle, &mut focus);
        assert!(
            focused.circle_instances.len() > unfocused.circle_instances.len(),
            "focus ring should add a circle outline when focused"
        );
    }
}
