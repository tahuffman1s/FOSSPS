//! Edit mode: `fossps edit` opens a game's level to fly around in, over a
//! ground grid that shows scale, with a list of what is in the level on the
//! left and what is known about the chosen thing on the right.
//!
//! The camera flies as the viewports of other editors do. Hold the right
//! mouse button to look around; W, A, S and D fly where the camera faces, E
//! and Q go straight up and down, Shift flies faster and the mouse wheel
//! changes the speed. On a gamepad the sticks fly and look, the triggers go
//! down and up, pressing the left stick flies faster and the d-pad changes
//! the speed.
//!
//! The panels are for the mouse: a click chooses, a double click flies the
//! camera there. A game with no level of its own is shown the stand-in one
//! it is played on.

mod panels;

use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;

use fossps_input::{ActionId, ActionMap, ActionSetId, InputFrame, MouseMode};

use self::panels::LevelPanels;
pub(crate) use self::panels::context as panel_context;
use crate::app::{self, AppError, Config};
use crate::game::{Frame, Game};
use crate::online::{MOUSE_TURN, STICK_TURN};

/// How fast the camera flies to begin with, in metres a second: a little
/// faster than a sprinting player.
const START_SPEED: f32 = 8.0;
/// The slowest and fastest the wheel and d-pad can make it.
const SPEED_RANGE: [f32; 2] = [0.5, 64.0];
/// How much one wheel notch or d-pad press changes the speed.
const SPEED_STEP: f32 = 1.25;
/// How many times faster the camera flies while `fast` is held.
const FAST: f32 = 4.0;
/// Flying to something leaves the camera this far from its edge, in metres,
/// never further than this from its middle (a floor can be huge), and
/// looking down at least this steeply, in radians.
const FLY_TO_NEAREST: f32 = 3.0;
const FLY_TO_FURTHEST: f32 = 60.0;
const FLY_TO_TILT: f32 = 0.35;
/// The camera never looks quite straight up or down, where turning would
/// spin the picture.
const STEEPEST: f32 = FRAC_PI_2 - 0.01;

/// What the person at the controls asks of the camera in one tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Asked {
    /// Flying to the right and forwards, each from -1 to 1.
    fly: [f32; 2],
    /// Going straight up (positive) or down, from -1 to 1.
    rise: f32,
    /// Turning right and tilting up, in radians.
    turn: [f32; 2],
    /// Flying faster while held.
    fast: bool,
    /// How many steps faster (positive) or slower the speed is to become.
    speed_steps: i8,
}

/// The editor's camera: where it is, which way it faces and how fast it flies.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FlyCamera {
    /// Where the eye is, in metres.
    position: [f32; 3],
    /// Which way it faces, as [`Frame::look_from`] takes it. Not wrapped, so
    /// that two states can be blended.
    yaw: f32,
    /// How far it looks up (positive) or down, in radians.
    pitch: f32,
    /// How fast it flies, in metres a second.
    speed: f32,
}

impl Default for FlyCamera {
    /// Above and behind the middle of the level, looking down at it.
    fn default() -> Self {
        Self {
            position: [0.0, 5.0, 22.0],
            yaw: 0.0,
            pitch: -0.2,
            speed: START_SPEED,
        }
    }
}

impl FlyCamera {
    /// Moves the camera through one tick of `step` seconds.
    fn step(&mut self, asked: &Asked, step: f32) {
        let numbers = [
            asked.fly[0],
            asked.fly[1],
            asked.rise,
            asked.turn[0],
            asked.turn[1],
            step,
        ];
        if numbers.iter().any(|number| !number.is_finite()) {
            return;
        }
        for _ in 0..asked.speed_steps.unsigned_abs() {
            if asked.speed_steps > 0 {
                self.speed *= SPEED_STEP;
            } else {
                self.speed /= SPEED_STEP;
            }
        }
        self.speed = self.speed.clamp(SPEED_RANGE[0], SPEED_RANGE[1]);

        // Yaw grows turning left; the mouse and stick report right. It is
        // kept within a few turns of zero without ever jumping by less than
        // a whole turn, so the view does not notice.
        self.yaw -= asked.turn[0];
        if self.yaw.abs() > 4.0 * TAU {
            self.yaw = self.yaw.rem_euclid(TAU);
        }
        self.pitch = (self.pitch + asked.turn[1]).clamp(-STEEPEST, STEEPEST);

        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let right = [cos_yaw, 0.0, -sin_yaw];
        let mut heading =
            [0, 1, 2].map(|axis| right[axis] * asked.fly[0] + forward[axis] * asked.fly[1]);
        heading[1] += asked.rise;
        // Flying forwards and up at once is no faster than either alone,
        // and a stick pushed half way flies at half speed.
        let length = heading.iter().map(|part| part * part).sum::<f32>().sqrt();
        let scale = if length > 1.0 { 1.0 / length } else { 1.0 };
        let metres = self.speed * if asked.fast { FAST } else { 1.0 } * step * scale;
        for (place, part) in self.position.iter_mut().zip(heading) {
            *place += part * metres;
        }
    }

