//! The physical inputs a player can touch: keys, mouse buttons and gamepad controls.

use crate::names::named_enum;

named_enum! {
    /// A keyboard key, identified by where it sits on the keyboard.
    ///
    /// Names follow a US QWERTY layout, but they describe positions, not
    /// letters: `W` is the key above `S` on every layout, so WASD movement
    /// still works on AZERTY or Dvorak keyboards.
    #[allow(missing_docs, reason = "each variant is the key of the same name")]
    pub enum Key("key") {
        A, B, C, D, E, F, G, H, I, J, K, L, M,
        N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
        Space, Enter, Escape, Tab, Backspace, CapsLock,
        LeftShift, RightShift, LeftControl, RightControl,
        LeftAlt, RightAlt, LeftSuper, RightSuper,
        ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
        Insert, Delete, Home, End, PageUp, PageDown,
        Backquote, Minus, Equal, LeftBracket, RightBracket, Backslash,
        Semicolon, Quote, Comma, Period, Slash,
        Numpad0, Numpad1, Numpad2, Numpad3, Numpad4,
        Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
        NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide,
        NumpadDecimal, NumpadEnter,
    }
}

named_enum! {
    /// A mouse button.
    pub enum MouseButton("mouse button") {
        /// The primary button.
        MouseLeft,
        /// The secondary button.
        MouseRight,
        /// The wheel, pressed down.
        MouseMiddle,
        /// The first side button, usually "back".
        MouseBack,
        /// The second side button, usually "forward".
        MouseForward,
    }
}

named_enum! {
    /// One notch of the mouse wheel, treated as a momentary button press.
    pub enum MouseWheel("mouse wheel direction") {
        /// The wheel rolled away from the player.
        MouseWheelUp,
        /// The wheel rolled toward the player.
        MouseWheelDown,
    }
}

named_enum! {
    /// A gamepad button, named by position so it reads the same on every pad.
    ///
    /// `GamepadSouth` is A on an Xbox pad and Cross on a PlayStation pad.
    pub enum GamepadButton("gamepad button") {
        /// Bottom face button: Xbox A, PlayStation Cross.
        GamepadSouth,
        /// Right face button: Xbox B, PlayStation Circle.
        GamepadEast,
        /// Left face button: Xbox X, PlayStation Square.
        GamepadWest,
        /// Top face button: Xbox Y, PlayStation Triangle.
        GamepadNorth,
        /// Left shoulder button: Xbox LB, PlayStation L1.
        LeftBumper,
        /// Right shoulder button: Xbox RB, PlayStation R1.
        RightBumper,
        /// Left analog trigger: Xbox LT, PlayStation L2.
        LeftTrigger,
        /// Right analog trigger: Xbox RT, PlayStation R2.
        RightTrigger,
        /// Clicking the left stick in.
        LeftStickPress,
        /// Clicking the right stick in.
        RightStickPress,
        /// Directional pad, up.
        DpadUp,
        /// Directional pad, down.
        DpadDown,
        /// Directional pad, left.
        DpadLeft,
        /// Directional pad, right.
        DpadRight,
        /// Right-hand menu button: Xbox Menu, PlayStation Options.
        GamepadStart,
        /// Left-hand menu button: Xbox View, PlayStation Share.
        GamepadSelect,
    }
}

named_enum! {
    /// A gamepad thumbstick, read as a two-dimensional direction.
    pub enum Stick("gamepad stick") {
        /// The left thumbstick.
        LeftStick,
        /// The right thumbstick.
        RightStick,
    }
}

/// One axis of a thumbstick, as reported by the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StickAxis {
    /// Left to right; right is positive.
    X,
    /// Down to up; up is positive.
    Y,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for key in Key::ALL {
            assert_eq!(key.name().parse::<Key>(), Ok(*key));
        }
        for button in GamepadButton::ALL {
            assert_eq!(button.name().parse::<GamepadButton>(), Ok(*button));
        }
    }

    #[test]
    fn names_ignore_case() {
        assert_eq!("space".parse::<Key>(), Ok(Key::Space));
        assert_eq!(
            "MOUSELEFT".parse::<MouseButton>(),
            Ok(MouseButton::MouseLeft)
        );
    }

    #[test]
    fn typo_gets_a_suggestion() {
        let error = "GamepadSuoth".parse::<GamepadButton>().unwrap_err();
        assert_eq!(error.suggestion.as_deref(), Some("GamepadSouth"));
    }

    #[test]
    fn index_matches_position_in_all() {
        for (position, key) in Key::ALL.iter().enumerate() {
            assert_eq!(key.index(), position);
        }
    }

    #[test]
    fn names_are_unique_across_devices() {
        let mut names: Vec<&str> = Key::ALL.iter().map(|v| v.name()).collect();
        names.extend(MouseButton::ALL.iter().map(|v| v.name()));
        names.extend(MouseWheel::ALL.iter().map(|v| v.name()));
        names.extend(GamepadButton::ALL.iter().map(|v| v.name()));
        names.extend(Stick::ALL.iter().map(|v| v.name()));
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total);
    }
}
