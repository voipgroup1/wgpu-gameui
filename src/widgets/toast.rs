//! Toast notifications — transient, auto-dismissing messages stacked in a
//! screen corner.
//!
//! [`ToastStack`] is caller-owned and persists across frames, mirroring
//! [`TooltipLayer`](super::TooltipLayer)'s lifecycle: [`push`](ToastStack::push)
//! a [`Toast`] when an event happens, [`tick`](ToastStack::tick) once per frame
//! with the frame `dt` (which ages toasts and drops expired ones), and
//! [`draw`](ToastStack::draw) last so the stack sits above the rest of the UI.
//! Each toast renders as a [`Banner`], stacked from the chosen [`Corner`] and
//! fading out in its final moments.
//!
//! ```ignore
//! // Persist across frames.
//! let mut toasts = ToastStack::new().with_corner(Corner::TopRight);
//!
//! // On some event:
//! toasts.push(Toast::success("Settings saved").with_title("Saved"));
//!
//! // Each frame:
//! toasts.tick(dt_seconds);
//! toasts.draw(screen_w, screen_h, &mut list, &style); // draw last
//! ```

use crate::layout::Rect;
use crate::StyleResolver;

use super::{Banner, DrawList, Severity};

/// Screen corner a [`ToastStack`] anchors to. The newest toast sits nearest the
/// corner; older toasts stack away from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    /// Top-right (default): newest at the top, stack downward.
    TopRight,
    /// Top-left: newest at the top, stack downward.
    TopLeft,
    /// Bottom-right: newest at the bottom, stack upward.
    BottomRight,
    /// Bottom-left: newest at the bottom, stack upward.
    BottomLeft,
}

impl Corner {
    fn is_right(self) -> bool {
        matches!(self, Corner::TopRight | Corner::BottomRight)
    }
    fn is_top(self) -> bool {
        matches!(self, Corner::TopRight | Corner::TopLeft)
    }
}

/// A single transient notification to enqueue into a [`ToastStack`].
#[derive(Debug, Clone)]
pub struct Toast {
    severity: Severity,
    title: Option<String>,
    message: String,
    /// Seconds the toast stays before auto-dismissing.
    ttl: f32,
}

/// Default seconds a toast is shown before it auto-dismisses.
pub const DEFAULT_TTL: f32 = 4.0;

impl Toast {
    /// A toast with an explicit severity and the default time-to-live.
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            title: None,
            message: message.into(),
            ttl: DEFAULT_TTL,
        }
    }

    /// Info toast.
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(Severity::Info, message)
    }
    /// Success toast.
    pub fn success(message: impl Into<String>) -> Self {
        Self::new(Severity::Success, message)
    }
    /// Warning toast.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }
    /// Error toast.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Add a bold title line.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Override the time-to-live (seconds).
    pub fn with_ttl(mut self, ttl: f32) -> Self {
        self.ttl = ttl.max(0.0);
        self
    }
}

/// A toast plus its elapsed lifetime.
struct Active {
    toast: Toast,
    elapsed: f32,
}

/// Alpha for a toast given its elapsed time, ttl and fade duration: full opacity
/// until the last `fade` seconds, then a linear ramp to 0 at expiry.
fn fade_alpha(elapsed: f32, ttl: f32, fade: f32) -> f32 {
    if fade <= 0.0 {
        return 1.0;
    }
    let remaining = ttl - elapsed;
    (remaining / fade).clamp(0.0, 1.0)
}

