//! [`Frame`] — a closure-scoped bracket around the interactive UI lifecycle.
//!
//! Building an interactive frame with [`UiContext`] requires three calls in a
//! fixed order:
//!
//! ```ignore
//! state.begin_frame(&mut input, &theme, dt, &KeyboardNav); // map nav + seed focus/anim
//! {
//!     let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
//!     ui.text_button("OK", None, None);
//! }                                                // ctx drops (balance checks)
//! state.end_frame();                              // resolve Tab / tree nav
//! ```
//!
//! Forgetting the trailing [`UiState::end_frame`] silently breaks Tab/focus and
//! tree navigation — exactly the kind of edge-state bug that doesn't surface
//! until the next frame. [`Frame`] removes the footgun: you hand it the build
//! closure and the begin/end pair runs around it automatically. The navigation
//! [`NavMap`] is a required constructor argument, so the input-mapping step can't
//! be forgotten either.
//!
//! ```
//! use wgpu_gameui::{DrawList, Frame, InputState, KeyboardNav, Theme, UiState};
//!
//! let theme = Theme::default();
//! let mut input = InputState::default();
//! let mut state = UiState::new();
//! let mut list = DrawList::new();
//!
//! // `begin_frame` runs before the closure, `end_frame` after — both guaranteed.
//! // `KeyboardNav` maps arrows/Tab/Enter/Space/Escape into `input.nav`.
//! let clicked = Frame::new(&mut state, &mut input, &theme, &KeyboardNav)
//!     .dt(0.016)
//!     .run(&mut list, |ui| ui.text_button("OK", Some(120.0), Some(32.0)));
//! assert!(!clicked); // nothing was hovered/pressed this frame
//! ```
//!
//! ## Navigation mapping
//!
//! The fourth [`Frame::new`] argument is a [`NavMap`] — pass
//! [`KeyboardNav`](crate::KeyboardNav) for the default keyboard binding, a
//! closure that also folds in a [`GamepadNav`](crate::GamepadNav)
//! (`|i| { map_keyboard(i); map_gamepad(i, &pad); }`), or
//! [`ManualNav`](crate::ManualNav) if you populate `input.nav` yourself.
//!
//! ## Scope
//!
//! `Frame` brackets the **per-surface** [`UiState`] lifecycle (the one façade
//! that holds frame state). It deliberately does **not** call
//! [`InputState::end_frame`]: that clears per-frame edge events (clicks,
//! key presses, nav intents) and must run **once per whole application frame**,
//! after every surface, layer, and manual widget has been drawn — not once per UI
//! region. Keep that single `input.end_frame()` call at the end of your app's
//! frame.

use crate::nav::NavMap;
use crate::theme::Theme;
use crate::ui_context::{UiContext, UiState};
use crate::widgets::DrawList;
use crate::{InputState, layer::LayerStack};

/// A closure-scoped interactive frame: runs [`UiState::begin_frame`] before your
/// build closure and [`UiState::end_frame`] after it, so the pair can't be
/// forgotten or mis-ordered. See the [module docs](self) for rationale.
///
/// Construct with [`Frame::new`] (or [`UiState::frame`]) — supplying caller state,
/// this frame's input, the theme, and a [`NavMap`] — optionally set the animation
/// delta with [`dt`](Self::dt), then call [`run`](Self::run) (a [`DrawList`]) or
/// [`run_layers`](Self::run_layers) (a [`LayerStack`]). The closure receives an
/// interactive [`UiContext`]; its return value is passed straight back out.
pub struct Frame<'a> {
    state: &'a mut UiState,
    input: &'a mut InputState,
    theme: &'a Theme,
    nav: &'a dyn NavMap,
    dt: f32,
}

impl<'a> Frame<'a> {
    /// Begin a frame against caller-owned `state`, this frame's `input`, the
    /// active `theme`, and a navigation map `nav` (e.g.
    /// [`KeyboardNav`](crate::KeyboardNav)). `nav` fills `input.nav` at
    /// [`run`](Self::run) time so navigation wiring can't be forgotten. The
    /// animation delta defaults to `0.0` (frozen); set it with [`dt`](Self::dt).
    /// Nothing happens until [`run`](Self::run) / [`run_layers`](Self::run_layers)
    /// is called.
    pub fn new(
        state: &'a mut UiState,
        input: &'a mut InputState,
        theme: &'a Theme,
        nav: &'a dyn NavMap,
    ) -> Self {
        Self {
            state,
            input,
            theme,
            nav,
            dt: 0.0,
        }
    }

    /// Set the animation delta-time (seconds) passed to [`UiState::begin_frame`].
    /// `0.0` freezes hover/press easing (the default — good for static renders or
    /// paused frames); pass your real frame delta for animated transitions.
    pub fn dt(mut self, dt: f32) -> Self {
        self.dt = dt;
        self
    }

    /// Build an interactive frame into `list`: runs `begin_frame` (applying the
    /// [`NavMap`] to fill `input.nav`), invokes `build` with an interactive
    /// [`UiContext`], drops the context (firing its debug balance checks for
    /// unbalanced `push`/`pop`), then runs `end_frame`. Returns whatever `build`
    /// returns.
    pub fn run<R>(self, list: &mut DrawList, build: impl FnOnce(&mut UiContext) -> R) -> R {
        self.state
            .begin_frame(self.input, self.theme, self.dt, self.nav);
        let result = {
            let mut ui = UiContext::interactive(list, self.input, &mut *self.state, self.theme);
            build(&mut ui)
        };
        self.state.end_frame();
        result
    }

