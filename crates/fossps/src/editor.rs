//! Edit mode: `fossps edit` opens a game's level to fly around in, over a
//! ground grid that shows scale.
//!
//! The camera flies as the viewports of other editors do. Hold the right
//! mouse button to look around; W, A, S and D fly where the camera faces, E
//! and Q go straight up and down, Shift flies faster and the mouse wheel
//! changes the speed. On a gamepad the sticks fly and look, the triggers go
//! down and up, pressing the left stick flies faster and the d-pad changes
//! the speed.
//!
//! Until levels are loaded from files (#60), the level is the stand-in one
//! every game is played in.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;

use fossps_input::{ActionId, ActionMap, ActionSetId, InputFrame, MouseMode};
use fossps_render::Font;
use fossps_ui::{Anchor, Color, Ui, UiInput, WidgetId};

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

/// What the editor writes over the picture: how to fly, and how fast.
const HELP: [[&str; 2]; 5] = [
    ["Look", "hold the right mouse button, or the right stick"],
    ["Fly", "W A S D, or the left stick"],
    ["Up, down", "E and Q, or the triggers"],
    ["Faster", "hold Shift, or press the left stick"],
    ["Speed", "mouse wheel, or d-pad up and down"],
];

/// The size of the help text, and how far apart its lines and its two
/// columns are, in pixels.
const HELP_TEXT: f32 = 18.0;
const HELP_LINE: f32 = 24.0;
const HELP_COLUMN: f32 = 140.0;

/// The words over the picture.
struct Hud {
    ui: Ui,
    /// The line that says how fast the camera flies and where it is.
    status: WidgetId,
    /// What that line says now, to change it only when it differs.
    shown: String,
    /// Where the next status is written, to keep its memory.
    next: String,
}

impl Hud {
    fn new() -> Self {
        let mut ui = Ui::default();
        // Text is measured in the font it is drawn in.
        ui.set_text_measure(Box::new(Font::bundled()));
        // The level can be any colour, so the words get a dark panel.
        // `HELP` has five lines; f32 holds that exactly.
        #[allow(clippy::cast_precision_loss)]
        let lines = HELP.len() as f32 + 1.0;
        ui.hud_rect(
            Anchor::TopLeft,
            [-8.0, -8.0],
            [760.0, lines * HELP_LINE + 16.0],
            Color::rgba(0.03, 0.04, 0.06, 0.72),
        );
        let mut down = 0.0;
        for [what, how] in HELP {
            ui.hud_text(Anchor::TopLeft, [0.0, down], HELP_TEXT, what);
            ui.hud_text(Anchor::TopLeft, [HELP_COLUMN, down], HELP_TEXT, how);
            down += HELP_LINE;
        }
        let status = ui.hud_text(Anchor::TopLeft, [0.0, down], HELP_TEXT, "");
        Self {
            ui,
            status,
            shown: String::new(),
            next: String::new(),
        }
    }

    /// Says how fast `camera` flies and where it is.
    fn show(&mut self, camera: &FlyCamera) {
        let [x, y, z] = camera.position;
        self.next.clear();
        // Writing to a `String` cannot fail.
        let _ = write_speed(&mut self.next, camera.speed);
        let _ = write!(self.next, " m/s at {x:.1}, {y:.1}, {z:.1}");
        if self.next != self.shown {
            // The widget is this UI's own text, so setting it cannot fail.
            let _ = self.ui.set_text(self.status, &self.next);
            std::mem::swap(&mut self.shown, &mut self.next);
        }
    }
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

/// A game's level opened for editing: a camera that flies around it.
struct Editor {
    controls: Controls,
    /// The camera after the last tick, and after the one before it: frames
    /// are drawn between the two.
    camera: FlyCamera,
    before: FlyCamera,
    hud: Hud,
}

impl Editor {
    /// An editor, and the actions it reads: start the window with them.
    ///
    /// `None` only if the editor's own bindings are misspelled, which a
    /// test rules out.
    fn new() -> Option<(Self, ActionMap)> {
        let (map, controls) = controls()?;
        let camera = FlyCamera::default();
        let editor = Self {
            controls,
            camera,
            before: camera,
            hud: Hud::new(),
        };
        Some((editor, map))
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
        let seen = self.camera.blended(&self.before, frame.alpha());
        frame.look_from(seen.position, seen.yaw, seen.pitch);
        self.hud.show(&self.camera);
        self.hud
            .ui
            .update(&UiInput::default(), 0.0, frame.screen_size());
        frame.draw_ui(&self.hud.ui);
    }
}

/// Opens a window on the level of the game called `name` and lets the
/// person fly around it until the window is closed.
///
/// Call this from the main thread.
///
/// # Errors
///
/// Fails as [`run`](crate::run) does: if the window system cannot be
/// reached, the window cannot be created, or no graphics card here can draw
/// with Vulkan 1.3. The error's message says what to do about it.
pub fn run(name: &str) -> Result<(), AppError> {
    let mut config = Config {
        title: if name.is_empty() {
            "FOSSPS editor".to_owned()
        } else {
            format!("{name} | FOSSPS editor")
        },
        ..Config::default()
    };
    let Some((editor, actions)) = Editor::new() else {
        // Not reachable: the bindings are fixed and covered by a test.
        return Ok(());
    };
    config.actions = actions;
    app::run_editor(config, editor)
}

#[cfg(test)]
mod tests {
    use fossps_input::{GamepadButton, Input, Key, MouseWheel, Stick, StickAxis};
    use fossps_ui::Draw;

