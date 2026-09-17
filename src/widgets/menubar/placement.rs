//! Pure popup placement and blocker geometry.
//!
//! Both functions are total and side-effect free, so the behaviour that is
//! hardest to eyeball — flipping at an edge, keeping a column inside an inset
//! viewport, and punching a hole for the bar strip in the blocker — is unit
//! testable without a GPU or a frame.

use crate::layout::Rect;

use super::model::SubmenuSide;

/// Gap between a bar label and the column that drops below it, in pixels. Small:
/// a drop is visually attached to its label, and the gap only keeps the column's
/// border from touching the strip.
pub(crate) const DROP_GAP: f32 = 2.0;

/// Where a column goes, given the label it hangs off, its own measured size, the
/// viewport it must stay inside, and the preferred side.
///
/// Returns the rect **and the side actually used**, so a caller can render a
/// directional affordance for the resolved side rather than the requested one.
///
/// The vertical rule is *flip, do not slide*: a column that does not fit below
/// its label goes above it, because sliding it up would cover the strip and
/// fight the hover-to-switch rule that keeps bar labels live. When it fits on
/// neither side (a column taller than the viewport) it is pinned to the viewport
/// top and the caller scrolls it.
///
/// The horizontal rule distinguishes preference from correction, which is what
/// [`SubmenuSide::Auto`] exists for: `Auto` flips to the other side when the
/// preferred one would overflow, while an explicit `Right`/`Left` is honoured and
/// merely *shifted* back inside the viewport. That asymmetry is deliberate and
/// the tests encode it.
pub fn place_popup(
    anchor: Rect,
    size: [f32; 2],
    viewport: Rect,
    side: SubmenuSide,
) -> (Rect, SubmenuSide) {
    let width = size[0].max(0.0);
    let height = size[1].max(0.0);

    // ---- vertical: below, flipping above rather than shifting ----
    let below = anchor.bottom() + DROP_GAP;
    let above = anchor.y - DROP_GAP - height;
    let y = if below + height <= viewport.bottom() {
        below
    } else if above >= viewport.y {
        above
    } else {
        // Taller than the room on either side: pin the top edge so the column
        // starts visible and let the scroll offset reach the rest.
        below.min(viewport.bottom() - height).max(viewport.y)
    };

    // ---- horizontal: honour the preference, flip only for `Auto` ----
    let effective = match side {
        SubmenuSide::Auto => {
            if anchor.x + width <= viewport.right() {
                SubmenuSide::Right
            } else {
                SubmenuSide::Left
            }
        }
        explicit => explicit,
    };
    let preferred_x = match effective {
        // Extends right of the anchor's left edge...
        SubmenuSide::Right | SubmenuSide::Auto => anchor.x,
        // ...or extends left of its right edge, so the column's right edge lines
        // up with the label instead of hanging off it.
        SubmenuSide::Left => anchor.right() - width,
    };
    // Only `Auto` flips; an explicit side slides.
    let x = preferred_x.min(viewport.right() - width).max(viewport.x);

    (Rect::new(x, y, width, height), effective)
}

/// Split the viewport minus the bar strip into up to four rects, returning how
/// many were written.
///
/// The open chain's blocker must cover everything *except* the strip: a single
/// full-viewport blocker region would rank above the base layer and kill the bar
/// labels, which have to stay live for hover-to-switch. A strip that spans the
/// viewport's full width needs two rects (above and below it, one of which
/// collapses when the strip is docked against an edge); a floating strip needs
/// four, because the band it occupies has open space on both sides of it. Regions
/// with no area are dropped rather than returned, so the count is the number of
/// rects actually written.
///
/// The regions tile exactly: every point of `viewport` is either inside the strip
/// or inside exactly one returned rect, and none of them overlap the strip.
pub fn blocker_regions(viewport: Rect, bar: Rect, out: &mut [Rect; 4]) -> usize {
    let Some(strip) = bar.intersection(viewport) else {
        // No strip to spare (or an empty viewport): block the lot.
        out[0] = viewport;
        return if viewport.is_empty() { 0 } else { 1 };
    };
    if viewport.is_empty() {
        return 0;
    }

    let mut count = 0;
    // Above the strip.
    if strip.y > viewport.y {
        out[count] = Rect::new(viewport.x, viewport.y, viewport.width, strip.y - viewport.y);
        count += 1;
    }
    // Below the strip.
    if strip.bottom() < viewport.bottom() {
        out[count] = Rect::new(
            viewport.x,
            strip.bottom(),
            viewport.width,
            viewport.bottom() - strip.bottom(),
        );
        count += 1;
    }
    // Left of the strip, within its band.
    if strip.x > viewport.x {
        out[count] = Rect::new(viewport.x, strip.y, strip.x - viewport.x, strip.height);
        count += 1;
    }
    // Right of the strip, within its band.
    if strip.right() < viewport.right() {
        out[count] = Rect::new(
            strip.right(),
            strip.y,
            viewport.right() - strip.right(),
            strip.height,
        );
        count += 1;
    }
    count
}
