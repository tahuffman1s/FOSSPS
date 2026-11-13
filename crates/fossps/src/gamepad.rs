//! Reads gamepads and feeds them into the input state.

use fossps_input::{GamepadButton, Input, Stick, StickAxis};
use gilrs::{Axis, EventType, GamepadId, Gilrs, GilrsBuilder};

use crate::translate;

/// How far a d-pad reported as an axis must be pushed to count as pressed.
const DPAD_AXIS_PRESS: f32 = 0.5;

/// How hard an idle gamepad must be pushed to take over from the active one.
///
/// Stick drift and resting-trigger noise stay well below this, so a gamepad
/// lying on the desk cannot steal control.
const TAKE_OVER_AT: f32 = 0.5;

/// Every stick and d-pad axis FOSSPS reads.
const AXES: [Axis; 6] = [
    Axis::LeftStickX,
    Axis::LeftStickY,
    Axis::RightStickX,
    Axis::RightStickY,
    Axis::DPadX,
    Axis::DPadY,
];

/// Every gamepad-library button FOSSPS has a name for.
const BUTTONS: [gilrs::Button; 16] = [
    gilrs::Button::South,
    gilrs::Button::East,
    gilrs::Button::West,
    gilrs::Button::North,
    gilrs::Button::LeftTrigger,
    gilrs::Button::RightTrigger,
    gilrs::Button::LeftTrigger2,
    gilrs::Button::RightTrigger2,
    gilrs::Button::LeftThumb,
    gilrs::Button::RightThumb,
    gilrs::Button::DPadUp,
    gilrs::Button::DPadDown,
    gilrs::Button::DPadLeft,
    gilrs::Button::DPadRight,
    gilrs::Button::Start,
    gilrs::Button::Select,
];

/// Every connected gamepad, merged into one player's input.
///
/// Only one gamepad plays at a time: the one that most recently produced input.
/// Picking up a different gamepad just works, and an idle one cannot fight the
/// active one.
pub(crate) struct Gamepads {
    /// `None` when the gamepad library could not start; the game then runs without gamepads.
    gilrs: Option<Gilrs>,
    active: Option<GamepadId>,
}

impl Gamepads {
    /// Starts watching for gamepads. Failure is logged, not fatal.
    pub(crate) fn new() -> Self {
        // FOSSPS applies its own dead zones and d-pad handling, so the
        // library's filters are turned off to avoid doing both.
        let gilrs = match GilrsBuilder::new().with_default_filters(false).build() {
            Ok(gilrs) => Some(gilrs),
            Err(error) => {
                log::warn!(
                    "Gamepads are unavailable: {error}. Keyboard and mouse still work. \
                     On Linux, check that your user can read /dev/input."
                );
                None
            }
        };
        Self {
            gilrs,
            active: None,
        }
    }

