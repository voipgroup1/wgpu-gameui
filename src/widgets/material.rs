//! The "4a" control material: a face plate resting on a plinth.
//!
//! Every raised control in the default design language is painted from the same
//! small vocabulary (see [`Theme`]'s material tokens):
//!
//! - a near-black **plinth** filling the control's full rect,
//! - a **face** inset `travel` px from the bottom (unpressed) — a vertical
//!   gradient (white sheen idly, or the accent/danger gradient for stateful
//!   tones) with a 1px near-black border edge,
//! - a 1px **inset highlight** under the face's top edge — replaced while
//!   pressed by a short dark shadow where the face has dropped onto the plinth,
//!
//! Pressing is geometric, not a color swap: the face drops by
//! [`StyleKey::Travel`] px so the plinth peeks out above it. Sunken surfaces
//! (input wells, tracks, the sunken tone) invert the model: dark fill, an inset
//! shadow under the top edge, and a faint light line beneath the bottom edge.
//!
//! Colors resolve through the [`StyleResolver`] (so a [`StyleOverlay`] retunes
//! any piece per-subtree). The inset shadow's depth is the
//! [`StyleKey::InnerShadowDepth`] scalar; the remaining band widths and border
//! alphas are language constants of the design, not theme fields.

use crate::DrawList;
use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};

/// Which face a control wears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Raised neutral face (the standard button).
    #[default]
    Default,
    /// Raised accent-gradient face (primary/stateful controls). Text should
    /// resolve to [`StyleKey::OnAccent`].
    Accent,
    /// Raised danger-gradient face (destructive controls).
    Danger,
    /// No plinth, transparent idle fill — the face only appears on hover/press
    /// (toolbar buttons, tabs, latch keys).
    Ghost,
    /// Sunken: dark fill with an inset shadow (checkbox troughs, toggled-off
    /// chips, pressed-in strips).
    Sunken,
}

/// One control's resolved material state.
pub struct Material {
    /// Which face the control wears.
    pub tone: Tone,
    /// Disabled controls fade to a fraction of their material (see
    /// `DISABLED_ALPHA`).
    pub enabled: bool,
    /// Hovered: face resolves the hover tokens.
    pub hovered: bool,
    /// Pressed: the face drops `travel` px onto the plinth.
    pub pressed: bool,
}

impl Material {
    /// A material in `tone`, enabled, at rest.
    #[must_use]
    pub fn new(tone: Tone) -> Self {
        Self {
            tone,
            enabled: true,
            hovered: false,
            pressed: false,
        }
    }

    /// Set the enabled flag (builder style).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the hovered flag (builder style).
    #[must_use]
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    /// Set the pressed flag (builder style).
    #[must_use]
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }
}

/// 1px inset highlight/shadow band under a face's top edge.
const BAND_H: f32 = 1.0;
/// 2px pressed shadow where the face dropped onto the plinth.
const PRESS_SHADOW_H: f32 = 2.0;
/// Face border alpha (the design's `1px solid rgba(0,0,0,0.5)`).
const FACE_EDGE_ALPHA: f32 = 0.5;
/// Ghost-tone border alphas: fainter idle, near-face-edge when active.
const GHOST_EDGE_IDLE: f32 = 0.25;
const GHOST_EDGE_ACTIVE: f32 = 0.4;
/// Sunken border alpha.
const SUNKEN_EDGE_ALPHA: f32 = 0.6;
/// Disabled controls fade to this fraction of their material.
const DISABLED_ALPHA: f32 = 0.45;

/// Draw the complete face-over-plinth material for `rect`.
///
/// Returns the rect the **face** occupies (the label should center in the face,
/// and while pressed it sits `travel` px lower).
pub fn draw(list: &mut DrawList, s: &StyleResolver, rect: Rect, m: &Material) -> Rect {
    draw_with_radius(list, s, rect, s.scalar(StyleKey::BorderRadius), m)
}

