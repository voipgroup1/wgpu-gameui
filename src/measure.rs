//! Contextual, single-shot widget measurement and measured stack arrangement.
//!
//! Measurement is deliberately separate from drawing: it may shape text and
//! resolve styles, but never emits paint, registers interaction, or invokes an
//! application draw callback. Containers consume the resulting plain records.

use crate::layout::{Constraint, CrossAlign, LayoutResult, MainAlign, NodeId, Rect, SizeSpec};
use crate::{FontSpec, StyleResolver, TextBlock, TextMeasurer, WrapMode};

/// Logical-pixel bounds supplied to one public measurement operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasureConstraints {
    /// Minimum accepted width.
    pub min_width: f32,
    /// Maximum accepted width, or `None` when unbounded.
    pub max_width: Option<f32>,
    /// Minimum accepted height.
    pub min_height: f32,
    /// Maximum accepted height, or `None` when unbounded.
    pub max_height: Option<f32>,
}

impl MeasureConstraints {
    /// No lower or upper bounds.
    pub const UNBOUNDED: Self = Self {
        min_width: 0.0,
        max_width: None,
        min_height: 0.0,
        max_height: None,
    };

    /// Constrain only the maximum width.
    pub fn with_max_width(max_width: f32) -> Self {
        Self {
            max_width: Some(max_width),
            ..Self::UNBOUNDED
        }
        .normalized()
    }

    /// Normalize non-finite/negative bounds and make each minimum win over a
    /// smaller maximum, matching the layout [`Constraint`] rule.
    pub fn normalized(self) -> Self {
        fn lower(value: f32) -> f32 {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        }
        fn upper(value: Option<f32>) -> Option<f32> {
            value.and_then(|v| v.is_finite().then_some(v.max(0.0)))
        }
        let min_width = lower(self.min_width);
        let min_height = lower(self.min_height);
        Self {
            min_width,
            max_width: upper(self.max_width).map(|v| v.max(min_width)),
            min_height,
            max_height: upper(self.max_height).map(|v| v.max(min_height)),
        }
    }

    pub(crate) fn clamp_size(self, size: [f32; 2]) -> [f32; 2] {
        let c = self.normalized();
        [
            Constraint {
                min: Some(c.min_width),
                max: c.max_width,
            }
            .apply(size[0].max(0.0)),
            Constraint {
                min: Some(c.min_height),
                max: c.max_height,
            }
            .apply(size[1].max(0.0)),
        ]
    }
}

impl Default for MeasureConstraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

/// Intrinsic and constraint-resolved geometry for one measured leaf.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    /// Smallest useful size.
    pub min: [f32; 2],
    /// Preferred size after applying the measurement constraints.
    pub preferred: [f32; 2],
    /// Maximum useful size per axis; `None` means expandable without a bound.
    pub max: [Option<f32>; 2],
    /// First baseline offset down from the top of the preferred box.
    pub baseline: Option<f32>,
    /// Width used to prepare height-sensitive content; `None` means height does
    /// not depend on the eventual arranged width.
    pub measured_width: Option<f32>,
}

impl Measurement {
    /// Construct and normalize an intrinsic result.
    pub fn new(
        min: [f32; 2],
        preferred: [f32; 2],
        max: [Option<f32>; 2],
        baseline: Option<f32>,
    ) -> Self {
        let min = [finite_nonnegative(min[0]), finite_nonnegative(min[1])];
        let mut max = [normalize_max(max[0], min[0]), normalize_max(max[1], min[1])];
        let preferred = [
            clamp_axis(preferred[0], min[0], max[0]),
            clamp_axis(preferred[1], min[1], max[1]),
        ];
        if max[0].is_some_and(|v| v < preferred[0]) {
            max[0] = Some(preferred[0]);
        }
        if max[1].is_some_and(|v| v < preferred[1]) {
            max[1] = Some(preferred[1]);
        }
        Self {
            min,
            preferred,
            max,
            baseline: baseline
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, preferred[1])),
            measured_width: None,
        }
    }

    /// Mark the width for which this result's preferred height was prepared.
    pub fn width_sensitive(mut self, width: f32) -> Self {
        self.measured_width = Some(finite_nonnegative(width));
        self
    }

    /// Whether assigning another width may invalidate the preferred height.
    pub fn is_width_sensitive(self) -> bool {
        self.measured_width.is_some()
    }
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn normalize_max(value: Option<f32>, min: f32) -> Option<f32> {
    value.and_then(|v| v.is_finite().then_some(v.max(min)))
}

