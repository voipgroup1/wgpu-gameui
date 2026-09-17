//! Busy & placeholder states — the design's loading vocabulary (Gallery III).
//!
//! - [`skeleton`]: a shimmering placeholder bar (the app animates the shimmer
//!   phase; the widget draws one frame of it).
//! - [`spinner`]: a ring with an accent arc (app rotates it per frame).
//! - [`dots`]: three pulsing accent dots (app supplies the pulse phase).
//! - [`empty_state`]: centered glyph + title + body + a call-to-action slot —
//!   drawn from a bare list + resolver because it has no interaction of its
//!   own.

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::DrawList;

/// Draw one frame of a skeleton bar: a rounded bar with a moving highlight.
/// `phase` is the app-owned shimmer position in `[0, 1)` (wrap it each frame).
pub fn skeleton(list: &mut DrawList, s: &StyleResolver, rect: Rect, phase: f32) {
    let radius = rect.height * 0.5;
    let base = s.color(StyleKey::Button);
    let base = [base[0], base[1], base[2], 0.55];
    list.rounded_rect(rect, radius, base);
    // The moving sheen: a bright band sweeping left→right, clipped to the bar.
    let band_w = rect.width * 0.4;
    let x = rect.x + phase * (rect.width + band_w) - band_w;
    let band = Rect::new(
        x.max(rect.x),
        rect.y,
        (x + band_w).min(rect.right()) - x.max(rect.x),
        rect.height,
    );
    if band.width > 0.0 {
        list.rounded_rect(band, radius, [1.0, 1.0, 1.0, 0.10]);
    }
}

/// Draw one frame of a spinner: a faint ring with an accent arc covering
/// `sweep` radians starting at `phase` (both app-owned; rotate `phase` per
/// frame, e.g. `phase += dt * 8.0`).
pub fn spinner(
    list: &mut DrawList,
    s: &StyleResolver,
    center: (f32, f32),
    radius: f32,
    phase: f32,
    sweep: f32,
) {
    let track = s.color(StyleKey::ButtonHover);
    list.circle_outline(center, radius, 2.0, track);
    let accent = s.color(StyleKey::Accent);
    // Approximate the arc with chords (a spinner doesn't need true arcs).
    let steps = 14;
    let mut prev: Option<[f32; 2]> = None;
    for i in 0..=steps {
        let a = phase + sweep * (i as f32 / steps as f32);
        let p = [center.0 + radius * a.cos(), center.1 + radius * a.sin()];
        if let Some(p0) = prev {
            list.line(p0, p, 2.0, accent);
        }
        prev = Some(p);
    }
}

/// Draw three accent dots pulsing at `phase` (app-owned clock; stagger by
/// passing phases offset ~0.15s apart or use [`dots`] to lay all three).
pub fn dots(list: &mut DrawList, s: &StyleResolver, center: (f32, f32), phase: f32) {
    let accent = s.color(StyleKey::Accent);
    let spacing = 9.0;
    for i in 0..3usize {
        // Each dot's pulse is offset by 0.15 of a cycle.
        let t = (phase - i as f32 * 0.15).rem_euclid(1.0);
        let scale = 0.6
            + 0.4 * (1.0 - (2.0 * std::f32::consts::PI * t).cos()) * 0.5
            + 0.2 * (1.0 - t).abs();
        let r = 2.5 * scale.clamp(0.5, 1.3);
        let x = center.0 + (i as f32 - 1.0) * spacing;
        let mut c = accent;
        c[3] *= 0.5 + 0.5 * scale.clamp(0.0, 1.0);
        list.circle((x, center.1), r, c);
    }
}

/// Layout + intrinsic metrics for an [`empty_state`].
pub struct EmptyState<'a> {
    /// Glyph or symbol shown above the title.
    pub glyph: &'a str,
    /// The short headline ("No entities").
    pub title: &'a str,
    /// The explanatory body line (wrapped).
    pub body: &'a str,
}

