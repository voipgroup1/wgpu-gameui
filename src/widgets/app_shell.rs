//! Application shell — the layout composition for a full editor chrome.
//!
//! The shell computes a fixed-structure layout (top-to-bottom peeling) and draws
//! the connecting chrome (splitters, dock panel headers, toolbar). The menu bar,
//! document tabs, status bar, and viewport content are drawn by the caller at
//! the rects the shell provides — the shell does not own those widgets.
//!
//! # Layout structure
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │  Menu bar (optional)                     │
//! ├──────────────────────────────────────────┤
//! │  Doc tabs (optional)                     │
//! ├─────┬─┬──────────────────┬─┬────────────┤
//! │     │ │   ┌──────────┐   │ │            │
//! │ L   │S│   │ Toolbar  │   │S│   R        │
//! │ e   │p│   ├──────────┤   │p│   i        │
//! │ f   │l│   │          │   │l│   g        │
//! │ t   │i│   │ Viewport │   │i│   h        │
//! │     │t│   │          │   │t│   t        │
//! │ d   │t│   │          │   │t│            │
//! │ o   │e│   └──────────┘   │e│   d        │
//! │ c   │r│                  │r│   o        │
//! │ k   │ │                  │ │   c        │
//! │     │ │                  │ │   k        │
//! ├─────┴─┼──────────────────┼─┴────────────┤
//! │       │  Splitter (h)    │               │
//! ├───────┴──────────────────┴───────────────┤
//! │  Bottom dock (optional)                  │
//! ├──────────────────────────────────────────┤
//! │  Status bar (optional)                   │
//! └──────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! let shell = AppShell::new()
//!     .with_menu_bar()
//!     .with_doc_tabs()
//!     .with_status_bar()
//!     .with_toolbar();
//!
//! let layout = shell.layout(
//!     Rect::new(0.0, 0.0, screen_w, screen_h),
//!     Some(&left_dock),
//!     Some(&right_dock),
//!     Some(&bottom_dock),
//!     Some(&toolbar_state),
//!     &styles,
//! );
//!
//! // Draw the menu bar, doc tabs, status bar at their rects...
//! // Then draw the shell chrome (splitters, dock headers, toolbar):
//! let chrome = shell.draw_chrome(
//!     &layout,
//!     Some((&left_tabs, &mut left_dock)),
//!     Some((&right_tabs, &mut right_dock)),
//!     Some((&bottom_tabs, &mut bottom_dock)),
//!     Some((&toolbar_items, &mut toolbar_state)),
//!     &mut drag_capture,
//!     &mut ctx,
//! );
//! // Draw viewport content into layout.viewport...
//! ```

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::widgets::dock_panel::{DockPanel, DockPanelOutput, DockPanelState, DockSide, DockTab};
use crate::widgets::drag::{DragCapture, DragId};
use crate::widgets::splitter::Splitter;
use crate::widgets::toolbar::{Toolbar, ToolbarEdge, ToolbarItem, ToolbarOutput, ToolbarState};

use super::DrawContext;

// ---------------------------------------------------------------------------
// Well-known DragId constants
// ---------------------------------------------------------------------------

/// [`DragId`] for the left dock's resize splitter.
pub const SHELL_DRAG_LEFT_SPLITTER: DragId = 0xA551_0001;
/// [`DragId`] for the right dock's resize splitter.
pub const SHELL_DRAG_RIGHT_SPLITTER: DragId = 0xA551_0002;
/// [`DragId`] for the bottom dock's resize splitter.
pub const SHELL_DRAG_BOTTOM_SPLITTER: DragId = 0xA551_0003;
/// [`DragId`] for the toolbar's grip handle.
pub const SHELL_DRAG_TOOLBAR_GRIP: DragId = 0xA551_0004;

// ---------------------------------------------------------------------------
// ShellLayout — pure geometry output
// ---------------------------------------------------------------------------

/// Computed rects for each zone of the shell. Returned by [`AppShell::layout`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellLayout {
    /// Menu bar strip. `None` when the shell has no menu bar.
    pub menu_bar: Option<Rect>,
    /// Document tabs strip. `None` when the shell has no doc tabs.
    pub doc_tabs: Option<Rect>,
    /// Left dock panel rect. `None` when hidden or absent.
    pub left_dock: Option<Rect>,
    /// Left resize splitter rect.
    pub left_splitter: Option<Rect>,
    /// Right dock panel rect. `None` when hidden or absent.
    pub right_dock: Option<Rect>,
    /// Right resize splitter rect.
    pub right_splitter: Option<Rect>,
    /// Bottom dock panel rect. `None` when hidden or absent.
    pub bottom_dock: Option<Rect>,
    /// Bottom resize splitter rect.
    pub bottom_splitter: Option<Rect>,
    /// Toolbar strip rect. `None` when no toolbar.
    pub toolbar: Option<Rect>,
    /// The viewport — the remaining area after all chrome is subtracted.
    pub viewport: Rect,
    /// Status bar strip. `None` when the shell has no status bar.
    pub status_bar: Option<Rect>,
}

