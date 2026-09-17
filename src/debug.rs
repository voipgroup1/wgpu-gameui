//! Layout inspection — what a frame actually drew, and what looks wrong with it.
//!
//! The problem this solves: a [`DrawList`] is a bag of world-space geometry with
//! no structure and no names, so "did my button land inside its panel, on
//! screen, aligned with its neighbours?" is not a question anyone — human or
//! agent — can answer by reading it. Rendering a PNG answers it only if you can
//! see, and only approximately.
//!
//! [`DebugReport`] turns a finished frame into a **named, nested tree of
//! bounding boxes plus a list of concrete problems**, renderable as indented
//! text or JSON:
//!
//! ```ignore
//! let report = DebugReport::measured(&mut list, Rect::new(0.0, 0.0, 800.0, 600.0));
//! println!("{}", report.to_text());
//! report.assert_clean();   // fails the test if the layout regressed
//! ```
//!
//! # Names are labels; declared rects are what unlock checks
//!
//! These are two separate things, and it is worth not conflating them — they
//! merely happen to arrive together via
//! [`push_debug_scope_rect`](DrawList::push_debug_scope_rect).
//!
//! A **name** is presentation. Anything drawn outside a scope still appears,
//! named on a best-effort basis — a text block by its own content, an icon by
//! its atlas key, otherwise by primitive kind and index — and is marked
//! [`DebugNode::named`]` == false`. Such a node is *not* a less-checked node:
//! every geometric lint (off-screen, fully/partially clipped, text overflowing
//! its box, dropped degenerate primitives) runs on it identically, falling back
//! to the bounds it actually painted. A misplaced widget is reported whether or
//! not anyone named it; the name only decides how legible the resulting line is.
//!
//! A **declared rect** is intent, and intent cannot be inferred from geometry at
//! all. A draw list records what was painted, never what layout assigned, so
//! "this is at x=100 and should have been at x=140" is unanswerable here — the
//! second number only ever existed in the layout code that already ran.
//! Declaring a rect supplies it, which is what
//! [`Problem::OverflowsDeclared`] and [`Problem::MissingPaint`] compare against;
//! without one a region's bounds are derived from what it painted, so it can
//! never be found to have overflowed itself, and a widget that collapsed to
//! nothing looks identical to one that was never drawn.
//!
//! Declaring is the **widget implementor's** job, not the application's — every
//! widget in this crate declares the `Rect` it was handed, so its allocation is
//! checked without anyone opting in. Application code has no rect to offer that
//! layout does not already own, which is why [`UiContext::debug_scope`] takes a
//! name and nothing else.
//!
//! [`UiContext::debug_scope`]: crate::UiContext::debug_scope
//!
//! The single exception where naming affects detection is **near-miss
//! alignment**, which is a claim about intent rather than geometry: that one
//! piece of layout code placed these elements together and meant their edges to
//! match. It therefore runs only between **named siblings inside a scope** —
//! anywhere else the parent was inferred from geometry, and an inferred parent
//! can be a bounding box drawn around unrelated shapes. (Named siblings only for
//! a second reason: a widget's own sub-primitives — a panel's four border quads —
//! sit a pixel apart by construction.) To get alignment analysis over a region,
//! name it: `ui.debug_scope("Toolbar", |ui| …)`.
//!
//! # Tree order is paint submission order
//!
//! The renderer walks the [`DrawList`] paint-command stream, which can freely
//! interleave nine-slices, colour geometry, icons, and text. Sibling nodes (and
//! scopes through their earliest painted descendant) therefore appear in that
//! same submission order. [`RenderPass`] remains on each leaf as useful pipeline
//! metadata; it no longer determines tree order.

use crate::layer::LayerStack;
use crate::layout::Rect;
use crate::render::RenderStats;
use crate::text::{TextAlign, TextBlock};
use crate::widgets::{DrawList, PaintCmd, PrimCounts};

/// Which pipeline family draws a node.
///
/// This is metadata, not an ordering key: the submission-order renderer may
/// interleave these families arbitrarily through its paint-command stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderPass {
    /// Nine-slice backgrounds (drawn first, under everything).
    NineSlice,
    /// Soup geometry and instanced chrome/circles.
    Color,
    /// Textured atlas icons and images.
    Icon,
    /// MSDF vector (Phosphor) icons.
    IconMsdf,
    /// Text glyphs (drawn last, over everything).
    Text,
}

impl RenderPass {
    /// Short lowercase label used in the text and JSON renderings.
    pub fn label(&self) -> &'static str {
        match self {
            RenderPass::NineSlice => "nine_slice",
            RenderPass::Color => "color",
            RenderPass::Icon => "icon",
            RenderPass::IconMsdf => "icon_msdf",
            RenderPass::Text => "text",
        }
    }
}

/// What a [`DebugNode`] represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A named region opened with [`DrawList::push_debug_scope`].
    Scope,
    /// A whole [`Layer`](crate::Layer) of a [`LayerStack`].
    Layer,
    /// An instanced chrome rect (panel/button background, outline).
    Chrome,
    /// An instanced circle or ring.
    Circle,
    /// Aggregated triangle-soup geometry (gradients, polygons, lines).
    Geometry,
    /// A textured atlas icon or image.
    Icon,
    /// An MSDF vector icon.
    IconMsdf,
    /// A nine-slice panel.
    NineSlice,
    /// A text block.
    Text,
}

impl NodeKind {
    /// Short lowercase label used in the text and JSON renderings.
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Scope => "scope",
            NodeKind::Layer => "layer",
            NodeKind::Chrome => "chrome",
            NodeKind::Circle => "circle",
            NodeKind::Geometry => "geometry",
            NodeKind::Icon => "icon",
            NodeKind::IconMsdf => "icon_msdf",
            NodeKind::NineSlice => "nine_slice",
            NodeKind::Text => "text",
        }
    }
}

/// One entry in a [`DebugReport`] — a named box with everything known about it.
#[derive(Clone, Debug)]
pub struct DebugNode {
    /// Index of this node in [`DebugReport::nodes`].
    pub id: usize,
    /// Enclosing node, if any.
    pub parent: Option<usize>,
    /// Nesting depth (0 = top level).
    pub depth: usize,
    /// Display name. From the debug scope when [`named`](Self::named), otherwise
    /// inferred — a text block's own content, an icon's atlas key, or
    /// `kind#index`.
    pub name: String,
    /// True when the name came from a real debug scope rather than inference.
    pub named: bool,
    /// What this node is.
    pub kind: NodeKind,
    /// Which pipeline family draws it (metadata, not tree order).
    pub pass: RenderPass,
    /// The area actually painted, in world space. For text this is the **ink**
    /// box (alignment resolved), not the layout box — see
    /// [`text_box`](Self::text_box).
    pub bounds: Rect,
    /// The box a scope said it would paint into, when it declared one.
    /// Comparing this against [`bounds`](Self::bounds) is the only authoritative
    /// overflow check.
    pub declared: Option<Rect>,
    /// The text layout box (`x`, `y`, `max_width`, measured height) for text
    /// nodes. Differs from `bounds` whenever the block is centred or
    /// right-aligned, or is narrower than `max_width`.
    pub text_box: Option<Rect>,
    /// Effective clip rect in world space, when clipped.
    pub clip: Option<Rect>,
    /// Text content, for text nodes.
    pub text: Option<String>,
    /// Primitives this node accounts for (a scope includes its descendants').
    pub counts: PrimCounts,
    /// False when a rotation or shear was active, meaning `bounds` is an
    /// inflated axis-aligned approximation. Geometry lints are suppressed for
    /// these rather than reported against a box that was never really there.
    pub axis_aligned: bool,
    /// Effects that paint outside the measured bounds (`"shadow"`, `"outline"`,
    /// `"glow"`), so a reader knows the bounds under-report.
    pub effects: Vec<&'static str>,
    /// True for a text node laid out in single-line ellipsis mode. Such a block
    /// is *supposed* to exceed its box (that is what triggers the ellipsis), so
    /// the overflow check is suppressed and reported as
    /// [`Problem::TextEllipsized`] instead.
    pub ellipsize: bool,
    /// Index into [`LayerStack::layers`] when the node came from an overlay
    /// layer; `None` for the base list.
    pub layer: Option<usize>,
}

impl DebugNode {
    /// Bounds to lint against: the declared box when there is one, else what was
    /// actually painted.
    fn lint_rect(&self) -> Rect {
        self.declared.unwrap_or(self.bounds)
    }
}

/// How much a [`Problem`] matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing, not necessarily wrong (e.g. a label was ellipsized).
    Info,
    /// Very likely a bug, but legitimate in some designs.
    Warning,
    /// Something is definitely not visible or not where it was asked to be.
    Error,
}

impl Severity {
    /// Uppercase label used in the text rendering.
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARN",
            Severity::Error => "ERROR",
        }
    }
}

/// A specific thing that looks wrong with the frame.
///
/// Every variant carries the node it concerns plus the rects that justify the
/// call, so the report is actionable without reading this crate's source.
#[derive(Clone, Debug)]
pub enum Problem {
    /// The node lies entirely outside the screen — completely invisible.
    OffScreen {
        /// Node index.
        node: usize,
        /// Where the node is.
        bounds: Rect,
        /// The viewport it missed.
        screen: Rect,
    },
    /// Part of the node hangs off the screen edge.
    PartiallyOffScreen {
        /// Node index.
        node: usize,
        /// Where the node is.
        bounds: Rect,
        /// The viewport.
        screen: Rect,
    },
    /// The active clip removes the node entirely — it is drawn but invisible.
    FullyClipped {
        /// Node index.
        node: usize,
        /// Where the node is.
        bounds: Rect,
        /// The clip that swallowed it.
        clip: Rect,
    },
    /// Part of the node is clipped away.
    PartiallyClipped {
        /// Node index.
        node: usize,
        /// Where the node is.
        bounds: Rect,
        /// The clip.
        clip: Rect,
    },
    /// A scope painted outside the box it declared.
    OverflowsDeclared {
        /// Node index.
        node: usize,
        /// What it actually painted.
        painted: Rect,
        /// What it said it would paint into.
        declared: Rect,
        /// Worst overshoot in pixels across the four sides.
        overshoot: f32,
    },
    /// A scope declared a rect and then painted nothing at all.
    MissingPaint {
        /// Node index.
        node: usize,
        /// The box that stayed empty.
        declared: Rect,
    },
    /// A scope declared a rect with no area, so nothing inside it could draw.
    DegenerateDeclaredRect {
        /// Node index.
        node: usize,
        /// The empty box.
        declared: Rect,
    },
    /// Primitives were dropped because a computed size came out non-positive.
    /// These leave no geometry behind at all — see
    /// [`DrawList::dropped_degenerate`].
    DroppedDegenerate {
        /// Node index (the enclosing scope, or the root).
        node: usize,
        /// How many primitives vanished.
        count: usize,
    },
    /// A text block's ink is wider or taller than its layout box, so it spills
    /// past where the layout thinks it ends.
    TextOverflowsBox {
        /// Node index.
        node: usize,
        /// The shaped ink.
        ink: Rect,
        /// The layout box.
        text_box: Rect,
    },
    /// A text block was truncated with an ellipsis. Informational: it means the
    /// content did not fit, which may be intended.
    TextEllipsized {
        /// Node index.
        node: usize,
        /// Width the content wanted.
        wanted: f32,
        /// Width it was given.
        available: f32,
    },
    /// A node sits *almost* on an edge shared by its siblings — the signature of
    /// an off-by-a-fraction layout bug.
    NearMissAlignment {
        /// Node index.
        node: usize,
        /// Which edge (`"left"`, `"right"`, `"top"`, `"bottom"`, `"center_x"`,
        /// `"center_y"`).
        edge: &'static str,
        /// Where this node's edge is.
        actual: f32,
        /// Where its siblings agree it should be.
        expected: f32,
        /// `actual - expected`.
        delta: f32,
    },
    /// A chrome rect is fully transparent in both fill and border — it occupies
    /// space in the draw list but paints nothing.
    InvisibleAlpha {
        /// Node index.
        node: usize,
        /// Where the invisible rect is.
        bounds: Rect,
    },
    /// Two sibling nodes overlap. Off by default: backgrounds legitimately sit
    /// under their own contents.
    SiblingOverlap {
        /// First node index.
        a: usize,
        /// Second node index.
        b: usize,
        /// The overlapping region.
        overlap: Rect,
    },
}

impl Problem {
    /// Index of the node this problem is about (the first of the pair, for
    /// [`SiblingOverlap`](Problem::SiblingOverlap)).
    pub fn node(&self) -> usize {
        match self {
            Problem::OffScreen { node, .. }
            | Problem::PartiallyOffScreen { node, .. }
            | Problem::FullyClipped { node, .. }
            | Problem::PartiallyClipped { node, .. }
            | Problem::OverflowsDeclared { node, .. }
            | Problem::MissingPaint { node, .. }
            | Problem::DegenerateDeclaredRect { node, .. }
            | Problem::DroppedDegenerate { node, .. }
            | Problem::TextOverflowsBox { node, .. }
            | Problem::TextEllipsized { node, .. }
            | Problem::NearMissAlignment { node, .. }
            | Problem::InvisibleAlpha { node, .. } => *node,
            Problem::SiblingOverlap { a, .. } => *a,
        }
    }