    /// Moves the camera to where it sees all of something `reach` metres
    /// across around `middle`, looking down at it a little if it was not
    /// looking down already.
    fn fly_to(&mut self, middle: [f32; 3], reach: f32) {
        if !(middle.iter().all(|part| part.is_finite()) && reach.is_finite()) {
            return;
        }
        self.pitch = self.pitch.min(-FLY_TO_TILT);
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let back = (reach * 2.0 + FLY_TO_NEAREST).min(FLY_TO_FURTHEST);
        self.position = [0, 1, 2].map(|axis| middle[axis] - forward[axis] * back);
    }

    /// The camera `alpha` of the way from `before` to this one.
    fn blended(&self, before: &Self, alpha: f32) -> Self {
        let blend = |from: f32, to: f32| from + (to - from) * alpha;
        // The short way round, in case one of the two was wrapped.
        let turned = (self.yaw - before.yaw + TAU / 2.0).rem_euclid(TAU) - TAU / 2.0;
        Self {
            position: [0, 1, 2].map(|axis| blend(before.position[axis], self.position[axis])),
            yaw: self.yaw - turned * (1.0 - alpha),
            pitch: blend(before.pitch, self.pitch),
            speed: self.speed,
        }
    }
}

/// The editor's actions, looked up once.
#[derive(Debug, Clone, Copy)]
struct Controls {
    /// The set they are all in, which leaves the cursor free.
    set: ActionSetId,
    fly: ActionId,
    look: ActionId,
    rise: ActionId,
    fast: ActionId,
    faster: ActionId,
    slower: ActionId,
}

/// The editor's own actions and what they are bound to: every one on
/// keyboard and mouse and on a gamepad.
///
/// They are kept apart from a game's actions in a set called `editor`,
/// which leaves the cursor free for pointing; the window captures the mouse
/// while the right mouse button is held.
fn controls() -> Option<(ActionMap, Controls)> {
    let mut map = ActionMap::new();
    let set = map.add_set("editor", MouseMode::Pointer).ok()?;
    let fly = map.axis2d("fly", ["WASD", "LeftStick"]).ok()?;
    let look = map.axis2d("look", ["MouseMotion", "RightStick"]).ok()?;
    map.set_stick_curve(look, 2.0);
    let rise = map
        .axis1d("rise", ["Q/E", "LeftTrigger/RightTrigger"])
        .ok()?;
    let fast = map.button("fast", ["LeftShift", "LeftStickPress"]).ok()?;
    let faster = map.button("faster", ["MouseWheelUp", "DpadUp"]).ok()?;
    let slower = map.button("slower", ["MouseWheelDown", "DpadDown"]).ok()?;
    let controls = Controls {
        set,
        fly,
        look,
        rise,
        fast,
        faster,
        slower,
    };
    Some((map, controls))
}

/// Writes `speed` as it is shown: whole metres a second from 10 up, and
/// one place after the point below that.
fn write_speed(words: &mut String, speed: f32) -> std::fmt::Result {
    if speed >= 9.95 {
        write!(words, "{speed:.0}")
    } else {
        write!(words, "{speed:.1}")
    }
}

/// A game's level opened for editing: a camera that flies around it, and
/// the panels that say what is in it.
struct Editor {
    controls: Controls,
    /// The camera after the last tick, and after the one before it: frames
    /// are drawn between the two.
    camera: FlyCamera,
    before: FlyCamera,
    panels: LevelPanels,
    /// How fast the camera flies and where it is, as the panels say it.
    status: String,
}

impl Editor {
    /// An editor, and the actions it reads: start the window with them.
    ///
    /// `None` only if the editor's own bindings are misspelled, which a
    /// test rules out.
    ///
    /// The panels list `level`, which is the engine's stand-in if `stand_in`.
    fn new(level: &fossps_assets::Scene, stand_in: bool) -> Option<(Self, ActionMap)> {
        let (map, controls) = controls()?;
        let camera = FlyCamera::default();
        let editor = Self {
            controls,
            camera,
            before: camera,
            panels: LevelPanels::new(level, stand_in),
            status: String::new(),
        };
        Some((editor, map))
    }

