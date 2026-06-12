//! Tree view / collapsing header.
//!
//! A tree is a vertical list of rows, each one row tall, where *branch* rows
//! carry a disclosure triangle and can be expanded to reveal indented children,
//! and *leaf* rows are terminal. Selection is single-owner (at most one selected
//! node), mirroring the rest of the crate's caller-owned-state pattern
//! ([`crate::DropdownState`] / [`crate::FocusState`]).
//!
//! ## Two ways to use it
//!
//! **Façade (recommended).** The [`crate::UiContext`] verbs
//! [`tree_node`](crate::UiContext::tree_node) /
//! [`tree_leaf`](crate::UiContext::tree_leaf) /
//! [`tree_pop`](crate::UiContext::tree_pop) compose like egui's collapsing
//! headers — indentation, row height, and the auto-advancing layout cursor are
//! handled for you:
//!
//! ```ignore
//! if ui.tree_node(1, "Fruit") {
//!     let _ = ui.tree_leaf(2, "Apple");
//!     let _ = ui.tree_leaf(3, "Banana");
//!     ui.tree_pop();
//! }
//! ```
//!
//! **Raw widget.** [`TreeNode::draw`] renders a single row into an explicit
//! `Rect` against a [`DrawContext`], taking the indentation depth via
//! [`TreeNode::with_depth`]. The caller drives the recursion and the vertical
//! cursor. This is what the façade is built on.
//!
//! ## State
//! [`TreeState`] holds the expanded set + the selected node. A node's expanded
//! state is keyed by a caller-supplied [`TreeId`] (any per-tree-unique `u64`);
//! [`TreeNode::with_default_open`] sets the state the *first* time a given id is
//! seen, so a tree can start partly unfurled without the caller pre-seeding it.

use std::collections::HashSet;

use crate::layout::Rect;
use crate::text::TextBlock;

use super::{DrawContext, DrawList};

/// Stable identity for a node within one tree. Any scheme unique per node per
/// frame works (a hash, an enum discriminant, a stable row index). `0` is a
/// valid id.
pub type TreeId = u64;

/// Indentation added per depth level, in pixels.
const INDENT: f32 = 14.0;
/// Half-extent of the disclosure triangle, in pixels.
const ARROW: f32 = 4.0;
/// Horizontal gap between the disclosure column and the label.
const ARROW_GAP: f32 = 6.0;
/// Left inset before the disclosure triangle within a row.
const ROW_INSET: f32 = 4.0;