    /// Like [`run`](Self::run) but builds into a [`LayerStack`], enabling the
    /// `modal_begin`/`popup_begin` verbs. Runs `begin_frame`/`end_frame` around
    /// the closure and returns its value.
    pub fn run_layers<R>(
        self,
        layers: &mut LayerStack,
        build: impl FnOnce(&mut UiContext) -> R,
    ) -> R {
        self.state
            .begin_frame(self.input, self.theme, self.dt, self.nav);
        let result = {
            let mut ui =
                UiContext::interactive_layers(layers, self.input, &mut *self.state, self.theme);
            build(&mut ui)
        };
        self.state.end_frame();
        result
    }
}

impl UiState {
    /// Begin a closure-scoped interactive [`Frame`] against this state. Sugar for
    /// [`Frame::new(self, input, theme, nav)`](Frame::new):
    ///
    /// ```
    /// # use wgpu_gameui::{DrawList, InputState, KeyboardNav, Theme, UiState};
    /// # let theme = Theme::default();
    /// # let mut input = InputState::default();
    /// # let mut state = UiState::new();
    /// # let mut list = DrawList::new();
    /// state.frame(&mut input, &theme, &KeyboardNav).dt(0.016).run(&mut list, |ui| {
    ///     ui.text_button("Play", None, None);
    /// });
    /// ```
    pub fn frame<'a>(
        &'a mut self,
        input: &'a mut InputState,
        theme: &'a Theme,
        nav: &'a dyn NavMap,
    ) -> Frame<'a> {
        Frame::new(self, input, theme, nav)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputState;
    use crate::KeyboardNav;
    use crate::theme::Theme;

    #[test]
    fn run_threads_the_closure_return_value() {
        let theme = Theme::default();
        let mut input = InputState::default();
        let mut state = UiState::new();
        let mut list = DrawList::new();
        let out =
            Frame::new(&mut state, &mut input, &theme, &KeyboardNav).run(&mut list, |_ui| 42u32);
        assert_eq!(out, 42);
    }

    #[test]
    fn run_draws_into_the_provided_list() {
        let theme = Theme::default();
        let mut input = InputState::default();
        let mut state = UiState::new();
        let mut list = DrawList::new();
        Frame::new(&mut state, &mut input, &theme, &KeyboardNav).run(&mut list, |ui| {
            ui.text_button("OK", Some(100.0), Some(30.0));
        });
        // The button's chrome reached the list — the frame actually built.
        assert!(!list.chrome_instances.is_empty());
    }

    #[test]
    fn run_layers_builds_into_the_base_layer() {
        let theme = Theme::default();
        let mut input = InputState::default();
        let mut state = UiState::new();
        let mut layers = LayerStack::new();
        Frame::new(&mut state, &mut input, &theme, &KeyboardNav).run_layers(&mut layers, |ui| {
            ui.text_button("OK", Some(100.0), Some(30.0));
        });
        assert!(!layers.base().chrome_instances.is_empty());
    }

    /// The supplied [`NavMap`] runs as part of `begin_frame`: a raw `key_down`
    /// edge becomes `input.nav.down`. `Frame` doesn't call `InputState::end_frame`
    /// (that's the app's once-per-frame call), so the mapped intent is still
    /// observable on `input` after the frame returns.
    #[test]
    fn run_applies_the_nav_map() {
        let theme = Theme::default();
        let mut input = InputState {
            key_down: true,
            ..Default::default()
        };
        let mut state = UiState::new();
        let mut list = DrawList::new();
        Frame::new(&mut state, &mut input, &theme, &KeyboardNav).run(&mut list, |_ui| {});
        assert!(input.nav.down, "KeyboardNav should map key_down → nav.down");
    }

    /// Proves `begin_frame` runs (the anim clock is ticked by `dt`) AND
    /// `end_frame` runs (frame 1's settled state carries into frame 2): the
    /// hovered button's fill eases strictly between idle and hover colors. If
    /// `begin_frame` were skipped, the bg would jump instantly to `button_hover`.
    #[test]
    fn run_brackets_begin_and_end_frame() {
        let theme = Theme::default();
        let mut state = UiState::new();

        // Frame 1: idle, dt=0 settles the fill at `theme.button`.
        let mut idle = InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..Default::default()
        };
        let mut list1 = DrawList::new();
        Frame::new(&mut state, &mut idle, &theme, &KeyboardNav).run(&mut list1, |ui| {
            ui.text_button("OK", Some(100.0), Some(30.0));
        });
        assert_eq!(list1.chrome_instances[0].bg, theme.button);

        // Frame 2: hover the same call-order-stable button at dt < duration →
        // the fill is partway between `button` and `button_hover`.
        let mut hover = InputState {
            mouse_x: 10.0,
            mouse_y: 10.0,
            ..Default::default()
        };
        let mut list2 = DrawList::new();
        Frame::new(&mut state, &mut hover, &theme, &KeyboardNav)
            .dt(0.04)
            .run(&mut list2, |ui| {
                ui.text_button("OK", Some(100.0), Some(30.0));
            });
        let bg = list2.chrome_instances[0].bg;
        assert_ne!(bg, theme.button, "hover should have started easing away from idle");
        assert_ne!(bg, theme.button_hover, "a sub-duration dt should not reach the hover color yet");
    }

    #[test]
    fn uistate_frame_sugar_matches_frame_new() {
        let theme = Theme::default();
        let mut input = InputState::default();
        let mut state = UiState::new();
        let mut list = DrawList::new();
        let out = state
            .frame(&mut input, &theme, &KeyboardNav)
            .dt(0.016)
            .run(&mut list, |_ui| 7i32);
        assert_eq!(out, 7);
        assert!(list.texts.is_empty() && list.chrome_instances.is_empty());
    }
}
