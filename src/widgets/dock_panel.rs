//! Dock panel — a tabbed, resizable, closable panel for left/right/bottom docks.
//!
//! A dock panel draws the chrome: a tab header row and a panel background. The
//! resize splitter between dock and viewport is *not* part of the dock — it is a
//! layout concern handled by [`AppShell`](crate::AppShell) or the caller
//! directly. This follows the existing pattern where [`Splitter`](crate::Splitter)
//! is drawn by the layout owner.
//!
//! # State ownership
//!
//! [`DockPanelState`] is caller-owned and persisted across frames — one instance
//! per dock position. It is *not* a field of `UiState` because dock visibility,
//! size, and active tab are application-level layout state.
//!
//! # Example
//!
//! ```ignore
//! let tabs = &[DockTab { label: "Outliner" }, DockTab { label: "Layers" }];
//! let out = DockPanel::new(DockSide::Left, tabs)
//!     .closable()
//!     .draw(rect, &mut dock_state, &mut ctx);
//!
//! if out.close_clicked { dock_state.visible = false; }
//! if let Some(i) = out.tab_clicked { dock_state.active_tab = i; }
//!
//! // Draw the active tab's content into out.body:
//! match dock_state.active_tab {
//!     0 => draw_outliner(out.body, ctx),
//!     1 => draw_layers(out.body, ctx),
//!     _ => {}
//! }
//! ```

use crate::layout::Rect;
use crate::style::StyleKey;
use crate::text::TextBlock;

use super::DrawContext;
use super::material;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Which side of the workspace this dock panel occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockSide {
    /// Left edge of the workspace (vertical panel, resizes horizontally).
    Left,
    /// Right edge of the workspace (vertical panel, resizes horizontally).
    Right,
    /// Bottom edge of the workspace (horizontal panel, resizes vertically).
    Bottom,
}

/// One dock tab's per-frame data, borrowed from the caller.
pub struct DockTab<'a> {
    /// Tab label text.
    pub label: &'a str,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Caller-owned state for one dock position. Persisted across frames.
pub struct DockPanelState {
    /// Whether this dock is currently visible.
    pub visible: bool,
    /// Size along the dock's resize axis: width for `Left`/`Right`, height for
    /// `Bottom`. The caller initializes this; [`Splitter`](crate::Splitter)
    /// drag adjusts it each frame.
    pub size: f32,
    /// Minimum size, clamped during resize drag.
    pub min_size: f32,
    /// Maximum size, clamped during resize drag.
    pub max_size: f32,
    /// Index of the active tab.
    pub active_tab: usize,
}

impl DockPanelState {
    /// Create dock state with the given initial `size`, visible, tab 0 active.
    pub fn new(size: f32) -> Self {
        Self {
            visible: true,
            size,
            min_size: 100.0,
            max_size: 600.0,
            active_tab: 0,
        }
    }

    /// Set the allowed size range (builder pattern).
    pub fn with_range(mut self, min: f32, max: f32) -> Self {
        self.min_size = min;
        self.max_size = max;
        self
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// A dock panel: tabbed header + panel background. The caller draws tab content
/// into the returned body rect.
pub struct DockPanel<'a> {
    tabs: &'a [DockTab<'a>],
    /// Which side the dock occupies (currently used by [`AppShell`](crate::AppShell)
    /// for layout; the panel chrome itself is side-agnostic).
    #[allow(dead_code)]
    side: DockSide,
    closable: bool,
}

