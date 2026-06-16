//! Layout system - anchors, stacking, proportional sizing.
//!
//! # Overview
//!
//! The layout system computes positions for UI elements in two phases:
//! 1. Build a layout tree describing structure and sizing
//! 2. Call `layout()` to compute final screen positions
//!
//! # Example
//!
//! ```ignore
//! // Floor switcher anchored to top-right
//! let floor_ui = Positioned::new(
//!     Anchor::TopRight { offset: (-10.0, 10.0) },
//!     Size::fixed(80.0, 120.0),
//!     VStack::new(8.0)
//!         .child(30.0, 80.0).id(0)   // up button   — tagged for get_by_id
//!         .child(30.0, 80.0).id(1)   // floor label
//!         .child(30.0, 80.0).id(2)   // down button
//! );
//!
//! // One-shot (allocates a fresh result):
//! let result = floor_ui.layout_screen(screen_width, screen_height);
//! let up = result.get_by_id(0).unwrap();   // order-independent lookup
//!
//! // Frame loop (reuse a caller-owned buffer, no per-frame allocation):
//! let mut layout_buf = LayoutResult::default();
//! floor_ui.layout_screen_into(screen_width, screen_height, &mut layout_buf);
//! // Draw using layout_buf.container() / .children() / .get_by_id(..)
//! ```

/// Computed rectangle after layout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Left edge in pixels.
    pub x: f32,
    /// Top edge in pixels.
    pub y: f32,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
}

impl Rect {
    /// Construct a rect from its top-left corner and size.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// A zero-sized rect at the origin.
    pub fn zero() -> Self {
        Self::default()
    }

    /// Check if a point is inside this rect.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }

    /// Return the overlapping rectangle, if any.
    pub fn intersection(&self, other: Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.width).min(other.x + other.width);
        let y1 = (self.y + self.height).min(other.y + other.height);

        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }
}

/// Anchor point for positioning relative to parent/screen.
#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    /// Top-left corner with offset (x, y) from corner.
    TopLeft {
        /// Pixel offset (x, y) from the corner.
        offset: (f32, f32),
    },
    /// Top-right corner with offset (x, y) from corner. X is typically negative.
    TopRight {
        /// Pixel offset (x, y) from the corner.
        offset: (f32, f32),
    },
    /// Bottom-left corner with offset (x, y) from corner. Y is typically negative.
    BottomLeft {
        /// Pixel offset (x, y) from the corner.
        offset: (f32, f32),
    },
    /// Bottom-right corner with offset (x, y) from corner. Both typically negative.
    BottomRight {
        /// Pixel offset (x, y) from the corner.
        offset: (f32, f32),
    },
    /// Centered in parent with offset (x, y) from center.
    Center {
        /// Pixel offset (x, y) from the center.
        offset: (f32, f32),
    },
    /// Centered horizontally, at top with offset.
    TopCenter {
        /// Pixel offset (x, y) from the top-center point.
        offset: (f32, f32),
    },
    /// Centered horizontally, at bottom with offset.
    BottomCenter {
        /// Pixel offset (x, y) from the bottom-center point.
        offset: (f32, f32),
    },
    /// Centered vertically, at left with offset.
    LeftCenter {
        /// Pixel offset (x, y) from the left-center point.
        offset: (f32, f32),
    },
    /// Centered vertically, at right with offset.
    RightCenter {
        /// Pixel offset (x, y) from the right-center point.
        offset: (f32, f32),
    },
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor::TopLeft { offset: (0.0, 0.0) }
    }
}

impl Anchor {
    /// Compute top-left position given parent rect and element size.
    pub fn resolve(&self, parent: Rect, width: f32, height: f32) -> (f32, f32) {
        match *self {
            Anchor::TopLeft { offset } => (parent.x + offset.0, parent.y + offset.1),
            Anchor::TopRight { offset } => (
                parent.x + parent.width - width + offset.0,
                parent.y + offset.1,
            ),
            Anchor::BottomLeft { offset } => (
                parent.x + offset.0,
                parent.y + parent.height - height + offset.1,
            ),
            Anchor::BottomRight { offset } => (
                parent.x + parent.width - width + offset.0,
                parent.y + parent.height - height + offset.1,
            ),
            Anchor::Center { offset } => (
                parent.x + (parent.width - width) / 2.0 + offset.0,
                parent.y + (parent.height - height) / 2.0 + offset.1,
            ),
            Anchor::TopCenter { offset } => (
                parent.x + (parent.width - width) / 2.0 + offset.0,
                parent.y + offset.1,
            ),
            Anchor::BottomCenter { offset } => (
                parent.x + (parent.width - width) / 2.0 + offset.0,
                parent.y + parent.height - height + offset.1,
            ),
            Anchor::LeftCenter { offset } => (
                parent.x + offset.0,
                parent.y + (parent.height - height) / 2.0 + offset.1,
            ),
            Anchor::RightCenter { offset } => (
                parent.x + parent.width - width + offset.0,
                parent.y + (parent.height - height) / 2.0 + offset.1,
            ),
        }
    }
}

/// Size specification for a dimension.
#[derive(Debug, Clone, Copy)]
pub enum SizeSpec {
    /// Fixed pixel size.
    Fixed(f32),
    /// Percentage of parent (0.0 to 1.0).
    Percent(f32),
    /// Fill remaining space (used in stacks).
    Fill,
    /// Size to fit content (for containers).
    Fit,
}

impl Default for SizeSpec {
    fn default() -> Self {
        SizeSpec::Fit
    }
}

impl SizeSpec {
    /// Resolve to pixels given parent size and content size.
    pub fn resolve(&self, parent_size: f32, content_size: f32) -> f32 {
        match *self {
            SizeSpec::Fixed(px) => px,
            SizeSpec::Percent(pct) => parent_size * pct,
            SizeSpec::Fill => parent_size,
            SizeSpec::Fit => content_size,
        }
    }
}

/// Optional lower/upper pixel bounds applied to an already-resolved dimension.
///
/// Orthogonal to [`SizeSpec`]: the base spec computes a preferred size, then the
/// constraint clamps it. This mirrors CSS `min-*`/`max-*` — e.g. "fill the
/// parent, but never narrower than 120px nor wider than 400px" is
/// `SizeSpec::Fill` + `Constraint::between(120.0, 400.0)`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Constraint {
    /// Optional lower pixel bound.
    pub min: Option<f32>,
    /// Optional upper pixel bound.
    pub max: Option<f32>,
}

impl Constraint {
    /// No bounds (the default).
    pub const NONE: Self = Self {
        min: None,
        max: None,
    };

    /// Lower bound only.
    pub fn min(min: f32) -> Self {
        Self {
            min: Some(min),
            max: None,
        }
    }

    /// Upper bound only.
    pub fn max(max: f32) -> Self {
        Self {
            min: None,
            max: Some(max),
        }
    }

    /// Both bounds.
    pub fn between(min: f32, max: f32) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
        }
    }

    /// Set the lower bound, keeping any existing upper bound.
    pub fn with_min(mut self, min: f32) -> Self {
        self.min = Some(min);
        self
    }

    /// Set the upper bound, keeping any existing lower bound.
    pub fn with_max(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }

    /// True when neither bound is set (clamping is a no-op).
    pub fn is_unbounded(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }

    /// Clamp `value` to `[min, max]`. The upper bound is applied first, so when
    /// `min > max` the lower bound wins — matching CSS, where `min-width`
    /// overrides `max-width`.
    pub fn apply(&self, value: f32) -> f32 {
        let mut v = value;
        if let Some(mx) = self.max {
            v = v.min(mx);
        }
        if let Some(mn) = self.min {
            v = v.max(mn);
        }
        v
    }
}

/// Size for both dimensions, with optional per-axis min/max clamps.
#[derive(Debug, Clone, Copy, Default)]
pub struct Size {
    /// Width sizing spec.
    pub width: SizeSpec,
    /// Height sizing spec.
    pub height: SizeSpec,
    /// Clamp applied to the resolved width.
    pub width_constraint: Constraint,
    /// Clamp applied to the resolved height.
    pub height_constraint: Constraint,
}

impl Size {
    /// Fixed pixel size on both axes.
    pub fn fixed(width: f32, height: f32) -> Self {
        Self {
            width: SizeSpec::Fixed(width),
            height: SizeSpec::Fixed(height),
            ..Default::default()
        }
    }

    /// Percentage-of-parent size on both axes (0.0 to 1.0).
    pub fn percent(width: f32, height: f32) -> Self {
        Self {
            width: SizeSpec::Percent(width),
            height: SizeSpec::Percent(height),
            ..Default::default()
        }
    }

    /// Fill the parent on both axes.
    pub fn fill() -> Self {
        Self {
            width: SizeSpec::Fill,
            height: SizeSpec::Fill,
            ..Default::default()
        }
    }

    /// Size to content on both axes.
    pub fn fit() -> Self {
        Self {
            width: SizeSpec::Fit,
            height: SizeSpec::Fit,
            ..Default::default()
        }
    }

