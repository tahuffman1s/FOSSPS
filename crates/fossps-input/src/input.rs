//! Collects device events and turns them into one [`InputFrame`] per tick.

use std::fmt;

use crate::binding::{ActionKind, Binding, Button};
use crate::devices::{GamepadButton, Key, Stick, StickAxis};
use crate::frame::{ActionState, InputFrame};
use crate::map::{ActionId, ActionMap, Conflict};
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
    /// The action waiting for a new key or button, and the one it replaces.
    listening: Option<(ActionId, Button)>,
    /// The last finished rebind, until it is taken.
    rebound: Option<Rebound>,
    /// Buttons pressed while listening; each is ignored until it comes back up.
    swallowed: [bool; Button::COUNT],
}

/// A finished rebind: what [`Input::start_rebind`] heard and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebound {
    /// The action that was rebound.
    pub action: ActionId,
    /// The key or button it no longer uses.
    pub from: Button,
    /// The key or button the player pressed, which it uses now.
    pub to: Button,
    /// The clashes this made: other actions that already used the new key or
    /// button. The rebind was made anyway; each prints as a sentence to show
    /// the player.
    pub conflicts: Vec<Conflict>,
}

/// Why [`Input::start_rebind`] did not start listening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebindError {
    /// The action's name, or `""` for an id from a different map.
    pub action: String,
    /// The key or button that was to be replaced.
    pub replacing: Button,
    /// What the action is bound to, as written in a bindings file.
    pub bound_to: Vec<String>,
}

impl fmt::Display for RebindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.action.is_empty() {
            return f.write_str(
                "That action is not in this action map. \
                 Look it up again with `map.action(\"name\")` after changing the map.",
            );
        }
        write!(
            f,
            "Action \"{}\" does not use \"{}\", so there is nothing to replace. ",
            self.action, self.replacing
        )?;
        if self.bound_to.is_empty() {
            f.write_str("It has nothing bound; give it bindings with `set_bindings`.")
        } else {
            write!(f, "It is bound to: {}.", self.bound_to.join(", "))
        }
    }
}

