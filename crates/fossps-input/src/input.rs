//! Collects device events and turns them into one [`InputFrame`] per tick.

use crate::binding::{ActionKind, Binding, Button};
use crate::devices::{GamepadButton, Stick, StickAxis};
use crate::frame::{ActionState, InputFrame};
use crate::map::ActionMap;
use crate::shaping::{PRESS_AT, RELEASE_AT, clamp_length, length, sanitize};

/// The live state of every device, resolved through an [`ActionMap`].
///
/// A window layer reports what the player does (`press`, `release`,
/// `move_mouse`, `set_stick`...) whenever events arrive. The simulation calls
/// [`tick`](Self::tick) once per fixed step to get an [`InputFrame`].
///
/// A press is never lost and never counted twice: a tap that starts and ends
/// between two ticks still shows up as pressed for exactly one tick, and when
/// one rendered frame runs several ticks, only the first sees the new presses
/// and mouse movement.
#[derive(Debug, Clone)]
pub struct Input {
    map: ActionMap,
    /// Buttons physically down right now.
    held: [bool; Button::COUNT],
    /// Buttons that went down since the last tick, even if already released.
    latched: [bool; Button::COUNT],
    /// What the last tick reported for each button.
    reported: [bool; Button::COUNT],
    /// Buttons pressed again this tick while the last tick already reported them.
    repressed: [bool; Button::COUNT],
    /// How far each gamepad button is pushed, 0 to 1.
    amount: [f32; GamepadButton::COUNT],
    /// Raw stick positions, before the dead zone.
    sticks: [[f32; 2]; Stick::COUNT],
    /// Mouse movement accumulated since the last tick.
    mouse: [f32; 2],
    frame: InputFrame,
}

impl Input {
    /// Starts with every device at rest.
    #[must_use]
    pub fn new(map: ActionMap) -> Self {
        let frame = InputFrame::idle(map.len());
        Self {
            map,
            held: [false; Button::COUNT],
            latched: [false; Button::COUNT],
            reported: [false; Button::COUNT],
            repressed: [false; Button::COUNT],
            amount: [0.0; GamepadButton::COUNT],
            sticks: [[0.0; 2]; Stick::COUNT],
            mouse: [0.0; 2],
            frame,
        }
    }

    /// The action map in use.
    #[must_use]
    pub fn map(&self) -> &ActionMap {
        &self.map
    }

    /// Replaces the action map. Ids from the old map must be looked up again.
    pub fn set_map(&mut self, map: ActionMap) {
        let tick = self.frame.tick;
        self.frame = InputFrame::idle(map.len());
        self.frame.tick = tick;
        self.map = map;
    }

    /// The frame produced by the most recent [`tick`](Self::tick).
    #[must_use]
    pub fn frame(&self) -> &InputFrame {
        &self.frame
    }

    /// Reports that a key or button went down. Key-repeat events are harmless.
    pub fn press(&mut self, button: impl Into<Button>) {
        let button = button.into();
        let index = button.index();
        match button {
            // A wheel notch has no "held" state: it is a press that is already over.
            Button::Wheel(_) => self.latched[index] = true,
            Button::Key(_) | Button::Mouse(_) | Button::Gamepad(_) => {
                if !self.held[index] {
                    self.latched[index] = true;
                }
                self.held[index] = true;
                if let Button::Gamepad(pad) = button {
                    self.amount[pad.index()] = 1.0;
                }
            }
        }
    }

    /// Reports that a key or button came up.
    pub fn release(&mut self, button: impl Into<Button>) {
        let button = button.into();
        self.held[button.index()] = false;
        if let Button::Gamepad(pad) = button {
            self.amount[pad.index()] = 0.0;
        }
    }