// ---------------------------------------------------------------------------
// ShellChromeOutput
// ---------------------------------------------------------------------------

/// Results from drawing the shell chrome (splitters, dock headers, toolbar).
#[derive(Debug, Default)]
pub struct ShellChromeOutput {
    /// Left splitter output (delta, dragging).
    pub left_splitter: Option<crate::widgets::splitter::SplitterOutput>,
    /// Right splitter output.
    pub right_splitter: Option<crate::widgets::splitter::SplitterOutput>,
    /// Bottom splitter output.
    pub bottom_splitter: Option<crate::widgets::splitter::SplitterOutput>,
    /// Left dock panel output.
    pub left_dock: Option<DockPanelOutput>,
    /// Right dock panel output.
    pub right_dock: Option<DockPanelOutput>,
    /// Bottom dock panel output.
    pub bottom_dock: Option<DockPanelOutput>,
    /// Toolbar output.
    pub toolbar: Option<ToolbarOutput>,
}

// ---------------------------------------------------------------------------
// AppShell
// ---------------------------------------------------------------------------

/// Configuration for the application shell layout.
///
/// Constructed per-frame (stateless). The caller toggles zones on or off with
/// builder methods. All mutable state lives in the caller-owned structs passed
/// to [`layout`](Self::layout) and [`draw_chrome`](Self::draw_chrome).
pub struct AppShell {
    menu_bar: bool,
    doc_tabs: bool,
    status_bar: bool,
    has_toolbar: bool,
}

impl AppShell {
    /// Create an empty shell (no zones enabled).
    pub fn new() -> Self {
        Self {
            menu_bar: false,
            doc_tabs: false,
            status_bar: false,
            has_toolbar: false,
        }
    }

    /// Include a menu bar zone at the top.
    pub fn with_menu_bar(mut self) -> Self {
        self.menu_bar = true;
        self
    }

    /// Include a document tabs zone below the menu bar.
    pub fn with_doc_tabs(mut self) -> Self {
        self.doc_tabs = true;
        self
    }

    /// Include a status bar zone at the bottom.
    pub fn with_status_bar(mut self) -> Self {
        self.status_bar = true;
        self
    }

    /// Include a toolbar in the layout (positioned by [`ToolbarState::edge`]).
    pub fn with_toolbar(mut self) -> Self {
        self.has_toolbar = true;
        self
    }