impl std::error::Error for RebindError {}

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
            listening: None,
            rebound: None,
            swallowed: [false; Button::COUNT],
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
        self.listening = None;
        self.rebound = None;
    }

    /// Starts listening: the next key or button the player presses replaces
    /// `replacing` in the action's bindings.
    ///
    /// Keep feeding events as usual. While listening, presses reach no action.
    /// The first new press of the same kind of device as `replacing` is taken:
    /// a key, mouse button or wheel notch for a keyboard or mouse binding, a
    /// gamepad button or trigger for a gamepad one, so an action never loses
    /// its binding on either device. Presses on the other device are dropped.
    /// Nothing pressed while listening, the chosen key included, reaches an
    /// action until it is let go. Choosing a key the action already uses
    /// elsewhere swaps the two.
    ///
    /// `Escape` (for keyboard and mouse) or `GamepadStart` (for a gamepad)
    /// cancels instead, so neither can be chosen this way; bind them in the
    /// bindings file. [`cancel_rebind`](Self::cancel_rebind) cancels too, for
    /// a time limit or a button on screen.
    ///
    /// Check [`is_rebinding`](Self::is_rebinding) each frame; once it is
    /// false, [`take_rebound`](Self::take_rebound) says what happened.
    ///
    /// ```
    /// use fossps_input::{ActionMap, Input, Key};
    ///
    /// let mut input = Input::new(ActionMap::default_fps());
    /// let jump = input.map().action("jump")?;
    ///
    /// input.start_rebind(jump, Key::Space.into())?;
    /// input.press(Key::F); // the player presses the key they want
    ///
    /// let rebound = input.take_rebound().ok_or("still listening")?;
    /// assert_eq!(rebound.to, Key::F.into());
    /// assert!(rebound.conflicts.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Fails if the action does not use `replacing`; the error lists what it
    /// is bound to.
    pub fn start_rebind(&mut self, action: ActionId, replacing: Button) -> Result<(), RebindError> {
        // Swapping a button for itself changes nothing, so this only asks
        // whether the action uses it.
        let uses = Binding::buttons_of(self.map.bindings(action)).any(|button| button == replacing);
        if !uses {
            return Err(RebindError {
                action: self.map.name(action).to_owned(),
                replacing,
                bound_to: (self.map.bindings(action).iter())
                    .map(ToString::to_string)
                    .collect(),
            });
        }
        self.listening = Some((action, replacing));
        self.rebound = None;
        Ok(())
    }

    /// Stops listening without changing any binding.
    pub fn cancel_rebind(&mut self) {
        self.listening = None;
    }

    /// Whether a rebind is waiting for the player to press something.
    #[must_use]
    pub fn is_rebinding(&self) -> bool {
        self.listening.is_some()
    }

    /// The rebind that just finished, once. `None` while still listening,
    /// and `None` after listening ended if it was cancelled.
    pub fn take_rebound(&mut self) -> Option<Rebound> {
        self.rebound.take()
    }

    /// Decides whether a fresh press of `button` belongs to a rebind instead
    /// of to the game. Returns `true` when the press must reach no action.
    fn intercept(&mut self, button: Button) -> bool {
        let index = button.index();
        if self.swallowed[index] {
            return true;
        }
        let Some((action, from)) = self.listening else {
            return false;
        };
        let wheel = matches!(button, Button::Wheel(_));
        if !wheel && self.held[index] {
            // Key repeat of something held since before listening began.
            return true;
        }
        // Whatever goes down while listening belongs to the rebind, not the
        // game, until it comes back up. A wheel notch never comes back up.
        self.swallowed[index] = !wheel;
        if button.is_gamepad() != from.is_gamepad() {
            return true;
        }
        self.listening = None;
        let cancel = if from.is_gamepad() {
            Button::Gamepad(GamepadButton::GamepadStart)
        } else {
            Button::Key(Key::Escape)
        };
        if button != cancel {
            let before = self.map.conflicts();
            self.map.replace_button(action, from, button);
            let mut conflicts = self.map.conflicts();
            conflicts.retain(|conflict| !before.contains(conflict));
            self.rebound = Some(Rebound {
                action,
                from,
                to: button,
                conflicts,
            });
        }
        true
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
        if self.intercept(button) {
            return;
        }
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
        self.swallowed[button.index()] = false;
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
        if self.swallowed[index] {
            if amount < RELEASE_AT {
                self.swallowed[index] = false;
            }
            return;
        }
        if !self.held[index] && amount >= PRESS_AT && self.intercept(Button::Gamepad(button)) {
            return;
        }
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
    ///
    /// A rebind that is listening is cancelled: the player has looked away.
    pub fn release_all(&mut self) {
        self.listening = None;
        self.swallowed = [false; Button::COUNT];
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
            self.swallowed[index] = false;
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
    use crate::devices::{MouseButton, MouseWheel};

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

    #[test]
    fn the_next_key_pressed_replaces_the_old_one_and_does_nothing_else() {
        let mut r = rig();
        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();
        assert!(r.input.is_rebinding());
        assert_eq!(r.input.take_rebound(), None);

        r.input.press(Key::F);
        assert!(!r.input.is_rebinding());
        let rebound = r.input.take_rebound().unwrap();
        assert_eq!(
            (rebound.action, rebound.from, rebound.to),
            (r.jump, Key::Space.into(), Key::F.into())
        );
        assert_eq!(rebound.conflicts, Vec::new());
        assert_eq!(r.input.take_rebound(), None, "handed over once");

        // The press that chose the key does not jump, however long it repeats.
        r.input.press(Key::F);
        assert!(!r.input.tick().pressed(r.jump));
        r.input.release(Key::F);
        r.input.tick();

        r.input.press(Key::F);
        assert!(r.input.tick().just_pressed(r.jump));
        r.input.release(Key::F);
        r.input.press(Key::Space);
        assert!(!r.input.tick().pressed(r.jump), "the old key is gone");
        // The gamepad binding is untouched.
        r.input.press(GamepadButton::GamepadSouth);
        assert!(r.input.tick().pressed(r.jump));
    }

    #[test]
    fn listening_hears_only_new_presses_on_the_same_kind_of_device() {
        let mut r = rig();
        r.input.press(MouseButton::MouseLeft);
        r.input.tick();
        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();

        // The click that opened the listener is still down, and repeats.
        r.input.press(MouseButton::MouseLeft);
        // A gamepad cannot fill a keyboard slot.
        r.input.press(GamepadButton::GamepadEast);
        r.input.set_button_amount(GamepadButton::LeftTrigger, 1.0);
        assert!(r.input.is_rebinding());
        assert!(!r.input.tick().pressed(r.jump), "dropped, not played");

        r.input.press(MouseWheel::MouseWheelUp);
        let rebound = r.input.take_rebound().unwrap();
        assert_eq!(rebound.to, MouseWheel::MouseWheelUp.into());
        r.input.tick();
        r.input.press(MouseWheel::MouseWheelUp);
        assert!(r.input.tick().just_pressed(r.jump));
    }

    #[test]
    fn a_trigger_can_be_chosen_for_a_gamepad_binding() {
        let mut r = rig();
        r.input
            .start_rebind(r.jump, GamepadButton::GamepadSouth.into())
            .unwrap();
        r.input.press(Key::F);
        r.input.set_button_amount(GamepadButton::LeftTrigger, 0.2);
        assert!(r.input.is_rebinding(), "a resting trigger is not a press");

        r.input.set_button_amount(GamepadButton::LeftTrigger, 0.9);
        let rebound = r.input.take_rebound().unwrap();
        assert_eq!(rebound.to, GamepadButton::LeftTrigger.into());
        r.input.set_button_amount(GamepadButton::LeftTrigger, 1.0);
        assert!(!r.input.tick().pressed(r.jump), "still the choosing press");

        r.input.set_button_amount(GamepadButton::LeftTrigger, 0.0);
        r.input.set_button_amount(GamepadButton::LeftTrigger, 0.8);
        let frame = r.input.tick();
        assert!(frame.just_pressed(r.jump));
        assert!(close(frame.value(r.jump), 0.8));
    }

    #[test]
    fn a_rebind_onto_a_used_key_goes_through_and_names_the_clash() {
        let mut r = rig();
        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();
        r.input.press(Key::W);
        let rebound = r.input.take_rebound().unwrap();
        let said: Vec<String> = rebound.conflicts.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "\"W\" is bound to both \"jump\" and \"move\", so it does both at once. \
              Bind one of them to something else, unless that is what you want."
            ]
        );
        assert_eq!(
            r.input.map().bindings(r.jump)[0],
            Binding::Button(Key::W.into())
        );
    }

    #[test]
    fn one_key_of_a_group_can_be_rebound() {
        let mut r = rig();
        r.input.start_rebind(r.movement, Key::W.into()).unwrap();
        r.input.press(Key::ArrowUp);
        r.input.release(Key::ArrowUp);
        r.input.tick();
        r.input.press(Key::ArrowUp);
        assert_eq!(r.input.tick().axis2d(r.movement), [0.0, 1.0]);
    }

    #[test]
    fn escape_start_and_a_call_all_cancel() {
        let mut r = rig();
        let before = r.input.map().clone();

        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();
        r.input.press(Key::Escape);
        assert!(!r.input.is_rebinding());
        assert_eq!(r.input.take_rebound(), None);

        r.input
            .start_rebind(r.jump, GamepadButton::GamepadSouth.into())
            .unwrap();
        r.input.press(GamepadButton::GamepadStart);
        assert!(!r.input.is_rebinding());
        assert_eq!(r.input.take_rebound(), None);

        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();
        r.input.cancel_rebind();
        r.input.press(Key::F);
        assert_eq!(r.input.take_rebound(), None);

        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();
        r.input.release_all();
        assert!(!r.input.is_rebinding(), "the window lost focus");

        assert_eq!(r.input.map(), &before);
    }

    #[test]
    fn a_rebind_that_cannot_start_says_what_the_action_uses() {
        let mut r = rig();
        let error = r.input.start_rebind(r.jump, Key::F.into()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Action \"jump\" does not use \"F\", so there is nothing to replace. \
             It is bound to: Space, GamepadSouth."
        );
        assert!(!r.input.is_rebinding());

        let foreign = Input::new(ActionMap::default_fps())
            .map()
            .action("pause")
            .unwrap();
        let error = r.input.start_rebind(foreign, Key::F.into()).unwrap_err();
        assert!(error.to_string().starts_with("That action is not in"));
    }

    #[test]
    fn choosing_a_key_the_group_already_uses_swaps_the_two() {
        let mut r = rig();
        r.input.start_rebind(r.movement, Key::W.into()).unwrap();
        r.input.press(Key::A);
        let rebound = r.input.take_rebound().unwrap();
        assert_eq!(rebound.conflicts, Vec::new());
        assert_eq!(
            r.input.map().bindings(r.movement)[0],
            Binding::directions(Key::A, Key::S, Key::W, Key::D)
        );

        r.input.start_rebind(r.lean, Key::Q.into()).unwrap();
        r.input.press(Key::E);
        assert_eq!(
            r.input.map().bindings(r.lean)[0],
            Binding::Axis {
                negative: Key::E.into(),
                positive: Key::Q.into()
            }
        );
    }

    #[test]
    fn nothing_pressed_while_listening_plays_once_listening_ends() {
        let mut r = rig();
        r.input.start_rebind(r.jump, Key::Space.into()).unwrap();
        // The wrong device: both stay down past the end of listening.
        r.input.set_button_amount(GamepadButton::RightTrigger, 1.0);
        r.input.press(Key::F);
        assert!(r.input.take_rebound().is_some());

        // A second rebind ends while F, chosen by the first, is still down.
        r.input
            .start_rebind(r.fire, MouseButton::MouseLeft.into())
            .unwrap();
        r.input.press(Key::G);
        assert!(r.input.take_rebound().is_some());

        // Key repeat and further trigger samples of the same presses.
        r.input.press(Key::F);
        r.input.press(Key::G);
        r.input.set_button_amount(GamepadButton::RightTrigger, 0.9);
        let frame = r.input.tick();
        assert!(!frame.pressed(r.jump) && !frame.pressed(r.fire));

        r.input.set_button_amount(GamepadButton::RightTrigger, 0.0);
        r.input.set_button_amount(GamepadButton::RightTrigger, 1.0);
        assert!(r.input.tick().just_pressed(r.fire), "let go, then pulled");
    }
}
