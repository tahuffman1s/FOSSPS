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
//!
//! The level file is never written. What the editor places (spawn points,
//! zones, objectives) goes in the game's `map.toml` beside it, which is
//! saved by itself a moment after each change, with no save button, and
//! read again when something else saves it. Each placed thing is marked in
//! the picture with a dot and a line the way it faces.
//!
//! A mark is moved by dragging it and turned by dragging the tip of its
//! line, on a grid; with keys or a gamepad, things are placed, picked up
//! and put down where the camera looks. See [`place`].
//!
//! F5, or Select on a gamepad, plays the level from where the camera is,
//! and the same key or button comes back to editing with the camera and
//! the map as they were. See [`play`].

mod panels;
mod place;
mod play;

use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;

use fossps_input::{ActionId, ActionMap, ActionSetId, InputFrame, MouseMode};

pub(crate) use self::panels::context as panel_context;
use self::panels::{LevelPanels, placed_name};
use self::place::{Drag, Held, Hit, Keys, Snap};
use self::play::Play;
use crate::app::{self, AppError, Config};
use crate::game::{Frame, Game};
use crate::map::{Map, MapFile, SPAWN};
use crate::online::{MOUSE_TURN, STICK_TURN, World};
use crate::sound_output::SoundAsk;

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

/// A change to the map is saved once this many seconds have passed with no
/// further change, so a number being dragged is written once, at its end.
const SAVE_AFTER: f32 = 0.4;
/// A save that failed is tried again after this many seconds.
const SAVE_AGAIN_AFTER: f32 = 5.0;
/// Something is placed on the ground where the camera looks if that is
/// within this many metres, and otherwise this far ahead of the camera.
const PLACE_WITHIN: f32 = 60.0;
const PLACE_AHEAD: f32 = 5.0;
/// How much of the picture is seen from bottom to top, in degrees: the
/// camera the window draws with.
const FOV_DEGREES: f32 = 74.0;
/// A placed thing's mark in the picture: the dot's size in `egui` points,
/// how long the line showing its facing is in metres, and how far away its
/// name is still written, in metres.
const MARK_SIZE: f32 = 6.0;
const MARK_FACING: f32 = 1.0;
const MARK_NAMED_WITHIN: f32 = 40.0;
/// The tip of the chosen thing's line, which is dragged to turn it, and the
/// ring where the next thing goes: their sizes in `egui` points.
const TIP_SIZE: f32 = 4.5;
const SPOT_SIZE: f32 = 7.0;

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

    /// The ways the camera's right, up and forwards point.
    fn axes(&self) -> [[f32; 3]; 3] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        [
            [cos_yaw, 0.0, -sin_yaw],
            [sin_yaw * sin_pitch, cos_pitch, cos_yaw * sin_pitch],
            [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch],
        ]
    }

    /// Where `point` is in a picture `size` across and down, and how far
    /// ahead of the camera it is in metres. `None` if it is behind the
    /// camera or right at it.
    fn pictured(&self, point: [f32; 3], size: [f32; 2]) -> Option<([f32; 2], f32)> {
        let [right, up, forward] = self.axes();
        let away = [0, 1, 2].map(|axis| point[axis] - self.position[axis]);
        let along = |way: [f32; 3]| (0..3).map(|axis| away[axis] * way[axis]).sum::<f32>();
        let ahead = along(forward);
        if !(ahead.is_finite() && ahead > 0.05 && size[1] > 0.0) {
            return None;
        }
        // As the renderer's camera does it: `zoom` half-pictures a metre
        // at a metre away, top to bottom.
        let zoom = 1.0 / (FOV_DEGREES.to_radians() / 2.0).tan();
        let half = size[1] / 2.0;
        let at = [
            size[0] / 2.0 + along(right) / ahead * zoom * half,
            half - along(up) / ahead * zoom * half,
        ];
        Some((at, ahead))
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
    /// Places a new thing where the camera looks, or puts down what is held.
    place: ActionId,
    /// Picks up what the camera looks at, or puts it down.
    grab: ActionId,
    /// Puts what is held back where it was, or stops placing by clicking.
    cancel: ActionId,
    turn_left: ActionId,
    turn_right: ActionId,
    /// Changes what things snap to.
    snap: ActionId,
    /// Plays the level from where the camera is.
    play: ActionId,
    /// Comes back from playing. Bound as `play` is, so it is one key and
    /// one button both ways, but among the game's own actions, which are
    /// the live ones while the level is played.
    back: ActionId,
}

/// What switches between editing and playing: one key and one gamepad
/// button. F5 is "play" in most editors; Select (View, Back) is the one
/// button left over once a game has Start for its pause menu.
const PLAY_KEYS: [&str; 2] = ["F5", "GamepadSelect"];