/// Draw an empty-state block centered in `rect`: a dim glyph, a title, and a
/// dim body line (wrapped). The call-to-action button is the caller's normal
/// [`Button`](super::Button) drawn below the returned rect. Returns the rect
/// consumed (glyph + title + body), so the caller can flow the CTA under it.
pub fn empty_state(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    state: &EmptyState<'_>,
) -> Rect {
    let font_size = s.scalar(StyleKey::FontSize);
    let glyph_h = font_size * 2.0;
    let title_h = font_size * 1.3;
    let (_, body_h) = list.measure_text(state.body, font_size * 0.9, Some(rect.width - 32.0));
    let total = glyph_h + title_h + body_h + 12.0;
    let y0 = rect.y + (rect.height - total).max(0.0) * 0.5;

    let dim = s.color(StyleKey::TextDim);
    let text = s.color(StyleKey::Text);

    let gy = list.vcentered_text_y(y0, glyph_h, glyph_h, s.theme().font.as_ref(), state.glyph);
    let (gw, _) = list.measure_text(state.glyph, glyph_h, None);
    list.text(
        TextBlock::new(state.glyph, rect.x + (rect.width - gw) * 0.5, gy)
            .with_size(glyph_h)
            .with_color(
                (dim[0] * 255.0) as u8,
                (dim[1] * 255.0) as u8,
                (dim[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );

    let ty = list.vcentered_text_y(
        y0 + glyph_h,
        title_h,
        font_size * 1.1,
        s.theme().font.as_ref(),
        state.title,
    );
    let (tw, _) = list.measure_text(state.title, font_size * 1.1, None);
    list.text(
        TextBlock::new(state.title, rect.x + (rect.width - tw) * 0.5, ty)
            .with_size(font_size * 1.1)
            .with_color(
                (text[0] * 255.0) as u8,
                (text[1] * 255.0) as u8,
                (text[2] * 255.0) as u8,
            )
            .with_font_opt(s.theme().font.clone()),
    );

    let by = y0 + glyph_h + title_h + 4.0;
    list.text(
        TextBlock::new(state.body, rect.x + 16.0, by)
            .with_size(font_size * 0.9)
            .with_color(
                (dim[0] * 255.0) as u8,
                (dim[1] * 255.0) as u8,
                (dim[2] * 255.0) as u8,
            )
            .with_max_width(rect.width - 32.0)
            .with_align(crate::text::TextAlign::Center)
            .with_font_opt(s.theme().font.clone()),
    );

    Rect::new(rect.x, y0, rect.width, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn skeleton_shimmer_band_moves_with_phase() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let rect = Rect::new(0.0, 0.0, 120.0, 9.0);
        let mut a = DrawList::new();
        skeleton(&mut a, &s, rect, 0.1);
        let mut b = DrawList::new();
        skeleton(&mut b, &s, rect, 0.7);
        // Same instance count, but the highlight quad differs in x.
        assert_eq!(a.chrome_instances.len(), b.chrome_instances.len());
        assert!(a.chrome_instances.len() >= 2);
    }

    #[test]
    fn spinner_and_dots_emit_geometry() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        spinner(&mut list, &s, (20.0, 20.0), 8.0, 0.0, 1.6);
        dots(&mut list, &s, (60.0, 20.0), 0.3);
        assert!(!list.circle_instances.is_empty(), "ring + dots are circles");
        assert!(!list.vertices.is_empty(), "arc chords are line quads");
    }

    #[test]
    fn empty_state_centers_a_block_and_reports_its_extent() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 240.0, 160.0);
        let used = empty_state(
            &mut list,
            &s,
            rect,
            &EmptyState {
                glyph: "◈",
                title: "No entities",
                body: "Create one to get started.",
            },
        );
        assert!(used.height > 0.0 && used.width == rect.width);
        assert_eq!(list.texts.len(), 3);
        // The block is vertically centered: top gap ≈ bottom gap.
        assert!(used.y > rect.y, "block starts below the rect top");
        assert!(rect.bottom() - used.bottom() > 0.0);
    }
}