    /// Compute the layout for the given screen rect and dock states.
    ///
    /// This is pure geometry — no drawing. A `None` dock or one with
    /// `visible == false` is excluded from the layout, and the viewport
    /// expands to fill the freed space.
    pub fn layout(
        &self,
        screen: Rect,
        left: Option<&DockPanelState>,
        right: Option<&DockPanelState>,
        bottom: Option<&DockPanelState>,
        toolbar: Option<&ToolbarState>,
        styles: &StyleResolver,
    ) -> ShellLayout {
        let mut out = ShellLayout::default();
        let mut rem = screen;

        let menu_h = styles.scalar(StyleKey::MenuRowHeight);
        let dock_tab_h = styles.scalar(StyleKey::DockTabHeight);
        let splitter_w = styles.scalar(StyleKey::DockSplitterWidth);
        let status_h = 26.0_f32; // STATUS_BAR_HEIGHT from status_bar.rs

        // --- Top: menu bar ---
        if self.menu_bar {
            out.menu_bar = Some(Rect::new(rem.x, rem.y, rem.width, menu_h));
            rem = Rect::new(
                rem.x,
                rem.y + menu_h,
                rem.width,
                (rem.height - menu_h).max(0.0),
            );
        }

        // --- Top: doc tabs ---
        if self.doc_tabs {
            out.doc_tabs = Some(Rect::new(rem.x, rem.y, rem.width, dock_tab_h));
            rem = Rect::new(
                rem.x,
                rem.y + dock_tab_h,
                rem.width,
                (rem.height - dock_tab_h).max(0.0),
            );
        }

        // --- Bottom: status bar ---
        if self.status_bar {
            let sy = rem.y + (rem.height - status_h).max(0.0);
            out.status_bar = Some(Rect::new(rem.x, sy, rem.width, status_h));
            rem = Rect::new(rem.x, rem.y, rem.width, (rem.height - status_h).max(0.0));
        }

        // --- Bottom: bottom dock + splitter ---
        if let Some(bs) = bottom {
            if bs.visible && rem.height > splitter_w + 1.0 {
                let dock_h = bs.size.min(rem.height - splitter_w);
                let sp_y = rem.y + rem.height - dock_h - splitter_w;
                out.bottom_splitter = Some(Rect::new(rem.x, sp_y, rem.width, splitter_w));
                out.bottom_dock = Some(Rect::new(rem.x, sp_y + splitter_w, rem.width, dock_h));
                rem = Rect::new(
                    rem.x,
                    rem.y,
                    rem.width,
                    (rem.height - dock_h - splitter_w).max(0.0),
                );
            }
        }

        // --- Left: left dock + splitter ---
        if let Some(ls) = left {
            if ls.visible && rem.width > splitter_w + 1.0 {
                let dock_w = ls.size.min(rem.width - splitter_w);
                out.left_dock = Some(Rect::new(rem.x, rem.y, dock_w, rem.height));
                out.left_splitter = Some(Rect::new(rem.x + dock_w, rem.y, splitter_w, rem.height));
                rem = Rect::new(
                    rem.x + dock_w + splitter_w,
                    rem.y,
                    (rem.width - dock_w - splitter_w).max(0.0),
                    rem.height,
                );
            }
        }

        // --- Right: right dock + splitter ---
        if let Some(rs) = right {
            if rs.visible && rem.width > splitter_w + 1.0 {
                let dock_w = rs.size.min(rem.width - splitter_w);
                let sp_x = rem.x + rem.width - dock_w - splitter_w;
                out.right_splitter = Some(Rect::new(sp_x, rem.y, splitter_w, rem.height));
                out.right_dock = Some(Rect::new(sp_x + splitter_w, rem.y, dock_w, rem.height));
                rem = Rect::new(
                    rem.x,
                    rem.y,
                    (rem.width - dock_w - splitter_w).max(0.0),
                    rem.height,
                );
            }
        }

        // --- Toolbar: peel from the toolbar's edge of the viewport ---
        if self.has_toolbar {
            if let Some(ts) = toolbar {
                let btn_size = styles.scalar(StyleKey::ToolbarButtonSize);
                let pad = styles.scalar(StyleKey::ToolbarPadding);
                let cross = btn_size + pad * 2.0;
                match ts.edge {
                    ToolbarEdge::Left => {
                        let tw = cross.min(rem.width);
                        out.toolbar = Some(Rect::new(rem.x, rem.y, tw, rem.height));
                        rem = Rect::new(rem.x + tw, rem.y, (rem.width - tw).max(0.0), rem.height);
                    }
                    ToolbarEdge::Right => {
                        let tw = cross.min(rem.width);
                        out.toolbar =
                            Some(Rect::new(rem.x + rem.width - tw, rem.y, tw, rem.height));
                        rem = Rect::new(rem.x, rem.y, (rem.width - tw).max(0.0), rem.height);
                    }
                    ToolbarEdge::Top => {
                        let th = cross.min(rem.height);
                        out.toolbar = Some(Rect::new(rem.x, rem.y, rem.width, th));
                        rem = Rect::new(rem.x, rem.y + th, rem.width, (rem.height - th).max(0.0));
                    }
                    ToolbarEdge::Bottom => {
                        let th = cross.min(rem.height);
                        out.toolbar =
                            Some(Rect::new(rem.x, rem.y + rem.height - th, rem.width, th));
                        rem = Rect::new(rem.x, rem.y, rem.width, (rem.height - th).max(0.0));
                    }
                }
            }
        }

        // --- Remainder is the viewport ---
        out.viewport = rem;
        out
    }

