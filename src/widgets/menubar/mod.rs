//! Menubar: a horizontal strip of top-level menus, one of which can drop a
//! popup column of items.
//!
//! ## Why this is shaped the way it is
//!
//! A menubar has the same two hard requirements as [`crate::Dropdown`] — the
//! open column must render **above** later widgets and must **block** presses
//! from reaching whatever sits under it — plus two of its own:
//!
//! * **Arming is a mode.** A tap of Alt (the default trigger) puts the bar into
//!   menu mode, where arrows walk the bar and Enter opens the highlighted menu.
//!   The trigger need not stay held, and a bare tap *leaves* the bar armed and
//!   visibly highlighted. That is why [`MenuBarState`] owns an `armed` flag
//!   rather than reading a held key, and why Alt carries press/release edges on
//!   [`InputState`](crate::InputState): a tap landing entirely between two
//!   rendered frames would be invisible to held-only state.
//! * **The bar must stay live while a chain is open.** Hovering a different
//!   label has to switch menus, so the blocker that swallows outside presses
//!   deliberately leaves a hole for the strip.
//!
//! ## Two-phase, like the dropdown
//!
//! The open chain cannot be drawn by the call that draws the bar: a popup layer
//! only blocks input if it is pushed *before* the base layer is resolved. So the
//! geometry is measured while the bar draws and promoted at the next frame's
//! frame-top:
//!
//! ```ignore
//! // per frame, in this order
//! menu.begin_frame(&mut input);                   // claims nav intents it will act on
//! let slots = menu.push_open_layers(&mut layers); // popups block base input from here
//! let base = layers.input_for_base(&input);
//! // draw the base layer, including the bar:
//! let out = MenuBar::new(MENU_BAR, MENUS).draw(bar_rect, &mut menu, &mut ctx);
//! // after the base scope, draw the chain and read the single activation channel:
//! if let Some(item) = menu.draw_open_layers(&mut layers, slots, MENUS, &mut env) { /* … */ }
//! menu.end_frame(&mut focus);                     // before FocusState::end_frame
//! ```
//!
//! Consequences, accepted and tested: a menu that opens this frame shows its
//! column on the next (~16 ms), exactly like a `Dropdown`; a level that closed
//! this frame still owns its hit regions for one more frame.
//!
//! ## Pointer support needs an [`InteractionScene`](crate::InteractionScene)
//!
//! Bar labels and rows resolve through retained interaction dispatch, not through
//! raw hit tests. That is not a style choice: while a chain is open the base
//! layer's input is *consumed*, and a scene response ignores
//! [`InputState::mouse_consumed`](crate::InputState::mouse_consumed) — which is
//! precisely what keeps hover-to-switch working under the blocker. Attach a scene
//! with [`DrawContext::with_interactions`]; without one the bar and its rows draw
//! but have **no** pointer support (Alt arming and keyboard navigation still
//! work).
//!
//! ## Theme
//!
//! No new colour keys: popups reuse `Panel`/`PanelBorder`, row text `Text`,
//! hover and the armed label `ButtonHover`, the open menu's label `Accent`,
//! hints and disabled items `TextDim`, separators `PanelBorder` + `BorderWidth`.
//! Sizing comes from three scalars: [`StyleKey::MenuRowHeight`],
//! [`StyleKey::MenuItemMinWidth`] and [`StyleKey::MenuAccelGap`].
//!
//! ## Not yet
//!
//! Submenus are *rendered* (a parent row shows its chevron) but not opened: the
//! chain is one level deep. Hover intent, the corridor rule, mnemonics and
//! accelerator dispatch are later phases of `docs/design/menubar.md`.

mod model;
mod paint;
mod placement;
mod state;
#[cfg(test)]
mod tests;

pub use model::{
    AccelPlatform, Accelerator, ActivatedItem, Key, Menu, MenuBarId, MenuItem, MenuItemId,
    MenuTrigger, Modifiers, SubmenuSide,
};
pub use placement::{blocker_regions, place_popup};
pub use state::{MenuBarState, MenuDrawEnv, MenuLayers};

use crate::layout::{Constraint, Rect};
use crate::text::{TextBlock, WrapMode};
use crate::{
    DrawContext, MeasureConstraints, MeasureContext, Measurement, StyleKey, StyleResolver,
};

use model::{MenuBarId as BarId, bar_label_id};
use state::ChainGeometry;

/// A horizontal strip of menus, drawn into a caller-supplied [`Rect`].
///
/// Borrowed and cheap to build per frame, like the rest of the crate's widgets:
/// the menu tree it points at can be a `static`, and all mutable state lives in
/// the caller's [`MenuBarState`].
pub struct MenuBar<'a> {
    id: BarId,
    menus: &'a [Menu<'a>],
    platform: AccelPlatform,
    side: SubmenuSide,
}

/// What a bar draw resolved. Deliberately **not** an activation: rows live in
/// popup layers and are resolved later in the frame by
/// [`MenuBarState::draw_open_layers`], and two activation channels would be
/// ambiguous.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuBarOutput {
    /// The strip's rect, as drawn (its height is the themed row height unless the
    /// caller supplied one).
    pub bar_rect: Rect,
    /// Whether the bar is in menu mode after this draw.
    pub armed: bool,
    /// The open top-level menu, if a chain is open.
    pub open_menu: Option<usize>,
    /// The bar label under the pointer, if any.
    pub hovered_menu: Option<usize>,
}