    /// Stable machine-readable identifier, e.g. `"off_screen"`.
    pub fn code(&self) -> &'static str {
        match self {
            Problem::OffScreen { .. } => "off_screen",
            Problem::PartiallyOffScreen { .. } => "partially_off_screen",
            Problem::FullyClipped { .. } => "fully_clipped",
            Problem::PartiallyClipped { .. } => "partially_clipped",
            Problem::OverflowsDeclared { .. } => "overflows_declared",
            Problem::MissingPaint { .. } => "missing_paint",
            Problem::DegenerateDeclaredRect { .. } => "degenerate_declared_rect",
            Problem::DroppedDegenerate { .. } => "dropped_degenerate",
            Problem::TextOverflowsBox { .. } => "text_overflows_box",
            Problem::TextEllipsized { .. } => "text_ellipsized",
            Problem::NearMissAlignment { .. } => "near_miss_alignment",
            Problem::InvisibleAlpha { .. } => "invisible_alpha",
            Problem::SiblingOverlap { .. } => "sibling_overlap",
        }
    }

    /// How much this matters.
    pub fn severity(&self) -> Severity {
        match self {
            Problem::OffScreen { .. }
            | Problem::MissingPaint { .. }
            | Problem::DegenerateDeclaredRect { .. }
            | Problem::FullyClipped { .. }
            | Problem::DroppedDegenerate { .. } => Severity::Error,
            Problem::PartiallyOffScreen { .. }
            | Problem::PartiallyClipped { .. }
            | Problem::OverflowsDeclared { .. }
            | Problem::TextOverflowsBox { .. }
            | Problem::NearMissAlignment { .. }
            | Problem::InvisibleAlpha { .. }
            | Problem::SiblingOverlap { .. } => Severity::Warning,
            Problem::TextEllipsized { .. } => Severity::Info,
        }
    }

    /// One-line explanation of the likely cause, so a reader does not have to
    /// consult this crate's source to act on the finding.
    pub fn hint(&self) -> &'static str {
        match self {
            Problem::OffScreen { .. } => {
                "the element is entirely outside the viewport — check its anchor/offset, \
                 or whether a parent rect was computed before the screen size was known"
            }
            Problem::PartiallyOffScreen { .. } => {
                "the element crosses a screen edge — it is probably positioned from the \
                 wrong corner, or is wider than the space left for it"
            }
            Problem::FullyClipped { .. } => {
                "drawn, but entirely removed by a clip that is a hard boundary, not a \
                 viewport — the element is invisible. Check its position against the \
                 container it was meant to sit in. (Content scrolled out of a scroll view \
                 or dropdown list is *not* reported here.)"
            }
            Problem::PartiallyClipped { .. } => {
                "part of the element is cut off by the clip; expected inside a scroll view, \
                 suspicious anywhere else"
            }
            Problem::OverflowsDeclared { .. } => {
                "painted outside the rect this scope declared — content is larger than the \
                 box reserved for it, so it will collide with whatever sits next to it"
            }
            Problem::MissingPaint { .. } => {
                "the scope reserved a box and drew nothing into it — a conditional that never \
                 fired, or a widget that bailed out early"
            }
            Problem::DegenerateDeclaredRect { .. } => {
                "the declared box has no area, so nothing inside it can draw — usually a \
                 width computed as `available - padding * 2` that went negative"
            }
            Problem::DroppedDegenerate { .. } => {
                "primitives were rejected for having a non-positive size, radius, or \
                 thickness; they left no geometry at all. Check for a subtraction that \
                 produced a negative extent"
            }
            Problem::TextOverflowsBox { .. } => {
                "the shaped text is bigger than its layout box, so it spills past where the \
                 layout expects it to end — widen `max_width`, shrink the font, or set \
                 `ellipsize`"
            }
            Problem::TextEllipsized { .. } => {
                "the label did not fit and was truncated with an ellipsis; intended if the \
                 content is user data, a layout bug if it is a fixed caption"
            }
            Problem::NearMissAlignment { .. } => {
                "this edge is a fraction off an edge its siblings share — almost always a \
                 stray offset or an inconsistent padding constant"
            }
            Problem::InvisibleAlpha { .. } => {
                "fill and border are both fully transparent, so this rect paints nothing — \
                 either a colour lookup returned the wrong key or an alpha was left at zero"
            }
            Problem::SiblingOverlap { .. } => {
                "two siblings cover the same pixels; fine for a background under its own \
                 content, a bug when both are meant to be readable"
            }
        }
    }
}

/// Which checks [`DebugReport`] runs, and how strict they are.
///
/// The defaults are chosen to be **quiet on correct UIs built with this crate's
/// own widgets**: checks that fire on legitimate patterns (a scroll view whose
/// content is taller than its viewport, a panel background under its own label)
/// are off, so that anything reported is worth looking at.
#[derive(Clone, Debug)]
pub struct LintConfig {
    /// Report nodes entirely outside the screen. Default on.
    pub off_screen: bool,
    /// Report nodes crossing a screen edge. Default **off** — toasts and
    /// tooltips animate in from off-screen, and the default text `max_width` of
    /// 800px makes labels hang off narrow viewports routinely.
    pub partially_off_screen: bool,
    /// Report nodes entirely removed by their clip. Default on. Never fires for
    /// content inside a
    /// [viewport](crate::DrawList::push_clip_viewport) — a scroll view hiding
    /// the rows either side of its window is working, not broken — so what is
    /// left is content that missed the hard boundary it was meant to sit in.
    pub fully_clipped: bool,
    /// Report nodes partly removed by their clip. Default **off** — a row
    /// half-scrolled off the top of a viewport is the normal state of every
    /// scroll view in the world, and viewport marking cannot excuse it the way
    /// it can for [`fully_clipped`](Self::fully_clipped) (the row is genuinely
    /// half-visible, so there is nothing to distinguish).
    pub partially_clipped: bool,
    /// Report scopes painting outside their declared rect. Default on.
    pub overflows_declared: bool,
    /// Report scopes that declared a rect and drew nothing. Default on.
    pub missing_paint: bool,
    /// Report primitives dropped for non-positive size. Default on.
    pub dropped_degenerate: bool,
    /// Report text whose ink exceeds its layout box. Default on. Never fires on
    /// an `ellipsize` block, where truncation is the intended behaviour.
    pub text_overflows_box: bool,
    /// Report ellipsized labels as info. Default on.
    pub text_ellipsized: bool,
    /// Report near-miss edge alignment. Default on, but hedged about with
    /// preconditions, because alignment is a claim about *intent* and the frame
    /// only records geometry. It is evaluated only between **named siblings
    /// inside a scope**, only for elements **stacked along the other axis** from
    /// the edge being compared, and at most **once per element per axis**. Each
    /// of those rules out a class of false positive the widget gallery produced:
    /// consensus manufactured between widgets 4000px apart under an inferred
    /// parent, the `left` edges of a row compared against each other, and one
    /// dropped element reported three times over.
    pub near_miss_alignment: bool,
    /// Report fully transparent chrome rects. Default on.
    pub invisible_alpha: bool,
    /// Report overlaps between layout peers outside a widget boundary. Default
    /// on: these are separate controls/content placed by their application
    /// parent, so sharing pixels is normally a layout defect.
    pub sibling_overlap: bool,
    /// Also report overlaps between a widget's own paint nodes. Default off:
    /// chrome legitimately sits behind labels and icons, but this is useful when
    /// asking for every symptom of a widget implementation defect. Enabled by
    /// [`strict`](Self::strict).
    pub sibling_overlap_inside_widgets: bool,
    /// Slack in pixels before a rect counts as escaping its container. Default
    /// `0.5` (sub-pixel rounding is not a bug).
    pub overflow_tolerance: f32,
    /// A misalignment is reported when it is larger than this and smaller than
    /// [`align_max`](Self::align_max). Default `0.5` — below this is beneath
    /// both f32 noise from composed transforms and human perception.
    pub align_min: f32,
    /// Upper bound on a reported misalignment. Default `8.0`; beyond it the two
    /// edges were probably never meant to line up.
    pub align_max: f32,
    /// How many siblings must agree on an edge before an outlier counts as
    /// misaligned. Default `3`.
    pub align_quorum: usize,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            off_screen: true,
            partially_off_screen: false,
            fully_clipped: true,
            partially_clipped: false,
            overflows_declared: true,
            missing_paint: true,
            dropped_degenerate: true,
            text_overflows_box: true,
            text_ellipsized: true,
            near_miss_alignment: true,
            invisible_alpha: true,
            sibling_overlap: true,
            sibling_overlap_inside_widgets: false,
            overflow_tolerance: 0.5,
            align_min: 0.5,
            align_max: 8.0,
            align_quorum: 3,
        }
    }
}

impl LintConfig {
    /// Every check on, including the noisy ones. Useful when hunting a specific
    /// bug rather than guarding against regressions.
    pub fn strict() -> Self {
        Self {
            partially_off_screen: true,
            partially_clipped: true,
            sibling_overlap: true,
            sibling_overlap_inside_widgets: true,
            ..Self::default()
        }
    }

    /// No checks — build the tree only.
    pub fn none() -> Self {
        Self {
            off_screen: false,
            partially_off_screen: false,
            fully_clipped: false,
            partially_clipped: false,
            overflows_declared: false,
            missing_paint: false,
            dropped_degenerate: false,
            text_overflows_box: false,
            text_ellipsized: false,
            near_miss_alignment: false,
            invisible_alpha: false,
            sibling_overlap: false,
            sibling_overlap_inside_widgets: false,
            ..Self::default()
        }
    }
}

/// A structured description of one rendered frame, plus everything that looks
/// wrong with it. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct DebugReport {
    /// The viewport the frame was laid out for.
    pub screen: Rect,
    /// Every node in paint-command submission order. Scopes are positioned by
    /// their earliest painted descendant; parents always precede children.
    pub nodes: Vec<DebugNode>,
    /// Everything the enabled lints found.
    pub problems: Vec<Problem>,
    /// Effective rects of the frame's viewport clips — the scroll views,
    /// dropdown lists and scrolled text fields that are *supposed* to hide
    /// content falling outside them. See
    /// [`DrawList::push_clip_viewport`](crate::DrawList::push_clip_viewport).
    pub viewports: Vec<Rect>,
    /// True when text nodes carry measured ink bounds. False for a report built
    /// with [`from_draw_list`](DebugReport::from_draw_list), where text nodes
    /// fall back to their layout box and the text lints do not run.
    pub text_measured: bool,
    /// Renderer telemetry attached after rendering.
    pub render_stats: Option<RenderStats>,
}

impl DebugReport {
    /// Build a report from a finished [`DrawList`] **without measuring text**.
    ///
    /// Cheap and borrow-free: text nodes get their layout box
    /// (`x`, `y`, `max_width`, `line_height`) rather than shaped ink, and the
    /// text lints are skipped. Use [`measured`](Self::measured) when you can
    /// borrow the list mutably — every text-measurement path in the crate
    /// mutates a cache, so exact text bounds require `&mut`.
    pub fn from_draw_list(list: &DrawList, screen: Rect) -> Self {
        Self::build(list, screen, None, &LintConfig::default(), false)
    }

    /// Build a report with exact, shaped text bounds. Preferred when possible.
    pub fn measured(list: &mut DrawList, screen: Rect) -> Self {
        let ink = measure_texts(list);
        Self::build(list, screen, Some(&ink), &LintConfig::default(), true)
    }

    /// Build a report over a whole [`LayerStack`] — the base list plus every
    /// overlay layer, each nested under a node named for its
    /// [`LayerKind`](crate::LayerKind).
    pub fn from_layers(layers: &LayerStack, screen: Rect) -> Self {
        Self::build_layers(layers, screen, false, &LintConfig::default())
    }

    /// [`from_layers`](Self::from_layers) with exact text bounds.
    pub fn measured_layers(layers: &mut LayerStack, screen: Rect) -> Self {
        Self::build_layers_measured(layers, screen, &LintConfig::default())
    }

    /// Re-run the lints with a different configuration, keeping the tree.
    pub fn with_lints(mut self, cfg: &LintConfig) -> Self {
        self.problems = lint(&self.nodes, self.screen, cfg, self.text_measured);
        self
    }

    /// Attach renderer telemetry collected for this frame.
    pub fn with_render_stats(mut self, stats: RenderStats) -> Self {
        self.render_stats = Some(stats);
        self
    }

    /// Everything the lints found.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// Problems at or above `min` severity.
    pub fn problems_at_least(&self, min: Severity) -> impl Iterator<Item = &Problem> {
        self.problems.iter().filter(move |p| p.severity() >= min)
    }

    /// Look a node up by name (first match, in tree order).
    pub fn node(&self, name: &str) -> Option<&DebugNode> {
        self.nodes.iter().find(|n| n.name == name)
    }

    /// How many nodes were named by inference rather than by a debug scope.
    pub fn inferred_count(&self) -> usize {
        self.nodes.iter().filter(|n| !n.named).count()
    }

    /// Panic with the full text report unless there are no [`Severity::Error`]
    /// problems. The intended way to guard a layout in a test.
    ///
    /// The floor is `Error` rather than `Warning` deliberately: errors are
    /// unambiguous defects (drawn off-screen, clipped away by a hard boundary,
    /// reserved a box and drew nothing, a size that collapsed to nothing),
    /// whereas warnings cover the merely suspicious — crossing a screen edge,
    /// overlapping a sibling, an edge half a pixel off its neighbours — which
    /// are sometimes intentional. Warnings still appear in
    /// [`to_text`](Self::to_text); use
    /// [`assert_clean_at`](Self::assert_clean_at) to tighten the floor.
    pub fn assert_clean(&self) {
        self.assert_clean_at(Severity::Error);
    }