    /// Flies the camera to one of the listed entities, at once.
    fn fly_to(&mut self, index: usize) {
        if let Some(entity) = self.panels.entities().get(index) {
            self.camera.fly_to(entity.middle, entity.reach);
            // No frame is drawn part of the way there.
            self.before = self.camera;
        }
    }

    /// What the person at the controls asks for this tick.
    fn read(&self, input: &InputFrame, step: f32) -> Asked {
        let controls = self.controls;
        let (mouse, stick) = (input.delta(controls.look), input.axis2d(controls.look));
        let steps = i8::from(input.just_pressed(controls.faster))
            - i8::from(input.just_pressed(controls.slower));
        Asked {
            fly: input.axis2d(controls.fly),
            rise: input.value(controls.rise),
            turn: [0, 1].map(|axis| mouse[axis] * MOUSE_TURN + stick[axis] * STICK_TURN * step),
            fast: input.pressed(controls.fast),
            speed_steps: steps,
        }
    }
}

impl Game for Editor {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.before = self.camera;
        let asked = self.read(input, step);
        self.camera.step(&asked, step);
    }

    fn frame(&mut self, frame: &mut Frame) {
        frame.use_action_set(self.controls.set);
        let [x, y, z] = self.camera.position;
        self.status.clear();
        // Writing to a `String` cannot fail.
        let _ = write_speed(&mut self.status, self.camera.speed);
        let _ = write!(self.status, " m/s at {x:.1}, {y:.1}, {z:.1}");
        let (panels, status) = (&mut self.panels, self.status.as_str());
        let mut asked = panels::Asked::default();
        frame.panels(|ui| asked = panels.show(ui, status));
        if let Some(index) = asked.fly_to {
            self.fly_to(index);
        }
        // After flying somewhere this is where the camera now is: there is
        // nothing to blend from.
        let seen = self.camera.blended(&self.before, frame.alpha());
        frame.look_from(seen.position, seen.yaw, seen.pitch);
    }
}

/// Opens a window on `level`, the level of the game called `name`, and lets
/// the person fly around it until the window is closed. With no level, the
/// engine's stand-in is shown.
///
/// Call this from the main thread.
///
/// # Errors
///
/// Fails as [`run`](crate::run) does: if the window system cannot be
/// reached, the window cannot be created, or no graphics card here can draw
/// with Vulkan 1.3. The error's message says what to do about it.
pub fn run(name: &str, level: Option<fossps_assets::Scene>) -> Result<(), AppError> {
    let mut config = Config {
        level,
        title: if name.is_empty() {
            "FOSSPS editor".to_owned()
        } else {
            format!("{name} | FOSSPS editor")
        },
        ..Config::default()
    };
    let made = match &config.level {
        Some(level) => Editor::new(level, false),
        None => Editor::new(&crate::standin::scene(), true),
    };
    let Some((editor, actions)) = made else {
        // Not reachable: the bindings are fixed and covered by a test.
        return Ok(());
    };
    config.actions = actions;
    app::run_editor(config, editor)
}

#[cfg(test)]
mod tests {
    use fossps_assets::{Instance, Light, LightKind, Marker, Mesh, MeshPart, Scene};
    use fossps_input::{GamepadButton, Input, Key, MouseWheel, Stick, StickAxis};

    use super::*;

    const STEP: f32 = 1.0 / 60.0;

    /// An editor and the input it is played with, with its action set live
    /// as the window makes it after the first frame.
    fn editor() -> (Editor, Input) {
        editor_of(&crate::standin::scene())
    }