impl<'a> MenuBar<'a> {
    /// A bar of `menus`, namespaced by `id` within the surface's shared
    /// [`InteractionScene`](crate::InteractionScene).
    pub const fn new(id: BarId, menus: &'a [Menu<'a>]) -> Self {
        Self {
            id,
            menus,
            platform: AccelPlatform::Pc,
            side: SubmenuSide::Auto,
        }
    }

    /// Set the word/glyph set used to render accelerator hints (`Pc` by default).
    pub const fn platform(mut self, platform: AccelPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// Set the preferred side columns extend to (`Auto` by default).
    pub const fn submenu_side(mut self, side: SubmenuSide) -> Self {
        self.side = side;
        self
    }

    /// The menus this bar shows.
    pub fn menus(&self) -> &'a [Menu<'a>] {
        self.menus
    }

    /// Contextual measurement of the strip: one [`StyleKey::MenuRowHeight`] tall,
    /// as wide as the sum of the labels plus their insets.
    pub fn measure(&self, cx: &mut MeasureContext<'_>) -> Measurement {
        let styles = cx.styles();
        let pad = styles.scalar(StyleKey::Padding).max(0.0);
        let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
        let mut width = 0.0;
        let mut baseline = row_h * 0.5;
        for menu in self.menus {
            let block = styles
                .text_block(menu.label(), 0.0, 0.0)
                .with_wrap(WrapMode::None);
            let text = cx.measure_text_with(block, MeasureConstraints::UNBOUNDED);
            width += text.metrics.size[0] + pad * 2.0;
            baseline = (row_h * 0.5 - text.metrics.visual_center + text.metrics.baseline)
                .clamp(0.0, row_h);
        }
        let constraints = cx.constraints();
        let preferred = [
            Constraint {
                min: Some(constraints.min_width),
                max: constraints.max_width,
            }
            .apply(width),
            Constraint {
                min: Some(constraints.min_height),
                max: constraints.max_height,
            }
            .apply(row_h),
        ];
        Measurement::new([0.0, row_h], preferred, [None, Some(row_h)], Some(baseline))
    }

    /// Draw the bar strip and resolve clicks on its labels. Base layer only.
    ///
    /// The bar-level keyboard transitions live here (left/right walk the
    /// highlight, down/confirm open the highlighted menu) because this is the only
    /// place that has both the menus and their label rects.
    ///
    /// The strip paints **no background of its own** — it composes over whatever
    /// chrome the host drew, as [`crate::Tabs`] does. A label is filled only when
    /// hovered, armed-highlighted, or when its menu is open (`Accent`, so menu
    /// mode is never invisible).
    pub fn draw(
        &self,
        rect: Rect,
        state: &mut MenuBarState,
        ctx: &mut DrawContext<'_>,
    ) -> MenuBarOutput {
        let theme = ctx.theme;
        let overlay = ctx.style;
        let styles = StyleResolver::with_overlay_opt(theme, overlay);
        let pad = styles.scalar(StyleKey::Padding).max(0.0);
        let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
        let height = if rect.height > 0.0 {
            rect.height
        } else {
            row_h
        };
        let strip = Rect::new(rect.x, rect.y, rect.width.max(0.0), height);
        let viewport = Rect::new(
            0.0,
            0.0,
            ctx.screen_width.max(0.0),
            ctx.screen_height.max(0.0),
        );
        state.bar_rect = strip;
        state.viewport = viewport;
        state.bar = self.id;

        if self.menus.is_empty() {
            state.next_geom = None;
            return self.output(state, None);
        }

        // Pass 1: intrinsic label widths, so every label rect is known before a
        // single hit region is registered. The measurement is cached, so the paint
        // pass's own `TextBlock` build is the only per-frame text cost.
        state.label_pool.clear();
        {
            let list = &mut *ctx.draw_list;
            for menu in self.menus {
                let block = styles.text_block(menu.label(), 0.0, 0.0);
                let (width, _) = list.measure_block(&block);
                state.label_pool.push(width + pad * 2.0);
            }
        }

        // Pass 2: hit regions, and the state changes they imply. The single
        // pointer can only be over one label, so resolving each label as it is
        // visited is equivalent to collecting them first — and it keeps the pass
        // allocation-free. The press is resolved before the hover, or opening a
        // menu on press would be undone by hover-to-switch in the same frame.
        let mut label_x = strip.x;
        let mut hovered_menu = None;
        for (index, menu) in self.menus.iter().enumerate() {
            let label_rect = Rect::new(label_x, strip.y, state.label_pool[index], strip.height);
            label_x = label_rect.right();
            let response =
                ctx.interact(bar_label_id(self.id, index), label_rect, menu.is_enabled());
            if response.clicked {
                state.click_claimed = true;
                if state.open_menu() == Some(index) {
                    // Clicking the open menu's own label is a full toggle: chain
                    // closed, bar out of menu mode.
                    state.close();
                } else {
                    state.open_menu_at(self.menus, index);
                }
            } else if response.hovered {
                hovered_menu = Some(index);
                if state.open_menu().is_some() {
                    // Hovering another label switches the open menu immediately.
                    if state.open_menu() != Some(index) {
                        state.open_menu_at(self.menus, index);
                    }
                } else if state.armed {
                    // Armed with nothing open: moving the highlight, not opening.
                    state.highlighted_menu = Some(index);
                }
            }
        }
        state.hovered_menu = hovered_menu;

        // Bar-level keyboard. Uses the highlight resolved above, so a highlight
        // that went stale (a disabled menu) is repaired on the same frame.
        //
        // With a chain open, left/right walk the bar and leave the chain open —
        // the mirror of "right on a leaf moves to the next top-level menu". This
        // lives here, not in the chain's own pass, because it has to land before
        // the geometry pass below measures the column it switches to.
        if state.open_menu().is_some() {
            if state.left {
                state.switch_menu(self.menus, -1);
            }
            if state.right {
                state.switch_menu(self.menus, 1);
            }
        } else if state.armed {
            if state.highlighted_menu.is_none()
                || !state
                    .highlighted_menu
                    .and_then(|index| self.menus.get(index))
                    .is_some_and(Menu::is_enabled)
            {
                state.highlighted_menu = state.first_enabled_menu(self.menus);
            }
            if state.left
                && let Some(from) = state.highlighted_menu
            {
                state.highlighted_menu = state.step_menu(self.menus, from, -1);
            }
            if state.right
                && let Some(from) = state.highlighted_menu
            {
                state.highlighted_menu = state.step_menu(self.menus, from, 1);
            }
            if (state.down || state.confirm)
                && let Some(index) = state.highlighted_menu
            {
                state.open_menu_at(self.menus, index);
            }
        }

        // Pass 3: paint. Nothing is filled unless it is hovered, highlighted or
        // open, so the strip leaves the host's own chrome showing through.
        let (accent, hover, text, dim) = (
            styles.color(StyleKey::Accent),
            styles.color(StyleKey::ButtonHover),
            styles.color(StyleKey::Text),
            styles.color(StyleKey::TextDim),
        );
        let font_size = styles.scalar(StyleKey::FontSize);
        let font = theme.font.clone();
        let (open_menu, armed, highlighted, hovered) = (
            state.open_menu(),
            state.armed,
            state.highlighted_menu,
            state.hovered_menu,
        );
        let mut label_x = strip.x;
        {
            let list = &mut *ctx.draw_list;
            list.push_debug_scope_rect("MenuBar strip", strip);
            for (index, menu) in self.menus.iter().enumerate() {
                let label_rect = Rect::new(label_x, strip.y, state.label_pool[index], strip.height);
                label_x = label_rect.right();

                let fill = if open_menu == Some(index) {
                    Some(accent)
                } else if (armed && highlighted == Some(index)) || hovered == Some(index) {
                    Some(hover)
                } else {
                    None
                };
                if let Some(fill) = fill {
                    list.quad(
                        label_rect.x,
                        label_rect.y,
                        label_rect.width,
                        label_rect.height,
                        fill,
                    );
                }
                let color = if menu.is_enabled() { text } else { dim };
                let ty = list.vcentered_text_y(
                    label_rect.y,
                    label_rect.height,
                    font_size,
                    font.as_ref(),
                    menu.label(),
                );
                let (r, g, b) = rgb(color);
                list.text(
                    TextBlock::new(menu.label(), label_rect.x + pad, ty)
                        .with_size(font_size)
                        .with_color(r, g, b)
                        .with_font_opt(font.clone()),
                );
            }
            list.pop_debug_scope();
        }

        // Stage the chain's geometry for the next frame's popup layers. The anchor
        // is the open menu's label rect; with nothing open this clears it.
        let anchor = open_menu.and_then(|open| self.label_rect(strip, state, open));
        let anchor = anchor.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        {
            let list = &mut *ctx.draw_list;
            state.collect_chain(
                ChainGeometry {
                    menus: self.menus,
                    bar_rect: strip,
                    anchor,
                    viewport,
                    platform: self.platform,
                    side: self.side,
                },
                list,
                theme,
                overlay,
            );
        }

        self.output(state, state.hovered_menu)
    }

    /// The rect of label `index` under the current label widths.
    fn label_rect(&self, strip: Rect, state: &MenuBarState, index: usize) -> Option<Rect> {
        let width = *state.label_pool.get(index)?;
        let x = strip.x
            + state
                .label_pool
                .iter()
                .take(index)
                .fold(0.0, |acc, w| acc + w);
        Some(Rect::new(x, strip.y, width, strip.height))
    }

    fn output(&self, state: &MenuBarState, hovered_menu: Option<usize>) -> MenuBarOutput {
        MenuBarOutput {
            bar_rect: state.bar_rect,
            armed: state.armed,
            open_menu: state.open,
            hovered_menu,
        }
    }
}

fn rgb(color: [f32; 4]) -> (u8, u8, u8) {
    (
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
    )
}
