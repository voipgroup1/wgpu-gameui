//! Tabs widget - a row of tab buttons.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{AnimSlot, AnimationState, Easing, InputState, StyleKey, StyleResolver};

use super::DrawList;

/// Output from drawing tabs.
pub struct TabsOutput {
    /// The tab that was clicked, if any.
    pub clicked: Option<usize>,
    /// The rect occupied by the tab bar.
    pub rect: Rect,
}

/// Tabs widget - a row of tab buttons.
///
/// # Example
/// ```ignore
/// let output = Tabs::new(&["Overview", "Stats", "Needs"])
///     .draw(rect, active_tab, &mut draw_list, &theme, &input);
/// if let Some(clicked) = output.clicked {
///     active_tab = clicked;
/// }
/// ```
pub struct Tabs<'a> {
    labels: &'a [&'a str],
    tab_height: f32,
    /// Base id for per-tab animation. When set (and an
    /// [`AnimationState`] is passed to [`draw`](Self::draw)), each tab's
    /// background and label color ease between active/hover/inactive states under
    /// the sub-key `base_id.wrapping_add(tab_index)`.
    anim_id: Option<u64>,
}

impl<'a> Tabs<'a> {
    /// Create a tab strip over the given labels, one tab per entry.
    pub fn new(labels: &'a [&'a str]) -> Self {
        Self {
            labels,
            tab_height: 28.0,
            anim_id: None,
        }
    }

    /// Set the height of the tab strip, in pixels.
    pub fn with_height(mut self, height: f32) -> Self {
        self.tab_height = height;
        self
    }

    /// Smooth each tab's background + label color transitions, keyed off `base_id`
    /// (per-tab sub-key `base_id.wrapping_add(i)`). Requires an
    /// [`AnimationState`] passed to [`draw`](Self::draw); a no-op (byte-identical)
    /// otherwise. `base_id` must be stable across frames.
    pub fn animated(mut self, base_id: u64) -> Self {
        self.anim_id = Some(base_id);
        self
    }

    /// Draw the tabs at the top of the given rect.
    /// Returns which tab was clicked (if any) and the rect used.
    pub fn draw(
        &self,
        rect: Rect,
        active: usize,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &InputState,
        mut anim: Option<&mut AnimationState>,
    ) -> TabsOutput {
        let duration = style.scalar(StyleKey::AnimationDuration);
        let tab_count = self.labels.len();
        if tab_count == 0 {
            return TabsOutput {
                clicked: None,
                rect: Rect::new(rect.x, rect.y, rect.width, 0.0),
            };
        }

        let tab_width = rect.width / tab_count as f32;
        let bar_rect = Rect::new(rect.x, rect.y, rect.width, self.tab_height);
        let mut clicked = None;

        // Draw background for entire tab bar
        list.quad(
            bar_rect.x,
            bar_rect.y,
            bar_rect.width,
            bar_rect.height,
            style.color(StyleKey::TabInactive),
        );

        // Draw each tab
        for (i, label) in self.labels.iter().enumerate() {
            let tab_x = rect.x + i as f32 * tab_width;
            let tab_rect = Rect::new(tab_x, rect.y, tab_width, self.tab_height);

            let is_active = i == active;
            let is_hovered = tab_rect.contains(input.mouse_x, input.mouse_y);
            let is_clicked = is_hovered && input.mouse_clicked;

            if is_clicked {
                clicked = Some(i);
            }

            // Tab background — resolve the discrete target, then ease toward it
            // (no-op without an AnimationState/anim_id → byte-identical).
            let target_bg = if is_active {
                style.color(StyleKey::TabActive)
            } else if is_hovered {
                style.color(StyleKey::TabHover)
            } else {
                style.color(StyleKey::TabInactive)
            };
            let bg_color = match (self.anim_id, anim.as_deref_mut()) {
                (Some(base), Some(a)) => a.animate_color(
                    base.wrapping_add(i as u64),
                    AnimSlot::Bg,
                    target_bg,
                    duration,
                    Easing::EaseOut,
                ),
                _ => target_bg,
            };
            list.quad(tab_x, rect.y, tab_width, self.tab_height, bg_color);

            // Active indicator (bottom border for active tab)
            if is_active {
                list.quad(
                    tab_x,
                    rect.y + self.tab_height - 2.0,
                    tab_width,
                    2.0,
                    style.color(StyleKey::Accent),
                );
            }

            // Tab separator (right edge, except for last tab)
            if i < tab_count - 1 {
                list.quad(
                    tab_x + tab_width - 1.0,
                    rect.y + 4.0,
                    1.0,
                    self.tab_height - 8.0,
                    style.color(StyleKey::TabBorder),
                );
            }

            // Tab label (centered) — eased between active/inactive text colors.
            let target_text = if is_active {
                style.color(StyleKey::TextHighlight)
            } else {
                style.color(StyleKey::Text)
            };
            let text_color = match (self.anim_id, anim.as_deref_mut()) {
                (Some(base), Some(a)) => a.animate_color(
                    base.wrapping_add(i as u64),
                    AnimSlot::Text,
                    target_text,
                    duration,
                    Easing::EaseOut,
                ),
                _ => target_text,
            };
            let font_size = style.scalar(StyleKey::FontSize) * 0.8;
            let text_y = list.vcentered_text_y(
                rect.y,
                self.tab_height,
                font_size,
                style.theme().font.as_ref(),
                label,
            );
            let (text_width, _) = list.measure_text(label, font_size, None);
            let text_x = tab_x + (tab_width - text_width) / 2.0;

            let text = TextBlock::new(*label, text_x, text_y)
                .with_size(font_size)
                .with_color(
                    (text_color[0] * 255.0) as u8,
                    (text_color[1] * 255.0) as u8,
                    (text_color[2] * 255.0) as u8,
                )
                .with_font_opt(style.theme().font.clone());
            list.text(text);
        }

        // Bottom border for tab bar
        list.quad(
            rect.x,
            rect.y + self.tab_height - 1.0,
            rect.width,
            1.0,
            style.color(StyleKey::TabBorder),
        );

        TabsOutput {
            clicked,
            rect: bar_rect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    const W: f32 = 240.0;
    const H: f32 = 100.0;
    const TAB_H: f32 = 28.0;

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, W, H)
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..Default::default()
        }
    }

    /// Background fill of tab `i` (the full-size bg quad at that tab's x, height
    /// `TAB_H` — distinguishes it from the 2px indicator and 1px separators).
    fn tab_bg(list: &DrawList, i: usize, tab_width: f32) -> [f32; 4] {
        let tab_x = i as f32 * tab_width;
        list.chrome_instances
            .iter()
            .find(|c| {
                (c.rect[0] - tab_x).abs() < 0.01
                    && (c.rect[2] - tab_width).abs() < 0.01
                    && (c.rect[3] - TAB_H).abs() < 0.01
            })
            .map(|c| c.bg)
            .expect("tab should emit a background quad")
    }

    #[test]
    fn animated_without_state_is_byte_identical() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let labels = ["A", "B", "C"];
        let tab_width = W / 3.0;

        let mut plain = DrawList::new();
        Tabs::new(&labels).draw(rect(), 0, &mut plain, &s, &idle(), None);
        let mut anim = DrawList::new();
        Tabs::new(&labels)
            .animated(100)
            .draw(rect(), 0, &mut anim, &s, &idle(), None);

        for i in 0..3 {
            assert_eq!(tab_bg(&plain, i, tab_width), tab_bg(&anim, i, tab_width));
        }
    }

    #[test]
    fn animated_active_bg_eases_on_switch() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let labels = ["A", "B", "C"];
        let tab_width = W / 3.0;
        let mut state = AnimationState::new();

        // Frame 1: tab 0 active → tab 1 settles at TabInactive.
        let mut l1 = DrawList::new();
        Tabs::new(&labels)
            .animated(100)
            .draw(rect(), 0, &mut l1, &s, &idle(), Some(&mut state));
        assert_eq!(tab_bg(&l1, 1, tab_width), theme.tab_inactive);

        // Tick a partial dt, switch active to tab 1: its bg eases toward TabActive.
        state.tick(0.04);
        let mut l2 = DrawList::new();
        Tabs::new(&labels)
            .animated(100)
            .draw(rect(), 1, &mut l2, &s, &idle(), Some(&mut state));
        let bg = tab_bg(&l2, 1, tab_width);
        let (lo, hi) = (
            theme.tab_inactive[0].min(theme.tab_active[0]),
            theme.tab_inactive[0].max(theme.tab_active[0]),
        );
        assert!(
            bg[0] > lo && bg[0] < hi,
            "mid-transition tab bg {} should be strictly between {} and {}",
            bg[0],
            lo,
            hi
        );
    }

    #[test]
    fn animated_first_frame_is_target_no_pop() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let labels = ["A", "B", "C"];
        let tab_width = W / 3.0;
        let mut state = AnimationState::new();

        // First sight: active tab 1 draws TabActive directly (no fade-in).
        let mut l = DrawList::new();
        Tabs::new(&labels)
            .animated(100)
            .draw(rect(), 1, &mut l, &s, &idle(), Some(&mut state));
        assert_eq!(tab_bg(&l, 1, tab_width), theme.tab_active);
    }
}