/// [`draw`] with an explicit corner radius (the design's stepper keys pass
/// `travel: 1, radius: 0`-style overrides).
///
/// Returns the face rect.
#[allow(clippy::too_many_arguments)]
pub fn draw_with_radius(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    radius: f32,
    m: &Material,
) -> Rect {
    let travel = s.scalar(StyleKey::Travel);
    // The face is always `travel` px shorter than the plinth: at rest it sits
    // at the top (plinth visible beneath), pressed it drops to the bottom
    // (plinth visible above). That's the design's plinth model — the gap
    // trades sides, the face never changes size.
    let edge = if m.enabled && m.pressed { travel } else { 0.0 };
    let face = Rect::new(
        rect.x,
        rect.y + edge,
        rect.width,
        (rect.height - travel).max(0.0),
    );

    match m.tone {
        Tone::Ghost => draw_ghost(list, s, rect, face, radius, m),
        Tone::Sunken => draw_sunken(list, s, rect, radius, m),
        tone => draw_raised(list, s, rect, face, radius, travel, m, tone),
    }
    face
}

/// Draw the **inset shadow** shared by every sunken surface: a band fading
/// down from the top edge (dark `InnerShadow` color to transparent), plus the
/// 1px light `EdgeShadow` line under the bottom edge (the "lit from above"
/// counter-edge).
///
/// `depth` is the band height; the outline stays inside `rect` by `inset` px
/// on every side (the surface's own border width — pass 0.0 for borderless
/// callers). Skips geometry cleanly when the band would be degenerate.
pub(crate) fn draw_inset_shadow(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    depth: f32,
    inset: f32,
) {
    let shadow = s.color(StyleKey::InnerShadow);
    let band_h = depth.max(1.0).min((rect.height - 2.0 * inset).max(0.0));
    let radius = s.scalar(StyleKey::BorderRadius);
    if band_h > 0.0 {
        list.chrome_rect_gradient(
            Rect::new(
                rect.x + inset,
                rect.y + inset,
                (rect.width - 2.0 * inset).max(0.0),
                band_h,
            ),
            radius,
            0.0,
            shadow,
            [shadow[0], shadow[1], shadow[2], 0.0],
            [0.0; 4],
        );
    }
    // The 1px light line under the bottom edge.
    let under = s.color(StyleKey::EdgeShadow);
    let line_h = BAND_H;
    let y = rect.y + rect.height - inset - line_h;
    if y > rect.y + inset {
        list.quad(
            rect.x + inset,
            y,
            (rect.width - 2.0 * inset).max(0.0),
            line_h,
            under,
        );
    }
}

