//! Input bindings — a capture-and-rebind value type for settings forms.
//!
//! A [`Binding`] is a device-agnostic snapshot of "what is pressed" drawn from
//! what [`InputState`] can already express: named keyboard keys, character keys
//! (with Ctrl/Shift/Alt chords), the three mouse buttons, and the gamepad
//! buttons of a [`GamepadNav`] snapshot. No new input plumbing — the enum is
//! deliberately bounded by the existing input layer, so anything the host can't
//! express there simply can't be bound (yet). Extensible by adding variants.

use crate::{GamepadNav, InputState};

/// One bindable input. Device-grouped: keyboard keys, then character chords,
/// then mouse, then gamepad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// A named keyboard key.
    Key(KeyCode),
    /// A character key with optional Ctrl/Shift/Alt modifiers (from
    /// `InputState::text_input` + the modifier held-flags).
    Char {
        /// The typed character, lowercased for comparison.
        c: char,
        /// Ctrl (Cmd) held.
        ctrl: bool,
        /// Shift held.
        shift: bool,
        /// Alt held.
        alt: bool,
    },
    /// Left mouse button.
    MouseLeft,
    /// Middle mouse button.
    MouseMiddle,
    /// Right mouse button.
    MouseRight,
    /// A gamepad control (a [`GamepadNav`] field, deadzone edges included).
    Pad(PadButton),
}

/// Named keyboard keys `InputState` carries as individual edge fields. Char
/// keys (letters/digits/symbols) go through [`Binding::Char`] instead — the
/// host hands them over as typed text, so there is no layout-independent
/// scancode to name them by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// <kbd>Esc</kbd>.
    Escape,
    /// <kbd>Tab</kbd>.
    Tab,
    /// <kbd>Space</kbd>.
    Space,
    /// <kbd>Enter</kbd>.
    Enter,
    /// <kbd>Backspace</kbd>.
    Backspace,
    /// <kbd>←</kbd>.
    Left,
    /// <kbd>→</kbd>.
    Right,
    /// <kbd>↑</kbd>.
    Up,
    /// <kbd>↓</kbd>.
    Down,
    /// <kbd>Home</kbd>.
    Home,
    /// <kbd>End</kbd>.
    End,
    /// <kbd>Del</kbd>.
    Delete,
}

impl KeyCode {
    /// Short human label ("Esc", "Space", "←").
    pub fn label(self) -> &'static str {
        match self {
            KeyCode::Escape => "Esc",
            KeyCode::Tab => "Tab",
            KeyCode::Space => "Space",
            KeyCode::Enter => "Enter",
            KeyCode::Backspace => "Bksp",
            KeyCode::Left => "←",
            KeyCode::Right => "→",
            KeyCode::Up => "↑",
            KeyCode::Down => "↓",
            KeyCode::Home => "Home",
            KeyCode::End => "End",
            KeyCode::Delete => "Del",
        }
    }

    /// Read this key's edge field from `input`.
    pub(crate) fn pressed(self, input: &InputState) -> bool {
        match self {
            KeyCode::Escape => input.key_escape,
            KeyCode::Tab => input.key_tab,
            KeyCode::Space => input.key_space,
            KeyCode::Enter => input.enter_pressed,
            KeyCode::Backspace => input.backspace_pressed,
            KeyCode::Left => input.key_left,
            KeyCode::Right => input.key_right,
            KeyCode::Up => input.key_up,
            KeyCode::Down => input.key_down,
            KeyCode::Home => input.key_home,
            KeyCode::End => input.key_end,
            KeyCode::Delete => input.key_delete,
        }
    }
}

/// The gamepad buttons a binding can capture — the fields of
/// [`GamepadNav`], which is what the host hands the UI per frame. Stick
/// directions use the same deadzoned edges the nav map consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadButton {
    /// D-pad up.
    DpadUp,
    /// D-pad down.
    DpadDown,
    /// D-pad left.
    DpadLeft,
    /// D-pad right.
    DpadRight,
    /// South face button (A / Cross).
    South,
    /// East face button (B / Circle).
    East,
    /// Left shoulder (LB / L1).
    LeftShoulder,
    /// Right shoulder (RB / R1).
    RightShoulder,
    /// Left stick up (deadzoned edge).
    StickUp,
    /// Left stick down (deadzoned edge).
    StickDown,
    /// Left stick left (deadzoned edge).
    StickLeft,
    /// Left stick right (deadzoned edge).
    StickRight,
}

impl PadButton {
    /// Short human label ("D-Up", "A", "LB", "Stick→").
    pub fn label(self) -> &'static str {
        match self {
            PadButton::DpadUp => "D-Up",
            PadButton::DpadDown => "D-Down",
            PadButton::DpadLeft => "D-Left",
            PadButton::DpadRight => "D-Right",
            PadButton::South => "A",
            PadButton::East => "B",
            PadButton::LeftShoulder => "LB",
            PadButton::RightShoulder => "RB",
            PadButton::StickUp => "Stick↑",
            PadButton::StickDown => "Stick↓",
            PadButton::StickLeft => "Stick←",
            PadButton::StickRight => "Stick→",
        }
    }

    /// Read this button from a gamepad snapshot.
    pub(crate) fn in_pad(self, pad: &GamepadNav) -> bool {
        match self {
            PadButton::DpadUp => pad.dpad_up,
            PadButton::DpadDown => pad.dpad_down,
            PadButton::DpadLeft => pad.dpad_left,
            PadButton::DpadRight => pad.dpad_right,
            PadButton::South => pad.south,
            PadButton::East => pad.east,
            PadButton::LeftShoulder => pad.left_shoulder,
            PadButton::RightShoulder => pad.right_shoulder,
            PadButton::StickUp => pad.stick_up,
            PadButton::StickDown => pad.stick_down,
            PadButton::StickLeft => pad.stick_left,
            PadButton::StickRight => pad.stick_right,
        }
    }
}