    /// Draw the shell chrome: splitters, dock panel headers, and toolbar.
    ///
    /// Does **not** draw the menu bar, doc tabs, status bar, or viewport
    /// content — those are drawn by the caller at the rects from
    /// [`layout`](Self::layout).
    ///
    /// Splitter deltas are applied to dock state with clamping before returning.
    pub fn draw_chrome<'t>(
        &self,
        shell: &ShellLayout,
        left: Option<(&[DockTab<'t>], &mut DockPanelState)>,
        right: Option<(&[DockTab<'t>], &mut DockPanelState)>,
        bottom: Option<(&[DockTab<'t>], &mut DockPanelState)>,
        toolbar: Option<(&[ToolbarItem<'t>], &mut ToolbarState)>,
        drag_capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> ShellChromeOutput {
        let mut out = ShellChromeOutput::default();

        // --- Left splitter + dock ---
        if let (Some(sp_rect), Some((tabs, dock_state))) = (shell.left_splitter, left) {
            let sp = Splitter::vertical(sp_rect.width).draw(
                SHELL_DRAG_LEFT_SPLITTER,
                drag_capture,
                sp_rect,
                ctx,
            );
            dock_state.size =
                (dock_state.size + sp.delta).clamp(dock_state.min_size, dock_state.max_size);
            out.left_splitter = Some(sp);

            if let Some(dock_rect) = shell.left_dock {
                let dock_out = DockPanel::new(DockSide::Left, tabs)
                    .closable()
                    .draw(dock_rect, dock_state, ctx);
                out.left_dock = Some(dock_out);
            }
        }

        // --- Right splitter + dock ---
        if let (Some(sp_rect), Some((tabs, dock_state))) = (shell.right_splitter, right) {
            let sp = Splitter::vertical(sp_rect.width).draw(
                SHELL_DRAG_RIGHT_SPLITTER,
                drag_capture,
                sp_rect,
                ctx,
            );
            // Right splitter: dragging left (negative delta) increases right dock width.
            dock_state.size =
                (dock_state.size - sp.delta).clamp(dock_state.min_size, dock_state.max_size);
            out.right_splitter = Some(sp);

            if let Some(dock_rect) = shell.right_dock {
                let dock_out = DockPanel::new(DockSide::Right, tabs)
                    .closable()
                    .draw(dock_rect, dock_state, ctx);
                out.right_dock = Some(dock_out);
            }
        }

        // --- Bottom splitter + dock ---
        if let (Some(sp_rect), Some((tabs, dock_state))) = (shell.bottom_splitter, bottom) {
            let sp = Splitter::horizontal(sp_rect.height).draw(
                SHELL_DRAG_BOTTOM_SPLITTER,
                drag_capture,
                sp_rect,
                ctx,
            );
            // Bottom splitter: dragging up (negative delta) increases bottom dock height.
            dock_state.size =
                (dock_state.size - sp.delta).clamp(dock_state.min_size, dock_state.max_size);
            out.bottom_splitter = Some(sp);

            if let Some(dock_rect) = shell.bottom_dock {
                let dock_out = DockPanel::new(DockSide::Bottom, tabs)
                    .closable()
                    .draw(dock_rect, dock_state, ctx);
                out.bottom_dock = Some(dock_out);
            }
        }

        // --- Toolbar ---
        if let (Some(tb_rect), Some((items, tb_state))) = (shell.toolbar, toolbar) {
            let tb = Toolbar::new(items);
            let tb_out = tb.draw(
                tb_rect,
                tb_state,
                drag_capture,
                SHELL_DRAG_TOOLBAR_GRIP,
                ctx,
            );
            out.toolbar = Some(tb_out);
        }

        out
    }
}

impl Default for AppShell {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::style::StyleResolver;

    #[test]
    fn all_zones_visible() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);
        let left = DockPanelState::new(180.0);
        let right = DockPanelState::new(200.0);
        let bottom = DockPanelState::new(120.0);
        let toolbar = ToolbarState::new(ToolbarEdge::Left);

        let shell = AppShell::new()
            .with_menu_bar()
            .with_doc_tabs()
            .with_status_bar()
            .with_toolbar();

        let layout = shell.layout(
            screen,
            Some(&left),
            Some(&right),
            Some(&bottom),
            Some(&toolbar),
            &s,
        );

        assert!(layout.menu_bar.is_some());
        assert!(layout.doc_tabs.is_some());
        assert!(layout.status_bar.is_some());
        assert!(layout.left_dock.is_some());
        assert!(layout.left_splitter.is_some());
        assert!(layout.right_dock.is_some());
        assert!(layout.right_splitter.is_some());
        assert!(layout.bottom_dock.is_some());
        assert!(layout.bottom_splitter.is_some());
        assert!(layout.toolbar.is_some());
        assert!(layout.viewport.width > 0.0);
        assert!(layout.viewport.height > 0.0);
    }

    #[test]
    fn hidden_dock_expands_viewport() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);

        let shell = AppShell::new();

        // No docks, no toolbar.
        let layout_none = shell.layout(screen, None, None, None, None, &s);

        // Left dock visible.
        let left = DockPanelState::new(180.0);
        let layout_left = shell.layout(screen, Some(&left), None, None, None, &s);

        assert!(
            layout_none.viewport.width > layout_left.viewport.width,
            "hidden dock should give wider viewport"
        );

        // Hidden dock (visible = false).
        let mut left_hidden = DockPanelState::new(180.0);
        left_hidden.visible = false;
        let layout_hidden = shell.layout(screen, Some(&left_hidden), None, None, None, &s);

        assert!(
            (layout_none.viewport.width - layout_hidden.viewport.width).abs() < 0.01,
            "invisible dock should not consume space"
        );
    }

    #[test]
    fn rects_do_not_overlap() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);
        let left = DockPanelState::new(180.0);
        let right = DockPanelState::new(200.0);
        let bottom = DockPanelState::new(120.0);
        let toolbar = ToolbarState::new(ToolbarEdge::Left);

        let layout = AppShell::new()
            .with_menu_bar()
            .with_doc_tabs()
            .with_status_bar()
            .with_toolbar()
            .layout(
                screen,
                Some(&left),
                Some(&right),
                Some(&bottom),
                Some(&toolbar),
                &s,
            );

        // Collect all rects that are present.
        let mut rects: Vec<(&str, Rect)> = vec![];
        if let Some(r) = layout.menu_bar {
            rects.push(("menu_bar", r));
        }
        if let Some(r) = layout.doc_tabs {
            rects.push(("doc_tabs", r));
        }
        if let Some(r) = layout.left_dock {
            rects.push(("left_dock", r));
        }
        if let Some(r) = layout.left_splitter {
            rects.push(("left_splitter", r));
        }
        if let Some(r) = layout.right_dock {
            rects.push(("right_dock", r));
        }
        if let Some(r) = layout.right_splitter {
            rects.push(("right_splitter", r));
        }
        if let Some(r) = layout.bottom_dock {
            rects.push(("bottom_dock", r));
        }
        if let Some(r) = layout.bottom_splitter {
            rects.push(("bottom_splitter", r));
        }
        if let Some(r) = layout.toolbar {
            rects.push(("toolbar", r));
        }
        rects.push(("viewport", layout.viewport));
        if let Some(r) = layout.status_bar {
            rects.push(("status_bar", r));
        }

        // Check that no two rects overlap (more than epsilon).
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let (na, a) = rects[i];
                let (nb, b) = rects[j];
                let area = if let Some(overlap) = a.intersection(b) {
                    overlap.width.max(0.0) * overlap.height.max(0.0)
                } else {
                    0.0
                };
                assert!(
                    area < 1.0,
                    "{na} and {nb} overlap by {area:.1}px²: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn toolbar_edge_placement() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);

        let shell = AppShell::new().with_toolbar();

        for edge in [
            ToolbarEdge::Left,
            ToolbarEdge::Right,
            ToolbarEdge::Top,
            ToolbarEdge::Bottom,
        ] {
            let toolbar = ToolbarState::new(edge);
            let layout = shell.layout(screen, None, None, None, Some(&toolbar), &s);
            let tb = layout.toolbar.expect("toolbar should be present");

            match edge {
                ToolbarEdge::Left => {
                    assert!(
                        (tb.x - screen.x).abs() < 0.01,
                        "left toolbar should touch left edge"
                    );
                }
                ToolbarEdge::Right => {
                    assert!(
                        ((tb.x + tb.width) - (screen.x + screen.width)).abs() < 0.01,
                        "right toolbar should touch right edge"
                    );
                }
                ToolbarEdge::Top => {
                    assert!(
                        (tb.y - screen.y).abs() < 0.01,
                        "top toolbar should touch top edge"
                    );
                }
                ToolbarEdge::Bottom => {
                    assert!(
                        ((tb.y + tb.height) - (screen.y + screen.height)).abs() < 0.01,
                        "bottom toolbar should touch bottom edge"
                    );
                }
            }
        }
    }

    #[test]
    fn menu_bar_and_status_bar_heights() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);
        let layout = AppShell::new()
            .with_menu_bar()
            .with_status_bar()
            .layout(screen, None, None, None, None, &s);

        let menu = layout.menu_bar.unwrap();
        assert!((menu.height - theme.menu_row_height).abs() < 0.01);
        assert!((menu.y - 0.0).abs() < 0.01);

        let status = layout.status_bar.unwrap();
        assert!((status.height - 26.0).abs() < 0.01);
        assert!(
            ((status.y + status.height) - screen.height).abs() < 0.01,
            "status bar should be at the bottom"
        );
    }

    #[test]
    fn empty_shell_gives_full_viewport() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);
        let layout = AppShell::new().layout(screen, None, None, None, None, &s);

        assert!((layout.viewport.x - screen.x).abs() < 0.01);
        assert!((layout.viewport.y - screen.y).abs() < 0.01);
        assert!((layout.viewport.width - screen.width).abs() < 0.01);
        assert!((layout.viewport.height - screen.height).abs() < 0.01);
    }
}
