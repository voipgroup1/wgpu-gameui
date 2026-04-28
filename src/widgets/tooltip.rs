//! Tooltip system - hover regions with rich content display.
//!
//! Tooltips live on the popup/tooltip layer of `LayerStack` so they always
//! draw above the rest of the UI without callers having to remember to
//! "draw last". They support a configurable hover delay (`with_delay_ms`) —
//! the tooltip only appears once the cursor has rested over the same hover
//! region for that many milliseconds.

use crate::layer::LayerStack;
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
    Rich {
        title: String,
        description: String,
        details: Vec<(String, String)>,
    },
}

impl TooltipContent {
    pub fn text(body: impl Into<String>) -> Self {
        Self::Text {
            title: None,
            body: body.into(),
        }
    }

    pub fn text_with_title(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Text {
            title: Some(title.into()),
            body: body.into(),
        }
    }

    pub fn lines(lines: Vec<String>) -> Self {
        Self::Lines { title: None, lines }
    }

    pub fn lines_with_title(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self::Lines {
            title: Some(title.into()),
            lines,
        }
    }

    pub fn rich(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::Rich {
            title: title.into(),
            description: description.into(),
            details: Vec::new(),
        }
    }

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
/// Hover-delay state persists across frames; clear regions every frame with
/// [`TooltipLayer::clear`] (or use the same instance over the lifetime of
/// your UI and re-register hover regions each frame).
///
/// ```ignore
/// // Persist across frames so hover delay accumulates.
/// let mut tooltips = TooltipLayer::new().with_delay_ms(400);
///
/// // Each frame:
/// tooltips.clear();
/// tooltips.register(stat_rect, TooltipContent::rich("Strength", "Physical power..."));
/// tooltips.tick(dt_seconds, &input);
///
/// // Either route into a LayerStack popup layer, or draw directly onto a
/// // DrawList that's the last thing drawn this frame.
/// tooltips.draw_into_layers(&mut layers, &input, &theme, screen_w, screen_h);
/// ```
pub struct TooltipLayer {
    regions: Vec<HoverRegion>,
    hover_delay_ms: u32,
    /// Index of the region currently hovered (for delay accumulation).
    hovered_idx: Option<usize>,
    /// Seconds the cursor has been over `hovered_idx`.
    hover_seconds: f32,
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
            hovered_idx: None,
            hover_seconds: 0.0,
        }
    }

    /// Set the hover delay before showing tooltips (in milliseconds).
    pub fn with_delay_ms(mut self, delay_ms: u32) -> Self {
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

    /// Advance the hover-delay timer based on the input cursor position
    /// and frame `dt` in seconds. Must be called once per frame *after*
    /// registering regions for that frame.
    pub fn tick(&mut self, dt_seconds: f32, input: &InputState) {
        let new_idx = self.regions.iter().position(|r| {
            !input.mouse_consumed && r.rect.contains(input.mouse_x, input.mouse_y)
        });

        if new_idx != self.hovered_idx {
            // Hover target changed — reset, then count this frame as the
            // first frame of hover on the new target.
            self.hovered_idx = new_idx;
            self.hover_seconds = if new_idx.is_some() { dt_seconds } else { 0.0 };
        } else if new_idx.is_some() {
            self.hover_seconds += dt_seconds;
        }
    }

    /// Whether the hovered tooltip is currently visible (delay satisfied).
    pub fn is_visible(&self) -> bool {
        self.hovered_idx.is_some()
            && self.hover_seconds * 1000.0 >= self.hover_delay_ms as f32
    }

    /// Render the active tooltip onto a fresh tooltip layer of `layers`.
    /// Does nothing if no tooltip is active or the delay has not elapsed.
    pub fn draw_into_layers(
        &self,
        layers: &mut LayerStack,
        input: &InputState,
        theme: &Theme,
        screen_width: f32,
        screen_height: f32,
    ) {
        if !self.is_visible() {
            return;
        }
        let region = match self.hovered_idx.and_then(|i| self.regions.get(i)) {
            Some(r) => r,
            None => return,
        };
        // Bound the layer rect by the on-screen tooltip area; we don't know
        // the exact size yet, so use a generous rect — tooltips never block
        // input, so this rect is purely informational.
        let bounds = Rect::new(0.0, 0.0, screen_width, screen_height);
        layers.push_tooltip(bounds);
        let list = layers.current_mut();
        draw_tooltip_body(list, theme, input, region, screen_width, screen_height);
        layers.pop_layer();
    }

    /// Direct-to-draw-list rendering for callers that aren't using
    /// `LayerStack` yet (legacy "call this at the end of your UI" path).
    /// Prefer `draw_into_layers` in new code.
    pub fn draw(
        &self,
        input: &InputState,
        list: &mut DrawList,
        theme: &Theme,
        screen_width: f32,
        screen_height: f32,
    ) {
        if !self.is_visible() {
            return;
        }
        let region = match self.hovered_idx.and_then(|i| self.regions.get(i)) {
            Some(r) => r,
            None => return,
        };
        draw_tooltip_body(list, theme, input, region, screen_width, screen_height);
    }
}