    use super::*;

    const STEP: f32 = 1.0 / 60.0;

    /// An editor and the input it is played with, with its action set live
    /// as the window makes it after the first frame.
    fn editor() -> (Editor, Input) {
        let (editor, map) = Editor::new().unwrap();
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

    #[test]
    fn the_frame_shows_the_view_the_help_and_the_speed() {
        let (mut editor, mut input) = editor();
        editor.tick(input.tick(), STEP);
        let mut frame = Frame::new(1.0);
        editor.frame(&mut frame);

        assert_eq!(frame.action_set(), Some(editor.controls.set));
        let view = frame.view().unwrap();
        assert_eq!(view.position, editor.camera.position);
        let drawn = frame.take_ui();
        let words: Vec<&str> = drawn
            .draw()
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text { text, .. } => Some(text),
                Draw::Rect { .. } => None,
            })
            .collect();
        assert!(words.contains(&"hold the right mouse button, or the right stick"));
        assert!(words.contains(&"8.0 m/s at 0.0, 5.0, 22.0"), "{words:?}");
        let mut speeds = String::new();
        write_speed(&mut speeds, 0.5).unwrap();
        write_speed(&mut speeds, 64.0).unwrap();
        assert_eq!(speeds, "0.564");
    }

    #[test]
    fn the_level_is_drawn_under_a_grid_with_the_help_in_the_corner() {
        use fossps_render::{Camera, Headless, RenderError, Upscaling};

        const SCREEN: [u32; 2] = [1280, 720];
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
        let mut frame = Frame::new(1.0).for_window(Some(SCREEN), fossps_ui::DrawBuffer::default());
        editor.frame(&mut frame);
        let view = frame.view().unwrap();
        let camera = Camera {
            position: view.position,
            yaw: view.yaw,
            pitch: view.pitch,
            ..Camera::default()
        };
        let level = headless.screenshot(&camera).unwrap();
        headless.set_grid(true);
        let with_grid = headless.screenshot(&camera).unwrap();
        let ui = frame.take_ui();
        headless.set_ui(ui.draw());
        let shown = headless.screenshot(&camera).unwrap();
        // Set `FOSSPS_SAVE_PICTURES` to a folder to look at the picture:
        // 1280 by 720 pixels of four bytes each.
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            std::fs::write(std::path::Path::new(&folder).join("editor.rgba"), &shown).unwrap();
        }

        let light = |picture: &[u8], x: usize, y: usize| {
            let at = (y * SCREEN[0] as usize + x) * 4;
            picture[at..at + 3]
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

        // The help is light text on a dark panel in the top-left corner.
        let panel = (30..150).flat_map(|y| (30..480).map(move |x| (x, y)));
        let (mut dark, mut bright) = (0, 0);
        for (x, y) in panel {
            match light(&shown, x, y) {
                0..=120 => dark += 1,
                600.. => bright += 1,
                _ => {}
            }
        }
        assert!(dark > 30_000, "{dark} dark pixels in the panel");
        assert!(bright > 500, "{bright} bright pixels in the panel");
        // Outside the panel, the picture is the one without the help.
        assert_eq!(light(&shown, 900, 400), light(&with_grid, 900, 400));
    }
}
