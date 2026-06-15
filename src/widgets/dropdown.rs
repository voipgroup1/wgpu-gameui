//! Dropdown (select / combo-box): a button that, when open, floats a list of
//! options above everything else and lets the user pick one.
//!
//! ## Why this is shaped the way it is
//! In an immediate-mode UI the open option list has two hard requirements that
//! a plain inline widget can't meet: it must render **above** later widgets
//! (z-order) and it must **block** clicks from reaching whatever sits under it.
//! Both are solved by the [`LayerStack`] `Popup` layer — but a popup only
//! blocks input correctly if it is pushed *before* the base widgets are drawn,
//! exactly like the modal in `examples/hello_ui.rs`. So the open list can't be
//! drawn from the same call that draws the button.
//!
//! The split mirrors the [`crate::TooltipLayer`] manager pattern crossed with
//! the modal's push-at-frame-top: a caller-owned [`DropdownState`] (single
//! owner — at most one dropdown open at a time, like [`crate::FocusState`] /
//! [`crate::DragCapture`]) holds the open id plus the geometry it needs to draw
//! the list. [`Dropdown::draw`] renders only the **button** inline; the open
//! **list** is deferred to [`DropdownState::draw_open_layer`], drawn into a
//! `Popup` layer that [`DropdownState::push_open_layer`] established at the top
//! of the frame.
//!
//! ```ignore
//! let mut dropdowns = DropdownState::new();
//! let mut focus = FocusState::new();
//! // --- per frame ---
//! dropdowns.begin_frame(&raw_input);
//! focus.begin_frame(&raw_input);
//! let popup = dropdowns.push_open_layer(&mut layers);   // from LAST frame's geometry
//! let base_input = layers.input_for_base(&raw_input);   // blocks clicks under the open list
//! // draw base widgets, including the dropdown button:
//! let mut ctx = DrawContext::new(layers.base_mut(), &mut focus, &theme, &base_input, w, h);
//! Dropdown::new(&items, sel).draw(MY_ID, rect, &mut dropdowns, &mut ctx);
//! drop(ctx); // release the borrow on layers.base_mut()
//! // after the base scope:
//! let style = StyleResolver::new(&theme);
//! if let Some((id, idx)) = dropdowns.draw_open_layer(&mut layers, popup, &style, &raw_input) {
//!     if id == MY_ID { sel = idx; }
//! }
//! dropdowns.end_frame();
//! focus.end_frame(None);
//! ```
//!
//! ## Timing
//! Because the popup is pushed at frame-top from the *previous* frame's
//! geometry, a freshly-opened list first appears on the next frame (~16 ms).
//! This is the same one-frame latency the focus model's Tab navigation has and
//! is imperceptible in practice. Selecting an item and closing are same-frame.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{InputState, LayerStack, StyleKey, StyleResolver};

use super::{DrawContext, DrawList};

/// Stable identity for a dropdown within one UI surface. Any scheme that is
/// unique per dropdown per frame works (a hash, an enum discriminant, a loop
/// index). `0` is a valid id.
pub type DropdownId = u64;

/// Height of one option row, in pixels.
const ITEM_HEIGHT: f32 = 28.0;
/// Options shown before the list becomes scrollable.
const DEFAULT_MAX_VISIBLE: usize = 8;
/// Vertical gap between the button and the floating list.
const GAP_BELOW_BUTTON: f32 = 2.0;
/// Half-width / half-height of the chevron glyph drawn at the button's right.
const CHEVRON: f32 = 4.0;