    /// Applies every gamepad event since the last call to `input`.
    ///
    /// While `focused` is false events are still drained but ignored, so a
    /// gamepad does not play the game while another window is in front.
    pub(crate) fn poll(&mut self, input: &mut Input, focused: bool) {
        let Some(gilrs) = &mut self.gilrs else {
            return;
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    log::info!("Gamepad connected: {}", gilrs.gamepad(event.id).name());
                }
                EventType::Disconnected => {
                    log::info!("Gamepad disconnected: {}", gilrs.gamepad(event.id).name());
                    if self.active == Some(event.id) {
                        self.active = None;
                        input.release_gamepad();
                    }
                }
                // Every button change arrives as `ButtonChanged`, including the
                // ones that also send `ButtonPressed` or `ButtonReleased`, so
                // this one arm sees them all and FOSSPS applies its own
                // press threshold.
                EventType::ButtonChanged(button, amount, _) if focused => {
                    if let Some(button) = translate::gamepad_button(button)
                        && Self::activate(&mut self.active, event.id, amount, input)
                    {
                        input.set_button_amount(button, amount);
                    }
                }
                EventType::AxisChanged(axis, value, _)
                    if focused && Self::activate(&mut self.active, event.id, value, input) =>
                {
                    apply_axis(input, axis, value);
                }
                _ => {}
            }
        }
    }

    /// Whether an event of strength `amount` from gamepad `id` should be applied.
    ///
    /// The active gamepad's events always are. Another gamepad takes over, and
    /// the previous one's controls are let go, only when it is pushed firmly.
    fn activate(
        active: &mut Option<GamepadId>,
        id: GamepadId,
        amount: f32,
        input: &mut Input,
    ) -> bool {
        if *active == Some(id) {
            return true;
        }
        if amount.abs() < TAKE_OVER_AT {
            return false;
        }
        if active.is_some() {
            input.release_gamepad();
        }
        *active = Some(id);
        true
    }

    /// Reads the active gamepad's current state into `input`.
    ///
    /// Gamepads only report changes, and changes are ignored while the window
    /// is unfocused. Without this, a stick held through a focus change would
    /// read as centred until it next moved.
    pub(crate) fn resync(&mut self, input: &mut Input) {
        let (Some(gilrs), Some(id)) = (&mut self.gilrs, self.active) else {
            return;
        };
        // Bring the library's own picture of the gamepad up to date first.
        while gilrs.next_event().is_some() {}
        let Some(gamepad) = gilrs.connected_gamepad(id) else {
            self.active = None;
            input.release_gamepad();
            return;
        };
        for axis in AXES {
            apply_axis(input, axis, gamepad.value(axis));
        }
        for button in BUTTONS {
            if let (Some(name), Some(data)) = (
                translate::gamepad_button(button),
                gamepad.button_data(button),
            ) {
                input.set_button_amount(name, data.value());
            }
        }
    }
}

/// Applies one axis reading. Some gamepads report the d-pad as two axes.
fn apply_axis(input: &mut Input, axis: Axis, value: f32) {
    match axis {
        Axis::LeftStickX => input.set_stick(Stick::LeftStick, StickAxis::X, value),
        Axis::LeftStickY => input.set_stick(Stick::LeftStick, StickAxis::Y, value),
        Axis::RightStickX => input.set_stick(Stick::RightStick, StickAxis::X, value),
        Axis::RightStickY => input.set_stick(Stick::RightStick, StickAxis::Y, value),
        Axis::DPadX => dpad_axis(
            input,
            GamepadButton::DpadLeft,
            GamepadButton::DpadRight,
            value,
        ),
        Axis::DPadY => dpad_axis(input, GamepadButton::DpadDown, GamepadButton::DpadUp, value),
        Axis::LeftZ | Axis::RightZ | Axis::Unknown => {}
    }
}

/// Turns one d-pad axis into its two buttons.
fn dpad_axis(input: &mut Input, negative: GamepadButton, positive: GamepadButton, value: f32) {
    for (button, pushed) in [
        (negative, value <= -DPAD_AXIS_PRESS),
        (positive, value >= DPAD_AXIS_PRESS),
    ] {
        if pushed {
            input.press(button);
        } else {
            input.release(button);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fossps_input::ActionMap;

    #[test]
    fn dpad_axes_become_buttons() {
        let mut map = ActionMap::new();
        let menu = map.axis2d("menu", ["Dpad"]).unwrap();
        let mut input = Input::new(map);

        apply_axis(&mut input, Axis::DPadX, -1.0);
        apply_axis(&mut input, Axis::DPadY, 1.0);
        let pushed = input.tick().axis2d(menu);
        assert!(pushed[0] < 0.0 && pushed[1] > 0.0);

        apply_axis(&mut input, Axis::DPadX, 0.0);
        apply_axis(&mut input, Axis::DPadY, 0.0);
        assert_eq!(input.tick().axis2d(menu), [0.0, 0.0]);
    }

    #[test]
    fn every_listed_button_has_a_name() {
        for button in BUTTONS {
            assert!(translate::gamepad_button(button).is_some(), "{button:?}");
        }
    }

    #[test]
    fn sticks_keep_up_as_positive() {
        let mut map = ActionMap::new();
        let movement = map.axis2d("move", ["LeftStick"]).unwrap();
        let mut input = Input::new(map);
        apply_axis(&mut input, Axis::LeftStickY, 1.0);
        assert_eq!(input.tick().axis2d(movement), [0.0, 1.0]);
    }
}
