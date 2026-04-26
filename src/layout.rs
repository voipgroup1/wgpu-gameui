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
//!         .child(Button::new("▲", ...))
//!         .child(Label::new("Ground"))
//!         .child(Button::new("▼", ...))
//! );
//!
//! let rects = floor_ui.layout(screen_width, screen_height);
//! // Draw using computed rects...
//! ```

/// Computed rectangle after layout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

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
    TopLeft { offset: (f32, f32) },
    /// Top-right corner with offset (x, y) from corner. X is typically negative.
    TopRight { offset: (f32, f32) },
    /// Bottom-left corner with offset (x, y) from corner. Y is typically negative.
    BottomLeft { offset: (f32, f32) },
    /// Bottom-right corner with offset (x, y) from corner. Both typically negative.
    BottomRight { offset: (f32, f32) },
    /// Centered in parent with offset (x, y) from center.
    Center { offset: (f32, f32) },
    /// Centered horizontally, at top with offset.
    TopCenter { offset: (f32, f32) },
    /// Centered horizontally, at bottom with offset.
    BottomCenter { offset: (f32, f32) },
    /// Centered vertically, at left with offset.
    LeftCenter { offset: (f32, f32) },
    /// Centered vertically, at right with offset.
    RightCenter { offset: (f32, f32) },
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
            Anchor::TopRight { offset } => {
                (parent.x + parent.width - width + offset.0, parent.y + offset.1)
            }
            Anchor::BottomLeft { offset } => {
                (parent.x + offset.0, parent.y + parent.height - height + offset.1)
            }
            Anchor::BottomRight { offset } => (
                parent.x + parent.width - width + offset.0,
                parent.y + parent.height - height + offset.1,
            ),
            Anchor::Center { offset } => (
                parent.x + (parent.width - width) / 2.0 + offset.0,
                parent.y + (parent.height - height) / 2.0 + offset.1,
            ),
            Anchor::TopCenter { offset } => {
                (parent.x + (parent.width - width) / 2.0 + offset.0, parent.y + offset.1)
            }
            Anchor::BottomCenter { offset } => (
                parent.x + (parent.width - width) / 2.0 + offset.0,
                parent.y + parent.height - height + offset.1,
            ),
            Anchor::LeftCenter { offset } => {
                (parent.x + offset.0, parent.y + (parent.height - height) / 2.0 + offset.1)
            }
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

/// Size for both dimensions.
#[derive(Debug, Clone, Copy, Default)]
pub struct Size {
    pub width: SizeSpec,
    pub height: SizeSpec,
}

impl Size {
    pub fn fixed(width: f32, height: f32) -> Self {
        Self {
            width: SizeSpec::Fixed(width),
            height: SizeSpec::Fixed(height),
        }
    }

    pub fn percent(width: f32, height: f32) -> Self {
        Self {
            width: SizeSpec::Percent(width),
            height: SizeSpec::Percent(height),
        }
    }

    pub fn fill() -> Self {
        Self {
            width: SizeSpec::Fill,
            height: SizeSpec::Fill,
        }
    }

    pub fn fit() -> Self {
        Self {
            width: SizeSpec::Fit,
            height: SizeSpec::Fit,
        }
    }

    pub fn width_fixed(mut self, width: f32) -> Self {
        self.width = SizeSpec::Fixed(width);
        self
    }

    pub fn height_fixed(mut self, height: f32) -> Self {
        self.height = SizeSpec::Fixed(height);
        self
    }
}

/// A layout node that can be positioned and sized.
pub trait LayoutNode {
    /// Compute minimum content size (for Fit sizing).
    fn content_size(&self) -> (f32, f32);

    /// Layout this node within the given bounds.
    /// Returns the computed rectangles for this node and all children.
    fn layout(&self, bounds: Rect) -> LayoutResult;
}

/// Result of layout computation.
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    /// Rectangles in order of layout tree traversal.
    /// Index 0 is always the container itself.
    pub rects: Vec<Rect>,
}

impl LayoutResult {
    pub fn single(rect: Rect) -> Self {
        Self { rects: vec![rect] }
    }

    pub fn get(&self, index: usize) -> Rect {
        self.rects.get(index).copied().unwrap_or_default()
    }
}

