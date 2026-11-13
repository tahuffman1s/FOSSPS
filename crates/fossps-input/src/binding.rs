//! What an action can be bound to.

use std::fmt;
use std::str::FromStr;

use crate::devices::{GamepadButton, Key, MouseButton, MouseWheel, Stick, StickAxis};
use crate::names::{UnknownName, unknown};

/// Anything that can be pressed: a key, a mouse button, a wheel notch or a gamepad button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// A keyboard key.
    Key(Key),
    /// A mouse button.
    Mouse(MouseButton),
    /// One notch of the mouse wheel; it counts as pressed for a single tick.
    Wheel(MouseWheel),
    /// A gamepad button or trigger.
    Gamepad(GamepadButton),
}

impl Button {
    /// How many distinct buttons exist across all devices.
    pub(crate) const COUNT: usize =
        Key::COUNT + MouseButton::COUNT + MouseWheel::COUNT + GamepadButton::COUNT;

    /// A unique position for this button in a flat array of [`Self::COUNT`] slots.
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Key(key) => key.index(),
            Self::Mouse(button) => Key::COUNT + button.index(),
            Self::Wheel(wheel) => Key::COUNT + MouseButton::COUNT + wheel.index(),
            Self::Gamepad(button) => {
                Key::COUNT + MouseButton::COUNT + MouseWheel::COUNT + button.index()
            }
        }
    }

    /// The name used in bindings, for example `"Space"` or `"GamepadSouth"`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Key(key) => key.name(),
            Self::Mouse(button) => button.name(),
            Self::Wheel(wheel) => wheel.name(),
            Self::Gamepad(button) => button.name(),
        }
    }

    /// Whether this button is on a gamepad.
    #[must_use]
    pub const fn is_gamepad(self) -> bool {
        matches!(self, Self::Gamepad(_))
    }

    /// Every button name on every device.
    fn all_names() -> impl Iterator<Item = &'static str> {
        (Key::ALL.iter().map(|v| v.name()))
            .chain(MouseButton::ALL.iter().map(|v| v.name()))
            .chain(MouseWheel::ALL.iter().map(|v| v.name()))
            .chain(GamepadButton::ALL.iter().map(|v| v.name()))
    }

    /// Parses a button name without building an error on failure.
    fn lookup(text: &str) -> Option<Self> {
        if let Ok(key) = text.parse() {
            Some(Self::Key(key))
        } else if let Ok(button) = text.parse() {
            Some(Self::Mouse(button))
        } else if let Ok(wheel) = text.parse() {
            Some(Self::Wheel(wheel))
        } else if let Ok(button) = text.parse() {
            Some(Self::Gamepad(button))
        } else {
            None
        }
    }
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Button {
    type Err = UnknownName;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::lookup(text).ok_or_else(|| unknown("key or button", text, Self::all_names()))
    }
}

impl From<Key> for Button {
    fn from(key: Key) -> Self {
        Self::Key(key)
    }
}

impl From<MouseButton> for Button {
    fn from(button: MouseButton) -> Self {
        Self::Mouse(button)
    }
}

impl From<MouseWheel> for Button {
    fn from(wheel: MouseWheel) -> Self {
        Self::Wheel(wheel)
    }
}

impl From<GamepadButton> for Button {
    fn from(button: GamepadButton) -> Self {
        Self::Gamepad(button)
    }
}

/// One way of driving an action.
///
/// Bindings parse from plain-English text, so `"Space"`, `"WASD"` and
/// `"LeftStick"` are all valid. See [`Binding::from_str`] for the full list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// A single key or button. Drives a button action.
    Button(Button),
    /// Two buttons pulling a 1D axis in opposite directions.
    Axis {
        /// Pushes the value toward -1.
        negative: Button,
        /// Pushes the value toward +1.
        positive: Button,
    },
    /// One axis of a thumbstick. Drives a 1D axis action.
    StickAxis(Stick, StickAxis),
    /// Four buttons forming a 2D direction, such as WASD.
    Directions {
        /// Pushes toward +Y.
        up: Button,
        /// Pushes toward -Y.
        down: Button,
        /// Pushes toward -X.
        left: Button,
        /// Pushes toward +X.
        right: Button,
    },
    /// A thumbstick. Drives a 2D axis action.
    Stick(Stick),
    /// Mouse movement, reported in raw counts through `delta`. Drives a 2D axis action.
    MouseMotion,
}