impl<'a> DockPanel<'a> {
    /// Create a dock panel on `side` with the given `tabs`.
    pub fn new(side: DockSide, tabs: &'a [DockTab<'a>]) -> Self {
        Self {
            tabs,
            side,
            closable: false,
        }
    }

    /// Show a close (×) button in the tab header that hides the panel.
    pub fn closable(mut self) -> Self {
        self.closable = true;
        self
    }

    /// Draw the dock panel chrome (header + background) at `rect`. Returns the
    /// body rect for the caller to draw content into, plus any events.
    pub fn draw(
        &self,
        rect: Rect,
        state: &mut DockPanelState,
        ctx: &mut DrawContext,
    ) -> DockPanelOutput {
        let s = ctx.styles();
        let input = ctx.input;
        let tab_h = s.scalar(StyleKey::DockTabHeight);
        let radius = s.scalar(StyleKey::BorderRadius);
        let font_size = s.scalar(StyleKey::FontSize) * 0.85;

        ctx.push_debug_scope_rect("DockPanel", rect);

        let mut out = DockPanelOutput {
            tab_clicked: None,
            close_clicked: false,
            body: Rect::new(
                rect.x,
                rect.y + tab_h,
                rect.width,
                (rect.height - tab_h).max(0.0),
            ),
            rect,
        };

        // --- Panel background ---
        let panel_bg = s.color(StyleKey::Panel);
        let panel_border = s.color(StyleKey::PanelBorder);
        ctx.draw_list.rounded_rect(rect, radius, panel_bg);
        ctx.draw_list
            .rounded_rect_outline(rect, radius, 1.0, panel_border);

        // --- Tab header background ---
        let header_rect = Rect::new(rect.x, rect.y, rect.width, tab_h);
        // Slightly lighter than the panel (sheen overlay).
        let header_bg = material::sheen_over(panel_bg, [1.0, 1.0, 1.0, 0.04]);
        ctx.draw_list.quad(
            header_rect.x,
            header_rect.y,
            header_rect.width,
            header_rect.height,
            header_bg,
        );
        // Bottom border of the header.
        ctx.draw_list.quad(
            header_rect.x,
            header_rect.y + header_rect.height - 1.0,
            header_rect.width,
            1.0,
            panel_border,
        );

        // --- Close button (right side of header) ---
        let close_w = if self.closable { 18.0 } else { 0.0 };
        if self.closable {
            let close_rect = Rect::new(
                rect.x + rect.width - close_w - 2.0,
                rect.y + (tab_h - 17.0) * 0.5,
                17.0,
                17.0,
            );
            let close_hovered =
                close_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
            let close_pressed = close_hovered && input.mouse_down;
            let close_clicked = close_hovered && input.mouse_clicked;

            // Ghost button chrome.
            if close_hovered {
                let hover_bg = [1.0, 1.0, 1.0, if close_pressed { 0.12 } else { 0.06 }];
                ctx.draw_list.rounded_rect(close_rect, radius, hover_bg);
            }
            // "×" glyph.
            let xc = if close_hovered {
                s.color(StyleKey::Text)
            } else {
                s.color(StyleKey::TextDim)
            };
            let xty = ctx.draw_list.vcentered_text_y(
                close_rect.y,
                close_rect.height,
                10.0,
                s.theme().font.as_ref(),
                "✕",
            );
            ctx.draw_list.text(
                TextBlock::new("✕", close_rect.x + (close_rect.width - 6.0) * 0.5, xty)
                    .with_size(10.0)
                    .with_color(
                        (xc[0] * 255.0) as u8,
                        (xc[1] * 255.0) as u8,
                        (xc[2] * 255.0) as u8,
                    )
                    .with_font_opt(s.theme().font.clone()),
            );

            if close_clicked {
                out.close_clicked = true;
            }
        }

        // --- Tab buttons ---
        if !self.tabs.is_empty() {
            let available_w = rect.width - close_w - 4.0; // 2px margin each side
            let max_tab_w = 120.0_f32;
            let tab_w = (available_w / self.tabs.len() as f32).min(max_tab_w);
            let tab_pad_h = 7.0;

            for (i, tab) in self.tabs.iter().enumerate() {
                let tx = rect.x + 2.0 + i as f32 * tab_w;
                let tab_rect = Rect::new(tx, rect.y + (tab_h - 18.0) * 0.5, tab_w, 18.0);
                let is_active = i == state.active_tab;
                let tab_hovered =
                    tab_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
                let tab_clicked = tab_hovered && input.mouse_clicked;

                // Tab background.
                if is_active {
                    // Gradient with subtle highlight.
                    let top =
                        material::sheen_over(s.color(StyleKey::TabActive), [1.0, 1.0, 1.0, 0.10]);
                    ctx.draw_list
                        .chrome_rect(tab_rect, radius, 1.0, top, [0.0, 0.0, 0.0, 0.35]);
                    // Inset highlight on active tab.
                    let hl = s.color(StyleKey::EdgeHighlight);
                    ctx.draw_list.quad(
                        tab_rect.x + 1.0,
                        tab_rect.y + 1.0,
                        (tab_rect.width - 2.0).max(0.0),
                        1.0,
                        hl,
                    );
                } else if tab_hovered {
                    ctx.draw_list
                        .rounded_rect(tab_rect, radius, [1.0, 1.0, 1.0, 0.04]);
                }

                // Tab label.
                let text_color = if is_active {
                    s.color(StyleKey::TextHighlight)
                } else if tab_hovered {
                    s.color(StyleKey::Text)
                } else {
                    s.color(StyleKey::TextDim)
                };
                let ty = ctx.draw_list.vcentered_text_y(
                    tab_rect.y,
                    tab_rect.height,
                    font_size,
                    s.theme().font.as_ref(),
                    tab.label,
                );
                ctx.draw_list.text(
                    TextBlock::new(tab.label, tx + tab_pad_h, ty)
                        .with_size(font_size)
                        .with_color(
                            (text_color[0] * 255.0) as u8,
                            (text_color[1] * 255.0) as u8,
                            (text_color[2] * 255.0) as u8,
                        )
                        .with_ellipsis()
                        .with_max_width((tab_w - tab_pad_h * 2.0).max(8.0))
                        .with_font_opt(s.theme().font.clone()),
                );

                if tab_clicked {
                    out.tab_clicked = Some(i);
                }
            }
        }

        ctx.pop_debug_scope();
        out
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Outcome of drawing a [`DockPanel`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DockPanelOutput {
    /// A tab was clicked (index into the tabs slice).
    pub tab_clicked: Option<usize>,
    /// The close button was clicked.
    pub close_clicked: bool,
    /// The body rect available for tab content (below the header).
    pub body: Rect,
    /// The full rect used by the panel.
    pub rect: Rect,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    fn tabs() -> Vec<DockTab<'static>> {
        vec![
            DockTab { label: "Outliner" },
            DockTab { label: "Layers" },
            DockTab { label: "Assets" },
        ]
    }

    #[test]
    fn clicking_a_tab_reports_its_index() {
        let theme = Theme::default();
        let ts = tabs();
        let rect = Rect::new(0.0, 0.0, 200.0, 150.0);
        let mut state = DockPanelState::new(200.0);

        // Click in the second tab area (tabs start at x=2, each ~120px max
        // but 200/3 ≈ 66px wide here).
        let tab_w = (200.0 - 4.0) / 3.0;
        let input = InputState {
            mouse_x: 2.0 + tab_w * 1.5,
            mouse_y: 9.0, // in the tab header
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = DockPanel::new(DockSide::Left, &ts).draw(
            rect,
            &mut state,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.tab_clicked, Some(1));
    }

    #[test]
    fn close_button_reports_click() {
        let theme = Theme::default();
        let ts = tabs();
        let rect = Rect::new(0.0, 0.0, 200.0, 150.0);
        let mut state = DockPanelState::new(200.0);

        // Click the close button area (right side of the header).
        let input = InputState {
            mouse_x: 200.0 - 12.0,
            mouse_y: 12.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = DockPanel::new(DockSide::Left, &ts).closable().draw(
            rect,
            &mut state,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.close_clicked);
    }

    #[test]
    fn body_rect_is_below_header() {
        let theme = Theme::default();
        let ts = tabs();
        let rect = Rect::new(10.0, 20.0, 180.0, 130.0);
        let mut state = DockPanelState::new(180.0);
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = DockPanel::new(DockSide::Left, &ts).draw(
            rect,
            &mut state,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let tab_h = theme.dock_tab_height;
        assert!((out.body.y - (rect.y + tab_h)).abs() < 0.01);
        assert!((out.body.height - (rect.height - tab_h)).abs() < 0.01);
        assert!((out.body.x - rect.x).abs() < 0.01);
        assert!((out.body.width - rect.width).abs() < 0.01);
    }

    #[test]
    fn empty_tabs_still_draw_background() {
        let theme = Theme::default();
        let tabs: Vec<DockTab<'_>> = vec![];
        let rect = Rect::new(0.0, 0.0, 200.0, 150.0);
        let mut state = DockPanelState::new(200.0);
        let input = InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = DockPanel::new(DockSide::Left, &tabs).draw(
            rect,
            &mut state,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        // Should still emit geometry (background) and return a body rect.
        assert!(!list.chrome_instances.is_empty() || !list.vertices.is_empty());
        assert!(out.body.height > 0.0);
    }

    #[test]
    fn consumed_input_suppresses_tab_click() {
        let theme = Theme::default();
        let ts = tabs();
        let rect = Rect::new(0.0, 0.0, 200.0, 150.0);
        let mut state = DockPanelState::new(200.0);

        let input = InputState {
            mouse_x: 30.0,
            mouse_y: 9.0,
            mouse_clicked: true,
            mouse_down: true,
            mouse_consumed: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = DockPanel::new(DockSide::Left, &ts).draw(
            rect,
            &mut state,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert_eq!(out.tab_clicked, None);
    }

    #[test]
    fn toggle_inverts_visibility() {
        let mut state = DockPanelState::new(200.0);
        assert!(state.visible);
        state.toggle();
        assert!(!state.visible);
        state.toggle();
        assert!(state.visible);
    }

    #[test]
    fn with_range_sets_bounds() {
        let state = DockPanelState::new(200.0).with_range(50.0, 400.0);
        assert!((state.min_size - 50.0).abs() < 0.01);
        assert!((state.max_size - 400.0).abs() < 0.01);
    }
}