    /// Write `frame.txt` and `frame.json` into `dir`, creating it if needed.
    ///
    /// The intended way to get a report out of a *running* application: call it
    /// from wherever your own debug flag lives, then read the files. It is a
    /// plain method rather than anything environment-driven on purpose — this
    /// crate keeps state caller-owned, and an explicit call is testable and
    /// greppable in a way that an ambient env var is not.
    ///
    /// ```ignore
    /// if self.dump_ui_next_frame {
    ///     DebugReport::measured_layers(&mut layers, screen).write_to_dir("target/ui-debug")?;
    ///     self.dump_ui_next_frame = false;
    /// }
    /// ```
    pub fn write_to_dir(&self, dir: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("frame.txt"), self.to_text())?;
        std::fs::write(dir.join("frame.json"), self.to_json())?;
        Ok(())
    }

    /// [`assert_clean`](Self::assert_clean) with an explicit severity floor.
    pub fn assert_clean_at(&self, min: Severity) {
        let hits: Vec<&Problem> = self.problems_at_least(min).collect();
        if hits.is_empty() {
            return;
        }
        panic!(
            "{} layout problem(s) at or above {}:\n\n{}",
            hits.len(),
            min.label(),
            self.to_text()
        );
    }
}

/// What a shaping pass tells us about one text block.
///
/// The two halves answer different questions and are not interchangeable:
/// `advance` is the slot the text *reserves* (advance width by line-box height),
/// which is what neighbouring layout must not tread on; `ink_band` is what it
/// *paints*, which is what may be found outside a box.
#[derive(Clone, Copy)]
struct TextExtent {
    /// `(advance width, line-box height)` — [`DrawList::measure_block`].
    advance: (f32, f32),
    /// `(top, bottom)` glyph-ink offsets below the block's top edge, or `None`
    /// when nothing inked (empty or whitespace-only).
    ink_band: Option<(f32, f32)>,
}

/// Measure every queued text block: reserved slot and painted ink both.
fn measure_texts(list: &mut DrawList) -> Vec<TextExtent> {
    let blocks: Vec<TextBlock> = list.texts.clone();
    blocks
        .iter()
        .map(|b| TextExtent {
            advance: list.measure_block(b),
            ink_band: list.measure_block_ink(b),
        })
        .collect()
}

/// Resolve the rect a text block actually paints into, from its measurement and
/// alignment.
///
/// Vertically this is the **ink band**, not the line box. A line box carries
/// leading above the ascent and below the descent, and `vcentered_text_y`
/// deliberately slides the whole box upward so the glyphs' optical centre lands
/// on the row centre — so a correctly centred label's line box always pokes out
/// of its row. Reporting that as painted area turns every centred label in the
/// frame into an overflow. When nothing inked there is no band to use and the
/// line box is the only answer available.
///
/// Horizontally this stays the advance box. Advance is what the next glyph (or
/// the next widget) has to clear, and for upright text it contains the ink, so
/// it is the conservative choice for "how much room did this take".
fn text_ink_rect(block: &TextBlock, measured: TextExtent) -> Rect {
    let (w, h) = measured.advance;
    // `Start`/`End` are direction-relative; for the overwhelmingly common LTR
    // case they coincide with Left/Right. RTL blocks mirror, which we do not try
    // to model — the width is right either way, only the x may be off.
    let x = match block.align {
        TextAlign::Center => block.x + (block.max_width - w) * 0.5,
        TextAlign::End | TextAlign::Right => block.x + (block.max_width - w),
        TextAlign::Start | TextAlign::Left => block.x,
    };
    match measured.ink_band {
        Some((top, bottom)) => Rect::new(x, block.y + top, w, bottom - top),
        None => Rect::new(x, block.y, w, h),
    }
}

/// Effects that paint outside the measured ink box.
fn text_effects(block: &TextBlock) -> Vec<&'static str> {
    let mut v = Vec::new();
    if block.outline.is_some() {
        v.push("outline");
    }
    if block.shadow.is_some() {
        v.push("shadow");
    }
    if block.glow.is_some() {
        v.push("glow");
    }
    v
}

/// Bounds of a clip rect encoded in the shader's `[x, y, w, h]` + enable flag.
fn clip_from_parts(clip: [f32; 4], enabled: f32) -> Option<Rect> {
    if enabled > 0.5 {
        Some(Rect::new(clip[0], clip[1], clip[2], clip[3]))
    } else {
        None
    }
}

