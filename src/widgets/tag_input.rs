//! Tag input — the design's chip-list text field (Gallery III "tag input").
//!
//! A well containing removable chips followed by an inline text field; Enter
//! commits the draft as a new tag. All state (the tag list, the draft text)
//! is caller-owned; the widget is a pure draw + interaction reporter.

use crate::layout::Rect;
use crate::style::StyleKey;
use crate::text::TextBlock;

use super::DrawContext;
use super::material;

/// Outcome of drawing the tag input.
#[derive(Debug, Clone, Default)]
pub struct TagOutput {
    /// Index of a tag whose ✕ was clicked this frame (caller removes it).
    pub removed: Option<usize>,
    /// A tag was committed this frame (Enter while the draft is non-empty).
    /// The draft itself is caller-owned; read it via the returned flag.
    pub committed: bool,
    /// The (possibly edited) draft text to store back.
    pub draft: Option<String>,
}

/// Draw the tag field.
///
/// * `tags` — the committed labels, in order.
/// * `draft` — the current draft text (caller-owned; edited in place).
/// * `focused` — whether the field holds keyboard focus (drives the accent
///   border, exactly like a text well).
///
/// The widget re-measures and re-flows chips every frame (immediate mode);
/// `draft` round-trips through [`TagOutput::draft`] when it changed.
pub fn draw(
    rect: Rect,
    tags: &[String],
    draft: &mut String,
    focused: bool,
    ctx: &mut DrawContext,
) -> TagOutput {
    ctx.push_debug_scope_rect("TagInput", rect);
    let input = ctx.input;
    let s = ctx.styles();
    let mut out = TagOutput::default();

    // The well.
    {
        let list = &mut *ctx.draw_list;
        material::draw_well_simple(list, &s, rect, focused, false);
    }

    // Click anywhere in the field focuses it (a scratch FocusId allocated by
    // hashing the rect so the widget stays stateless).
    let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
    if hovered {
        ctx.request_cursor(crate::CursorIcon::Text);
    }
    if hovered && input.mouse_clicked {
        let click_focus: crate::FocusId =
            crate::widgets::material::focus_id_for("tag_input", &rect);
        ctx.focus.request(click_focus);
    }

    let list = &mut *ctx.draw_list;

    let font_size = s.scalar(StyleKey::FontSize) * 0.85;
    let pad = s.scalar(StyleKey::Padding) + 2.0;
    let chip_h = 17.0f32.max(font_size + 6.0);
    let gap = 4.0;
    let inner_w = rect.width - pad * 2.0;

    let mut x = rect.x + pad;
    let mut y = rect.y + pad;

    for (i, tag) in tags.iter().enumerate() {
        let (tw, _) = list.measure_text(tag, font_size, None);
        let chip_w = (tw + 14.0 + 12.0).min(inner_w);
        if x + chip_w > rect.right() - pad && x > rect.x + pad {
            // Wrap to the next row.
            x = rect.x + pad;
            y += chip_h + gap;
        }
        let chip_rect = Rect::new(x, y, chip_w, chip_h);
        let radius = 9.0f32.min(chip_h * 0.5);

        // Chip: raised face pill.
        let base = s.color(StyleKey::Button);
        let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
        let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
        list.chrome_rect_gradient(chip_rect, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.55]);
        // Like standalone chips, rely on the curved face gradient for sheen.
        // A straight top-edge band cuts across the pill caps as a white slash.

        let fg = s.color(StyleKey::Text);
        let ty = list.vcentered_text_y(
            chip_rect.y,
            chip_rect.height,
            font_size,
            s.theme().font.as_ref(),
            tag,
        );
        list.text(
            TextBlock::new(tag.as_str(), chip_rect.x + 7.0, ty)
                .with_size(font_size)
                .with_color(
                    (fg[0] * 255.0) as u8,
                    (fg[1] * 255.0) as u8,
                    (fg[2] * 255.0) as u8,
                )
                .with_ellipsis()
                .with_max_width(chip_w - 14.0 - 12.0)
                .with_font_opt(s.theme().font.clone()),
        );

        // ✕ affordance at the chip's right edge.
        let x_rect = Rect::new(
            chip_rect.right() - 14.0,
            chip_rect.y,
            14.0,
            chip_rect.height,
        );
        let xh = x_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        if xh {
            // Hover wash + removal click (emit the wash via the same `list`
            // borrow — no `ctx` re-borrow inside the loop).
            list.quad(
                x_rect.x,
                x_rect.y,
                x_rect.width,
                x_rect.height,
                [1.0, 1.0, 1.0, 0.09],
            );
            if input.mouse_clicked {
                out.removed = Some(i);
            }
        }
        let xc = s.color(StyleKey::TextDim);
        let x_size = font_size * 0.9;
        let (xw, _) = list.measure_text("✕", x_size, None);
        let x_y = list.vcentered_text_y(
            x_rect.y,
            x_rect.height,
            x_size,
            s.theme().font.as_ref(),
            "✕",
        );
        list.text(
            TextBlock::new("✕", x_rect.x + (x_rect.width - xw) * 0.5, x_y)
                .with_size(x_size)
                .with_color(
                    (xc[0] * 255.0) as u8,
                    (xc[1] * 255.0) as u8,
                    (xc[2] * 255.0) as u8,
                )
                .with_font_opt(s.theme().font.clone()),
        );

        x += chip_w + gap;
    }

    // The draft text field inline after the last chip.
    {
        if x + 60.0 > rect.right() - pad {
            x = rect.x + pad;
            y += chip_h + gap;
        }
        let draft_rect = Rect::new(x, y, (rect.right() - pad - x).max(40.0), chip_h);
        let fg = if draft.is_empty() && !focused {
            s.color(StyleKey::TextDim)
        } else {
            s.color(StyleKey::Text)
        };
        let ty = list.vcentered_text_y(
            draft_rect.y,
            draft_rect.height,
            font_size,
            s.theme().font.as_ref(),
            draft,
        );
        let shown: &str = if draft.is_empty() && !focused {
            "add tag…"
        } else {
            draft.as_str()
        };
        list.text(
            TextBlock::new(shown, draft_rect.x, ty)
                .with_size(font_size)
                .with_color(
                    (fg[0] * 255.0) as u8,
                    (fg[1] * 255.0) as u8,
                    (fg[2] * 255.0) as u8,
                )
                .with_ellipsis()
                .with_max_width(draft_rect.width)
                .with_font_opt(s.theme().font.clone()),
        );
        // Caret while focused.
        if focused {
            let (cw, _) = list.measure_text(draft, font_size, None);
            let cx = draft_rect.x + cw + 1.0;
            // Steady caret; blink is the app's clock.
            let caret = s.color(StyleKey::Text);
            list.quad(cx, ty, 1.0, font_size, [caret[0], caret[1], caret[2], 0.9]);
        }
    }

    // Draft editing: printable characters append, Backspace deletes, Enter
    // commits. Keyboard arrives via the standard InputState text fields.
    if focused {
        for ch in input.text_input.chars() {
            draft.push(ch);
            out.draft = Some(draft.clone());
        }
        if input.backspace_pressed {
            draft.pop();
            out.draft = Some(draft.clone());
        }
        if input.enter_pressed && !draft.is_empty() {
            out.committed = true;
        }
    }

    ctx.pop_debug_scope();
    out
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
    fn clicking_the_x_reports_a_removal() {
        let theme = Theme::default();
        let tags = vec!["occlusion".to_string(), "static".to_string()];
        // Probe a horizontal strip across the chip row: some probe must land
        // on a ✕ affordance and report its tag index.
        for probe_x in 40..240 {
            let input = InputState {
                mouse_x: probe_x as f32,
                mouse_y: 8.0,
                mouse_clicked: true,
                mouse_down: true,
                ..Default::default()
            };
            let mut list = DrawList::new();
            let mut focus = FocusState::new();
            let mut d = String::new();
            let out = draw(
                Rect::new(0.0, 0.0, 260.0, 26.0),
                &tags,
                &mut d,
                false,
                &mut ctx(&mut list, &mut focus, &theme, &input),
            );
            if let Some(i) = out.removed {
                assert!(i < tags.len());
                return;
            }
        }
        panic!("no ✕ hit in the chip strip");
    }

    #[test]
    fn tag_chips_use_gradient_sheen_without_a_straight_top_highlight() {
        let theme = Theme::default();
        let tags = vec!["occlusion".to_string()];
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut draft = String::new();
        draw(
            Rect::new(0.0, 0.0, 200.0, 26.0),
            &tags,
            &mut draft,
            false,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        assert!(
            !list.chrome_instances.iter().any(|instance| {
                instance.rect[1] == theme.padding + 3.0
                    && instance.rect[3] == 1.0
                    && instance.rect[2] > 10.0
            }),
            "tag chip must not paint a straight top highlight over its curved pill face"
        );
    }

    #[test]
    fn tag_remove_cross_is_centered_in_its_hit_area() {
        let theme = Theme::default();
        let tags = vec!["occlusion".to_string()];
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut draft = String::new();
        draw(
            Rect::new(0.0, 0.0, 200.0, 26.0),
            &tags,
            &mut draft,
            false,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let cross_y = list
            .texts
            .iter()
            .find(|text| text.content == "✕")
            .expect("tag chip emits a remove cross")
            .y;
        let font_size = theme.font_size * 0.85;
        let chip_h = 17.0f32.max(font_size + 6.0);
        let expected_y =
            list.vcentered_text_y(theme.padding + 2.0, chip_h, font_size * 0.9, None, "✕");
        assert_eq!(cross_y, expected_y);
    }

    #[test]
    fn typing_edits_the_draft_and_enter_commits() {
        let theme = Theme::default();
        let tags: Vec<String> = Vec::new();
        let mut draft = String::new();

        let input = InputState {
            mouse_x: 10.0,
            mouse_y: 10.0,
            mouse_clicked: true,
            mouse_down: true,
            text_input: "ab".to_string(),
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = draw(
            Rect::new(0.0, 0.0, 200.0, 26.0),
            &tags,
            &mut draft,
            true,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(draft, "ab");
        assert_eq!(out.draft.as_deref(), Some("ab"));
        assert!(!out.committed);

        // Enter with a non-empty draft commits.
        draft = "ab".to_string();
        let input = InputState {
            enter_pressed: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = draw(
            Rect::new(0.0, 0.0, 200.0, 26.0),
            &tags,
            &mut draft,
            true,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.committed);
    }

    #[test]
    fn unfocused_field_shows_placeholder_and_takes_no_keys() {
        let theme = Theme::default();
        let tags: Vec<String> = Vec::new();
        let mut draft = String::new();
        let input = InputState {
            text_input: "x".to_string(),
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = draw(
            Rect::new(0.0, 0.0, 200.0, 26.0),
            &tags,
            &mut draft,
            false,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(draft.is_empty(), "unfocused typing is ignored");
        assert!(!out.committed);
    }
}