    /// Set the width to a fixed pixel value.
    pub fn width_fixed(mut self, width: f32) -> Self {
        self.width = SizeSpec::Fixed(width);
        self
    }

    /// Set the height to a fixed pixel value.
    pub fn height_fixed(mut self, height: f32) -> Self {
        self.height = SizeSpec::Fixed(height);
        self
    }

    /// Set the width clamp.
    pub fn width_constraint(mut self, c: Constraint) -> Self {
        self.width_constraint = c;
        self
    }

    /// Set the height clamp.
    pub fn height_constraint(mut self, c: Constraint) -> Self {
        self.height_constraint = c;
        self
    }

    /// Minimum resolved width in pixels.
    pub fn min_width(mut self, min: f32) -> Self {
        self.width_constraint = self.width_constraint.with_min(min);
        self
    }

    /// Maximum resolved width in pixels.
    pub fn max_width(mut self, max: f32) -> Self {
        self.width_constraint = self.width_constraint.with_max(max);
        self
    }

    /// Minimum resolved height in pixels.
    pub fn min_height(mut self, min: f32) -> Self {
        self.height_constraint = self.height_constraint.with_min(min);
        self
    }

    /// Maximum resolved height in pixels.
    pub fn max_height(mut self, max: f32) -> Self {
        self.height_constraint = self.height_constraint.with_max(max);
        self
    }
}

/// Stable identity for a layout node, assigned by the caller.
///
/// Positional indexing into a [`LayoutResult`] (`get(1)`, `get(2)`, …) is fragile:
/// inserting or reordering a child silently shifts every later index. Tag the
/// children you care about with an id via the `.id(..)` builder (stacks) or
/// [`Flow::item_id`], then look them up with [`LayoutResult::get_by_id`] — the
/// lookup is order-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

impl From<u64> for NodeId {
    fn from(value: u64) -> Self {
        NodeId(value)
    }
}

/// A layout node that can be positioned and sized.
pub trait LayoutNode {
    /// Compute minimum content size (for Fit sizing).
    fn content_size(&self) -> (f32, f32);

    /// Layout this node within `bounds`, writing the computed rects into `out`.
    ///
    /// `out` is **cleared first**, then filled with one entry per node in
    /// traversal order (entry 0 is the container, `1..` its children). Pass a
    /// caller-owned [`LayoutResult`] reused across frames to avoid the per-frame
    /// allocation that [`layout`](Self::layout) incurs.
    fn layout_into(&self, bounds: Rect, out: &mut LayoutResult);

    /// Convenience wrapper that allocates a fresh [`LayoutResult`]. Prefer
    /// [`layout_into`](Self::layout_into) with a reused buffer in a frame loop.
    fn layout(&self, bounds: Rect) -> LayoutResult {
        let mut out = LayoutResult::default();
        self.layout_into(bounds, &mut out);
        out
    }
}

/// One laid-out node: its caller-assigned [`NodeId`] (if any) and computed rect.
#[derive(Debug, Clone, Copy)]
struct LayoutEntry {
    id: Option<NodeId>,
    rect: Rect,
}

/// Result of layout computation — entries in traversal order, entry 0 the
/// container and `1..` its children.
///
/// Holds an internal buffer that [`LayoutNode::layout_into`] reuses, so a caller
/// can keep one `LayoutResult` as frame-scratch and pay no per-frame allocation.
/// Access rects positionally with [`get`](Self::get)/[`container`](Self::container)
/// /[`children`](Self::children), or by stable identity with
/// [`get_by_id`](Self::get_by_id).
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    entries: Vec<LayoutEntry>,
}

impl LayoutResult {
    /// A result holding a single (container) rect with no id.
    pub fn single(rect: Rect) -> Self {
        Self {
            entries: vec![LayoutEntry { id: None, rect }],
        }
    }

    /// Append an entry. Internal — nodes build results via `layout_into`.
    fn push(&mut self, id: Option<NodeId>, rect: Rect) {
        self.entries.push(LayoutEntry { id, rect });
    }

    /// The container rect (entry 0), or a zero rect if empty.
    pub fn container(&self) -> Rect {
        self.entries.first().map(|e| e.rect).unwrap_or_default()
    }

    /// The rect at `index` (0 = container), or a zero rect if out of bounds.
    pub fn get(&self, index: usize) -> Rect {
        self.entries.get(index).map(|e| e.rect).unwrap_or_default()
    }

    /// The rect of the node tagged with `id`, or `None` if no node carries it.
    /// Order-independent — the whole point of [`NodeId`]. Linear scan (child
    /// counts are small).
    pub fn get_by_id(&self, id: impl Into<NodeId>) -> Option<Rect> {
        let id = id.into();
        self.entries
            .iter()
            .find(|e| e.id == Some(id))
            .map(|e| e.rect)
    }

    /// Number of entries (container + children).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when there are no entries at all (a default-constructed buffer).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of child entries (excludes the container).
    pub fn child_count(&self) -> usize {
        self.entries.len().saturating_sub(1)
    }

    /// All rects in order, including the container.
    pub fn iter(&self) -> impl Iterator<Item = Rect> + '_ {
        self.entries.iter().map(|e| e.rect)
    }

    /// Child rects in order (skips the container at index 0).
    pub fn children(&self) -> impl Iterator<Item = Rect> + '_ {
        self.entries.iter().skip(1).map(|e| e.rect)
    }
}

/// A single positioned element.
pub struct Positioned<T> {
    /// Where the element is anchored within its parent.
    pub anchor: Anchor,
    /// Resolved size of the element.
    pub size: Size,
    /// The wrapped layout node.
    pub child: T,
}

impl<T> Positioned<T> {
    /// Wrap `child`, anchored and sized within its parent.
    pub fn new(anchor: Anchor, size: Size, child: T) -> Self {
        Self {
            anchor,
            size,
            child,
        }
    }

    /// Layout starting from screen coordinates (allocates a fresh result).
    pub fn layout_screen(&self, screen_width: f32, screen_height: f32) -> LayoutResult
    where
        T: LayoutNode,
    {
        let screen = Rect::new(0.0, 0.0, screen_width, screen_height);
        self.layout(screen)
    }

    /// Layout from screen coordinates into a caller-owned buffer (no allocation
    /// when `out` already has capacity). The frame-loop counterpart of
    /// [`layout_screen`](Self::layout_screen).
    pub fn layout_screen_into(&self, screen_width: f32, screen_height: f32, out: &mut LayoutResult)
    where
        T: LayoutNode,
    {
        let screen = Rect::new(0.0, 0.0, screen_width, screen_height);
        self.layout_into(screen, out);
    }
}

impl<T: LayoutNode> LayoutNode for Positioned<T> {
    fn content_size(&self) -> (f32, f32) {
        self.child.content_size()
    }

    fn layout_into(&self, parent: Rect, out: &mut LayoutResult) {
        let (content_w, content_h) = self.child.content_size();
        let width = self
            .size
            .width_constraint
            .apply(self.size.width.resolve(parent.width, content_w));
        let height = self
            .size
            .height_constraint
            .apply(self.size.height.resolve(parent.height, content_h));
        let (x, y) = self.anchor.resolve(parent, width, height);

        let bounds = Rect::new(x, y, width, height);
        // Delegate: the child's container becomes entry 0 (it clears `out`).
        self.child.layout_into(bounds, out);
    }
}

/// Alignment of a child on the cross axis (perpendicular to the stack direction).
///
/// Defaults to [`Stretch`](CrossAlign::Stretch), which fills the full cross-axis
/// span — the existing behavior. `Start`/`Center`/`End` pin the child at its
/// natural [`StackChild::cross_size`] instead, leaving the leftover space on
/// the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossAlign {
    /// Align to the start of the cross axis (top for VStack, left for HStack).
    Start,
    /// Center on the cross axis.
    Center,
    /// Align to the end of the cross axis (bottom for VStack, right for HStack).
    End,
    /// Fill the full cross-axis span (default).
    #[default]
    Stretch,
}

/// Distribution of children along the **main axis** when there is leftover space
/// — the equivalent of CSS `justify-content`.
///
/// Defaults to [`Start`](MainAlign::Start) (children packed at the start, leftover
/// space trailing — the existing behavior). Only has an effect when no
/// [`SizeSpec::Fill`] child is present: a `Fill` child grows to consume the
/// slack, leaving nothing to distribute (just like `flex-grow` vs
/// `justify-content` in CSS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainAlign {
    /// Pack children at the start; leftover space trails (default).
    #[default]
    Start,
    /// Center the children as a group; leftover space splits before and after.
    Center,
    /// Pack children at the end; leftover space leads.
    End,
    /// Leftover space divided evenly *between* adjacent children (none at the
    /// ends). A single child behaves like [`Start`](MainAlign::Start).
    SpaceBetween,
    /// Leftover space divided so each child has equal space around it — the end
    /// gaps are half the size of the gaps between children.
    SpaceAround,
    /// Leftover space divided into equal gaps, including the two ends.
    SpaceEvenly,
}

