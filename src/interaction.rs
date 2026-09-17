//! Retained hit geometry and topmost-first pointer dispatch.
//!
//! Immediate widgets cannot know whether a widget submitted later in the frame
//! will cover them. `InteractionScene` therefore resolves input against the
//! regions from the previous completed frame — the geometry the user actually
//! saw — while collecting the next frame's regions.

use std::collections::{HashMap, HashSet};

use crate::{Affine2, InputState, layout::Rect};

/// Stable identity for an interactive widget within one UI surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u64);

impl From<u64> for WidgetId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// A local-space shape used for hit testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitShape {
    /// Axis-aligned rectangle in the widget's local coordinate system.
    Rect(Rect),
    /// Rounded rectangle in local coordinates.
    RoundedRect {
        /// Outer rectangle.
        rect: Rect,
        /// Corner radius in logical pixels.
        radius: f32,
    },
}

impl HitShape {
    fn bounds(self) -> Rect {
        match self {
            Self::Rect(rect) | Self::RoundedRect { rect, .. } => rect,
        }
    }

    fn contains(self, point: [f32; 2]) -> bool {
        let rect = self.bounds();
        if !rect.contains(point[0], point[1]) {
            return false;
        }
        let Self::RoundedRect { radius, .. } = self else {
            return true;
        };
        let radius = radius.max(0.0).min(rect.width.min(rect.height) * 0.5);
        if radius == 0.0 {
            return true;
        }
        let cx = point[0].clamp(rect.x + radius, rect.right() - radius);
        let cy = point[1].clamp(rect.y + radius, rect.bottom() - radius);
        let dx = point[0] - cx;
        let dy = point[1] - cy;
        dx * dx + dy * dy <= radius * radius
    }
}

/// Pointer participation policy for a hit region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerPolicy {
    /// The region may become the topmost pointer target.
    #[default]
    Target,
    /// The region is visual/diagnostic only and pointer input passes through it.
    PassThrough,
}

/// Total ordering shared by retained hit regions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderKey {
    /// Outer layer order (`0` is the base list; larger values are above it).
    pub layer: u32,
    /// Submission sequence within the layer.
    pub sequence: u64,
}

/// Retained geometry and dispatch metadata for one interactive widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRegion {
    /// Stable widget identity.
    pub id: WidgetId,
    /// Local-space hit shape.
    pub shape: HitShape,
    /// Transform from local to world/screen coordinates.
    pub transform: Affine2,
    /// Effective world-space clip inherited when the widget was submitted.
    pub clip: Option<Rect>,
    /// Visual/input ordering key.
    pub order: OrderKey,
    /// Whether this region can receive input.
    pub enabled: bool,
    /// Whether the region targets or passes through pointer input.
    pub pointer_policy: PointerPolicy,
}

impl HitRegion {
    fn local_point(self, world: [f32; 2]) -> Option<[f32; 2]> {
        if self
            .clip
            .is_some_and(|clip| !clip.contains(world[0], world[1]))
        {
            return None;
        }
        let inverse = self.transform.try_inverse()?;
        let local = inverse.transform_point(world);
        self.shape.contains(local).then_some(local)
    }
}

/// Per-frame interaction result for a widget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Response {
    /// Stable identity of the widget, when this is not an idle result.
    pub id: Option<WidgetId>,
    /// Local allocation/hit bounds retained for this widget.
    pub rect: Rect,
    /// True when this response was resolved from geometry presented in the
    /// previous frame. False on a widget's first registered frame, allowing
    /// compatibility wrappers to retain immediate behavior while warming up.
    pub resolved: bool,
    /// Pointer is over this widget and it won topmost dispatch.
    pub hovered: bool,
    /// Primary pointer button is held by this widget.
    pub pressed: bool,
    /// Primary pointer press began on this widget this frame.
    pub clicked: bool,
    /// Primary pointer button was released for this widget this frame.
    pub released: bool,
    /// This widget owns a held pointer gesture.
    pub held: bool,
    /// Double-click edge targeted this widget.
    pub double_clicked: bool,
    /// Pointer position in the widget's local coordinate system.
    pub local_pos: Option<[f32; 2]>,
    /// Wheel delta targeted at this widget.
    pub scroll_delta: f32,
}

impl Response {
    /// An inert response for `id` and its current local bounds.
    pub fn idle(id: WidgetId, rect: Rect) -> Self {
        Self {
            id: Some(id),
            rect,
            ..Self::default()
        }
    }
}