fn draw_tooltip_body(
    list: &mut DrawList,
    theme: &Theme,
    input: &InputState,
    region: &HoverRegion,
    screen_width: f32,
    screen_height: f32,
) {
    let padding = 8.0;
    let title_size = theme.font_size * 0.85;
    let body_size = theme.font_size * 0.75;
    let line_height = body_size + 3.0;
    let title_height = title_size + 4.0;

    let mut rich_desc_h: f32 = 0.0;

    let (width, height) = match &region.content {
        TooltipContent::Text { title, body } => {
            let title_h = if title.is_some() { title_height } else { 0.0 };
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
            rich_desc_h = desc_h;
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

    let bg_color = [0.10, 0.10, 0.15, 0.95];
    let border_color = theme.panel_border;
    list.quad(x, y, width, height, bg_color);

    let border = 1.0;
    list.quad(x, y, width, border, border_color);
    list.quad(x, y + height - border, width, border, border_color);
    list.quad(x, y, border, height, border_color);
    list.quad(x + width - border, y, border, height, border_color);

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

            cursor_y += rich_desc_h;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    #[test]
    fn no_delay_shows_immediately() {
        let mut tt = TooltipLayer::new();
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.0, &input_at(10.0, 10.0));
        assert!(tt.is_visible());
    }

    #[test]
    fn delay_blocks_immediate_display() {
        let mut tt = TooltipLayer::new().with_delay_ms(400);
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.05, &input_at(10.0, 10.0));
        assert!(!tt.is_visible(), "tooltip should still be hidden at 50ms");
    }

    #[test]
    fn delay_satisfied_after_enough_hover_time() {
        let mut tt = TooltipLayer::new().with_delay_ms(200);
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.1, &input_at(10.0, 10.0));
        assert!(!tt.is_visible());
        tt.tick(0.15, &input_at(10.0, 10.0));
        assert!(tt.is_visible(), "tooltip should be visible after 250ms");
    }

    #[test]
    fn moving_off_resets_hover_timer() {
        let mut tt = TooltipLayer::new().with_delay_ms(200);
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.5, &input_at(10.0, 10.0));
        assert!(tt.is_visible());

        // Move off — clear regions and re-register? In a real frame the
        // caller calls clear/register every frame. We simulate that.
        tt.clear();
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.05, &input_at(500.0, 500.0)); // outside
        assert!(!tt.is_visible(), "moving off should hide tooltip");
        tt.tick(0.05, &input_at(10.0, 10.0)); // re-enter
        assert!(!tt.is_visible(), "re-enter should restart delay");
    }

    #[test]
    fn consumed_input_blocks_tooltip() {
        let mut tt = TooltipLayer::new();
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        let mut inp = input_at(10.0, 10.0);
        inp.mouse_consumed = true;
        tt.tick(0.0, &inp);
        assert!(!tt.is_visible());
    }
}