impl MainAlign {
    /// Resolve a main-axis distribution into a leading offset and the extra gap
    /// to insert between adjacent children, given `free` leftover space and `n`
    /// children. `Start` (and `n == 0`) yields `(0.0, 0.0)` — byte-identical to
    /// the un-justified layout.
    fn resolve(self, free: f32, n: usize) -> (f32, f32) {
        if n == 0 {
            return (0.0, 0.0);
        }
        let nf = n as f32;
        match self {
            MainAlign::Start => (0.0, 0.0),
            MainAlign::Center => (free * 0.5, 0.0),
            MainAlign::End => (free, 0.0),
            MainAlign::SpaceBetween => {
                if n > 1 {
                    (0.0, free / (nf - 1.0))
                } else {
                    (0.0, 0.0)
                }
            }
            MainAlign::SpaceAround => (free / (2.0 * nf), free / nf),
            MainAlign::SpaceEvenly => (free / (nf + 1.0), free / (nf + 1.0)),
        }
    }
}

/// Vertical stack - children arranged top to bottom.
pub struct VStack {
    /// Gap in pixels between adjacent children.
    pub spacing: f32,
    /// Inset in pixels applied on all four sides.
    pub padding: f32,
    /// Children in top-to-bottom order.
    pub children: Vec<StackChild>,
    /// Main-axis (vertical) distribution of leftover space. Defaults to
    /// [`MainAlign::Start`]; ignored when a [`SizeSpec::Fill`] child is present.
    pub main_align: MainAlign,
}

/// Horizontal stack - children arranged left to right.
pub struct HStack {
    /// Gap in pixels between adjacent children.
    pub spacing: f32,
    /// Inset in pixels applied on all four sides.
    pub padding: f32,
    /// Children in left-to-right order.
    pub children: Vec<StackChild>,
    /// Main-axis (horizontal) distribution of leftover space. Defaults to
    /// [`MainAlign::Start`]; ignored when a [`SizeSpec::Fill`] child is present.
    pub main_align: MainAlign,
}

/// A child in a stack with its sizing.
pub struct StackChild {
    /// Main-axis sizing spec.
    pub size: SizeSpec,
    /// Natural size along the stack (main) axis.
    pub content_size: f32, // Size along stack axis
    /// Natural size perpendicular to the stack (cross) axis.
    pub cross_size: f32,   // Size perpendicular to stack axis
    /// Clamp applied to the resolved main-axis size of this child.
    pub constraint: Constraint,
    /// Alignment on the cross axis (defaults to [`CrossAlign::Stretch`]).
    pub align: CrossAlign,
    /// Relative share of the remaining main-axis space for [`SizeSpec::Fill`]
    /// children (defaults to `1.0`). Each `Fill` child receives
    /// `remaining * (weight / sum_of_fill_weights)`, so two fills at `1.0`/`1.0`
    /// split evenly while `2.0`/`1.0` gives a 2:1 split. Ignored for non-`Fill`
    /// children.
    pub weight: f32,
    /// Optional stable identity for [`LayoutResult::get_by_id`] (defaults to
    /// `None`). Set via the `.id(..)` builder.
    pub id: Option<NodeId>,
}

impl StackChild {
    /// Construct a child with default `constraint`/`align`/`weight`/`id`. The
    /// `child*` builders funnel through this so new fields are set in one place.
    fn new(size: SizeSpec, content_size: f32, cross_size: f32) -> Self {
        Self {
            size,
            content_size,
            cross_size,
            constraint: Constraint::default(),
            align: CrossAlign::Stretch,
            weight: 1.0,
            id: None,
        }
    }
}

impl VStack {
    /// A vertical stack with `spacing` pixels between children and no padding.
    pub fn new(spacing: f32) -> Self {
        Self {
            spacing,
            padding: 0.0,
            children: Vec::new(),
            main_align: MainAlign::Start,
        }
    }

    /// Set the inset applied on all four sides.
    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the main-axis (vertical) distribution of leftover space — the
    /// `justify-content` equivalent. Defaults to [`MainAlign::Start`]. No effect
    /// when a [`SizeSpec::Fill`] child is present (the fill consumes the slack).
    ///
    /// ```ignore
    /// VStack::new(8.0).justify(MainAlign::SpaceBetween)
    ///     .child(30.0, 80.0).child(30.0, 80.0)
    /// ```
    pub fn justify(mut self, main_align: MainAlign) -> Self {
        self.main_align = main_align;
        self
    }

    /// Add a child with fixed height.
    pub fn child(mut self, height: f32, width: f32) -> Self {
        self.children
            .push(StackChild::new(SizeSpec::Fixed(height), height, width));
        self
    }

    /// Add a child that fills remaining space.
    pub fn child_fill(mut self, width: f32) -> Self {
        self.children
            .push(StackChild::new(SizeSpec::Fill, 0.0, width));
        self
    }

    /// Add a child with percentage height.
    pub fn child_percent(mut self, percent: f32, width: f32) -> Self {
        self.children
            .push(StackChild::new(SizeSpec::Percent(percent), 0.0, width));
        self
    }

    /// Apply a min/max pixel clamp to the most recently added child's height.
    /// Composes with any `child*` builder, e.g.
    /// `VStack::new(4.0).child_fill(100.0).constrain(Constraint::between(50.0, 200.0))`.
    ///
    /// Note: for `Fill` children the clamp is applied after remaining space is
    /// split evenly; this is a single pass, so a clamped `Fill` does not
    /// redistribute its slack to siblings.
    pub fn constrain(mut self, constraint: Constraint) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.constraint = constraint;
        }
        self
    }

    /// Set the cross-axis alignment for the most recently added child.
    ///
    /// By default every child stretches to fill the full width ([`CrossAlign::Stretch`]).
    /// Call this after a `child*` builder to pin the child at its `cross_size`
    /// instead:
    ///
    /// ```ignore
    /// VStack::new(8.0)
    ///     .child(30.0, 80.0).align(CrossAlign::Center)
    ///     .child_fill(60.0)
    /// ```
    pub fn align(mut self, align: CrossAlign) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.align = align;
        }
        self
    }

    /// Set the fill-distribution weight for the most recently added child.
    ///
    /// Only affects [`SizeSpec::Fill`] children: remaining space is split in
    /// proportion to each fill child's weight (default `1.0`). A 2:1 split is
    /// `…child_fill(w).weight(2.0)…child_fill(w).weight(1.0)`. Negative weights
    /// are clamped to `0.0`. No-op when there is no child yet.
    pub fn weight(mut self, weight: f32) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.weight = weight.max(0.0);
        }
        self
    }

    /// Tag the most recently added child with a stable [`NodeId`] for
    /// order-independent lookup via [`LayoutResult::get_by_id`]. No-op when there
    /// is no child yet.
    pub fn id(mut self, id: impl Into<NodeId>) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.id = Some(id.into());
        }
        self
    }
}

impl LayoutNode for VStack {
    fn content_size(&self) -> (f32, f32) {
        let mut height = self.padding * 2.0;
        let mut width: f32 = 0.0;

        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                height += self.spacing;
            }
            height += child.content_size;
            width = width.max(child.cross_size);
        }

        (width + self.padding * 2.0, height)
    }

    fn layout_into(&self, bounds: Rect, out: &mut LayoutResult) {
        out.entries.clear();
        out.push(None, bounds);
        let inner_width = bounds.width - self.padding * 2.0;
        let inner_height = bounds.height - self.padding * 2.0;

        // Calculate total fixed height and sum fill weights
        let mut fixed_height = 0.0;
        let mut fill_weight = 0.0;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                fixed_height += self.spacing;
            }
            match child.size {
                SizeSpec::Fixed(h) => fixed_height += h,
                SizeSpec::Percent(p) => fixed_height += inner_height * p,
                SizeSpec::Fill => fill_weight += child.weight,
                SizeSpec::Fit => fixed_height += child.content_size,
            }
        }

        // Distribute remaining space to fill children in proportion to weight
        // (equal weights reduce to remaining / fill_count, byte-identical).
        let remaining = (inner_height - fixed_height).max(0.0);
        let height_per_weight = if fill_weight > 0.0 {
            remaining / fill_weight
        } else {
            0.0
        };

        // Main-axis justification distributes the leftover space when no Fill
        // child claimed it. `remaining` already equals that slack here (fill
        // children resolve to 0 height), so reuse it directly. Default
        // `MainAlign::Start` → (0, 0), byte-identical to the un-justified layout.
        let (justify_offset, justify_gap) = if fill_weight == 0.0 {
            self.main_align.resolve(remaining, self.children.len())
        } else {
            (0.0, 0.0)
        };

        // Position children
        let mut y = bounds.y + self.padding + justify_offset;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                y += self.spacing + justify_gap;
            }

            let base_height = match child.size {
                SizeSpec::Fixed(h) => h,
                SizeSpec::Percent(p) => inner_height * p,
                SizeSpec::Fill => height_per_weight * child.weight,
                SizeSpec::Fit => child.content_size,
            };
            let height = child.constraint.apply(base_height);

            // Resolve cross-axis alignment (VStack cross axis = width).
            let (cx, cwidth) = match child.align {
                CrossAlign::Stretch => (bounds.x + self.padding, inner_width),
                CrossAlign::Start => (bounds.x + self.padding, child.cross_size.min(inner_width)),
                CrossAlign::Center => {
                    let w = child.cross_size.min(inner_width);
                    (bounds.x + self.padding + (inner_width - w) * 0.5, w)
                }
                CrossAlign::End => {
                    let w = child.cross_size.min(inner_width);
                    (bounds.x + self.padding + inner_width - w, w)
                }
            };

            out.push(child.id, Rect::new(cx, y, cwidth, height));
            y += height;
        }
    }
}

