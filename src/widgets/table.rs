//! Table widget - displays tabular data with headers, scrolling, and row selection.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;

/// Column width specification.
#[derive(Debug, Clone, Copy)]
pub enum ColumnWidth {
    /// Fixed pixel width.
    Fixed(f32),
    /// Flexible width - takes proportion of remaining space.
    Flex(f32),
}

/// Text alignment within a cell.
#[derive(Debug, Clone, Copy, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Column definition for a table.
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub label: String,
    pub width: ColumnWidth,
    pub align: Align,
}

impl TableColumn {
    pub fn new(label: impl Into<String>, width: ColumnWidth) -> Self {
        Self {
            label: label.into(),
            width,
            align: Align::Left,
        }
    }

    pub fn with_align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }
}

/// A single cell in a table row.
#[derive(Debug, Clone, Default)]
pub struct TableCell {
    pub text: String,
    pub color: Option<[f32; 4]>,
}

impl TableCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: None,
        }
    }

    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = Some(color);
        self
    }
}

impl From<String> for TableCell {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for TableCell {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Scroll state for scrollable containers.
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    pub offset: f32,
    pub content_height: f32,
    pub visible_height: f32,
}

impl ScrollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update scroll offset with mouse wheel delta.
    /// Call this when the mouse is over the scrollable area.
    pub fn scroll(&mut self, delta: f32) {
        let max_scroll = (self.content_height - self.visible_height).max(0.0);
        self.offset = (self.offset - delta * 20.0).clamp(0.0, max_scroll);
    }

    /// Reset scroll to top.
    pub fn reset(&mut self) {
        self.offset = 0.0;
    }

    /// Check if content is scrollable.
    pub fn is_scrollable(&self) -> bool {
        self.content_height > self.visible_height
    }
}

/// Output from drawing a table.
pub struct TableOutput {
    /// The rect occupied by the entire table (including header).
    pub rect: Rect,
    /// Index of the row that was clicked, if any.
    pub clicked_row: Option<usize>,
    /// Index of the row currently hovered, if any.
    pub hovered_row: Option<usize>,
    /// True if mouse is over the table content area (for scroll handling).
    pub mouse_over_content: bool,
}

/// Table widget - displays tabular data with headers, scrolling, and row selection.
///
/// # Example
/// ```ignore
/// let columns = vec![
///     TableColumn::new("Name", ColumnWidth::Flex(1.0)),
///     TableColumn::new("Status", ColumnWidth::Fixed(80.0)),
/// ];
///
/// let rows: Vec<Vec<TableCell>> = data.iter().map(|item| vec![
///     TableCell::new(&item.name),
///     TableCell::new(&item.status).with_color(status_color),
/// ]).collect();
///
/// let output = Table::new(&columns)
///     .with_row_height(24.0)
///     .draw(rect, &rows, &mut scroll, draw_list, theme, input);
///
/// if let Some(idx) = output.clicked_row {
///     selected = Some(data[idx].id);
/// }
/// ```
pub struct Table<'a> {
    columns: &'a [TableColumn],
    row_height: f32,
    header_height: f32,
    show_header: bool,
    zebra_stripe: bool,
}

impl<'a> Table<'a> {
    pub fn new(columns: &'a [TableColumn]) -> Self {
        Self {
            columns,
            row_height: 24.0,
            header_height: 20.0,
            show_header: true,
            zebra_stripe: false,
        }
    }

    pub fn with_row_height(mut self, height: f32) -> Self {
        self.row_height = height;
        self
    }

    pub fn with_header_height(mut self, height: f32) -> Self {
        self.header_height = height;
        self
    }

    pub fn with_header(mut self, show: bool) -> Self {
        self.show_header = show;
        self
    }

    pub fn with_zebra_stripe(mut self, enable: bool) -> Self {
        self.zebra_stripe = enable;
        self
    }

