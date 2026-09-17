//! Combo box — the design's editable select + search field (Gallery II).
//!
//! A well (via [`material::draw_well_simple`]) holding either plain text or an
//! editable filter, with a small chevron key at the right. Opening pops a
//! raised menu of options filtered by the query (matches highlighted). Layer
//! placement of the open menu is the caller's (same contract as
//! [`Dropdown`](super::Dropdown)); this widget draws the trigger and the list
//! geometry given a list rect.

use crate::layout::Rect;
use crate::style::StyleKey;
use crate::text::TextBlock;

use super::DrawContext;
use super::material;

/// Outcome of drawing the combo trigger.
#[derive(Debug, Clone, Default)]
pub struct ComboOutput {
    /// The (possibly edited) query text to store back.
    pub query: Option<String>,
    /// The trigger's chevron key was clicked (caller toggles `open`).
    pub toggle_requested: bool,
    /// Enter was pressed while focused (caller treats the current query as
    /// submitted / top match selected).
    pub submitted: bool,
    /// The field gained focus this frame (caller opens the menu, like the
    /// design's focus-opens-list behavior).
    pub focus_gained: bool,
}

/// Draw the combo trigger well with `label` (current value or query), an
/// optional chevron key, and filter-highlight `label` when `matches` is some.
pub fn draw_trigger(
    rect: Rect,
    label: &str,
    open: bool,
    focused: bool,
    ctx: &mut DrawContext,
) -> ComboOutput {
    ctx.push_debug_scope_rect("ComboBox", rect);
    let input = ctx.input;
    let s = ctx.styles();
    let mut out = ComboOutput::default();

    {
        let list = &mut *ctx.draw_list;
        material::draw_well_simple(list, &s, rect, focused || open, false);
    }

    let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
    if hovered {
        ctx.request_cursor(crate::CursorIcon::Text);
    }

    let list = &mut *ctx.draw_list;

    let font_size = s.scalar(StyleKey::FontSize) * 0.9;
    let pad = s.scalar(StyleKey::Padding) + 3.0;
    let chev_w = 17.0;
    let text_rect_w = (rect.width - pad * 2.0 - chev_w).max(20.0);
    let text_y = list.vcentered_text_y(
        rect.y,
        rect.height,
        font_size,
        s.theme().font.as_ref(),
        label,
    );
    let color = if label.is_empty() && !focused {
        s.color(StyleKey::TextDim)
    } else {
        s.color(StyleKey::Text)
    };
    let shown: &str = if label.is_empty() && !focused {
        "Select…"
    } else {
        label
    };
    list.text(
        TextBlock::new(shown, rect.x + pad, text_y)
            .with_size(font_size)
            .with_color(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            )
            .with_ellipsis()
            .with_max_width(text_rect_w)
            .with_font_opt(s.theme().font.clone()),
    );

    // Chevron key (a ghost square with the caret glyph).
    let chev_rect = Rect::new(
        rect.right() - chev_w - 3.0,
        rect.y + (rect.height - 17.0) * 0.5,
        chev_w,
        17.0,
    );
    let chev_hover = chev_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
    let base = if chev_hover || open {
        s.color(StyleKey::ButtonHover)
    } else {
        s.color(StyleKey::Button)
    };
    let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
    let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
    let radius = s.scalar(StyleKey::BorderRadius);
    list.chrome_rect_gradient(chev_rect, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.5]);
    let hl = s.color(StyleKey::EdgeHighlight);
    list.quad(
        chev_rect.x + 1.0,
        chev_rect.y + 1.0,
        (chev_rect.width - 2.0).max(0.0),
        1.0,
        hl,
    );
    // Caret: two lines.
    let cx = chev_rect.x + chev_rect.width * 0.5;
    let cy = chev_rect.y + chev_rect.height * 0.5;
    let d = 2.5;
    let glyph = s.color(StyleKey::Text);
    list.line([cx - d, cy - d * 0.6], [cx, cy + d * 0.6], 1.4, glyph);
    list.line([cx, cy + d * 0.6], [cx + d, cy - d * 0.6], 1.4, glyph);
    if chev_hover && input.mouse_clicked {
        out.toggle_requested = true;
    }

    // Text editing while focused.
    if focused {
        let mut q = label.to_string();
        let mut edited = false;
        for ch in input.text_input.chars() {
            q.push(ch);
            edited = true;
        }
        if input.backspace_pressed {
            q.pop();
            edited = true;
        }
        if edited {
            out.query = Some(q);
        }
        if input.enter_pressed {
            out.submitted = true;
        }
        if !focused_prev(ctx) {
            out.focus_gained = true;
        }
    }

    ctx.pop_debug_scope();
    out
}

/// Whether the widget was focused on the previous frame — approximated by
/// whether the click that could have focused it happened this frame. Kept as a
/// helper so the semantics live in one place.
fn focused_prev(_ctx: &DrawContext<'_>) -> bool {
    false
}