/// Axis-aligned bounding box of four transformed corners.
fn corners_bounds(corners: &[[f32; 2]; 4]) -> Rect {
    let xs = corners.iter().map(|c| c[0]);
    let ys = corners.iter().map(|c| c[1]);
    let min_x = xs.clone().fold(f32::INFINITY, f32::min);
    let max_x = xs.fold(f32::NEG_INFINITY, f32::max);
    let min_y = ys.clone().fold(f32::INFINITY, f32::min);
    let max_y = ys.fold(f32::NEG_INFINITY, f32::max);
    Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// True when four corners form an axis-aligned rectangle (no rotation/shear).
fn corners_axis_aligned(corners: &[[f32; 2]; 4]) -> bool {
    const EPS: f32 = 1e-3;
    // TL/TR share a y, BR/BL share a y, TL/BL share an x, TR/BR share an x.
    (corners[0][1] - corners[1][1]).abs() < EPS
        && (corners[2][1] - corners[3][1]).abs() < EPS
        && (corners[0][0] - corners[3][0]).abs() < EPS
        && (corners[1][0] - corners[2][0]).abs() < EPS
}

// ---------------------------------------------------------------------------
// Tree construction
// ---------------------------------------------------------------------------

/// A node under construction, before ids are assigned in tree order.
struct RawNode {
    parent: Option<usize>,
    name: String,
    named: bool,
    kind: NodeKind,
    pass: RenderPass,
    bounds: Rect,
    declared: Option<Rect>,
    text_box: Option<Rect>,
    clip: Option<Rect>,
    text: Option<String>,
    counts: PrimCounts,
    axis_aligned: bool,
    effects: Vec<&'static str>,
    ellipsize: bool,
    layer: Option<usize>,
    /// Sort key within a parent: list/layer, paint command, then item in run.
    order: (usize, usize, usize),
}

impl RawNode {
    fn new(name: String, named: bool, kind: NodeKind, pass: RenderPass, index: usize) -> Self {
        Self {
            parent: None,
            name,
            named,
            kind,
            pass,
            bounds: Rect::zero(),
            declared: None,
            text_box: None,
            clip: None,
            text: None,
            counts: PrimCounts::default(),
            axis_aligned: true,
            effects: Vec::new(),
            ellipsize: false,
            layer: None,
            order: (0, usize::MAX, index),
        }
    }
}

/// Truncate a label so one long paragraph cannot dominate the report.
fn short_label(s: &str) -> String {
    const MAX: usize = 48;
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= MAX {
        flat
    } else {
        let head: String = flat.chars().take(MAX - 1).collect();
        format!("{head}…")
    }
}

impl DebugReport {
    fn build(
        list: &DrawList,
        screen: Rect,
        ink: Option<&[TextExtent]>,
        cfg: &LintConfig,
        text_measured: bool,
    ) -> Self {
        let mut raw = Vec::new();
        collect_nodes(list, ink, None, None, &mut raw);
        let mut nodes = finalize(raw);
        let viewports = list.viewport_clips().to_vec();
        mark_off_viewport(&mut nodes, &viewports);
        let problems = lint(&nodes, screen, cfg, text_measured);
        Self {
            screen,
            nodes,
            problems,
            viewports,
            text_measured,
            render_stats: None,
        }
    }

    fn build_layers(
        layers: &LayerStack,
        screen: Rect,
        text_measured: bool,
        cfg: &LintConfig,
    ) -> Self {
        let mut raw = Vec::new();
        collect_nodes(layers.base(), None, None, None, &mut raw);
        for (i, layer) in layers.layers().iter().enumerate() {
            let root = raw.len();
            let mut node = RawNode::new(
                format!("{:?}#{i}", layer.kind),
                true,
                NodeKind::Layer,
                RenderPass::NineSlice,
                i,
            );
            node.declared = Some(layer.rect);
            node.layer = Some(i);
            raw.push(node);
            collect_nodes(&layer.list, None, Some(root), Some(i), &mut raw);
            let bounds = child_union(&raw, root, layer.list.viewport_clips());
            raw[root].bounds = bounds;
        }
        let mut nodes = finalize(raw);
        let viewports = layer_viewports(layers);
        mark_off_viewport(&mut nodes, &viewports);
        let problems = lint(&nodes, screen, cfg, text_measured);
        Self {
            screen,
            nodes,
            problems,
            viewports,
            text_measured,
            render_stats: None,
        }
    }

    fn build_layers_measured(layers: &mut LayerStack, screen: Rect, cfg: &LintConfig) -> Self {
        let base_ink = measure_texts(layers.base_mut());
        let layer_ink: Vec<Vec<TextExtent>> = (0..layers.layers().len())
            .map(|i| measure_texts(&mut layers.layers_mut()[i].list))
            .collect();

        let mut raw = Vec::new();
        collect_nodes(layers.base(), Some(&base_ink), None, None, &mut raw);
        for (i, layer) in layers.layers().iter().enumerate() {
            let root = raw.len();
            let mut node = RawNode::new(
                format!("{:?}#{i}", layer.kind),
                true,
                NodeKind::Layer,
                RenderPass::NineSlice,
                i,
            );
            node.declared = Some(layer.rect);
            node.layer = Some(i);
            raw.push(node);
            collect_nodes(
                &layer.list,
                Some(&layer_ink[i]),
                Some(root),
                Some(i),
                &mut raw,
            );
            let bounds = child_union(&raw, root, layer.list.viewport_clips());
            raw[root].bounds = bounds;
        }
        let mut nodes = finalize(raw);
        let viewports = layer_viewports(layers);
        mark_off_viewport(&mut nodes, &viewports);
        let problems = lint(&nodes, screen, cfg, true);
        Self {
            screen,
            nodes,
            problems,
            viewports,
            text_measured: true,
            render_stats: None,
        }
    }
}

/// Every viewport clip in a layer stack: the base list's plus each overlay's.
fn layer_viewports(layers: &LayerStack) -> Vec<Rect> {
    let mut v = layers.base().viewport_clips().to_vec();
    for layer in layers.layers() {
        v.extend_from_slice(layer.list.viewport_clips());
    }
    v
}

/// Tag every node a *viewport* clip removed entirely.
///
/// A clip that erases an element is a layout bug when the clip is a hard
/// boundary (a window, a panel) and the entire point when it is a viewport — a
/// scroll view hides the rows either side of its window on every frame it is
/// working correctly. Only the code that pushed the clip knows which it meant,
/// which is why [`DrawList::push_clip_viewport`](crate::DrawList::push_clip_viewport)
/// exists. The tag both explains the absence when reading the tree and tells
/// the clip lint to stay quiet.
fn mark_off_viewport(nodes: &mut [DebugNode], viewports: &[Rect]) {
    if viewports.is_empty() {
        return;
    }
    // A node's clip is the whole clip stack intersected, and nesting only ever
    // shrinks it — so anything drawn inside a viewport carries a clip contained
    // by that viewport, however deeply nested. Plain containment therefore
    // covers the whole subtree. Written out rather than using
    // `Rect::contains_rect` because that treats an *empty* rect as contained by
    // anything, and a clip collapsed to zero size (content entirely outside its
    // parent clip) is precisely a case we must not excuse on position alone.
    let within = |outer: &Rect, inner: Rect| {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.right() <= outer.right()
            && inner.bottom() <= outer.bottom()
    };
    for node in nodes.iter_mut() {
        let Some(clip) = node.clip else { continue };
        if node.bounds.intersection(clip).is_some() {
            continue;
        }
        if viewports.iter().any(|v| within(v, clip)) {
            node.effects.push(OFF_VIEWPORT);
        }
    }
}

/// Marker pushed onto [`DebugNode::effects`] by [`mark_off_viewport`].
pub(crate) const OFF_VIEWPORT: &str = "off-viewport";

/// Union of every descendant's bounds, with anything hidden by a **viewport**
/// contributing only its visible part.
///
/// A scroll view's content is taller than its viewport by design, and a scope's
/// bounds are derived from its children — so unioning raw bounds would make
/// every scrolling container appear to overflow the rect it declared, which is
/// exactly what [`DrawList::push_clip_viewport`](crate::DrawList::push_clip_viewport)
/// exists to say is fine. Content under a *hard* clip still contributes in full:
/// a widget that missed its container is a bug we want reported, and the clip
/// erasing it must not hide that.
fn child_union(raw: &[RawNode], root: usize, viewports: &[Rect]) -> Rect {
    let mut acc = Rect::zero();
    for (i, n) in raw.iter().enumerate() {
        if i == root || !descends_from(raw, i, root) {
            continue;
        }
        acc = acc.union(visible_within_viewport(n.bounds, n.clip, viewports));
    }
    acc
}

/// `bounds` reduced to what a viewport clip leaves visible, or `bounds`
/// unchanged when the clip is a hard boundary (or there is none).
fn visible_within_viewport(bounds: Rect, clip: Option<Rect>, viewports: &[Rect]) -> Rect {
    let Some(clip) = clip else { return bounds };
    if viewports.is_empty() {
        return bounds;
    }
    // Same containment test as `mark_off_viewport`, and for the same reason:
    // nesting only shrinks a clip, so anything drawn inside a viewport carries a
    // clip contained by it however deeply nested.
    let within = |outer: &Rect| {
        clip.x >= outer.x
            && clip.y >= outer.y
            && clip.right() <= outer.right()
            && clip.bottom() <= outer.bottom()
    };
    if !viewports.iter().any(within) {
        return bounds;
    }
    bounds.intersection(clip).unwrap_or(Rect::zero())
}

fn descends_from(raw: &[RawNode], mut node: usize, ancestor: usize) -> bool {
    let mut guard = 0;
    while let Some(p) = raw[node].parent {
        if p == ancestor {
            return true;
        }
        node = p;
        guard += 1;
        if guard > raw.len() {
            return false;
        }
    }
    false
}

/// Turn one `DrawList` into raw nodes, appended to `out`.
fn collect_nodes(
    list: &DrawList,
    ink: Option<&[TextExtent]>,
    root: Option<usize>,
    layer: Option<usize>,
    out: &mut Vec<RawNode>,
) {
    let base = out.len();
    let final_counts = list_counts(list);
    let scopes = list.debug_scopes();

    // ---- 1. One node per debug scope, in push order (parents first). ----
    for scope in scopes {
        let mut node = RawNode::new(
            scope.name.clone(),
            true,
            NodeKind::Scope,
            RenderPass::NineSlice,
            0,
        );
        node.parent = Some(
            scope
                .parent
                .map_or(root.unwrap_or(usize::MAX), |p| base + p),
        );
        if node.parent == Some(usize::MAX) {
            node.parent = None;
        }
        node.declared = scope.declared;
        node.clip = scope.clip;
        node.layer = layer;
        let end = if scope.closed {
            scope.end
        } else {
            final_counts
        };
        node.counts = end.since(scope.start);
        out.push(node);
    }

    /// Innermost scope covering element `i` of one buffer.
    fn owner(
        scopes: &[crate::widgets::DebugScope],
        final_counts: PrimCounts,
        base: usize,
        i: usize,
        pick: impl Fn(&PrimCounts) -> usize,
    ) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None; // (depth, node)
        for (si, s) in scopes.iter().enumerate() {
            let end = if s.closed { s.end } else { final_counts };
            if pick(&s.start) <= i && i < pick(&end) {
                let better = best.is_none_or(|(d, _)| s.depth >= d);
                if better {
                    best = Some((s.depth, base + si));
                }
            }
        }
        best.map(|(_, n)| n)
    }

    let scope_base = base;
    let own = |i: usize, pick: fn(&PrimCounts) -> usize| -> Option<usize> {
        owner(scopes, final_counts, scope_base, i, pick).or(root)
    };

    // Map each payload to the command which submits it. Public payload arrays
    // can still be filled directly; those entries retain the renderer's legacy
    // family order after the explicit stream.
    let stream_len = list.paint_commands().len();
    let mut nine_order = vec![(stream_len, 0); list.nine_slices.len()];
    let mut chrome_order = vec![(stream_len + 1, 0); list.chrome_instances.len()];
    let mut circle_order = vec![(stream_len + 1, 0); list.circle_instances.len()];
    let mut icon_order = vec![(stream_len + 2, 0); list.icons.len()];
    #[cfg(feature = "phosphor-icons")]
    let mut msdf_order = vec![(stream_len + 3, 0); list.icons_msdf.len()];
    let mut text_order = vec![(stream_len + 4, 0); list.texts.len()];
    let mut soup_runs = Vec::new();
    for (command, cmd) in list.paint_commands().iter().enumerate() {
        let map = |range: &std::ops::Range<u32>, orders: &mut [(usize, usize)]| {
            for (within, i) in (range.start as usize..range.end as usize).enumerate() {
                if let Some(order) = orders.get_mut(i) {
                    *order = (command, within);
                }
            }
        };
        match cmd {
            PaintCmd::Soup { indices } => soup_runs.push((command, indices.clone())),
            PaintCmd::Chrome { instances } => map(instances, &mut chrome_order),
            PaintCmd::Circle { instances } => map(instances, &mut circle_order),
            PaintCmd::NineSlice { draws } => map(draws, &mut nine_order),
            PaintCmd::Icon { draws } => map(draws, &mut icon_order),
            #[cfg(feature = "phosphor-icons")]
            PaintCmd::IconMsdf { draws } => map(draws, &mut msdf_order),
            PaintCmd::Text { draws } => map(draws, &mut text_order),
        }
    }
    let trailing = list.trailing_soup_range();
    if !trailing.is_empty() {
        soup_runs.push((stream_len, trailing));
    }
    let list_order = layer.map_or(0, |i| i + 1);

    // ---- 2. Leaf nodes, in paint-command submission order. ----

    for (i, ns) in list.nine_slices.iter().enumerate() {
        let name = if ns.texture_key.is_empty() {
            format!("nine_slice#{i}")
        } else {
            ns.texture_key.clone()
        };
        let mut n = RawNode::new(name, false, NodeKind::NineSlice, RenderPass::NineSlice, i);
        n.parent = own(i, |c| c.nine_slices);
        n.bounds = ns.transform.transform_rect_aabb(ns.local);
        n.axis_aligned = ns.transform.is_translate_only();
        n.clip = ns.clip;
        n.counts.nine_slices = 1;
        n.layer = layer;
        n.order = (list_order, nine_order[i].0, nine_order[i].1);
        out.push(n);
    }

    for (i, c) in list.chrome_instances.iter().enumerate() {
        let mut n = RawNode::new(
            format!("chrome#{i}"),
            false,
            NodeKind::Chrome,
            RenderPass::Color,
            i,
        );
        n.parent = own(i, |c| c.chrome_instances);
        n.bounds = Rect::new(c.rect[0], c.rect[1], c.rect[2], c.rect[3]);
        n.clip = clip_from_parts(c.clip, c.params[2]);
        n.counts.chrome_instances = 1;
        n.layer = layer;
        // Fully transparent fill *and* border paints nothing at all. The
        // gradient partner `bg2` counts as paint: a falloff band (e.g. a drop
        // shadow's top skirt) is transparent at one edge by design.
        if c.bg[3] <= 0.0 && c.bg2[3] <= 0.0 && c.border[3] <= 0.0 {
            n.effects.push("invisible");
        }
        n.order = (list_order, chrome_order[i].0, chrome_order[i].1);
        out.push(n);
    }

    for (i, c) in list.circle_instances.iter().enumerate() {
        let (cx, cy, r) = (c.center[0], c.center[1], c.center[2]);
        let mut n = RawNode::new(
            format!("circle#{i}"),
            false,
            NodeKind::Circle,
            RenderPass::Color,
            i,
        );
        n.parent = own(i, |c| c.circle_instances);
        n.bounds = Rect::new(cx - r, cy - r, r * 2.0, r * 2.0);
        n.clip = clip_from_parts(c.clip, c.params[0]);
        n.counts.circle_instances = 1;
        n.layer = layer;
        n.order = (list_order, circle_order[i].0, circle_order[i].1);
        out.push(n);
    }

    // Soup vertices are individually meaningless. Aggregate each command run
    // separately (and then by owner), so an intervening command remains visible
    // in tree order instead of being swallowed by one cross-run geometry node.
    let mut geometry_index = 0;
    for (command, indices) in soup_runs {
        use std::collections::BTreeMap;
        let mut groups: BTreeMap<Option<usize>, (Rect, usize, Option<Rect>)> = BTreeMap::new();
        let mut vertex_ids = std::collections::BTreeSet::new();
        for &index in &list.indices[indices.start as usize..indices.end as usize] {
            vertex_ids.insert(index as usize);
        }
        for i in vertex_ids {
            let Some(v) = list.vertices.get(i) else {
                continue;
            };
            let parent = own(i, |c| c.vertices);
            let entry = groups.entry(parent).or_insert((
                Rect::zero(),
                0,
                clip_from_parts(v.clip, v.clip_enabled),
            ));
            let p = Rect::new(v.position[0], v.position[1], 0.0, 0.0);
            entry.0 = if entry.1 == 0 {
                p
            } else {
                let x0 = entry.0.x.min(p.x);
                let y0 = entry.0.y.min(p.y);
                let x1 = entry.0.right().max(p.x);
                let y1 = entry.0.bottom().max(p.y);
                Rect::new(x0, y0, x1 - x0, y1 - y0)
            };
            entry.1 += 1;
        }
        for (within, (parent, (bounds, count, clip))) in groups.into_iter().enumerate() {
            let mut n = RawNode::new(
                format!("geometry#{geometry_index}"),
                false,
                NodeKind::Geometry,
                RenderPass::Color,
                geometry_index,
            );
            geometry_index += 1;
            n.parent = parent;
            n.bounds = bounds;
            n.clip = clip;
            n.counts.vertices = count;
            n.layer = layer;
            n.order = (list_order, command, within);
            out.push(n);
        }
    }

    for (i, icon) in list.icons.iter().enumerate() {
        let name = if icon.icon_key.is_empty() {
            format!("icon#{i}")
        } else {
            icon.icon_key.clone()
        };
        let mut n = RawNode::new(name, false, NodeKind::Icon, RenderPass::Icon, i);
        n.parent = own(i, |c| c.icons);
        n.bounds = corners_bounds(&icon.corners);
        n.axis_aligned = corners_axis_aligned(&icon.corners);
        n.clip = icon.clip;
        n.counts.icons = 1;
        n.layer = layer;
        n.order = (list_order, icon_order[i].0, icon_order[i].1);
        out.push(n);
    }

    #[cfg(feature = "phosphor-icons")]
    for (i, icon) in list.icons_msdf.iter().enumerate() {
        let mut n = RawNode::new(
            format!("icon_msdf#{i}"),
            false,
            NodeKind::IconMsdf,
            RenderPass::IconMsdf,
            i,
        );
        n.parent = own(i, |c| c.icons_msdf);
        n.bounds = icon.transform.transform_rect_aabb(icon.local);
        n.axis_aligned = icon.transform.is_translate_only();
        n.clip = icon.clip;
        n.counts.icons_msdf = 1;
        n.layer = layer;
        n.order = (list_order, msdf_order[i].0, msdf_order[i].1);
        out.push(n);
    }

    for (i, block) in list.texts.iter().enumerate() {
        let measured = ink.and_then(|m| m.get(i).copied());
        let text_box = Rect::new(
            block.x,
            block.y,
            block.max_width,
            measured.map_or(block.line_height, |m| m.advance.1),
        );
        let bounds = match measured {
            Some(m) => text_ink_rect(block, m),
            None => text_box,
        };
        let name = if block.content.trim().is_empty() {
            format!("text#{i}")
        } else {
            format!("\"{}\"", short_label(&block.content))
        };
        let mut n = RawNode::new(name, false, NodeKind::Text, RenderPass::Text, i);
        n.parent = own(i, |c| c.texts);
        n.bounds = bounds;
        n.text_box = Some(text_box);
        n.clip = block.clip;
        n.text = Some(block.content.clone());
        n.counts.texts = 1;
        n.effects = text_effects(block);
        n.ellipsize = block.ellipsize;
        n.layer = layer;
        n.order = (list_order, text_order[i].0, text_order[i].1);
        out.push(n);
    }

    // ---- 3. Scope bounds = union of everything inside them. ----
    for si in 0..scopes.len() {
        let node = scope_base + si;
        let bounds = child_union(out, node, list.viewport_clips());
        out[node].bounds = bounds;
    }

    // ---- 4. Nest orphans by geometric containment. ----
    nest_by_containment(out, base, root);

    // A scope occupies the paint position of its earliest descendant while
    // retaining ownership, bounds, and parent-before-child tree structure.
    for si in (0..scopes.len()).rev() {
        let node = scope_base + si;
        if let Some(order) = out
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != node && descends_from(out, *i, node))
            .map(|(_, n)| n.order)
            .min()
        {
            out[node].order = order;
        }
    }

    // ---- 5. Attribute list-wide degenerate drops not owned by any scope. ----
    let scoped_drops: usize = scopes
        .iter()
        .filter(|s| s.parent.is_none())
        .map(|s| {
            let end = if s.closed { s.end } else { final_counts };
            end.since(s.start).dropped_degenerate
        })
        .sum();
    let loose = final_counts.dropped_degenerate.saturating_sub(scoped_drops);
    if loose > 0 {
        match root {
            Some(r) => out[r].counts.dropped_degenerate += loose,
            None => {
                // Recorded on the first top-level node so it is never lost; the
                // root-level lint reads it back off the report totals.
                let mut n = RawNode::new(
                    "<dropped>".to_string(),
                    true,
                    NodeKind::Scope,
                    RenderPass::NineSlice,
                    0,
                );
                n.counts.dropped_degenerate = loose;
                n.layer = layer;
                out.push(n);
            }
        }
    }
}

fn list_counts(list: &DrawList) -> PrimCounts {
    PrimCounts {
        vertices: list.vertices.len(),
        indices: list.indices.len(),
        texts: list.texts.len(),
        icons: list.icons.len(),
        nine_slices: list.nine_slices.len(),
        #[cfg(feature = "phosphor-icons")]
        icons_msdf: list.icons_msdf.len(),
        chrome_instances: list.chrome_instances.len(),
        circle_instances: list.circle_instances.len(),
        dropped_degenerate: list.dropped_degenerate() as usize,
    }
}