fn clamp_axis(value: f32, min: f32, max: Option<f32>) -> f32 {
    let value = finite_nonnegative(value).max(min);
    max.map_or(value, |max| value.min(max))
}

fn measured_axis(measurement: Measurement, axis: usize, value: f32) -> f32 {
    clamp_axis(value, measurement.min[axis], measurement.max[axis])
}

/// Structured geometry from shaping one configured text block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    /// Advance width and total line-box height.
    pub size: [f32; 2],
    /// Approximate painted ink bounds relative to the block origin. Horizontal
    /// bounds use the shaped advance; vertical bounds use actual glyph outlines.
    pub ink_bounds: Option<Rect>,
    /// First baseline offset down from the block top.
    pub baseline: f32,
    /// Visual-center offset down from the block top, using the same font/script
    /// policy as widget label painting.
    pub visual_center: f32,
    /// Number of shaped lines (empty text still occupies one line box).
    pub line_count: usize,
    /// True when unwrapped content was wider than the supplied maximum width.
    pub overflowed_width: bool,
}

/// Text configured and measured once, ready to be consumed by paint. Intentionally
/// not `Clone`: consuming it transfers owned text/spans into paint without a
/// per-item heap copy.
pub struct MeasuredText {
    block: TextBlock,
    allocation: [f32; 2],
    /// Structured metrics for the prepared block.
    pub metrics: TextMetrics,
    /// Intrinsic geometry consumable by measured containers.
    pub measurement: Measurement,
}

impl MeasuredText {
    /// Consume the prepared text and place its origin for paint without cloning
    /// its owned content or span vectors.
    pub fn into_block_at(mut self, x: f32, y: f32) -> TextBlock {
        self.block.x = x;
        self.block.y = y;
        if self.metrics.size[0] > self.allocation[0] || self.metrics.size[1] > self.allocation[1] {
            self.block.clip = Some(Rect::new(x, y, self.allocation[0], self.allocation[1]));
        }
        self.block
    }

    /// Inspect the configured block before consuming it.
    pub fn block(&self) -> &TextBlock {
        &self.block
    }

    /// Consume this prepared result and return its configured block unchanged.
    pub fn into_block(self) -> TextBlock {
        let [x, y] = [self.block.x, self.block.y];
        self.into_block_at(x, y)
    }
}

/// Borrowed resources and resolved environment used by widget measurement.
pub struct MeasureContext<'a> {
    text: &'a mut TextMeasurer,
    styles: StyleResolver<'a>,
    font: FontSpec,
    constraints: MeasureConstraints,
    scale_factor: f32,
    wrap: WrapMode,
}

impl<'a> MeasureContext<'a> {
    /// Construct a narrow measurement context. Dimensions are logical pixels;
    /// scale is carried for scale-dependent policy but does not snap geometry.
    pub fn new(
        text: &'a mut TextMeasurer,
        styles: StyleResolver<'a>,
        font: FontSpec,
        constraints: MeasureConstraints,
        scale_factor: f32,
        wrap: WrapMode,
    ) -> Self {
        Self {
            text,
            styles,
            font,
            constraints: constraints.normalized(),
            scale_factor: if scale_factor.is_finite() && scale_factor > 0.0 {
                scale_factor
            } else {
                1.0
            },
            wrap,
        }
    }