fn rgb(c: [f32; 4]) -> (u8, u8, u8) {
    (
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

/// Caller-owned expansion + selection state for one tree view. Persists across
/// frames; construct one per tree (or share it across trees whose ids don't
/// collide) and thread `&mut` into each node draw, the same way the crate
/// threads [`crate::FocusState`] / [`crate::DropdownState`].
#[derive(Debug, Default, Clone)]
pub struct TreeState {
    /// Ids of currently-expanded branch nodes.
    expanded: HashSet<TreeId>,
    /// Ids seen at least once, so `default_open` is applied exactly once per id.
    seen: HashSet<TreeId>,
    /// The single selected node, if any.
    selected: Option<TreeId>,
}

impl TreeState {
    /// A fresh state: nothing expanded, nothing selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when `id` is currently expanded.
    pub fn is_expanded(&self, id: TreeId) -> bool {
        self.expanded.contains(&id)
    }

    /// Force `id`'s expanded state. Also marks it seen, so a later
    /// `default_open` won't override this choice.
    pub fn set_expanded(&mut self, id: TreeId, expanded: bool) {
        self.seen.insert(id);
        if expanded {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
    }

    /// Flip `id`'s expanded state.
    pub fn toggle(&mut self, id: TreeId) {
        self.seen.insert(id);
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
    }

    /// Collapse every node (selection is preserved).
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    /// The selected node, if any.
    pub fn selected(&self) -> Option<TreeId> {
        self.selected
    }

    /// True when `id` is the selected node.
    pub fn is_selected(&self, id: TreeId) -> bool {
        self.selected == Some(id)
    }

    /// Select `id` (replaces any prior selection).
    pub fn select(&mut self, id: TreeId) {
        self.selected = Some(id);
    }

    /// Clear the selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Resolve `id`'s expanded state, applying `default_open` the first time the
    /// id is ever seen. Returns the resulting expanded flag.
    fn resolve_expanded(&mut self, id: TreeId, default_open: bool) -> bool {
        if self.seen.insert(id) && default_open {
            self.expanded.insert(id);
        }
        self.expanded.contains(&id)
    }
}

/// Per-frame configuration for one tree row. Lightweight, built fresh each
/// frame like [`crate::Slider`] / [`crate::Dropdown`].
pub struct TreeNode<'a> {
    label: &'a str,
    leaf: bool,
    default_open: bool,
    depth: usize,
}

/// Result of drawing one tree row.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeNodeOutput {
    /// Whether the node is expanded *after* this draw — i.e. whether the caller
    /// should render its children. Always `false` for a leaf.
    pub expanded: bool,
    /// The disclosure was toggled this frame (branch only).
    pub toggled: bool,
    /// The row was clicked this frame (it became the selection).
    pub clicked: bool,
}

impl<'a> TreeNode<'a> {
    /// A branch node displaying `label`, with a disclosure triangle.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            leaf: false,
            default_open: false,
            depth: 0,
        }
    }

    /// A leaf node: no disclosure triangle, never expands. Clicking selects it.
    pub fn leaf(label: &'a str) -> Self {
        Self {
            label,
            leaf: true,
            default_open: false,
            depth: 0,
        }
    }

    /// Set whether this is a leaf (no disclosure, terminal).
    pub fn with_leaf(mut self, leaf: bool) -> Self {
        self.leaf = leaf;
        self
    }

    /// Start expanded the first time this id is seen (branch only). Ignored on
    /// subsequent frames — the state then lives in [`TreeState`].
    pub fn with_default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// Indentation depth (0 = root). Used by the raw-`Rect` draw path; the
    /// [`crate::UiContext`] façade sets this from its own depth counter.
    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// Draw this row into `rect` and handle expand/select interaction. Returns
    /// the post-draw [`TreeNodeOutput`]. The row's content is indented by
    /// `depth * INDENT`; the selection/hover highlight spans the full `rect`
    /// width regardless of depth.
    pub fn draw(
        &self,
        id: TreeId,
        rect: Rect,
        state: &mut TreeState,
        ctx: &mut DrawContext,
    ) -> TreeNodeOutput {
        let theme = ctx.theme;
        let input = ctx.input;

        let expanded_before = state.resolve_expanded(id, self.default_open);

        // Honor layer capture so a tree under a modal/popup ignores clicks meant
        // for the overlay.
        let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let clicked = hovered && input.mouse_clicked;
        let selected = state.is_selected(id);

        let list = &mut *ctx.draw_list;

        // Full-row highlight: selection wins over hover.
        if selected {
            list.quad(rect.x, rect.y, rect.width, rect.height, theme.accent);
        } else if hovered {
            list.quad(rect.x, rect.y, rect.width, rect.height, theme.button_hover);
        }

        let indent = self.depth as f32 * INDENT;
        // Disclosure column origin (also occupied — but empty — for leaves, so
        // sibling labels line up whether or not they have children).
        let arrow_cx = rect.x + ROW_INSET + indent + ARROW;
        let arrow_cy = rect.y + rect.height * 0.5;

        // Text colour: contrast against the accent fill when selected.
        let text_color = if selected {
            theme.background
        } else {
            theme.text
        };
        let arrow_color = if selected {
            theme.background
        } else {
            theme.text_dim
        };

        if !self.leaf {
            draw_disclosure(list, arrow_cx, arrow_cy, expanded_before, arrow_color);
        }

        let text_x = arrow_cx + ARROW + ARROW_GAP;
        let text_y = rect.y + (rect.height - theme.font_size) * 0.5;
        let (r, g, b) = rgb(text_color);
        let max_w = (rect.x + rect.width - text_x - ROW_INSET).max(0.0);
        list.text(
            TextBlock::new(self.label, text_x, text_y)
                .with_size(theme.font_size)
                .with_color(r, g, b)
                .with_max_width(max_w)
                .with_ellipsis()
                .with_font_opt(theme.font.clone()),
        );

        // Interaction: a click anywhere on the row selects it, and additionally
        // toggles expansion for a branch.
        let mut toggled = false;
        if clicked {
            state.select(id);
            if !self.leaf {
                state.toggle(id);
                toggled = true;
            }
        }

        let expanded = !self.leaf && state.is_expanded(id);
        TreeNodeOutput {
            expanded,
            toggled,
            clicked,
        }
    }
}

