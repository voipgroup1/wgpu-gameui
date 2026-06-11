//! UI widgets - buttons, text inputs, panels, etc.

mod button;
mod checkbox;
mod drag;
mod draw_list;
mod dropdown;
mod focus;
mod image;
mod image_button;
mod panel;
mod progress_bar;
mod scroll_view;
mod slider;
mod table;
mod tabs;
mod text_input;
mod tooltip;

pub use button::Button;
pub use checkbox::{Checkbox, CHECKBOX_CHECKED_ICON, CHECKBOX_ICON};
pub use drag::{DragCapture, DragId};
pub(crate) use draw_list::ColorCmd;
pub use draw_list::{
    ChromeInstance, CircleInstance, DrawList, IconDraw, NineSliceDraw, NineSliceId, Vertex,
};
pub use dropdown::{Dropdown, DropdownId, DropdownOutput, DropdownState};
pub use focus::{FocusId, FocusState};
pub use image::{Image, ImageAlign, ImageFit};
pub use image_button::ImageButton;
pub use panel::{label, label_at, label_centered_at, title, title_at, Panel};
pub use progress_bar::ProgressBar;
pub use scroll_view::{ScrollBegin, ScrollState, ScrollView};
pub use slider::{Slider, SliderOutput, SLIDER_SCRUBBER_ICON, SLIDER_TRACK_NINE_SLICE};
pub use table::{Align, ColumnWidth, Table, TableCell, TableColumn, TableOutput};
pub use tabs::{Tabs, TabsOutput};
pub use text_input::TextInput;
pub use tooltip::{TooltipContent, TooltipLayer};

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
}
