//! Device-agnostic **navigation intents** and the mappers that fill them.
//!
//! The library never reads an input device — your game populates [`InputState`]
//! however it likes (winit, SDL, gilrs, a replay file). What the library *owns*
//! is the **meaning** of navigation: move the selection, confirm, cancel, focus
//! the next/previous widget. Those intents live in [`InputState::nav`] as a
//! small [`NavInput`] struct, and the focus system + widgets read *them* — never
//! raw key names. That way a keyboard and a gamepad drive the same UI through one
//! vocabulary.
//!
//! ## Who fills `input.nav`?
//!
//! A [`NavMap`]. It is a **required argument** to
//! [`UiState::begin_frame`](crate::UiState::begin_frame) (and
//! [`Frame::new`](crate::Frame)), so you can't build an interactive frame and
//! silently forget to wire navigation. Pick one:
//!
//! - [`KeyboardNav`] — the default binding: arrows → directional, `Tab` /
//!   `Shift+Tab` → next/prev, `Enter`/`Space` → confirm, `Escape` → cancel.
//! - A closure / `fn(&mut InputState)` — combine devices, e.g.
//!   `|i| { map_keyboard(i); map_gamepad(i, &pad); }`.
//! - [`ManualNav`] — a no-op: you've already set `input.nav` yourself (or you
//!   don't want any navigation this frame).
//!
//! ## Gamepad
//!
//! Fill a [`GamepadNav`] from your gamepad backend each frame (the library does
//! not touch the device, so deadzones / edge-detection on sticks are yours), then
//! feed it through [`map_gamepad`]:
//!
//! ```
//! use wgpu_gameui::{GamepadNav, InputState, map_gamepad};
//!
//! let mut input = InputState::default();
//! let pad = GamepadNav { south: true, dpad_down: true, ..Default::default() };
//! map_gamepad(&mut input, &pad);
//! assert!(input.nav.confirm); // A / Cross
//! assert!(input.nav.down);    // d-pad down
//! ```
//!
//! ## Raw keys still exist
//!
//! Intents are for *navigation* only. Text editors ([`TextInput`](crate::TextInput),
//! [`NumberInput`](crate::NumberInput)) keep reading the literal `key_left` /
//! `key_right` / `enter_pressed` fields for caret movement and submit — a focused
//! text field should get its arrow keys, and a gamepad d-pad must *not* move a
//! caret (it only sets `nav.*`). Both can come from the same physical key; the
//! focused widget decides which meaning applies.

use crate::InputState;

/// Per-frame, device-agnostic navigation intents, stored in
/// [`InputState::nav`]. Each field is an **edge** (the intent fired *this*
/// frame); hold-to-repeat is the game's concern (re-emit the edge, like key
/// repeat). Cleared by [`InputState::end_frame`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NavInput {
    /// Move selection / focus up (or decrement a focused slider).
    pub up: bool,
    /// Move selection / focus down (or decrement a focused slider).
    pub down: bool,
    /// Move selection / focus left (or decrement a focused slider).
    pub left: bool,
    /// Move selection / focus right (or increment a focused slider).
    pub right: bool,
    /// Activate the focused widget — A / Cross, `Enter`, or `Space`.
    pub confirm: bool,
    /// Back out / dismiss — B / Circle or `Escape`.
    pub cancel: bool,
    /// Focus the next widget in the ring — right shoulder or `Tab`.
    pub next: bool,
    /// Focus the previous widget in the ring — left shoulder or `Shift+Tab`.
    pub prev: bool,
}