/// A single positioned element.
pub struct Positioned<T> {
    pub anchor: Anchor,
    pub size: Size,
    pub child: T,
}

impl<T> Positioned<T> {
    pub fn new(anchor: Anchor, size: Size, child: T) -> Self {
        Self { anchor, size, child }
    }

    /// Layout starting from screen coordinates.
    pub fn layout_screen(&self, screen_width: f32, screen_height: f32) -> LayoutResult
    where
        T: LayoutNode,
    {
        let screen = Rect::new(0.0, 0.0, screen_width, screen_height);
        self.layout(screen)
    }
}

impl<T: LayoutNode> LayoutNode for Positioned<T> {
    fn content_size(&self) -> (f32, f32) {
        self.child.content_size()
    }

    fn layout(&self, parent: Rect) -> LayoutResult {
        let (content_w, content_h) = self.child.content_size();
        let width = self.size.width.resolve(parent.width, content_w);
        let height = self.size.height.resolve(parent.height, content_h);
        let (x, y) = self.anchor.resolve(parent, width, height);

        let bounds = Rect::new(x, y, width, height);
        self.child.layout(bounds)
    }
}

/// Vertical stack - children arranged top to bottom.
pub struct VStack {
    pub spacing: f32,
    pub padding: f32,
    pub children: Vec<StackChild>,
}

/// Horizontal stack - children arranged left to right.
pub struct HStack {
    pub spacing: f32,
    pub padding: f32,
    pub children: Vec<StackChild>,
}

/// A child in a stack with its sizing.
pub struct StackChild {
    pub size: SizeSpec,
    pub content_size: f32, // Size along stack axis
    pub cross_size: f32,   // Size perpendicular to stack axis
}

impl VStack {
    pub fn new(spacing: f32) -> Self {
        Self {
            spacing,
            padding: 0.0,
            children: Vec::new(),
        }
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Add a child with fixed height.
    pub fn child(mut self, height: f32, width: f32) -> Self {
        self.children.push(StackChild {
            size: SizeSpec::Fixed(height),
            content_size: height,
            cross_size: width,
        });
        self
    }

    /// Add a child that fills remaining space.
    pub fn child_fill(mut self, width: f32) -> Self {
        self.children.push(StackChild {
            size: SizeSpec::Fill,
            content_size: 0.0,
            cross_size: width,
        });
        self
    }

    /// Add a child with percentage height.
    pub fn child_percent(mut self, percent: f32, width: f32) -> Self {
        self.children.push(StackChild {
            size: SizeSpec::Percent(percent),
            content_size: 0.0,
            cross_size: width,
        });
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

    fn layout(&self, bounds: Rect) -> LayoutResult {
        let mut rects = vec![bounds];
        let inner_width = bounds.width - self.padding * 2.0;
        let inner_height = bounds.height - self.padding * 2.0;

        // Calculate total fixed height and count fill children
        let mut fixed_height = 0.0;
        let mut fill_count = 0;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                fixed_height += self.spacing;
            }
            match child.size {
                SizeSpec::Fixed(h) => fixed_height += h,
                SizeSpec::Percent(p) => fixed_height += inner_height * p,
                SizeSpec::Fill => fill_count += 1,
                SizeSpec::Fit => fixed_height += child.content_size,
            }
        }

        // Distribute remaining space to fill children
        let remaining = (inner_height - fixed_height).max(0.0);
        let fill_height = if fill_count > 0 {
            remaining / fill_count as f32
        } else {
            0.0
        };

        // Position children
        let mut y = bounds.y + self.padding;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                y += self.spacing;
            }

            let height = match child.size {
                SizeSpec::Fixed(h) => h,
                SizeSpec::Percent(p) => inner_height * p,
                SizeSpec::Fill => fill_height,
                SizeSpec::Fit => child.content_size,
            };

            rects.push(Rect::new(bounds.x + self.padding, y, inner_width, height));
            y += height;
        }

        LayoutResult { rects }
    }
}