/// The shape of value a binding produces; it must match the action's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionKind {
    /// Pressed or not, like jump or fire.
    Button,
    /// A single number from -1 to 1, like lean.
    Axis1D,
    /// A direction with X and Y from -1 to 1, like move or look.
    Axis2D,
}

impl ActionKind {
    /// How this kind is described in error messages.
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::Button => "a button",
            Self::Axis1D => "a 1D axis (one number)",
            Self::Axis2D => "a 2D axis (a direction)",
        }
    }
}

/// Shorthand names for common composite bindings.
const SHORTHANDS: &[(&str, Binding)] = &[
    ("WASD", Binding::directions(Key::W, Key::S, Key::A, Key::D)),
    (
        "ArrowKeys",
        Binding::directions(
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
        ),
    ),
    (
        "Dpad",
        Binding::Directions {
            up: Button::Gamepad(GamepadButton::DpadUp),
            down: Button::Gamepad(GamepadButton::DpadDown),
            left: Button::Gamepad(GamepadButton::DpadLeft),
            right: Button::Gamepad(GamepadButton::DpadRight),
        },
    ),
    ("LeftStick", Binding::Stick(Stick::LeftStick)),
    ("RightStick", Binding::Stick(Stick::RightStick)),
    (
        "LeftStickX",
        Binding::StickAxis(Stick::LeftStick, StickAxis::X),
    ),
    (
        "LeftStickY",
        Binding::StickAxis(Stick::LeftStick, StickAxis::Y),
    ),
    (
        "RightStickX",
        Binding::StickAxis(Stick::RightStick, StickAxis::X),
    ),
    (
        "RightStickY",
        Binding::StickAxis(Stick::RightStick, StickAxis::Y),
    ),
    ("MouseMotion", Binding::MouseMotion),
];

impl Binding {
    /// Four keys forming a 2D direction.
    #[must_use]
    pub const fn directions(up: Key, down: Key, left: Key, right: Key) -> Self {
        Self::Directions {
            up: Button::Key(up),
            down: Button::Key(down),
            left: Button::Key(left),
            right: Button::Key(right),
        }
    }

    /// The kind of action this binding can drive.
    #[must_use]
    pub const fn kind(self) -> ActionKind {
        match self {
            Self::Button(_) => ActionKind::Button,
            Self::Axis { .. } | Self::StickAxis(..) => ActionKind::Axis1D,
            Self::Directions { .. } | Self::Stick(_) | Self::MouseMotion => ActionKind::Axis2D,
        }
    }

    /// Whether this binding is driven by a gamepad.
    #[must_use]
    pub const fn is_gamepad(self) -> bool {
        match self {
            Self::Button(button) => button.is_gamepad(),
            Self::Axis { negative, positive } => negative.is_gamepad() && positive.is_gamepad(),
            Self::Directions {
                up,
                down,
                left,
                right,
            } => up.is_gamepad() && down.is_gamepad() && left.is_gamepad() && right.is_gamepad(),
            Self::StickAxis(..) | Self::Stick(_) => true,
            Self::MouseMotion => false,
        }
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((name, _)) = SHORTHANDS.iter().find(|(_, binding)| binding == self) {
            return f.write_str(name);
        }
        match self {
            Self::Button(button) => f.write_str(button.name()),
            Self::Axis { negative, positive } => write!(f, "{negative}/{positive}"),
            Self::Directions {
                up,
                down,
                left,
                right,
            } => write!(f, "{up}/{down}/{left}/{right}"),
            // Sticks, stick axes and mouse motion are all shorthands, handled above.
            Self::StickAxis(stick, StickAxis::X) => write!(f, "{stick}X"),
            Self::StickAxis(stick, StickAxis::Y) => write!(f, "{stick}Y"),
            Self::Stick(stick) => f.write_str(stick.name()),
            Self::MouseMotion => f.write_str("MouseMotion"),
        }
    }
}

impl FromStr for Binding {
    type Err = UnknownName;

