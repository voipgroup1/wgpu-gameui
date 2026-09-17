//! Driving the open chain: its own keyboard handling, its column chrome and rows
//! (check gutter, label, hint, chevron, separator), its hit regions, and the
//! activation it resolves.
//!
//! The bar strip itself is drawn by [`MenuBar::draw`](super::MenuBar::draw),
//! because the bar-level keyboard transitions need the menus and their label
//! rects. This module owns everything that happens *inside* the popup layers.

use crate::layout::Rect;
use crate::{
    Affine2, DrawContext, HitShape, InteractionScene, LayerStack, PointerPolicy, StyleKey,
};

use super::model::{ActivatedItem, Menu, MenuBarId, blocker_region_id, column_blocker_id, row_id};
use super::placement;
use super::state::{MenuBarState, MenuDrawEnv, MenuLayers, activation_id};

/// Check-mark stroke width, as a fraction of the row height.
const CHECK_STROKE: f32 = 0.10;
/// Half-extent of the submenu chevron, as a fraction of the row height. Matches
/// the gutter reserved by the geometry pass.
const CHEVRON: f32 = 0.18;

/// Register the viewport blocker as scene regions at the blocker layer.
///
/// `InteractionScene` dispatches among *registered regions* only, so a popup
/// layer alone does not stop a scene-backed base widget from winning pointer
/// dispatch underneath the chain. These regions are the scene half of the
/// blocker, and they exclude the bar strip so the bar's hover-to-switch stays
/// live.
fn register_viewport_blocker(
    scene: &mut InteractionScene,
    bar: MenuBarId,
    layer: usize,
    viewport: Rect,
    bar_rect: Rect,
) {
    let mut regions = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    let count = placement::blocker_regions(viewport, bar_rect, &mut regions);
    // `OrderKey.layer` is the popup layer's index + 1, matching what
    // `DrawContext::interact` derives from `active_layer`.
    let scene_layer = layer as u32 + 1;
    for (index, region) in regions[..count].iter().enumerate() {
        scene.register(
            blocker_region_id(bar, index),
            HitShape::Rect(*region),
            Affine2::IDENTITY,
            None,
            scene_layer,
            true,
            PointerPolicy::Target,
        );
    }
}