impl HStack {
    /// A horizontal stack with `spacing` pixels between children and no padding.
    pub fn new(spacing: f32) -> Self {
        Self {
            spacing,
            padding: 0.0,
            children: Vec::new(),
            main_align: MainAlign::Start,
        }
    }

    /// Set the inset applied on all four sides.
    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the main-axis (horizontal) distribution of leftover space — the
    /// `justify-content` equivalent. Defaults to [`MainAlign::Start`]. No effect
    /// when a [`SizeSpec::Fill`] child is present (the fill consumes the slack).
    ///
    /// ```ignore
    /// HStack::new(8.0).justify(MainAlign::SpaceBetween)
    ///     .child(80.0, 30.0).child(80.0, 30.0)
    /// ```
    pub fn justify(mut self, main_align: MainAlign) -> Self {
        self.main_align = main_align;
        self
    }

    /// Add a child with fixed width.
    pub fn child(mut self, width: f32, height: f32) -> Self {
        self.children
            .push(StackChild::new(SizeSpec::Fixed(width), width, height));
        self
    }

    /// Add a child that fills remaining space.
    pub fn child_fill(mut self, height: f32) -> Self {
        self.children
            .push(StackChild::new(SizeSpec::Fill, 0.0, height));
        self
    }

    /// Add a child with percentage width.
    pub fn child_percent(mut self, percent: f32, height: f32) -> Self {
        self.children
            .push(StackChild::new(SizeSpec::Percent(percent), 0.0, height));
        self
    }

    /// Apply a min/max pixel clamp to the most recently added child's width.
    /// Composes with any `child*` builder, e.g.
    /// `HStack::new(4.0).child_fill(100.0).constrain(Constraint::between(50.0, 200.0))`.
    ///
    /// Note: for `Fill` children the clamp is applied after remaining space is
    /// split evenly; this is a single pass, so a clamped `Fill` does not
    /// redistribute its slack to siblings.
    pub fn constrain(mut self, constraint: Constraint) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.constraint = constraint;
        }
        self
    }

    /// Set the cross-axis alignment for the most recently added child.
    ///
    /// By default every child stretches to fill the full height ([`CrossAlign::Stretch`]).
    /// Call this after a `child*` builder to pin the child at its `cross_size`
    /// instead:
    ///
    /// ```ignore
    /// HStack::new(8.0)
    ///     .child(80.0, 30.0).align(CrossAlign::Center)
    ///     .child_fill(60.0)
    /// ```
    pub fn align(mut self, align: CrossAlign) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.align = align;
        }
        self
    }

    /// Set the fill-distribution weight for the most recently added child.
    ///
    /// Only affects [`SizeSpec::Fill`] children: remaining space is split in
    /// proportion to each fill child's weight (default `1.0`). A 2:1 split is
    /// `…child_fill(h).weight(2.0)…child_fill(h).weight(1.0)`. Negative weights
    /// are clamped to `0.0`. No-op when there is no child yet.
    pub fn weight(mut self, weight: f32) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.weight = weight.max(0.0);
        }
        self
    }

    /// Tag the most recently added child with a stable [`NodeId`] for
    /// order-independent lookup via [`LayoutResult::get_by_id`]. No-op when there
    /// is no child yet.
    pub fn id(mut self, id: impl Into<NodeId>) -> Self {
        if let Some(last) = self.children.last_mut() {
            last.id = Some(id.into());
        }
        self
    }
}

impl LayoutNode for HStack {
    fn content_size(&self) -> (f32, f32) {
        let mut width = self.padding * 2.0;
        let mut height: f32 = 0.0;

        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                width += self.spacing;
            }
            width += child.content_size;
            height = height.max(child.cross_size);
        }

        (width, height + self.padding * 2.0)
    }

    fn layout_into(&self, bounds: Rect, out: &mut LayoutResult) {
        out.entries.clear();
        out.push(None, bounds);
        let inner_width = bounds.width - self.padding * 2.0;
        let inner_height = bounds.height - self.padding * 2.0;

        // Calculate total fixed width and sum fill weights
        let mut fixed_width = 0.0;
        let mut fill_weight = 0.0;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                fixed_width += self.spacing;
            }
            match child.size {
                SizeSpec::Fixed(w) => fixed_width += w,
                SizeSpec::Percent(p) => fixed_width += inner_width * p,
                SizeSpec::Fill => fill_weight += child.weight,
                SizeSpec::Fit => fixed_width += child.content_size,
            }
        }

        // Distribute remaining space to fill children in proportion to weight
        // (equal weights reduce to remaining / fill_count, byte-identical).
        let remaining = (inner_width - fixed_width).max(0.0);
        let width_per_weight = if fill_weight > 0.0 {
            remaining / fill_weight
        } else {
            0.0
        };

        // Main-axis justification distributes the leftover space when no Fill
        // child claimed it. `remaining` already equals that slack here (fill
        // children resolve to 0 width), so reuse it directly. Default
        // `MainAlign::Start` → (0, 0), byte-identical to the un-justified layout.
        let (justify_offset, justify_gap) = if fill_weight == 0.0 {
            self.main_align.resolve(remaining, self.children.len())
        } else {
            (0.0, 0.0)
        };

        // Position children
        let mut x = bounds.x + self.padding + justify_offset;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                x += self.spacing + justify_gap;
            }

            let base_width = match child.size {
                SizeSpec::Fixed(w) => w,
                SizeSpec::Percent(p) => inner_width * p,
                SizeSpec::Fill => width_per_weight * child.weight,
                SizeSpec::Fit => child.content_size,
            };
            let width = child.constraint.apply(base_width);

            // Resolve cross-axis alignment (HStack cross axis = height).
            let (cy, cheight) = match child.align {
                CrossAlign::Stretch => (bounds.y + self.padding, inner_height),
                CrossAlign::Start => (bounds.y + self.padding, child.cross_size.min(inner_height)),
                CrossAlign::Center => {
                    let h = child.cross_size.min(inner_height);
                    (bounds.y + self.padding + (inner_height - h) * 0.5, h)
                }
                CrossAlign::End => {
                    let h = child.cross_size.min(inner_height);
                    (bounds.y + self.padding + inner_height - h, h)
                }
            };

            out.push(child.id, Rect::new(x, cy, width, cheight));
            x += width;
        }
    }
}

/// A simple leaf node with fixed size.
pub struct Leaf {
    /// Fixed width in pixels.
    pub width: f32,
    /// Fixed height in pixels.
    pub height: f32,
}

impl Leaf {
    /// A leaf node with the given fixed size.
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl LayoutNode for Leaf {
    fn content_size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    fn layout_into(&self, bounds: Rect, out: &mut LayoutResult) {
        out.entries.clear();
        out.push(None, bounds);
    }
}

/// Wrapping flow / grid container — children are placed left-to-right and wrap
/// to a new row when the next item would overflow the bounds width.
///
/// Unlike [`VStack`]/[`HStack`] (single-line), `Flow` is the layout for
/// inventory grids, tag clouds, and mod lists: a run of fixed-size items that
/// reflow to as many rows as the width requires. Items keep their own
/// `(width, height)`; rows are as tall as their tallest item.
///
/// The wrapped height depends on the available width, which the width-less
/// [`LayoutNode::content_size`] can't express — so [`content_size`](Self::content_size)
/// reports the *unwrapped single-row* extents, and callers that need the true
/// wrapped height (to size a scroll viewport or a `Fit` parent) call
/// [`measure_height`](Self::measure_height) with the real width.
///
/// # Example
/// ```ignore
/// let grid = Flow::new(8.0).with_padding(8.0)
///     .item(48.0, 48.0).item(48.0, 48.0).item(48.0, 48.0);
/// let result = grid.layout(Rect::new(0.0, 0.0, 120.0, grid.measure_height(120.0)));
/// // result.children() yields the item rects, wrapped into rows.
/// ```
pub struct Flow {
    /// Gap between items within a row (main axis).
    pub spacing: f32,
    /// Gap between rows (cross axis). Defaults to `spacing`.
    pub run_spacing: f32,
    /// Inset on all four sides.
    pub padding: f32,
    /// Items in add order.
    pub children: Vec<FlowItem>,
}

/// One item in a [`Flow`]: its size and optional stable [`NodeId`].
#[derive(Debug, Clone, Copy)]
pub struct FlowItem {
    /// Item width in pixels.
    pub width: f32,
    /// Item height in pixels.
    pub height: f32,
    /// Stable identity for [`LayoutResult::get_by_id`]; `None` for `.item(..)`.
    pub id: Option<NodeId>,
}

impl Flow {
    /// A flow with `spacing` between items and rows, no padding.
    pub fn new(spacing: f32) -> Self {
        Self {
            spacing,
            run_spacing: spacing,
            padding: 0.0,
            children: Vec::new(),
        }
    }