fn rgb(c: [f32; 4]) -> (u8, u8, u8) {
    (
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

/// Everything the deferred list draw needs, snapshotted while the open
/// dropdown's button draws. Cloned once per frame for the single open dropdown
/// (closed dropdowns snapshot nothing).
#[derive(Debug, Clone)]
struct OpenGeom {
    id: DropdownId,
    button_rect: Rect,
    items: Vec<String>,
    selected: usize,
    width: f32,
    item_h: f32,
    max_visible: usize,
}

/// Arbitrates which dropdown (if any) is open, and carries the geometry needed
/// to float its list. Caller-owned; persists across frames. Construct one per
/// UI surface and drive it with [`begin_frame`](Self::begin_frame) /
/// [`push_open_layer`](Self::push_open_layer) /
/// [`draw_open_layer`](Self::draw_open_layer) / [`end_frame`](Self::end_frame)
/// each frame, the same way the crate threads [`crate::FocusState`].
#[derive(Debug, Default, Clone)]
pub struct DropdownState {
    /// The dropdown that is currently open, if any.
    open: Option<DropdownId>,
    /// Geometry used to push + draw the list THIS frame (collected last frame).
    geom: Option<OpenGeom>,
    /// Geometry collected during THIS frame's button draws; promoted to `geom`
    /// at the next `begin_frame`. The one-frame delay is what lets
    /// `push_open_layer` know the list rect at frame-top.
    next_geom: Option<OpenGeom>,
    /// Vertical scroll offset of the open list (pixels), for long lists.
    scroll_offset: f32,
    /// Keyboard-hovered item index within the open list (defaults to `selected`
    /// when the dropdown opens).
    highlighted: usize,
    // ---- per-frame input edges, captured in begin_frame (mirrors FocusState) ----
    escape: bool,
    mouse_clicked: bool,
    click_claimed: bool,
    key_up: bool,
    key_down: bool,
    enter: bool,
}

impl DropdownState {
    /// A fresh owner with nothing open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a frame: promote the geometry collected last frame into the
    /// view used this frame, and capture this frame's Esc/keyboard/click edges.
    /// Call once per frame, before [`push_open_layer`](Self::push_open_layer).
    pub fn begin_frame(&mut self, input: &InputState) {
        self.geom = self.next_geom.take();
        self.escape = input.key_escape;
        self.mouse_clicked = input.mouse_clicked;
        self.click_claimed = false;
        self.key_up = input.key_up;
        self.key_down = input.key_down;
        self.enter = input.enter_pressed;
    }

    /// True when `id` is the open dropdown.
    pub fn is_open(&self, id: DropdownId) -> bool {
        self.open == Some(id)
    }

    /// The open dropdown, if any.
    pub fn open(&self) -> Option<DropdownId> {
        self.open
    }

    /// Force the open dropdown closed.
    pub fn close(&mut self) {
        self.open = None;
        self.geom = None;
        self.next_geom = None;
        self.scroll_offset = 0.0;
        self.highlighted = 0;
    }

    /// The floating list rect for `geom` — directly below the button, as wide
    /// as the button, tall enough for up to `max_visible` rows.
    fn list_rect(geom: &OpenGeom) -> Rect {
        let visible = geom.items.len().min(geom.max_visible).max(1);
        Rect::new(
            geom.button_rect.x,
            geom.button_rect.y + geom.button_rect.height + GAP_BELOW_BUTTON,
            geom.width,
            visible as f32 * geom.item_h,
        )
    }

    /// Push the `Popup` layer for the open dropdown using last frame's geometry,
    /// returning its layer index. Call once per frame, at frame-top, **before**
    /// dispatching `input_for_base` — that is what lets the popup block clicks
    /// to widgets underneath the open list. Returns `None` when nothing is open
    /// (or on the first frame a dropdown opens, before its geometry is known).
    ///
    /// The layer is pushed then immediately popped: it stays in the stack (so
    /// `input_for_base`/`input_for_layer` account for it) and is drawn into by
    /// index in [`draw_open_layer`](Self::draw_open_layer).
    pub fn push_open_layer(&mut self, layers: &mut LayerStack) -> Option<usize> {
        if self.open.is_none() {
            return None;
        }
        let geom = self.geom.as_ref()?;
        let rect = Self::list_rect(geom);
        let idx = layers.push_popup(rect);
        layers.pop_layer();
        Some(idx)
    }

    /// Draw the open list into the popup layer pushed by
    /// [`push_open_layer`](Self::push_open_layer) and resolve clicks on it. Call
    /// once per frame, after the base widgets are drawn. Returns
    /// `Some((id, index))` when an option was clicked or Enter-pressed (the
    /// dropdown closes), and `None` otherwise. `popup` is the index returned by
    /// `push_open_layer`; when `None` but a dropdown is open (same-frame open),
    /// this method pushes a popup layer on the fly.
    pub fn draw_open_layer(
        &mut self,
        layers: &mut LayerStack,
        popup: Option<usize>,
        style: &StyleResolver,
        input: &InputState,
    ) -> Option<(DropdownId, usize)> {
        let geom = self.geom.clone()?;
        let list_rect = Self::list_rect(&geom);

        // For same-frame open, `push_open_layer` didn't run yet — push a popup
        // now so we have a DrawList to draw into. It won't block input this
        // frame (that requires being pushed before input_for_base), but the
        // visual list appears immediately.
        let idx = match popup {
            Some(i) => i,
            None => {
                let i = layers.push_popup(list_rect);
                layers.pop_layer();
                i
            }
        };

        // Input as the popup layer sees it (consumed if a higher layer covers
        // the cursor — e.g. a modal above the dropdown).
        let li = layers.input_for_layer(idx, input);

        // Scrolling for long lists.
        let content_h = geom.items.len() as f32 * geom.item_h;
        let max_scroll = (content_h - list_rect.height).max(0.0);
        if max_scroll > 0.0 && li.scroll_delta != 0.0 && list_rect.contains(li.mouse_x, li.mouse_y)
        {
            self.scroll_offset -= li.scroll_delta * geom.item_h;
        }
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);
        let scroll = self.scroll_offset;

        // ---- Keyboard navigation ----
        // Arrow keys move the highlight. This uses the raw input (not
        // layer-dispatched) because keyboard events aren't positional.
        if !geom.items.is_empty() {
            if input.key_up {
                self.highlighted = self.highlighted.saturating_sub(1);
                // Scroll to keep highlighted in view.
                let top = self.highlighted as f32 * geom.item_h;
                let bottom = top + geom.item_h;
                if top < self.scroll_offset {
                    self.scroll_offset = top;
                } else if bottom > self.scroll_offset + list_rect.height {
                    self.scroll_offset = bottom - list_rect.height;
                }
            }
            if input.key_down {
                self.highlighted = (self.highlighted + 1).min(geom.items.len() - 1);
                let top = self.highlighted as f32 * geom.item_h;
                let bottom = top + geom.item_h;
                if top < self.scroll_offset {
                    self.scroll_offset = top;
                } else if bottom > self.scroll_offset + list_rect.height {
                    self.scroll_offset = bottom - list_rect.height;
                }
            }
        }

        let (sel_r, sel_g, sel_b) = rgb(style.color(StyleKey::Background));
        let (txt_r, txt_g, txt_b) = rgb(style.color(StyleKey::Text));
        let pad = style.scalar(StyleKey::Padding);
        let font_size = style.scalar(StyleKey::FontSize);
        let font = style.theme().font.clone();
        let accent = style.color(StyleKey::Accent);
        let hover = style.color(StyleKey::ButtonHover);

        {
            let l = &mut layers.layers_mut()[idx].list;
            // List background + border.
            l.chrome_rect(
                list_rect,
                style.scalar(StyleKey::BorderRadius),
                style.scalar(StyleKey::BorderWidth),
                style.color(StyleKey::Panel),
                style.color(StyleKey::PanelBorder),
            );
            l.push_clip(list_rect);
            for (i, item) in geom.items.iter().enumerate() {
                let iy = list_rect.y + i as f32 * geom.item_h - scroll;
                // Cull rows fully outside the viewport.
                if iy + geom.item_h <= list_rect.y || iy >= list_rect.y + list_rect.height {
                    continue;
                }
                let hovered = list_rect.contains(li.mouse_x, li.mouse_y)
                    && li.mouse_y >= iy
                    && li.mouse_y < iy + geom.item_h
                    && !li.mouse_consumed;
                let is_selected = i == geom.selected;
                let is_highlighted = self.highlighted == i;
                if is_selected {
                    l.quad(list_rect.x, iy, list_rect.width, geom.item_h, accent);
                } else if is_highlighted {
                    // Keyboard highlight: a brighter/stronger hover.
                    l.quad(list_rect.x, iy, list_rect.width, geom.item_h, hover);
                } else if hovered {
                    // Mouse hover only when keyboard isn't already highlighting
                    // a different item (to avoid fighting the user).
                    if !input.key_up && !input.key_down {
                        self.highlighted = i;
                    }
                    l.quad(list_rect.x, iy, list_rect.width, geom.item_h, hover);
                }
                let (r, g, b) = if is_selected {
                    (sel_r, sel_g, sel_b)
                } else {
                    (txt_r, txt_g, txt_b)
                };
                let text_y =
                    l.vcentered_text_y(iy, geom.item_h, font_size, font.as_ref(), item);
                l.text(
                    TextBlock::new(item.clone(), list_rect.x + pad, text_y)
                        .with_size(font_size)
                        .with_color(r, g, b)
                        .with_max_width(list_rect.width - pad * 2.0)
                        .with_font_opt(font.clone()),
                );
            }
            l.pop_clip();
        }

        // Resolve a click on the list.
        let mut result: Option<(DropdownId, usize)> = None;
        if li.mouse_clicked && list_rect.contains(li.mouse_x, li.mouse_y) {
            // A click anywhere inside the popup is "claimed" so end_frame's
            // click-elsewhere blur doesn't also fire.
            self.click_claimed = true;
            let rel = li.mouse_y - list_rect.y + scroll;
            let row = (rel / geom.item_h).floor();
            if row >= 0.0 && (row as usize) < geom.items.len() {
                result = Some((geom.id, row as usize));
                self.close();
            }
        }
        // Enter on the highlighted item selects it.
        if input.enter_pressed && !geom.items.is_empty() {
            let row = self.highlighted;
            if row < geom.items.len() {
                result = Some((geom.id, row));
                // Claim the click so end_frame doesn't also close us.
                self.click_claimed = true;
                self.close();
            }
        }
        result
    }

    /// End a frame: close the dropdown on Escape or on a click that no dropdown
    /// claimed (click-elsewhere-to-dismiss). Call once per frame, after
    /// [`draw_open_layer`](Self::draw_open_layer).
    pub fn end_frame(&mut self) {
        if self.escape || (self.mouse_clicked && !self.click_claimed) {
            self.close();
        }
    }

    /// Test/seed helper: open `id` with the given options so the floating list
    /// renders immediately (skips the one-frame open latency). Used by the
    /// gallery to snapshot an open dropdown.
    #[doc(hidden)]
    pub fn open_for_test(
        &mut self,
        id: DropdownId,
        button_rect: Rect,
        items: &[&str],
        selected: usize,
    ) {
        let geom = OpenGeom {
            id,
            button_rect,
            items: items.iter().map(|s| s.to_string()).collect(),
            selected,
            width: button_rect.width,
            item_h: ITEM_HEIGHT,
            max_visible: DEFAULT_MAX_VISIBLE,
        };
        self.open = Some(id);
        self.geom = Some(geom.clone());
        self.next_geom = Some(geom);
    }
}

/// Per-frame configuration for one dropdown. Lightweight, like
/// [`crate::Tabs`] / [`crate::Slider`] — built fresh each frame.
pub struct Dropdown<'a> {
    items: &'a [&'a str],
    selected: usize,
    max_visible: usize,
}