/// Drive the open chain, and return the item it activated, if any.
///
/// Activation is reported from exactly here — never from the bar — so there is no
/// ambiguity about which result to read.
///
/// The input half runs whenever a chain is open, whether or not this frame has a
/// paintable column: the geometry is deliberately a frame behind the state (the
/// layer rects that block the base layer can only come from the *previous*
/// frame's measurement), so the frame after a switch has nothing to paint — and
/// it must still take the keyboard, or the arrow key that switched the menu would
/// swallow the next one.
pub(super) fn draw_columns<'a>(
    state: &mut MenuBarState,
    bar: MenuBarId,
    menus: &'a [Menu<'a>],
    layers: &mut LayerStack,
    slots: Option<MenuLayers>,
    env: &mut MenuDrawEnv<'_>,
) -> Option<ActivatedItem<'a>> {
    let menu_index = state.open?;
    let Some(menu) = menus.get(menu_index) else {
        // The bar shrank under the open chain: there is no menu left to draw it
        // from, so drop the chain rather than index past the list.
        state.close();
        return None;
    };
    // Only a chain that was *already* open at frame-top takes input: the edge that
    // opened it must not also move in it.
    let navigable = state.was_open == state.open;

    // ---- input ----
    // Rows move the highlight; the bar's own left/right walk happens in
    // `MenuBar::draw`, which runs earlier and can still measure the column it
    // switches to.
    let mut keyed = false;
    if navigable {
        if state.up {
            keyed |= state.step_item(menu.items(), -1);
        }
        if state.down {
            keyed |= state.step_item(menu.items(), 1);
        }
        if state.cancel {
            // Unwind one level. With nothing left open the bar stays armed, so menu
            // mode survives the first Escape.
            state.open = None;
            state.highlighted_item = None;
            state.scroll = 0.0;
            forget_geometry(state);
            return None;
        }
    }

    // ---- paint ----
    // The promoted geometry is only paintable while it still describes the open
    // menu: the bar may have switched or closed the chain earlier this frame (a
    // label click or hover), leaving the geometry describing a menu nobody has
    // open.
    //
    // The columns are taken out of the state for the duration of the paint: the
    // rows are mutated (their measured text is `take`n) while the rest of the state
    // stays reachable, and putting the buffer back is what lets the next frame's
    // measurement reuse its capacity.
    let mut columns = std::mem::take(&mut state.columns);
    let geom = state
        .geom
        .take()
        .filter(|geom| geom.menu_index == menu_index);
    let Some(geom) = geom else {
        columns.clear();
        state.columns = columns;
        return None;
    };

    // The bar's rect from this frame if it was drawn, else the promoted one.
    let bar_rect = if state.bar_rect.is_empty() {
        geom.bar_rect
    } else {
        state.bar_rect
    };
    let viewport = if state.viewport.is_empty() {
        geom.viewport
    } else {
        state.viewport
    };
    if let Some(slots) = slots {
        register_viewport_blocker(env.interactions, bar, slots.blocker, viewport, bar_rect);
    }

    let mut hovered_row = None;
    let mut clicked_row = None;
    let mut column_click = false;

    for (level, column) in columns.iter_mut().enumerate() {
        state.scroll_into_view(column);

        // ---- register this column's hit regions ----
        let index = match slots {
            Some(slots) => slots.blocker + 1 + level,
            None => {
                // A host that drew the bar but never pushed the layers still gets
                // a visible, interactive column; it just cannot block input this
                // frame.
                let index = layers.push_popup(column.rect);
                layers.pop_layer();
                index
            }
        };
        let rect = column.rect;
        let row_h = column.row_h;
        let label_avail = column.label_avail;
        let hint_right = column.hint_right;
        let menu_id = menus.get(column.menu_index).and_then(Menu::id_value);

        {
            let mut ctx = DrawContext::new(
                &mut layers.layers_mut()[index].list,
                env.focus,
                env.theme,
                env.input,
                env.screen_width,
                env.screen_height,
            );
            ctx.active_layer = Some(index);
            ctx.style = env.style;
            ctx.animations = env.animations.as_deref_mut();
            ctx.cursor = env.cursor.as_deref_mut();
            ctx.interactions = Some(&mut *env.interactions);

            // The column's own blocker, registered *before* the rows so the rows
            // win dispatch within the layer: a scene-backed base widget under the
            // column's padding, border or a separator would otherwise still win
            // hover and click.
            ctx.interact(column_blocker_id(bar, column.menu_index, level), rect, true);

            ctx.draw_list.push_debug_scope_rect("Menu column", rect);
            // Keep row washes inside the raised sheet so a highlighted first or
            // last row cannot cover the same outer edge an idle row exposes.
            let row_paint_rect = rect.inset(ctx.styles().scalar(StyleKey::BorderWidth).max(1.0));
            ctx.draw_list.push_clip_viewport(row_paint_rect);

            // Rows: register for dispatch (culling the scrolled-out band and
            // everything that can never be highlighted) and collect what the
            // previous frame resolved.
            for row in column.rows.iter() {
                if row.separator || row.disabled {
                    continue;
                }
                let y = rect.y + row.item_index as f32 * row_h - state.scroll;
                if y + row_h <= rect.y || y >= rect.bottom() {
                    continue;
                }
                let response = ctx.interact(
                    row_id(bar, column.menu_index, menu_id, row.item_index, level),
                    Rect::new(rect.x, y, rect.width, row_h),
                    true,
                );
                if response.hovered {
                    hovered_row = Some(row.item_index);
                    ctx.request_cursor(crate::CursorIcon::Pointer);
                }
                if response.clicked {
                    clicked_row = Some(row.item_index);
                }
            }

            // The pointer takes the highlight back only when it actually moved,
            // so a stationary pointer over the column doesn't fight arrow keys.
            if state.pointer_moved
                && !keyed
                && let Some(item) = hovered_row
            {
                state.highlighted_item = Some(item);
            }

            // ---- paint ----
            let s = ctx.styles();
            let list = &mut *ctx.draw_list;
            // The menu sheet: the raised panel (4a blurred-sheet look reads as a
            // near-black panel with a black edge; apps can add a backdrop blur
            // behind it via `UiRenderer::blur_backdrop`).
            list.chrome_rect(
                rect,
                s.scalar(StyleKey::BorderRadius),
                s.scalar(StyleKey::BorderWidth),
                s.color(StyleKey::Panel),
                [0.0, 0.0, 0.0, 0.7],
            );
            let dim = rgb(s.color(StyleKey::TextDim));
            let text = rgb(s.color(StyleKey::Text));
            let selected_bg = s.color(StyleKey::Accent);
            let border_width = s.scalar(StyleKey::BorderWidth).max(1.0);
            let check_w = column.check_w;

            for row in column.rows.iter_mut() {
                let y = rect.y + row.item_index as f32 * row_h - state.scroll;
                if y + row_h <= rect.y || y >= rect.bottom() {
                    continue;
                }
                if row.separator {
                    list.quad(
                        rect.x + column.pad,
                        y + row_h * 0.5 - border_width * 0.5,
                        (rect.width - column.pad * 2.0).max(0.0),
                        border_width,
                        s.color(StyleKey::PanelBorder),
                    );
                    continue;
                }
                if row.checked {
                    let stroke = (row_h * CHECK_STROKE).max(1.0);
                    let (x, w, h) = (rect.x + column.pad, check_w, row_h);
                    let color = s.color(StyleKey::Text);
                    list.line(
                        [x + w * 0.16, y + h * 0.52],
                        [x + w * 0.40, y + h * 0.76],
                        stroke,
                        color,
                    );
                    list.line(
                        [x + w * 0.40, y + h * 0.76],
                        [x + w * 0.84, y + h * 0.22],
                        stroke,
                        color,
                    );
                }
                if row.submenu {
                    let half = row_h * CHEVRON;
                    let cx = rect.right() - column.pad - half * 0.6;
                    let cy = y + row_h * 0.5;
                    list.triangle(
                        (cx - half * 0.5, cy - half),
                        (cx - half * 0.5, cy + half),
                        (cx + half * 0.7, cy),
                        s.color(if row.disabled {
                            StyleKey::TextDim
                        } else {
                            StyleKey::Text
                        }),
                    );
                }
                if state.highlighted_item == Some(row.item_index) && !row.disabled {
                    // The design's menu selection: a translucent accent wash.
                    let mut c = selected_bg;
                    c[3] = 0.2;
                    list.quad(rect.x, y, rect.width, row_h, c);
                    if row.checked {
                        // Selected + checked: solid accent row with dark text.
                        list.quad(rect.x, y, rect.width, row_h, selected_bg);
                    }
                }
                if row.checked && state.highlighted_item != Some(row.item_index) {
                    // Latched filter row: accent wash.
                    let mut c = selected_bg;
                    c[3] = 0.16;
                    list.quad(rect.x, y, rect.width, row_h, c);
                }

                let (r, g, b) = if row.disabled { dim } else { text };
                if let Some(measured) = row.label.take() {
                    let ty = y + row_h * 0.5 - measured.metrics.visual_center;
                    let mut block = measured.into_block_at(rect.x + column.pad + check_w, ty);
                    if row.label_w > label_avail {
                        // Narrower than its intrinsic text (a column clamped by
                        // the viewport): truncate rather than overflow.
                        block = block.with_max_width(label_avail).with_ellipsis();
                    }
                    list.text(block.with_color(r, g, b));
                }
                if let Some(measured) = row.hint.take() {
                    let ty = y + row_h * 0.5 - measured.metrics.visual_center;
                    let x = hint_right - measured.metrics.size[0];
                    list.text(
                        measured
                            .into_block_at(x, ty)
                            .with_color(dim.0, dim.1, dim.2),
                    );
                }
            }

            list.pop_clip();
            list.pop_debug_scope();
        }

        // A press anywhere inside the column is the menu's, not the widget's:
        // clicking padding or a separator keeps the menu open instead of
        // counting as a click elsewhere.
        if state.mouse_clicked && rect.contains(state.mouse_x, state.mouse_y) {
            column_click = true;
        }
    }

    // ---- resolve ----
    let mut activated = None;
    if navigable {
        let chosen = clicked_row.or(if state.confirm {
            state.highlighted_item
        } else {
            None
        });
        if let Some(item_index) = chosen
            && let Some(item) = menu.items().get(item_index)
            && item.is_enabled()
            && !item.is_submenu()
        {
            state.click_claimed = true;
            activated = Some(ActivatedItem {
                id: activation_id(menu, item),
                item,
            });
            // Acting on an item ends the interaction: the chain closes and the
            // bar leaves menu mode.
            state.close();
        }
    }
    if column_click {
        state.click_claimed = true;
    }
    // Retained for the next frame (and for `debug_geometry`): a chain that is
    // still open keeps what was last drawn, one that closed or switched during the
    // paint does not. `close` already dropped the state's copy, but the columns
    // were out of the state for the duration of the paint.
    state.columns = columns;
    if state.open != Some(menu_index) {
        state.columns.clear();
    }
    activated
}

/// Drop the promoted chain geometry, keeping the buffer's capacity. Called when
/// the chain unwinds a level: what was measured describes the level that closed.
fn forget_geometry(state: &mut MenuBarState) {
    state.geom = None;
    state.columns.clear();
}

/// Convert a theme color to 8-bit RGB, as the text primitives want.
fn rgb(c: [f32; 4]) -> (u8, u8, u8) {
    (
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}
