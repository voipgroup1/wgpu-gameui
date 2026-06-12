//! Tree view / collapsing header.
//!
//! A tree is a vertical list of rows, each one row tall. A *branch* row carries
//! a disclosure triangle and can be expanded to reveal indented children; a
//! *leaf* row is terminal. Each row can also host **action icons**: a leading
//! column before the label (e.g. a visibility toggle) and a right-aligned
//! trailing column (e.g. rename / delete) — the classic scene/layer-outliner
//! shape. Each action icon is its own hit target: clicking it returns its
//! caller-defined id and does **not** select or expand the node.
//!
//! Selection is single-owner (at most one selected node), mirroring the rest of
//! the crate's caller-owned-state pattern ([`crate::DropdownState`] /
//! [`crate::FocusState`]).
//!
//! ## Interaction model
//! - Clicking the **disclosure triangle** expands/collapses a branch.
//! - Clicking the **label / row body** selects the node (and *also* toggles a
//!   branch only if [`TreeNode::with_toggle_on_label`] is set — the default for
//!   the no-icon collapsing-header verbs).
//! - Clicking a **leading/trailing action icon** fires that action alone.
//!
//! ## Two ways to use it
//!
//! **Façade.** The [`crate::UiContext`] verbs handle indentation, row height,
//! and the auto-advancing layout cursor. [`tree_node`](crate::UiContext::tree_node)
//! /[`tree_leaf`](crate::UiContext::tree_leaf) are the simple no-icon path;
//! [`tree_row`](crate::UiContext::tree_row) takes a fully-configured
//! [`TreeNode`] (with action icons) and returns its [`TreeNodeOutput`]:
//!
//! ```ignore
//! let out = ui.tree_row(id, TreeNode::new("Layer 1")
//!     .with_leading(&[TreeAction::sprite(VIS, eye)])
//!     .with_trailing(&[TreeAction::sprite(RENAME, pen), TreeAction::sprite(DEL, trash)]));
//! match out.action {
//!     Some(VIS) => layer.visible = !layer.visible,
//!     Some(DEL) => delete(layer),
//!     _ => {}
//! }
//! if out.expanded { /* children */ ui.tree_pop(); }
//! ```
//!
//! **Raw widget.** [`TreeNode::draw`] renders one row into an explicit `Rect`
//! against a [`DrawContext`], taking the indentation depth via
//! [`TreeNode::with_depth`]. The caller drives the recursion and the vertical
//! cursor — what the façade is built on, and what a scrolled outliner panel
//! that owns its own layout would use directly.
//!
//! ## State
//! [`TreeState`] holds the expanded set + the selected node, keyed by a
//! caller-supplied [`TreeId`] (any per-tree-unique `u64`).
//! [`TreeNode::with_default_open`] sets the state the *first* time a given id is
//! seen, so a tree can start partly unfurled without the caller pre-seeding it.

use std::collections::HashSet;

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::SpriteId;

use super::{DrawContext, DrawList};

/// Stable identity for a node within one tree. Any scheme unique per node per
/// frame works (a hash, an enum discriminant, a stable row index). `0` is a
/// valid id.
pub type TreeId = u64;

/// Indentation added per depth level, in pixels.
const INDENT: f32 = 14.0;
/// Half-extent of the disclosure triangle, in pixels.
const ARROW: f32 = 4.0;
/// Horizontal gap after the disclosure column and around the label.
const GAP: f32 = 6.0;
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

/// Where an action icon's image comes from. Borrowed, so an action slice costs
/// no allocation. Mirrors the [`crate::Image`] source split.
#[derive(Debug, Clone, Copy)]
pub enum TreeIcon<'a> {
    /// A pre-resolved sprite handle (supports tint).
    Sprite(SpriteId),
    /// A string-keyed sprite, resolved by name at render time.
    Key(&'a str),
}

/// One action icon in a tree row's leading or trailing column. The caller picks
/// the icon for the node's *current* state (e.g. eye vs eye-slash) and gets back
/// `id` from [`TreeNodeOutput::action`] when it is clicked.
#[derive(Debug, Clone, Copy)]
pub struct TreeAction<'a> {
    /// Caller-defined identifier reported when this icon is clicked. Must be
    /// unique among a single row's leading + trailing actions.
    pub id: u32,
    /// The icon image.
    pub icon: TreeIcon<'a>,
    /// Multiplied into the sampled colour (sprite source only).
    pub tint: [f32; 4],
}