/// Result of drawing a dropdown's button.
pub struct DropdownOutput {
    /// The button was clicked this frame.
    pub clicked: bool,
    /// The dropdown is open after this draw.
    pub open: bool,
}

impl<'a> Dropdown<'a> {
    /// A dropdown over `items`, displaying `items[selected]` on the button.
    pub fn new(items: &'a [&'a str], selected: usize) -> Self {
        Self {
            items,
            selected,
            max_visible: DEFAULT_MAX_VISIBLE,
        }
    }

    /// Number of options shown before the open list becomes scrollable.
    pub fn with_max_visible(mut self, n: usize) -> Self {
        self.max_visible = n.max(1);
        self
    }

    /// The screen-space rect the open option list occupies when this dropdown is
    /// open below `button_rect` — it floats directly below the button and is as
    /// tall as `min(items, max_visible)` rows.
    ///
    /// The live widget draws the list into a popup layer (so it overlays, not
    /// reserves), but this lets a caller that *does* need the footprint reason
    /// about it: reserve layout space beneath the button, or decide whether to
    /// flip the list upward when it would run off-screen.
    pub fn open_list_rect(&self, button_rect: Rect) -> Rect {
        // Mirror the private `list_rect` geometry (same GAP_BELOW_BUTTON /
        // ITEM_HEIGHT / max_visible clamp) from this frame's config.
        let visible = self.items.len().min(self.max_visible).max(1);
        Rect::new(
            button_rect.x,
            button_rect.y + button_rect.height + GAP_BELOW_BUTTON,
            button_rect.width,
            visible as f32 * ITEM_HEIGHT,
        )
    }

    /// Draw the dropdown **button** at `rect` and handle open/close toggling.
    /// The open option list is drawn separately by
    /// [`DropdownState::draw_open_layer`]. Returns whether the button was
    /// clicked and whether the dropdown is now open.
    pub fn draw(
        &self,
        id: DropdownId,
        rect: Rect,
        state: &mut DropdownState,
        ctx: &mut DrawContext,
    ) -> DropdownOutput {
        // Register as focusable for Tab nav (scoped to active layer).
        ctx.register_focus(id);

        // Hand cursor over the closed dropdown button (before borrowing ctx).
        if ctx.input.is_hovered(rect.x, rect.y, rect.width, rect.height) {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }

        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        let input = ctx.input;

        let hovered = input.is_hovered(rect.x, rect.y, rect.width, rect.height);
        let clicked = hovered && input.mouse_clicked;

        // Activate on click, Space, or Enter when focused.
        let focused = ctx.focus.is_focused(id);
        let keyboard_activate = focused && (input.enter_pressed || input.key_space);

        // Clicking the button toggles this dropdown (single owner: opening one
        // implicitly leaves any other to be closed by its own end_frame).
        if clicked {
            if state.is_open(id) {
                state.close();
            } else {
                state.open = Some(id);
                state.highlighted = self.selected;
                state.scroll_offset = (self.selected as f32 * ITEM_HEIGHT).max(0.0);
                state.click_claimed = true;
            }
        } else if keyboard_activate {
            // Keyboard open — same-frame open by setting geom immediately.
            if state.is_open(id) {
                state.close();
            } else {
                state.open = Some(id);
                state.highlighted = self.selected;
                state.scroll_offset = (self.selected as f32 * ITEM_HEIGHT).max(0.0);
                let geom = OpenGeom {
                    id,
                    button_rect: rect,
                    items: self.items.iter().map(|s| s.to_string()).collect(),
                    selected: self.selected,
                    width: rect.width,
                    item_h: ITEM_HEIGHT,
                    max_visible: self.max_visible,
                };
                state.geom = Some(geom.clone());
                state.next_geom = Some(geom);
                // Claim the click too, to avoid end_frame closing us.
                state.click_claimed = true;
            }
        }
        let open = state.is_open(id);

        // Button chrome: focus-style border when open, accent on hover.
        let border = if open {
            s.color(StyleKey::InputFocusBorder)
        } else if hovered {
            s.color(StyleKey::Accent)
        } else {
            s.color(StyleKey::InputBorder)
        };
        list.chrome_rect(
            rect,
            s.scalar(StyleKey::BorderRadius),
            s.scalar(StyleKey::BorderWidth),
            s.color(StyleKey::InputBackground),
            border,
        );

        // Selected label.
        let label = self.items.get(self.selected).copied().unwrap_or("");
        let (r, g, b) = rgb(if self.items.is_empty() {
            s.color(StyleKey::TextDim)
        } else {
            s.color(StyleKey::Text)
        });
        let font_size = s.scalar(StyleKey::FontSize);
        let pad = s.scalar(StyleKey::Padding);
        let text_y = list.vcentered_text_y(
            rect.y,
            rect.height,
            font_size,
            s.theme().font.as_ref(),
            label,
        );
        list.text(
            TextBlock::new(label, rect.x + pad, text_y)
                .with_size(font_size)
                .with_color(r, g, b)
                .with_max_width(rect.width - pad * 2.0 - CHEVRON * 3.0)
                .with_font_opt(s.theme().font.clone()),
        );

        // Chevron at the right edge: down when closed, up when open.
        draw_chevron(list, rect, &s, open);

        // Snapshot geometry for the deferred list draw (this frame's
        // draw_open_layer reads last frame's; next begin_frame promotes this).
        if open {
            state.next_geom = Some(OpenGeom {
                id,
                button_rect: rect,
                items: self.items.iter().map(|s| s.to_string()).collect(),
                selected: self.selected,
                width: rect.width,
                item_h: ITEM_HEIGHT,
                max_visible: self.max_visible,
            });
        }

        DropdownOutput { clicked, open }
    }
}

