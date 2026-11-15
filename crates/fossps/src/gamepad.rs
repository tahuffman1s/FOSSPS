//! Reads gamepads and feeds them into the input state.

use fossps_input::{GamepadButton, Input, Rumble, Stick, StickAxis};
use gilrs::ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Repeat, Replay, Ticks};
use gilrs::{Axis, EventType, GamepadId, Gilrs, GilrsBuilder};

use crate::translate;

/// How far a d-pad reported as an axis must be pushed to count as pressed.
const DPAD_AXIS_PRESS: f32 = 0.5;

/// How hard an idle gamepad must be pushed to take over from the active one.
///
/// Stick drift and resting-trigger noise stay well below this, so a gamepad
/// lying on the desk cannot steal control.
const TAKE_OVER_AT: f32 = 0.5;

/// How long a rumble's motors wait before they would start again, in
/// milliseconds: far longer than any rumble lasts, so they never do.
const NEVER_AGAIN_MS: u32 = 60_000;

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
    /// The rumble that was last started. Dropping it stops it, and it stops
    /// by itself when its time is up.
    rumble: Option<Effect>,
    /// Whether a rumble that could not be played has been reported.
    rumble_failed: bool,
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
            rumble: None,
            rumble_failed: false,
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
        let held = self.active;
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
        // A rumble belongs to the gamepad it started on: it does not carry
        // on in one that was put down or unplugged.
        if self.active != held {
            self.rumble = None;
        }
    }

    /// Shakes the gamepad being played with, in place of the rumble before.
    ///
    /// Nothing happens when no gamepad is in use or the one in use cannot
    /// rumble.
    pub(crate) fn rumble(&mut self, rumble: Rumble) {
        // Dropping the rumble before stops it.
        self.rumble = None;
        let (Some(gilrs), Some(id)) = (&mut self.gilrs, self.active) else {
            return;
        };
        let Some((strong, weak, time)) = motor_levels(rumble) else {
            return;
        };
        // The library plays a motor for `play_for`, waits `with_delay` and
        // starts over. A wait longer than any rumble makes it play once, and
        // the whole rumble is ended on the step the motors go quiet; ending
        // it by `Repeat::For` alone would play two steps (100 ms) too long.
        let motor = |kind| BaseEffect {
            kind,
            scheduling: Replay {
                play_for: time,
                with_delay: Ticks::from_ms(NEVER_AGAIN_MS),
                ..Replay::default()
            },
            ..BaseEffect::default()
        };
        let started = EffectBuilder::new()
            .add_effect(motor(BaseEffectType::Strong { magnitude: strong }))
            .add_effect(motor(BaseEffectType::Weak { magnitude: weak }))
            .repeat(Repeat::For(time - Ticks::from_ms(1)))
            .gamepads(&[id])
            .finish(gilrs)
            .and_then(|effect| effect.play().map(|()| effect));
        match started {
            Ok(effect) => self.rumble = Some(effect),
            // Plenty of gamepads have no motors, and one can be unplugged at
            // any moment: neither is worth a word.
            Err(gilrs::ff::Error::FfNotSupported(_) | gilrs::ff::Error::Disconnected(_)) => {}
            Err(error) => {
                if !self.rumble_failed {
                    self.rumble_failed = true;
                    log::warn!(
                        "The gamepad could not be made to rumble: {error} The game carries on \
                         without rumble."
                    );
                }
            }
        }
    }

    /// Stops the rumble that is playing, if one is.
    pub(crate) fn stop_rumble(&mut self) {
        self.rumble = None;
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

/// What the gamepad library is told for `rumble`: the strength of the strong
/// and the weak motor out of `u16::MAX`, and how long to play. `None` when
/// there is nothing to feel.
fn motor_levels(rumble: Rumble) -> Option<(u16, u16, Ticks)> {
    if rumble.is_still() {
        return None;
    }
    // Strengths are 0 to 1 and the time is at most a few seconds, so both
    // products fit: the casts cannot cut anything off.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let level = |strength: f32| (strength * f32::from(u16::MAX)).round() as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let milliseconds = (rumble.seconds() * 1000.0).ceil() as u32;
    // The library counts time in steps of 50 ms and rounds up to the next,
    // so there is always at least one step.
    Some((
        level(rumble.strong()),
        level(rumble.weak()),
        Ticks::from_ms(milliseconds.max(1)),
    ))
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
    fn rumble_strengths_become_motor_levels() {
        let (strong, weak, time) = motor_levels(Rumble::new(1.0, 0.5, 0.2)).unwrap();
        assert_eq!((strong, weak), (u16::MAX, 32768));
        assert_eq!(time, Ticks::from_ms(200));
        // Out-of-range numbers were already brought into range.
        let (strong, weak, time) = motor_levels(Rumble::new(9.0, 0.0, 99.0)).unwrap();
        assert_eq!((strong, weak), (u16::MAX, 0));
        assert_eq!(time, Ticks::from_ms(5000));
        // A rumble cannot outlast the wait that keeps it from starting again.
        assert!(Ticks::from_ms(NEVER_AGAIN_MS) > time);
    }

    #[test]
    fn a_very_short_rumble_still_plays_and_a_still_one_does_not() {
        let (_, _, time) = motor_levels(Rumble::new(0.0, 1.0, 0.001)).unwrap();
        assert_eq!(time, Ticks::from_ms(50));
        assert_eq!(motor_levels(Rumble::STOP), None);
        assert_eq!(motor_levels(Rumble::new(1.0, 1.0, 0.0)), None);
    }

    #[test]
    fn rumble_without_a_gamepad_does_nothing() {
        // No gamepad has been used, whether or not one is plugged in.
        let mut gamepads = Gamepads::new();
        gamepads.rumble(Rumble::new(1.0, 1.0, 0.1));
        assert!(gamepads.rumble.is_none());
        gamepads.stop_rumble();
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
