//! UI widgets - buttons, text inputs, panels, etc.

mod app_shell;
mod asset_grid;
mod badge;
mod banner;
mod breadcrumb;
mod busy;
mod button;
mod checkbox;
mod color_picker;
mod combo_box;
mod curve_editor;
mod doc_tabs;
mod dock_panel;
mod drag;
mod drag_handle;
mod draw_list;
mod dropdown;
mod focus;
mod gradient_ramp;
mod group;
mod hit_zone;
#[cfg(feature = "phosphor-icons")]
mod icon;
mod image;
mod image_button;
mod list;
mod material;
mod menu_screens;
mod binding;
mod settings;
mod menubar;
mod number_input;
mod panel;
mod popover;
mod progress_bar;
mod radio;
mod scroll_view;
mod separator;
mod slider;
mod splitter;
mod status_bar;
mod table;
mod tabs;
mod tag_input;
mod text_input;
mod toast;
mod toggle;
mod toolbar;
mod tooltip;
mod tree;
mod vector_field;

pub use app_shell::{
    AppShell, SHELL_DRAG_BOTTOM_SPLITTER, SHELL_DRAG_LEFT_SPLITTER, SHELL_DRAG_RIGHT_SPLITTER,
    SHELL_DRAG_TOOLBAR_GRIP, ShellChromeOutput, ShellLayout,
};
pub use asset_grid::{AssetGrid, AssetGridOutput};
pub use badge::{ChipOutput, badge, chip, keycap};
pub use banner::{Banner, Severity};
pub use breadcrumb::{Breadcrumb, Pager, PagerOutput};
pub use busy::{EmptyState, dots, empty_state, skeleton, spinner};
pub use button::Button;
pub use checkbox::{CHECKBOX_CHECKED_ICON, CHECKBOX_ICON, Checkbox};
pub use color_picker::{ColorPicker, ColorPickerOutput};
pub use combo_box::{
    ComboOutput, draw_list as draw_combo_list, draw_trigger as draw_combo_trigger,
};
pub use curve_editor::{CurveOutput, draw as draw_curve_editor};
pub use doc_tabs::{DocTab, DocTabsOutput, draw as draw_doc_tabs};
pub use dock_panel::{DockPanel, DockPanelOutput, DockPanelState, DockSide, DockTab};
pub use drag::{DragCapture, DragId};
pub use drag_handle::{DragHandle, DragHandleOutput};
#[cfg(feature = "phosphor-icons")]
pub use draw_list::IconMsdf;
pub(crate) use draw_list::PaintCmd;
pub use draw_list::{
    ChromeInstance, CircleInstance, DebugScope, DrawList, IconDraw, NineSliceDraw, NineSliceId,
    PrimCounts, Vertex,
};
pub use dropdown::{Dropdown, DropdownId, DropdownOutput, DropdownState};
pub use focus::{FocusId, FocusState};
pub use gradient_ramp::{
    GradientStop, RampOutput, draw as draw_gradient_ramp, readout as gradient_ramp_readout,
    sample as sample_ramp,
};
pub use group::Group;
pub use hit_zone::{HitZone, HitZoneOutput};
#[cfg(feature = "phosphor-icons")]
pub use icon::Icon;
pub use image::{Image, ImageAlign, ImageFit};
pub use image_button::ImageButton;
pub use list::{List, ListItem, ListOutput, ListState, SelectionMode};
pub(crate) use material::sheen_over;
pub use material::{Material, Tone};
pub use menu_screens::{MenuList, MenuListOutput, draw_scrim};
pub use binding::{Binding, KeyCode, PadButton};
pub use settings::{
    default_values, SettingField, SettingValue, SettingsForm, SettingsFormOutput, SettingsSpec,
    SettingsFormState,
};
pub use menubar::{
    AccelPlatform, Accelerator, ActivatedItem, Key, Menu, MenuBar, MenuBarId, MenuBarOutput,
    MenuBarState, MenuDrawEnv, MenuItem, MenuItemId, MenuLayers, MenuTrigger, Modifiers,
    SubmenuSide, blocker_regions, place_popup,
};
pub use number_input::{NumberInput, NumberOutput};
pub use panel::{Panel, label, label_at, label_centered_at, title, title_at};
pub use popover::{Popover, PopoverOutput, PopoverSide, measure_sheet_height, place_popover};
pub use progress_bar::{ProgressBar, ProgressFill};
pub use radio::RadioGroup;
pub use scroll_view::{ScrollBegin, ScrollState, ScrollView};
pub use separator::{Orientation, Separator};
pub use slider::{Slider, SliderOutput};
pub use splitter::{SplitAxis, Splitter, SplitterOutput};
pub use status_bar::{STATUS_BAR_HEIGHT, StatusCell, draw as draw_status_bar};
pub use table::{Align, ColumnWidth, Table, TableCell, TableColumn, TableOutput};
pub use tabs::{Tabs, TabsOutput};
pub use tag_input::{TagOutput, draw as draw_tag_input};
pub use text_input::{ClipboardGet, ClipboardSet, TextInput};
pub use toast::{Corner, DEFAULT_TTL, Toast, ToastStack};
pub use toggle::Toggle;
pub use toolbar::{ToolDef, Toolbar, ToolbarEdge, ToolbarItem, ToolbarOutput, ToolbarState};
pub use tooltip::{TooltipContent, TooltipLayer};
pub use tree::{TreeAction, TreeIcon, TreeId, TreeNode, TreeNodeOutput, TreeState};
pub use vector_field::{AXIS_TINTS, VectorField, VectorFieldOutput, VectorScrub};

