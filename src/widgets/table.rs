//! Table widget - displays tabular data with headers, scrolling, and row selection.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;
use super::scroll_view::{ScrollState, ScrollView};

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
///     .draw(rect, &rows, &mut scroll, draw_list, theme, &mut input);
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
        input: &mut InputState,
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

        // Update scroll state's content extent — ScrollView handles clamping +
        // wheel + scrollbar interaction from here.
        scroll.content_size = [rect.width, rows.len() as f32 * self.row_height];

        // Calculate column widths
        let col_widths = self.calculate_column_widths(rect.width);

        // Draw header
        if self.show_header {
            self.draw_header(rect, &col_widths, list, theme);
        }

        // Track interaction results from inside the closure.
        let mut clicked_row = None;
        let mut hovered_row = None;
        let font_size = theme.font_size * 0.75;

        let mouse_over_content =
            content_rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;

        // Snapshot offset for use inside the closure — scroll is borrowed
        // mutably by ScrollView::draw, so we read once up front. Same for
        // mouse position: ScrollView gets `&mut input` and may zero scroll
        // fields, but the fields we use here (mouse position + click state)
        // are stable for the lifetime of the call.
        let scroll_y = scroll.offset[1];
        let mouse_y = input.mouse_y;
        let mouse_clicked = input.mouse_clicked;

        ScrollView::new(content_rect).vertical_only().draw(
            scroll,
            list,
            theme,
            input,
            |list, vp| {
                // The ScrollView has translated by -scroll.offset and clipped
                // to `vp`. Draw rows in vp-local world coordinates: row N lives
                // at y = vp.y + N * row_height (pre-translation), which the
                // ScrollView shifts by -offset for us.
                let first_visible = (scroll_y / self.row_height).floor() as usize;
                let visible_count = (vp.height / self.row_height).ceil() as usize + 1;

                let end = (first_visible + visible_count).min(rows.len());
                for (row_idx, row) in rows
                    .iter()
                    .enumerate()
                    .skip(first_visible)
                    .take(end.saturating_sub(first_visible))
                {
                    // `world_y` here is in CONTENT space (pre-scroll-translate)
                    // — this is what we draw at, because the ScrollView has
                    // already pushed a `translate(0, -scroll_y)` for us.
                    let world_y = vp.y + row_idx as f32 * self.row_height;
                    // `screen_y` is where this row actually lands on screen.
                    // Mouse coords are in screen space, so we hit-test against
                    // this, not against `world_y`.
                    let screen_y = world_y - scroll_y;

                    let row_hovered = mouse_over_content
                        && mouse_y >= screen_y
                        && mouse_y < screen_y + self.row_height
                        && mouse_y >= vp.y
                        && mouse_y < vp.y + vp.height;

                    if row_hovered {
                        hovered_row = Some(row_idx);
                        if mouse_clicked {
                            clicked_row = Some(row_idx);
                        }
                    }

                    let bg_color = if row_hovered {
                        theme.button_hover
                    } else if self.zebra_stripe && row_idx % 2 == 1 {
                        let mut c = theme.panel;
                        c[0] *= 1.1;
                        c[1] *= 1.1;
                        c[2] *= 1.1;
                        c
                    } else {
                        [0.0, 0.0, 0.0, 0.0]
                    };

                    if bg_color[3] > 0.0 {
                        list.quad(rect.x, world_y, rect.width, self.row_height, bg_color);
                    }

                    let mut x = rect.x;
                    for (col_idx, col_width) in col_widths.iter().enumerate() {
                        if col_idx < row.len() {
                            let cell = &row[col_idx];
                            let cell_rect = Rect::new(x, world_y, *col_width, self.row_height);
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
            },
        );

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
                    let (text_width, _) = list.measure_text(&col.label, font_size, None);
                    x + (col_width - text_width) / 2.0
                }
                Align::Right => {
                    let (text_width, _) = list.measure_text(&col.label, font_size, None);
                    x + col_width - text_width - padding
                }
            };

            let text_y = list.vcentered_text_y(
                rect.y,
                self.header_height,
                font_size,
                theme.font.as_ref(),
                &col.label,
            );
            let text = TextBlock::new(&col.label, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (theme.text_dim[0] * 255.0) as u8,
                    (theme.text_dim[1] * 255.0) as u8,
                    (theme.text_dim[2] * 255.0) as u8,
                )
                .with_font_opt(theme.font.clone());
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
        let text_y = list.vcentered_text_y(
            rect.y,
            rect.height,
            font_size,
            theme.font.as_ref(),
            &cell.text,
        );

        let text_x = match column.align {
            Align::Left => rect.x + padding,
            Align::Center => {
                let (text_width, _) = list.measure_text(&cell.text, font_size, None);
                rect.x + (rect.width - text_width) / 2.0
            }
            Align::Right => {
                let (text_width, _) = list.measure_text(&cell.text, font_size, None);
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
            )
            .with_font_opt(theme.font.clone());
        list.text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<TableColumn> {
        vec![
            TableColumn::new("A", ColumnWidth::Flex(1.0)),
            TableColumn::new("B", ColumnWidth::Fixed(40.0)),
        ]
    }

    fn rows(n: usize) -> Vec<Vec<TableCell>> {
        (0..n)
            .map(|i| {
                vec![
                    TableCell::new(format!("a{}", i)),
                    TableCell::new(format!("b{}", i)),
                ]
            })
            .collect()
    }

    #[test]
    fn click_targets_correct_row_after_scroll() {
        // Build a table with 50 rows of height 24 (1200px content) into a
        // 200x100 viewport (header off). After scrolling down 100px, row 0 is
        // off-screen and the first visible row should be row ~4 at screen y=
        // viewport.y + 4*24 - 100 = -4. The mouse hovers at the screen-y of
        // row 5: viewport.y + 5*24 - 100 = 20.
        let columns = cols();
        let row_data = rows(50);
        let table = Table::new(&columns)
            .with_row_height(24.0)
            .with_header(false);

        let mut scroll = ScrollState::default();
        scroll.content_size = [200.0, 50.0 * 24.0];
        scroll.offset = [0.0, 100.0];

        let viewport = Rect::new(0.0, 0.0, 200.0, 100.0);
        let row_idx = 5usize;
        // Screen y for the top of row_idx after scrolling.
        let screen_y = viewport.y + row_idx as f32 * 24.0 - scroll.offset[1];
        // Mouse in middle of that row (in screen space).
        let mouse_y = screen_y + 12.0;

        let mut input = InputState {
            mouse_x: viewport.x + 50.0,
            mouse_y,
            mouse_clicked: true,
            mouse_down: true,
            ..InputState::default()
        };

        let mut list = DrawList::new();
        let theme = Theme::default();
        let out = table.draw(
            viewport,
            &row_data,
            &mut scroll,
            &mut list,
            &theme,
            &mut input,
        );
        assert_eq!(
            out.clicked_row,
            Some(row_idx),
            "expected row {} (screen_y={}, mouse_y={}) but got {:?}",
            row_idx,
            screen_y,
            mouse_y,
            out.clicked_row
        );
        assert_eq!(out.hovered_row, Some(row_idx));
    }

    #[test]
    fn click_above_viewport_does_not_pick_a_row() {
        // Mouse above the table — must not register as a hover even though
        // the math could otherwise land on a content-space row position.
        let columns = cols();
        let row_data = rows(50);
        let table = Table::new(&columns)
            .with_row_height(24.0)
            .with_header(false);

        let mut scroll = ScrollState::default();
        scroll.content_size = [200.0, 50.0 * 24.0];
        scroll.offset = [0.0, 100.0];
        let viewport = Rect::new(0.0, 200.0, 200.0, 100.0);
        let mut input = InputState {
            mouse_x: 50.0,
            mouse_y: 50.0, // above viewport.y == 200
            mouse_clicked: true,
            ..InputState::default()
        };
        let mut list = DrawList::new();
        let theme = Theme::default();
        let out = table.draw(
            viewport,
            &row_data,
            &mut scroll,
            &mut list,
            &theme,
            &mut input,
        );
        assert_eq!(out.clicked_row, None);
        assert_eq!(out.hovered_row, None);
    }
}