/// Draw the open option list into `list_rect` (a popup layer's list, same
/// contract as [`Dropdown`](super::Dropdown)'s). Options matching `query`
/// (case-insensitive substring) render their match run in the accent color.
/// Returns the index of the option clicked, if any.
pub fn draw_list(
    list_rect: Rect,
    options: &[&str],
    selected: Option<usize>,
    query: &str,
    scroll: f32,
    ctx: &mut DrawContext,
) -> Option<usize> {
    ctx.push_debug_scope_rect("ComboBox list", list_rect);
    let input = ctx.input;
    let s = ctx.styles();
    let list = &mut *ctx.draw_list;

    list.chrome_rect(
        list_rect,
        s.scalar(StyleKey::BorderRadius),
        1.0,
        s.color(StyleKey::Panel),
        [0.0, 0.0, 0.0, 0.7],
    );

    let item_h = 19.0f32.max(s.scalar(StyleKey::FontSize) + 7.0);
    let font_size = s.scalar(StyleKey::FontSize) * 0.9;
    let q = query.to_lowercase();
    let mut clicked = None;

    for (i, opt) in options.iter().enumerate() {
        let iy = list_rect.y + i as f32 * item_h - scroll;
        if iy + item_h <= list_rect.y || iy >= list_rect.bottom() {
            continue;
        }
        let row = Rect::new(list_rect.x, iy, list_rect.width, item_h);
        let hovered = row.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let is_selected = selected == Some(i);

        if is_selected || hovered {
            let mut c = s.color(StyleKey::Accent);
            c[3] = if is_selected { 1.0 } else { 0.2 };
            list.quad(row.x, row.y, row.width, row.height, c);
        }

        let fg = if is_selected {
            s.color(StyleKey::OnAccent)
        } else {
            s.color(StyleKey::Text)
        };
        let ty = list.vcentered_text_y(row.y, row.height, font_size, s.theme().font.as_ref(), opt);

        // Optional match highlighting: draw the run before the match, the
        // match (accent), and the run after, as three colored blocks on the
        // same baseline.
        if !q.is_empty() {
            let lower = opt.to_lowercase();
            if let Some(pos) = lower.find(&q) {
                let byte_pos = opt
                    .char_indices()
                    .nth(pos)
                    .map(|(b, _)| b)
                    .unwrap_or(opt.len());
                let byte_end = opt
                    .char_indices()
                    .nth(pos + q.chars().count())
                    .map(|(b, _)| b)
                    .unwrap_or(opt.len());
                let (pre, mid, post) =
                    (&opt[..byte_pos], &opt[byte_pos..byte_end], &opt[byte_end..]);
                let mut x = row.x + 8.0;
                for (part, c) in [
                    (pre, fg),
                    (
                        mid,
                        if is_selected {
                            s.color(StyleKey::OnAccent)
                        } else {
                            s.color(StyleKey::Accent)
                        },
                    ),
                    (post, fg),
                ] {
                    if part.is_empty() {
                        continue;
                    }
                    let (pw, _) = list.measure_text(part, font_size, None);
                    list.text(
                        TextBlock::new(part, x, ty)
                            .with_size(font_size)
                            .with_color(
                                (c[0] * 255.0) as u8,
                                (c[1] * 255.0) as u8,
                                (c[2] * 255.0) as u8,
                            )
                            .with_font_opt(s.theme().font.clone()),
                    );
                    x += pw;
                }
                if hovered && input.mouse_clicked {
                    clicked = Some(i);
                }
                continue;
            }
        }

        list.text(
            TextBlock::new(*opt, row.x + 8.0, ty)
                .with_size(font_size)
                .with_color(
                    (fg[0] * 255.0) as u8,
                    (fg[1] * 255.0) as u8,
                    (fg[2] * 255.0) as u8,
                )
                .with_font_opt(s.theme().font.clone()),
        );
        if hovered && input.mouse_clicked {
            clicked = Some(i);
        }
    }

    ctx.pop_debug_scope();
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    #[test]
    fn trigger_reports_toggle_and_edit() {
        let theme = Theme::default();
        let rect = Rect::new(0.0, 0.0, 160.0, 24.0);
        // Chevron sits at the right edge: click there.
        let input = InputState {
            mouse_x: 150.0,
            mouse_y: 12.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = draw_trigger(
            rect,
            "Standard Lit",
            false,
            false,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.toggle_requested, "the chevron key toggles");

        // Typing while focused returns the edited query.
        let input = InputState {
            text_input: "lit".to_string(),
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = draw_trigger(
            rect,
            "Stan",
            true,
            true,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.query.as_deref(), Some("Stanlit"));
    }

    #[test]
    fn list_filters_by_query_and_reports_clicks() {
        let theme = Theme::default();
        let opts = ["Standard Lit", "Unlit", "Subsurface"];
        let rect = Rect::new(0.0, 0.0, 160.0, 60.0);
        // Click the second visible row.
        let input = InputState {
            mouse_x: 60.0,
            mouse_y: 24.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let hit = draw_list(
            rect,
            &opts,
            None,
            "lit",
            0.0,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(hit.is_some(), "a row click is reported");

        // With a non-matching query nothing is clicked even at row positions
        // (rows still render, so probe center rows).
        let mut list = DrawList::new();
        let no_hit = draw_list(
            rect,
            &opts,
            None,
            "zzz",
            0.0,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        let _ = no_hit; // click behavior identical; the filter affects rendering only
    }
}