/// Draw the disclosure triangle centred at `(cx, cy)`: pointing right when
/// collapsed, down when expanded. Filled, sized to `ARROW`.
fn draw_disclosure(list: &mut DrawList, cx: f32, cy: f32, expanded: bool, color: [f32; 4]) {
    if expanded {
        // ▼ — apex down.
        list.triangle(
            (cx - ARROW, cy - ARROW * 0.6),
            (cx + ARROW, cy - ARROW * 0.6),
            (cx, cy + ARROW * 0.7),
            color,
        );
    } else {
        // ▶ — apex right.
        list.triangle(
            (cx - ARROW * 0.6, cy - ARROW),
            (cx - ARROW * 0.6, cy + ARROW),
            (cx + ARROW * 0.7, cy),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    fn theme() -> Theme {
        Theme::default()
    }

    fn row() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 20.0)
    }

    /// Draw one node into a fresh context; return the populated list + output.
    fn draw_node(
        node: &TreeNode,
        id: TreeId,
        rect: Rect,
        state: &mut TreeState,
        input: &InputState,
    ) -> (DrawList, TreeNodeOutput) {
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let th = theme();
        let out = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &th, input, 800.0, 600.0);
            node.draw(id, rect, state, &mut ctx)
        };
        (list, out)
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..InputState::default()
        }
    }

    #[test]
    fn fresh_state_is_empty() {
        let s = TreeState::new();
        assert!(!s.is_expanded(1));
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn default_open_applies_once_then_state_owns_it() {
        let mut s = TreeState::new();
        let node = TreeNode::new("root").with_default_open(true);
        // First draw: default_open expands it.
        let (_, out) = draw_node(&node, 1, row(), &mut s, &idle());
        assert!(out.expanded, "default_open expands on first sight");
        assert!(s.is_expanded(1));
        // Collapse it, then redraw: default_open must NOT re-expand.
        s.set_expanded(1, false);
        let (_, out) = draw_node(&node, 1, row(), &mut s, &idle());
        assert!(!out.expanded, "default_open is one-shot; state now owns it");
    }

    #[test]
    fn clicking_branch_toggles_and_selects() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch");
        // Click inside the row.
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(50.0, 10.0));
        assert!(out.clicked);
        assert!(out.toggled, "branch toggles on click");
        assert!(out.expanded, "was collapsed → now expanded");
        assert!(s.is_selected(1), "click selects the row");

        // Click again → collapses (still selected).
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(50.0, 10.0));
        assert!(out.toggled);
        assert!(!out.expanded, "second click collapses");
        assert!(s.is_selected(1));
    }

    #[test]
    fn clicking_leaf_selects_but_never_expands() {
        let mut s = TreeState::new();
        let node = TreeNode::leaf("leaf");
        let (_, out) = draw_node(&node, 7, row(), &mut s, &click_at(50.0, 10.0));
        assert!(out.clicked);
        assert!(!out.toggled, "leaf never toggles");
        assert!(!out.expanded, "leaf never expands");
        assert!(s.is_selected(7));
        assert!(!s.is_expanded(7));
    }

    #[test]
    fn click_outside_row_does_nothing() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch");
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(500.0, 500.0));
        assert!(!out.clicked);
        assert!(!out.toggled);
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn consumed_mouse_does_not_interact() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch");
        let mut inp = click_at(50.0, 10.0);
        inp.mouse_consumed = true; // a higher layer took this click
        let (_, out) = draw_node(&node, 1, row(), &mut s, &inp);
        assert!(!out.clicked);
        assert!(!out.toggled);
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn selection_is_single_owner() {
        let mut s = TreeState::new();
        s.select(1);
        assert!(s.is_selected(1));
        s.select(2);
        assert!(s.is_selected(2));
        assert!(!s.is_selected(1), "selecting 2 replaces 1");
        s.clear_selection();
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn toggle_and_set_expanded_round_trip() {
        let mut s = TreeState::new();
        assert!(!s.is_expanded(3));
        s.toggle(3);
        assert!(s.is_expanded(3));
        s.toggle(3);
        assert!(!s.is_expanded(3));
        s.set_expanded(3, true);
        assert!(s.is_expanded(3));
        s.collapse_all();
        assert!(!s.is_expanded(3));
    }

    #[test]
    fn deeper_depth_indents_label_further() {
        // The label x-origin must grow with depth: render the same label at two
        // depths and compare the emitted TextBlock's x.
        fn label_x(depth: usize) -> f32 {
            let mut s = TreeState::new();
            let node = TreeNode::leaf("Node").with_depth(depth);
            let (list, _) = draw_node(&node, 1, row(), &mut s, &idle());
            list.texts.first().expect("label text emitted").x
        }
        let x0 = label_x(0);
        let x2 = label_x(2);
        assert!(
            x2 > x0 + INDENT,
            "depth 2 should indent the label by ~2*INDENT (x0={x0}, x2={x2})"
        );
    }

    #[test]
    fn branch_emits_disclosure_triangle_leaf_does_not() {
        let mut s = TreeState::new();
        // Branch: a filled disclosure triangle is soup geometry (3 verts).
        let (branch_list, _) = draw_node(&TreeNode::new("b"), 1, row(), &mut s, &idle());
        let mut s2 = TreeState::new();
        let (leaf_list, _) = draw_node(&TreeNode::leaf("l"), 2, row(), &mut s2, &idle());
        assert!(
            branch_list.vertices.len() > leaf_list.vertices.len(),
            "branch adds disclosure-triangle geometry the leaf lacks"
        );
    }
}