use crate::{
    AnimSlot, AnimationState, Easing, InputState, StyleKey, StyleOverlay, StyleResolver, Theme,
};

/// Context for drawing UI elements.
///
/// Bundles the per-frame resources a widget needs: mutable draw list + focus
/// state, plus read-only theme, input, and screen dimensions. Callers construct
/// one per frame (or per layer dispatch) and pass it to every widget; Rust's
/// borrow checker tracks each field independently when they come from separate
/// originals, so `ctx.draw_list` and `ctx.focus` can be `.`-accessed freely.
///
/// When drawing into a modal/popup layer, set [`active_layer`](Self::active_layer)
/// to that layer's index so focus registration is automatically scoped.
pub struct DrawContext<'a> {
    /// Mutable draw list the widget emits geometry into.
    pub draw_list: &'a mut DrawList,
    /// Mutable focus/Tab-ring state for focusable widgets.
    pub focus: &'a mut FocusState,
    /// Read-only theme backing style resolution.
    pub theme: &'a Theme,
    /// Read-only per-frame input snapshot.
    pub input: &'a InputState,
    /// Screen width in pixels.
    pub screen_width: f32,
    /// Screen height in pixels.
    pub screen_height: f32,
    /// When drawing into a specific layer (modal/popup), set this to the
    /// layer index so [`register_focus`](Self::register_focus) automatically
    /// scopes the focusable to that layer's Tab ring.
    pub active_layer: Option<usize>,
    /// Optional scoped style overrides layered over [`theme`](Self::theme).
    /// `None` (the default) resolves every key straight from the theme; set via
    /// [`with_style`](Self::with_style) so a caller can recolor a subtree without
    /// cloning the theme. Widgets read styles through [`color`](Self::color) /
    /// [`scalar`](Self::scalar), which consult this first.
    pub style: Option<&'a StyleOverlay>,
    /// Optional caller-owned animation clock for smoothing hover/press color
    /// transitions. `None` (the default) makes every state change instant
    /// (byte-identical to the un-animated path); set via
    /// [`with_animations`](Self::with_animations). Widgets that take a stable id
    /// read eased values through [`animate_color`](Self::animate_color) /
    /// [`animate_scalar`](Self::animate_scalar).
    pub animations: Option<&'a mut AnimationState>,
    /// Optional caller-owned cursor-request accumulator. `None` (the default)
    /// makes [`request_cursor`](Self::request_cursor) a no-op; set via
    /// [`with_cursor`](Self::with_cursor) so hovered widgets can ask for an
    /// I-beam / hand / grab cursor. The application reads the resolved
    /// [`CursorIcon`](crate::CursorIcon) after the frame and applies it to its
    /// window.
    pub cursor: Option<&'a mut crate::CursorState>,
    /// Optional retained interaction scene. When attached, widgets with stable
    /// IDs register their local geometry here and receive topmost-first responses
    /// resolved against the previous completed frame.
    pub interactions: Option<&'a mut crate::InteractionScene>,
}