/// Give parentless nodes a home: the smallest node that strictly contains them.
///
/// This is the "best effort" structure for draws made outside any debug scope —
/// a panel background ends up as the parent of the label sitting on it, which is
/// usually what the author meant even though they never said so.
///
/// Two node kinds are barred from adopting. **Text** because a label is a leaf.
/// **Geometry** because it is the one node whose bounds do not describe a shape:
/// loose triangles are aggregated per scope, so its rect is a bounding box drawn
/// around unrelated pieces and covers mostly empty space. In the widget gallery
/// that single aggregate spanned the whole 4482px page and adopted 309 nodes,
/// burying the real structure and making every widget a sibling of every other.
fn nest_by_containment(out: &mut [RawNode], base: usize, root: Option<usize>) {
    let candidates: Vec<(usize, Rect, f32)> = (base..out.len())
        .filter(|&i| !out[i].bounds.is_empty())
        .map(|i| {
            let b = out[i].bounds;
            (i, b, b.width * b.height)
        })
        .collect();

    for i in base..out.len() {
        if out[i].parent != root || out[i].bounds.is_empty() {
            continue;
        }
        let me = out[i].bounds;
        let my_area = me.width * me.height;
        let mut best: Option<(usize, f32)> = None;
        for &(j, rect, area) in &candidates {
            if j == i || out[j].kind == NodeKind::Text || out[j].kind == NodeKind::Geometry {
                continue;
            }
            // Strictly larger, and containing — ties would make a cycle.
            if area <= my_area || !rect.contains_rect(me, 0.0) {
                continue;
            }
            if best.is_none_or(|(_, a)| area < a) {
                best = Some((j, area));
            }
        }
        if let Some((parent, _)) = best {
            // Only adopt when the candidate itself is not a descendant of us.
            if !descends_from(out, parent, i) {
                out[i].parent = Some(parent);
            }
        }
    }
}

