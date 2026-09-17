//! Badges, keycaps, and chips — the design's small "callout" atoms (Gallery III).
//!
//! - [`badge`] / [`Badge`]: a tiny uppercase mono label on a tinted pill
//!   (status readouts: `ok` / `over` / `stale` in the design's table).
//! - [`keycap`]: a keyboard-key cap (`⇧` `Ctrl` `F`) — raised face over a
//!   black edge with a bottom drop line.
//! - [`chip`]: a toggleable filter pill — raised at rest, held-in when on.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::DrawList;
use super::material;
use super::material::draw_inset_shadow;

/// Draw a badge: a small rounded pill with `text`, tinted `tint` (bg alpha is
/// derived) and a legible foreground resolved by contrast. Height follows the
/// theme font; width follows the text. Returns the rect drawn.
///
/// The design badges are uppercase mono (`ok`, `over`, `stale`) — callers pick
/// the text; the widget only styles.
pub fn badge(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    text: &str,
    tint: [f32; 4],
) -> Rect {
    let h = 15.0f32
        .max(s.scalar(StyleKey::FontSize) * 0.9)
        .min(rect.height);
    let (w, _) = list.measure_text(text, s.scalar(StyleKey::FontSize) * 0.75, None);
    let bw = (w + 12.0).min(rect.width);
    let r = Rect::new(rect.x, rect.y + (rect.height - h) * 0.5, bw, h);

    // Status badges are recessed tinted labels, not flat translucent pills:
    // a low-alpha tint lives under a dark edge, with the shared top inset and
    // lower counter-edge supplying the 4a material read.
    let bg = [tint[0], tint[1], tint[2], tint[3] * 0.22];
    let top = material::sheen_over(bg, [1.0, 1.0, 1.0, 0.10]);
    list.chrome_rect_gradient(
        r,
        s.scalar(StyleKey::BorderRadius),
        1.0,
        top,
        bg,
        [0.0, 0.0, 0.0, 0.6],
    );
    draw_inset_shadow(
        list,
        s,
        r,
        s.scalar(StyleKey::InnerShadowDepth).min(3.0),
        1.0,
    );
    let fg = crate::widgets::sheen_over(s.color(StyleKey::Text), [tint[0], tint[1], tint[2], 0.55]);
    let text_y = list.vcentered_text_y(
        r.y,
        r.height,
        s.scalar(StyleKey::FontSize) * 0.75,
        s.theme().font.as_ref(),
        text,
    );
    list.text(
        TextBlock::new(text, r.x + 6.0, text_y)
            .with_size(s.scalar(StyleKey::FontSize) * 0.75)
            .with_color(
                (fg[0] * 255.0) as u8,
                (fg[1] * 255.0) as u8,
                (fg[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );
    r
}

/// Draw a keycap: a keyboard-key cap with `label` (design: raised white-sheen
/// face over a black edge, plus a dark line under the bottom edge selling the
/// key's side). `min_w` floors the width so single-glyph caps read as keys.
pub fn keycap(list: &mut DrawList, s: &StyleResolver, rect: Rect, label: &str, min_w: f32) -> Rect {
    let font_size = s.scalar(StyleKey::FontSize) * 0.85;
    let (tw, _) = list.measure_text(label, font_size, None);
    let h = 18.0f32.max(font_size + 8.0).min(rect.height);
    let w = (min_w.max(tw + 10.0)).min(rect.width);
    let r = Rect::new(rect.x, rect.y + (rect.height - h) * 0.5, w, h);
    let radius = s.scalar(StyleKey::BorderRadius);

    // Side: a dark line under the cap.
    list.quad(r.x, r.y + h - 1.0, w, 1.0, [0.0, 0.0, 0.0, 0.5]);

    let base = s.color(StyleKey::Button);
    let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
    let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
    list.chrome_rect_gradient(
        Rect::new(r.x, r.y, w, h - 1.0),
        radius,
        1.0,
        top,
        bottom,
        [0.0, 0.0, 0.0, 0.6],
    );
    // Inset highlight under the top edge.
    let hl = s.color(StyleKey::EdgeHighlight);
    list.quad(r.x + 1.0, r.y + 1.0, (w - 2.0).max(0.0), 1.0, hl);

    let text_color = s.color(StyleKey::Text);
    let text_y = list.vcentered_text_y(r.y, h - 1.0, font_size, s.theme().font.as_ref(), label);
    list.text(
        TextBlock::new(label, r.x + (w - tw) * 0.5, text_y)
            .with_size(font_size)
            .with_color(
                (text_color[0] * 255.0) as u8,
                (text_color[1] * 255.0) as u8,
                (text_color[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );
    r
}

/// Outcome of drawing a [`chip`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipOutput {
    /// The chip was clicked this frame (caller flips its `on` state).
    pub clicked: bool,
}

/// Draw a filter chip: a pill that is raised when off and held-in when on
/// (the design's log filter row). Click handling honors
/// [`InputState::mouse_consumed`](crate::InputState::mouse_consumed).
pub fn chip(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    label: &str,
    on: bool,
    input: &crate::InputState,
) -> ChipOutput {
    let font_size = s.scalar(StyleKey::FontSize) * 0.85;
    let (tw, _) = list.measure_text(label, font_size, None);
    let h = 17.0f32.max(font_size + 7.0).min(rect.height);
    let w = (tw + 16.0).min(rect.width);
    let r = Rect::new(rect.x, rect.y + (rect.height - h) * 0.5, w, h);
    let radius = 9.0f32.min(h * 0.5);

    let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
    let clicked = hovered && input.mouse_clicked;
    if hovered {
        // Chips draw from a bare list+style path; emit the pointer request
        // through the same channel other widgets use is impossible here, so
        // hover feedback is purely visual.
    }

    if on {
        // Held in the well: dark accent base, inset shadow, accent text.
        let base = s.color(StyleKey::Accent);
        let top = material::sheen_over(base, [0.0, 0.0, 0.0, 0.45]);
        list.chrome_rect_gradient(r, radius, 1.0, top, base, [0.0, 0.0, 0.0, 0.65]);
        draw_inset_shadow(list, s, r, s.scalar(StyleKey::InnerShadowDepth), 1.0);
    } else {
        let base = if hovered {
            s.color(StyleKey::ButtonHover)
        } else {
            s.color(StyleKey::Button)
        };
        let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
        let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
        list.chrome_rect_gradient(r, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.5]);
        // A pill's top edge is curved; a straight, full-width 1px highlight
        // reads as a conspicuous white slash. The face gradient already carries
        // the design's restrained sheen, so don't add the keycap-only band.
    }

    let fg = if on {
        s.color(StyleKey::OnAccent)
    } else {
        s.color(StyleKey::Text)
    };
    let text_y = list.vcentered_text_y(r.y, r.height, font_size, s.theme().font.as_ref(), label);
    list.text(
        TextBlock::new(label, r.x + 8.0, text_y)
            .with_size(font_size)
            .with_color(
                (fg[0] * 255.0) as u8,
                (fg[1] * 255.0) as u8,
                (fg[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );
    ChipOutput { clicked }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn badge_uses_inset_status_material() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        badge(
            &mut list,
            &s,
            Rect::new(0.0, 0.0, 100.0, 20.0),
            "ok",
            s.color(StyleKey::Success),
        );
        assert!(
            list.chrome_instances.len() >= 3,
            "badge face plus inset bands"
        );
        assert_eq!(list.chrome_instances[0].border, [0.0, 0.0, 0.0, 0.6]);
        assert_eq!(list.chrome_instances[1].bg, s.color(StyleKey::InnerShadow));
    }

    #[test]
    fn badge_sizes_to_its_text() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut short = DrawList::new();
        let a = badge(
            &mut short,
            &s,
            Rect::new(0.0, 0.0, 200.0, 20.0),
            "ok",
            s.color(StyleKey::Success),
        );
        let mut long = DrawList::new();
        let b = badge(
            &mut long,
            &s,
            Rect::new(0.0, 0.0, 200.0, 20.0),
            "outdated",
            s.color(StyleKey::Warning),
        );
        assert!(b.width > a.width, "wider text draws a wider badge");
        assert!(a.width > 0.0 && a.height > 0.0);
    }

    #[test]
    fn keycap_is_at_least_min_wide() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let r = keycap(&mut list, &s, Rect::new(0.0, 0.0, 200.0, 24.0), "F", 24.0);
        assert!(r.width >= 24.0);
        assert!(r.height > 0.0);
        // Cap face + highlight band + side line are all instanced quads; the
        // label is a text block.
        assert!(
            list.chrome_instances.len() >= 2,
            "cap face + highlight/side bands instanced"
        );
        assert!(!list.texts.is_empty(), "label block");
    }

    #[test]
    fn idle_chip_uses_its_gradient_sheen_without_a_white_top_line() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        chip(
            &mut list,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            false,
            &crate::InputState::default(),
        );
        assert_eq!(
            list.chrome_instances.len(),
            1,
            "only the pill face is painted"
        );
        assert_ne!(
            list.chrome_instances[0].bg, list.chrome_instances[0].bg2,
            "face retains its vertical sheen"
        );
    }

    #[test]
    fn chip_on_and_off_paint_different_faces_and_report_clicks() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let input = crate::InputState {
            mouse_x: 10.0,
            mouse_y: 10.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut off = DrawList::new();
        let off_out = chip(
            &mut off,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            false,
            &input,
        );
        let mut on = DrawList::new();
        let on_out = chip(
            &mut on,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            true,
            &input,
        );
        assert!(off_out.clicked && on_out.clicked, "click inside reports");
        assert_ne!(
            off.chrome_instances[0].bg, on.chrome_instances[0].bg,
            "off is a raised neutral, on is the held accent"
        );

        let far = crate::InputState {
            mouse_x: 500.0,
            mouse_y: 500.0,
            ..Default::default()
        };
        let mut quiet = DrawList::new();
        let out = chip(
            &mut quiet,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            false,
            &far,
        );
        assert!(!out.clicked);
    }
}