    /// The same, for `level`.
    fn editor_of(level: &Scene) -> (Editor, Input) {
        let (editor, map) = Editor::new(level, true).unwrap();
        let mut input = Input::new(map);
        input.set_active_set(editor.controls.set);
        (editor, input)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn every_action_is_on_keyboard_and_mouse_and_on_a_gamepad() {
        let (map, controls) = controls().unwrap();
        assert_eq!(map.parity_gaps(), Vec::new());
        assert_eq!(map.conflicts(), Vec::new());
        assert_eq!(map.mouse_mode(controls.set), Some(MouseMode::Pointer));
        for action in map.actions() {
            assert_eq!(map.set_of(action), Some(controls.set));
        }
    }

    #[test]
    fn keys_fly_where_the_camera_faces_and_straight_up() {
        let (mut editor, mut input) = editor();
        editor.camera = FlyCamera {
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            speed: 6.0,
        };

        input.press(Key::W);
        for _ in 0..60 {
            editor.tick(input.tick(), STEP);
        }
        input.release(Key::W);
        let [x, y, z] = editor.camera.position;
        assert!(
            close(x, 0.0) && close(y, 0.0) && close(z, -6.0),
            "{x} {y} {z}"
        );

        // Looking up, forwards is up too; E is straight up whatever the view.
        editor.camera.pitch = 0.5;
        input.press(Key::W);
        editor.tick(input.tick(), STEP);
        input.release(Key::W);
        assert!(editor.camera.position[1] > 0.0);
        let before = editor.camera.position;
        input.press(Key::E);
        editor.tick(input.tick(), STEP);
        input.release(Key::E);
        let after = editor.camera.position;
        assert!(close(after[1] - before[1], 6.0 * STEP));
        assert!(close(after[0], before[0]) && close(after[2], before[2]));

        // Q is down, and D is to the right of where the camera faces.
        editor.camera = FlyCamera {
            yaw: TAU / 4.0,
            ..editor.camera
        };
        let before = editor.camera.position;
        input.press(Key::Q);
        input.press(Key::D);
        editor.tick(input.tick(), STEP);
        let after = editor.camera.position;
        assert!(after[1] < before[1]);
        // Turned a quarter to the left, the camera faces -X; its right is -Z.
        assert!(after[2] < before[2] && close(after[0], before[0]));
    }

    #[test]
    fn two_directions_at_once_are_no_faster_than_one() {
        let mut camera = FlyCamera {
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            speed: 10.0,
        };
        let asked = Asked {
            fly: [1.0, 1.0],
            rise: 1.0,
            ..Asked::default()
        };
        camera.step(&asked, 1.0);
        let moved = camera.position.iter().map(|part| part * part).sum::<f32>();
        assert!(close(moved.sqrt(), 10.0), "{moved}");

        // Half a push is half the speed.
        let mut camera = FlyCamera {
            position: [0.0; 3],
            ..camera
        };
        let asked = Asked {
            fly: [0.0, 0.5],
            ..Asked::default()
        };
        camera.step(&asked, 1.0);
        assert!(close(camera.position[2], -5.0));
    }

    #[test]
    fn shift_flies_faster_and_the_wheel_changes_the_speed() {
        let (mut editor, mut input) = editor();
        editor.camera.pitch = 0.0;
        let start = editor.camera.position[2];
        input.press(Key::W);
        input.press(Key::LeftShift);
        editor.tick(input.tick(), STEP);
        assert!(close(
            start - editor.camera.position[2],
            START_SPEED * FAST * STEP
        ));
        input.release(Key::W);
        input.release(Key::LeftShift);

        input.press(MouseWheel::MouseWheelUp);
        editor.tick(input.tick(), STEP);
        assert!(close(editor.camera.speed, START_SPEED * SPEED_STEP));
        input.press(MouseWheel::MouseWheelDown);
        editor.tick(input.tick(), STEP);
        input.press(MouseWheel::MouseWheelDown);
        editor.tick(input.tick(), STEP);
        assert!(close(editor.camera.speed, START_SPEED / SPEED_STEP));

        // The speed stops at both ends.
        let mut camera = FlyCamera::default();
        let slower = Asked {
            speed_steps: -100,
            ..Asked::default()
        };
        camera.step(&slower, STEP);
        assert_eq!(camera.speed, SPEED_RANGE[0]);
        let faster = Asked {
            speed_steps: 100,
            ..Asked::default()
        };
        camera.step(&faster, STEP);
        assert_eq!(camera.speed, SPEED_RANGE[1]);
    }

    #[test]
    fn the_mouse_turns_the_view_and_it_never_tips_over() {
        let (mut editor, mut input) = editor();
        let (yaw, pitch) = (editor.camera.yaw, editor.camera.pitch);
        input.move_mouse(100.0, 50.0);
        editor.tick(input.tick(), STEP);
        // Right turns right, which is yaw going down; up looks up.
        assert!(close(editor.camera.yaw, yaw - 100.0 * MOUSE_TURN));
        assert!(close(editor.camera.pitch, pitch + 50.0 * MOUSE_TURN));

        input.move_mouse(0.0, 100_000.0);
        editor.tick(input.tick(), STEP);
        assert_eq!(editor.camera.pitch, STEEPEST);
        input.move_mouse(0.0, -200_000.0);
        editor.tick(input.tick(), STEP);
        assert_eq!(editor.camera.pitch, -STEEPEST);

        // Numbers that are not numbers move nothing.
        let before = editor.camera;
        let broken = Asked {
            turn: [f32::NAN, 0.0],
            fly: [1.0, 1.0],
            ..Asked::default()
        };
        editor.camera.step(&broken, STEP);
        assert_eq!(editor.camera, before);
    }

    #[test]
    fn a_gamepad_does_everything_the_keyboard_and_mouse_do() {
        let (mut editor, mut input) = editor();
        editor.camera = FlyCamera {
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            speed: 6.0,
        };

        input.set_stick(Stick::LeftStick, StickAxis::Y, 1.0);
        editor.tick(input.tick(), STEP);
        assert!(editor.camera.position[2] < 0.0, "the left stick flies");
        input.set_stick(Stick::LeftStick, StickAxis::Y, 0.0);

        input.set_stick(Stick::RightStick, StickAxis::X, 1.0);
        editor.tick(input.tick(), STEP);
        assert!(editor.camera.yaw < 0.0, "the right stick looks");
        input.set_stick(Stick::RightStick, StickAxis::X, 0.0);

        let height = editor.camera.position[1];
        input.set_button_amount(GamepadButton::RightTrigger, 1.0);
        editor.tick(input.tick(), STEP);
        assert!(
            editor.camera.position[1] > height,
            "the right trigger rises"
        );
        input.set_button_amount(GamepadButton::RightTrigger, 0.0);
        let height = editor.camera.position[1];
        input.set_button_amount(GamepadButton::LeftTrigger, 1.0);
        editor.tick(input.tick(), STEP);
        assert!(editor.camera.position[1] < height, "the left trigger sinks");
        input.set_button_amount(GamepadButton::LeftTrigger, 0.0);

        input.press(GamepadButton::DpadUp);
        editor.tick(input.tick(), STEP);
        input.release(GamepadButton::DpadUp);
        assert!(close(editor.camera.speed, 6.0 * SPEED_STEP));
        input.press(GamepadButton::DpadDown);
        editor.tick(input.tick(), STEP);
        assert!(close(editor.camera.speed, 6.0));

        let from = editor.camera.position[2];
        input.set_stick(Stick::LeftStick, StickAxis::Y, 1.0);
        input.press(GamepadButton::LeftStickPress);
        editor.tick(input.tick(), STEP);
        assert!(close(from - editor.camera.position[2], 6.0 * FAST * STEP));
    }

    #[test]
    fn frames_between_ticks_are_blended_the_short_way_round() {
        let before = FlyCamera {
            position: [0.0, 0.0, 0.0],
            yaw: 0.1,
            pitch: 0.0,
            speed: 8.0,
        };
        let after = FlyCamera {
            position: [2.0, 0.0, -4.0],
            // The same as -0.1, one turn on.
            yaw: TAU - 0.1,
            pitch: 0.2,
            speed: 8.0,
        };
        let half = after.blended(&before, 0.5);
        assert_eq!(half.position, [1.0, 0.0, -2.0]);
        assert!(close(half.pitch, 0.1));
        assert!(close(half.yaw.rem_euclid(TAU), 0.0) || close(half.yaw.rem_euclid(TAU), TAU));
        assert_eq!(after.blended(&before, 1.0).position, after.position);
        assert!(close(after.blended(&before, 0.0).yaw.rem_euclid(TAU), 0.1));
    }

    /// A crate placed twice (once named, once turned a quarter to the
    /// left), a spawn marker and a sun.
    fn small_level() -> Scene {
        let mut level = Scene::default();
        level.materials.push(fossps_assets::Material {
            base_color: [1.0, 0.0, 0.0, 1.0],
            ..fossps_assets::Material::default()
        });
        level.meshes.push(Mesh {
            name: Some("Crate".to_owned()),
            parts: vec![MeshPart {
                indices: vec![0; 36],
                bounds: fossps_assets::Bounds {
                    min: [-1.0, 0.0, -2.0],
                    max: [1.0, 2.0, 2.0],
                },
                ..MeshPart::default()
            }],
        });
        let mut there = crate::standin::scene().instances[0].transform;
        there = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [10.0, 0.0, -4.0, there[3][3]],
        ];
        level.instances.push(Instance {
            mesh: 0,
            transform: there,
            mirrored: false,
            name: Some("Crate A".to_owned()),
            extras: None,
        });
        level.instances.push(Instance {
            mesh: 0,
            // A quarter turn to the left about the upright axis.
            transform: [
                [0.0, 0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            mirrored: false,
            name: None,
            extras: None,
        });
        level.markers.push(Marker {
            name: Some("Spawn red".to_owned()),
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [3.0, 0.0, 5.0, 1.0],
            ],
            extras: Some(r#"{"team":"red"}"#.to_owned()),
        });
        level.lights.push(Light {
            name: Some("Sun".to_owned()),
            kind: LightKind::Sun,
            color: [1.0, 1.0, 1.0],
            intensity: 100_000.0,
            range: None,
            position: [0.0; 3],
            direction: [0.0, -1.0, 0.0],
        });
        level
    }

    const SCREEN: [u32; 2] = [1280, 720];

    /// One frame of the editor in a window `SCREEN` in size, in which the
    /// mouse did `events` at `time` seconds.
    fn panel_frame(
        editor: &mut Editor,
        context: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Frame, egui::FullOutput) {
        #[allow(clippy::cast_precision_loss)]
        let size = egui::vec2(SCREEN[0] as f32, SCREEN[1] as f32);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time),
            events,
            ..egui::RawInput::default()
        };
        let mut frame = Frame::new(1.0)
            .for_window(Some(SCREEN), fossps_ui::DrawBuffer::default())
            .with_panels(context.clone(), input);
        editor.frame(&mut frame);
        let mut output = frame.take_panels().unwrap();
        // Nothing is drawn with these; `egui` asks to be told so.
        output.textures_delta.clear();
        (frame, output)
    }