/// A game-filled snapshot of the gamepad buttons that matter for UI navigation.
///
/// The library never reads a controller; fill this from your backend (gilrs,
/// SDL, …) each frame and pass it to [`map_gamepad`]. Button fields are treated
/// as "pressed this frame" edges. The stick fields are optional directional
/// input — apply your own deadzone and edge-detection before setting them
/// (a raw analog axis held off-center would otherwise nav every frame).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GamepadNav {
    /// D-pad up → [`NavInput::up`].
    pub dpad_up: bool,
    /// D-pad down → [`NavInput::down`].
    pub dpad_down: bool,
    /// D-pad left → [`NavInput::left`].
    pub dpad_left: bool,
    /// D-pad right → [`NavInput::right`].
    pub dpad_right: bool,
    /// South face button (A / Cross) → [`NavInput::confirm`].
    pub south: bool,
    /// East face button (B / Circle) → [`NavInput::cancel`].
    pub east: bool,
    /// Left shoulder (LB / L1) → [`NavInput::prev`].
    pub left_shoulder: bool,
    /// Right shoulder (RB / R1) → [`NavInput::next`].
    pub right_shoulder: bool,
    /// Left stick pushed up (caller-deadzoned edge) → [`NavInput::up`].
    pub stick_up: bool,
    /// Left stick pushed down (caller-deadzoned edge) → [`NavInput::down`].
    pub stick_down: bool,
    /// Left stick pushed left (caller-deadzoned edge) → [`NavInput::left`].
    pub stick_left: bool,
    /// Left stick pushed right (caller-deadzoned edge) → [`NavInput::right`].
    pub stick_right: bool,
}

/// Fills [`InputState::nav`] for the frame. Required by
/// [`UiState::begin_frame`](crate::UiState::begin_frame) so navigation wiring
/// can't be forgotten. Implemented by [`KeyboardNav`], [`ManualNav`], and any
/// `Fn(&mut InputState)` closure.
pub trait NavMap {
    /// Populate `input.nav` with this frame's navigation intents. Implementations
    /// typically *OR* into the existing intents so several maps can compose.
    fn apply(&self, input: &mut InputState);
}

/// The default keyboard binding: arrows → directional, `Tab` / `Shift+Tab` →
/// next/prev, `Enter`/`Space` → confirm, `Escape` → cancel. See [`map_keyboard`].
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyboardNav;

impl NavMap for KeyboardNav {
    fn apply(&self, input: &mut InputState) {
        map_keyboard(input);
    }
}

/// A no-op map: leaves `input.nav` exactly as the caller set it. Use when you
/// populate intents yourself (e.g. custom device fusion before the frame) or when
/// a frame should have no navigation at all.
#[derive(Debug, Clone, Copy, Default)]
pub struct ManualNav;

impl NavMap for ManualNav {
    fn apply(&self, _input: &mut InputState) {}
}

/// Any `Fn(&mut InputState)` is a [`NavMap`] — lets you pass a closure that
/// combines devices, e.g. `|i| { map_keyboard(i); map_gamepad(i, &pad); }`.
impl<F: Fn(&mut InputState)> NavMap for F {
    fn apply(&self, input: &mut InputState) {
        self(input)
    }
}

/// Map this frame's raw keyboard edges into [`InputState::nav`], OR-ing into any
/// intents already set (so it composes with [`map_gamepad`]).
///
/// - arrows → [`up`](NavInput::up)/[`down`](NavInput::down)/[`left`](NavInput::left)/[`right`](NavInput::right)
/// - `Enter` or `Space` → [`confirm`](NavInput::confirm)
/// - `Escape` → [`cancel`](NavInput::cancel)
/// - `Tab` → [`next`](NavInput::next), or [`prev`](NavInput::prev) with `Shift`
pub fn map_keyboard(input: &mut InputState) {
    input.nav.up |= input.key_up;
    input.nav.down |= input.key_down;
    input.nav.left |= input.key_left;
    input.nav.right |= input.key_right;
    input.nav.confirm |= input.enter_pressed || input.key_space;
    input.nav.cancel |= input.key_escape;
    if input.key_tab {
        if input.shift_pressed {
            input.nav.prev = true;
        } else {
            input.nav.next = true;
        }
    }
}

