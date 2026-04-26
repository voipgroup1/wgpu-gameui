//! Tooltip system - hover regions with rich content display.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{InputState, Theme};

use super::DrawList;

/// Content that can be displayed in a tooltip.
/// Designed to be extensible for future content types (images, icons, etc.)
#[derive(Clone)]
pub enum TooltipContent {
    /// Simple text tooltip with optional title.
    Text { title: Option<String>, body: String },
    /// Multi-line text with optional title.
    Lines {
        title: Option<String>,
        lines: Vec<String>,
    },
    /// Rich content with title, description, and key-value pairs.
    /// Useful for stat tooltips like "Strength: 85 / Affects carry capacity..."
    Rich {
        title: String,
        description: String,
        details: Vec<(String, String)>,
    },
}

impl TooltipContent {
    /// Create a simple text tooltip.
    pub fn text(body: impl Into<String>) -> Self {
        Self::Text {
            title: None,
            body: body.into(),
        }
    }

    /// Create a text tooltip with a title.
    pub fn text_with_title(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Text {
            title: Some(title.into()),
            body: body.into(),
        }
    }

    /// Create a multi-line tooltip.
    pub fn lines(lines: Vec<String>) -> Self {
        Self::Lines { title: None, lines }
    }

    /// Create a multi-line tooltip with a title.
    pub fn lines_with_title(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self::Lines {
            title: Some(title.into()),
            lines,
        }
    }

    /// Create a rich tooltip with title, description, and optional details.
    pub fn rich(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::Rich {
            title: title.into(),
            description: description.into(),
            details: Vec::new(),
        }
    }

    /// Add a detail line to a rich tooltip.
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if let Self::Rich {
            ref mut details, ..
        } = self
        {
            details.push((key.into(), value.into()));
        }
        self
    }
}

/// A registered hover region with its tooltip content.
struct HoverRegion {
    rect: Rect,
    content: TooltipContent,
}

/// Manages tooltip display for a frame.
///
/// # Usage
/// ```ignore
/// let mut tooltips = TooltipLayer::new();
///
/// // During UI building, register hover regions:
/// tooltips.register(stat_rect, TooltipContent::rich("Strength", "Physical power...")
///     .with_detail("Current", "85"));
///
/// // At the end of the frame, draw any active tooltip:
/// tooltips.draw(&input, &mut draw_list, &theme, screen_width, screen_height);
/// ```
pub struct TooltipLayer {
    regions: Vec<HoverRegion>,
    hover_delay_ms: u32,
}