impl HStack {
    pub fn new(spacing: f32) -> Self {
        Self {
            spacing,
            padding: 0.0,
            children: Vec::new(),
        }
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Add a child with fixed width.
    pub fn child(mut self, width: f32, height: f32) -> Self {
        self.children.push(StackChild {
            size: SizeSpec::Fixed(width),
            content_size: width,
            cross_size: height,
        });
        self
    }

    /// Add a child that fills remaining space.
    pub fn child_fill(mut self, height: f32) -> Self {
        self.children.push(StackChild {
            size: SizeSpec::Fill,
            content_size: 0.0,
            cross_size: height,
        });
        self
    }

    /// Add a child with percentage width.
    pub fn child_percent(mut self, percent: f32, height: f32) -> Self {
        self.children.push(StackChild {
            size: SizeSpec::Percent(percent),
            content_size: 0.0,
            cross_size: height,
        });
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

    fn layout(&self, bounds: Rect) -> LayoutResult {
        let mut rects = vec![bounds];
        let inner_width = bounds.width - self.padding * 2.0;
        let inner_height = bounds.height - self.padding * 2.0;

        // Calculate total fixed width and count fill children
        let mut fixed_width = 0.0;
        let mut fill_count = 0;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                fixed_width += self.spacing;
            }
            match child.size {
                SizeSpec::Fixed(w) => fixed_width += w,
                SizeSpec::Percent(p) => fixed_width += inner_width * p,
                SizeSpec::Fill => fill_count += 1,
                SizeSpec::Fit => fixed_width += child.content_size,
            }
        }

        // Distribute remaining space to fill children
        let remaining = (inner_width - fixed_width).max(0.0);
        let fill_width = if fill_count > 0 {
            remaining / fill_count as f32
        } else {
            0.0
        };

        // Position children
        let mut x = bounds.x + self.padding;
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                x += self.spacing;
            }

            let width = match child.size {
                SizeSpec::Fixed(w) => w,
                SizeSpec::Percent(p) => inner_width * p,
                SizeSpec::Fill => fill_width,
                SizeSpec::Fit => child.content_size,
            };

            rects.push(Rect::new(x, bounds.y + self.padding, width, inner_height));
            x += width;
        }

        LayoutResult { rects }
    }
}

/// A simple leaf node with fixed size.
pub struct Leaf {
    pub width: f32,
    pub height: f32,
}

impl Leaf {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl LayoutNode for Leaf {
    fn content_size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    fn layout(&self, bounds: Rect) -> LayoutResult {
        LayoutResult::single(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anchor_top_right() {
        let screen = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let anchor = Anchor::TopRight { offset: (-10.0, 10.0) };
        let (x, y) = anchor.resolve(screen, 80.0, 120.0);
        assert_eq!(x, 1280.0 - 80.0 - 10.0); // 1190
        assert_eq!(y, 10.0);
    }

    #[test]
    fn test_vstack_layout() {
        let stack = VStack::new(8.0)
            .with_padding(10.0)
            .child(30.0, 60.0)  // button
            .child(20.0, 60.0)  // label
            .child(30.0, 60.0); // button

        let bounds = Rect::new(100.0, 50.0, 80.0, 150.0);
        let result = stack.layout(bounds);

        // Container rect
        assert_eq!(result.rects[0], bounds);

        // First child: y = 50 + 10 padding = 60
        assert_eq!(result.rects[1].y, 60.0);
        assert_eq!(result.rects[1].height, 30.0);

        // Second child: y = 60 + 30 + 8 spacing = 98
        assert_eq!(result.rects[2].y, 98.0);
        assert_eq!(result.rects[2].height, 20.0);

        // Third child: y = 98 + 20 + 8 spacing = 126
        assert_eq!(result.rects[3].y, 126.0);
        assert_eq!(result.rects[3].height, 30.0);
    }

    #[test]
    fn test_positioned_top_right() {
        let layout = Positioned::new(
            Anchor::TopRight { offset: (-10.0, 10.0) },
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
            Anchor::TopRight { offset: (-10.0, 10.0) },
            Size::fixed(80.0, 110.0),
            VStack::new(5.0)
                .with_padding(5.0)
                .child(30.0, 70.0)  // up button
                .child(24.0, 70.0)  // floor label
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
        assert_eq!(up_btn.y, 10.0 + 5.0);   // container.y + padding
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
}