/// Previous/current retained interaction geometry for one UI surface.
#[derive(Debug, Default)]
pub struct InteractionScene {
    previous: Vec<HitRegion>,
    current: Vec<HitRegion>,
    responses: HashMap<WidgetId, Response>,
    candidates: Vec<WidgetId>,
    winner: Option<WidgetId>,
    capture: Option<WidgetId>,
    duplicate_ids: HashSet<WidgetId>,
    next_sequence: u64,
}

impl InteractionScene {
    /// Create an empty scene. The first frame only registers geometry; input can
    /// target it from the following frame onward.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve this frame's input against the previous completed scene and begin
    /// collecting replacement geometry.
    pub fn begin_frame(&mut self, input: &InputState) {
        self.current.clear();
        self.responses.clear();
        self.candidates.clear();
        self.duplicate_ids.clear();
        self.next_sequence = 0;

        let world = [input.mouse_x, input.mouse_y];
        let mut hits: Vec<(OrderKey, usize, HitRegion, [f32; 2])> = self
            .previous
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, region)| region.enabled)
            .filter_map(|(index, region)| {
                region
                    .local_point(world)
                    .map(|local| (region.order, index, region, local))
            })
            .collect();
        hits.sort_by_key(|(order, index, _, _)| (*order, *index));
        hits.reverse();
        self.candidates
            .extend(hits.iter().map(|(_, _, region, _)| region.id));

        let hover = hits
            .iter()
            .find(|(_, _, region, _)| region.pointer_policy == PointerPolicy::Target)
            .map(|(_, _, region, local)| (*region, *local));
        self.winner = hover.map(|(region, _)| region.id);

        if input.mouse_clicked {
            self.capture = self.winner;
        }
        let owner = self.capture.or(self.winner);

        for region in &self.previous {
            let local = region.local_point(world);
            let is_hover = self.winner == Some(region.id) && local.is_some();
            let owns_pointer = owner == Some(region.id);
            self.responses.insert(
                region.id,
                Response {
                    id: Some(region.id),
                    rect: region.shape.bounds(),
                    resolved: true,
                    hovered: is_hover,
                    pressed: owns_pointer && input.mouse_down,
                    clicked: self.capture == Some(region.id) && input.mouse_clicked,
                    released: self.capture == Some(region.id) && input.mouse_released,
                    held: owns_pointer && input.mouse_held,
                    double_clicked: is_hover && input.mouse_double_clicked,
                    local_pos: if owns_pointer || is_hover {
                        local
                    } else {
                        None
                    },
                    scroll_delta: if is_hover && !input.scroll_consumed {
                        input.scroll_delta
                    } else {
                        0.0
                    },
                },
            );
        }

        if input.mouse_released || (!input.mouse_down && !input.mouse_clicked) {
            self.capture = None;
        }
    }

    /// Register current geometry and return the response resolved from the
    /// previous presented geometry carrying the same stable ID.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        id: WidgetId,
        shape: HitShape,
        transform: Affine2,
        clip: Option<Rect>,
        layer: u32,
        enabled: bool,
        pointer_policy: PointerPolicy,
    ) -> Response {
        if self.current.iter().any(|region| region.id == id) {
            self.duplicate_ids.insert(id);
        }
        let response = self
            .responses
            .get(&id)
            .copied()
            .unwrap_or_else(|| Response::idle(id, shape.bounds()));
        self.current.push(HitRegion {
            id,
            shape,
            transform,
            clip,
            order: OrderKey {
                layer,
                sequence: self.next_sequence,
            },
            enabled,
            pointer_policy,
        });
        self.next_sequence += 1;
        response
    }

    /// Complete collection and retain this frame's regions for next-frame input.
    pub fn end_frame(&mut self) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
    }

    /// IDs under the pointer, topmost first, including pass-through regions.
    pub fn candidates(&self) -> &[WidgetId] {
        &self.candidates
    }

    /// Topmost target region under the pointer this frame.
    pub fn winner(&self) -> Option<WidgetId> {
        self.winner
    }

    /// Duplicate stable IDs registered while collecting the current frame.
    pub fn duplicate_ids(&self) -> impl Iterator<Item = WidgetId> + '_ {
        self.duplicate_ids.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(scene: &mut InteractionScene, id: u64, rect: Rect, layer: u32) -> Response {
        scene.register(
            WidgetId(id),
            HitShape::Rect(rect),
            Affine2::IDENTITY,
            None,
            layer,
            true,
            PointerPolicy::Target,
        )
    }

    #[test]
    fn later_region_wins_on_the_next_frame() {
        let mut scene = InteractionScene::new();
        scene.begin_frame(&InputState::default());
        region(&mut scene, 1, Rect::new(0.0, 0.0, 20.0, 20.0), 0);
        region(&mut scene, 2, Rect::new(0.0, 0.0, 20.0, 20.0), 0);
        scene.end_frame();

        let input = InputState {
            mouse_x: 5.0,
            mouse_y: 5.0,
            mouse_clicked: true,
            mouse_down: true,
            ..InputState::default()
        };
        scene.begin_frame(&input);
        assert!(!region(&mut scene, 1, Rect::new(0.0, 0.0, 20.0, 20.0), 0).clicked);
        assert!(region(&mut scene, 2, Rect::new(0.0, 0.0, 20.0, 20.0), 0).clicked);
        assert_eq!(scene.candidates(), &[WidgetId(2), WidgetId(1)]);
    }

    #[test]
    fn clip_and_rotation_use_the_presented_local_shape() {
        let mut scene = InteractionScene::new();
        scene.begin_frame(&InputState::default());
        scene.register(
            WidgetId(7),
            HitShape::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Affine2::translation(20.0, 20.0)
                .compose(&Affine2::rotation(std::f32::consts::FRAC_PI_4)),
            Some(Rect::new(15.0, 15.0, 20.0, 20.0)),
            0,
            true,
            PointerPolicy::Target,
        );
        scene.end_frame();

        scene.begin_frame(&InputState {
            mouse_x: 20.0,
            mouse_y: 22.0,
            ..Default::default()
        });
        assert_eq!(scene.winner(), Some(WidgetId(7)));
        scene.begin_frame(&InputState {
            mouse_x: 40.0,
            mouse_y: 40.0,
            ..Default::default()
        });
        assert_eq!(scene.winner(), None);
    }

    #[test]
    fn singular_transform_is_not_hittable() {
        let mut scene = InteractionScene::new();
        scene.begin_frame(&InputState::default());
        scene.register(
            WidgetId(1),
            HitShape::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Affine2::scale(0.0, 1.0),
            None,
            0,
            true,
            PointerPolicy::Target,
        );
        scene.end_frame();
        scene.begin_frame(&InputState::default());
        assert_eq!(scene.winner(), None);
    }

    #[test]
    fn higher_layer_wins_and_pass_through_does_not_block() {
        let mut scene = InteractionScene::new();
        scene.begin_frame(&InputState::default());
        region(&mut scene, 1, Rect::new(0.0, 0.0, 10.0, 10.0), 0);
        scene.register(
            WidgetId(2),
            HitShape::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Affine2::IDENTITY,
            None,
            2,
            true,
            PointerPolicy::PassThrough,
        );
        region(&mut scene, 3, Rect::new(0.0, 0.0, 10.0, 10.0), 1);
        scene.end_frame();
        scene.begin_frame(&InputState {
            mouse_x: 2.0,
            mouse_y: 2.0,
            ..Default::default()
        });
        assert_eq!(scene.candidates(), &[WidgetId(2), WidgetId(3), WidgetId(1)]);
        assert_eq!(scene.winner(), Some(WidgetId(3)));
    }

    #[test]
    fn pointer_capture_survives_leaving_until_release() {
        let mut scene = InteractionScene::new();
        scene.begin_frame(&InputState::default());
        region(&mut scene, 9, Rect::new(0.0, 0.0, 10.0, 10.0), 0);
        scene.end_frame();

        scene.begin_frame(&InputState {
            mouse_x: 2.0,
            mouse_y: 2.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        });
        assert!(region(&mut scene, 9, Rect::new(0.0, 0.0, 10.0, 10.0), 0).clicked);
        scene.end_frame();

        scene.begin_frame(&InputState {
            mouse_x: 30.0,
            mouse_y: 30.0,
            mouse_down: true,
            mouse_held: true,
            ..Default::default()
        });
        let held = region(&mut scene, 9, Rect::new(0.0, 0.0, 10.0, 10.0), 0);
        assert!(held.pressed && held.held);
        assert!(!held.hovered);
        scene.end_frame();

        scene.begin_frame(&InputState {
            mouse_x: 30.0,
            mouse_y: 30.0,
            mouse_released: true,
            ..Default::default()
        });
        assert!(region(&mut scene, 9, Rect::new(0.0, 0.0, 10.0, 10.0), 0).released);
    }

    #[test]
    fn duplicate_ids_are_reported() {
        let mut scene = InteractionScene::new();
        scene.begin_frame(&InputState::default());
        region(&mut scene, 4, Rect::new(0.0, 0.0, 1.0, 1.0), 0);
        region(&mut scene, 4, Rect::new(2.0, 2.0, 1.0, 1.0), 0);
        assert_eq!(scene.duplicate_ids().collect::<Vec<_>>(), vec![WidgetId(4)]);
    }
}