    /// Resolved styles for the measured subtree.
    pub fn styles(&self) -> StyleResolver<'a> {
        self.styles
    }

    /// Borrow the shared CPU text measurer for custom widget metrics that need
    /// the same shaping/cache path without access to paint buffers.
    pub fn text_measurer(&mut self) -> &mut TextMeasurer {
        self.text
    }
    /// Active font selection.
    pub fn font(&self) -> &FontSpec {
        &self.font
    }
    /// Normalized logical constraints.
    pub fn constraints(&self) -> MeasureConstraints {
        self.constraints
    }
    /// Host scale factor (not applied to logical coordinates in this phase).
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
    /// Active wrapping policy.
    pub fn wrap(&self) -> WrapMode {
        self.wrap
    }

    /// Build body text from the resolved style, then apply the active font stack.
    pub fn text_block(&self, content: impl Into<String>) -> TextBlock {
        let mut block = self.styles.text_block(content, 0.0, 0.0);
        block.font_size = self.font.size;
        block.line_height = self.font.size * 1.25;
        block.font = self.font.font.clone();
        block.letter_spacing = self.font.letter_spacing;
        block.weight = self.font.weight;
        block.style = self.font.style;
        block.wrap = self.wrap;
        block
    }

    /// Measure an already configured block exactly once at the public API level
    /// and retain it for allocation-free ownership transfer into paint.
    pub fn measure_text(&mut self, block: TextBlock) -> MeasuredText {
        self.measure_text_with(block, self.constraints)
    }

    /// Measure a configured block under an explicit constraint set while keeping
    /// this context's resolved font/style/wrap environment.
    pub fn measure_text_with(
        &mut self,
        mut block: TextBlock,
        constraints: MeasureConstraints,
    ) -> MeasuredText {
        let constraints = constraints.normalized();
        block.wrap = self.wrap;
        let natural = self.text.measure_styled_with_letter_spacing(
            &block.content,
            block.font_size,
            None,
            block.font.as_ref(),
            block.weight,
            block.style,
            WrapMode::None,
            block.letter_spacing,
        );
        let constrained_width = constraints.max_width;
        block.max_width = constrained_width.unwrap_or(f32::MAX / 4.0);
        let measured = self.text.measure_block(&block);
        let shaped_size = [measured.0, measured.1];
        let preferred = constraints.clamp_size(shaped_size);
        let ink_bounds = self
            .text
            .measure_block_ink(&block)
            .map(|(top, bottom)| Rect::new(0.0, top, shaped_size[0], (bottom - top).max(0.0)));
        let vm = self
            .text
            .vmetrics(block.font.as_ref(), block.weight, block.style);
        let baseline = (vm.baseline_ratio * block.font_size).clamp(0.0, shaped_size[1]);
        let visual_center =
            (vm.visual_center_ratio(&block.content) * block.font_size).clamp(0.0, shaped_size[1]);
        let line_height = block.line_height.max(f32::EPSILON);
        let line_count = (measured.1 / line_height).round().max(1.0) as usize;
        let overflowed_width = constrained_width.is_some_and(|width| natural.0 > width);
        let mut measurement = Measurement::new(
            [0.0, block.line_height],
            preferred,
            [None, constraints.max_height.or(Some(preferred[1]))],
            Some(baseline.min(preferred[1])),
        );
        if !block.ellipsize
            && self.wrap != WrapMode::None
            && let Some(width) = constrained_width
        {
            measurement = measurement.width_sensitive(width);
        }
        MeasuredText {
            block,
            allocation: preferred,
            metrics: TextMetrics {
                size: shaped_size,
                ink_bounds,
                baseline,
                visual_center,
                line_count,
                overflowed_width,
            },
            measurement,
        }
    }
}

/// Plain measured child declaration stored in reusable stack scratch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredChild {
    /// Stable layout identity.
    pub id: Option<NodeId>,
    /// Main-axis sizing policy.
    pub size: SizeSpec,
    /// Contextual intrinsic result.
    pub measurement: Measurement,
    /// Main-axis clamp.
    pub constraint: Constraint,
    /// Cross-axis alignment.
    pub align: CrossAlign,
    /// Fill weight.
    pub weight: f32,
}

impl MeasuredChild {
    /// A fit-content measured child.
    pub fn fit(measurement: Measurement) -> Self {
        Self {
            id: None,
            size: SizeSpec::Fit,
            measurement,
            constraint: Constraint::NONE,
            align: CrossAlign::Stretch,
            weight: 1.0,
        }
    }
    /// A fixed-main-axis measured child.
    pub fn fixed(main: f32, measurement: Measurement) -> Self {
        Self {
            size: SizeSpec::Fixed(main),
            ..Self::fit(measurement)
        }
    }
    /// A remaining-space measured child.
    pub fn fill(measurement: Measurement) -> Self {
        Self {
            size: SizeSpec::Fill,
            ..Self::fit(measurement)
        }
    }
    /// A parent-percentage measured child.
    pub fn percent(percent: f32, measurement: Measurement) -> Self {
        Self {
            size: SizeSpec::Percent(percent),
            ..Self::fit(measurement)
        }
    }
    /// Set stable identity.
    pub fn id(mut self, id: impl Into<NodeId>) -> Self {
        self.id = Some(id.into());
        self
    }
    /// Set cross-axis alignment.
    pub fn align(mut self, align: CrossAlign) -> Self {
        self.align = align;
        self
    }
    /// Set main-axis clamp.
    pub fn constrain(mut self, constraint: Constraint) -> Self {
        self.constraint = constraint;
        self
    }
    /// Set fill weight.
    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = weight.max(0.0);
        self
    }
}