    /// Reports how far an analog gamepad button, such as a trigger, is pushed: 0 to 1.
    ///
    /// The button counts as pressed from half way, and as released once it
    /// falls back below 0.4, so a trigger held near the middle does not flicker.
    pub fn set_button_amount(&mut self, button: GamepadButton, amount: f32) {
        let amount = sanitize(amount).max(0.0);
        let index = Button::Gamepad(button).index();
        self.amount[button.index()] = amount;
        if self.held[index] {
            if amount < RELEASE_AT {
                self.held[index] = false;
            }
        } else if amount >= PRESS_AT {
            self.held[index] = true;
            self.latched[index] = true;
        }
    }

    /// Reports one axis of a thumbstick, from -1 to 1. Right and up are positive.
    pub fn set_stick(&mut self, stick: Stick, axis: StickAxis, value: f32) {
        let slot = match axis {
            StickAxis::X => 0,
            StickAxis::Y => 1,
        };
        self.sticks[stick.index()][slot] = sanitize(value);
    }

    /// Reports raw mouse movement in counts. Right and up are positive.
    pub fn move_mouse(&mut self, right: f32, up: f32) {
        if right.is_finite() && up.is_finite() {
            self.mouse[0] += right;
            self.mouse[1] += up;
        }
    }

    /// Lets go of everything, for example when the window loses focus.
    ///
    /// Held actions report `just_released` on the next tick. Presses and mouse
    /// movement that no tick has seen yet are discarded.
    pub fn release_all(&mut self) {
        self.held = [false; Button::COUNT];
        self.latched = [false; Button::COUNT];
        self.mouse = [0.0; 2];
        self.release_gamepad();
    }

    /// Lets go of every gamepad control, for example when the gamepad disconnects.
    pub fn release_gamepad(&mut self) {
        for button in GamepadButton::ALL {
            let index = Button::Gamepad(*button).index();
            self.held[index] = false;
            self.latched[index] = false;
        }
        self.amount = [0.0; GamepadButton::COUNT];
        self.sticks = [[0.0; 2]; Stick::COUNT];
    }

    /// Advances one simulation tick and returns the state of every action.
    ///
    /// This never allocates.
    pub fn tick(&mut self) -> &InputFrame {
        for index in 0..Button::COUNT {
            let now = self.held[index] || self.latched[index];
            self.repressed[index] = self.latched[index] && self.reported[index];
            self.reported[index] = now;
            self.latched[index] = false;
        }
        let mouse = std::mem::take(&mut self.mouse);

        let (actions, bindings) = self.map.parts();
        let deadzone = self.map.deadzone();
        for (action, state) in actions.iter().zip(&mut self.frame.actions) {
            let bindings = bindings.get(action.bindings.clone()).unwrap_or_default();
            let mut axis = [0.0_f32; 2];
            let mut delta = [0.0_f32; 2];
            let mut repressed = false;
            let amount = |button: Button| button_amount(&self.reported, &self.amount, button);

            for binding in bindings {
                let candidate = match *binding {
                    Binding::Button(button) => {
                        repressed |= self.repressed[button.index()];
                        [amount(button), 0.0]
                    }
                    Binding::Axis { negative, positive } => {
                        [amount(positive) - amount(negative), 0.0]
                    }
                    Binding::StickAxis(stick, stick_axis) => {
                        let shaped = deadzone.apply(self.sticks[stick.index()], action.stick_curve);
                        match stick_axis {
                            StickAxis::X => [shaped[0], 0.0],
                            StickAxis::Y => [shaped[1], 0.0],
                        }
                    }
                    Binding::Directions {
                        up,
                        down,
                        left,
                        right,
                    } => clamp_length([amount(right) - amount(left), amount(up) - amount(down)]),
                    Binding::Stick(stick) => {
                        deadzone.apply(self.sticks[stick.index()], action.stick_curve)
                    }
                    Binding::MouseMotion => {
                        delta = mouse;
                        [0.0, 0.0]
                    }
                };
                // When several bindings are active the strongest wins. Adding
                // them would make keyboard plus stick faster than either alone.
                if length(candidate) > length(axis) {
                    axis = candidate;
                }
            }
            if action.kind != ActionKind::Axis2D {
                axis[1] = 0.0;
            }

            let pressed = axis != [0.0, 0.0];
            *state = ActionState {
                pressed,
                just_pressed: pressed && (!state.pressed || repressed),
                just_released: !pressed && state.pressed,
                axis,
                delta,
            };
        }

        self.frame.tick += 1;
        &self.frame
    }
}