    /// Set a row gap distinct from the in-row item gap.
    pub fn with_run_spacing(mut self, run_spacing: f32) -> Self {
        self.run_spacing = run_spacing;
        self
    }

    /// Set the inset applied on all four sides.
    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Append one `width`×`height` item (no id).
    pub fn item(mut self, width: f32, height: f32) -> Self {
        self.children.push(FlowItem {
            width,
            height,
            id: None,
        });
        self
    }

    /// Append one `width`×`height` item tagged with a stable [`NodeId`] for
    /// order-independent lookup via [`LayoutResult::get_by_id`].
    pub fn item_id(mut self, id: impl Into<NodeId>, width: f32, height: f32) -> Self {
        self.children.push(FlowItem {
            width,
            height,
            id: Some(id.into()),
        });
        self
    }

    /// Total wrapped content height for a given available `width`, including
    /// padding on both sides. This is the value [`content_size`](Self::content_size)
    /// cannot return (height depends on width) — use it to size a scroll
    /// viewport or `Fit` parent around the flow. Returns `2 * padding` when empty.
    pub fn measure_height(&self, width: f32) -> f32 {
        let inner_w = (width - self.padding * 2.0).max(0.0);
        let mut cur_x = 0.0_f32;
        let mut cur_y = 0.0_f32;
        let mut row_h = 0.0_f32;
        let mut placed_any = false;
        for &FlowItem {
            width: w,
            height: h,
            ..
        } in &self.children
        {
            // Wrap before placing when this item would overflow the row (but
            // never on the first item of a row, so an oversized item still fits).
            if cur_x > 0.0 && cur_x + w > inner_w {
                cur_y += row_h + self.run_spacing;
                cur_x = 0.0;
                row_h = 0.0;
            }
            cur_x += w + self.spacing;
            row_h = row_h.max(h);
            placed_any = true;
        }
        let content_h = if placed_any { cur_y + row_h } else { 0.0 };
        content_h + self.padding * 2.0
    }
}

impl LayoutNode for Flow {
    /// Unwrapped single-row extents: total item width (+ gaps) and the tallest
    /// item, plus padding. See the type docs — for the wrapped height use
    /// [`Flow::measure_height`].
    fn content_size(&self) -> (f32, f32) {
        let mut width = 0.0_f32;
        let mut height = 0.0_f32;
        for (i, item) in self.children.iter().enumerate() {
            if i > 0 {
                width += self.spacing;
            }
            width += item.width;
            height = height.max(item.height);
        }
        (width + self.padding * 2.0, height + self.padding * 2.0)
    }