    /// Draw the table and return interaction results.
    pub fn draw(
        &self,
        rect: Rect,
        rows: &[Vec<TableCell>],
        scroll: &mut ScrollState,
        list: &mut DrawList,
        theme: &Theme,
        input: &InputState,
    ) -> TableOutput {
        let header_h = if self.show_header {
            self.header_height
        } else {
            0.0
        };
        let content_rect = Rect::new(
            rect.x,
            rect.y + header_h,
            rect.width,
            rect.height - header_h,
        );

        // Update scroll state
        scroll.content_height = rows.len() as f32 * self.row_height;
        scroll.visible_height = content_rect.height;

        // Check mouse over content area
        let mouse_over_content = content_rect.contains(input.mouse_x, input.mouse_y);

        // Handle scroll input when mouse is over content
        if mouse_over_content && input.scroll_delta != 0.0 {
            scroll.scroll(input.scroll_delta);
        }

        // Calculate column widths
        let col_widths = self.calculate_column_widths(rect.width);

        // Draw header
        if self.show_header {
            self.draw_header(rect, &col_widths, list, theme);
        }

        // Draw rows
        let mut clicked_row = None;
        let mut hovered_row = None;
        let font_size = theme.font_size * 0.75;

        let first_visible = (scroll.offset / self.row_height).floor() as usize;
        let visible_count = (content_rect.height / self.row_height).ceil() as usize + 1;

        list.push_clip(content_rect);

        for row_idx in first_visible..(first_visible + visible_count).min(rows.len()) {
            if row_idx >= rows.len() {
                break;
            }

            let row = &rows[row_idx];
            let y = content_rect.y + (row_idx as f32 * self.row_height) - scroll.offset;

            // Skip if above visible area
            if y + self.row_height < content_rect.y {
                continue;
            }
            // Skip if below visible area
            if y > content_rect.y + content_rect.height {
                break;
            }

            let row_rect = Rect::new(rect.x, y, rect.width, self.row_height);

            // Check hover/click (only if within content bounds)
            let row_hovered = mouse_over_content
                && input.mouse_y >= y
                && input.mouse_y < y + self.row_height
                && input.mouse_y >= content_rect.y
                && input.mouse_y < content_rect.y + content_rect.height;

            if row_hovered {
                hovered_row = Some(row_idx);
                if input.mouse_clicked {
                    clicked_row = Some(row_idx);
                }
            }

            // Row background
            let bg_color = if row_hovered {
                theme.button_hover
            } else if self.zebra_stripe && row_idx % 2 == 1 {
                let mut c = theme.panel;
                c[0] *= 1.1;
                c[1] *= 1.1;
                c[2] *= 1.1;
                c
            } else {
                [0.0, 0.0, 0.0, 0.0] // transparent
            };

            if bg_color[3] > 0.0 {
                list.quad(
                    row_rect.x,
                    row_rect.y,
                    row_rect.width,
                    row_rect.height,
                    bg_color,
                );
            }

            // Draw cells
            let mut x = rect.x;
            for (col_idx, col_width) in col_widths.iter().enumerate() {
                if col_idx < row.len() {
                    let cell = &row[col_idx];
                    let cell_rect = Rect::new(x, y, *col_width, self.row_height);
                    self.draw_cell(
                        cell,
                        &self.columns[col_idx],
                        cell_rect,
                        font_size,
                        list,
                        theme,
                    );
                }
                x += col_width;
            }
        }

        list.pop_clip();

        // Draw scroll bar if needed
        if scroll.is_scrollable() {
            let scroll_bar_width = 4.0;
            let scroll_track_x = rect.x + rect.width - scroll_bar_width - 2.0;
            let scroll_bar_height =
                content_rect.height * (content_rect.height / scroll.content_height);
            let scroll_bar_y =
                content_rect.y + (scroll.offset / scroll.content_height) * content_rect.height;
            list.quad(
                scroll_track_x,
                scroll_bar_y,
                scroll_bar_width,
                scroll_bar_height,
                theme.text_dim,
            );
        }

        TableOutput {
            rect,
            clicked_row,
            hovered_row,
            mouse_over_content,
        }
    }

    fn calculate_column_widths(&self, total_width: f32) -> Vec<f32> {
        let mut widths = vec![0.0; self.columns.len()];
        let mut fixed_total = 0.0;
        let mut flex_total = 0.0;

        for (i, col) in self.columns.iter().enumerate() {
            match col.width {
                ColumnWidth::Fixed(w) => {
                    widths[i] = w;
                    fixed_total += w;
                }
                ColumnWidth::Flex(f) => {
                    flex_total += f;
                }
            }
        }

        let remaining = (total_width - fixed_total).max(0.0);

        for (i, col) in self.columns.iter().enumerate() {
            if let ColumnWidth::Flex(f) = col.width {
                widths[i] = if flex_total > 0.0 {
                    remaining * (f / flex_total)
                } else {
                    0.0
                };
            }
        }

        widths
    }

    fn draw_header(&self, rect: Rect, col_widths: &[f32], list: &mut DrawList, theme: &Theme) {
        let header_rect = Rect::new(rect.x, rect.y, rect.width, self.header_height);

        // Header background
        list.quad(
            header_rect.x,
            header_rect.y,
            header_rect.width,
            header_rect.height,
            theme.tab_inactive,
        );

        // Header bottom border
        list.quad(
            rect.x,
            rect.y + self.header_height - 1.0,
            rect.width,
            1.0,
            theme.panel_border,
        );

        // Header labels
        let font_size = theme.font_size * 0.7;
        let mut x = rect.x;

        for (i, col) in self.columns.iter().enumerate() {
            let col_width = col_widths[i];
            let padding = 4.0;

            let text_x = match col.align {
                Align::Left => x + padding,
                Align::Center => {
                    let (text_width, _) = list.measure_text(&col.label, font_size);
                    x + (col_width - text_width) / 2.0
                }
                Align::Right => {
                    let (text_width, _) = list.measure_text(&col.label, font_size);
                    x + col_width - text_width - padding
                }
            };

            let text = TextBlock::new(
                &col.label,
                text_x,
                rect.y + (self.header_height - font_size) / 2.0,
            )
            .with_size(font_size)
            .with_color(
                (theme.text_dim[0] * 255.0) as u8,
                (theme.text_dim[1] * 255.0) as u8,
                (theme.text_dim[2] * 255.0) as u8,
            );
            list.text(text);

            x += col_width;
        }
    }

    fn draw_cell(
        &self,
        cell: &TableCell,
        column: &TableColumn,
        rect: Rect,
        font_size: f32,
        list: &mut DrawList,
        theme: &Theme,
    ) {
        let padding = 4.0;
        let text_y = rect.y + (rect.height - font_size) / 2.0;

        let text_x = match column.align {
            Align::Left => rect.x + padding,
            Align::Center => {
                let (text_width, _) = list.measure_text(&cell.text, font_size);
                rect.x + (rect.width - text_width) / 2.0
            }
            Align::Right => {
                let (text_width, _) = list.measure_text(&cell.text, font_size);
                rect.x + rect.width - text_width - padding
            }
        };

        let color = cell.color.unwrap_or(theme.text);
        let text = TextBlock::new(&cell.text, text_x, text_y)
            .with_size(font_size)
            .with_color(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            );
        list.text(text);
    }
}