impl<'a> TreeAction<'a> {
    /// An action showing a pre-resolved sprite, untinted.
    pub fn sprite(id: u32, sprite: SpriteId) -> Self {
        Self {
            id,
            icon: TreeIcon::Sprite(sprite),
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }

    /// An action showing a string-keyed sprite, resolved by name at render time.
    pub fn key(id: u32, key: &'a str) -> Self {
        Self {
            id,
            icon: TreeIcon::Key(key),
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }

    /// Set the icon tint (sprite source only).
    pub fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }
}

/// Per-frame configuration for one tree row. Lightweight, built fresh each
/// frame like [`crate::Slider`] / [`crate::Dropdown`]; the action slices are
/// borrowed so a row costs no allocation.
pub struct TreeNode<'a> {
    label: &'a str,
    leaf: bool,
    default_open: bool,
    depth: usize,
    toggle_on_label: bool,
    leading: &'a [TreeAction<'a>],
    trailing: &'a [TreeAction<'a>],
    slot_size: Option<f32>,
}

/// Result of drawing one tree row.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeNodeOutput {
    /// Whether the node is expanded *after* this draw — i.e. whether the caller
    /// should render its children. Always `false` for a leaf.
    pub expanded: bool,
    /// The disclosure was toggled this frame (branch only).
    pub toggled: bool,
    /// The row body (label area) was clicked this frame — it became the
    /// selection. `false` when an action icon or the disclosure was clicked.
    pub clicked: bool,
    /// A leading/trailing action icon was clicked this frame; carries its
    /// [`TreeAction::id`]. Mutually exclusive with `clicked`/`toggled`.
    pub action: Option<u32>,
}