/// Assign final ids in depth-first tree order so parents always precede children.
fn finalize(raw: Vec<RawNode>) -> Vec<DebugNode> {
    let n = raw.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    for (i, node) in raw.iter().enumerate() {
        match node.parent {
            Some(p) if p < n && p != i => children[p].push(i),
            _ => roots.push(i),
        }
    }
    let sort_key = |raw: &Vec<RawNode>, a: &usize, b: &usize| raw[*a].order.cmp(&raw[*b].order);
    roots.sort_by(|a, b| sort_key(&raw, a, b));
    for list in children.iter_mut() {
        list.sort_by(|a, b| sort_key(&raw, a, b));
    }

    let mut order = Vec::with_capacity(n);
    let mut depths = vec![0usize; n];
    let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&r| (r, 0)).collect();
    let mut seen = vec![false; n];
    while let Some((node, depth)) = stack.pop() {
        if seen[node] {
            continue;
        }
        seen[node] = true;
        depths[node] = depth;
        order.push(node);
        for &c in children[node].iter().rev() {
            stack.push((c, depth + 1));
        }
    }
    // Any node left unvisited (a cycle we refused to follow) is emitted flat.
    for (i, &visited) in seen.iter().enumerate().take(n) {
        if !visited {
            order.push(i);
        }
    }

    let mut new_id = vec![0usize; n];
    for (new, &old) in order.iter().enumerate() {
        new_id[old] = new;
    }

    order
        .iter()
        .enumerate()
        .map(|(new, &old)| {
            let r = &raw[old];
            DebugNode {
                id: new,
                parent: r.parent.filter(|&p| p < n && p != old).map(|p| new_id[p]),
                depth: depths[old],
                name: r.name.clone(),
                named: r.named,
                kind: r.kind,
                pass: r.pass,
                bounds: r.bounds,
                declared: r.declared,
                text_box: r.text_box,
                clip: r.clip,
                text: r.text.clone(),
                counts: r.counts,
                axis_aligned: r.axis_aligned,
                effects: r.effects.clone(),
                ellipsize: r.ellipsize,
                layer: r.layer,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Lints
// ---------------------------------------------------------------------------

/// Run the enabled checks over a finished node tree.
fn lint(nodes: &[DebugNode], screen: Rect, cfg: &LintConfig, text_measured: bool) -> Vec<Problem> {
    let mut out = Vec::new();
    let tol = cfg.overflow_tolerance;

    for node in nodes {
        let bounds = node.bounds;
        let has_area = !bounds.is_empty();

        // Bounds under rotation/shear are an inflated AABB of a shape that was
        // never really that big, so geometry checks would report a box that does
        // not exist. Skip them rather than lie.
        let geometric = node.axis_aligned;

        if cfg.dropped_degenerate && node.counts.dropped_degenerate > 0 {
            out.push(Problem::DroppedDegenerate {
                node: node.id,
                count: node.counts.dropped_degenerate,
            });
        }

        if let Some(declared) = node.declared {
            // Overlay layers are pushed speculatively every frame and are empty
            // whenever nothing is open, so an empty one is not a defect.
            // (Layer nodes are likewise exempt from `overflows_declared`
            // below: a layer's rect is its input-blocking/hit-test bounds,
            // not a paint contract — popovers and tooltips legitimately cast
            // soft shadows past it.)
            if cfg.missing_paint
                && node.kind != NodeKind::Layer
                && !declared.is_empty()
                && node.counts.total() == 0
            {
                out.push(Problem::MissingPaint {
                    node: node.id,
                    declared,
                });
            }
            if declared.is_empty() {
                out.push(Problem::DegenerateDeclaredRect {
                    node: node.id,
                    declared,
                });
            } else if cfg.overflows_declared
                && node.kind != NodeKind::Layer
                && geometric
                && has_area
                && !declared.contains_rect(bounds, tol)
            {
                let overshoot = overshoot_of(bounds, declared);
                out.push(Problem::OverflowsDeclared {
                    node: node.id,
                    painted: bounds,
                    declared,
                    overshoot,
                });
            }
        }

        if cfg.invisible_alpha && node.effects.contains(&"invisible") {
            out.push(Problem::InvisibleAlpha {
                node: node.id,
                bounds,
            });
        }

        if geometric && has_area {
            // Screen containment. A node fully outside is invisible; one that
            // merely crosses an edge is suspicious but often intentional.
            if bounds.intersection(screen).is_none() {
                if cfg.off_screen {
                    out.push(Problem::OffScreen {
                        node: node.id,
                        bounds,
                        screen,
                    });
                }
            } else if cfg.partially_off_screen && !screen.contains_rect(bounds, tol) {
                out.push(Problem::PartiallyOffScreen {
                    node: node.id,
                    bounds,
                    screen,
                });
            }

            if let Some(clip) = node.clip {
                if bounds.intersection(clip).is_none() {
                    // Scrolled out of a viewport is the design, not a defect —
                    // see `mark_off_viewport`.
                    if cfg.fully_clipped && !node.effects.contains(&OFF_VIEWPORT) {
                        out.push(Problem::FullyClipped {
                            node: node.id,
                            bounds,
                            clip,
                        });
                    }
                } else if cfg.partially_clipped && !clip.contains_rect(bounds, tol) {
                    out.push(Problem::PartiallyClipped {
                        node: node.id,
                        bounds,
                        clip,
                    });
                }
            }
        }

        // Text checks need real shaped extents; a report built without
        // measuring has only the layout box, where these would be meaningless.
        if text_measured
            && node.kind == NodeKind::Text
            && let Some(text_box) = node.text_box
        {
            if node.ellipsize {
                if cfg.text_ellipsized && bounds.width > text_box.width + tol {
                    out.push(Problem::TextEllipsized {
                        node: node.id,
                        wanted: bounds.width,
                        available: text_box.width,
                    });
                }
            } else if cfg.text_overflows_box
                && (bounds.width > text_box.width + tol || bounds.height > text_box.height + tol)
            {
                out.push(Problem::TextOverflowsBox {
                    node: node.id,
                    ink: bounds,
                    text_box,
                });
            }
        }
    }

    if cfg.near_miss_alignment {
        check_alignment(nodes, cfg, &mut out);
    }
    if cfg.sibling_overlap {
        check_overlap(nodes, cfg.sibling_overlap_inside_widgets, &mut out);
    }

    out.sort_by(|a, b| {
        b.severity()
            .cmp(&a.severity())
            .then_with(|| a.node().cmp(&b.node()))
            .then_with(|| a.code().cmp(b.code()))
    });
    out
}

/// Worst distance `inner` pokes out past `outer` on any side.
fn overshoot_of(inner: Rect, outer: Rect) -> f32 {
    let left = outer.x - inner.x;
    let top = outer.y - inner.y;
    let right = inner.right() - outer.right();
    let bottom = inner.bottom() - outer.bottom();
    left.max(top).max(right).max(bottom).max(0.0)
}

/// The axis an edge measures along. An edge only expresses *alignment* between
/// elements laid out along the **other** axis: things side by side in a row line
/// up on their tops, things stacked in a column line up on their lefts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// The six edges alignment is checked on, with the axis each measures along.
type AlignEdge = (&'static str, Axis, fn(&Rect) -> f32);

const ALIGN_EDGES: [AlignEdge; 6] = [
    ("left", Axis::X, |r| r.x),
    ("right", Axis::X, |r| r.right()),
    ("center_x", Axis::X, |r| r.x + r.width * 0.5),
    ("top", Axis::Y, |r| r.y),
    ("bottom", Axis::Y, |r| r.bottom()),
    ("center_y", Axis::Y, |r| r.y + r.height * 0.5),
];

/// Whether two rects occupy separate bands of `axis` — i.e. are stacked along
/// it rather than sitting beside each other.
fn separated_along(axis: Axis, a: Rect, b: Rect) -> bool {
    match axis {
        Axis::X => a.right() <= b.x || b.right() <= a.x,
        Axis::Y => a.bottom() <= b.y || b.bottom() <= a.y,
    }
}

/// Whether `rect` already agrees *exactly* with a quorum of `siblings` on some
/// edge of `axis`.
///
/// Differently sized elements can only line up on one edge per axis. A 16px icon
/// top-aligned beside a 32px one necessarily disagrees on `bottom` and
/// `center_y` — by exactly half the size difference, which lands squarely in
/// near-miss range. Having found the edge they *do* share, the other two are
/// explained, and reporting them is reporting the size difference twice.
fn aligned_exactly_on(rect: Rect, siblings: &[&DebugNode], axis: Axis, quorum: usize) -> bool {
    ALIGN_EDGES
        .iter()
        .filter(|(_, a, _)| *a == axis)
        .any(|(_, _, get)| {
            let v = get(&rect);
            siblings
                .iter()
                .filter(|s| (get(&s.lint_rect()) - v).abs() <= f32::EPSILON * 8.0)
                .count()
                >= quorum
        })
}

/// Flag edges that sit *almost* where their siblings agree they should be.
///
/// Three things must hold before a near miss is worth saying out loud, and each
/// exists because dropping it produced a frame's worth of false positives:
///
/// 1. **The parent is a scope.** Alignment is a claim about intent — that one
///    piece of layout code placed these together — and a scope is the only node
///    that exists because someone said "this is a region". Nesting elsewhere is
///    inferred from geometry, and an inferred parent can be a bounding box over
///    unrelated shapes: the widget gallery's loose primitives aggregate into one
///    4482px-tall node that "contains" the entire page, which made every widget
///    on it a sibling of every other and manufactured consensus between elements
///    4000px apart. To get alignment analysis over a region, name it with
///    `ui.debug_scope("name", …)`.
/// 2. **Only named siblings.** The sub-rects a single widget emits — a panel's
///    four border quads, an outline's four edges — sit a pixel or two apart by
///    construction, so including inferred nodes would flag every panel.
/// 3. **The elements are stacked along the other axis.** `left` edges are a
///    claim about a column, so they are only compared between elements in
///    different rows. Comparing the `left` edges of five icons *in* a row is
///    comparing five deliberately different numbers.
///
/// One element can only be misplaced *once* per axis, so at most one finding is
/// reported per node per axis — a button dropped 4px has a wrong top, bottom and
/// centre, but that is one bug and one line. The origin edges (`left`, `top`)
/// are preferred, being the ones layout code actually computes.
fn check_alignment(nodes: &[DebugNode], cfg: &LintConfig, out: &mut Vec<Problem>) {
    use std::collections::{BTreeMap, HashSet};

    let mut by_parent: BTreeMap<usize, Vec<&DebugNode>> = BTreeMap::new();
    for node in nodes {
        let Some(parent) = node.parent else { continue };
        if nodes.get(parent).is_none_or(|p| p.kind != NodeKind::Scope) {
            continue;
        }
        if node.named && node.axis_aligned && !node.bounds.is_empty() {
            by_parent.entry(parent).or_default().push(node);
        }
    }

    for siblings in by_parent.values() {
        if siblings.len() <= cfg.align_quorum {
            continue;
        }
        // (node id, axis) pairs already spoken for, so a displaced element is
        // reported on its origin edge and not again on the two that follow from
        // it. `ALIGN_EDGES` is ordered to put those origin edges first.
        let mut reported: HashSet<(usize, usize)> = HashSet::new();
        for (edge_name, axis, get) in ALIGN_EDGES {
            let mut values: Vec<(&DebugNode, f32)> =
                siblings.iter().map(|n| (*n, get(&n.lint_rect()))).collect();
            values.sort_by(|a, b| a.1.total_cmp(&b.1));

            // Cluster values that are within `align_max` of each other, then
            // look for a member that disagrees with the cluster's majority.
            let mut i = 0;
            while i < values.len() {
                let mut j = i + 1;
                while j < values.len() && (values[j].1 - values[j - 1].1).abs() <= cfg.align_max {
                    j += 1;
                }
                let cluster = &values[i..j];
                if cluster.len() > cfg.align_quorum {
                    // The consensus edge is the most repeated exact value.
                    let mut best_val = cluster[0].1;
                    let mut best_n = 0;
                    for &(_, v) in cluster {
                        let n = cluster
                            .iter()
                            .filter(|(_, w)| (w - v).abs() <= f32::EPSILON * 8.0)
                            .count();
                        if n > best_n {
                            best_n = n;
                            best_val = v;
                        }
                    }
                    if best_n >= cfg.align_quorum {
                        let consensus: Vec<Rect> = cluster
                            .iter()
                            .filter(|(_, v)| (v - best_val).abs() <= f32::EPSILON * 8.0)
                            .map(|(n, _)| n.lint_rect())
                            .collect();
                        for &(node, v) in cluster {
                            let delta = v - best_val;
                            if delta.abs() <= cfg.align_min || delta.abs() > cfg.align_max {
                                continue;
                            }
                            let me = node.lint_rect();
                            // Sharing this edge only means anything if the
                            // elements are stacked along the other axis.
                            let stacked = consensus
                                .iter()
                                .all(|c| separated_along(other_axis(axis), me, *c));
                            if !stacked {
                                continue;
                            }
                            // Already lined up on a different edge of the same
                            // axis: this delta is a size difference, not a
                            // placement error.
                            if aligned_exactly_on(me, siblings, axis, cfg.align_quorum) {
                                continue;
                            }
                            if !reported.insert((node.id, axis as usize)) {
                                continue;
                            }
                            out.push(Problem::NearMissAlignment {
                                node: node.id,
                                edge: edge_name,
                                actual: v,
                                expected: best_val,
                                delta,
                            });
                        }
                    }
                }
                i = j;
            }
        }
    }
}

/// The axis perpendicular to `axis`.
fn other_axis(axis: Axis) -> Axis {
    match axis {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

/// Flag siblings covering the same pixels, ignoring the containment case (a
/// background under its own content is the normal way to draw a panel).
///
/// By default this operates at application/layout boundaries only: children of
/// a named scope whose parent is not itself a declared widget allocation. A
/// widget scope's direct children are implementation paint (chrome, caption,
/// icon) and are intentionally skipped unless `inside_widgets` asks for every
/// internal symptom as well.
fn check_overlap(nodes: &[DebugNode], inside_widgets: bool, out: &mut Vec<Problem>) {
    use std::collections::BTreeMap;
    let mut by_parent: BTreeMap<Option<usize>, Vec<&DebugNode>> = BTreeMap::new();
    for node in nodes {
        if node.bounds.is_empty() || !node.axis_aligned {
            continue;
        }
        if !inside_widgets
            && node
                .parent
                .and_then(|parent| nodes.get(parent))
                .is_some_and(|parent| parent.named && parent.declared.is_some())
        {
            continue;
        }
        by_parent.entry(node.parent).or_default().push(node);
    }
    for siblings in by_parent.values() {
        for (i, a) in siblings.iter().enumerate() {
            for b in siblings.iter().skip(i + 1) {
                // At an application boundary, inferred chrome/geometry is
                // usually a row or panel background. Compare named controls and
                // readable text, not those implementation primitives. Strict
                // mode keeps every pair for widget-author diagnostics.
                if !inside_widgets
                    && !(a.named || a.kind == NodeKind::Text)
                    && !(b.named || b.kind == NodeKind::Text)
                {
                    continue;
                }
                if !inside_widgets
                    && ((a.named || a.kind == NodeKind::Text)
                        != (b.named || b.kind == NodeKind::Text))
                {
                    continue;
                }
                if a.bounds.contains_rect(b.bounds, 0.0) || b.bounds.contains_rect(a.bounds, 0.0) {
                    continue;
                }
                if let Some(overlap) = a.bounds.intersection(b.bounds) {
                    out.push(Problem::SiblingOverlap {
                        a: a.id,
                        b: b.id,
                        overlap,
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn fmt_rect(r: Rect) -> String {
    format!("{:.1},{:.1} {:.1}x{:.1}", r.x, r.y, r.width, r.height)
}

impl DebugReport {
    /// Render as an indented tree followed by a `PROBLEMS` section.
    ///
    /// This is the format meant to be read directly — by a person or by an agent
    /// grepping for `!!`. Every problem line carries its coordinates and a
    /// one-line explanation of the likely cause.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "screen {}   nodes={}  problems={}  text={}\n",
            fmt_rect(self.screen),
            self.nodes.len(),
            self.problems.len(),
            if self.text_measured {
                "measured"
            } else {
                "unmeasured (layout boxes only)"
            }
        ));
        if let Some(stats) = self.render_stats {
            s.push_str("RENDER:\n");
            s.push_str(&format!("  draw_lists={} primitives={} paint_runs={} draw_calls={}\n  color_runs={} text_runs={} icon_runs={} fragmentation={:.3}\n  buffer_write_calls={} buffer_bytes_uploaded={} atlas_uploads={} atlas_bytes_uploaded={} buffer_reallocations={}\n", stats.draw_lists, stats.primitives, stats.paint_runs, stats.draw_calls, stats.color_runs, stats.text_runs, stats.icon_runs, stats.fragmentation_ratio(), stats.buffer_write_calls, stats.buffer_bytes_uploaded, stats.atlas_uploads, stats.atlas_bytes_uploaded, stats.buffer_reallocations));
            for warning in stats.warnings() {
                s.push_str(&format!("!! WARN  render {warning}\n"));
            }
            s.push_str(&"─".repeat(72));
            s.push('\n');
        }

        s.push_str(&"─".repeat(72));
        s.push('\n');

        for node in &self.nodes {
            let indent = "  ".repeat(node.depth);
            s.push_str(&format!(
                "{indent}{} [{}] {}",
                node.name,
                node.kind.label(),
                fmt_rect(node.bounds)
            ));
            if let Some(d) = node.declared
                && d != node.bounds
            {
                s.push_str(&format!("  declared={}", fmt_rect(d)));
            }
            if let Some(c) = node.clip {
                s.push_str(&format!("  clip={}", fmt_rect(c)));
            }
            if !node.axis_aligned {
                s.push_str("  rotated(bounds are an AABB approximation)");
            }
            if !node.effects.is_empty() {
                s.push_str(&format!("  effects={}", node.effects.join(",")));
            }
            if node.counts.dropped_degenerate > 0 {
                s.push_str(&format!("  dropped={}", node.counts.dropped_degenerate));
            }
            s.push('\n');
        }

        s.push_str(&"─".repeat(72));
        s.push('\n');
        if self.problems.is_empty() {
            s.push_str("PROBLEMS: none\n");
        } else {
            s.push_str(&format!("PROBLEMS ({}):\n", self.problems.len()));
            // Cap how many times one code may repeat. A scroll view can clip
            // away dozens of rows; listing every one buries the single
            // off-screen panel that actually needs fixing. The JSON rendering
            // keeps them all.
            const MAX_PER_CODE: usize = 5;
            let mut shown: std::collections::BTreeMap<&str, usize> = Default::default();
            let mut suppressed: std::collections::BTreeMap<&str, usize> = Default::default();
            for p in &self.problems {
                let seen = shown.entry(p.code()).or_default();
                if *seen >= MAX_PER_CODE {
                    *suppressed.entry(p.code()).or_default() += 1;
                    continue;
                }
                *seen += 1;
                let name = self
                    .nodes
                    .get(p.node())
                    .map(|n| n.name.as_str())
                    .unwrap_or("?");
                s.push_str(&format!(
                    "!! {:<5} {:<24} {}\n   {}\n   {}\n",
                    p.severity().label(),
                    p.code(),
                    name,
                    p.detail(),
                    p.hint()
                ));
            }
            for (code, n) in suppressed {
                s.push_str(&format!(
                    "   … and {n} more {code} (see frame.json for the full list)\n"
                ));
            }
        }

        let inferred = self.inferred_count();
        if inferred > 0 {
            s.push_str(&format!(
                "\nnote: {inferred} of {} nodes were named by inference, and so declared \
                 no rect — typically the individual primitives a widget paints inside \
                 its own scope. Every geometric check — off-screen, clipped, text \
                 overflow — still ran on them; what is missing is the box they were \
                 *supposed* to fill, which is what the overflow and \"drew nothing\" \
                 checks compare against. Widgets in this crate declare that box \
                 themselves; if you have written your own, call \
                 `DrawList::push_debug_scope_rect(\"name\", rect)` in it.\n",
                self.nodes.len()
            ));
        }
        s
    }

    /// Render as JSON. Hand-written (the crate has no serde dependency), stable
    /// enough to parse:
    /// `{screen, text_measured, inferred_nodes, viewports: [...], nodes: [...], problems: [...]}`.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\n");
        s.push_str(&format!("  \"screen\": {},\n", json_rect(self.screen)));
        s.push_str(&format!("  \"text_measured\": {},\n", self.text_measured));
        s.push_str(&format!(
            "  \"inferred_nodes\": {},\n",
            self.inferred_count()
        ));
        s.push_str("  \"viewports\": [");
        for (i, v) in self.viewports.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&json_rect(*v));
        }
        s.push_str("],\n");

        match self.render_stats {
            Some(stats) => {
                s.push_str(&format!("  \"render_stats\": {{\"draw_lists\": {}, \"primitives\": {}, \"paint_runs\": {}, \"draw_calls\": {}, \"color_runs\": {}, \"text_runs\": {}, \"icon_runs\": {}, \"buffer_write_calls\": {}, \"buffer_bytes_uploaded\": {}, \"atlas_uploads\": {}, \"atlas_bytes_uploaded\": {}, \"buffer_reallocations\": {}, \"fragmentation_ratio\": {:.6}, \"warnings\": [", stats.draw_lists, stats.primitives, stats.paint_runs, stats.draw_calls, stats.color_runs, stats.text_runs, stats.icon_runs, stats.buffer_write_calls, stats.buffer_bytes_uploaded, stats.atlas_uploads, stats.atlas_bytes_uploaded, stats.buffer_reallocations, stats.fragmentation_ratio()));
                for (i, warning) in stats.warnings().iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(&json_str(warning));
                }
                s.push_str("]},\n");
            }
            None => s.push_str("  \"render_stats\": null,\n"),
        }

        s.push_str("  \"nodes\": [\n");
        for (i, n) in self.nodes.iter().enumerate() {
            s.push_str("    {");
            s.push_str(&format!("\"id\": {}, ", n.id));
            match n.parent {
                Some(p) => s.push_str(&format!("\"parent\": {p}, ")),
                None => s.push_str("\"parent\": null, "),
            }
            s.push_str(&format!("\"depth\": {}, ", n.depth));
            s.push_str(&format!("\"name\": {}, ", json_str(&n.name)));
            s.push_str(&format!("\"named\": {}, ", n.named));
            s.push_str(&format!("\"kind\": \"{}\", ", n.kind.label()));
            s.push_str(&format!("\"pass\": \"{}\", ", n.pass.label()));
            s.push_str(&format!("\"bounds\": {}", json_rect(n.bounds)));
            if let Some(d) = n.declared {
                s.push_str(&format!(", \"declared\": {}", json_rect(d)));
            }
            if let Some(b) = n.text_box {
                s.push_str(&format!(", \"text_box\": {}", json_rect(b)));
            }
            if let Some(c) = n.clip {
                s.push_str(&format!(", \"clip\": {}", json_rect(c)));
            }
            if let Some(t) = &n.text {
                s.push_str(&format!(", \"text\": {}", json_str(t)));
            }
            if !n.axis_aligned {
                s.push_str(", \"axis_aligned\": false");
            }
            if n.counts.dropped_degenerate > 0 {
                s.push_str(&format!(
                    ", \"dropped_degenerate\": {}",
                    n.counts.dropped_degenerate
                ));
            }
            if let Some(l) = n.layer {
                s.push_str(&format!(", \"layer\": {l}"));
            }
            s.push('}');
            if i + 1 < self.nodes.len() {
                s.push(',');
            }
            s.push('\n');
        }
        s.push_str("  ],\n");

        s.push_str("  \"problems\": [\n");
        for (i, p) in self.problems.iter().enumerate() {
            let name = self
                .nodes
                .get(p.node())
                .map(|n| n.name.as_str())
                .unwrap_or("?");
            s.push_str(&format!(
                "    {{\"code\": \"{}\", \"severity\": \"{}\", \"node\": {}, \"name\": {}, \
                 \"detail\": {}, \"hint\": {}}}",
                p.code(),
                p.severity().label(),
                p.node(),
                json_str(name),
                json_str(&p.detail()),
                json_str(p.hint())
            ));
            if i + 1 < self.problems.len() {
                s.push(',');
            }
            s.push('\n');
        }
        s.push_str("  ]\n}\n");
        s
    }
}

fn json_rect(r: Rect) -> String {
    format!(
        "{{\"x\": {:.2}, \"y\": {:.2}, \"w\": {:.2}, \"h\": {:.2}}}",
        r.x, r.y, r.width, r.height
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Problem {
    /// The concrete numbers behind this finding, as one line.
    pub fn detail(&self) -> String {
        match self {
            Problem::OffScreen { bounds, screen, .. } => format!(
                "bounds {} lies entirely outside the viewport {}",
                fmt_rect(*bounds),
                fmt_rect(*screen)
            ),
            Problem::PartiallyOffScreen { bounds, screen, .. } => format!(
                "bounds {} crosses the edge of viewport {}",
                fmt_rect(*bounds),
                fmt_rect(*screen)
            ),
            Problem::FullyClipped { bounds, clip, .. } => format!(
                "bounds {} does not intersect clip {}",
                fmt_rect(*bounds),
                fmt_rect(*clip)
            ),
            Problem::PartiallyClipped { bounds, clip, .. } => format!(
                "bounds {} extends past clip {}",
                fmt_rect(*bounds),
                fmt_rect(*clip)
            ),
            Problem::OverflowsDeclared {
                painted,
                declared,
                overshoot,
                ..
            } => format!(
                "painted {} but declared {} — overshoots by {:.1}px",
                fmt_rect(*painted),
                fmt_rect(*declared),
                overshoot
            ),
            Problem::MissingPaint { declared, .. } => {
                format!("declared {} and drew nothing", fmt_rect(*declared))
            }
            Problem::DegenerateDeclaredRect { declared, .. } => {
                format!("declared {} has zero or negative area", fmt_rect(*declared))
            }
            Problem::DroppedDegenerate { count, .. } => {
                format!("{count} primitive(s) dropped for non-positive size/radius/thickness")
            }
            Problem::TextOverflowsBox { ink, text_box, .. } => format!(
                "shaped ink {} exceeds layout box {}",
                fmt_rect(*ink),
                fmt_rect(*text_box)
            ),
            Problem::TextEllipsized {
                wanted, available, ..
            } => format!("wants {wanted:.1}px of width, has {available:.1}px"),
            Problem::NearMissAlignment {
                edge,
                actual,
                expected,
                delta,
                ..
            } => format!(
                "{edge} edge at {actual:.2}, siblings agree on {expected:.2} (off by {delta:+.2}px)"
            ),
            Problem::InvisibleAlpha { bounds, .. } => {
                format!(
                    "rect {} has zero alpha in fill and border",
                    fmt_rect(*bounds)
                )
            }
            Problem::SiblingOverlap { a, b, overlap } => {
                format!("node {a} and node {b} overlap over {}", fmt_rect(*overlap))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;
    use crate::text::TextBlock;
    use crate::widgets::DrawList;

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    fn find<'a>(r: &'a DebugReport, name: &str) -> &'a DebugNode {
        r.node(name)
            .unwrap_or_else(|| panic!("no node named {name:?} in:\n{}", r.to_text()))
    }

    fn codes(r: &DebugReport) -> Vec<&'static str> {
        r.problems.iter().map(|p| p.code()).collect()
    }

    // ---- Tree construction ----

    #[test]
    fn scope_bounds_are_the_union_of_what_it_painted() {
        let mut list = DrawList::new();
        list.push_debug_scope("toolbar");
        list.chrome_rect(
            Rect::new(10.0, 10.0, 40.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.chrome_rect(
            Rect::new(60.0, 15.0, 40.0, 30.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let toolbar = find(&report, "toolbar");
        assert_eq!(toolbar.kind, NodeKind::Scope);
        assert!(toolbar.named);
        assert_eq!(toolbar.bounds, Rect::new(10.0, 10.0, 90.0, 35.0));
        assert_eq!(toolbar.counts.chrome_instances, 2);
    }

    #[test]
    fn nested_scopes_form_a_tree_with_parents_before_children() {
        let mut list = DrawList::new();
        list.push_debug_scope("window");
        list.push_debug_scope("row");
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let window = find(&report, "window");
        let row = find(&report, "row");
        assert_eq!(row.parent, Some(window.id));
        assert!(window.id < row.id, "parents must precede children");
        assert_eq!(window.depth, 0);
        assert_eq!(row.depth, 1);
        // The outer scope's bounds include the inner scope's geometry.
        assert_eq!(window.bounds, Rect::new(0.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn primitives_attach_to_their_innermost_scope() {
        let mut list = DrawList::new();
        list.push_debug_scope("outer");
        list.chrome_rect(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.push_debug_scope("inner");
        list.chrome_rect(
            Rect::new(10.0, 10.0, 20.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let inner = find(&report, "inner");
        let chrome1 = find(&report, "chrome#1");
        assert_eq!(chrome1.parent, Some(inner.id), "innermost scope wins");
    }

    #[test]
    fn unscoped_draws_are_still_reported_and_marked_inferred() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(5.0, 5.0, 50.0, 50.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.text(TextBlock::new("Save", 10.0, 10.0));

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(report.inferred_count() >= 2);
        // Text names itself from its own content — the best clue available.
        let save = find(&report, "\"Save\"");
        assert!(!save.named);
        assert_eq!(save.text.as_deref(), Some("Save"));
        assert!(
            report.to_text().contains("push_debug_scope_rect"),
            "hints at better naming"
        );
    }

    #[test]
    fn unscoped_nodes_nest_by_geometric_containment() {
        let mut list = DrawList::new();
        // A background with a label on it, drawn with no scopes at all.
        list.chrome_rect(
            Rect::new(0.0, 0.0, 200.0, 60.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.text(TextBlock::new("Hello", 20.0, 20.0).with_max_width(80.0));

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let bg = find(&report, "chrome#0");
        let label = find(&report, "\"Hello\"");
        assert_eq!(
            label.parent,
            Some(bg.id),
            "the label sits inside the background"
        );
    }

    #[test]
    fn unclosed_scope_still_owns_everything_after_it() {
        let mut list = DrawList::new();
        list.push_debug_scope("leaked");
        list.chrome_rect(
            Rect::new(0.0, 0.0, 30.0, 30.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        // no pop

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let leaked = find(&report, "leaked");
        assert_eq!(leaked.bounds, Rect::new(0.0, 0.0, 30.0, 30.0));
    }

    #[test]
    fn nodes_are_ordered_by_paint_submission_across_families() {
        let mut list = DrawList::new();
        list.text(TextBlock::new("label", 0.0, 0.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let kinds: Vec<_> = report.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NodeKind::Text, NodeKind::Chrome]);
        assert_eq!(report.nodes[0].pass, RenderPass::Text);
    }

    #[test]
    fn soup_runs_separated_by_another_command_stay_separate() {
        let mut list = DrawList::new();
        list.triangle((0.0, 0.0), (2.0, 0.0), (0.0, 2.0), [1.0; 4]);
        list.text(TextBlock::new("middle", 10.0, 10.0));
        list.triangle((20.0, 20.0), (22.0, 20.0), (20.0, 22.0), [1.0; 4]);

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let kinds: Vec<_> = report.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [NodeKind::Geometry, NodeKind::Text, NodeKind::Geometry]
        );
        assert_eq!(report.nodes[0].name, "geometry#0");
        assert_eq!(report.nodes[2].name, "geometry#1");
    }

    // ---- Lints ----

    #[test]
    fn off_screen_element_is_reported() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(900.0, 100.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"off_screen"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn on_screen_element_is_not_reported() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(100.0, 100.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(report.problems.is_empty(), "{}", report.to_text());
    }

    #[test]
    fn partially_off_screen_is_off_by_default_and_on_under_strict() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(780.0, 10.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(!codes(&report).contains(&"partially_off_screen"));

        let strict = report.with_lints(&LintConfig::strict());
        assert!(codes(&strict).contains(&"partially_off_screen"));
    }

    #[test]
    fn fully_clipped_element_is_reported() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(200.0, 200.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"fully_clipped"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn content_scrolled_out_of_a_viewport_is_not_a_problem() {
        let mut list = DrawList::new();
        // A viewport showing one row and hiding the next: exactly a scroll view
        // doing its job. Neither the hidden row nor the visible one is a defect.
        list.push_clip_viewport(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(10.0, 10.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.chrome_rect(
            Rect::new(10.0, 200.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            !codes(&report).contains(&"fully_clipped"),
            "{}",
            report.to_text()
        );
        // It is still *visible* in the tree, tagged with why it did not draw.
        let hidden = report
            .nodes
            .iter()
            .find(|n| n.bounds.y == 200.0)
            .expect("hidden row is still a node");
        assert!(hidden.effects.contains(&"off-viewport"));
        assert_eq!(report.viewports, vec![Rect::new(0.0, 0.0, 100.0, 100.0)]);
    }

    #[test]
    fn a_scope_is_not_overflowing_just_because_its_content_scrolls() {
        // The scroll-view shape: a scope declaring the viewport, whose content is
        // deliberately taller than it. A scope's bounds are the union of its
        // children, so unioning *unclipped* bounds would make every scrolling
        // container in the crate report as overflowing the box it declared.
        let mut list = DrawList::new();
        list.push_debug_scope_rect("ScrollView", Rect::new(0.0, 0.0, 100.0, 100.0));
        list.push_clip_viewport(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.chrome_rect(
            Rect::new(0.0, 40.0, 100.0, 260.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            !codes(&report).contains(&"overflows_declared"),
            "{}",
            report.to_text()
        );
        let sv = find(&report, "ScrollView");
        assert_eq!(sv.bounds.height, 100.0, "bounds clipped to the viewport");
    }

    #[test]
    fn a_hard_clip_does_not_hide_content_that_missed_its_container() {
        // The mirror image: same geometry, but a *boundary* clip. Here the
        // overflow is a real bug and the clip erasing it must not conceal that.
        let mut list = DrawList::new();
        list.push_debug_scope_rect("Panel", Rect::new(0.0, 0.0, 100.0, 100.0));
        list.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.chrome_rect(
            Rect::new(0.0, 40.0, 100.0, 260.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"overflows_declared"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn a_viewport_does_not_excuse_content_clipped_away_elsewhere() {
        let mut list = DrawList::new();
        // A viewport somewhere in the frame...
        list.push_clip_viewport(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.pop_clip();
        // ...must not excuse an unrelated hard boundary losing its content.
        list.push_clip(Rect::new(300.0, 300.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(500.0, 500.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"fully_clipped"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn nested_clips_inside_a_viewport_are_still_excused() {
        let mut list = DrawList::new();
        list.push_clip_viewport(Rect::new(0.0, 0.0, 100.0, 100.0));
        // A row that pushes its own clip and is then scrolled out of view: its
        // effective clip is a sub-rect of the viewport, so containment excuses it.
        list.push_clip(Rect::new(10.0, 10.0, 50.0, 20.0));
        list.chrome_rect(
            Rect::new(10.0, 400.0, 50.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();
        list.pop_clip();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            !codes(&report).contains(&"fully_clipped"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn viewport_marking_resets_with_the_list() {
        let mut list = DrawList::new();
        list.push_clip_viewport(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.pop_clip();
        assert_eq!(list.viewport_clips().len(), 1);
        list.clear();
        assert!(list.viewport_clips().is_empty());
    }

    #[test]
    fn partially_clipped_is_off_by_default_because_scroll_views_do_it() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(10.0, 50.0, 50.0, 200.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_clip();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(!codes(&report).contains(&"partially_clipped"));
        let strict = report.with_lints(&LintConfig::strict());
        assert!(codes(&strict).contains(&"partially_clipped"));
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn overlap_defaults_to_layout_peers_but_strict_includes_widget_internals() {
        let mut list = DrawList::new();
        list.push_debug_scope("form");

        // Application peers: a caption intrudes into the neighbouring widget.
        list.push_debug_scope_rect("TextInput", Rect::new(40.0, 0.0, 60.0, 24.0));
        list.chrome_rect(
            Rect::new(40.0, 0.0, 60.0, 24.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();
        list.text(TextBlock::new("Label", 0.0, 0.0).with_max_width(50.0));

        // Widget internals: chrome and a wrapped caption partially overlap. This
        // is useful in strict/all diagnostics, but redundant in the default view
        // because the widget's declared allocation owns the implementation bug.
        list.push_debug_scope_rect("Dropdown", Rect::new(0.0, 40.0, 60.0, 24.0));
        list.chrome_rect(
            Rect::new(0.0, 40.0, 60.0, 24.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.text(TextBlock::new("Long dropdown caption", 8.0, 40.0).with_max_width(44.0));
        list.pop_debug_scope();
        list.pop_debug_scope();

        let report = DebugReport::measured(&mut list, SCREEN);
        let default_overlaps = report
            .problems
            .iter()
            .filter(|problem| problem.code() == "sibling_overlap")
            .count();
        assert_eq!(default_overlaps, 1, "{}", report.to_text());

        let strict = report.with_lints(&LintConfig::strict());
        let strict_overlaps = strict
            .problems
            .iter()
            .filter(|problem| problem.code() == "sibling_overlap")
            .count();
        assert_eq!(strict_overlaps, 2, "{}", strict.to_text());
    }

    #[test]
    fn overflow_of_a_declared_box_is_reported_with_the_overshoot() {
        let mut list = DrawList::new();
        list.push_debug_scope_rect("card", Rect::new(0.0, 0.0, 100.0, 50.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 130.0, 50.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let p = report
            .problems
            .iter()
            .find(|p| p.code() == "overflows_declared")
            .unwrap_or_else(|| panic!("{}", report.to_text()));
        match p {
            Problem::OverflowsDeclared { overshoot, .. } => {
                assert!((overshoot - 30.0).abs() < 0.01, "overshoot {overshoot}")
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_scope_within_its_declared_box_is_clean() {
        let mut list = DrawList::new();
        list.push_debug_scope_rect("card", Rect::new(0.0, 0.0, 100.0, 50.0));
        list.chrome_rect(
            Rect::new(5.0, 5.0, 90.0, 40.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(report.problems.is_empty(), "{}", report.to_text());
    }

    #[test]
    fn a_scope_that_declared_a_box_and_drew_nothing_is_reported() {
        let mut list = DrawList::new();
        list.push_debug_scope_rect("ghost", Rect::new(10.0, 10.0, 100.0, 50.0));
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"missing_paint"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn a_collapsed_declared_box_is_reported() {
        let mut list = DrawList::new();
        // width - padding * 2 went negative.
        list.push_debug_scope_rect("squashed", Rect::new(10.0, 10.0, 0.0, 50.0));
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"degenerate_declared_rect"),
            "{}",
            report.to_text()
        );
    }

    /// A label centred by `vcentered_text_y` sits with its *line box* poking out
    /// of the row on purpose — the box is slid up so the glyph ink, not the box,
    /// lands on the row's centre. Measuring the box instead of the ink turned
    /// every centred label in the widget gallery into an overflow warning.
    #[cfg(feature = "bundled-font")]
    #[test]
    fn an_optically_centred_label_does_not_overflow_its_row() {
        let row = Rect::new(0.0, 100.0, 120.0, 20.0);
        let mut list = DrawList::new();
        list.push_debug_scope_rect("row", row);
        let y = list.vcentered_text_y(row.y, row.height, 14.0, None, "Medium");
        assert!(
            y < row.y,
            "precondition: optical centring lifts the line box above the row \
             (line box at {y}, row top {})",
            row.y
        );
        list.text(TextBlock::new("Medium", row.x + 4.0, y).with_size(14.0));
        list.pop_debug_scope();

        let report = DebugReport::measured(&mut list, SCREEN);
        assert!(
            !codes(&report).contains(&"overflows_declared"),
            "{}",
            report.to_text()
        );

        // And the recorded bounds really are the ink, sitting inside the row.
        let text = report
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Text)
            .expect("the label is in the report");
        assert!(
            text.bounds.y >= row.y && text.bounds.bottom() <= row.bottom(),
            "ink {:?} should sit inside row {row:?}",
            text.bounds
        );
    }

    /// The counterpart: ink that genuinely leaves the box must still be caught,
    /// or the fix above would have bought silence rather than accuracy.
    #[test]
    fn text_that_really_escapes_its_row_is_still_reported() {
        let row = Rect::new(0.0, 100.0, 120.0, 20.0);
        let mut list = DrawList::new();
        list.push_debug_scope_rect("row", row);
        // Font far too large for the row: the ink overshoots on its own merits.
        list.text(TextBlock::new("Medium", row.x, row.y).with_size(40.0));
        list.pop_debug_scope();

        let report = DebugReport::measured(&mut list, SCREEN);
        assert!(
            codes(&report).contains(&"overflows_declared"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn silently_dropped_primitives_are_surfaced() {
        // The headline case: nothing lands in any buffer, so only the counter
        // can prove the element was meant to exist.
        let mut list = DrawList::new();
        list.push_debug_scope_rect("row", Rect::new(0.0, 0.0, 100.0, 20.0));
        list.quad(0.0, 0.0, -8.0, 20.0, [1.0; 4]);
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let c = codes(&report);
        assert!(c.contains(&"dropped_degenerate"), "{}", report.to_text());
        assert!(c.contains(&"missing_paint"));
    }

    #[test]
    fn dropped_primitives_outside_any_scope_are_still_surfaced() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, 10.0, -1.0, [1.0; 4]);
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"dropped_degenerate"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn invisible_chrome_is_reported() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(10.0, 10.0, 40.0, 20.0),
            0.0,
            0.0,
            [1.0, 1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0, 0.0],
        );
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            codes(&report).contains(&"invisible_alpha"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn rotated_bounds_do_not_trigger_geometry_lints() {
        // Under rotation the AABB is an inflated approximation of a shape that
        // was never that big; reporting against it would be a lie.
        let mut list = DrawList::new();
        list.push_transform();
        list.rotate(0.4);
        list.icon("gear", 700.0, 500.0, 300.0, 300.0);
        list.pop_transform();
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(!codes(&report).contains(&"partially_off_screen"));
    }

    // ---- Alignment ----

    #[test]
    fn a_column_off_by_half_a_pixel_is_caught() {
        let mut list = DrawList::new();
        list.push_debug_scope("form");
        for (i, x) in [20.0f32, 20.0, 21.5, 20.0].into_iter().enumerate() {
            list.push_debug_scope_rect(
                format!("field{i}"),
                Rect::new(x, 10.0 + i as f32 * 30.0, 100.0, 20.0),
            );
            list.chrome_rect(
                Rect::new(x, 10.0 + i as f32 * 30.0, 100.0, 20.0),
                0.0,
                0.0,
                [1.0; 4],
                [0.0; 4],
            );
            list.pop_debug_scope();
        }
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        let misaligned: Vec<_> = report
            .problems
            .iter()
            .filter(|p| p.code() == "near_miss_alignment")
            .collect();
        assert!(!misaligned.is_empty(), "{}", report.to_text());
        match misaligned[0] {
            Problem::NearMissAlignment {
                node, edge, delta, ..
            } => {
                assert_eq!(report.nodes[*node].name, "field2");
                assert_eq!(*edge, "left");
                assert!((delta - 1.5).abs() < 0.01, "delta {delta}");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_properly_aligned_column_is_clean() {
        let mut list = DrawList::new();
        list.push_debug_scope("form");
        for i in 0..4 {
            list.push_debug_scope_rect(
                format!("field{i}"),
                Rect::new(20.0, 10.0 + i as f32 * 30.0, 100.0, 20.0),
            );
            list.chrome_rect(
                Rect::new(20.0, 10.0 + i as f32 * 30.0, 100.0, 20.0),
                0.0,
                0.0,
                [1.0; 4],
                [0.0; 4],
            );
            list.pop_debug_scope();
        }
        list.pop_debug_scope();

        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(report.problems.is_empty(), "{}", report.to_text());
    }

    #[test]
    fn alignment_ignores_inferred_nodes() {
        // A panel's own border quads sit a pixel apart by construction. If
        // inferred nodes took part, every panel in the frame would be flagged.
        let mut list = DrawList::new();
        for x in [10.0f32, 10.0, 11.0, 10.0] {
            list.chrome_rect(Rect::new(x, 0.0, 5.0, 100.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        }
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            !codes(&report).contains(&"near_miss_alignment"),
            "{}",
            report.to_text()
        );
    }

    /// Lay out `rects` as named scopes inside a `form` scope and report.
    fn scoped_layout(rects: &[Rect]) -> DebugReport {
        let mut list = DrawList::new();
        list.push_debug_scope("form");
        for (i, r) in rects.iter().enumerate() {
            list.push_debug_scope_rect(format!("field{i}"), *r);
            list.chrome_rect(*r, 0.0, 0.0, [1.0; 4], [0.0; 4]);
            list.pop_debug_scope();
        }
        list.pop_debug_scope();
        DebugReport::from_draw_list(&list, SCREEN)
    }

    #[test]
    fn a_row_is_not_compared_on_the_edges_it_varies_along() {
        // Five boxes side by side. Their lefts differ by a few px because their
        // widths do — that is what a row *is*, not a near miss.
        let report = scoped_layout(&[
            Rect::new(0.0, 0.0, 40.0, 20.0),
            Rect::new(44.0, 0.0, 36.0, 20.0),
            Rect::new(84.0, 0.0, 42.0, 20.0),
            Rect::new(130.0, 0.0, 38.0, 20.0),
        ]);
        assert!(
            !codes(&report).contains(&"near_miss_alignment"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn a_row_of_mixed_heights_aligned_on_top_is_clean() {
        // Tops match exactly, so bottoms and centres cannot. Reporting those is
        // reporting the height difference, which was deliberate.
        let report = scoped_layout(&[
            Rect::new(0.0, 0.0, 40.0, 20.0),
            Rect::new(50.0, 0.0, 40.0, 26.0),
            Rect::new(100.0, 0.0, 40.0, 22.0),
            Rect::new(150.0, 0.0, 40.0, 28.0),
        ]);
        assert!(
            !codes(&report).contains(&"near_miss_alignment"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn one_displaced_element_is_one_finding() {
        // A dropped row member has a wrong top, bottom *and* centre. One bug.
        let report = scoped_layout(&[
            Rect::new(0.0, 0.0, 40.0, 20.0),
            Rect::new(50.0, 0.0, 40.0, 20.0),
            Rect::new(100.0, 3.0, 40.0, 20.0),
            Rect::new(150.0, 0.0, 40.0, 20.0),
        ]);
        let hits: Vec<_> = report
            .problems
            .iter()
            .filter(|p| p.code() == "near_miss_alignment")
            .collect();
        assert_eq!(hits.len(), 1, "{}", report.to_text());
        match hits[0] {
            Problem::NearMissAlignment {
                node, edge, delta, ..
            } => {
                assert_eq!(report.nodes[*node].name, "field2");
                assert_eq!(*edge, "top", "the origin edge, not bottom or centre");
                assert!((delta - 3.0).abs() < 0.01, "delta {delta}");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn alignment_needs_a_declared_region() {
        // The same stray column with no enclosing scope. Whatever parent the
        // tree infers is a geometric accident, not a statement that these were
        // laid out together — in the widget gallery it was a single aggregate
        // spanning the whole page.
        let mut list = DrawList::new();
        for (i, x) in [20.0f32, 20.0, 21.5, 20.0].into_iter().enumerate() {
            list.push_debug_scope_rect(
                format!("field{i}"),
                Rect::new(x, 10.0 + i as f32 * 30.0, 100.0, 20.0),
            );
            list.chrome_rect(
                Rect::new(x, 10.0 + i as f32 * 30.0, 100.0, 20.0),
                0.0,
                0.0,
                [1.0; 4],
                [0.0; 4],
            );
            list.pop_debug_scope();
        }
        let report = DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            !codes(&report).contains(&"near_miss_alignment"),
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn an_aggregate_of_loose_geometry_never_adopts_anything() {
        // Two quads at opposite corners aggregate into one node whose bounds
        // span the gap between them — a rect that covers mostly nothing. Left
        // eligible as a parent it swallows everything in between.
        let mut list = DrawList::new();
        list.triangle((0.0, 0.0), (10.0, 0.0), (0.0, 10.0), [1.0; 4]);
        list.triangle((700.0, 500.0), (710.0, 500.0), (700.0, 510.0), [1.0; 4]);
        list.push_debug_scope_rect("widget", Rect::new(300.0, 200.0, 40.0, 20.0));
        list.chrome_rect(
            Rect::new(300.0, 200.0, 40.0, 20.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();

        let report = DebugReport::measured(&mut list, SCREEN);
        let soup = find(&report, "geometry#0");
        assert!(
            soup.bounds.width > 700.0,
            "precondition: the aggregate spans both quads ({:?})",
            soup.bounds
        );
        let widget = find(&report, "widget");
        assert_ne!(
            widget.parent,
            Some(soup.id),
            "loose geometry must not adopt a real region\n\n{}",
            report.to_text()
        );
    }

    // ---- Assertions & rendering ----

    #[test]
    #[should_panic(expected = "layout problem")]
    fn assert_clean_panics_with_the_report() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(2000.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        DebugReport::from_draw_list(&list, SCREEN).assert_clean();
    }

    #[test]
    fn assert_clean_passes_on_a_good_layout() {
        let mut list = DrawList::new();
        list.push_debug_scope_rect("panel", Rect::new(10.0, 10.0, 200.0, 100.0));
        list.chrome_rect(
            Rect::new(10.0, 10.0, 200.0, 100.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();
        DebugReport::from_draw_list(&list, SCREEN).assert_clean();
    }

    #[test]
    fn json_is_well_formed_and_escapes_content() {
        let mut list = DrawList::new();
        list.push_debug_scope_rect("panel", Rect::new(0.0, 0.0, 100.0, 40.0));
        list.text(TextBlock::new("say \"hi\"\n\tnow", 5.0, 5.0).with_max_width(90.0));
        list.pop_debug_scope();

        let json = DebugReport::from_draw_list(&list, SCREEN).to_json();
        assert!(json.starts_with('{') && json.trim_end().ends_with('}'));
        assert!(json.contains(r#"\"hi\""#), "quotes escaped: {json}");
        assert!(json.contains("\\n") && json.contains("\\t"));
        // Braces balance — a crude but effective well-formedness check.
        assert_eq!(
            json.chars().filter(|&c| c == '{').count(),
            json.chars().filter(|&c| c == '}').count()
        );
    }

    #[test]
    fn text_rendering_lists_nodes_and_problems() {
        let mut list = DrawList::new();
        list.push_debug_scope_rect("sidebar", Rect::new(0.0, 0.0, 100.0, 100.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.pop_debug_scope();

        let text = DebugReport::from_draw_list(&list, SCREEN).to_text();
        assert!(text.contains("sidebar"));
        assert!(text.contains("PROBLEMS: none"));
        assert!(text.contains("screen 0.0,0.0 800.0x600.0"));
    }

    #[test]
    fn render_stats_are_opt_in_and_render_in_text_and_json() {
        let list = DrawList::new();
        let plain = DebugReport::from_draw_list(&list, SCREEN);
        assert!(plain.render_stats.is_none());
        assert!(plain.to_json().contains("\"render_stats\": null"));

        let stats = RenderStats {
            draw_lists: 2,
            primitives: 100,
            paint_runs: 130,
            draw_calls: 7,
            color_runs: 3,
            text_runs: 2,
            icon_runs: 2,
            buffer_write_calls: 4,
            buffer_bytes_uploaded: 4096,
            atlas_uploads: 2,
            atlas_bytes_uploaded: 512,
            buffer_reallocations: 1,
        };
        let report = plain.with_render_stats(stats);
        let text = report.to_text();
        assert!(text.contains("RENDER:"));
        assert!(text.contains("draw_lists=2 primitives=100 paint_runs=130 draw_calls=7"));
        assert!(text.contains("!! WARN  render"));
        let json = report.to_json();
        assert!(json.contains("\"render_stats\": {"));
        assert!(json.contains("\"buffer_bytes_uploaded\": 4096"));
        assert!(json.contains("\"fragmentation_ratio\": 1.300000"));
        assert!(json.contains("\"warnings\": ["));
    }

    #[test]
    fn lint_config_none_finds_nothing_but_still_builds_the_tree() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(5000.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        let report = DebugReport::from_draw_list(&list, SCREEN).with_lints(&LintConfig::none());
        assert!(report.problems.is_empty());
        assert!(!report.nodes.is_empty());
    }
}
