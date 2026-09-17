//! Toolbar widget — a strip of tool buttons with separators and a grip handle.
//!
//! The toolbar docks to an edge of the viewport (left/right/top/bottom). It is
//! purely draw-time: no popup layers, no navigation intent claiming, no
//! frame-deferred geometry. The grip handle's drag is arbitrated through a
//! caller-owned [`DragCapture`]; tooltips are reported via the output for the
//! caller's own [`TooltipLayer`](crate::TooltipLayer).
//!
//! # State ownership
//!
//! [`ToolbarState`] is caller-owned and persists across frames — the same
//! contract as [`MenuBarState`](crate::MenuBarState). It is *not* a field of
//! `UiState` because `active_tool` is application state, not UI plumbing.
//!
//! # Example
//!
//! ```ignore
//! let items: &[ToolbarItem] = &[
//!     ToolbarItem::tool(1, Icon::new(PhosphorIcon::Cursor), "Select", "Q"),
//!     ToolbarItem::tool(2, Icon::new(PhosphorIcon::ArrowsOutCardinal), "Move", "W"),
//!     ToolbarItem::separator(),
//!     ToolbarItem::tool(3, Icon::new(PhosphorIcon::Cube), "Box brush", "B"),
//! ];
//! let out = Toolbar::new(items).draw(rect, &mut state, &mut capture, GRIP_ID, &mut ctx);
//! if let Some(id) = out.clicked { state.active_tool = Some(id); }
//! ```

use crate::layout::Rect;
use crate::style::StyleKey;
use crate::widgets::drag::{DragCapture, DragId};
use crate::widgets::icon::Icon;
use crate::widgets::material::{self, Material, Tone};
use crate::widgets::separator::Separator;

use super::DrawContext;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Which edge of the viewport the toolbar docks to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolbarEdge {
    /// Left edge (vertical strip, tools stacked top-to-bottom).
    #[default]
    Left,
    /// Right edge (vertical strip).
    Right,
    /// Top edge (horizontal strip, tools laid out left-to-right).
    Top,
    /// Bottom edge (horizontal strip).
    Bottom,
}

impl ToolbarEdge {
    /// Whether this edge produces a vertical toolbar.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// One tool button's description, borrowed per-frame.
pub struct ToolDef<'a> {
    /// Stable identity used for active-tool matching and reported in output.
    pub id: u64,
    /// The icon to draw inside the button.
    pub icon: Icon,
    /// Display name (shown in tooltip).
    pub label: &'a str,
    /// Shortcut key hint (shown in tooltip, e.g. `"B"` or `"Shift+G"`).
    pub shortcut: &'a str,
    /// Whether this tool can be selected.
    pub enabled: bool,
}

/// A toolbar item: either a tool button or a visual separator.
pub enum ToolbarItem<'a> {
    /// A tool button.
    Tool(ToolDef<'a>),
    /// A thin separator line between tool button groups.
    Separator,
}

impl<'a> ToolbarItem<'a> {
    /// Shorthand for a tool item.
    pub fn tool(id: u64, icon: Icon, label: &'a str, shortcut: &'a str) -> Self {
        Self::Tool(ToolDef {
            id,
            icon,
            label,
            shortcut,
            enabled: true,
        })
    }