/// Draw a small chevron (▼ closed / ▲ open) at the button's right edge.
fn draw_chevron(list: &mut DrawList, rect: Rect, s: &StyleResolver, open: bool) {
    let cx = rect.x + rect.width - s.scalar(StyleKey::Padding) - CHEVRON;
    let cy = rect.y + rect.height / 2.0;
    let dy = if open { -CHEVRON * 0.5 } else { CHEVRON * 0.5 };
    let color = s.color(StyleKey::TextDim);
    // Two strokes meeting at the point.
    list.line([cx - CHEVRON, cy - dy], [cx, cy + dy], 1.5, color);
    list.line([cx + CHEVRON, cy - dy], [cx, cy + dy], 1.5, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, Theme};

    fn input(clicked: bool, escape: bool, mouse: (f32, f32)) -> InputState {
        InputState {
            mouse_x: mouse.0,
            mouse_y: mouse.1,
            mouse_clicked: clicked,
            key_escape: escape,
            ..Default::default()
        }
    }

    const ITEMS: [&str; 3] = ["Red", "Green", "Blue"];
    fn rect_a() -> Rect {
        Rect::new(10.0, 10.0, 120.0, 28.0)
    }
    fn rect_b() -> Rect {
        Rect::new(10.0, 80.0, 120.0, 28.0)
    }

    /// Drive one full frame: begin, draw both buttons, end. Returns nothing —
    /// inspect `state` afterwards.
    fn frame(state: &mut DropdownState, focus: &mut FocusState, theme: &Theme, inp: &InputState) {
        focus.begin_frame(inp);
        state.begin_frame(inp);
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, focus, theme, inp, 800.0, 600.0);
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), state, &mut ctx);
        Dropdown::new(&ITEMS, 0).draw(2, rect_b(), state, &mut ctx);
        state.end_frame();
        focus.end_frame(None);
    }

    #[test]
    fn open_list_rect_floats_below_button_sized_to_rows() {
        let btn = rect_a();
        let menu = Dropdown::new(&ITEMS, 0).open_list_rect(btn);
        // Floats just below the button, same x/width.
        assert_eq!(menu.x, btn.x);
        assert_eq!(menu.width, btn.width);
        assert_eq!(menu.y, btn.y + btn.height + GAP_BELOW_BUTTON);
        // Height = all 3 rows (under the default max_visible).
        assert_eq!(menu.height, ITEMS.len() as f32 * ITEM_HEIGHT);
    }

    #[test]
    fn open_list_rect_clamps_height_to_max_visible() {
        let btn = rect_a();
        let menu = Dropdown::new(&ITEMS, 0)
            .with_max_visible(2)
            .open_list_rect(btn);
        assert_eq!(menu.height, 2.0 * ITEM_HEIGHT, "scrollable list caps at max_visible rows");
    }

    #[test]
    fn fresh_state_has_nothing_open() {
        let s = DropdownState::new();
        assert_eq!(s.open(), None);
        assert!(!s.is_open(1));
    }

    #[test]
    fn clicking_button_toggles_open() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        // Click inside button A.
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 20.0)),
        );
        assert!(s.is_open(1));
        // Click it again → closes.
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 20.0)),
        );
        assert!(!s.is_open(1));
        assert_eq!(s.open(), None);
    }

    #[test]
    fn single_owner_opening_b_replaces_a() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 20.0)),
        ); // open A
        assert!(s.is_open(1));
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 90.0)),
        ); // click B
        assert!(s.is_open(2));
        assert!(!s.is_open(1));
    }

    #[test]
    fn escape_closes() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 20.0)),
        ); // open A
        assert!(s.is_open(1));
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(false, true, (20.0, 20.0)),
        ); // Esc
        assert!(!s.is_open(1));
    }

    #[test]
    fn click_elsewhere_closes() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 20.0)),
        ); // open A
        assert!(s.is_open(1));
        // Click far from any button → unclaimed → end_frame closes.
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (500.0, 500.0)),
        );
        assert!(!s.is_open(1));
    }

    #[test]
    fn no_click_keeps_open() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        frame(
            &mut s,
            &mut focus,
            &theme,
            &input(true, false, (20.0, 20.0)),
        ); // open A
        frame(&mut s, &mut focus, &theme, &input(false, false, (0.0, 0.0))); // idle
        assert!(s.is_open(1));
    }

    #[test]
    fn open_list_appears_next_frame_and_selecting_returns_index() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        let mut layers = LayerStack::new();

        // Frame 1: open A. At frame-top there's no geometry yet, so no popup.
        focus.begin_frame(&input(true, false, (20.0, 20.0)));
        s.begin_frame(&input(true, false, (20.0, 20.0)));
        let popup = s.push_open_layer(&mut layers);
        assert_eq!(popup, None, "freshly-opened list has no layer this frame");
        let mut list = DrawList::new();
        let input_open = input(true, false, (20.0, 20.0));
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input_open, 800.0, 600.0);
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), &mut s, &mut ctx);
        drop(ctx);
        let pick = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &input_open);
        assert_eq!(pick, None);
        s.end_frame();
        focus.end_frame(None);
        assert!(s.is_open(1));

        // Frame 2: geometry promoted → popup pushed; click the 2nd row ("Green").
        layers.clear();
        // List sits at y = 10 + 28 + 2 = 40; row 1 spans [68, 96).
        let click_row1 = input(true, false, (20.0, 80.0));
        focus.begin_frame(&click_row1);
        s.begin_frame(&click_row1);
        let popup = s.push_open_layer(&mut layers);
        assert!(popup.is_some(), "open list has a layer on the next frame");
        let mut list = DrawList::new();
        let input_idle = input(false, false, (20.0, 80.0));
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input_idle, 800.0, 600.0);
        // Button isn't clicked this frame (cursor is over the list, not the button).
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), &mut s, &mut ctx);
        drop(ctx);
        let pick = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &click_row1);
        assert_eq!(pick, Some((1, 1)), "clicking row 1 returns (id=1, index=1)");
        s.end_frame();
        focus.end_frame(None);
        assert!(!s.is_open(1), "selecting an option closes the dropdown");
    }

    /// Input helper that sets arrow-up or arrow-down.
    fn input_keys(up: bool, down: bool, enter: bool) -> InputState {
        InputState {
            key_up: up,
            key_down: down,
            enter_pressed: enter,
            ..Default::default()
        }
    }

    #[test]
    fn arrow_down_moves_highlighted_in_open_list() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        let inp = input(true, false, (20.0, 20.0));
        focus.begin_frame(&inp);
        s.begin_frame(&inp);
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &inp, 800.0, 600.0);
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), &mut s, &mut ctx);
        drop(ctx);
        s.end_frame();
        focus.end_frame(None);
        assert!(s.is_open(1));

        // Frame 2: arrow down twice.
        let kbd = input_keys(false, true, false);
        focus.begin_frame(&kbd);
        s.begin_frame(&kbd);
        let mut layers = LayerStack::new();
        let popup = s.push_open_layer(&mut layers);
        // draw_open_layer processes arrow-down from input
        let _ = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &kbd);
        assert_eq!(s.highlighted, 1, "arrow down moves to index 1");

        let kbd2 = input_keys(false, true, false);
        let _ = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &kbd2);
        assert_eq!(s.highlighted, 2, "arrow down moves to index 2");
        s.end_frame();
    }

    #[test]
    fn arrow_up_moves_highlighted() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        let inp = input(true, false, (20.0, 20.0));
        focus.begin_frame(&inp);
        s.begin_frame(&inp);
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &inp, 800.0, 600.0);
        Dropdown::new(&ITEMS, 2).draw(1, rect_a(), &mut s, &mut ctx); // selected = 2 → highlighted starts at 2
        drop(ctx);
        s.end_frame();
        focus.end_frame(None);
        assert!(s.is_open(1));
        assert_eq!(s.highlighted, 2);

        // Frame 2: arrow up.
        let kbd = input_keys(true, false, false);
        focus.begin_frame(&kbd);
        s.begin_frame(&kbd);
        let mut layers = LayerStack::new();
        let popup = s.push_open_layer(&mut layers);
        let _ = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &kbd);
        assert_eq!(s.highlighted, 1, "arrow up moves to index 1");
        s.end_frame();
    }

    #[test]
    fn enter_selects_highlighted_item() {
        let theme = Theme::default();
        let mut s = DropdownState::new();
        let mut focus = FocusState::new();
        let inp = input(true, false, (20.0, 20.0));
        focus.begin_frame(&inp);
        s.begin_frame(&inp);
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &inp, 800.0, 600.0);
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), &mut s, &mut ctx);
        drop(ctx);
        s.end_frame();
        focus.end_frame(None);
        assert!(s.is_open(1));

        // Frame 2: draw button (not clicked) + arrow down.
        let kbd = InputState {
            key_down: true,
            enter_pressed: false,
            ..Default::default()
        };
        focus.begin_frame(&kbd);
        s.begin_frame(&kbd);
        let mut layers = LayerStack::new();
        let popup = s.push_open_layer(&mut layers);
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &kbd, 800.0, 600.0);
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), &mut s, &mut ctx);
        drop(ctx);
        let _ = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &kbd);
        assert_eq!(s.highlighted, 1);
        s.end_frame();
        focus.end_frame(None);

        // Frame 3: draw button (not clicked) + Enter selects highlighted (index 1).
        let enter = InputState {
            enter_pressed: true,
            ..Default::default()
        };
        focus.begin_frame(&enter);
        s.begin_frame(&enter);
        let popup = s.push_open_layer(&mut layers);
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &enter, 800.0, 600.0);
        Dropdown::new(&ITEMS, 0).draw(1, rect_a(), &mut s, &mut ctx);
        drop(ctx);
        let result = s.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &enter);
        assert_eq!(result, Some((1, 1)), "Enter selects highlighted item");
        s.end_frame();
        assert!(!s.is_open(1), "selecting closes the dropdown");
    }
}