/// Map a game-filled [`GamepadNav`] snapshot into [`InputState::nav`], OR-ing into
/// any intents already set (so it composes with [`map_keyboard`]). D-pad and left
/// stick both feed the directional intents.
pub fn map_gamepad(input: &mut InputState, pad: &GamepadNav) {
    input.nav.up |= pad.dpad_up || pad.stick_up;
    input.nav.down |= pad.dpad_down || pad.stick_down;
    input.nav.left |= pad.dpad_left || pad.stick_left;
    input.nav.right |= pad.dpad_right || pad.stick_right;
    input.nav.confirm |= pad.south;
    input.nav.cancel |= pad.east;
    input.nav.next |= pad.right_shoulder;
    input.nav.prev |= pad.left_shoulder;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_arrows_map_to_directional() {
        let mut input = InputState {
            key_up: true,
            key_down: true,
            key_left: true,
            key_right: true,
            ..Default::default()
        };
        map_keyboard(&mut input);
        assert_eq!(
            input.nav,
            NavInput {
                up: true,
                down: true,
                left: true,
                right: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn keyboard_enter_and_space_both_confirm() {
        let mut a = InputState {
            enter_pressed: true,
            ..Default::default()
        };
        map_keyboard(&mut a);
        assert!(a.nav.confirm);

        let mut b = InputState {
            key_space: true,
            ..Default::default()
        };
        map_keyboard(&mut b);
        assert!(b.nav.confirm);
    }

    #[test]
    fn keyboard_escape_cancels() {
        let mut input = InputState {
            key_escape: true,
            ..Default::default()
        };
        map_keyboard(&mut input);
        assert!(input.nav.cancel);
    }

    #[test]
    fn keyboard_tab_is_next_and_shift_tab_is_prev() {
        let mut fwd = InputState {
            key_tab: true,
            ..Default::default()
        };
        map_keyboard(&mut fwd);
        assert!(fwd.nav.next && !fwd.nav.prev);

        let mut back = InputState {
            key_tab: true,
            shift_pressed: true,
            ..Default::default()
        };
        map_keyboard(&mut back);
        assert!(back.nav.prev && !back.nav.next);
    }

    #[test]
    fn gamepad_face_buttons_map_to_confirm_and_cancel() {
        let mut input = InputState::default();
        map_gamepad(
            &mut input,
            &GamepadNav {
                south: true,
                east: true,
                ..Default::default()
            },
        );
        assert!(input.nav.confirm);
        assert!(input.nav.cancel);
    }

    #[test]
    fn gamepad_shoulders_cycle_focus() {
        let mut input = InputState::default();
        map_gamepad(
            &mut input,
            &GamepadNav {
                left_shoulder: true,
                right_shoulder: true,
                ..Default::default()
            },
        );
        assert!(input.nav.prev);
        assert!(input.nav.next);
    }

    #[test]
    fn gamepad_stick_and_dpad_both_feed_directional() {
        let mut dpad = InputState::default();
        map_gamepad(
            &mut dpad,
            &GamepadNav {
                dpad_left: true,
                ..Default::default()
            },
        );
        assert!(dpad.nav.left);

        let mut stick = InputState::default();
        map_gamepad(
            &mut stick,
            &GamepadNav {
                stick_left: true,
                ..Default::default()
            },
        );
        assert!(stick.nav.left);
    }

    #[test]
    fn maps_compose_via_or() {
        let mut input = InputState {
            key_up: true, // keyboard up
            ..Default::default()
        };
        map_keyboard(&mut input);
        map_gamepad(
            &mut input,
            &GamepadNav {
                south: true, // gamepad confirm
                ..Default::default()
            },
        );
        assert!(input.nav.up); // from keyboard
        assert!(input.nav.confirm); // from gamepad — not clobbered
    }

    #[test]
    fn keyboard_nav_trait_matches_free_fn() {
        let mut via_trait = InputState {
            key_down: true,
            ..Default::default()
        };
        KeyboardNav.apply(&mut via_trait);

        let mut via_fn = InputState {
            key_down: true,
            ..Default::default()
        };
        map_keyboard(&mut via_fn);

        assert_eq!(via_trait.nav, via_fn.nav);
    }

    #[test]
    fn manual_nav_is_a_noop_preserving_caller_intents() {
        let mut input = InputState::default();
        input.nav.confirm = true; // caller set it directly
        ManualNav.apply(&mut input);
        assert!(input.nav.confirm); // untouched
    }

    #[test]
    fn closure_is_a_nav_map() {
        let pad = GamepadNav {
            dpad_up: true,
            ..Default::default()
        };
        let combined = |i: &mut InputState| {
            map_keyboard(i);
            map_gamepad(i, &pad);
        };
        let mut input = InputState {
            key_left: true,
            ..Default::default()
        };
        combined.apply(&mut input);
        assert!(input.nav.left); // keyboard
        assert!(input.nav.up); // gamepad d-pad
    }
}