/// Raised tones: plinth under a sheen gradient face.
///
/// The neutral face resolves its **base** from the state keys (`Button` /
/// `ButtonHover` / `ButtonPressed`) and composites the white-sheen tokens
/// (`FaceTop*` / `FaceBottom*`) over it — so an overlay retuning
/// [`StyleKey::Button`](StyleKey::Button) still recolors buttons, while the
/// sheen stays themeable separately. Accent/danger faces use their own opaque
/// gradients.
fn draw_raised(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    face: Rect,
    radius: f32,
    travel: f32,
    m: &Material,
    tone: Tone,
) {
    let dim = |mut c: [f32; 4]| {
        if !m.enabled {
            c[3] *= DISABLED_ALPHA;
        }
        c
    };
    // Plinth: the dark slab the face rests on; visible only in the `travel`
    // gap while the face is at rest. Hollow while pressed (the face covers it).
    if travel > 0.0 {
        list.chrome_rect(rect, radius, 0.0, dim(s.color(StyleKey::Plinth)), [0.0; 4]);
    }

    let pressed = m.enabled && m.pressed;
    let hovered = m.enabled && m.hovered;

    let (top, bottom, highlight) = match tone {
        Tone::Accent if pressed => (
            s.color(StyleKey::AccentFaceTopPressed),
            s.color(StyleKey::AccentFaceBottomPressed),
            s.color(StyleKey::EdgeHighlightPressed),
        ),
        Tone::Accent if hovered => (
            s.color(StyleKey::AccentFaceTopHover),
            s.color(StyleKey::AccentFaceBottomHover),
            s.color(StyleKey::EdgeHighlightHover),
        ),
        Tone::Accent => (
            s.color(StyleKey::AccentFaceTop),
            s.color(StyleKey::AccentFaceBottom),
            s.color(StyleKey::EdgeHighlight),
        ),
        Tone::Danger if pressed => (
            s.color(StyleKey::DangerFaceTopPressed),
            s.color(StyleKey::DangerFaceBottomPressed),
            s.color(StyleKey::EdgeHighlightPressed),
        ),
        Tone::Danger if hovered => (
            s.color(StyleKey::DangerFaceTopHover),
            s.color(StyleKey::DangerFaceBottomHover),
            s.color(StyleKey::EdgeHighlightHover),
        ),
        Tone::Danger => (
            s.color(StyleKey::DangerFaceTop),
            s.color(StyleKey::DangerFaceBottom),
            s.color(StyleKey::EdgeHighlight),
        ),
        // Neutral: state base under the sheen gradient.
        _ => {
            let base = if pressed {
                s.color(StyleKey::ButtonPressed)
            } else if hovered {
                s.color(StyleKey::ButtonHover)
            } else {
                s.color(StyleKey::Button)
            };
            let sheen_top = if pressed {
                s.color(StyleKey::FaceTopPressed)
            } else if hovered {
                s.color(StyleKey::FaceTopHover)
            } else {
                s.color(StyleKey::FaceTop)
            };
            let sheen_bottom = if pressed {
                s.color(StyleKey::FaceBottomPressed)
            } else if hovered {
                s.color(StyleKey::FaceBottomHover)
            } else {
                s.color(StyleKey::FaceBottom)
            };
            let hl = if pressed {
                s.color(StyleKey::EdgeHighlightPressed)
            } else if hovered {
                s.color(StyleKey::EdgeHighlightHover)
            } else {
                s.color(StyleKey::EdgeHighlight)
            };
            (
                sheen_over(base, sheen_top),
                sheen_over(base, sheen_bottom),
                hl,
            )
        }
    };

    let border = [0.0, 0.0, 0.0, FACE_EDGE_ALPHA];
    list.chrome_rect_gradient(face, radius, 1.0, dim(top), dim(bottom), dim(border));

    // Top edge decoration: at rest a 1px white highlight; pressed, the short
    // shadow of the face sitting in the plinth (plus its own faint highlight).
    if pressed {
        let shadow = dim(s.color(StyleKey::InnerShadow));
        list.chrome_rect(
            Rect::new(face.x + 1.0, face.y + 1.0, face.width - 2.0, PRESS_SHADOW_H),
            radius,
            0.0,
            shadow,
            [0.0; 4],
        );
    }
    list.quad(
        face.x + 1.0,
        face.y + 1.0,
        (face.width - 2.0).max(0.0),
        BAND_H,
        dim(highlight),
    );
}

/// Composite a translucent `sheen` color over an opaque-ish `base`
/// (source-over, per channel). The neutral face is a white sheen over the
/// state base, so this is the per-draw blend of the two token layers.
pub(crate) fn sheen_over(base: [f32; 4], sheen: [f32; 4]) -> [f32; 4] {
    let a = sheen[3];
    let out_a = a + base[3] * (1.0 - a);
    if out_a <= 0.0 {
        return [0.0; 4];
    }
    [
        (sheen[0] * a + base[0] * base[3] * (1.0 - a)) / out_a,
        (sheen[1] * a + base[1] * base[3] * (1.0 - a)) / out_a,
        (sheen[2] * a + base[2] * base[3] * (1.0 - a)) / out_a,
        out_a,
    ]
}