impl<'a> TreeNode<'a> {
    /// A branch node displaying `label`, with a disclosure triangle.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            leaf: false,
            default_open: false,
            depth: 0,
            toggle_on_label: false,
            leading: &[],
            trailing: &[],
            slot_size: None,
        }
    }

    /// A leaf node: no disclosure triangle, never expands. Clicking selects it.
    pub fn leaf(label: &'a str) -> Self {
        Self {
            leaf: true,
            ..Self::new(label)
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

    /// When set, clicking the label/body of a *branch* also toggles its
    /// expansion (in addition to selecting it). Default off — only the
    /// disclosure triangle toggles, so a node can be selected without
    /// collapsing it (outliner feel). The simple `tree_node` façade verb turns
    /// this on for classic whole-row collapsing-header behaviour.
    pub fn with_toggle_on_label(mut self, toggle: bool) -> Self {
        self.toggle_on_label = toggle;
        self
    }

    /// Leading action icons, drawn left-to-right between the disclosure and the
    /// label (e.g. a visibility toggle).
    pub fn with_leading(mut self, actions: &'a [TreeAction<'a>]) -> Self {
        self.leading = actions;
        self
    }

    /// Trailing action icons, drawn right-aligned at the row's end, left-to-right
    /// (e.g. rename, delete).
    pub fn with_trailing(mut self, actions: &'a [TreeAction<'a>]) -> Self {
        self.trailing = actions;
        self
    }

    /// Square edge (px) of each action-icon slot. Defaults to the row height.
    pub fn with_slot_size(mut self, size: f32) -> Self {
        self.slot_size = Some(size);
        self
    }

    /// Draw this row into `rect` and handle expand / select / action
    /// interaction. The row's content is indented by `depth * INDENT`; the
    /// selection/hover highlight spans the full `rect` width regardless of
    /// depth. Action-icon slots are excluded from the body hit area.
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
        let mouse_in = !input.mouse_consumed;
        let mx = input.mouse_x;
        let my = input.mouse_y;
        let row_hovered = mouse_in && rect.contains(mx, my);
        let selected = state.is_selected(id);

        // ---- layout -------------------------------------------------------
        let slot = self.slot_size.unwrap_or(rect.height).min(rect.height).max(1.0);
        let slot_y = rect.y + (rect.height - slot) * 0.5;
        let indent = self.depth as f32 * INDENT;
        let disclosure_left = rect.x + ROW_INSET + indent;
        let arrow_cx = disclosure_left + ARROW;
        let arrow_cy = rect.y + rect.height * 0.5;
        let disclosure_right = arrow_cx + ARROW + GAP; // end of triangle column

        let leading_x = disclosure_right;
        let leading_w = self.leading.len() as f32 * slot;
        let label_x = leading_x + leading_w + if self.leading.is_empty() { 0.0 } else { GAP };

        let trailing_w = self.trailing.len() as f32 * slot;
        let trailing_x0 = rect.x + rect.width - ROW_INSET - trailing_w;
        let label_right = if self.trailing.is_empty() {
            rect.x + rect.width - ROW_INSET
        } else {
            trailing_x0 - GAP
        };
        let label_max_w = (label_right - label_x).max(0.0);

        let leading_slot = |i: usize| Rect::new(leading_x + i as f32 * slot, slot_y, slot, slot);
        let trailing_slot = |i: usize| Rect::new(trailing_x0 + i as f32 * slot, slot_y, slot, slot);

        let list = &mut *ctx.draw_list;

        // ---- full-row highlight (selection wins over hover) ---------------
        if selected {
            list.quad(rect.x, rect.y, rect.width, rect.height, theme.accent);
        } else if row_hovered {
            list.quad(rect.x, rect.y, rect.width, rect.height, theme.button_hover);
        }

        let text_color = if selected { theme.background } else { theme.text };
        let arrow_color = if selected { theme.background } else { theme.text_dim };

        // ---- disclosure triangle ------------------------------------------
        if !self.leaf {
            draw_disclosure(list, arrow_cx, arrow_cy, expanded_before, arrow_color);
        }

        // ---- action icons (drawn here; clicks resolved below) -------------
        let mut action: Option<u32> = None;
        for (i, a) in self.leading.iter().enumerate() {
            let s = leading_slot(i);
            let hov = mouse_in && s.contains(mx, my);
            draw_action(list, s, &a.icon, a.tint, hov);
            if hov && input.mouse_clicked && action.is_none() {
                action = Some(a.id);
            }
        }
        for (i, a) in self.trailing.iter().enumerate() {
            let s = trailing_slot(i);
            let hov = mouse_in && s.contains(mx, my);
            draw_action(list, s, &a.icon, a.tint, hov);
            if hov && input.mouse_clicked && action.is_none() {
                action = Some(a.id);
            }
        }

        // ---- label --------------------------------------------------------
        let text_x = label_x;
        let text_y = rect.y + (rect.height - theme.font_size) * 0.5;
        let (r, g, b) = rgb(text_color);
        list.text(
            TextBlock::new(self.label, text_x, text_y)
                .with_size(theme.font_size)
                .with_color(r, g, b)
                .with_max_width(label_max_w)
                .with_ellipsis()
                .with_font_opt(theme.font.clone()),
        );

        // ---- interaction precedence: action > disclosure > body -----------
        let mut toggled = false;
        let mut body_clicked = false;
        if input.mouse_clicked && row_hovered && action.is_none() {
            let in_disclosure = mx >= disclosure_left && mx < disclosure_right;
            if !self.leaf && in_disclosure {
                state.toggle(id);
                toggled = true;
            } else {
                state.select(id);
                body_clicked = true;
                if self.toggle_on_label && !self.leaf {
                    state.toggle(id);
                    toggled = true;
                }
            }
        }

        let expanded = !self.leaf && state.is_expanded(id);
        TreeNodeOutput {
            expanded,
            toggled,
            clicked: body_clicked,
            action,
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

/// Draw one action icon into its `slot`, inset slightly, with a translucent
/// hover overlay. Alloc-free — calls the draw list's sprite/key paths directly
/// rather than constructing an [`crate::Image`].
fn draw_action(list: &mut DrawList, slot: Rect, icon: &TreeIcon, tint: [f32; 4], hovered: bool) {
    let inset = (slot.height * 0.18).min(4.0);
    let r = Rect::new(
        slot.x + inset,
        slot.y + inset,
        (slot.width - inset * 2.0).max(1.0),
        (slot.height - inset * 2.0).max(1.0),
    );
    match icon {
        TreeIcon::Sprite(sprite) => list.image(*sprite, r, tint),
        TreeIcon::Key(key) => list.icon(key, r.x, r.y, r.width, r.height),
    }
    if hovered {
        list.quad(slot.x, slot.y, slot.width, slot.height, [1.0, 1.0, 1.0, 0.12]);
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

    // A click that lands on the label/body (right of the disclosure column, away
    // from any trailing icons).
    fn click_body() -> InputState {
        click_at(90.0, 10.0)
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
        let (_, out) = draw_node(&node, 1, row(), &mut s, &idle());
        assert!(out.expanded, "default_open expands on first sight");
        s.set_expanded(1, false);
        let (_, out) = draw_node(&node, 1, row(), &mut s, &idle());
        assert!(!out.expanded, "default_open is one-shot; state now owns it");
    }

    #[test]
    fn body_click_selects_only_by_default() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch");
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_body());
        assert!(out.clicked, "body click selects");
        assert!(!out.toggled, "default does NOT toggle on body click");
        assert!(!out.expanded);
        assert!(s.is_selected(1));
    }

    #[test]
    fn disclosure_click_toggles_without_selecting() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch");
        // Click on the triangle column (near the left inset).
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(6.0, 10.0));
        assert!(out.toggled, "clicking the disclosure toggles");
        assert!(out.expanded, "was collapsed → now expanded");
        assert!(!out.clicked, "disclosure click is not a body select");
        assert_eq!(s.selected(), None, "disclosure click does not select");
    }

    #[test]
    fn toggle_on_label_flag_toggles_and_selects() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch").with_toggle_on_label(true);
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_body());
        assert!(out.clicked);
        assert!(out.toggled, "with_toggle_on_label, body click also toggles");
        assert!(out.expanded);
        assert!(s.is_selected(1));
    }

    #[test]
    fn clicking_leaf_selects_but_never_expands() {
        let mut s = TreeState::new();
        let node = TreeNode::leaf("leaf");
        let (_, out) = draw_node(&node, 7, row(), &mut s, &click_body());
        assert!(out.clicked);
        assert!(!out.toggled, "leaf never toggles");
        assert!(!out.expanded);
        assert!(s.is_selected(7));
    }

    #[test]
    fn leading_action_click_fires_action_not_selection() {
        let mut s = TreeState::new();
        const VIS: u32 = 11;
        let acts = [TreeAction::key(VIS, "eye")];
        let node = TreeNode::new("branch").with_leading(&acts);
        // The leading slot sits just right of the disclosure column. With a
        // 20px row, slot=20: disclosure_right ≈ 4+8+6=18, leading slot [18,38).
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(28.0, 10.0));
        assert_eq!(out.action, Some(VIS), "clicking the eye fires its action");
        assert!(!out.clicked, "an action click is not a body select");
        assert!(!out.toggled);
        assert_eq!(s.selected(), None, "the node was not selected");
    }

    #[test]
    fn trailing_action_click_fires_action_not_selection() {
        let mut s = TreeState::new();
        const RENAME: u32 = 21;
        const DEL: u32 = 22;
        let acts = [TreeAction::key(RENAME, "pen"), TreeAction::key(DEL, "trash")];
        let node = TreeNode::leaf("leaf").with_trailing(&acts);
        // Trailing slots right-aligned: slot=20, two slots end at x=200-4=196,
        // so [156,176) rename, [176,196) delete. Click the delete slot.
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(186.0, 10.0));
        assert_eq!(out.action, Some(DEL), "clicking the trash fires delete");
        assert!(!out.clicked);
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn body_click_between_icons_still_selects() {
        let mut s = TreeState::new();
        let lead = [TreeAction::key(1, "eye")];
        let trail = [TreeAction::key(2, "trash")];
        let node = TreeNode::new("branch").with_leading(&lead).with_trailing(&trail);
        // Click in the label area (well clear of both icon columns).
        let (_, out) = draw_node(&node, 5, row(), &mut s, &click_at(90.0, 10.0));
        assert_eq!(out.action, None, "label click is not an action");
        assert!(out.clicked);
        assert!(s.is_selected(5));
    }

    #[test]
    fn consumed_mouse_does_not_interact() {
        let mut s = TreeState::new();
        let acts = [TreeAction::key(9, "eye")];
        let node = TreeNode::new("branch").with_leading(&acts);
        let mut inp = click_at(28.0, 10.0);
        inp.mouse_consumed = true; // a higher layer took this click
        let (_, out) = draw_node(&node, 1, row(), &mut s, &inp);
        assert_eq!(out.action, None);
        assert!(!out.clicked);
        assert!(!out.toggled);
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn click_outside_row_does_nothing() {
        let mut s = TreeState::new();
        let node = TreeNode::new("branch");
        let (_, out) = draw_node(&node, 1, row(), &mut s, &click_at(500.0, 500.0));
        assert!(!out.clicked);
        assert!(!out.toggled);
        assert_eq!(out.action, None);
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
    fn trailing_icons_shrink_label_width() {
        // The label's max width must be smaller when trailing icons reserve the
        // right side.
        fn label_max_w(trailing: &[TreeAction]) -> f32 {
            let mut s = TreeState::new();
            let node = TreeNode::new("Node").with_trailing(trailing);
            let (list, _) = draw_node(&node, 1, row(), &mut s, &idle());
            list.texts.first().expect("label text emitted").max_width
        }
        let none = label_max_w(&[]);
        let two = label_max_w(&[TreeAction::key(1, "a"), TreeAction::key(2, "b")]);
        assert!(two < none, "trailing icons reserve width (none={none}, two={two})");
    }

    #[test]
    fn branch_emits_disclosure_triangle_leaf_does_not() {
        let mut s = TreeState::new();
        let (branch_list, _) = draw_node(&TreeNode::new("b"), 1, row(), &mut s, &idle());
        let mut s2 = TreeState::new();
        let (leaf_list, _) = draw_node(&TreeNode::leaf("l"), 2, row(), &mut s2, &idle());
        assert!(
            branch_list.vertices.len() > leaf_list.vertices.len(),
            "branch adds disclosure-triangle geometry the leaf lacks"
        );
    }
}