    /// Parses a binding from text. Names ignore case.
    ///
    /// - A key or button: `"Space"`, `"MouseLeft"`, `"MouseWheelUp"`, `"GamepadSouth"`.
    /// - A shorthand: `"WASD"`, `"ArrowKeys"`, `"Dpad"`, `"LeftStick"`, `"RightStick"`,
    ///   `"LeftStickX"`, `"RightStickY"`, `"MouseMotion"`.
    /// - Two buttons as a 1D axis, negative first: `"Q/E"`.
    /// - Four buttons as a 2D direction, in up/down/left/right order: `"I/K/J/L"`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if let Some((_, binding)) = SHORTHANDS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(text))
        {
            return Ok(*binding);
        }
        if let Some(button) = Button::lookup(text) {
            return Ok(Self::Button(button));
        }
        let mut parts = text.split('/').map(str::trim);
        let buttons = [parts.next(), parts.next(), parts.next(), parts.next()];
        match (buttons, parts.next()) {
            ([Some(negative), Some(positive), None, None], None) => Ok(Self::Axis {
                negative: part(text, negative)?,
                positive: part(text, positive)?,
            }),
            ([Some(up), Some(down), Some(left), Some(right)], None) => Ok(Self::Directions {
                up: part(text, up)?,
                down: part(text, down)?,
                left: part(text, left)?,
                right: part(text, right)?,
            }),
            _ => Err(unknown(
                "binding",
                text,
                Button::all_names().chain(SHORTHANDS.iter().map(|(name, _)| *name)),
            )),
        }
    }
}

/// Parses one button of the composite binding `whole`.
///
/// On a typo the error covers the whole binding, so its suggestion can be
/// pasted in place of what was written.
fn part(whole: &str, name: &str) -> Result<Button, UnknownName> {
    name.parse().map_err(|error: UnknownName| UnknownName {
        kind: "binding",
        given: whole.to_owned(),
        suggestion: error
            .suggestion
            .map(|fixed| whole.replacen(name, &fixed, 1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_indices_are_unique_and_dense() {
        let mut seen = [false; Button::COUNT];
        let all = (Key::ALL.iter().map(|k| Button::Key(*k)))
            .chain(MouseButton::ALL.iter().map(|b| Button::Mouse(*b)))
            .chain(MouseWheel::ALL.iter().map(|w| Button::Wheel(*w)))
            .chain(GamepadButton::ALL.iter().map(|b| Button::Gamepad(*b)));
        for button in all {
            assert!(!seen[button.index()], "{button} shares an index");
            seen[button.index()] = true;
        }
        assert!(seen.iter().all(|slot| *slot));
    }

    #[test]
    fn parses_buttons_from_every_device() {
        assert_eq!("space".parse(), Ok(Binding::Button(Key::Space.into())));
        assert_eq!(
            "MouseLeft".parse(),
            Ok(Binding::Button(MouseButton::MouseLeft.into()))
        );
        assert_eq!(
            "MouseWheelUp".parse(),
            Ok(Binding::Button(MouseWheel::MouseWheelUp.into()))
        );
        assert_eq!(
            "RightTrigger".parse(),
            Ok(Binding::Button(GamepadButton::RightTrigger.into()))
        );
    }

    #[test]
    fn parses_shorthands_and_composites() {
        assert_eq!(
            "wasd".parse(),
            Ok(Binding::directions(Key::W, Key::S, Key::A, Key::D))
        );
        assert_eq!("LeftStick".parse(), Ok(Binding::Stick(Stick::LeftStick)));
        assert_eq!(
            "Q / E".parse(),
            Ok(Binding::Axis {
                negative: Key::Q.into(),
                positive: Key::E.into()
            })
        );
        assert_eq!(
            "I/K/J/L".parse(),
            Ok(Binding::directions(Key::I, Key::K, Key::J, Key::L))
        );
    }

    #[test]
    fn display_round_trips() {
        for text in [
            "Space",
            "WASD",
            "Dpad",
            "RightStick",
            "LeftStickX",
            "MouseMotion",
            "Q/E",
            "I/K/J/L",
        ] {
            let binding: Binding = text.parse().unwrap();
            assert_eq!(binding.to_string(), text);
        }
    }

    #[test]
    fn typos_get_suggestions() {
        let error = "LeftStik".parse::<Binding>().unwrap_err();
        assert_eq!(error.suggestion.as_deref(), Some("LeftStick"));
        let error = "Q/Entr".parse::<Binding>().unwrap_err();
        assert_eq!(error.suggestion.as_deref(), Some("Q/Enter"));
        assert!("A/B/C".parse::<Binding>().is_err());
        assert!("A/B/C/D/E".parse::<Binding>().is_err());
    }

    #[test]
    fn gamepad_detection() {
        assert!("Dpad".parse::<Binding>().unwrap().is_gamepad());
        assert!("LeftTrigger".parse::<Binding>().unwrap().is_gamepad());
        assert!(!"WASD".parse::<Binding>().unwrap().is_gamepad());
        assert!(!"MouseMotion".parse::<Binding>().unwrap().is_gamepad());
    }
}
