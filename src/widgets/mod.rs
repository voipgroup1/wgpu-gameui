//! UI widgets - buttons, text inputs, panels, etc.

mod button;
mod checkbox;
mod drag;
mod drag_handle;
mod draw_list;
mod dropdown;
mod focus;
#[cfg(feature = "phosphor-icons")]
mod icon;
mod image;
mod image_button;
mod list;
mod number_input;
mod panel;
mod progress_bar;
mod scroll_view;
mod slider;
mod table;
mod tabs;
mod text_input;
mod tooltip;
mod tree;

pub use button::Button;
pub use checkbox::{CHECKBOX_CHECKED_ICON, CHECKBOX_ICON, Checkbox};
pub use drag::{DragCapture, DragId};
pub use drag_handle::{DragHandle, DragHandleOutput};
pub(crate) use draw_list::ColorCmd;
#[cfg(feature = "phosphor-icons")]
pub use draw_list::IconMsdf;
pub use draw_list::{
    ChromeInstance, CircleInstance, DrawList, IconDraw, NineSliceDraw, NineSliceId, Vertex,
};
pub use dropdown::{Dropdown, DropdownId, DropdownOutput, DropdownState};
pub use focus::{FocusId, FocusState};
#[cfg(feature = "phosphor-icons")]
pub use icon::Icon;
pub use image::{Image, ImageAlign, ImageFit};
pub use image_button::ImageButton;
pub use list::{List, ListItem, ListOutput, ListState, SelectionMode};
pub use number_input::{NumberInput, NumberOutput};
pub use panel::{Panel, label, label_at, label_centered_at, title, title_at};
pub use progress_bar::ProgressBar;
pub use scroll_view::{ScrollBegin, ScrollState, ScrollView};
pub use slider::{Slider, SliderOutput};
pub use table::{Align, ColumnWidth, Table, TableCell, TableColumn, TableOutput};
pub use tabs::{Tabs, TabsOutput};
pub use text_input::TextInput;
pub use tooltip::{TooltipContent, TooltipLayer};
pub use tree::{TreeAction, TreeIcon, TreeId, TreeNode, TreeNodeOutput, TreeState};

use crate::{InputState, Theme};

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
    pub draw_list: &'a mut DrawList,
    pub focus: &'a mut FocusState,
    pub theme: &'a Theme,
    pub input: &'a InputState,
    pub screen_width: f32,
    pub screen_height: f32,
    /// When drawing into a specific layer (modal/popup), set this to the
    /// layer index so [`register_focus`](Self::register_focus) automatically
    /// scopes the focusable to that layer's Tab ring.
    pub active_layer: Option<usize>,
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
        }
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
        self.draw_list.rounded_rect_outline(
            rect,
            self.theme.border_radius,
            2.0,
            self.theme.focus_ring,
        );
    }
}