/// Reusable flat storage for measured children.
#[derive(Default)]
pub struct MeasureBuffer {
    children: Vec<MeasuredChild>,
}

impl MeasureBuffer {
    /// Empty reusable buffer.
    pub fn new() -> Self {
        Self::default()
    }
    /// Remove declarations while retaining capacity.
    pub fn clear(&mut self) {
        self.children.clear();
    }
    /// Append one measured child.
    pub fn push(&mut self, child: MeasuredChild) {
        self.children.push(child);
    }
    /// Borrow all declarations.
    pub fn children(&self) -> &[MeasuredChild] {
        &self.children
    }
    /// Allocated child capacity, exposed for allocation-reuse diagnostics/tests.
    pub fn capacity(&self) -> usize {
        self.children.capacity()
    }

    /// Arrange as a vertical stack without invoking measurement again.
    pub fn arrange_vstack_into(
        &self,
        bounds: Rect,
        spacing: f32,
        padding: f32,
        main_align: MainAlign,
        out: &mut LayoutResult,
    ) -> Result<(), ArrangeError> {
        arrange(
            self.children(),
            Axis::Vertical,
            bounds,
            spacing,
            padding,
            main_align,
            out,
        )
    }

    /// Arrange as a horizontal stack, including baseline-aligned children.
    pub fn arrange_hstack_into(
        &self,
        bounds: Rect,
        spacing: f32,
        padding: f32,
        main_align: MainAlign,
        out: &mut LayoutResult,
    ) -> Result<(), ArrangeError> {
        arrange(
            self.children(),
            Axis::Horizontal,
            bounds,
            spacing,
            padding,
            main_align,
            out,
        )
    }
}

/// An arrangement could not honor a single-shot prepared measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrangeError {
    /// A width-sensitive child was assigned a width other than the one for which
    /// its height was prepared.
    WidthMismatch {
        /// Stable child identity, when supplied.
        id: Option<NodeId>,
        /// Width used by the single measurement operation.
        measured: f32,
        /// Width assigned by arrangement.
        arranged: f32,
    },
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