/// Derive a stable [`FocusId`](crate::FocusId) from a name + rect, for
/// stateless widgets that still want click-to-focus (the id is stable across
/// frames as long as the field doesn't move; movement merely re-keys it).
pub(crate) fn focus_id_for(name: &str, rect: &Rect) -> crate::FocusId {
    let mut h = crate::style::fnv1a64(name);
    for v in [rect.x, rect.y, rect.width, rect.height] {
        for b in v.to_bits().to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Ghost tone: no plinth, transparent at rest; hover/press paint the same white
/// faces on a faint border.
fn draw_ghost(
    list: &mut DrawList,
    s: &StyleResolver,
    _rect: Rect,
    face: Rect,
    radius: f32,
    m: &Material,
) {
    let dim = |mut c: [f32; 4]| {
        if !m.enabled {
            c[3] *= DISABLED_ALPHA;
        }
        c
    };
    let active = m.enabled && (m.pressed || m.hovered);
    let pressed = m.enabled && m.pressed;

    let (top, bottom, highlight) = if pressed {
        (
            s.color(StyleKey::FaceTopPressed),
            s.color(StyleKey::FaceBottomPressed),
            s.color(StyleKey::EdgeHighlightPressed),
        )
    } else if active {
        (
            s.color(StyleKey::FaceTopHover),
            s.color(StyleKey::FaceBottomHover),
            s.color(StyleKey::EdgeHighlightHover),
        )
    } else {
        // Fully transparent face at rest.
        ([0.0; 4], [0.0; 4], [0.0; 4])
    };

    let edge_alpha = if active {
        GHOST_EDGE_ACTIVE
    } else {
        GHOST_EDGE_IDLE
    };
    let border = [0.0, 0.0, 0.0, edge_alpha];
    list.chrome_rect_gradient(face, radius, 1.0, dim(top), dim(bottom), dim(border));

    if pressed {
        let shadow = dim(s.color(StyleKey::InnerShadow));
        list.chrome_rect(
            Rect::new(face.x + 1.0, face.y + 1.0, face.width - 2.0, PRESS_SHADOW_H),
            radius,
            0.0,
            shadow,
            [0.0; 4],
        );
    }
    if active {
        list.quad(
            face.x + 1.0,
            face.y + 1.0,
            (face.width - 2.0).max(0.0),
            BAND_H,
            dim(highlight),
        );
    }
}

/// Sunken tone: dark trough with an inset shadow and a light line beneath.
fn draw_sunken(list: &mut DrawList, s: &StyleResolver, rect: Rect, radius: f32, m: &Material) {
    let dim = |mut c: [f32; 4]| {
        if !m.enabled {
            c[3] *= DISABLED_ALPHA;
        }
        c
    };
    let fill = dim(s.color(StyleKey::InputBackground));
    let border = [0.0, 0.0, 0.0, SUNKEN_EDGE_ALPHA];
    list.chrome_rect(rect, radius, 1.0, fill, dim(border));
    draw_inset_shadow(list, s, rect, s.scalar(StyleKey::InnerShadowDepth), 1.0);
}

/// Draw an input **well**: the sunken field every text entry sits in, plus the
/// focus treatment (accent border + soft outer ring). `focused`/`invalid`
/// switch the border to the accent/danger color and add the outer ring.
pub fn draw_well(list: &mut DrawList, s: &StyleResolver, rect: Rect, focused: bool, invalid: bool) {
    draw_well_simple(list, s, rect, focused, invalid);

    // Focus/invalid ring: a 1px soft outline just inside the border (kept
    // within the widget's declared rect so debug lints don't flag it).
    if focused || invalid {
        let radius = s.scalar(StyleKey::BorderRadius);
        let ring = if invalid {
            s.color(StyleKey::Error)
        } else {
            s.color(StyleKey::Accent)
        };
        let ring_out = [ring[0], ring[1], ring[2], 0.16];
        list.rounded_rect_outline(rect, radius + 0.5, 1.0, ring_out);
    }
}

/// [`draw_well`] alias — the sunken field with the accent-when-focused border,
/// no outer ring (embedders that draw their own focus treatment: caret widgets,
/// cell-grids).
pub fn draw_well_simple(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    focused: bool,
    invalid: bool,
) {
    let radius = s.scalar(StyleKey::BorderRadius);
    let border_w = s.scalar(StyleKey::BorderWidth);
    let fill = if focused {
        let mut c = s.color(StyleKey::InputBackground);
        c[3] = (c[3] + 0.08).min(1.0);
        c
    } else {
        s.color(StyleKey::InputBackground)
    };
    let border = if invalid {
        s.color(StyleKey::Error)
    } else if focused {
        s.color(StyleKey::Accent)
    } else {
        s.color(StyleKey::InputBorder)
    };
    list.chrome_rect(rect, radius, border_w, fill, border);
    draw_inset_shadow(
        list,
        s,
        rect,
        s.scalar(StyleKey::InnerShadowDepth),
        border_w,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;
    use crate::{StyleResolver, Theme};
    use std::sync::OnceLock;

    fn styles() -> &'static StyleResolver<'static> {
        // Leak a theme so the resolver's lifetime is 'static in tests; the
        // process exits before it matters.
        static S: OnceLock<StyleResolver<'static>> = OnceLock::new();
        S.get_or_init(|| StyleResolver::new(Box::leak(Box::new(Theme::default()))))
    }

    #[test]
    fn face_drops_by_travel_when_pressed() {
        let s = styles();
        let rect = Rect::new(10.0, 10.0, 60.0, 24.0);
        let mut idle = DrawList::new();
        let f = draw(&mut idle, &s, rect, &Material::new(Tone::Default));
        assert_eq!(
            (f.y - rect.y, f.height),
            (0.0, 22.0),
            "at rest the face sits at the top, `travel` short of the control"
        );

        let mut pressed = DrawList::new();
        let f = draw(
            &mut pressed,
            &s,
            rect,
            &Material::new(Tone::Default).pressed(true),
        );
        assert_eq!(
            (f.y - rect.y, f.height),
            (2.0, 22.0),
            "pressing drops the face onto the plinth by `travel`"
        );
    }

    #[test]
    fn raised_paints_plinth_then_sheen_face() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, &s, rect, &Material::new(Tone::Default));
        // [0] plinth (flat dark), [1] face gradient = sheen over the state base.
        assert_eq!(list.chrome_instances[0].bg, s.color(StyleKey::Plinth));
        let expected_top = sheen_over(s.color(StyleKey::Button), s.color(StyleKey::FaceTop));
        assert_eq!(list.chrome_instances[1].bg, expected_top);
        let expected_bot = sheen_over(s.color(StyleKey::Button), s.color(StyleKey::FaceBottom));
        assert_eq!(list.chrome_instances[1].bg2, expected_bot);
        // Hovered composites the sheen over the hover base instead.
        let mut hov = DrawList::new();
        draw(
            &mut hov,
            &s,
            rect,
            &Material::new(Tone::Default).hovered(true),
        );
        let expected_hover = sheen_over(
            s.color(StyleKey::ButtonHover),
            s.color(StyleKey::FaceTopHover),
        );
        assert_eq!(hov.chrome_instances[1].bg, expected_hover);
    }

    #[test]
    fn overlay_recoloring_button_base_reaches_the_face() {
        // The seam: an overlay retuning `StyleKey::Button` shifts the neutral
        // face even though the sheen tokens stay in place.
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_color(StyleKey::Button, [0.7, 0.1, 0.2, 1.0]);
        let styled = StyleResolver::with_overlay(s.theme(), &overlay);
        let mut list = DrawList::new();
        draw(&mut list, &styled, rect, &Material::new(Tone::Default));
        let expected = sheen_over([0.7, 0.1, 0.2, 1.0], s.color(StyleKey::FaceTop));
        assert_eq!(list.chrome_instances[1].bg, expected);
    }

    #[test]
    fn ghost_is_transparent_at_rest_and_skips_the_plinth() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, &s, rect, &Material::new(Tone::Ghost));
        assert_eq!(list.chrome_instances.len(), 1, "no plinth instance");
        assert_eq!(
            list.chrome_instances[0].bg[3], 0.0,
            "idle ghost face is fully transparent"
        );
    }

    #[test]
    fn accent_tone_resolves_accent_faces_and_disabled_dims() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, &s, rect, &Material::new(Tone::Accent));
        assert_eq!(
            list.chrome_instances[1].bg,
            s.color(StyleKey::AccentFaceTop)
        );

        let mut off = DrawList::new();
        draw(
            &mut off,
            &s,
            rect,
            &Material::new(Tone::Accent).enabled(false),
        );
        let face = off.chrome_instances[1].bg;
        let full = s.color(StyleKey::AccentFaceTop);
        assert!(
            (face[3] - full[3] * DISABLED_ALPHA).abs() < 1e-4,
            "disabled alpha fades the face"
        );
    }

    #[test]
    fn sunken_paints_fill_shadow_and_underline() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, &s, rect, &Material::new(Tone::Sunken));
        assert_eq!(
            list.chrome_instances[0].bg,
            s.color(StyleKey::InputBackground)
        );
    }

    #[test]
    fn well_focus_ring_appears_only_when_focused() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 60.0, 22.0);
        let mut idle = DrawList::new();
        draw_well(&mut idle, &s, rect, false, false);
        let mut focus = DrawList::new();
        draw_well(&mut focus, &s, rect, true, false);
        assert!(
            focus.chrome_instances.len() > idle.chrome_instances.len(),
            "the focus ring is an extra stroke instance"
        );
    }
}