    /// Shorthand for a separator.
    pub fn separator() -> Self {
        Self::Separator
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Caller-owned toolbar state, persisted across frames.
pub struct ToolbarState {
    /// Which edge the toolbar currently docks to.
    pub edge: ToolbarEdge,
    /// Which tool id is currently active/held. The toolbar highlights this tool
    /// with the accent material.
    pub active_tool: Option<u64>,
}

impl ToolbarState {
    /// Create a new toolbar state docked to `edge` with no active tool.
    pub fn new(edge: ToolbarEdge) -> Self {
        Self {
            edge,
            active_tool: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Toolbar widget — a strip of tool buttons with separators and a grip handle.
pub struct Toolbar<'a> {
    items: &'a [ToolbarItem<'a>],
}

impl<'a> Toolbar<'a> {
    /// Create a toolbar from a slice of tool items.
    pub fn new(items: &'a [ToolbarItem<'a>]) -> Self {
        Self { items }
    }

    /// Compute the preferred size of the toolbar along the main axis.
    ///
    /// For a vertical toolbar this is the height; for horizontal, the width.
    /// Cross-axis size is `button_size + 2 * padding`.
    pub fn preferred_extent(&self, button_size: f32, padding: f32) -> f32 {
        let grip = GRIP_SIZE;
        let sep_thick = SEPARATOR_THICKNESS + SEPARATOR_GAP * 2.0;
        let mut extent = grip;
        for item in self.items {
            match item {
                ToolbarItem::Tool(_) => extent += button_size,
                ToolbarItem::Separator => extent += sep_thick,
            }
        }
        extent + padding * 2.0
    }

    /// Compute the preferred cross-axis size.
    pub fn preferred_cross(&self, button_size: f32, padding: f32) -> f32 {
        button_size + padding * 2.0
    }

    /// Draw the toolbar into `rect`, returning the output.
    ///
    /// `state.edge` determines orientation. The caller is responsible for
    /// placing `rect` at the correct edge of the viewport (or using
    /// [`AppShell`](crate::AppShell) which does this automatically).
    pub fn draw(
        &self,
        rect: Rect,
        state: &mut ToolbarState,
        drag_capture: &mut DragCapture,
        grip_drag_id: DragId,
        ctx: &mut DrawContext,
    ) -> ToolbarOutput {
        let s = ctx.styles();
        let button_size = s.scalar(StyleKey::ToolbarButtonSize);
        let padding = s.scalar(StyleKey::ToolbarPadding);
        let border_radius = s.scalar(StyleKey::BorderRadius);
        let vertical = state.edge.is_vertical();

        ctx.push_debug_scope_rect("Toolbar", rect);

        // --- Background ---
        let panel_bg = s.color(StyleKey::Panel);
        let panel_border = s.color(StyleKey::PanelBorder);
        ctx.draw_list.rounded_rect(rect, border_radius, panel_bg);
        ctx.draw_list
            .rounded_rect_outline(rect, border_radius, 1.0, panel_border);

        // --- Walk the items ---
        let mut cursor = if vertical {
            rect.y + padding
        } else {
            rect.x + padding
        };
        let cross_start = if vertical {
            rect.x + padding
        } else {
            rect.y + padding
        };

        let mut output = ToolbarOutput {
            clicked: None,
            hovered: None,
            grip_dragging: false,
            grip_delta: [0.0, 0.0],
            rect,
        };

        // --- Grip handle ---
        let grip_rect = if vertical {
            Rect::new(rect.x, cursor, rect.width, GRIP_SIZE)
        } else {
            Rect::new(cursor, rect.y, GRIP_SIZE, rect.height)
        };
        self.draw_grip(
            grip_rect,
            vertical,
            drag_capture,
            grip_drag_id,
            &mut output,
            ctx,
        );
        cursor += GRIP_SIZE;

        // --- Tool buttons and separators ---
        for item in self.items {
            match item {
                ToolbarItem::Tool(tool) => {
                    let tool_rect = if vertical {
                        Rect::new(cross_start, cursor, button_size, button_size)
                    } else {
                        Rect::new(cursor, cross_start, button_size, button_size)
                    };
                    // Clamp to available space.
                    if vertical && tool_rect.y + tool_rect.height > rect.y + rect.height {
                        break;
                    }
                    if !vertical && tool_rect.x + tool_rect.width > rect.x + rect.width {
                        break;
                    }
                    self.draw_tool_button(tool, tool_rect, state, &mut output, ctx);
                    cursor += button_size;
                }
                ToolbarItem::Separator => {
                    cursor += SEPARATOR_GAP;
                    let sep_rect = if vertical {
                        let inset = padding + 2.0;
                        Rect::new(
                            rect.x + inset,
                            cursor,
                            rect.width - inset * 2.0,
                            SEPARATOR_THICKNESS,
                        )
                    } else {
                        let inset = padding + 2.0;
                        Rect::new(
                            cursor,
                            rect.y + inset,
                            SEPARATOR_THICKNESS,
                            rect.height - inset * 2.0,
                        )
                    };
                    let sep = if vertical {
                        Separator::horizontal()
                    } else {
                        Separator::vertical()
                    };
                    sep.draw(sep_rect, ctx.draw_list, &s);
                    cursor += SEPARATOR_THICKNESS + SEPARATOR_GAP;
                }
            }
        }

        ctx.pop_debug_scope();
        output
    }

    // --- Internal helpers ---

    fn draw_grip(
        &self,
        rect: Rect,
        vertical: bool,
        capture: &mut DragCapture,
        drag_id: DragId,
        output: &mut ToolbarOutput,
        ctx: &mut DrawContext,
    ) {
        let input = ctx.input;

        // Release-first protocol.
        if !input.mouse_down {
            capture.release(drag_id);
        }

        let hovered = !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);

        if hovered && input.mouse_clicked && capture.is_free() {
            capture.try_begin(drag_id);
        }

        let dragging = capture.is_active(drag_id);
        output.grip_dragging = dragging;
        if dragging {
            output.grip_delta = input.drag_delta;
        }

        // Draw grip dots.
        let dot_color = if dragging {
            ctx.styles().color(StyleKey::Accent)
        } else if hovered {
            ctx.styles().color(StyleKey::TextHighlight)
        } else {
            ctx.styles().color(StyleKey::TextDim)
        };

        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;

        if vertical {
            // Three dots in a horizontal row.
            for dx in [-3.0_f32, 0.0, 3.0] {
                ctx.draw_list.circle((cx + dx, cy), 1.0, dot_color);
            }
        } else {
            // Three dots in a vertical column.
            for dy in [-3.0_f32, 0.0, 3.0] {
                ctx.draw_list.circle((cx, cy + dy), 1.0, dot_color);
            }
        }

        // Cursor.
        if hovered || dragging {
            let icon = if dragging {
                crate::CursorIcon::Grabbing
            } else {
                crate::CursorIcon::Grab
            };
            ctx.request_cursor(icon);
        }
    }

    fn draw_tool_button(
        &self,
        tool: &ToolDef<'_>,
        rect: Rect,
        state: &ToolbarState,
        output: &mut ToolbarOutput,
        ctx: &mut DrawContext,
    ) {
        let input = ctx.input;
        let s = ctx.styles();
        let radius = s.scalar(StyleKey::BorderRadius);

        let is_active = state.active_tool == Some(tool.id);
        let hovered =
            tool.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;

        // Determine tone.
        let tone = if is_active { Tone::Accent } else { Tone::Ghost };

        let mat = Material::new(tone)
            .enabled(tool.enabled)
            .hovered(hovered)
            .pressed(pressed);

        let face = material::draw_with_radius(ctx.draw_list, &s, rect, radius, &mat);

        // Draw the icon centered in the face.
        let icon_inset = 3.0;
        let icon_rect = Rect::new(
            face.x + icon_inset,
            face.y + icon_inset,
            (face.width - icon_inset * 2.0).max(0.0),
            (face.height - icon_inset * 2.0).max(0.0),
        );
        let icon_tint = if is_active {
            s.color(StyleKey::OnAccent)
        } else if tool.enabled {
            s.color(StyleKey::Text)
        } else {
            s.color(StyleKey::TextDim)
        };
        tool.icon.tint(icon_tint).draw(icon_rect, ctx.draw_list);

        // Report hover and click.
        if hovered {
            output.hovered = Some(tool.id);
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        if clicked && tool.enabled {
            output.clicked = Some(tool.id);
        }
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Outcome of drawing a [`Toolbar`].
#[derive(Debug, Clone, Copy)]
pub struct ToolbarOutput {
    /// Tool that was clicked this frame, if any.
    pub clicked: Option<u64>,
    /// Tool that is hovered this frame (for external tooltip display).
    pub hovered: Option<u64>,
    /// Whether the grip handle is being dragged.
    pub grip_dragging: bool,
    /// Drag delta from the grip handle this frame.
    pub grip_delta: [f32; 2],
    /// The rect consumed by the toolbar.
    pub rect: Rect,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Height (or width) of the grip handle zone.
const GRIP_SIZE: f32 = 16.0;

/// Thickness of a separator line.
const SEPARATOR_THICKNESS: f32 = 1.0;

/// Gap on each side of a separator.
const SEPARATOR_GAP: f32 = 3.0;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::layout::Rect;
    use crate::widgets::DrawList;
    use crate::widgets::drag::DragCapture;
    use crate::widgets::focus::FocusState;

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a crate::InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    fn sample_items() -> Vec<ToolbarItem<'static>> {
        vec![
            ToolbarItem::tool(1, Icon::new(crate::render::PhosphorIcon::Plus), "Add", "A"),
            ToolbarItem::tool(
                2,
                Icon::new(crate::render::PhosphorIcon::Minus),
                "Remove",
                "D",
            ),
            ToolbarItem::separator(),
            ToolbarItem::tool(
                3,
                Icon::new(crate::render::PhosphorIcon::Gear),
                "Settings",
                "S",
            ),
        ]
    }

    #[test]
    fn clicking_a_tool_reports_its_id() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        // The first tool button starts after grip (16px) + padding (2px).
        let tool_y = 2.0 + GRIP_SIZE + 12.0; // midpoint of first 24px button
        let tool_x = 14.0;

        let mut input = crate::InputState::default();
        input.mouse_x = tool_x;
        input.mouse_y = tool_y;
        input.mouse_clicked = true;
        input.mouse_down = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(out.clicked, Some(1), "should report first tool's id");
    }

    #[test]
    fn active_tool_draws_accent_material() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        state.active_tool = Some(2);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);
        let input = crate::InputState::default();

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let _out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);