impl Default for TooltipLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl TooltipLayer {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            hover_delay_ms: 0,
        }
    }

    /// Set the hover delay before showing tooltips (in milliseconds).
    pub fn with_delay(mut self, delay_ms: u32) -> Self {
        self.hover_delay_ms = delay_ms;
        self
    }

    /// Clear all registered regions (call at start of frame).
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Register a hover region with tooltip content.
    pub fn register(&mut self, rect: Rect, content: TooltipContent) {
        self.regions.push(HoverRegion { rect, content });
    }

    /// Draw the tooltip for the currently hovered region (if any).
    /// Call this at the end of your UI drawing, so tooltips appear on top.
    pub fn draw(
        &self,
        input: &InputState,
        list: &mut DrawList,
        theme: &Theme,
        screen_width: f32,
        screen_height: f32,
    ) {
        // Find the first hovered region (could prioritize by z-order if needed)
        let hovered = self
            .regions
            .iter()
            .find(|r| r.rect.contains(input.mouse_x, input.mouse_y));

        let region = match hovered {
            Some(r) => r,
            None => return,
        };

        // Calculate tooltip size based on content
        let padding = 8.0;
        let title_size = theme.font_size * 0.85;
        let body_size = theme.font_size * 0.75;
        let line_height = body_size + 3.0;
        let title_height = title_size + 4.0;

        let (width, height) = match &region.content {
            TooltipContent::Text { title, body } => {
                let title_h = if title.is_some() { title_height } else { 0.0 };
                // Determine width first using an unconstrained measurement, then clamp
                // and re-measure with that as the wrap width to get the real height.
                let (body_width_natural, _) = list.measure_text(body, body_size, None);
                let w = 220.0f32.max(body_width_natural).min(300.0);
                let inner_w = w - padding * 2.0;
                let (_, body_h) = list.measure_text(body, body_size, Some(inner_w));
                let h = padding * 2.0 + title_h + body_h;
                (w, h)
            }
            TooltipContent::Lines { title, lines } => {
                let title_h = if title.is_some() { title_height } else { 0.0 };
                let max_line_width = lines
                    .iter()
                    .map(|line| list.measure_text(line, body_size, None).0)
                    .fold(0.0f32, f32::max);
                let w = 180.0f32.max(max_line_width).min(300.0);
                let h = padding * 2.0 + title_h + lines.len() as f32 * line_height;
                (w, h)
            }
            TooltipContent::Rich {
                title: _,
                description,
                details,
            } => {
                let w = 240.0;
                let inner_w = w - padding * 2.0;
                let (_, desc_h) = list.measure_text(description, body_size, Some(inner_w));
                let h = padding * 2.0
                    + title_height
                    + desc_h
                    + if !details.is_empty() {
                        8.0 + details.len() as f32 * line_height
                    } else {
                        0.0
                    };
                (w, h)
            }
        };

        // Position tooltip near mouse, but keep on screen
        let margin = 12.0;
        let mut x = input.mouse_x + margin;
        let mut y = input.mouse_y + margin;

        if x + width > screen_width - margin {
            x = input.mouse_x - width - margin;
        }
        if y + height > screen_height - margin {
            y = input.mouse_y - height - margin;
        }
        x = x.max(margin);
        y = y.max(margin);

        // Draw tooltip background
        let bg_color = [0.10, 0.10, 0.15, 0.95];
        let border_color = theme.panel_border;
        list.quad(x, y, width, height, bg_color);

        // Border
        let border = 1.0;
        list.quad(x, y, width, border, border_color);
        list.quad(x, y + height - border, width, border, border_color);
        list.quad(x, y, border, height, border_color);
        list.quad(x + width - border, y, border, height, border_color);

        // Draw content
        let content_x = x + padding;
        let mut cursor_y = y + padding;

        match &region.content {
            TooltipContent::Text { title, body } => {
                if let Some(t) = title {
                    let title_block = TextBlock::new(t, content_x, cursor_y)
                        .with_size(title_size)
                        .with_color(
                            (theme.text_highlight[0] * 255.0) as u8,
                            (theme.text_highlight[1] * 255.0) as u8,
                            (theme.text_highlight[2] * 255.0) as u8,
                        );
                    list.text(title_block);
                    cursor_y += title_height;
                }

                let body_block = TextBlock::new(body, content_x, cursor_y)
                    .with_size(body_size)
                    .with_color(
                        (theme.text[0] * 255.0) as u8,
                        (theme.text[1] * 255.0) as u8,
                        (theme.text[2] * 255.0) as u8,
                    )
                    .with_max_width(width - padding * 2.0);
                list.text(body_block);
            }
            TooltipContent::Lines { title, lines } => {
                if let Some(t) = title {
                    let title_block = TextBlock::new(t, content_x, cursor_y)
                        .with_size(title_size)
                        .with_color(
                            (theme.text_highlight[0] * 255.0) as u8,
                            (theme.text_highlight[1] * 255.0) as u8,
                            (theme.text_highlight[2] * 255.0) as u8,
                        );
                    list.text(title_block);
                    cursor_y += title_height;
                }

                for line in lines {
                    let line_block = TextBlock::new(line, content_x, cursor_y)
                        .with_size(body_size)
                        .with_color(
                            (theme.text[0] * 255.0) as u8,
                            (theme.text[1] * 255.0) as u8,
                            (theme.text[2] * 255.0) as u8,
                        );
                    list.text(line_block);
                    cursor_y += line_height;
                }
            }
            TooltipContent::Rich {
                title,
                description,
                details,
            } => {
                let title_block = TextBlock::new(title, content_x, cursor_y)
                    .with_size(title_size)
                    .with_color(
                        (theme.text_highlight[0] * 255.0) as u8,
                        (theme.text_highlight[1] * 255.0) as u8,
                        (theme.text_highlight[2] * 255.0) as u8,
                    );
                list.text(title_block);
                cursor_y += title_height;

                let desc_block = TextBlock::new(description, content_x, cursor_y)
                    .with_size(body_size)
                    .with_color(
                        (theme.text[0] * 255.0) as u8,
                        (theme.text[1] * 255.0) as u8,
                        (theme.text[2] * 255.0) as u8,
                    )
                    .with_max_width(width - padding * 2.0);
                list.text(desc_block);

                let (_, desc_h) =
                    list.measure_text(description, body_size, Some(width - padding * 2.0));
                cursor_y += desc_h;

                if !details.is_empty() {
                    cursor_y += 8.0;

                    for (key, value) in details {
                        let detail_text = format!("{}: {}", key, value);
                        let detail_block = TextBlock::new(&detail_text, content_x, cursor_y)
                            .with_size(body_size)
                            .with_color(
                                (theme.text_dim[0] * 255.0) as u8,
                                (theme.text_dim[1] * 255.0) as u8,
                                (theme.text_dim[2] * 255.0) as u8,
                            );
                        list.text(detail_block);
                        cursor_y += line_height;
                    }
                }
            }
        }
    }
}