/// A caller-owned stack of transient toast notifications.
pub struct ToastStack {
    active: Vec<Active>,
    corner: Corner,
    width: f32,
    gap: f32,
    margin: f32,
    fade: f32,
    max: usize,
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastStack {
    /// A new, empty stack (top-right corner, sensible defaults).
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            corner: Corner::TopRight,
            width: 300.0,
            gap: 8.0,
            margin: 16.0,
            fade: 0.4,
            max: 4,
        }
    }

    /// Anchor corner (default [`Corner::TopRight`]).
    pub fn with_corner(mut self, corner: Corner) -> Self {
        self.corner = corner;
        self
    }
    /// Toast width in px (default 300).
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width.max(1.0);
        self
    }
    /// Max simultaneously *visible* toasts (default 4). Extra (older) toasts stay
    /// queued and appear as visible ones expire.
    pub fn with_max(mut self, max: usize) -> Self {
        self.max = max;
        self
    }
    /// Fade-out duration in seconds before expiry (default 0.4; 0 disables).
    pub fn with_fade(mut self, fade: f32) -> Self {
        self.fade = fade.max(0.0);
        self
    }
    /// Gap between stacked toasts in px (default 8).
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }
    /// Margin from the screen edges in px (default 16).
    pub fn with_margin(mut self, margin: f32) -> Self {
        self.margin = margin.max(0.0);
        self
    }

    /// Enqueue a toast.
    pub fn push(&mut self, toast: Toast) {
        self.active.push(Active { toast, elapsed: 0.0 });
    }

    /// Age all toasts by `dt` seconds and drop any that have outlived their ttl.
    pub fn tick(&mut self, dt: f32) {
        for a in &mut self.active {
            a.elapsed += dt;
        }
        self.active.retain(|a| a.elapsed < a.toast.ttl);
    }

    /// Whether there are no active toasts.
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Number of active (queued + visible) toasts.
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Draw the visible toasts. Call last so they sit above the rest of the UI.
    pub fn draw(
        &self,
        screen_width: f32,
        screen_height: f32,
        list: &mut DrawList,
        style: &StyleResolver,
    ) {
        if self.active.is_empty() || self.max == 0 {
            return;
        }

        let x = if self.corner.is_right() {
            screen_width - self.margin - self.width
        } else {
            self.margin
        };

        // Show the newest `max`, newest nearest the corner.
        let start = self.active.len().saturating_sub(self.max);
        let visible = &self.active[start..];

        // Running edge: top corners advance downward from the top margin; bottom
        // corners advance upward from the bottom margin.
        let mut top_edge = self.margin;
        let mut bottom_edge = screen_height - self.margin;

        for a in visible.iter().rev() {
            let banner = self.banner_for(&a.toast);
            let h = banner.measure_height(list, style, self.width);
            let y = if self.corner.is_top() {
                let y = top_edge;
                top_edge += h + self.gap;
                y
            } else {
                bottom_edge -= h;
                let y = bottom_edge;
                bottom_edge -= self.gap;
                y
            };

            let alpha = fade_alpha(a.elapsed, a.toast.ttl, self.fade);
            list.push_tint();
            list.multiply_tint([1.0, 1.0, 1.0, alpha]);
            banner.draw(Rect::new(x, y, self.width, h), list, style);
            list.pop_tint();
        }
    }

    /// Build a borrowed [`Banner`] view of a toast.
    fn banner_for<'t>(&self, toast: &'t Toast) -> Banner<'t> {
        let mut b = Banner::new(toast.severity, &toast.message);
        if let Some(title) = &toast.title {
            b = b.with_title(title);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, Theme};

    fn style() -> StyleResolver<'static> {
        let theme: &'static Theme = Box::leak(Box::new(Theme::default()));
        StyleResolver::new(theme)
    }

    #[test]
    fn tick_past_ttl_drops_the_toast() {
        let mut s = ToastStack::new();
        s.push(Toast::info("hi").with_ttl(1.0));
        assert_eq!(s.len(), 1);
        s.tick(0.5);
        assert_eq!(s.len(), 1, "still alive at 0.5s");
        s.tick(0.6);
        assert!(s.is_empty(), "expired past 1.0s");
    }

    #[test]
    fn oldest_expires_first() {
        let mut s = ToastStack::new();
        s.push(Toast::info("old").with_ttl(1.0));
        s.push(Toast::info("new").with_ttl(5.0));
        s.tick(2.0);
        assert_eq!(s.len(), 1, "only the short-lived toast expired");
    }

    #[test]
    fn fade_alpha_ramps_in_final_window() {
        // No fade in the steady state.
        assert_eq!(fade_alpha(0.0, 4.0, 0.4), 1.0);
        assert_eq!(fade_alpha(3.5, 4.0, 0.4), 1.0, "before the fade window");
        // Halfway through the 0.4s fade window (remaining 0.2) → 0.5.
        assert!((fade_alpha(3.8, 4.0, 0.4) - 0.5).abs() < 1e-4);
        // At expiry → 0.
        assert_eq!(fade_alpha(4.0, 4.0, 0.4), 0.0);
        // Disabled fade is always opaque.
        assert_eq!(fade_alpha(3.99, 4.0, 0.0), 1.0);
    }

    #[test]
    fn draw_caps_visible_at_max() {
        let s_style = style();
        let mut list = DrawList::new();
        let mut s = ToastStack::new().with_max(2);
        for i in 0..3 {
            s.push(Toast::info(format!("toast {i}")));
        }
        s.draw(800.0, 600.0, &mut list, &s_style);
        // Each banner emits exactly 2 chrome instances (bg + accent bar); with a
        // cap of 2, only 2 banners (4 chrome instances) are drawn despite 3 queued.
        assert_eq!(s.len(), 3, "all three stay queued");
        assert_eq!(list.chrome_instances.len(), 4, "only 2 banners rendered");
    }

    #[test]
    fn right_and_left_corners_place_at_opposite_edges() {
        let s_style = style();
        let mut s = ToastStack::new().with_width(300.0).with_margin(16.0);
        s.push(Toast::info("x"));

        let mut right = DrawList::new();
        s.draw(800.0, 600.0, &mut right, &s_style);
        // First chrome instance is the toast bg; its left edge.
        let right_x = right.chrome_instances[0].rect[0];
        assert!((right_x - (800.0 - 16.0 - 300.0)).abs() < 1e-3, "right-anchored");

        let s = ToastStack::new()
            .with_width(300.0)
            .with_margin(16.0)
            .with_corner(Corner::TopLeft);
        let mut s = s;
        s.push(Toast::info("x"));
        let mut left = DrawList::new();
        s.draw(800.0, 600.0, &mut left, &s_style);
        let left_x = left.chrome_instances[0].rect[0];
        assert!((left_x - 16.0).abs() < 1e-3, "left-anchored");
    }

    #[test]
    fn top_stacks_down_bottom_stacks_up() {
        let s_style = style();

        // Top corner: first (newest) toast near the top margin.
        let mut top = ToastStack::new().with_corner(Corner::TopRight).with_margin(16.0);
        top.push(Toast::info("a"));
        let mut tlist = DrawList::new();
        top.draw(800.0, 600.0, &mut tlist, &s_style);
        let top_y = tlist.chrome_instances[0].rect[1];
        assert!((top_y - 16.0).abs() < 1e-3, "top toast at the top margin");

        // Bottom corner: toast's bottom edge near the bottom margin.
        let mut bot = ToastStack::new()
            .with_corner(Corner::BottomRight)
            .with_margin(16.0);
        bot.push(Toast::info("a"));
        let mut blist = DrawList::new();
        bot.draw(800.0, 600.0, &mut blist, &s_style);
        let r = blist.chrome_instances[0].rect;
        let bottom_edge = r[1] + r[3];
        assert!(
            (bottom_edge - (600.0 - 16.0)).abs() < 1e-3,
            "bottom toast bottom edge at the bottom margin"
        );
    }

    #[test]
    fn empty_stack_draws_nothing() {
        let s_style = style();
        let mut list = DrawList::new();
        let s = ToastStack::new();
        s.draw(800.0, 600.0, &mut list, &s_style);
        assert!(list.chrome_instances.is_empty());
    }
}