        // The accent material draws chrome instances (plinth + face per button).
        // Verify that at least some chrome was emitted.
        assert!(
            !list.chrome_instances.is_empty(),
            "toolbar should emit chrome instances"
        );
    }

    #[test]
    fn disabled_tool_does_not_report_click() {
        let items = vec![ToolbarItem::Tool(ToolDef {
            id: 1,
            icon: Icon::new(crate::render::PhosphorIcon::Plus),
            label: "Add",
            shortcut: "A",
            enabled: false,
        })];
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        let tool_y = 2.0 + GRIP_SIZE + 12.0;
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = tool_y;
        input.mouse_clicked = true;
        input.mouse_down = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(out.clicked, None, "disabled tool should not report click");
    }

    #[test]
    fn consumed_input_suppresses_hover() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        let tool_y = 2.0 + GRIP_SIZE + 12.0;
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = tool_y;
        input.mouse_consumed = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(out.hovered, None, "consumed input should suppress hover");
    }

    #[test]
    fn horizontal_toolbar_lays_out_left_to_right() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Top);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 300.0, 28.0);

        // The first tool button starts after padding (2px) + grip (16px).
        // Click the midpoint of the first 24px tool button.
        let tool_x = 2.0 + GRIP_SIZE + 12.0;
        let tool_y = 14.0;

        let mut input = crate::InputState::default();
        input.mouse_x = tool_x;
        input.mouse_y = tool_y;
        input.mouse_clicked = true;
        input.mouse_down = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(
            out.clicked,
            Some(1),
            "horizontal layout should find first tool"
        );
    }

    #[test]
    fn grip_drag_reports_delta() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        // Click in the grip zone.
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = 2.0 + GRIP_SIZE * 0.5;
        input.mouse_clicked = true;
        input.mouse_down = true;
        input.drag_delta = [5.0, 3.0];

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert!(out.grip_dragging, "grip should be dragging");
        assert_eq!(out.grip_delta, [5.0, 3.0]);
    }

    #[test]
    fn preferred_extent_accounts_for_all_items() {
        let items = sample_items();
        let toolbar = Toolbar::new(&items);
        let btn = 24.0;
        let pad = 2.0;
        // 3 tools + 1 separator + grip
        let expected =
            GRIP_SIZE + 3.0 * btn + (SEPARATOR_THICKNESS + SEPARATOR_GAP * 2.0) + pad * 2.0;
        let actual = toolbar.preferred_extent(btn, pad);
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {expected}, got {actual}"
        );
    }
}
