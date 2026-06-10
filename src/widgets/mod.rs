//! UI widgets - buttons, text inputs, panels, etc.

mod button;
mod checkbox;
mod drag;
mod draw_list;
mod image;
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
pub use draw_list::{DrawList, IconDraw, NineSliceDraw, NineSliceId, Vertex};
pub use image::{Image, ImageAlign, ImageFit};
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
/// Bundles commonly-passed drawing parameters to reduce function argument counts.
/// Use this when passing drawing resources through multiple UI layers.
pub struct DrawContext<'a> {
    pub draw_list: &'a mut DrawList,
    pub theme: &'a Theme,
    pub input: &'a InputState,
    pub screen_width: f32,
    pub screen_height: f32,
}

impl<'a> DrawContext<'a> {
    /// Create a new draw context.
    pub fn new(
        draw_list: &'a mut DrawList,
        theme: &'a Theme,
        input: &'a InputState,
        screen_width: f32,
        screen_height: f32,
    ) -> Self {
        Self {
            draw_list,
            theme,
            input,
            screen_width,
            screen_height,
        }
    }
}