/// How far a button is pushed this tick: 0 when up, 1 for keys, 0 to 1 for triggers.
fn button_amount(
    reported: &[bool; Button::COUNT],
    amounts: &[f32; GamepadButton::COUNT],
    button: Button,
) -> f32 {
    if !reported[button.index()] {
        return 0.0;
    }
    match button {
        // A tap that already ended has no amount left, so it counts as a full press.
        Button::Gamepad(pad) if amounts[pad.index()] > 0.0 => amounts[pad.index()],
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{Key, MouseButton, MouseWheel};
    use crate::map::ActionId;

    struct Rig {
        input: Input,
        jump: ActionId,
        fire: ActionId,
        next: ActionId,
        lean: ActionId,
        movement: ActionId,
        look: ActionId,
    }

    fn rig() -> Rig {
        let mut map = ActionMap::new();
        let jump = map.button("jump", ["Space", "GamepadSouth"]).unwrap();
        let fire = map.button("fire", ["MouseLeft", "RightTrigger"]).unwrap();
        let next = map.button("next", ["MouseWheelDown"]).unwrap();
        let lean = map.axis1d("lean", ["Q/E", "RightStickX"]).unwrap();
        let movement = map.axis2d("move", ["WASD", "LeftStick"]).unwrap();
        let look = map.axis2d("look", ["MouseMotion", "RightStick"]).unwrap();
        Rig {
            input: Input::new(map),
            jump,
            fire,
            next,
            lean,
            movement,
            look,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn press_hold_release() {
        let mut r = rig();
        r.input.press(Key::Space);
        let frame = r.input.tick();
        assert!(frame.pressed(r.jump) && frame.just_pressed(r.jump));
        assert_eq!(frame.value(r.jump), 1.0);

        let frame = r.input.tick();
        assert!(frame.pressed(r.jump) && !frame.just_pressed(r.jump));

        r.input.release(Key::Space);
        let frame = r.input.tick();
        assert!(!frame.pressed(r.jump) && frame.just_released(r.jump));

        let frame = r.input.tick();
        assert_eq!(frame.get(r.jump), ActionState::default());
    }

    #[test]
    fn tap_shorter_than_a_tick_is_not_lost() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.release(Key::Space);
        let frame = r.input.tick();
        assert!(frame.pressed(r.jump) && frame.just_pressed(r.jump));
        let frame = r.input.tick();
        assert!(!frame.pressed(r.jump) && frame.just_released(r.jump));
    }

    #[test]
    fn several_ticks_in_one_frame_see_a_press_once() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.move_mouse(4.0, 2.0);
        let presses: Vec<bool> = (0..3)
            .map(|_| r.input.tick().just_pressed(r.jump))
            .collect();
        assert_eq!(presses, [true, false, false]);

        r.input.move_mouse(4.0, 2.0);
        assert_eq!(r.input.tick().delta(r.look), [4.0, 2.0]);
        assert_eq!(r.input.tick().delta(r.look), [0.0, 0.0]);
    }

    #[test]
    fn key_repeat_does_not_press_again() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.tick();
        r.input.press(Key::Space);
        assert!(!r.input.tick().just_pressed(r.jump));
    }

    #[test]
    fn release_and_press_again_within_a_tick_is_a_new_press() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.tick();
        r.input.release(Key::Space);
        r.input.press(Key::Space);
        let frame = r.input.tick();
        assert!(frame.pressed(r.jump) && frame.just_pressed(r.jump));
        assert!(!frame.just_released(r.jump));
    }

    #[test]
    fn second_binding_does_not_press_a_held_action_again() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.tick();
        r.input.press(GamepadButton::GamepadSouth);
        assert!(!r.input.tick().just_pressed(r.jump));
        r.input.release(Key::Space);
        let frame = r.input.tick();
        assert!(frame.pressed(r.jump) && !frame.just_released(r.jump));
    }

    #[test]
    fn every_wheel_notch_tick_is_a_press() {
        let mut r = rig();
        r.input.press(MouseWheel::MouseWheelDown);
        assert!(r.input.tick().just_pressed(r.next));
        r.input.press(MouseWheel::MouseWheelDown);
        assert!(r.input.tick().just_pressed(r.next));
        let frame = r.input.tick();
        assert!(frame.just_released(r.next) && !frame.pressed(r.next));
    }

    #[test]
    fn trigger_presses_half_way_and_does_not_flicker() {
        let mut r = rig();
        r.input.set_button_amount(GamepadButton::RightTrigger, 0.45);
        assert!(!r.input.tick().pressed(r.fire));

        r.input.set_button_amount(GamepadButton::RightTrigger, 0.6);
        let frame = r.input.tick();
        assert!(frame.just_pressed(r.fire));
        assert!(close(frame.value(r.fire), 0.6));

        r.input.set_button_amount(GamepadButton::RightTrigger, 0.45);
        let frame = r.input.tick();
        assert!(frame.pressed(r.fire) && !frame.just_pressed(r.fire));

        r.input.set_button_amount(GamepadButton::RightTrigger, 0.3);
        assert!(r.input.tick().just_released(r.fire));
    }

    #[test]
    fn trigger_tap_between_ticks_counts_as_a_full_press() {
        let mut r = rig();
        r.input.set_button_amount(GamepadButton::RightTrigger, 1.0);
        r.input.set_button_amount(GamepadButton::RightTrigger, 0.0);
        let frame = r.input.tick();
        assert!(frame.just_pressed(r.fire));
        assert_eq!(frame.value(r.fire), 1.0);
    }

    #[test]
    fn keys_form_a_direction_and_diagonals_are_not_faster() {
        let mut r = rig();
        r.input.press(Key::W);
        assert_eq!(r.input.tick().axis2d(r.movement), [0.0, 1.0]);
        r.input.press(Key::D);
        let diagonal = r.input.tick().axis2d(r.movement);
        assert!(close(length(diagonal), 1.0));
        assert!(close(diagonal[0], diagonal[1]));
        r.input.press(Key::S);
        assert_eq!(r.input.tick().axis2d(r.movement), [1.0, 0.0]);
    }

    #[test]
    fn stick_and_keys_do_not_add_up() {
        let mut r = rig();
        r.input.set_stick(Stick::LeftStick, StickAxis::Y, 1.0);
        r.input.press(Key::W);
        assert!(close(r.input.tick().axis2d(r.movement)[1], 1.0));

        r.input.release(Key::W);
        r.input.set_stick(Stick::LeftStick, StickAxis::Y, 0.55);
        let walk = r.input.tick().axis2d(r.movement)[1];
        assert!(close(walk, 0.5), "analog walk speed is kept, got {walk}");
    }

    #[test]
    fn stick_dead_zone_applies_to_actions() {
        let mut r = rig();
        r.input.set_stick(Stick::LeftStick, StickAxis::X, 0.1);
        let frame = r.input.tick();
        assert_eq!(frame.axis2d(r.movement), [0.0, 0.0]);
        assert!(!frame.pressed(r.movement));
    }

    #[test]
    fn one_dimensional_axes() {
        let mut r = rig();
        r.input.press(Key::Q);
        assert_eq!(r.input.tick().value(r.lean), -1.0);
        r.input.press(Key::E);
        assert_eq!(r.input.tick().value(r.lean), 0.0);
        r.input.release(Key::Q);
        r.input.release(Key::E);
        r.input.set_stick(Stick::RightStick, StickAxis::X, 1.0);
        r.input.set_stick(Stick::RightStick, StickAxis::Y, 1.0);
        let frame = r.input.tick();
        assert!(frame.value(r.lean) > 0.5);
        assert_eq!(frame.get(r.lean).axis[1], 0.0);
    }

    #[test]
    fn look_keeps_mouse_counts_and_stick_rate_apart() {
        let mut r = rig();
        r.input.move_mouse(3.0, -1.0);
        r.input.move_mouse(2.0, 0.5);
        r.input.set_stick(Stick::RightStick, StickAxis::X, -1.0);
        let frame = r.input.tick();
        assert_eq!(frame.delta(r.look), [5.0, -0.5]);
        assert!(close(frame.axis2d(r.look)[0], -1.0));
        assert_eq!(frame.delta(r.movement), [0.0, 0.0]);
    }

    #[test]
    fn stick_curve_is_per_action() {
        let mut map = ActionMap::new();
        let movement = map.axis2d("move", ["RightStick"]).unwrap();
        let look = map.axis2d("look", ["RightStick"]).unwrap();
        map.set_stick_curve(look, 2.0);
        let mut input = Input::new(map);
        input.set_stick(Stick::RightStick, StickAxis::X, 0.55);
        let frame = input.tick();
        assert!(close(frame.axis2d(movement)[0], 0.5));
        assert!(close(frame.axis2d(look)[0], 0.25));
    }

    #[test]
    fn losing_focus_releases_everything_and_drops_unseen_input() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.set_stick(Stick::LeftStick, StickAxis::X, 1.0);
        r.input.tick();
        r.input.press(MouseButton::MouseLeft);
        r.input.move_mouse(9.0, 9.0);
        r.input.release_all();
        let frame = r.input.tick();
        assert!(frame.just_released(r.jump));
        assert!(frame.just_released(r.movement));
        assert!(!frame.pressed(r.fire));
        assert_eq!(frame.delta(r.look), [0.0, 0.0]);
    }

    #[test]
    fn gamepad_disconnect_leaves_the_keyboard_alone() {
        let mut r = rig();
        r.input.press(Key::W);
        r.input.press(GamepadButton::GamepadSouth);
        r.input.set_button_amount(GamepadButton::RightTrigger, 1.0);
        r.input.tick();
        r.input.release_gamepad();
        let frame = r.input.tick();
        assert!(frame.just_released(r.jump) && frame.just_released(r.fire));
        assert_eq!(frame.axis2d(r.movement), [0.0, 1.0]);
    }

    #[test]
    fn bad_readings_are_ignored() {
        let mut r = rig();
        r.input.set_stick(Stick::LeftStick, StickAxis::X, f32::NAN);
        r.input
            .set_button_amount(GamepadButton::RightTrigger, f32::INFINITY);
        r.input.move_mouse(f32::NAN, 1.0);
        let frame = r.input.tick();
        assert_eq!(frame.axis2d(r.movement), [0.0, 0.0]);
        assert!(!frame.pressed(r.fire));
        assert_eq!(frame.delta(r.look), [0.0, 0.0]);
    }

    #[test]
    fn same_events_give_the_same_frames() {
        let run = || {
            let mut r = rig();
            let mut frames = Vec::new();
            for step in 0..20_u8 {
                if step % 3 == 0 {
                    r.input.press(Key::Space);
                }
                if step % 4 == 0 {
                    r.input.release(Key::Space);
                }
                r.input.move_mouse(f32::from(step), 1.5);
                r.input
                    .set_stick(Stick::LeftStick, StickAxis::X, f32::from(step) / 20.0);
                frames.push(r.input.tick().clone());
            }
            frames
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn ticks_are_numbered_from_one() {
        let mut r = rig();
        assert_eq!(r.input.frame().tick, 0);
        assert_eq!(r.input.tick().tick, 1);
        assert_eq!(r.input.tick().tick, 2);
    }

    #[test]
    fn replacing_the_map_resets_action_state() {
        let mut r = rig();
        r.input.press(Key::Space);
        r.input.tick();
        let mut map = ActionMap::new();
        let hop = map.button("hop", ["Space"]).unwrap();
        r.input.set_map(map);
        let frame = r.input.tick();
        assert_eq!(frame.tick, 2);
        assert!(frame.pressed(hop));
    }
}