    /// The left mouse button going down or up at `at`.
    fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Every word drawn in `output`, in one string.
    fn words(output: &egui::FullOutput) -> String {
        fn gather(shape: &egui::Shape, into: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    into.push_str(text.galley.text());
                    into.push('\n');
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| gather(shape, into)),
                _ => {}
            }
        }
        let mut all = String::new();
        for clipped in &output.shapes {
            gather(&clipped.shape, &mut all);
        }
        all
    }

    #[test]
    fn the_list_names_everything_in_the_level_by_sort() {
        let level = small_level();
        let listed = panels::entities(&level);
        let names: Vec<(&str, panels::Kind)> = listed
            .iter()
            .map(|entity| (entity.name.as_str(), entity.kind))
            .collect();
        assert_eq!(
            names,
            [
                ("Crate A", panels::Kind::Object),
                ("Crate", panels::Kind::Object),
                ("Spawn red", panels::Kind::Marker),
                ("Sun", panels::Kind::Light),
            ]
        );
        // The crate is 2 by 2 by 4 m, standing on its origin at 10, 0, -4.
        assert_eq!(listed[0].middle, [10.0, 1.0, -4.0]);
        assert!(close(listed[0].reach, 24.0_f32.sqrt() / 2.0));
        // Turned a quarter, its long side lies along X.
        assert_eq!(listed[1].middle, [0.0, 1.0, 0.0]);
        assert_eq!(listed[2].middle, [3.0, 0.0, 5.0]);

        // With no name of its own or of its shape, it is numbered.
        let mut nameless = level.clone();
        nameless.meshes[0].name = None;
        assert_eq!(panels::entities(&nameless)[1].name, "Unnamed object 2");

        // An empty level lists nothing and says so.
        let (mut editor, _) = editor_of(&Scene::default());
        let context = panel_context();
        let (_, output) = panel_frame(&mut editor, &context, 0.0, Vec::new());
        assert!(editor.panels.entities().is_empty());
        assert!(words(&output).contains("Nothing in it yet"));
    }

    #[test]
    fn the_panels_show_the_list_the_inspector_and_how_to_fly() {
        let (mut editor, mut input) = editor_of(&small_level());
        editor.tick(input.tick(), STEP);
        let context = panel_context();
        // Panels find their sizes in the first frame.
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        let (frame, output) = panel_frame(&mut editor, &context, 0.1, Vec::new());

        assert_eq!(frame.action_set(), Some(editor.controls.set));
        assert_eq!(frame.view().unwrap().position, editor.camera.position);
        let shown = words(&output);
        for expected in [
            "Stand-in level",
            "2 objects, 1 marker, 1 light",
            "Crate A",
            "Spawn red",
            "Sun",
            "Nothing chosen",
            "Click a name in the list on the left to see it here.",
            "hold the right mouse button",
            "Gamepad: the sticks fly and look",
            "8.0 m/s at 0.0, 5.0, 22.0",
        ] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }
        let mut speeds = String::new();
        write_speed(&mut speeds, 0.5).unwrap();
        write_speed(&mut speeds, 64.0).unwrap();
        assert_eq!(speeds, "0.564");

        // Without a window there is nothing to make panels with, and the
        // frame still says where to look from.
        let mut frame = Frame::new(1.0);
        editor.frame(&mut frame);
        assert!(frame.take_panels().is_none());
        assert!(frame.view().is_some());
    }

    #[test]
    fn a_click_chooses_and_the_inspector_says_what_is_known() {
        let (mut editor, _) = editor_of(&small_level());
        let context = panel_context();
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        assert_eq!(editor.panels.selected(), None);
        let rows = editor.panels.rows.clone();
        assert_eq!(rows.len(), 4);
        // Rows are as wide as the panel, so a click anywhere on one counts.
        assert!(rows[0].1.width() > 200.0, "{:?}", rows[0].1);

        let click = |editor: &mut Editor, at: egui::Pos2, time: f64| {
            let down = vec![egui::Event::PointerMoved(at), button(at, true)];
            let _ = panel_frame(editor, &context, time, down);
            panel_frame(editor, &context, time + 0.05, vec![button(at, false)])
        };
        let camera = editor.camera;
        let (_, _) = click(&mut editor, rows[1].1.center(), 1.0);
        assert_eq!(editor.panels.selected(), Some(1));
        let (_, output) = panel_frame(&mut editor, &context, 1.2, Vec::new());
        let shown = words(&output);
        for expected in [
            "0.00, 0.00, 0.00 m",
            "4.00 wide, 2.00 high, 2.00 deep (m)",
            "90° to the left",
            "Crate, 12 triangles",
            "Fly to it",
            "cannot be changed here yet",
        ] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }
        assert!(!shown.contains("Nothing chosen"));
        assert_eq!(editor.camera, camera, "one click does not move the camera");

        // The marker shows what was written on it in the 3D tool.
        let _ = click(&mut editor, rows[2].1.center(), 3.0);
        assert_eq!(editor.panels.selected(), Some(2));
        let (_, output) = panel_frame(&mut editor, &context, 3.2, Vec::new());
        let shown = words(&output);
        assert!(shown.contains("3.00, 0.00, 5.00 m"), "{shown}");
        assert!(shown.contains(r#"{"team":"red"}"#), "{shown}");
        assert!(shown.contains("0° to the left"), "{shown}");

        // The sun has no place, only a direction and a brightness.
        let _ = click(&mut editor, rows[3].1.center(), 5.0);
        let (_, output) = panel_frame(&mut editor, &context, 5.2, Vec::new());
        let shown = words(&output);
        assert!(shown.contains("100000 lux"), "{shown}");
        assert!(shown.contains("0.00, -1.00, 0.00"), "{shown}");
        assert!(!shown.contains("Position"), "{shown}");

        // A click on the picture, between the panels, chooses nothing else.
        let _ = click(&mut editor, egui::pos2(640.0, 300.0), 7.0);
        assert_eq!(editor.panels.selected(), Some(3));
    }

    #[test]
    fn a_double_click_flies_the_camera_to_what_was_clicked() {
        let (mut editor, _) = editor_of(&small_level());
        let context = panel_context();
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        let at = editor.panels.rows[0].1.center();
        let mut time = 1.0;
        let mut view = None;
        for pressed in [true, false, true, false] {
            let mut events = vec![button(at, pressed)];
            if view.is_none() {
                events.insert(0, egui::Event::PointerMoved(at));
            }
            let (frame, _) = panel_frame(&mut editor, &context, time, events);
            view = frame.view();
            time += 0.05;
        }
        assert_eq!(editor.panels.selected(), Some(0));
        // The crate's middle is 10, 1, -4 and the camera looks at it.
        let camera = editor.camera;
        assert_eq!(camera, editor.before, "it is there at once");
        assert_eq!(view.unwrap().position, camera.position);
        let (sin_yaw, cos_yaw) = camera.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = camera.pitch.sin_cos();
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let away = [0, 1, 2].map(|axis| [10.0, 1.0, -4.0][axis] - camera.position[axis]);
        let distance = away.iter().map(|part| part * part).sum::<f32>().sqrt();
        assert!((5.0..12.0).contains(&distance), "{distance} m away");
        for axis in 0..3 {
            assert!(close(away[axis] / distance, forward[axis]), "{away:?}");
        }
        assert!(camera.pitch <= -FLY_TO_TILT, "it looks down at it");

        // Something huge is not flown a kilometre away from, and numbers
        // that are not numbers move nothing.
        let mut camera = FlyCamera::default();
        camera.fly_to([0.0; 3], 700.0);
        let far = camera.position.iter().map(|part| part * part).sum::<f32>();
        assert!(close(far.sqrt(), FLY_TO_FURTHEST));
        let before = camera;
        camera.fly_to([f32::NAN, 0.0, 0.0], 1.0);
        camera.fly_to([0.0; 3], f32::INFINITY);
        assert_eq!(camera, before);
    }

    #[test]
    fn the_level_is_drawn_under_a_grid_between_the_panels() {
        use fossps_render::{Camera, Headless, Panels, RenderError, Upscaling};

        let mut headless = match Headless::new(SCREEN[0], SCREEN[1], Upscaling::Off) {
            Ok(headless) => headless,
            Err(
                error @ (RenderError::NoVulkan(_)
                | RenderError::OldVulkan { .. }
                | RenderError::NoSuitableGpu { .. }),
            ) => {
                let required =
                    std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
                assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
                eprintln!("Skipping the drawing test: {error}");
                return;
            }
            Err(error) => panic!("{error}"),
        };
        headless.set_scene(&crate::standin::scene()).unwrap();

        // What the window does: a tick, then the editor's frame.
        let (mut editor, mut input) = editor();
        editor.tick(input.tick(), STEP);
        let context = panel_context();
        let view = {
            let mut frame = Frame::new(1.0);
            editor.frame(&mut frame);
            frame.view().unwrap()
        };
        let camera = Camera {
            position: view.position,
            yaw: view.yaw,
            pitch: view.pitch,
            ..Camera::default()
        };
        let level = headless.screenshot(&camera).unwrap();
        headless.set_grid(true);
        let with_grid = headless.screenshot(&camera).unwrap();
        // Two frames, as panels find their sizes in the first. Each is
        // handed over, as the window does, though only the second is drawn:
        // the letters come with the first.
        #[allow(clippy::cast_precision_loss)]
        let size = egui::vec2(SCREEN[0] as f32, SCREEN[1] as f32);
        for time in [0.0, 0.1] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                ..egui::RawInput::default()
            };
            let mut frame = Frame::new(1.0)
                .for_window(Some(SCREEN), fossps_ui::DrawBuffer::default())
                .with_panels(context.clone(), input);
            editor.frame(&mut frame);
            let output = frame.take_panels().unwrap();
            headless.set_panels(Panels {
                shapes: context.tessellate(output.shapes, output.pixels_per_point),
                textures: output.textures_delta,
                pixels_per_point: output.pixels_per_point,
            });
        }
        let shown = headless.screenshot(&camera).unwrap();
        // A second frame draws the same panels: the letters sent once stay.
        assert_eq!(headless.screenshot(&camera).unwrap(), shown);
        // Set `FOSSPS_SAVE_PICTURES` to a folder to look at the picture:
        // 1280 by 720 pixels of four bytes each.
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            std::fs::write(std::path::Path::new(&folder).join("editor.rgba"), &shown).unwrap();
        }

        let pixel = |picture: &[u8], x: usize, y: usize| {
            let at = (y * SCREEN[0] as usize + x) * 4;
            [picture[at], picture[at + 1], picture[at + 2]]
        };
        let light = |picture: &[u8], x: usize, y: usize| {
            pixel(picture, x, y)
                .iter()
                .map(|&part| u32::from(part))
                .sum::<u32>()
        };
        // The grid's lines are lighter than the floor they lie on: along a
        // row of the floor near the bottom, some pixels are lighter with
        // the grid.
        let row = 640;
        let lighter = (0..SCREEN[0] as usize)
            .filter(|&x| light(&with_grid, x, row) > light(&level, x, row) + 20)
            .count();
        assert!((8..400).contains(&lighter), "{lighter} lighter pixels");
        // The sky is left alone.
        assert_eq!(light(&with_grid, 640, 20), light(&level, 640, 20));

        // Each side panel is its plain dark colour, exactly as egui's dark
        // look has it, with light words on it; so is the bar along the
        // bottom.
        let fill = context.global_style().visuals.panel_fill;
        let fill = [fill.r(), fill.g(), fill.b()];
        let near = |a: [u8; 3], b: [u8; 3]| (0..3).all(|part| a[part].abs_diff(b[part]) <= 1);
        for (name, columns, rows) in [
            ("list", 4..240, 4..600),
            ("inspector", 1000..1276, 4..600),
            ("how to fly", 4..1276, 684..716),
        ] {
            let (mut plain, mut bright, mut all) = (0, 0, 0);
            for y in rows.clone() {
                for x in columns.clone() {
                    all += 1;
                    plain += u32::from(near(pixel(&shown, x, y), fill));
                    bright += u32::from(light(&shown, x, y) > 330);
                }
            }
            assert!(plain * 10 > all * 7, "{name}: {plain} of {all} plain");
            assert!(bright > 200, "{name}: {bright} bright pixels");
        }
        // Between the panels, the picture is the one without them.
        for (x, y) in [(640, 300), (500, 600), (800, 100)] {
            assert_eq!(pixel(&shown, x, y), pixel(&with_grid, x, y));
        }
    }
}
