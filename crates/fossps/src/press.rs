//! Presses every action with no player there, for `fossps check`.
//!
//! A script's `if game.input.justPressed("jump")` is only run when jump is
//! pressed, so a mistake inside it stays hidden while nothing is. This
//! presses, holds and lets go of everything each action is bound to, one
//! action at a time, through the same [`Input`] a window feeds.

use fossps_input::{
    ActionId, ActionMap, Binding, Button, Input, InputFrame, Stick, StickAxis as Axis,
};

/// Every way a stick is pushed, all the way round: up first, then clockwise.
/// Each is `[right, up]`.
const ROUND: [[f32; 2]; 8] = [
    [0.0, 1.0],
    [1.0, 1.0],
    [1.0, 0.0],
    [1.0, -1.0],
    [0.0, -1.0],
    [-1.0, -1.0],
    [-1.0, 0.0],
    [-1.0, 1.0],
];

/// The mouse movements tried, in counts: a nudge, then a fast flick the
/// other way.
const MOUSE: [[f32; 2]; 2] = [[3.0, 1.0], [-240.0, -90.0]];

/// What [`every_action`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pressed {
    /// How many actions were pressed.
    pub actions: usize,
    /// The actions with nothing bound, which could not be pressed.
    pub unbound: Vec<String>,
}

/// Presses, holds and lets go of every action in `input`'s map, one at a
/// time and in the map's order, and calls `tick` with the action and the
/// input of each tick that passes meanwhile.
///
/// Everything an action is bound to is used in turn, so it is pressed once
/// from the keyboard and mouse and once from the gamepad. A button is seen
/// going down, then held, then let go. A stick is pushed all the way round
/// through its eight directions, direction keys likewise, and the mouse is
/// moved a little and then a lot. Between two actions one tick passes with
/// nothing pressed. An action in another action set is pressed with its set
/// live; the set that was live before is live again afterwards.
///
/// Call it with every device at rest.
pub fn every_action(input: &mut Input, mut tick: impl FnMut(ActionId, &InputFrame)) -> Pressed {
    // Its own copy, so the map can be read while the input is driven.
    let map: ActionMap = input.map().clone();
    let live = input.active_set();
    let mut pressed = Pressed::default();
    for action in map.actions() {
        let bindings = map.bindings(action);
        if bindings.is_empty() {
            pressed.unbound.push(map.name(action).to_owned());
            continue;
        }
        if let Some(set) = map.set_of(action) {
            input.set_active_set(set);
        }
        let mut pass = |input: &mut Input| tick(action, input.tick());
        for binding in bindings {
            use_binding(input, *binding, &mut pass);
        }
        // Nothing is down: the action is seen at rest before the next one.
        pass(input);
        pressed.actions += 1;
    }
    input.set_active_set(live);
    pressed
}

/// Works one binding through everything it can report and leaves it at rest.
/// `pass` lets one tick go by.
fn use_binding(input: &mut Input, binding: Binding, pass: &mut impl FnMut(&mut Input)) {
    match binding {
        Binding::Button(button) => {
            hold(input, &[button], pass);
            // The tick that sees it let go.
            pass(input);
        }
        Binding::Axis { negative, positive } => {
            hold(input, &[positive], pass);
            hold(input, &[negative], pass);
            pass(input);
        }
        Binding::StickAxis(stick, axis) => {
            for value in [1.0, -1.0] {
                input.set_stick(stick, axis, value);
                pass(input);
                pass(input);
            }
            input.set_stick(stick, axis, 0.0);
            pass(input);
        }
        Binding::Directions {
            up,
            down,
            left,
            right,
        } => {
            for [x, y] in ROUND {
                let across = if x > 0.0 { Some(right) } else { None };
                let across = if x < 0.0 { Some(left) } else { across };
                let along = if y > 0.0 { Some(up) } else { None };
                let along = if y < 0.0 { Some(down) } else { along };
                for button in [across, along].into_iter().flatten() {
                    input.press(button);
                }
                pass(input);
                for button in [across, along].into_iter().flatten() {
                    input.release(button);
                }
            }
            pass(input);
        }
        Binding::Stick(stick) => {
            for [x, y] in ROUND {
                push(input, stick, [x, y]);
                pass(input);
            }
            push(input, stick, [0.0, 0.0]);
            pass(input);
        }
        Binding::MouseMotion => {
            for [right, up] in MOUSE {
                input.move_mouse(right, up);
                pass(input);
            }
            // The tick that sees the mouse still again.
            pass(input);
        }
    }
}

/// Presses `buttons`, lets two ticks pass (one sees them go down, one sees
/// them held) and lets them go.
fn hold(input: &mut Input, buttons: &[Button], pass: &mut impl FnMut(&mut Input)) {
    for button in buttons {
        input.press(*button);
    }
    pass(input);
    pass(input);
    for button in buttons {
        input.release(*button);
    }
}

/// Pushes `stick` to `[right, up]`.
fn push(input: &mut Input, stick: Stick, [right, up]: [f32; 2]) {
    input.set_stick(stick, Axis::X, right);
    input.set_stick(stick, Axis::Y, up);
}

#[cfg(test)]
mod tests {
    use fossps_input::{ActionState, MouseMode};

    use super::*;