impl Binding {
    /// Label like "Ctrl+Shift+S", "←", "Mouse3", "A". Empty never.
    pub fn label(&self) -> String {
        match self {
            Binding::Key(k) => k.label().to_string(),
            Binding::Char { c, ctrl, shift, alt } => {
                let mut s = String::new();
                if *ctrl {
                    s.push_str("Ctrl+");
                }
                if *alt {
                    s.push_str("Alt+");
                }
                if *shift {
                    s.push_str("Shift+");
                }
                s.push(c.to_ascii_uppercase());
                s
            }
            Binding::MouseLeft => "Mouse1".to_string(),
            Binding::MouseMiddle => "Mouse3".to_string(),
            Binding::MouseRight => "Mouse2".to_string(),
            Binding::Pad(p) => p.label().to_string(),
        }
    }

    /// Whether `input` satisfies the binding this frame — a key/char edge or
    /// a still-held mouse/pad button. Held-state matching is what makes
    /// "press any key" capture possible from the same per-frame snapshot.
    /// `pad` is the same [`GamepadNav`] snapshot the host feeds
    /// [`map_gamepad`](crate::map_gamepad); keyboard-only hosts pass `None`.
    pub fn is_down(&self, input: &InputState, pad: Option<&GamepadNav>) -> bool {
        match self {
            Binding::Key(k) => k.pressed(input),
            Binding::Char { c, ctrl, shift, alt } => {
                input.text_input.chars().any(|ch| {
                    ch.eq_ignore_ascii_case(c)
                }) && input.ctrl_pressed == *ctrl
                    && input.shift_pressed == *shift
                    && input.alt_down == *alt
            }
            Binding::MouseLeft => input.mouse_down,
            Binding::MouseMiddle => input.mouse_middle_down,
            Binding::MouseRight => input.mouse_right_down,
            Binding::Pad(p) => pad.is_some_and(|pad| p.in_pad(pad)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn labels_cover_all_variants_and_read_naturally() {
        assert_eq!(Binding::Key(KeyCode::Escape).label(), "Esc");
        assert_eq!(Binding::Key(KeyCode::Left).label(), "←");
        assert_eq!(
            Binding::Char { c: 's', ctrl: true, shift: true, alt: false }.label(),
            "Ctrl+Shift+S"
        );
        assert_eq!(Binding::MouseRight.label(), "Mouse2");
        assert_eq!(Binding::Pad(PadButton::South).label(), "A");
        assert_eq!(Binding::Pad(PadButton::StickRight).label(), "Stick→");
    }

    #[test]
    fn key_edges_match_their_input_fields() {
        let mut input = InputState::default();
        input.key_escape = true;
        assert!(Binding::Key(KeyCode::Escape).is_down(&input, None));

        let mut input = InputState::default();
        input.key_down = true;
        assert!(Binding::Key(KeyCode::Down).is_down(&input, None));
        assert!(!Binding::Key(KeyCode::Up).is_down(&input, None));
    }

    #[test]
    fn char_binding_requires_exact_modifier_match() {
        let mut input = InputState::default();
        input.text_input = "s".into();
        input.ctrl_pressed = true;
        let bind = Binding::Char { c: 's', ctrl: true, shift: false, alt: false };
        assert!(bind.is_down(&input, None), "Ctrl+S matches");
        input.shift_pressed = true;
        assert!(!bind.is_down(&input, None), "extra Shift breaks the chord");
    }

    #[test]
    fn mouse_bindings_read_held_state() {
        let mut input = InputState::default();
        input.mouse_middle_down = true;
        assert!(Binding::MouseMiddle.is_down(&input, None));
        assert!(!Binding::MouseLeft.is_down(&input, None));
    }

    #[test]
    fn pad_binding_reads_the_passed_snapshot() {
        let input = InputState::default();
        assert!(
            !Binding::Pad(PadButton::East).is_down(&input, None),
            "no pad snapshot: nothing matches"
        );
        let pad = GamepadNav { east: true, ..Default::default() };
        assert!(Binding::Pad(PadButton::East).is_down(&input, Some(&pad)));
        assert!(!Binding::Pad(PadButton::South).is_down(&input, Some(&pad)));
    }

    #[test]
    fn all_keys_and_pad_buttons_have_labels() {
        for k in [
            KeyCode::Escape,
            KeyCode::Tab,
            KeyCode::Space,
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Delete,
        ] {
            assert!(!k.label().is_empty());
        }
        for p in [
            PadButton::DpadUp,
            PadButton::DpadDown,
            PadButton::DpadLeft,
            PadButton::DpadRight,
            PadButton::South,
            PadButton::East,
            PadButton::LeftShoulder,
            PadButton::RightShoulder,
            PadButton::StickUp,
            PadButton::StickDown,
            PadButton::StickLeft,
            PadButton::StickRight,
        ] {
            assert!(!p.label().is_empty());
        }
    }

    #[test]
    fn char_label_uppercases_and_orders_modifiers() {
        let b = Binding::Char { c: 'a', ctrl: false, shift: false, alt: true };
        assert_eq!(b.label(), "Alt+A");
        let b = Binding::Char { c: 'x', ctrl: true, shift: true, alt: true };
        assert_eq!(b.label(), "Ctrl+Alt+Shift+X");
    }
}