impl<'a> DrawContext<'a> {
    /// Create a new draw context with all required per-frame resources.
    pub fn new(
        draw_list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
        screen_width: f32,
        screen_height: f32,
    ) -> Self {
        Self {
            draw_list,
            focus,
            theme,
            input,
            screen_width,
            screen_height,
            active_layer: None,
            style: None,
            animations: None,
            cursor: None,
            interactions: None,
        }
    }

    /// Layer `overlay` over the theme for this context: every [`color`](Self::color)
    /// / [`scalar`](Self::scalar) lookup consults the overlay first, so a caller
    /// can restyle the widgets drawn through this context without cloning the
    /// theme. Builder-style; chain after [`new`](Self::new).
    pub fn with_style(mut self, overlay: &'a StyleOverlay) -> Self {
        self.style = Some(overlay);
        self
    }

    /// Attach a caller-owned [`AnimationState`] so widgets that take a stable id
    /// ease their hover/press color changes instead of switching instantly.
    /// Builder-style; chain after [`new`](Self::new). The state must be
    /// [`tick`](AnimationState::tick)ed once per frame by the caller.
    pub fn with_animations(mut self, animations: &'a mut AnimationState) -> Self {
        self.animations = Some(animations);
        self
    }

    /// Attach a caller-owned [`CursorState`](crate::CursorState) so hovered
    /// widgets can request an OS cursor shape (I-beam over text, hand over
    /// buttons, grab over drag handles). Builder-style; chain after
    /// [`new`](Self::new). The caller [`begin_frame`](crate::CursorState::begin_frame)s
    /// it before drawing and applies [`resolve`](crate::CursorState::resolve)
    /// to its window afterwards.
    pub fn with_cursor(mut self, cursor: &'a mut crate::CursorState) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Attach the retained interaction scene for this surface. Widgets that use
    /// [`interact`](Self::interact) then share their exact local allocation,
    /// active transform, clip, and layer ordering with hit testing.
    pub fn with_interactions(mut self, interactions: &'a mut crate::InteractionScene) -> Self {
        self.interactions = Some(interactions);
        self
    }

    /// Register a rectangular interactive allocation and return the response
    /// resolved from the same widget ID in the previous completed frame.
    pub fn interact(
        &mut self,
        id: impl Into<crate::WidgetId>,
        rect: crate::layout::Rect,
        enabled: bool,
    ) -> crate::Response {
        let id = id.into();
        let transform = self.draw_list.current_transform();
        let clip = self.draw_list.current_clip();
        let layer = self.active_layer.map_or(0, |index| index as u32 + 1);
        match self.interactions.as_deref_mut() {
            Some(scene) => scene.register(
                id,
                crate::HitShape::Rect(rect),
                transform,
                clip,
                layer,
                enabled,
                crate::PointerPolicy::Target,
            ),
            None => crate::Response::idle(id, rect),
        }
    }

    /// Whether this context has retained interaction dispatch attached.
    pub fn has_interactions(&self) -> bool {
        self.interactions.is_some()
    }

    /// Request an OS cursor shape for this frame on behalf of a hovered widget.
    /// No-op when no [`CursorState`](crate::CursorState) is attached (so the
    /// un-wired path is unaffected). Conflicts are arbitrated by
    /// [`CursorState::request`](crate::CursorState::request).
    pub fn request_cursor(&mut self, icon: crate::CursorIcon) {
        if let Some(cursor) = self.cursor.as_deref_mut() {
            cursor.request(icon);
        }
    }

    /// Eased color to draw this frame for `(id, slot)` walking toward `target`,
    /// using the theme's [`AnimationDuration`](StyleKey::AnimationDuration) and a
    /// default ease-out curve. Returns `target` unchanged when no
    /// [`AnimationState`] is attached (so the un-animated path is byte-identical).
    ///
    /// Borrow note: this takes `&mut self`, so call it (and store the returned
    /// value) **before** taking `let list = &mut *ctx.draw_list` — never
    /// interleave it with a live mutable borrow of `draw_list`.
    pub fn animate_color(&mut self, id: u64, slot: AnimSlot, target: [f32; 4]) -> [f32; 4] {
        let duration = self.scalar(StyleKey::AnimationDuration);
        match self.animations.as_deref_mut() {
            Some(anim) => anim.animate_color(id, slot, target, duration, Easing::EaseOut),
            None => target,
        }
    }

    /// Scalar counterpart of [`animate_color`](Self::animate_color) (e.g. a hover
    /// overlay's alpha). Same borrow note applies.
    pub fn animate_scalar(&mut self, id: u64, slot: AnimSlot, target: f32) -> f32 {
        let duration = self.scalar(StyleKey::AnimationDuration);
        match self.animations.as_deref_mut() {
            Some(anim) => anim.animate_scalar(id, slot, target, duration, Easing::EaseOut),
            None => target,
        }
    }

    /// A [`StyleResolver`] bound to this context's theme + optional overlay — the
    /// single resolution path widgets read through.
    pub fn styles(&self) -> StyleResolver<'a> {
        StyleResolver::with_overlay_opt(self.theme, self.style)
    }