    fn layout_into(&self, bounds: Rect, out: &mut LayoutResult) {
        out.entries.clear();
        out.push(None, bounds);
        let inner_w = (bounds.width - self.padding * 2.0).max(0.0);
        let mut cur_x = self.padding;
        let mut cur_y = self.padding;
        let mut row_h = 0.0_f32;
        for item in &self.children {
            let (w, h) = (item.width, item.height);
            // Wrap before placing when this item overflows the row, except as the
            // first item in a row (an item wider than `inner_w` still gets placed
            // once, overflowing, rather than looping forever).
            if cur_x > self.padding && cur_x + w > self.padding + inner_w {
                cur_x = self.padding;
                cur_y += row_h + self.run_spacing;
                row_h = 0.0;
            }
            out.push(item.id, Rect::new(bounds.x + cur_x, bounds.y + cur_y, w, h));
            cur_x += w + self.spacing;
            row_h = row_h.max(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anchor_top_right() {
        let screen = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let anchor = Anchor::TopRight {
            offset: (-10.0, 10.0),
        };
        let (x, y) = anchor.resolve(screen, 80.0, 120.0);
        assert_eq!(x, 1280.0 - 80.0 - 10.0); // 1190
        assert_eq!(y, 10.0);
    }

    #[test]
    fn test_vstack_layout() {
        let stack = VStack::new(8.0)
            .with_padding(10.0)
            .child(30.0, 60.0) // button
            .child(20.0, 60.0) // label
            .child(30.0, 60.0); // button

        let bounds = Rect::new(100.0, 50.0, 80.0, 150.0);
        let result = stack.layout(bounds);

        // Container rect
        assert_eq!(result.get(0), bounds);

        // First child: y = 50 + 10 padding = 60
        assert_eq!(result.get(1).y, 60.0);
        assert_eq!(result.get(1).height, 30.0);

        // Second child: y = 60 + 30 + 8 spacing = 98
        assert_eq!(result.get(2).y, 98.0);
        assert_eq!(result.get(2).height, 20.0);

        // Third child: y = 98 + 20 + 8 spacing = 126
        assert_eq!(result.get(3).y, 126.0);
        assert_eq!(result.get(3).height, 30.0);
    }

    #[test]
    fn constraint_apply_clamps_both_bounds() {
        let c = Constraint::between(50.0, 200.0);
        assert_eq!(c.apply(10.0), 50.0); // below min -> min
        assert_eq!(c.apply(120.0), 120.0); // within -> unchanged
        assert_eq!(c.apply(500.0), 200.0); // above max -> max
    }

    #[test]
    fn constraint_apply_single_bounds_and_unbounded() {
        assert_eq!(Constraint::min(50.0).apply(10.0), 50.0);
        assert_eq!(Constraint::min(50.0).apply(80.0), 80.0);
        assert_eq!(Constraint::max(100.0).apply(150.0), 100.0);
        assert_eq!(Constraint::max(100.0).apply(40.0), 40.0);
        assert!(Constraint::NONE.is_unbounded());
        assert_eq!(Constraint::NONE.apply(123.0), 123.0);
    }

    #[test]
    fn constraint_min_wins_when_min_exceeds_max() {
        // CSS semantics: min-width overrides max-width.
        let c = Constraint::between(200.0, 100.0);
        assert_eq!(c.apply(150.0), 200.0);
    }

    #[test]
    fn positioned_clamps_resolved_size() {
        // Fill width would be 1000, but max_width caps it; Fit height of 0 from
        // the leaf is lifted by min_height.
        let node = Positioned::new(
            Anchor::TopLeft { offset: (0.0, 0.0) },
            Size::fill()
                .max_width(300.0)
                .height_fixed(20.0)
                .min_height(50.0),
            Leaf::new(10.0, 10.0),
        );
        let result = node.layout(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        let r = result.get(0);
        assert_eq!(r.width, 300.0, "fill width clamped to max_width");
        assert_eq!(r.height, 50.0, "fixed 20 lifted to min_height 50");
    }

    #[test]
    fn vstack_constrain_clamps_fill_child() {
        // A single Fill child would take all 400px of inner height, but the
        // clamp caps it at 120.
        let stack = VStack::new(0.0)
            .child_fill(60.0)
            .constrain(Constraint::max(120.0));
        let result = stack.layout(Rect::new(0.0, 0.0, 60.0, 400.0));
        assert_eq!(result.get(1).height, 120.0);
    }

    #[test]
    fn vstack_constrain_lifts_small_child_to_min() {
        // A 10px fixed child raised to a 40px floor.
        let stack = VStack::new(0.0)
            .child(10.0, 60.0)
            .constrain(Constraint::min(40.0));
        let result = stack.layout(Rect::new(0.0, 0.0, 60.0, 400.0));
        assert_eq!(result.get(1).height, 40.0);
    }

    #[test]
    fn hstack_constrain_clamps_fill_child() {
        let stack = HStack::new(0.0)
            .child_fill(20.0)
            .constrain(Constraint::between(50.0, 150.0));
        let result = stack.layout(Rect::new(0.0, 0.0, 1000.0, 20.0));
        assert_eq!(result.get(1).width, 150.0, "fill width clamped to max");
    }

    #[test]
    fn constrain_without_children_is_noop() {
        // Should not panic when there's no last child to clamp.
        let stack = VStack::new(0.0).constrain(Constraint::min(10.0));
        let result = stack.layout(Rect::new(0.0, 0.0, 60.0, 400.0));
        assert_eq!(result.len(), 1, "only the container rect");
    }

    #[test]
    fn test_positioned_top_right() {
        let layout = Positioned::new(
            Anchor::TopRight {
                offset: (-10.0, 10.0),
            },
            Size::fixed(80.0, 120.0),
            Leaf::new(80.0, 120.0),
        );

        let result = layout.layout_screen(1280.0, 720.0);
        let rect = result.get(0);

        assert_eq!(rect.x, 1190.0);
        assert_eq!(rect.y, 10.0);
        assert_eq!(rect.width, 80.0);
        assert_eq!(rect.height, 120.0);
    }

    /// Example: Floor switcher UI anchored to top-right corner.
    ///
    /// Layout:
    /// ```text
    /// +--------+
    /// |   ▲    |  <- up button (index 1)
    /// +--------+
    /// | Ground |  <- floor label (index 2)
    /// +--------+
    /// |   ▼    |  <- down button (index 3)
    /// +--------+
    /// ```
    #[test]
    fn test_floor_switcher_layout() {
        // Define the layout once
        let floor_switcher = Positioned::new(
            Anchor::TopRight {
                offset: (-10.0, 10.0),
            },
            Size::fixed(80.0, 110.0),
            VStack::new(5.0)
                .with_padding(5.0)
                .child(30.0, 70.0) // up button
                .child(24.0, 70.0) // floor label
                .child(30.0, 70.0), // down button
        );

        // Compute layout for 1280x720 screen
        let result = floor_switcher.layout_screen(1280.0, 720.0);

        // Container is anchored to top-right with 10px offset
        let container = result.get(0);
        assert_eq!(container.x, 1280.0 - 80.0 - 10.0); // 1190
        assert_eq!(container.y, 10.0);
        assert_eq!(container.width, 80.0);
        assert_eq!(container.height, 110.0);

        // Up button (index 1)
        let up_btn = result.get(1);
        assert_eq!(up_btn.x, 1190.0 + 5.0); // container.x + padding
        assert_eq!(up_btn.y, 10.0 + 5.0); // container.y + padding
        assert_eq!(up_btn.height, 30.0);

        // Floor label (index 2)
        let floor_label = result.get(2);
        assert_eq!(floor_label.y, up_btn.y + 30.0 + 5.0); // after button + spacing
        assert_eq!(floor_label.height, 24.0);

        // Down button (index 3)
        let down_btn = result.get(3);
        assert_eq!(down_btn.y, floor_label.y + 24.0 + 5.0);
        assert_eq!(down_btn.height, 30.0);

        // Example of how you'd use this with widgets:
        //
        // fn draw_floor_switcher(
        //     current_floor: i8,
        //     list: &mut DrawList,
        //     theme: &Theme,
        //     input: &InputState,
        //     screen_width: f32,
        //     screen_height: f32,
        // ) -> Option<i8> {
        //     let layout = floor_switcher.layout_screen(screen_width, screen_height);
        //     let mut new_floor = None;
        //
        //     // Draw panel background
        //     Panel::draw_at(layout.get(0), list, theme);
        //
        //     // Up button
        //     if Button::draw_at("▲", layout.get(1), current_floor < 3, list, theme, input) {
        //         new_floor = Some(current_floor + 1);
        //     }
        //
        //     // Floor label
        //     let floor_name = match current_floor {
        //         -2 => "B2", -1 => "B1", 0 => "Ground",
        //         1 => "F1", 2 => "F2", 3 => "F3", _ => "?",
        //     };
        //     label_centered_at(list, theme, floor_name, layout.get(2));
        //
        //     // Down button
        //     if Button::draw_at("▼", layout.get(3), current_floor > -2, list, theme, input) {
        //         new_floor = Some(current_floor - 1);
        //     }
        //
        //     new_floor
        // }
    }

    // ---- Per-child cross-axis alignment ----

    #[test]
    fn vstack_align_stretch_fills_inner_width() {
        // Stretch (default) is the same as the old behavior: child fills full width.
        let stack = VStack::new(0.0)
            .child(30.0, 40.0) // cross_size=40, but Stretch ignores it
            .child(20.0, 40.0);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        assert_eq!(result.get(1).x, 10.0);
        assert_eq!(result.get(1).width, 100.0);
        assert_eq!(result.get(2).x, 10.0);
        assert_eq!(result.get(2).width, 100.0);
    }

    #[test]
    fn vstack_align_start_pins_left() {
        let stack = VStack::new(0.0).child(30.0, 40.0).align(CrossAlign::Start);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        assert_eq!(result.get(1).x, 10.0, "start aligns to left edge");
        assert_eq!(
            result.get(1).width, 40.0,
            "start uses cross_size, not inner_width"
        );
    }

    #[test]
    fn vstack_align_center_centers_horizontally() {
        let stack = VStack::new(0.0).child(30.0, 40.0).align(CrossAlign::Center);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        // inner_width = 100 (no padding), center = 10 + (100 - 40) / 2 = 40
        assert_eq!(result.get(1).x, 40.0);
        assert_eq!(result.get(1).width, 40.0);
    }

    #[test]
    fn vstack_align_end_pins_right() {
        let stack = VStack::new(0.0).child(30.0, 40.0).align(CrossAlign::End);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        // inner_width = 100, end = 10 + 100 - 40 = 70
        assert_eq!(result.get(1).x, 70.0);
        assert_eq!(result.get(1).width, 40.0);
    }

    #[test]
    fn vstack_align_respects_padding() {
        let stack = VStack::new(0.0)
            .with_padding(10.0)
            .child(30.0, 40.0)
            .align(CrossAlign::Center);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        // inner_width = 100 - 20 = 80, center x = 10 + 10 + (80 - 40) / 2 = 40
        assert_eq!(result.get(1).x, 40.0);
        assert_eq!(result.get(1).width, 40.0);
    }

    #[test]
    fn vstack_align_clamps_cross_size_to_inner_width() {
        // cross_size larger than inner_width should be clamped.
        let stack = VStack::new(0.0).child(30.0, 200.0).align(CrossAlign::Start);
        let result = stack.layout(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(result.get(1).width, 100.0, "clamped to inner_width");
    }

    #[test]
    fn hstack_align_start_pins_top() {
        let stack = HStack::new(0.0).child(30.0, 20.0).align(CrossAlign::Start);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        assert_eq!(result.get(1).y, 10.0, "start aligns to top edge");
        assert_eq!(
            result.get(1).height, 20.0,
            "start uses cross_size, not inner_height"
        );
    }

    #[test]
    fn hstack_align_center_centers_vertically() {
        let stack = HStack::new(0.0).child(30.0, 20.0).align(CrossAlign::Center);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        // inner_height = 100, center y = 10 + (100 - 20) / 2 = 50
        assert_eq!(result.get(1).y, 50.0);
        assert_eq!(result.get(1).height, 20.0);
    }

    #[test]
    fn hstack_align_end_pins_bottom() {
        let stack = HStack::new(0.0).child(30.0, 20.0).align(CrossAlign::End);
        let result = stack.layout(Rect::new(10.0, 10.0, 100.0, 100.0));
        // inner_height = 100, end y = 10 + 100 - 20 = 90
        assert_eq!(result.get(1).y, 90.0);
        assert_eq!(result.get(1).height, 20.0);
    }

    #[test]
    fn hstack_multiple_children_mixed_alignment() {
        // Two children, each with different cross-axis alignment.
        let stack = HStack::new(10.0)
            .with_padding(5.0)
            .child(40.0, 30.0)
            .align(CrossAlign::Start) // pinned to top
            .child(40.0, 30.0)
            .align(CrossAlign::End); // pinned to bottom
        let result = stack.layout(Rect::new(0.0, 0.0, 200.0, 100.0));
        // inner_height = 100 - 10 = 90
        assert_eq!(result.get(1).height, 30.0, "start child uses cross_size");
        assert_eq!(result.get(1).y, 5.0, "start child at top edge");
        assert_eq!(result.get(2).height, 30.0, "end child uses cross_size");
        assert_eq!(
            result.get(2).y, 65.0,
            "end child at bottom edge (5 + 90 - 30)"
        );
    }

    #[test]
    fn vstack_mixed_alignment_in_stack() {
        // Two fixed children with different alignments + one fill child.
        let stack = VStack::new(4.0)
            .with_padding(4.0)
            .child(20.0, 50.0)
            .align(CrossAlign::Center)
            .child_fill(60.0) // default Stretch
            .child(20.0, 40.0)
            .align(CrossAlign::End);
        let result = stack.layout(Rect::new(0.0, 0.0, 120.0, 200.0));
        // inner_width = 120 - 8 = 112
        assert_eq!(result.get(1).width, 50.0, "center child uses cross_size");
        assert_eq!(
            result.get(1).x,
            4.0 + (112.0 - 50.0) * 0.5,
            "center child x"
        );
        assert_eq!(
            result.get(2).width, 112.0,
            "fill child stretches full width"
        );
        assert_eq!(result.get(3).width, 40.0, "end child uses cross_size");
        assert_eq!(
            result.get(3).x,
            4.0 + 112.0 - 40.0,
            "end child at right edge"
        );
    }

    // ---- Weighted fill children ----

    #[test]
    fn hstack_weighted_fill_splits_2_to_1() {
        // No padding/spacing: 300px split 2:1 → 200 / 100.
        let stack = HStack::new(0.0)
            .child_fill(30.0)
            .weight(2.0)
            .child_fill(30.0)
            .weight(1.0);
        let result = stack.layout(Rect::new(0.0, 0.0, 300.0, 40.0));
        assert_eq!(result.get(1).width, 200.0, "weight 2 child gets 2/3");
        assert_eq!(result.get(2).width, 100.0, "weight 1 child gets 1/3");
        assert_eq!(result.get(2).x, 200.0, "second child starts after first");
    }

    #[test]
    fn vstack_weighted_fill_splits_three_ways() {
        // 2:1:1 over 400px → 200 / 100 / 100.
        let stack = VStack::new(0.0)
            .child_fill(10.0)
            .weight(2.0)
            .child_fill(10.0)
            .weight(1.0)
            .child_fill(10.0)
            .weight(1.0);
        let result = stack.layout(Rect::new(0.0, 0.0, 50.0, 400.0));
        assert_eq!(result.get(1).height, 200.0);
        assert_eq!(result.get(2).height, 100.0);
        assert_eq!(result.get(3).height, 100.0);
    }

    #[test]
    fn equal_weight_is_byte_identical_to_unweighted() {
        // The back-compat invariant: default weight 1.0 reproduces the old
        // remaining / fill_count split exactly.
        let weighted = HStack::new(0.0).child_fill(10.0).child_fill(10.0);
        let r = weighted.layout(Rect::new(0.0, 0.0, 300.0, 20.0));
        assert_eq!(r.get(1).width, 150.0);
        assert_eq!(r.get(2).width, 150.0);
    }

    #[test]
    fn zero_weight_fill_gets_nothing() {
        // A lone fill at weight 0 → fill_weight is 0 → 0px (no div-by-zero).
        let stack = HStack::new(0.0).child(100.0, 20.0).child_fill(20.0).weight(0.0);
        let result = stack.layout(Rect::new(0.0, 0.0, 300.0, 20.0));
        assert_eq!(result.get(1).width, 100.0, "fixed child unaffected");
        assert_eq!(result.get(2).width, 0.0, "zero-weight fill gets no space");
    }

    #[test]
    fn weighted_fill_clamped_does_not_redistribute() {
        // Single-pass: a weighted fill clamped by a constraint keeps its slack
        // (no second redistribution pass), matching the unweighted behavior.
        let stack = HStack::new(0.0)
            .child_fill(20.0)
            .weight(3.0)
            .constrain(Constraint::max(50.0))
            .child_fill(20.0)
            .weight(1.0);
        let result = stack.layout(Rect::new(0.0, 0.0, 400.0, 20.0));
        // Raw split: 300 / 100; first clamped to 50, second stays 100.
        assert_eq!(result.get(1).width, 50.0, "clamped to max");
        assert_eq!(result.get(2).width, 100.0, "sibling keeps its 1/4 share");
    }

    #[test]
    fn negative_weight_clamped_to_zero() {
        let stack = HStack::new(0.0)
            .child_fill(20.0)
            .weight(-5.0)
            .child_fill(20.0)
            .weight(1.0);
        let result = stack.layout(Rect::new(0.0, 0.0, 300.0, 20.0));
        assert_eq!(result.get(1).width, 0.0, "negative weight clamped to 0");
        assert_eq!(result.get(2).width, 300.0, "other child takes all");
    }

    // ---- Main-axis justification (justify-content) ----

    #[test]
    fn justify_start_is_byte_identical_default() {
        // Default (no .justify) and explicit Start must match exactly.
        let a = HStack::new(10.0).child(40.0, 20.0).child(40.0, 20.0);
        let b = HStack::new(10.0)
            .justify(MainAlign::Start)
            .child(40.0, 20.0)
            .child(40.0, 20.0);
        let ra = a.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        let rb = b.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        assert_eq!(ra.get(1).x, rb.get(1).x);
        assert_eq!(ra.get(2).x, rb.get(2).x);
        assert_eq!(ra.get(1).x, 0.0);
        assert_eq!(ra.get(2).x, 50.0, "40 width + 10 spacing");
    }

    #[test]
    fn justify_center_offsets_group() {
        // Two 40px items + 10px gap = 90 content; 200 wide → 110 free → 55 lead.
        let stack = HStack::new(10.0)
            .justify(MainAlign::Center)
            .child(40.0, 20.0)
            .child(40.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        assert_eq!(r.get(1).x, 55.0);
        assert_eq!(r.get(2).x, 105.0, "55 + 40 + 10");
    }

    #[test]
    fn justify_end_pushes_to_far_edge() {
        let stack = HStack::new(10.0)
            .justify(MainAlign::End)
            .child(40.0, 20.0)
            .child(40.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        // Last item ends flush at the right edge.
        assert_eq!(r.get(2).x + r.get(2).width, 200.0);
        assert_eq!(r.get(1).x, 110.0, "200 - 90 content");
    }

    #[test]
    fn justify_space_between_spreads_to_edges() {
        // 3 items × 40 = 120 (gap 0 for clean math); 240 wide → 120 free over 2
        // gaps = 60 each.
        let stack = HStack::new(0.0)
            .justify(MainAlign::SpaceBetween)
            .child(40.0, 20.0)
            .child(40.0, 20.0)
            .child(40.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 240.0, 30.0));
        assert_eq!(r.get(1).x, 0.0, "first flush left");
        assert_eq!(r.get(2).x, 100.0, "40 + 60 gap");
        assert_eq!(r.get(3).x + r.get(3).width, 240.0, "last flush right");
    }

    #[test]
    fn justify_space_between_single_child_is_start() {
        let stack = HStack::new(0.0)
            .justify(MainAlign::SpaceBetween)
            .child(40.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        assert_eq!(r.get(1).x, 0.0, "lone child stays at start");
    }

    #[test]
    fn justify_space_around_half_end_gaps() {
        // 2 items × 50 = 100; 200 wide → 100 free; around → each item gets 50,
        // so 25 lead, 50 between, 25 trail.
        let stack = HStack::new(0.0)
            .justify(MainAlign::SpaceAround)
            .child(50.0, 20.0)
            .child(50.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        assert_eq!(r.get(1).x, 25.0, "half-gap lead");
        assert_eq!(r.get(2).x, 125.0, "25 + 50 + 50 gap");
    }

    #[test]
    fn justify_space_evenly_equal_gaps() {
        // 2 items × 50 = 100; 200 wide → 100 free over 3 equal gaps ≈ 33.33.
        let stack = HStack::new(0.0)
            .justify(MainAlign::SpaceEvenly)
            .child(50.0, 20.0)
            .child(50.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        let gap = 100.0 / 3.0;
        assert!((r.get(1).x - gap).abs() < 0.001, "lead gap");
        assert!((r.get(2).x - (gap + 50.0 + gap)).abs() < 0.001);
    }

    #[test]
    fn justify_is_noop_with_fill_child() {
        // A Fill child consumes the slack, so justify has nothing to distribute:
        // result matches Start exactly.
        let justified = HStack::new(0.0)
            .justify(MainAlign::SpaceBetween)
            .child(40.0, 20.0)
            .child_fill(20.0);
        let plain = HStack::new(0.0).child(40.0, 20.0).child_fill(20.0);
        let rj = justified.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        let rp = plain.layout(Rect::new(0.0, 0.0, 200.0, 30.0));
        assert_eq!(rj.get(1).x, rp.get(1).x);
        assert_eq!(rj.get(2).x, rp.get(2).x);
        assert_eq!(rj.get(2).width, rp.get(2).width);
    }

    #[test]
    fn justify_vstack_center_offsets_vertically() {
        // Mirror of the HStack center test on the vertical axis.
        let stack = VStack::new(10.0)
            .justify(MainAlign::Center)
            .child(40.0, 20.0)
            .child(40.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 30.0, 200.0));
        assert_eq!(r.get(1).y, 55.0);
        assert_eq!(r.get(2).y, 105.0);
    }

    #[test]
    fn justify_respects_padding() {
        // padding 20 → inner 160; 2×40 content = 80 → 80 free; center lead 40,
        // plus padding 20 = 60.
        let stack = HStack::new(0.0)
            .with_padding(20.0)
            .justify(MainAlign::Center)
            .child(40.0, 20.0)
            .child(40.0, 20.0);
        let r = stack.layout(Rect::new(0.0, 0.0, 200.0, 60.0));
        assert_eq!(r.get(1).x, 60.0, "padding 20 + center lead 40");
    }

    // ---- Flow (wrap) layout ----

    #[test]
    fn flow_wraps_into_rows() {
        // Three 100px items, 10px spacing, in a 250px-wide bound: items 0 and 1
        // fit on row 0 (100 + 10 + 100 = 210 <= 250); item 2 (would reach 320)
        // wraps to row 1.
        let flow = Flow::new(10.0).item(100.0, 40.0).item(100.0, 40.0).item(100.0, 40.0);
        let r = flow.layout(Rect::new(0.0, 0.0, 250.0, 200.0));
        assert_eq!(r.len(), 4, "container + 3 items");
        // Row 0.
        assert_eq!((r.get(1).x, r.get(1).y), (0.0, 0.0));
        assert_eq!((r.get(2).x, r.get(2).y), (110.0, 0.0));
        // Row 1: item 2 drops below, x resets.
        assert_eq!(r.get(3).x, 0.0, "wrapped item resets to left");
        assert_eq!(r.get(3).y, 50.0, "wrapped to row 1 (row_h 40 + spacing 10)");
    }

    #[test]
    fn flow_padding_and_run_spacing_offsets() {
        // padding 8, item gap 4, distinct run gap 20.
        let flow = Flow::new(4.0)
            .with_run_spacing(20.0)
            .with_padding(8.0)
            .item(50.0, 30.0)
            .item(50.0, 30.0); // second overflows inner width 100-? -> wraps
        // inner_w = 120 - 16 = 104; first at x=8 reaches 8+50+4=62; second would
        // reach 62+50=112 > 8+104=112? exactly equal, not greater -> stays. Use a
        // narrower bound to force a wrap.
        let r = flow.layout(Rect::new(0.0, 0.0, 100.0, 200.0));
        // inner_w = 100 - 16 = 84; item 0 at (8,8); item 1 (would reach 62+50=112
        // > 8+84=92) wraps.
        assert_eq!((r.get(1).x, r.get(1).y), (8.0, 8.0), "first item at padding");
        assert_eq!(r.get(2).x, 8.0, "second wraps to left padding");
        assert_eq!(
            r.get(2).y, 8.0 + 30.0 + 20.0,
            "second on row 1 (padding + row_h + run_spacing)"
        );
    }

    #[test]
    fn flow_measure_height_matches_last_rect_bottom() {
        // The cross-check that measure_height and layout agree.
        let flow = Flow::new(10.0)
            .with_padding(6.0)
            .item(80.0, 30.0)
            .item(80.0, 50.0)
            .item(80.0, 20.0);
        let width = 200.0;
        let r = flow.layout(Rect::new(0.0, 0.0, width, 500.0));
        let last_bottom = r
            .children()
            .map(|rc| rc.y + rc.height)
            .fold(0.0_f32, f32::max);
        // measure_height includes the bottom padding; last_bottom does not.
        assert_eq!(flow.measure_height(width), last_bottom + 6.0);
    }

    #[test]
    fn flow_empty_is_just_bounds() {
        let flow = Flow::new(8.0);
        let bounds = Rect::new(1.0, 2.0, 100.0, 50.0);
        let r = flow.layout(bounds);
        assert_eq!(r.len(), 1);
        assert_eq!(r.get(0), bounds);
        assert_eq!(flow.measure_height(100.0), 0.0, "empty flow has no content height");
    }

    #[test]
    fn flow_oversized_item_is_placed_once() {
        // An item wider than the inner width must still be placed exactly once
        // (no infinite wrap loop), overflowing the bound.
        let flow = Flow::new(4.0).item(500.0, 30.0).item(20.0, 30.0);
        let r = flow.layout(Rect::new(0.0, 0.0, 100.0, 200.0));
        assert_eq!(r.len(), 3, "both items placed");
        assert_eq!((r.get(1).x, r.get(1).y), (0.0, 0.0), "oversized item on row 0");
        // The small item can't share row 0 (cur_x already past inner_w) -> row 1.
        assert_eq!(r.get(2).x, 0.0);
        assert_eq!(r.get(2).y, 34.0, "small item wraps below oversized one");
    }

    // ---- Stable node IDs (get_by_id) ----

    #[test]
    fn get_by_id_resolves_tagged_children() {
        let stack = HStack::new(0.0)
            .child(40.0, 20.0)
            .id(10)
            .child(60.0, 20.0)
            .id(20);
        let r = stack.layout(Rect::new(0.0, 0.0, 100.0, 20.0));
        assert_eq!(r.get_by_id(10), Some(r.get(1)));
        assert_eq!(r.get_by_id(20), Some(r.get(2)));
        assert_eq!(r.get_by_id(10).unwrap().width, 40.0);
        assert_eq!(r.get_by_id(20).unwrap().x, 40.0);
    }

    #[test]
    fn get_by_id_is_order_independent() {
        // The core regression `NodeId` prevents: reordering children shifts every
        // positional index, but id lookup still resolves to the same logical node.
        let a = HStack::new(0.0).child(40.0, 20.0).id(1).child(60.0, 20.0).id(2);
        let b = HStack::new(0.0).child(60.0, 20.0).id(2).child(40.0, 20.0).id(1);
        let ra = a.layout(Rect::new(0.0, 0.0, 100.0, 20.0));
        let rb = b.layout(Rect::new(0.0, 0.0, 100.0, 20.0));
        // Positional indices disagree after the swap...
        assert_ne!(ra.get(1).width, rb.get(1).width);
        // ...but each id still maps to a 40-wide / 60-wide box respectively.
        assert_eq!(ra.get_by_id(1).unwrap().width, 40.0);
        assert_eq!(rb.get_by_id(1).unwrap().width, 40.0);
        assert_eq!(ra.get_by_id(2).unwrap().width, 60.0);
        assert_eq!(rb.get_by_id(2).unwrap().width, 60.0);
    }

    #[test]
    fn get_by_id_unknown_and_untagged_return_none() {
        let stack = VStack::new(0.0).child(10.0, 20.0).id(7).child(10.0, 20.0); // 2nd untagged
        let r = stack.layout(Rect::new(0.0, 0.0, 20.0, 40.0));
        assert_eq!(r.get_by_id(99), None, "unknown id");
        // The untagged child has no id, so nothing resolves to it by id.
        assert_eq!(r.get_by_id(0), None);
        assert_eq!(r.get_by_id(7), Some(r.get(1)));
    }

    #[test]
    fn flow_item_id_is_addressable() {
        let flow = Flow::new(8.0)
            .item(40.0, 40.0)
            .item_id(5, 40.0, 40.0)
            .item(40.0, 40.0);
        let r = flow.layout(Rect::new(0.0, 0.0, 200.0, 60.0));
        assert_eq!(r.get_by_id(5), Some(r.get(2)), "tagged tile resolves by id");
        assert_eq!(r.get_by_id(0), None, "untagged tiles have no id");
    }

    // ---- Arena / buffer reuse (layout_into) ----

    #[test]
    fn layout_into_clears_stale_entries() {
        let big = VStack::new(0.0).child(10.0, 20.0).child(10.0, 20.0).child(10.0, 20.0);
        let small = VStack::new(0.0).child(10.0, 20.0);
        let mut buf = LayoutResult::default();
        big.layout_into(Rect::new(0.0, 0.0, 20.0, 60.0), &mut buf);
        assert_eq!(buf.len(), 4, "container + 3 children");
        // Reusing the same buffer for a smaller tree must not leave stragglers.
        small.layout_into(Rect::new(0.0, 0.0, 20.0, 20.0), &mut buf);
        assert_eq!(buf.len(), 2, "container + 1 child; stale entries cleared");
    }

    #[test]
    fn layout_into_matches_fresh_layout() {
        let stack = HStack::new(6.0)
            .child(40.0, 20.0)
            .child_fill(20.0)
            .weight(2.0)
            .child_fill(20.0);
        let bounds = Rect::new(3.0, 5.0, 300.0, 40.0);
        let fresh = stack.layout(bounds);
        let mut buf = LayoutResult::default();
        stack.layout_into(bounds, &mut buf);
        assert_eq!(buf.len(), fresh.len());
        for i in 0..fresh.len() {
            assert_eq!(buf.get(i), fresh.get(i), "entry {i} must match fresh layout");
        }
    }

    #[test]
    fn layout_into_reuses_capacity() {
        // Sanity that we reuse the buffer's allocation rather than reallocating
        // every frame: after the first fill, capacity covers the entry count and
        // does not shrink on subsequent equal-size fills.
        let stack = VStack::new(0.0)
            .child(10.0, 20.0)
            .child(10.0, 20.0)
            .child(10.0, 20.0);
        let bounds = Rect::new(0.0, 0.0, 20.0, 60.0);
        let mut buf = LayoutResult::default();
        stack.layout_into(bounds, &mut buf);
        let cap = buf.entries.capacity();
        assert!(cap >= 4);
        stack.layout_into(bounds, &mut buf);
        assert_eq!(buf.entries.capacity(), cap, "capacity reused, no realloc");
    }
}