    /// What `name` reported on each tick it was the action being pressed.
    fn seen(map: ActionMap, name: &str) -> Vec<ActionState> {
        let wanted = map.action(name).unwrap();
        let mut input = Input::new(map);
        let mut states = Vec::new();
        every_action(&mut input, |action, frame| {
            if action == wanted {
                states.push(frame.get(wanted));
            }
        });
        states
    }

    #[test]
    fn a_button_goes_down_is_held_and_comes_up_on_each_device() {
        let mut map = ActionMap::new();
        map.button("jump", ["Space", "GamepadSouth"]).unwrap();
        let states = seen(map, "jump");

        let downs = states.iter().filter(|state| state.just_pressed).count();
        let ups = states.iter().filter(|state| state.just_released).count();
        let held = (states.iter())
            .filter(|state| state.pressed && !state.just_pressed)
            .count();
        assert_eq!((downs, held, ups), (2, 2, 2), "{states:?}");
        assert_eq!(states.last(), Some(&ActionState::default()));
    }

    #[test]
    fn a_wheel_notch_and_a_trigger_are_pressed_too() {
        let mut map = ActionMap::new();
        map.button("next", ["MouseWheelDown"]).unwrap();
        map.button("fire", ["RightTrigger"]).unwrap();
        for name in ["next", "fire"] {
            let states = seen(map.clone(), name);
            assert!(states.iter().any(|state| state.just_pressed), "{name}");
            assert!(states.iter().any(|state| state.just_released), "{name}");
        }
    }

    #[test]
    fn a_one_way_axis_goes_both_ways_from_keys_and_from_a_stick() {
        let mut map = ActionMap::new();
        map.axis1d("lean", ["Q/E", "RightStickX"]).unwrap();
        let states = seen(map, "lean");

        let right = states.iter().filter(|state| state.axis[0] > 0.9).count();
        let left = states.iter().filter(|state| state.axis[0] < -0.9).count();
        assert_eq!((right, left), (4, 4), "{states:?}");
        assert_eq!(states.last(), Some(&ActionState::default()));
    }

    #[test]
    fn a_stick_and_direction_keys_go_all_the_way_round() {
        for binding in ["WASD", "LeftStick"] {
            let mut map = ActionMap::new();
            map.axis2d("move", [binding]).unwrap();
            let states = seen(map, "move");

            // Each of the eight directions, as the signs of x and y.
            #[allow(clippy::cast_possible_truncation)]
            let mut ways: Vec<[i8; 2]> = (states.iter())
                .filter(|state| state.pressed)
                .map(|state| state.axis.map(|part| (part * 1.3).round() as i8))
                .collect();
            assert_eq!(ways.len(), 8, "{binding}: {states:?}");
            ways.sort_unstable();
            ways.dedup();
            assert_eq!(ways.len(), 8, "{binding}: {states:?}");
            assert_eq!(states.last(), Some(&ActionState::default()));
        }
    }

    #[test]
    fn the_mouse_is_moved_and_then_still() {
        let mut map = ActionMap::new();
        map.axis2d("look", ["MouseMotion"]).unwrap();
        let states = seen(map, "look");

        let moves: Vec<[f32; 2]> = states.iter().map(|state| state.delta).collect();
        assert_eq!(moves[..2], MOUSE, "{states:?}");
        assert_eq!(moves.last(), Some(&[0.0, 0.0]));
    }

    #[test]
    fn actions_are_pressed_one_at_a_time_in_the_maps_order() {
        let map = ActionMap::default_fps();
        let names: Vec<String> = (map.actions())
            .map(|action| map.name(action).to_owned())
            .collect();
        let mut input = Input::new(map.clone());
        let mut order: Vec<String> = Vec::new();
        let pressed = every_action(&mut input, |action, frame| {
            for other in map.actions().filter(|other| *other != action) {
                assert!(
                    !frame.pressed(other),
                    "{} while pressing {}",
                    map.name(other),
                    map.name(action)
                );
            }
            if order.last().map(String::as_str) != Some(map.name(action)) {
                order.push(map.name(action).to_owned());
            }
        });

        assert_eq!(order, names);
        assert_eq!(pressed.actions, names.len());
        assert_eq!(pressed.unbound, Vec::<String>::new());
        // Everything is at rest afterwards.
        let frame = input.tick();
        assert!(map.actions().all(|action| !frame.pressed(action)));
    }

    #[test]
    fn an_action_in_another_set_is_pressed_and_the_live_set_is_put_back() {
        let mut map = ActionMap::new();
        map.button("pause", ["Escape"]).unwrap();
        map.add_set("menu", MouseMode::Pointer).unwrap();
        map.button("resume", ["Escape"]).unwrap();
        let states = seen(map.clone(), "resume");
        assert!(states.iter().any(|state| state.just_pressed), "{states:?}");

        let mut input = Input::new(map);
        every_action(&mut input, |_, _| {});
        assert_eq!(input.active_set(), ActionMap::GAMEPLAY);
    }

    #[test]
    fn an_action_with_nothing_bound_is_named_and_not_counted() {
        let mut map = ActionMap::new();
        let jump = map.button("jump", ["Space"]).unwrap();
        map.button("fire", ["MouseLeft"]).unwrap();
        map.set_bindings(jump, Vec::<Binding>::new()).unwrap();
        let mut input = Input::new(map);

        let pressed = every_action(&mut input, |_, _| {});
        assert_eq!(pressed.actions, 1);
        assert_eq!(pressed.unbound, ["jump"]);
    }
}