fn arrange(
    children: &[MeasuredChild],
    axis: Axis,
    bounds: Rect,
    spacing: f32,
    padding: f32,
    main_align: MainAlign,
    out: &mut LayoutResult,
) -> Result<(), ArrangeError> {
    let spacing = spacing.max(0.0);
    let padding = padding.max(0.0);
    let inner_main = match axis {
        Axis::Horizontal => bounds.width,
        Axis::Vertical => bounds.height,
    } - padding * 2.0;
    let inner_cross = match axis {
        Axis::Horizontal => bounds.height,
        Axis::Vertical => bounds.width,
    } - padding * 2.0;
    let inner_main = inner_main.max(0.0);
    let inner_cross = inner_cross.max(0.0);
    let gaps = spacing * children.len().saturating_sub(1) as f32;
    let mut fixed = gaps;
    let mut fill_weight = 0.0;
    for child in children {
        let preferred_main = match axis {
            Axis::Horizontal => child.measurement.preferred[0],
            Axis::Vertical => child.measurement.preferred[1],
        };
        match child.size {
            SizeSpec::Fixed(v) => fixed += v,
            SizeSpec::Percent(p) => fixed += inner_main * p,
            SizeSpec::Fill => fill_weight += child.weight,
            SizeSpec::Fit => fixed += preferred_main,
        }
    }
    let free = (inner_main - fixed).max(0.0);
    let per_weight = if fill_weight > 0.0 {
        free / fill_weight
    } else {
        0.0
    };
    let (leading, extra_gap) = if fill_weight == 0.0 {
        main_align.distribution(free, children.len())
    } else {
        (0.0, 0.0)
    };
    let baseline_target = children
        .iter()
        .filter(|c| c.align == CrossAlign::Baseline)
        .filter_map(|c| c.measurement.baseline)
        .fold(0.0, f32::max);

    // Validate every prepared width before touching caller output. An error must
    // leave the previous completed layout intact rather than exposing a partial
    // prefix of the new arrangement.
    for child in children {
        let preferred_main = match axis {
            Axis::Horizontal => child.measurement.preferred[0],
            Axis::Vertical => child.measurement.preferred[1],
        };
        let main_axis = match axis {
            Axis::Horizontal => 0,
            Axis::Vertical => 1,
        };
        let main = child.constraint.apply(measured_axis(
            child.measurement,
            main_axis,
            match child.size {
                SizeSpec::Fixed(v) => v,
                SizeSpec::Percent(p) => inner_main * p,
                SizeSpec::Fill => per_weight * child.weight,
                SizeSpec::Fit => preferred_main,
            },
        ));
        let preferred_cross = match axis {
            Axis::Horizontal => child.measurement.preferred[1],
            Axis::Vertical => child.measurement.preferred[0],
        };
        let cross_axis = match axis {
            Axis::Horizontal => 1,
            Axis::Vertical => 0,
        };
        let cross = measured_axis(
            child.measurement,
            cross_axis,
            match child.align {
                CrossAlign::Stretch => inner_cross,
                CrossAlign::Start | CrossAlign::Center | CrossAlign::End | CrossAlign::Baseline => {
                    preferred_cross.min(inner_cross)
                }
            },
        )
        .min(inner_cross);
        let arranged_width = match axis {
            Axis::Horizontal => main,
            Axis::Vertical => cross,
        };
        if let Some(measured_width) = child.measurement.measured_width
            && (measured_width - arranged_width).abs() > 0.01
        {
            return Err(ArrangeError::WidthMismatch {
                id: child.id,
                measured: measured_width,
                arranged: arranged_width,
            });
        }
    }

    out.begin_arrangement(bounds);
    let mut cursor = padding + leading;
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            cursor += spacing + extra_gap;
        }
        let preferred = child.measurement.preferred;
        let preferred_main = match axis {
            Axis::Horizontal => preferred[0],
            Axis::Vertical => preferred[1],
        };
        let main_axis = match axis {
            Axis::Horizontal => 0,
            Axis::Vertical => 1,
        };
        let main = child.constraint.apply(measured_axis(
            child.measurement,
            main_axis,
            match child.size {
                SizeSpec::Fixed(v) => v,
                SizeSpec::Percent(p) => inner_main * p,
                SizeSpec::Fill => per_weight * child.weight,
                SizeSpec::Fit => preferred_main,
            },
        ));
        let preferred_cross = match axis {
            Axis::Horizontal => preferred[1],
            Axis::Vertical => preferred[0],
        };
        let (cross_offset, cross) = match child.align {
            CrossAlign::Stretch => {
                let axis = match axis {
                    Axis::Horizontal => 1,
                    Axis::Vertical => 0,
                };
                (
                    0.0,
                    measured_axis(child.measurement, axis, inner_cross).min(inner_cross),
                )
            }
            CrossAlign::Start => {
                let axis = match axis {
                    Axis::Horizontal => 1,
                    Axis::Vertical => 0,
                };
                (
                    0.0,
                    measured_axis(child.measurement, axis, preferred_cross).min(inner_cross),
                )
            }
            CrossAlign::Center => {
                let axis = match axis {
                    Axis::Horizontal => 1,
                    Axis::Vertical => 0,
                };
                let cross =
                    measured_axis(child.measurement, axis, preferred_cross).min(inner_cross);
                ((inner_cross - cross) * 0.5, cross)
            }
            CrossAlign::End => {
                let axis = match axis {
                    Axis::Horizontal => 1,
                    Axis::Vertical => 0,
                };
                let cross =
                    measured_axis(child.measurement, axis, preferred_cross).min(inner_cross);
                (inner_cross - cross, cross)
            }
            CrossAlign::Baseline => match (axis, child.measurement.baseline) {
                (Axis::Horizontal, Some(own)) => {
                    let cross =
                        measured_axis(child.measurement, 1, preferred_cross).min(inner_cross);
                    (baseline_target - own, cross)
                }
                (Axis::Horizontal, None) => {
                    let cross =
                        measured_axis(child.measurement, 1, preferred_cross).min(inner_cross);
                    ((inner_cross - cross) * 0.5, cross)
                }
                (Axis::Vertical, _) => {
                    let cross =
                        measured_axis(child.measurement, 0, preferred_cross).min(inner_cross);
                    (0.0, cross)
                }
            },
        };
        let rect = match axis {
            Axis::Horizontal => Rect::new(
                bounds.x + cursor,
                bounds.y + padding + cross_offset,
                main,
                cross,
            ),
            Axis::Vertical => Rect::new(
                bounds.x + padding + cross_offset,
                bounds.y + cursor,
                cross,
                main,
            ),
        };
        out.push_arranged(child.id, rect);
        cursor += main;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(w: f32, h: f32, baseline: Option<f32>) -> Measurement {
        Measurement::new([0.0, 0.0], [w, h], [None, None], baseline)
    }

    #[test]
    fn constraints_and_measurements_normalize() {
        let c = MeasureConstraints {
            min_width: 20.0,
            max_width: Some(10.0),
            min_height: f32::NAN,
            max_height: Some(-2.0),
        }
        .normalized();
        assert_eq!(c.max_width, Some(20.0));
        assert_eq!(c.min_height, 0.0);
        assert_eq!(c.max_height, Some(0.0));
        let m = Measurement::new(
            [10.0, 4.0],
            [2.0, 100.0],
            [Some(8.0), Some(20.0)],
            Some(99.0),
        );
        assert_eq!(m.preferred, [10.0, 20.0]);
        assert_eq!(m.baseline, Some(20.0));
    }

    #[test]
    fn unbounded_wrapped_text_is_not_capped_at_text_block_default() {
        let theme = crate::Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut text = TextMeasurer::new();
        let mut cx = MeasureContext::new(
            &mut text,
            styles,
            FontSpec::default(),
            MeasureConstraints::UNBOUNDED,
            1.0,
            WrapMode::Word,
        );
        let content = "word ".repeat(100);
        let measured = cx.measure_text(cx.text_block(content));
        assert_eq!(measured.metrics.line_count, 1);
        assert!(measured.metrics.size[0] > 800.0);
        assert_eq!(measured.measurement.measured_width, None);
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn constrained_height_clips_prepared_paint_but_preserves_shaped_metrics() {
        let theme = crate::Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut text = TextMeasurer::new();
        let constraints = MeasureConstraints {
            max_width: Some(50.0),
            max_height: Some(20.0),
            ..MeasureConstraints::UNBOUNDED
        };
        let mut cx = MeasureContext::new(
            &mut text,
            styles,
            FontSpec::default(),
            constraints,
            1.0,
            WrapMode::Word,
        );
        let measured = cx.measure_text(cx.text_block("one two three four"));
        assert!(measured.metrics.size[1] > measured.measurement.preferred[1]);
        // The clip rect is the layout allocation, whose width is derived from
        // shaped line advances; those drift slightly across shaper/font
        // dependency versions, so assert the relationship, not a magic float.
        let allocated_width = measured.measurement.preferred[0];
        let block = measured.into_block_at(5.0, 7.0);
        let clip = block.clip.expect("constrained text must carry a clip rect");
        assert_eq!((clip.x, clip.y, clip.height), (5.0, 7.0, 20.0));
        assert_eq!(clip.width, allocated_width);
        assert!(clip.width < 50.0, "clip must fit the max_width budget");
        let clip_width = clip.width;
        assert!(
            (clip_width - 40.0).abs() < 1.0,
            "clip width should track the widest wrapped line, got {clip_width}"
        );
    }

    #[test]
    fn text_measurement_is_structured_and_consumes_into_paint_block() {
        let theme = crate::Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut text = TextMeasurer::new();
        let mut cx = MeasureContext::new(
            &mut text,
            styles,
            FontSpec::default(),
            MeasureConstraints::with_max_width(55.0),
            2.0,
            WrapMode::Word,
        );
        let block = cx.text_block("one two three");
        let measured = cx.measure_text(block);
        assert!(measured.metrics.line_count > 1);
        assert!(measured.metrics.overflowed_width);
        assert!(measured.metrics.ink_bounds.is_some());
        assert!(measured.measurement.is_width_sensitive());
        assert_eq!(measured.block().max_width, 55.0);
        let painted = measured.into_block_at(12.0, 34.0);
        assert_eq!((painted.x, painted.y), (12.0, 34.0));
        assert_eq!(painted.content, "one two three");
    }

    #[test]
    fn vertical_stack_accepts_text_prepared_for_its_inner_width() {
        let mut b = MeasureBuffer::new();
        b.push(MeasuredChild::fit(
            m(76.0, 40.0, Some(14.0)).width_sensitive(76.0),
        ));
        let mut out = LayoutResult::default();
        b.arrange_vstack_into(
            Rect::new(0.0, 0.0, 80.0, 100.0),
            0.0,
            2.0,
            MainAlign::Start,
            &mut out,
        )
        .unwrap();
        assert_eq!(out.get(1), Rect::new(2.0, 2.0, 76.0, 40.0));
    }

    #[test]
    fn horizontal_baselines_align() {
        let mut b = MeasureBuffer::new();
        b.push(MeasuredChild::fit(m(20.0, 20.0, Some(15.0))).align(CrossAlign::Baseline));
        b.push(MeasuredChild::fit(m(30.0, 12.0, Some(8.0))).align(CrossAlign::Baseline));
        let mut out = LayoutResult::default();
        b.arrange_hstack_into(
            Rect::new(0.0, 0.0, 100.0, 30.0),
            4.0,
            0.0,
            MainAlign::Start,
            &mut out,
        )
        .unwrap();
        assert_eq!(out.get(1).y + 15.0, out.get(2).y + 8.0);
        assert_eq!(out.get(1).height, 20.0);
        assert_eq!(out.get(2).height, 12.0);
    }

    #[test]
    fn missing_baseline_falls_back_inside_the_row() {
        let mut b = MeasureBuffer::new();
        b.push(MeasuredChild::fit(m(20.0, 12.0, Some(8.0))).align(CrossAlign::Baseline));
        b.push(MeasuredChild::fit(m(20.0, 20.0, None)).align(CrossAlign::Baseline));
        let mut out = LayoutResult::default();
        b.arrange_hstack_into(
            Rect::new(0.0, 10.0, 100.0, 30.0),
            4.0,
            0.0,
            MainAlign::Start,
            &mut out,
        )
        .unwrap();
        assert!(out.get(2).y >= 10.0);
        assert!(out.get(2).bottom() <= 40.0);
    }

    #[test]
    fn width_sensitive_mismatch_is_explicit() {
        let mut b = MeasureBuffer::new();
        b.push(MeasuredChild::fill(m(20.0, 40.0, Some(10.0)).width_sensitive(20.0)).id(7));
        let mut out = LayoutResult::default();
        assert_eq!(
            b.arrange_hstack_into(
                Rect::new(0.0, 0.0, 80.0, 40.0),
                0.0,
                0.0,
                MainAlign::Start,
                &mut out
            ),
            Err(ArrangeError::WidthMismatch {
                id: Some(NodeId(7)),
                measured: 20.0,
                arranged: 80.0
            })
        );
    }

    #[test]
    fn arrangement_error_leaves_previous_output_untouched() {
        let mut b = MeasureBuffer::new();
        b.push(MeasuredChild::fill(
            m(20.0, 40.0, Some(10.0)).width_sensitive(20.0),
        ));
        let mut out = LayoutResult::single(Rect::new(1.0, 2.0, 3.0, 4.0));
        let before = out.items().to_vec();
        assert!(
            b.arrange_hstack_into(
                Rect::new(0.0, 0.0, 80.0, 40.0),
                0.0,
                0.0,
                MainAlign::Start,
                &mut out,
            )
            .is_err()
        );
        assert_eq!(out.items(), before);
    }

    #[test]
    fn measure_buffer_reuses_capacity() {
        let mut b = MeasureBuffer::new();
        for _ in 0..32 {
            b.push(MeasuredChild::fit(m(1.0, 1.0, None)));
        }
        let cap = b.capacity();
        b.clear();
        for _ in 0..32 {
            b.push(MeasuredChild::fit(m(1.0, 1.0, None)));
        }
        assert_eq!(b.capacity(), cap);
    }
}