/// The editor's own actions and what they are bound to, added to `game`,
/// the actions of the game being edited: every one on keyboard and mouse
/// and on a gamepad.
///
/// They are kept apart from the game's actions in a set called `editor`,
/// which leaves the cursor free for pointing; the window captures the mouse
/// while the right mouse button is held. Their names begin with `editor_`,
/// so none is a name a game uses. The one that comes back from playing
/// joins the game's own.
///
/// `None` if `game` already has a set or an action by one of these names.
fn controls(mut map: ActionMap) -> Option<(ActionMap, Controls)> {
    let back = map.button("editor_back", PLAY_KEYS).ok()?;
    let set = map.add_set("editor", MouseMode::Pointer).ok()?;
    let fly = map.axis2d("editor_fly", ["WASD", "LeftStick"]).ok()?;
    let look = map
        .axis2d("editor_look", ["MouseMotion", "RightStick"])
        .ok()?;
    map.set_stick_curve(look, 2.0);
    let rise = map
        .axis1d("editor_rise", ["Q/E", "LeftTrigger/RightTrigger"])
        .ok()?;
    let fast = map
        .button("editor_fast", ["LeftShift", "LeftStickPress"])
        .ok()?;
    let faster = map
        .button("editor_faster", ["MouseWheelUp", "DpadUp"])
        .ok()?;
    let slower = map
        .button("editor_slower", ["MouseWheelDown", "DpadDown"])
        .ok()?;
    // Not Space or Enter, which also press whichever button of the panels
    // was clicked last.
    let place = map.button("editor_place", ["P", "GamepadSouth"]).ok()?;
    let grab = map.button("editor_grab", ["F", "GamepadWest"]).ok()?;
    let cancel = map
        .button("editor_cancel", ["Escape", "GamepadEast"])
        .ok()?;
    let turn_left = map.button("editor_turn_left", ["Z", "LeftBumper"]).ok()?;
    let turn_right = map.button("editor_turn_right", ["C", "RightBumper"]).ok()?;
    let snap = map.button("editor_snap", ["G", "GamepadNorth"]).ok()?;
    let play = map.button("editor_play", PLAY_KEYS).ok()?;
    let controls = Controls {
        play,
        back,
        set,
        fly,
        look,
        rise,
        fast,
        faster,
        slower,
        place,
        grab,
        cancel,
        turn_left,
        turn_right,
        snap,
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
    /// What is placed in the level.
    map: Map,
    /// Where the map is saved. Without one, nothing is saved.
    file: Option<MapFile>,
    /// How many seconds ago the map was last changed, if it has changes
    /// that are not saved yet. Below zero while waiting to try a failed
    /// save again.
    unsaved: Option<f32>,
    /// How saving went, as the panels say it.
    saved: String,
    /// What positions and turns snap to.
    snap: Snap,
    /// What is being dragged in the picture with the mouse.
    drag: Option<Drag>,
    /// What was picked up with a key or a gamepad, carried where the
    /// camera looks.
    held: Option<Held>,
    /// Whether things were last placed or carried with keys or a gamepad:
    /// the spot the camera looks at is then ringed in the picture.
    aiming: bool,
    /// Every action the window reads: the game's and the editor's.
    actions: ActionMap,
    /// The level's solid shapes, as a game played from here walks them.
    world: World,
    /// The level being played, while it is.
    play: Option<Play>,
    /// The "Play from here" button was clicked: the next tick starts.
    play_asked: bool,
    /// Why playing could not start or stopped by itself, as the panels
    /// say it until the next try.
    play_problem: String,
    /// Playing has just stopped, so the game's sounds are to stop too.
    hush: bool,
}

impl Editor {
    /// An editor, and the actions it reads: start the window with them.
    ///
    /// `None` only if the editor's own bindings are misspelled, which a
    /// test rules out.
    ///
    /// The panels list `level`, which is the engine's stand-in if
    /// `stand_in`, and `placed`: the game's map and the file it is saved in.
    ///
    /// `game` is the actions the game is played with, and `tick_rate` how
    /// many times a second the window ticks: what playing the level needs.
    fn new(
        level: &fossps_assets::Scene,
        stand_in: bool,
        placed: Option<(Map, MapFile)>,
        game: ActionMap,
        tick_rate: u32,
    ) -> Option<(Self, ActionMap)> {
        let (actions, controls) = controls(game)?;
        // A game with no level of its own is played on a flat floor.
        let world = World::for_editor((!stand_in).then_some(level), tick_rate);
        let camera = FlyCamera::default();
        let (map, file) = match placed {
            Some((map, file)) => (map, Some(file)),
            None => (Map::default(), None),
        };
        let editor = Self {
            controls,
            camera,
            before: camera,
            panels: LevelPanels::new(level, stand_in),
            status: String::new(),
            map,
            file,
            unsaved: None,
            saved: String::new(),
            snap: Snap::default(),
            drag: None,
            held: None,
            aiming: false,
            actions: actions.clone(),
            world,
            play: None,
            play_asked: false,
            play_problem: String::new(),
            hush: false,
        };
        Some((editor, actions))
    }

    /// Plays the level from where the camera is. What was being carried or
    /// dragged is put down where it is.
    fn start_playing(&mut self) {
        self.play_asked = false;
        self.held = None;
        self.drag = None;
        self.play_problem.clear();
        // The game's own folder is the one its map is kept in.
        let folder = self.file.as_ref().and_then(|file| file.path().parent());
        let camera = self.camera;
        match Play::start(
            folder,
            &self.map,
            &self.world,
            &self.actions,
            camera.position,
            [camera.yaw, camera.pitch],
        ) {
            Ok(play) => self.play = Some(play),
            Err(problem) => {
                log::error!("{problem}");
                self.play_problem = problem;
            }
        }
    }

    /// Comes back from playing: the camera, what is chosen and the map are
    /// as they were, since playing never touched them.
    fn stop_playing(&mut self) {
        self.play = None;
        self.hush = true;
        // No frame is drawn part of the way from an older tick.
        self.before = self.camera;
    }

    /// Writes the map to its file now, and notes how that went.
    fn save(&mut self) {
        let Some(file) = &mut self.file else {
            self.unsaved = None;
            return;
        };
        // Something else saved the file while a change made here was
        // waiting: the editor's map is what is kept, and that is said.
        let replaced = file.saved_elsewhere();
        match file.save(&self.map) {
            Ok(()) => {
                self.unsaved = None;
                self.saved.clear();
                self.saved.push_str(if replaced {
                    log::warn!(
                        "{} was saved outside the editor while a change made in the editor \
                         was waiting to be saved. The editor's map was saved over it.",
                        file.path().display()
                    );
                    "Saved over a change made outside the editor while a change made here \
                     was waiting."
                } else {
                    "Saved."
                });
            }
            Err(error) => {
                let said = format!("{error} It is tried again every few seconds.");
                // Said once in the terminal, however many times it is tried.
                if self.saved != said {
                    log::error!("{said}");
                    self.saved = said;
                }
                self.unsaved = Some(-SAVE_AGAIN_AFTER);
            }
        }
    }

    /// Saves the map a moment after its last change, and takes in a save
    /// made by something else: a text editor, or a merge.
    fn keep_map(&mut self, step: f32) {
        if let Some(waited) = &mut self.unsaved {
            *waited += step;
            if *waited >= SAVE_AFTER {
                self.save();
            }
            return;
        }
        // With changes of our own waiting, ours are what is kept.
        let Some(change) = self.file.as_mut().and_then(MapFile::changed) else {
            return;
        };
        self.saved.clear();
        match change {
            Ok(map) => {
                self.map = map;
                self.saved
                    .push_str("Loaded again: it was saved outside the editor.");
            }
            Err(error) => {
                log::error!("{error}\nThe editor keeps the map it had.");
                // Writing to a `String` cannot fail.
                let _ = write!(
                    self.saved,
                    "{error} The editor keeps the map it had, and the next change made \
                     here saves over the file."
                );
            }
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

    /// Places, picks up, puts down and turns things as the keys and the
    /// gamepad ask this tick, where the camera looks.
    fn edit(&mut self, input: &InputFrame) {
        let controls = self.controls;
        let pressed = |action| input.just_pressed(action);
        let mut changed = false;
        if pressed(controls.snap) {
            self.snap = self.snap.next();
        }
        if pressed(controls.cancel) {
            if let Some(held) = self.held.take() {
                if let Some(entity) = self.map.get_mut(&held.id) {
                    changed |= entity.position != held.from;
                    entity.position = held.from;
                }
            } else {
                self.panels.placing = false;
            }
        }
        if pressed(controls.grab) {
            self.aiming = true;
            if self.held.take().is_none() {
                let id = self
                    .camera
                    .aimed_at(&self.map)
                    .or(self.panels.chosen_placed())
                    .map(str::to_owned);
                if let Some(entity) = id.and_then(|id| self.map.get(&id)) {
                    self.held = Some(Held {
                        id: entity.id.clone(),
                        from: entity.position,
                    });
                    self.panels.choose_placed(&entity.id);
                }
            }
        }
        if pressed(controls.place) {
            self.aiming = true;
            if self.held.take().is_none() {
                let (spot, facing) = self.camera.place_spot(0.0, self.snap);
                changed |= self.panels.place(&mut self.map, spot, facing);
            }
        }
        let turns = f32::from(
            i8::from(pressed(controls.turn_left)) - i8::from(pressed(controls.turn_right)),
        );
        if turns != 0.0
            && let Some(id) = self.panels.chosen_placed()
            && let Some(entity) = self.map.get_mut(id)
        {
            let facing = self
                .snap
                .degrees(entity.facing + turns * self.snap.turn(false), false);
            changed |= facing != entity.facing;
            entity.facing = facing;
        }
        // What is held goes where the camera looks, at the height it has.
        if let Some(held) = &self.held {
            if let Some(entity) = self.map.get_mut(&held.id) {
                let (spot, _) = self.camera.place_spot(entity.position[1], self.snap);
                changed |= spot != entity.position;
                entity.position = spot;
            } else {
                // Something else saved the map without it.
                self.held = None;
            }
        }
        if changed {
            self.unsaved = Some(0.0);
        }
    }
}

impl Game for Editor {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        if self.play.is_some() {
            // The map goes on being saved, and read again when something
            // else saves it: playing has its own copy.
            self.keep_map(step);
            if input.just_pressed(self.controls.back) {
                self.stop_playing();
            } else if let Some(problem) = self.play.as_mut().and_then(|play| play.tick(input, step))
            {
                log::error!("{problem}");
                self.play_problem = problem;
                self.stop_playing();
            }
            return;
        }
        if self.play_asked || input.just_pressed(self.controls.play) {
            self.start_playing();
            if self.play.is_some() {
                return;
            }
        }
        self.before = self.camera;
        let asked = self.read(input, step);
        self.camera.step(&asked, step);
        // A change made outside is taken in before anything is changed
        // here, so that a change made here is never lost to it.
        self.keep_map(step);
        self.edit(input);
    }

    fn frame(&mut self, frame: &mut Frame) {
        if let Some(play) = &mut self.play {
            // The game's own actions are live, and the mouse looks around.
            frame.use_action_set(ActionMap::GAMEPLAY);
            frame.show_grid(false);
            play.frame(frame);
            return;
        }
        frame.use_action_set(self.controls.set);
        frame.show_grid(true);
        if std::mem::take(&mut self.hush) {
            frame.sound(SoundAsk::StopAll);
        }
        let [x, y, z] = self.camera.position;
        self.status.clear();
        // Writing to a `String` cannot fail.
        let _ = write_speed(&mut self.status, self.camera.speed);
        let _ = write!(self.status, " m/s at {x:.1}, {y:.1}, {z:.1}");
        if let Some(held) = &self.held
            && let Some(entity) = self.map.get(&held.id)
        {
            let _ = write!(
                self.status,
                "    Carrying {}: F or X puts it down, Esc or B puts it back",
                placed_name(entity)
            );
        }
        let Self {
            panels,
            status,
            map,
            saved,
            camera,
            before,
            snap,
            drag,
            held,
            aiming,
            play_asked,
            play_problem,
            ..
        } = self;
        let alpha = frame.alpha();
        let mut changed = false;
        frame.panels(|ui| {
            let spot = camera.place_spot(0.0, *snap);
            let said = [status.as_str(), play_problem.as_str()];
            let asked = panels.show(ui, said, map, saved, spot, snap);
            changed |= asked.changed;
            *play_asked |= asked.play;
            if let Some((middle, reach)) = asked.fly_to {
                camera.fly_to(middle, reach);
                // No frame is drawn part of the way there.
                *before = *camera;
            }
            // Over the picture, in what the panels leave of the window, as
            // the camera of this frame sees it.
            let seen = camera.blended(before, alpha);
            let picture = ui.available_rect_before_wrap();
            let window = ui.ctx().content_rect();
            let size = [window.width(), window.height()];
            let pointer = ui.interact(
                picture,
                egui::Id::new("fossps picture"),
                egui::Sense::click_and_drag(),
            );
            let (keys, pressed_at, moving) = ui.input(|input| {
                (
                    Keys {
                        free: input.modifiers.command,
                        lift: input.modifiers.alt,
                    },
                    input.pointer.press_origin(),
                    input.pointer.is_moving(),
                )
            });
            let left = egui::PointerButton::Primary;
            // What is carried with keys or a gamepad is not dragged as well.
            if held.is_some() {
                *drag = None;
            } else if pointer.drag_started_by(left)
                && let Some(at) = pressed_at
            {
                let at = [at.x, at.y];
                *drag = match seen.hit(at, size, map, panels.chosen_placed()) {
                    Some(Hit::Tip(id)) => Some(Drag::Turn { id }),
                    Some(Hit::Mark(id)) => {
                        panels.choose_placed(&id);
                        let offset = place::drag_offset(&seen, at, size, map, &id);
                        Some(Drag::Move { id, offset })
                    }
                    None => None,
                };
            }
            if let Some(dragged) = drag.as_ref() {
                if pointer.dragged_by(left)
                    && let Some(at) = pointer.interact_pointer_pos()
                {
                    let at = [at.x, at.y];
                    changed |= place::drag_to(dragged, &seen, at, size, map, *snap, keys);
                } else if !pointer.drag_started_by(left) {
                    *drag = None;
                }
            }
            if pointer.hovered() && moving {
                *aiming = false;
            }
            // Where a click would place something, if clicks place.
            let mut spot = None;
            if panels.placing
                && drag.is_none()
                && held.is_none()
                && let Some(at) = pointer.hover_pos()
                && seen
                    .hit([at.x, at.y], size, map, panels.chosen_placed())
                    .is_none()
                && let Some([x, _, z]) = seen.pointed([at.x, at.y], size, 0.0)
            {
                spot = Some([snap.metres(x, keys.free), 0.0, snap.metres(z, keys.free)]);
            }
            if pointer.clicked_by(left)
                && held.is_none()
                && let Some(at) = pointer.interact_pointer_pos()
            {
                match seen.hit([at.x, at.y], size, map, panels.chosen_placed()) {
                    Some(Hit::Mark(id) | Hit::Tip(id)) => panels.choose_placed(&id),
                    None => {
                        if let Some(spot) = spot {
                            let facing = snap.degrees(seen.yaw.to_degrees(), keys.free);
                            changed |= panels.place(map, spot, facing);
                        }
                    }
                }
            }
            // With keys or a gamepad, the next thing goes where the camera
            // looks: that spot is ringed, unless something is carried there.
            if spot.is_none() && *aiming && held.is_none() {
                spot = Some(seen.place_spot(0.0, *snap).0);
            }
            let painter = ui.painter_at(picture);
            mark_placed(&painter, window, &seen, map, panels.chosen_placed());
            if let Some(spot) = spot
                && let Some(([x, y], _)) = seen.pictured(spot, size)
            {
                mark_spot(&painter, egui::pos2(x, y));
            }
        });
        if changed {
            self.unsaved = Some(0.0);
        }
        let seen = self.camera.blended(&self.before, alpha);
        frame.look_from(seen.position, seen.yaw, seen.pitch);
    }
}

impl Drop for Editor {
    /// A change made just before the window was closed is saved too.
    fn drop(&mut self) {
        if self.unsaved.is_some() {
            self.save();
        }
    }
}

/// Marks each thing placed in `map` in the picture, as `camera` sees it in
/// a `window` this size: a dot where it stands, a line the way it faces
/// and, close by, what it is called. The chosen one is ringed.
///
/// The marks are drawn over the level, walls included, so something placed
/// behind a wall can still be found.
fn mark_placed(
    painter: &egui::Painter,
    window: egui::Rect,
    camera: &FlyCamera,
    map: &Map,
    chosen: Option<&str>,
) {
    let size = [window.width(), window.height()];
    let spawn = egui::Color32::from_rgb(110, 220, 130);
    let other = egui::Color32::from_rgb(240, 200, 90);
    let dark = egui::Color32::from_black_alpha(200);
    for entity in map.entities() {
        let Some(([x, y], ahead)) = camera.pictured(entity.position, size) else {
            continue;
        };
        let at = egui::pos2(x, y);
        if !painter.clip_rect().expand(40.0).contains(at) {
            continue;
        }
        let color = if entity.kind == SPAWN { spawn } else { other };
        if let Some(([x, y], _)) = camera.pictured(place::tip(entity), size) {
            let to = egui::pos2(x, y);
            painter.line_segment([at, to], egui::Stroke::new(4.0, dark));
            painter.line_segment([at, to], egui::Stroke::new(2.0, color));
            // The tip of the chosen one's line is dragged to turn it.
            if chosen == Some(entity.id.as_str()) {
                painter.circle(
                    to,
                    TIP_SIZE,
                    egui::Color32::WHITE,
                    egui::Stroke::new(1.5, dark),
                );
            }
        }
        painter.circle(at, MARK_SIZE, color, egui::Stroke::new(1.5, dark));
        if chosen == Some(entity.id.as_str()) {
            painter.circle_stroke(
                at,
                MARK_SIZE + 4.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
            );
        }
        if ahead <= MARK_NAMED_WITHIN {
            let font = egui::FontId::proportional(13.0);
            let place = at + egui::vec2(MARK_SIZE + 6.0, 0.0);
            for (offset, ink) in [
                (egui::vec2(1.0, 1.0), dark),
                (egui::Vec2::ZERO, egui::Color32::WHITE),
            ] {
                painter.text(
                    place + offset,
                    egui::Align2::LEFT_CENTER,
                    placed_name(entity),
                    font.clone(),
                    ink,
                );
            }
        }
    }
}

/// Rings the spot at `at` in the picture, where the next thing goes.
fn mark_spot(painter: &egui::Painter, at: egui::Pos2) {
    let dark = egui::Color32::from_black_alpha(200);
    painter.circle_stroke(at, SPOT_SIZE, egui::Stroke::new(4.0, dark));
    painter.circle_stroke(at, SPOT_SIZE, egui::Stroke::new(2.0, egui::Color32::WHITE));
}

/// Opens a window on `level`, the level of the game called `name`, and lets
/// the person fly around it until the window is closed. With no level, the
/// engine's stand-in is shown.
///
/// `placed` is the game's map and the file it is kept in, as
/// [`MapFile::open`] gives them: what is placed in the editor is saved
/// there as it is changed. With `None`, things can be placed but nothing is
/// saved.
///
/// Call this from the main thread.
///
/// # Errors
///
/// Fails as [`run`](crate::run) does: if the window system cannot be
/// reached, the window cannot be created, or no graphics card here can draw
/// with Vulkan 1.3. The error's message says what to do about it.
pub fn run(
    name: &str,
    level: Option<fossps_assets::Scene>,
    placed: Option<(Map, MapFile)>,
) -> Result<(), AppError> {
    let mut config = Config {
        level,
        title: if name.is_empty() {
            "FOSSPS editor".to_owned()
        } else {
            format!("{name} | FOSSPS editor")
        },
        ..Config::default()
    };
    // Playing the level from here is done with the player's own controls,
    // as `fossps run` does it.
    if let Some(path) = crate::controls::file() {
        for problem in crate::controls::load(&path, &mut config.actions) {
            log::warn!("{problem}");
        }
    }
    let (game, rate) = (config.actions.clone(), config.tick_rate);
    let made = match &config.level {
        Some(level) => Editor::new(level, false, placed, game, rate),
        None => Editor::new(&crate::standin::scene(), true, placed, game, rate),
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
        let (editor, map) = Editor::new(level, true, None, ActionMap::default_fps(), 60).unwrap();
        let mut input = Input::new(map);
        input.set_active_set(editor.controls.set);
        (editor, input)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn every_action_is_on_keyboard_and_mouse_and_on_a_gamepad() {
        // Added to no game's actions, the editor's own are all there is.
        let (map, controls) = controls(ActionMap::new()).unwrap();
        assert_eq!(map.parity_gaps(), Vec::new());
        assert_eq!(map.conflicts(), Vec::new());
        assert_eq!(map.mouse_mode(controls.set), Some(MouseMode::Pointer));
        for action in map.actions() {
            // Coming back from playing is among the game's actions, which
            // are the live ones then; the rest are the editor's.
            let set = if action == controls.back {
                ActionMap::GAMEPLAY
            } else {
                controls.set
            };
            assert_eq!(map.set_of(action), Some(set), "{}", map.name(action));
            assert!(map.name(action).starts_with("editor_"));
        }
        // One key and one button, the same both ways.
        assert_eq!(map.bindings(controls.play), map.bindings(controls.back));
        assert_eq!(map.bindings(controls.play).len(), 2);

        // Added to a game's actions, they leave the game's as they were
        // and bring no gap: only Select now does two things while playing,
        // since the usual controls show the scoreboard with it.
        let game = ActionMap::default_fps();
        let (map, controls) = super::controls(game.clone()).unwrap();
        assert_eq!(map.parity_gaps(), Vec::new());
        for action in game.actions() {
            assert_eq!(map.action(game.name(action)), Ok(action));
            assert_eq!(map.bindings(action), game.bindings(action));
            assert_eq!(map.set_of(action), Some(ActionMap::GAMEPLAY));
        }
        assert_eq!(map.set_of(controls.back), Some(ActionMap::GAMEPLAY));
        assert_eq!(map.conflicts().len(), 1, "{:?}", map.conflicts());

        // A game that already has an action by one of these names is not
        // edited over: the editor says it cannot start.
        let mut taken = ActionMap::default_fps();
        taken.button("editor_play", ["F6", "DpadUp"]).unwrap();
        assert!(super::controls(taken).is_none());
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
            "which the editor never writes",
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

    /// An empty folder for one test.
    fn scratch(test: &str) -> std::path::PathBuf {
        let folder =
            std::env::temp_dir().join(format!("fossps-editor-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// An editor for the game in `folder`, on a small level.
    fn editor_in(folder: &std::path::Path) -> (Editor, Input) {
        let (file, map) = MapFile::open(folder);
        let (editor, actions) = Editor::new(
            &small_level(),
            false,
            Some((map.unwrap(), file)),
            ActionMap::default_fps(),
            60,
        )
        .unwrap();
        let mut input = Input::new(actions);
        input.set_active_set(editor.controls.set);
        (editor, input)
    }

    /// The left mouse button pressed and let go at `at`, over two frames
    /// starting at `time`. Gives what the second frame drew.
    fn click(
        editor: &mut Editor,
        context: &egui::Context,
        at: egui::Pos2,
        time: f64,
    ) -> egui::FullOutput {
        let down = vec![egui::Event::PointerMoved(at), button(at, true)];
        let _ = panel_frame(editor, context, time, down);
        panel_frame(editor, context, time + 0.05, vec![button(at, false)]).1
    }

    /// A click on the button or field called `what`.
    fn click_on(
        editor: &mut Editor,
        context: &egui::Context,
        what: &str,
        time: f64,
    ) -> egui::FullOutput {
        let at = editor.panels.spot(what);
        click(editor, context, at, time)
    }

    #[test]
    fn a_thing_is_placed_where_the_camera_looks_and_saved_a_moment_later() {
        let folder = scratch("place");
        let (mut editor, mut input) = editor_in(&folder);
        let file = folder.join(crate::map::MAP_FILE);
        let context = panel_context();
        let (_, output) = panel_frame(&mut editor, &context, 0.0, Vec::new());
        let shown = words(&output);
        for expected in ["map.toml", "Nothing placed yet", "Place a spawn here"] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }

        // The camera starts at 0, 5, 22 looking 0.2 radians down along -Z:
        // it looks at the ground 5 / tan(0.2) metres ahead, and the spawn
        // lands on the half-metre grid.
        let output = click_on(&mut editor, &context, "place", 1.0);
        let [placed] = editor.map.entities() else {
            panic!("{:?}", editor.map);
        };
        assert_eq!(placed.kind, "spawn");
        assert_eq!(placed.position, [0.0, 0.0, -2.5]);
        assert_eq!(placed.facing, 0.0);
        let id = placed.id.clone();
        // It is chosen, listed first and can be changed on the right.
        assert_eq!(editor.panels.chosen_placed(), Some(id.as_str()));
        assert_eq!(editor.panels.rows.len(), 5);
        let (_, output_after) = panel_frame(&mut editor, &context, 1.2, Vec::new());
        let shown = words(&output_after);
        for expected in [
            "1 placed: 1 spawn",
            "placed in map.toml",
            "Type",
            "Turned",
            id.as_str(),
            "Remove",
            "saved in map.toml as you go",
        ] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }
        drop(output);

        // Nothing is written at once: a moment after the last change.
        assert!(!file.exists());
        editor.tick(input.tick(), 0.1);
        assert!(!file.exists());
        editor.tick(input.tick(), SAVE_AFTER);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains(&format!(
                "[[entity]]\nid = \"{id}\"\ntype = \"spawn\"\nposition = [0.0, 0.0, -2.5]\nfacing = 0.0\n"
            )),
            "{text}"
        );
        let (_, output) = panel_frame(&mut editor, &context, 2.0, Vec::new());
        assert!(words(&output).contains("Saved."));

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn what_is_placed_can_be_changed_and_removed_and_each_change_is_saved() {
        let folder = scratch("change");
        let (mut editor, mut input) = editor_in(&folder);
        let file = folder.join(crate::map::MAP_FILE);
        let context = panel_context();
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        let _ = click_on(&mut editor, &context, "place", 1.0);
        let _ = panel_frame(&mut editor, &context, 1.2, Vec::new());
        let id = editor.map.entities()[0].id.clone();

        // Typing in the type field changes the type; the click puts the
        // cursor at the end of what is there.
        let _ = click_on(&mut editor, &context, "type", 2.0);
        let typed = vec![egui::Event::Text("er".to_owned())];
        let _ = panel_frame(&mut editor, &context, 2.2, typed);
        assert_eq!(editor.map.get(&id).unwrap().kind, "spawner");
        // A name, too.
        let _ = click_on(&mut editor, &context, "name", 3.0);
        let typed = vec![egui::Event::Text("Red base".to_owned())];
        let (_, output) = panel_frame(&mut editor, &context, 3.2, typed);
        assert_eq!(editor.map.get(&id).unwrap().name, "Red base");
        let (_, output_after) = panel_frame(&mut editor, &context, 3.3, Vec::new());
        assert!(words(&output_after).contains("Place a spawner here"));
        drop(output);

        // Dragging a number moves it: 0.05 m for each point dragged.
        let from = editor.panels.spot("x");
        let before = editor.map.get(&id).unwrap().position;
        let _ = panel_frame(
            &mut editor,
            &context,
            4.0,
            vec![egui::Event::PointerMoved(from), button(from, true)],
        );
        let mut time = 4.05;
        for step in 1..=4_u8 {
            let at = from + egui::vec2(f32::from(step) * 10.0, 0.0);
            let _ = panel_frame(
                &mut editor,
                &context,
                time,
                vec![egui::Event::PointerMoved(at)],
            );
            time += 0.05;
        }
        let to = from + egui::vec2(40.0, 0.0);
        let _ = panel_frame(&mut editor, &context, time, vec![button(to, false)]);
        let after = editor.map.get(&id).unwrap().position;
        assert!(
            (1.0..=2.5).contains(&(after[0] - before[0])),
            "{before:?} to {after:?}"
        );
        assert_eq!(after[1..], before[1..]);

        // All of it is in the file a moment later, once.
        editor.tick(input.tick(), SAVE_AFTER + 0.1);
        let saved = Map::load(&folder).unwrap();
        let read = saved.get(&id).unwrap();
        assert_eq!(
            (read.kind.as_str(), read.name.as_str()),
            ("spawner", "Red base")
        );
        assert!((read.position[0] - after[0]).abs() < 0.001);

        // An empty type is not a type: the entity keeps the one it had.
        let _ = click_on(&mut editor, &context, "type", 6.0);
        let mut time = 6.2;
        for _ in 0.."spawner".len() {
            let key = |pressed| egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = panel_frame(&mut editor, &context, time, vec![key(true), key(false)]);
            time += 0.05;
        }
        assert_eq!(editor.map.get(&id).unwrap().kind, "s");
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = panel_frame(&mut editor, &context, time, vec![key(true), key(false)]);
        assert_eq!(editor.map.get(&id).unwrap().kind, "s", "empty is not taken");
        let typed = vec![egui::Event::Text("zone".to_owned())];
        let _ = panel_frame(&mut editor, &context, time + 0.1, typed);
        assert_eq!(editor.map.get(&id).unwrap().kind, "zone");
        // Nor is one that ends with a space: "zone " would not be a zone.
        let typed = vec![egui::Event::Text(" ".to_owned())];
        let _ = panel_frame(&mut editor, &context, time + 0.2, typed);
        assert_eq!(editor.map.get(&id).unwrap().kind, "zone");
        let typed = vec![egui::Event::Text("2".to_owned())];
        let _ = panel_frame(&mut editor, &context, time + 0.3, typed);
        assert_eq!(editor.map.get(&id).unwrap().kind, "zone 2");

        // Removing takes it out of the map, the list and the file.
        let _ = click_on(&mut editor, &context, "remove", 9.0);
        assert!(editor.map.is_empty());
        assert_eq!(editor.panels.chosen_placed(), None);
        let (_, output) = panel_frame(&mut editor, &context, 9.3, Vec::new());
        assert!(words(&output).contains("Nothing chosen"));
        assert!(!Map::load(&folder).unwrap().is_empty(), "not saved yet");
        // Closing the window saves what was changed just before.
        drop(editor);
        assert!(Map::load(&folder).unwrap().is_empty());
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("version = 1")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_map_saved_outside_the_editor_shows_up_and_an_open_edit_wins() {
        use std::time::{Duration, Instant};

        let folder = scratch("outside");
        let file = folder.join(crate::map::MAP_FILE);
        let written = "version = 1\n[[entity]]\nid = \"flag\"\ntype = \"objective\"\n\
                       name = \"The flag\"\nposition = [2, 0, 3]\n[entity.properties]\nteam = \"red\"\n";
        std::fs::write(&file, written).unwrap();
        let (mut editor, mut input) = editor_in(&folder);
        let context = panel_context();
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        // What the file holds is listed first, and its properties are shown.
        let row = editor.panels.rows[0].1.center();
        let _ = click(&mut editor, &context, row, 1.0);
        assert_eq!(editor.panels.chosen_placed(), Some("flag"));
        let (_, output) = panel_frame(&mut editor, &context, 1.2, Vec::new());
        let shown = words(&output);
        for expected in [
            "The flag",
            "objective",
            "team = \"red\"",
            "Place a objective here",
        ] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }

        // Someone saves the file with the flag moved and a spawn added.
        let moved = written.replace("[2, 0, 3]", "[9, 0, 3]")
            + "[[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [0, 0, 0]\n";
        std::fs::write(&file, &moved).unwrap();
        let give_up = Instant::now() + Duration::from_secs(20);
        while editor.map.entities().len() < 2 && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(20));
            editor.tick(input.tick(), STEP);
        }
        assert_eq!(editor.map.get("flag").unwrap().position, [9.0, 0.0, 3.0]);
        assert_eq!(editor.panels.chosen_placed(), Some("flag"), "still chosen");
        let (_, output) = panel_frame(&mut editor, &context, 2.0, Vec::new());
        assert!(words(&output).contains("saved outside the editor"));

        // A save with a mistake in it is said and changes nothing here.
        std::fs::write(&file, "version = 1\nsky = 1\n").unwrap();
        let give_up = Instant::now() + Duration::from_secs(20);
        while !editor.saved.contains("sky") && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(20));
            editor.tick(input.tick(), STEP);
        }
        assert!(
            editor.saved.contains("line 2: \"sky\" is not something"),
            "{}",
            editor.saved
        );
        assert_eq!(editor.map.entities().len(), 2);

        // Saved without what was chosen, nothing is chosen any more.
        std::fs::write(&file, "version = 1\n").unwrap();
        let give_up = Instant::now() + Duration::from_secs(20);
        while !editor.map.is_empty() && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(20));
            editor.tick(input.tick(), STEP);
        }
        let _ = panel_frame(&mut editor, &context, 3.0, Vec::new());
        assert_eq!(editor.panels.chosen_placed(), None);

        // A save from outside that lands while a change made here is
        // waiting is replaced by the editor's map, and that is said.
        let _ = click_on(&mut editor, &context, "place", 3.5);
        std::fs::write(&file, written).unwrap();
        editor.tick(input.tick(), SAVE_AFTER + 0.1);
        assert!(
            editor.saved.starts_with("Saved over a change made outside"),
            "{}",
            editor.saved
        );
        assert_eq!(Map::load(&folder).unwrap(), editor.map);
        assert!(editor.map.get("flag").is_none());
        // The next save has nothing to say about it.
        let _ = click_on(&mut editor, &context, "place", 3.8);
        editor.tick(input.tick(), SAVE_AFTER + 0.1);
        assert_eq!(editor.saved, "Saved.");

        // A folder that cannot be written to says so in the panel, and the
        // change is kept to be tried again.
        let (lost, _) = MapFile::open(&folder.join("gone"));
        editor.file = Some(lost);
        let _ = click_on(&mut editor, &context, "place", 4.0);
        editor.tick(input.tick(), SAVE_AFTER + 0.1);
        assert!(
            editor.saved.contains("could not be saved"),
            "{}",
            editor.saved
        );
        assert!(editor.unsaved.is_some_and(|waited| waited < 0.0));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A key or button pressed for one tick.
    fn tap(editor: &mut Editor, input: &mut Input, what: impl Into<fossps_input::Button>) {
        let what = what.into();
        input.press(what);
        editor.tick(input.tick(), STEP);
        input.release(what);
        editor.tick(input.tick(), STEP);
    }

    #[test]
    fn keys_and_a_gamepad_place_carry_turn_and_put_back_where_the_camera_looks() {
        let folder = scratch("carry");
        let (mut editor, mut input) = editor_in(&folder);
        let place = |editor: &Editor, id: &str| editor.map.get(id).unwrap().position;

        // A places a spawn where the camera looks, on the grid, and it is
        // chosen.
        tap(&mut editor, &mut input, GamepadButton::GamepadSouth);
        let id = editor.map.entities()[0].id.clone();
        assert_eq!(place(&editor, &id), [0.0, 0.0, -2.5]);
        assert_eq!(editor.panels.chosen_placed(), Some(id.as_str()));
        assert!(editor.aiming, "the spot the camera looks at is ringed");

        // RB turns it a step to the right; Z, twice, a step to the left.
        tap(&mut editor, &mut input, GamepadButton::RightBumper);
        assert_eq!(editor.map.get(&id).unwrap().facing, -15.0);
        tap(&mut editor, &mut input, Key::Z);
        tap(&mut editor, &mut input, Key::Z);
        assert_eq!(editor.map.get(&id).unwrap().facing, 15.0);
        tap(&mut editor, &mut input, GamepadButton::LeftBumper);
        tap(&mut editor, &mut input, Key::C);
        assert_eq!(editor.map.get(&id).unwrap().facing, 15.0);

        // Y and G change what things snap to, round and back.
        tap(&mut editor, &mut input, GamepadButton::GamepadNorth);
        assert_eq!(editor.snap.words(), "Snap: 1 m, 15°");
        for _ in 0..4 {
            tap(&mut editor, &mut input, Key::G);
        }
        assert_eq!(editor.snap, Snap::default());

        // X picks up what the camera looks at, and it goes where the camera
        // looks from then on.
        tap(&mut editor, &mut input, GamepadButton::GamepadWest);
        assert_eq!(
            editor.held.as_ref().map(|held| held.id.as_str()),
            Some(&*id)
        );
        assert_eq!(place(&editor, &id), [0.0, 0.0, -2.5]);
        editor.camera.position[0] = 3.0;
        editor.tick(input.tick(), STEP);
        assert_eq!(place(&editor, &id), [3.0, 0.0, -2.5]);
        let mut frame = Frame::new(1.0);
        editor.frame(&mut frame);
        assert!(
            editor.status.contains("Carrying spawn"),
            "{}",
            editor.status
        );
        // B puts it back where it was.
        tap(&mut editor, &mut input, GamepadButton::GamepadEast);
        assert_eq!(editor.held, None);
        assert_eq!(place(&editor, &id), [0.0, 0.0, -2.5]);

        // The camera now looks well to the side of it: F brings what is
        // chosen, and F again puts it down there.
        assert_eq!(editor.camera.aimed_at(&editor.map), None);
        tap(&mut editor, &mut input, Key::F);
        assert_eq!(place(&editor, &id), [3.0, 0.0, -2.5]);
        editor.camera.position[0] = 2.0;
        editor.tick(input.tick(), STEP);
        tap(&mut editor, &mut input, Key::F);
        assert_eq!(editor.held, None);
        editor.camera.position[0] = -6.0;
        editor.tick(input.tick(), STEP);
        assert_eq!(place(&editor, &id), [2.0, 0.0, -2.5]);

        // Something carried up high stays as high, and P puts it down too
        // without placing another.
        editor.map.get_mut(&id).unwrap().position[1] = 3.0;
        tap(&mut editor, &mut input, Key::F);
        assert_eq!(place(&editor, &id)[1], 3.0);
        tap(&mut editor, &mut input, Key::P);
        assert_eq!(editor.held, None);
        assert_eq!(editor.map.entities().len(), 1);
        // With nothing carried, P places another, of the same type.
        tap(&mut editor, &mut input, Key::P);
        assert_eq!(editor.map.entities().len(), 2);

        // All of it is saved a moment after the last change.
        editor.tick(input.tick(), SAVE_AFTER + 0.1);
        assert_eq!(Map::load(&folder).unwrap(), editor.map);

        // Something carried that a save from outside took away is let go.
        tap(&mut editor, &mut input, Key::F);
        let carried = editor.held.clone().unwrap().id;
        editor.map.remove(&carried);
        editor.tick(input.tick(), STEP);
        assert_eq!(editor.held, None);

        // Esc and B stop placing by clicking.
        editor.panels.placing = true;
        tap(&mut editor, &mut input, Key::Escape);
        assert!(!editor.panels.placing);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The left mouse button pressed at `from`, moved to `to` and let go,
    /// over frames starting at `time`, with `modifiers` held.
    fn drag(
        editor: &mut Editor,
        context: &egui::Context,
        [from, to]: [egui::Pos2; 2],
        time: f64,
        modifiers: egui::Modifiers,
    ) {
        let frame = |editor: &mut Editor, time: f64, events: Vec<egui::Event>| {
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
            frame.take_panels().unwrap().textures_delta.clear();
        };
        frame(
            editor,
            time,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(from),
                button(from, true),
            ],
        );
        for step in 1..=5_u8 {
            let at = from + (to - from) * (f32::from(step) / 5.0);
            frame(
                editor,
                time + 0.05 * f64::from(step),
                vec![egui::Event::PointerMoved(at)],
            );
        }
        frame(
            editor,
            time + 0.3,
            vec![
                button(to, false),
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            ],
        );
    }

    #[test]
    fn a_click_places_and_a_drag_moves_lifts_and_turns_in_the_picture() {
        let folder = scratch("drag");
        let (mut editor, mut input) = editor_in(&folder);
        let context = panel_context();
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        let (_, output) = panel_frame(&mut editor, &context, 0.1, Vec::new());
        let shown = words(&output);
        for expected in [
            "Click in the picture to place",
            "Snap: 0.5 m, 15°",
            "Move: drag a mark",
            "X carries what you look at",
        ] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }
        // Where a point of the level is in the picture.
        let at = |editor: &Editor, point: [f32; 3]| {
            let ([x, y], _) = editor.camera.pictured(point, [1280.0, 720.0]).unwrap();
            egui::pos2(x, y)
        };
        let none = egui::Modifiers::NONE;

        // A click on the ground does nothing until clicks place.
        let ground = at(&editor, [2.1, 0.0, 1.2]);
        let _ = click(&mut editor, &context, ground, 0.5);
        assert!(editor.map.is_empty());
        let _ = click_on(&mut editor, &context, "placing", 1.0);
        assert!(editor.panels.placing);
        let _ = click(&mut editor, &context, ground, 1.5);
        let [first] = editor.map.entities() else {
            panic!("{:?}", editor.map);
        };
        assert_eq!((first.position, first.facing), ([2.0, 0.0, 1.0], 0.0));
        let first = first.id.clone();
        assert_eq!(editor.panels.chosen_placed(), Some(first.as_str()));
        let elsewhere = at(&editor, [-3.0, 0.0, 4.2]);
        let _ = click(&mut editor, &context, elsewhere, 2.0);
        assert_eq!(editor.map.entities().len(), 2);
        let second = editor.panels.chosen_placed().unwrap().to_owned();
        assert_eq!(editor.map.get(&second).unwrap().position, [-3.0, 0.0, 4.0]);
        // A click on a mark chooses it, and places nothing on top of it.
        let mark = at(&editor, [2.0, 0.0, 1.0]);
        let _ = click(&mut editor, &context, mark, 2.5);
        assert_eq!(editor.map.entities().len(), 2);
        assert_eq!(editor.panels.chosen_placed(), Some(first.as_str()));
        // The sky is nowhere to put anything.
        let _ = click(&mut editor, &context, egui::pos2(640.0, 30.0), 3.0);
        assert_eq!(editor.map.entities().len(), 2);
        // The snap button changes the grid: the next lands on whole metres.
        let _ = click_on(&mut editor, &context, "snap", 3.5);
        assert_eq!(editor.snap.words(), "Snap: 1 m, 15°");
        let third = at(&editor, [-5.4, 0.0, -2.6]);
        let _ = click(&mut editor, &context, third, 4.0);
        let third = editor.panels.chosen_placed().unwrap().to_owned();
        assert_eq!(editor.map.get(&third).unwrap().position, [-5.0, 0.0, -3.0]);
        editor.snap = Snap::default();
        // Esc turns placing off again, as the button does.
        tap(&mut editor, &mut input, Key::Escape);
        assert!(!editor.panels.placing);
        let _ = click(&mut editor, &context, ground + egui::vec2(60.0, 0.0), 4.5);
        assert_eq!(editor.map.entities().len(), 3);

        // The second mark is dragged over the ground, which chooses it: let
        // go where 5.2, 0, -2.9 is, it stands on the grid beside that.
        let from = at(&editor, [-3.0, 0.0, 4.0]);
        let to = at(&editor, [5.2, 0.0, -2.9]);
        drag(&mut editor, &context, [from, to], 5.0, none);
        assert_eq!(editor.panels.chosen_placed(), Some(second.as_str()));
        assert_eq!(editor.map.get(&second).unwrap().position, [5.0, 0.0, -3.0]);
        assert_eq!(editor.drag, None, "let go, nothing is dragged");
        assert_eq!(editor.map.get(&first).unwrap().position, [2.0, 0.0, 1.0]);
        // With Ctrl held it leaves the grid.
        let from = at(&editor, [5.0, 0.0, -3.0]);
        let to = at(&editor, [5.33, 0.0, -3.0]);
        let ctrl = egui::Modifiers::COMMAND;
        drag(&mut editor, &context, [from, to], 6.0, ctrl);
        let [x, y, z] = editor.map.get(&second).unwrap().position;
        assert!((x - 5.33).abs() < 0.03 && y == 0.0 && (z + 3.0).abs() < 0.03);
        // With Alt held it goes up and down, and not along the ground.
        let before = editor.map.get(&second).unwrap().position;
        let from = at(&editor, before);
        let to = at(&editor, [before[0], 2.1, before[2]]);
        drag(&mut editor, &context, [from, to], 7.0, egui::Modifiers::ALT);
        let after = editor.map.get(&second).unwrap().position;
        assert_eq!(after, [before[0], 2.0, before[2]]);

        // The tip of the chosen one's line, a metre ahead of it, turns it:
        // dragged round to its right (+X), it faces a quarter to the right.
        let from = at(&editor, [after[0], 2.0, after[2] - 1.0]);
        let to = at(&editor, [after[0] + 2.0, 2.0, after[2] + 0.2]);
        drag(&mut editor, &context, [from, to], 8.0, none);
        assert_eq!(editor.map.get(&second).unwrap().facing, -90.0);
        assert_eq!(editor.map.get(&second).unwrap().position, after);
        // The tip of one that is not chosen is not a handle: pressed
        // there, so near its mark, the drag moves it and turns nothing.
        let from = at(&editor, [2.0, 0.0, 0.0]);
        let to = at(&editor, [3.5, 0.0, 1.0]);
        drag(&mut editor, &context, [from, to], 9.0, none);
        assert_eq!(editor.map.get(&first).unwrap().facing, 0.0);
        assert_eq!(editor.map.get(&first).unwrap().position, [3.5, 0.0, 2.0]);
        assert_eq!(editor.panels.chosen_placed(), Some(first.as_str()));
        // A drag that starts on a panel moves nothing in the picture.
        let row = editor.panels.rows[0].1.center();
        let to = at(&editor, [-3.0, 0.0, 4.0]);
        drag(&mut editor, &context, [row, to], 10.0, none);
        assert_eq!(editor.map.get(&first).unwrap().position, [3.5, 0.0, 2.0]);
        // Nor does one that starts on bare ground.
        let from = at(&editor, [-3.0, 0.0, 4.0]);
        let to = at(&editor, [3.5, 0.0, 2.0]);
        drag(&mut editor, &context, [from, to], 11.0, none);
        assert_eq!(editor.map.get(&first).unwrap().position, [3.5, 0.0, 2.0]);

        // Everything dragged is in the file a moment later.
        assert!(editor.unsaved.is_some());
        editor.tick(input.tick(), SAVE_AFTER + 0.1);
        assert_eq!(Map::load(&folder).unwrap(), editor.map);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_mark_is_drawn_where_the_camera_sees_what_is_placed() {
        let camera = FlyCamera {
            position: [0.0, 2.0, 10.0],
            yaw: 0.0,
            pitch: 0.0,
            speed: 8.0,
        };
        let size = [1280.0, 720.0];
        // Straight ahead is the middle of the picture.
        let (at, ahead) = camera.pictured([0.0, 2.0, 0.0], size).unwrap();
        assert!(close(at[0], 640.0) && close(at[1], 360.0), "{at:?}");
        assert!(close(ahead, 10.0));
        // 74 degrees are seen from bottom to top: 37 up is the top edge.
        let rise = 10.0 * 37.0_f32.to_radians().tan();
        let (at, _) = camera.pictured([0.0, 2.0 + rise, 0.0], size).unwrap();
        assert!(close(at[0], 640.0) && at[1].abs() < 0.01, "{at:?}");
        // To the right in the world is to the right in the picture, by as
        // many pixels as up is.
        let (at, _) = camera.pictured([rise, 2.0, 0.0], size).unwrap();
        assert!(
            (at[0] - 1000.0).abs() < 0.01 && close(at[1], 360.0),
            "{at:?}"
        );
        // Behind the camera is not in the picture.
        assert_eq!(camera.pictured([0.0, 2.0, 20.0], size), None);
        // Turned a quarter to the left, -X is ahead.
        let turned = FlyCamera {
            yaw: TAU / 4.0,
            pitch: -0.3,
            ..camera
        };
        let (_, ahead) = turned.pictured([-8.0, 2.0, 10.0], size).unwrap();
        assert!(close(ahead, 8.0 * 0.3_f32.cos()), "{ahead}");
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

    /// What is placed is drawn for real, with no window, and the picture is
    /// read back: a green dot where a spawn point stands, a yellow one for
    /// anything else, a white ring round the chosen one, and nothing over
    /// the panels.
    #[test]
    fn what_is_placed_is_marked_in_the_picture_and_not_over_the_panels() {
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
        let level = crate::level::from_bytes("level.glb", crate::scaffold::STARTER_LEVEL).unwrap();
        headless.set_scene(&level).unwrap();
        headless.set_grid(true);

        let map = Map::parse(
            "map.toml",
            "version = 1\n\
             [[entity]]\nid = \"a\"\ntype = \"spawn\"\nname = \"Red spawn\"\n\
             position = [-3, 0, -10]\nfacing = 180\n\
             [[entity]]\nid = \"b\"\ntype = \"spawn\"\nposition = [3, 0, 10]\n\
             [[entity]]\nid = \"c\"\ntype = \"zone\"\nname = \"Middle\"\nposition = [0, 0, 0]\nfacing = 90\n\
             [[entity]]\nid = \"d\"\ntype = \"spawn\"\nposition = [-12, 0, 10]\n",
        )
        .unwrap();
        let folder = scratch("marks");
        let (file, _) = MapFile::open(&folder);
        let (mut editor, mut input) = Editor::new(
            &level,
            false,
            Some((map, file)),
            ActionMap::default_fps(),
            60,
        )
        .unwrap();
        let mut input = {
            let mut made = Input::new(std::mem::take(&mut input));
            made.set_active_set(editor.controls.set);
            made
        };
        editor.tick(input.tick(), STEP);
        let context = panel_context();
        #[allow(clippy::cast_precision_loss)]
        let size = egui::vec2(SCREEN[0] as f32, SCREEN[1] as f32);
        let draw = |editor: &mut Editor, headless: &mut Headless, time: f64, events| {
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
            let output = frame.take_panels().unwrap();
            headless.set_panels(Panels {
                shapes: context.tessellate(output.shapes, output.pixels_per_point),
                textures: output.textures_delta,
                pixels_per_point: output.pixels_per_point,
            });
            let view = frame.view().unwrap();
            let camera = Camera {
                position: view.position,
                yaw: view.yaw,
                pitch: view.pitch,
                ..Camera::default()
            };
            headless.screenshot(&camera).unwrap()
        };
        let _ = draw(&mut editor, &mut headless, 0.0, Vec::new());
        // The zone in the middle is chosen: the third row of the list.
        let row = editor.panels.rows[2].1.center();
        let down = vec![egui::Event::PointerMoved(row), button(row, true)];
        let _ = draw(&mut editor, &mut headless, 1.0, down);
        let _ = draw(&mut editor, &mut headless, 1.05, vec![button(row, false)]);
        assert_eq!(editor.panels.chosen_placed(), Some("c"));
        let shown = draw(&mut editor, &mut headless, 1.2, Vec::new());
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            std::fs::write(std::path::Path::new(&folder).join("placed.rgba"), &shown).unwrap();
        }

        let pixel = |x: f32, y: f32| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let at = (y.round() as usize * SCREEN[0] as usize + x.round() as usize) * 4;
            [shown[at], shown[at + 1], shown[at + 2]].map(i32::from)
        };
        let green = |[red, green, blue]: [i32; 3]| green > red + 60 && green > blue + 60;
        let yellow = |[red, green, blue]: [i32; 3]| red > blue + 90 && green > blue + 60;
        let size = [size.x, size.y];
        // Each spawn point's dot is green where the camera sees it stand.
        for id in ["a", "b"] {
            let place = editor.map.get(id).unwrap().position;
            let ([x, y], _) = editor.camera.pictured(place, size).unwrap();
            assert!(green(pixel(x, y)), "{id} at {x}, {y}: {:?}", pixel(x, y));
        }
        // The zone's is yellow, with the white ring of what is chosen
        // round it: lighter than the floor just outside the ring.
        let ([x, y], _) = editor.camera.pictured([0.0; 3], size).unwrap();
        assert!(yellow(pixel(x, y)), "{:?}", pixel(x, y));
        let light = |color: [i32; 3]| color.iter().sum::<i32>();
        let (ring, outside) = (pixel(x, y - MARK_SIZE - 4.0), pixel(x, y - MARK_SIZE - 9.0));
        assert!(
            ring.iter().all(|&part| part > 200) && light(ring) > light(outside) + 60,
            "{ring:?} beside {outside:?}"
        );
        // The tip of its line, a metre to its left as it is turned a
        // quarter that way, is the white handle that turns it.
        let ([x, y], _) = editor.camera.pictured([-1.0, 0.0, 0.0], size).unwrap();
        assert!(
            pixel(x, y).iter().all(|&part| part > 200),
            "{:?}",
            pixel(x, y)
        );
        // The tip of one that is not chosen is only the end of a line.
        let ([x, y], _) = editor.camera.pictured([3.0, 0.0, 9.0], size).unwrap();
        assert!(green(pixel(x, y - 1.0)), "{:?}", pixel(x, y - 1.0));
        // A spawn point that is not chosen has no ring.
        let place = editor.map.get("b").unwrap().position;
        let ([x, y], _) = editor.camera.pictured(place, size).unwrap();
        let (ring, outside) = (pixel(x, y - MARK_SIZE - 4.0), pixel(x, y - MARK_SIZE - 9.0));
        assert!(
            (light(ring) - light(outside)).abs() < 30,
            "{ring:?} beside {outside:?}"
        );
        // The spawn off to the left would be drawn over the list: it is
        // not, and nothing green is anywhere near where it would be.
        let ([x, y], _) = editor.camera.pictured([-12.0, 0.0, 10.0], size).unwrap();
        assert!(
            (20.0..240.0).contains(&x) && (20.0..600.0).contains(&y),
            "{x}, {y}"
        );
        for across in -8_i8..=8 {
            for down in -8_i8..=8 {
                let seen = pixel(x + f32::from(across), y + f32::from(down));
                assert!(!green(seen), "{seen:?}");
            }
        }

        // Placing with keys or a gamepad rings the spot the camera looks
        // at, where the next thing goes: the floor there was not white.
        let (spot, _) = editor.camera.place_spot(0.0, editor.snap);
        let ([x, y], _) = editor.camera.pictured(spot, size).unwrap();
        // Looked for at its top (the chosen zone's ring is just below it),
        // a pixel either way.
        editor.aiming = true;
        let ringed = draw(&mut editor, &mut headless, 1.4, Vec::new());
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            std::fs::write(std::path::Path::new(&folder).join("aiming.rgba"), &ringed).unwrap();
        }
        let lightest = |picture: &[u8]| {
            (-2_i8..=2)
                .map(|down| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let at = ((y - SPOT_SIZE + f32::from(down)).round() as usize
                        * SCREEN[0] as usize
                        + x.round() as usize)
                        * 4;
                    picture[at..at + 3]
                        .iter()
                        .map(|&part| i32::from(part))
                        .sum::<i32>()
                })
                .max()
                .unwrap_or(0)
        };
        let (before, after) = (lightest(&shown), lightest(&ringed));
        assert!(after > before + 100 && after > 700, "{before} to {after}");
        let _ = std::fs::remove_dir_all(&folder);
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
            ("how to fly", 4..1276, 644..716),
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

    /// One spawn point, at the far end of the starter level.
    const ONE_SPAWN: &str =
        "version = 1\n[[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [-3, 0, -10]\n";

    /// An editor for a game that can be played: a folder with a script (an
    /// empty one) and `map` as its map file, on the starter level.
    fn game_in(test: &str, map: &str) -> (std::path::PathBuf, Editor, Input) {
        let folder = scratch(test);
        std::fs::write(folder.join(fossps_script::ENTRY_SCRIPT), "").unwrap();
        if !map.is_empty() {
            std::fs::write(folder.join(crate::map::MAP_FILE), map).unwrap();
        }
        let level = crate::level::from_bytes("level.glb", crate::scaffold::STARTER_LEVEL).unwrap();
        let (file, map) = MapFile::open(&folder);
        let placed = Some((map.unwrap(), file));
        let (editor, actions) =
            Editor::new(&level, false, placed, ActionMap::default_fps(), 60).unwrap();
        let mut input = Input::new(actions);
        input.set_active_set(editor.controls.set);
        (folder, editor, input)
    }

    /// One tick and the frame after it, as the window does them: the
    /// action set the frame asks for is the live one from then on.
    fn tick_and_frame(editor: &mut Editor, input: &mut Input) -> Frame {
        editor.tick(input.tick(), STEP);
        let mut frame = Frame::new(1.0);
        editor.frame(&mut frame);
        if let Some(set) = frame.action_set() {
            input.set_active_set(set);
        }
        frame
    }

    #[test]
    fn one_key_plays_from_where_the_camera_is_and_the_same_key_comes_back() {
        use fossps_input::Button;

        for toggle in [
            Button::from(Key::F5),
            Button::from(GamepadButton::GamepadSelect),
        ] {
            let (folder, mut editor, mut input) = game_in("play", ONE_SPAWN);
            // Over the clear middle of the starter level, 4 m up, looking
            // down and off to the left.
            editor.camera.position = [1.0, 4.0, 2.0];
            editor.camera.yaw = 0.7;
            editor.camera.pitch = -0.3;
            let _ = tick_and_frame(&mut editor, &mut input);
            // Something placed a moment ago is not saved yet.
            tap(&mut editor, &mut input, Key::P);
            assert!(editor.unsaved.is_some());
            assert_eq!(editor.map.entities().len(), 2);
            let (camera, map) = (editor.camera, editor.map.clone());
            let chosen = editor.panels.chosen_placed().map(str::to_owned);
            assert!(chosen.is_some());

            input.press(toggle);
            let frame = tick_and_frame(&mut editor, &mut input);
            input.release(toggle);
            let play = editor.play.as_ref().expect("the level is being played");
            assert!(play.started_here());
            // What is played has the map as the editor holds it, saved or not.
            assert_eq!(play.map(), &map);
            // The game's own actions are live, the grid is gone, and the
            // first frame is seen from just where the camera was.
            assert_eq!(frame.action_set(), Some(ActionMap::GAMEPLAY));
            assert_eq!(frame.grid(), Some(false));
            let view = frame.view().unwrap();
            for axis in 0..3 {
                assert!(
                    close(view.position[axis], camera.position[axis]),
                    "{view:?}"
                );
            }
            assert!(close(view.yaw, 0.7) && close(view.pitch, -0.3), "{view:?}");

            // The player drops to the floor and stands there, eyes 1.6 m up.
            for _ in 0..120 {
                let _ = tick_and_frame(&mut editor, &mut input);
            }
            let eye = editor.play.as_ref().unwrap().eye();
            assert!(close(eye[0], 1.0) && close(eye[2], 2.0), "{eye:?}");
            assert!((eye[1] - 1.6).abs() < 0.05, "{eye:?}");
            // W walks the way the camera faced: to the left of -Z.
            input.press(Key::W);
            for _ in 0..60 {
                let _ = tick_and_frame(&mut editor, &mut input);
            }
            input.release(Key::W);
            let play = editor.play.as_ref().unwrap();
            let eye = play.eye();
            assert!(eye[0] < -1.0 && eye[2] < -0.5, "{eye:?}");
            assert!(close(play.facing()[0], 0.7), "{:?}", play.facing());
            // The editor meanwhile: its camera stood still, and the change
            // made before playing was saved as ever.
            assert_eq!(editor.camera, camera);
            assert_eq!(editor.map, map);
            assert!(editor.unsaved.is_none());
            assert_eq!(MapFile::open(&folder).1.unwrap(), map);

            // The same key or button again: back to editing, with the
            // camera, the map and what was chosen as they were.
            input.press(toggle);
            let mut frame = tick_and_frame(&mut editor, &mut input);
            input.release(toggle);
            assert!(editor.play.is_none());
            assert_eq!(frame.action_set(), Some(editor.controls.set));
            assert_eq!(frame.grid(), Some(true));
            assert_eq!(frame.view().unwrap().position, camera.position);
            // The game's sounds stop with it.
            assert!(matches!(frame.take_sounds()[..], [SoundAsk::StopAll]));
            assert_eq!(editor.camera, camera);
            assert_eq!(editor.before, camera);
            assert_eq!(editor.map, map);
            assert_eq!(editor.panels.chosen_placed(), chosen.as_deref());
            // Letting go and pressing nothing does not start it again, and
            // the camera flies at once.
            let mut frame = tick_and_frame(&mut editor, &mut input);
            assert!(editor.play.is_none());
            assert!(frame.take_sounds().is_empty());
            input.press(Key::W);
            let _ = tick_and_frame(&mut editor, &mut input);
            assert_ne!(editor.camera.position, camera.position);
            let _ = std::fs::remove_dir_all(&folder);
        }
    }

    #[test]
    fn a_camera_with_nothing_under_it_starts_the_player_at_a_spawn_point_and_says_so() {
        let (folder, mut editor, mut input) = game_in("play-off", ONE_SPAWN);
        // Far outside the level's walls.
        editor.camera.position = [200.0, 5.0, 200.0];
        editor.tick(input.tick(), STEP);
        let context = panel_context();
        let _ = panel_frame(&mut editor, &context, 0.0, Vec::new());
        let (_, output) = panel_frame(&mut editor, &context, 0.1, Vec::new());
        assert!(words(&output).contains("Play: F5"));
        assert!(words(&output).contains("Select plays"));

        // The button along the bottom plays, as F5 does.
        let _ = click_on(&mut editor, &context, "play", 1.0);
        editor.tick(input.tick(), STEP);
        assert!(!editor.play.as_ref().expect("playing").started_here());
        let (frame, output) = panel_frame(&mut editor, &context, 2.0, Vec::new());
        input.set_active_set(frame.action_set().unwrap());
        let shown = words(&output);
        for expected in [
            "Playing",
            "F5 or Select: back to editing",
            "nothing under the camera to stand on",
        ] {
            assert!(shown.contains(expected), "no {expected:?} in:\n{shown}");
        }
        // The panels are not drawn while playing.
        assert!(!shown.contains("hold the right mouse button"), "{shown}");
        for _ in 0..30 {
            let _ = tick_and_frame(&mut editor, &mut input);
        }
        let eye = editor.play.as_ref().unwrap().eye();
        assert!(close(eye[0], -3.0) && close(eye[2], -10.0), "{eye:?}");

        // Back, and the game's script is taken away: it cannot be played,
        // which is said along the bottom, and the editor carries on.
        tap(&mut editor, &mut input, Key::F5);
        assert!(editor.play.is_none());
        let _ = tick_and_frame(&mut editor, &mut input);
        std::fs::remove_file(folder.join(fossps_script::ENTRY_SCRIPT)).unwrap();
        tap(&mut editor, &mut input, Key::F5);
        assert!(editor.play.is_none());
        assert!(
            editor.play_problem.starts_with("Could not play: "),
            "{}",
            editor.play_problem
        );
        let (_, output) = panel_frame(&mut editor, &context, 3.0, Vec::new());
        let shown = words(&output);
        assert!(shown.contains("Could not play: "), "{shown}");
        assert!(shown.contains("hold the right mouse button"), "{shown}");

        // With the script back, it plays, and the problem is forgotten.
        std::fs::write(folder.join(fossps_script::ENTRY_SCRIPT), "").unwrap();
        tap(&mut editor, &mut input, Key::F5);
        assert!(editor.play.is_some());
        assert_eq!(editor.play_problem, "");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Playing is drawn for real, with no window, and the picture is read
    /// back: the level from where the camera was, with no grid and no
    /// panels over it, and a line along the top that says how to come back.
    #[test]
    fn while_playing_the_picture_is_the_level_alone_under_a_line_on_how_to_come_back() {
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
        let level = crate::level::from_bytes("level.glb", crate::scaffold::STARTER_LEVEL).unwrap();
        headless.set_scene(&level).unwrap();

        let (folder, mut editor, mut input) = game_in("play-picture", ONE_SPAWN);
        editor.camera.position = [0.0, 3.0, 6.0];
        editor.camera.pitch = -0.25;
        editor.tick(input.tick(), STEP);
        let context = panel_context();
        #[allow(clippy::cast_precision_loss)]
        let size = egui::vec2(SCREEN[0] as f32, SCREEN[1] as f32);
        // What the window does with a frame: the grid as the editor asks
        // for it, the panels it made, seen from where it says.
        let draw = |editor: &mut Editor, headless: &mut Headless, time: f64| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                ..egui::RawInput::default()
            };
            let mut frame = Frame::new(1.0)
                .for_window(Some(SCREEN), fossps_ui::DrawBuffer::default())
                .with_panels(context.clone(), input);
            editor.frame(&mut frame);
            if let Some(shown) = frame.grid() {
                headless.set_grid(shown);
            }
            let output = frame.take_panels().unwrap();
            headless.set_panels(Panels {
                shapes: context.tessellate(output.shapes, output.pixels_per_point),
                textures: output.textures_delta,
                pixels_per_point: output.pixels_per_point,
            });
            let view = frame.view().unwrap();
            let camera = Camera {
                position: view.position,
                yaw: view.yaw,
                pitch: view.pitch,
                ..Camera::default()
            };
            headless.screenshot(&camera).unwrap()
        };
        // The level alone, before there are any panels or a grid.
        let alone = headless
            .screenshot(&Camera {
                position: editor.camera.position,
                yaw: editor.camera.yaw,
                pitch: editor.camera.pitch,
                ..Camera::default()
            })
            .unwrap();
        let _ = draw(&mut editor, &mut headless, 0.0);
        let editing = draw(&mut editor, &mut headless, 0.1);

        input.press(Key::F5);
        editor.tick(input.tick(), STEP);
        assert!(editor.play.is_some());
        let playing = draw(&mut editor, &mut headless, 0.2);
        // Set `FOSSPS_SAVE_PICTURES` to a folder to look at the pictures:
        // 1280 by 720 pixels of four bytes each.
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            let folder = std::path::Path::new(&folder);
            std::fs::write(folder.join("playing.rgba"), &playing).unwrap();
            std::fs::write(folder.join("before-playing.rgba"), &editing).unwrap();
        }

        let pixel = |picture: &[u8], x: usize, y: usize| {
            let at = (y * SCREEN[0] as usize + x) * 4;
            [picture[at], picture[at + 1], picture[at + 2]]
        };
        let near = |a: [u8; 3], b: [u8; 3]| (0..3).all(|part| a[part].abs_diff(b[part]) <= 3);
        let fill = context.global_style().visuals.panel_fill;
        let fill = [fill.r(), fill.g(), fill.b()];
        // Where the list, the inspector and the bar along the bottom were
        // while editing, the level shows while playing; and all over the
        // picture below the line at the top, it is the level with no grid.
        for (x, y) in [(100, 300), (1180, 300), (640, 700)] {
            assert!(near(pixel(&editing, x, y), fill), "{x}, {y} while editing");
        }
        let (mut same, mut all) = (0_u32, 0_u32);
        for y in (80..SCREEN[1] as usize).step_by(4) {
            for x in (0..SCREEN[0] as usize).step_by(4) {
                all += 1;
                same += u32::from(near(pixel(&playing, x, y), pixel(&alone, x, y)));
            }
        }
        assert!(
            same * 1000 >= all * 995,
            "{same} of {all} as the level alone"
        );
        // The line: white letters on a dark ground, in the middle at the top.
        let (mut white, mut dark) = (0, 0);
        for y in 10..46 {
            for x in 440..840 {
                let [red, green, blue] = pixel(&playing, x, y);
                white += u32::from(red > 220 && green > 220 && blue > 220);
                dark += u32::from(u32::from(red) + u32::from(green) + u32::from(blue) < 200);
            }
        }
        assert!(white > 150, "{white} white pixels in the line");
        assert!(dark > 3000, "{dark} dark pixels behind the line");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