    /// Resolve a built-in color [`StyleKey`] (overlay → theme). Equivalent to the
    /// old direct `theme.<field>` read when no overlay is set.
    pub fn color(&self, key: StyleKey) -> [f32; 4] {
        self.styles().color(key)
    }

    /// Resolve a built-in scalar [`StyleKey`] (overlay → theme).
    pub fn scalar(&self, key: StyleKey) -> f32 {
        self.styles().scalar(key)
    }

    /// Resolve a color key, falling back to `default` when unset (for `Custom`
    /// keys).
    pub fn color_or(&self, key: StyleKey, default: [f32; 4]) -> [f32; 4] {
        self.styles().color_or(key, default)
    }

    /// Resolve a scalar key, falling back to `default` when unset (for `Custom`
    /// keys).
    pub fn scalar_or(&self, key: StyleKey, default: f32) -> f32 {
        self.styles().scalar_or(key, default)
    }

    /// Register `id` as focusable in the active layer (or base if no layer).
    /// Convenience that delegates to [`FocusState::register`] or
    /// [`FocusState::register_layer`] based on [`active_layer`](Self::active_layer).
    pub fn register_focus(&mut self, id: FocusId) {
        match self.active_layer {
            Some(layer) => self.focus.register_layer(id, layer),
            None => self.focus.register(id),
        }
    }

    /// Draw the keyboard-focus ring around `rect`: a 2px rounded outline in
    /// [`Theme::focus_ring`]. Focusable widgets call this when they hold focus so
    /// every widget gets a consistent focus indicator from one place.
    pub fn draw_focus_ring(&mut self, rect: crate::layout::Rect) {
        let radius = self.scalar(StyleKey::BorderRadius);
        let color = self.color(StyleKey::FocusRing);
        self.draw_list
            .rounded_rect_outline(rect, radius, 2.0, color);
    }

    /// Open a debug scope declaring the box this widget was allocated — see
    /// [`DrawList::push_debug_scope_rect`].
    ///
    /// Widgets call this with the `Rect` they were handed so the report can tell
    /// whether they stayed inside it. Applications never need to.
    pub fn push_debug_scope_rect(&mut self, name: impl Into<String>, rect: crate::layout::Rect) {
        self.draw_list.push_debug_scope_rect(name, rect);
    }

    /// Close the innermost debug scope.
    pub fn pop_debug_scope(&mut self) {
        self.draw_list.pop_debug_scope();
    }
}

/// Build a widget scope name: the type name, plus the author-written label in
/// quotes when the widget has one (`Button "Save"`).
///
/// The type leads so names stay greppable and machine-splittable; the label is
/// what makes [`DebugReport::node`](crate::debug::DebugReport::node) — which is
/// first-match-wins — actually able to address one button out of thirty.
///
/// Only ever pass an author-written label (a title, a caption), never
/// content the user typed or a long message: scope names are not truncated.
pub(crate) fn scope_name(kind: &str, label: &str) -> String {
    if label.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {label:?}")
    }
}
