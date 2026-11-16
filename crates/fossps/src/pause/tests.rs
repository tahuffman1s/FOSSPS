//! The pause menu, worked the way a window works it: ticks, then a frame,
//! with keys, a gamepad and a mouse that are not there.

use std::fs;
use std::path::PathBuf;

use fossps_input::{GamepadButton, Input, Key, MouseButton, MouseMode, Stick, StickAxis};
use fossps_ui::{Draw, DrawBuffer};

use super::*;

const SCREEN: [u32; 2] = [1280, 720];
const FRAME: f32 = 1.0 / 60.0;

/// A game that notes what it was told.
#[derive(Default)]
struct Probe {
    ticks: u32,
    /// Ticks in which `fire` was held.
    fired: u32,
    /// The mouse movement `look` reported, added up.
    looked: [f32; 2],
    fire: Option<ActionId>,
    look: Option<ActionId>,
    /// The HUD size the last frame was told.
    ui_scale: f32,
    /// An action set to ask for in the next frame, once.
    ask_for: Option<ActionSetId>,
    /// A rumble to ask for in every frame.
    rumble: Option<Rumble>,
}

impl Game for Probe {
    fn tick(&mut self, input: &InputFrame, _step: f32) {
        self.ticks += 1;
        if self.fire.is_some_and(|fire| input.pressed(fire)) {
            self.fired += 1;
        }
        if let Some(look) = self.look {
            let moved = input.delta(look);
            self.looked = [self.looked[0] + moved[0], self.looked[1] + moved[1]];
        }
    }

    fn frame(&mut self, frame: &mut Frame) {
        self.ui_scale = frame.ui_scale();
        if let Some(set) = self.ask_for.take() {
            frame.use_action_set(set);
        }
        if let Some(rumble) = self.rumble {
            frame.rumble(rumble);
        }
    }
}

/// A game under the pause menu and everything a window keeps for it.
struct Played {
    game: PauseMenu<Probe>,
    input: Input,
    pointer: Option<[f32; 2]>,
    pointer_down: bool,
    /// What the last frame drew.
    drawn: DrawBuffer,
    /// Whether a frame asked for the window to close.
    closed: bool,
    /// The settings the frames asked to be put into effect, in order.
    applied: Vec<Settings>,
    /// The rumble the last frame asked for.
    rumble: Option<Rumble>,
}

impl Played {
    fn new() -> Self {
        Self::with(ActionMap::default_fps(), Settings::default(), None)
    }

    fn with(actions: ActionMap, settings: Settings, file: Option<PathBuf>) -> Self {
        let probe = Probe {
            fire: actions.action("fire").ok(),
            look: actions.action("look").ok(),
            ..Probe::default()
        };
        Self {
            game: PauseMenu::over(probe, &actions, settings, file),
            input: Input::new(actions),
            pointer: None,
            pointer_down: false,
            drawn: DrawBuffer::default(),
            closed: false,
            applied: Vec::new(),
            rumble: None,
        }
    }

    fn tick(&mut self) {
        self.game.tick(self.input.tick(), FRAME);
    }

    /// One frame, as the window makes it: what the game asks for is done.
    fn frame(&mut self) {
        let mut frame = Frame::new(0.0)
            .for_window(Some(SCREEN), std::mem::take(&mut self.drawn))
            .with_pointer(FRAME, self.pointer, self.pointer_down);
        self.game.frame(&mut frame);
        self.drawn = frame.take_ui();
        self.closed |= frame.closing();
        self.rumble = frame.rumble_requested();
        if let Some(settings) = frame.take_settings() {
            if let Some(look) = self.game.game.look {
                self.input.set_sensitivity(look, settings.look());
            }
            self.applied.push(settings);
        }
        if let Some(set) = frame.action_set() {
            self.input.set_active_set(set);
        }
    }

    /// One tick and one frame.
    fn step(&mut self) {
        self.tick();
        self.frame();
    }

    /// Presses `button` for one step and lets it go for the next.
    fn tap(&mut self, button: impl Into<fossps_input::Button> + Copy) {
        self.input.press(button);
        self.step();
        self.input.release(button);
        self.step();
    }

    /// Clicks the middle of the widget that shows `words`.
    fn click(&mut self, words: &str) {
        let at = self.place_of(words);
        self.pointer = Some([at[0] + 4.0, at[1] + 4.0]);
        self.step();
        self.pointer_down = true;
        self.step();
        self.pointer_down = false;
        self.step();
    }

    /// Where the text `words` was drawn in the last frame.
    fn place_of(&self, words: &str) -> [f32; 2] {
        self.drawn
            .draw()
            .iter()
            .find_map(|draw| match draw {
                Draw::Text { position, text, .. } if text == words => Some(position),
                _ => None,
            })
            .unwrap_or_else(|| panic!("\"{words}\" is not on screen: {:?}", self.words()))
    }

    /// Every piece of text the last frame drew, in order.
    fn words(&self) -> Vec<String> {
        self.drawn
            .draw()
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text { text, .. } => Some(text.to_owned()),
                Draw::Rect { .. } => None,
            })
            .collect()
    }

    fn menu(&self) -> &Menu {
        self.game.menu.as_ref().unwrap()
    }

    fn is_open(&self) -> bool {
        self.menu().ui.is_menu_open()
    }

    fn live_set(&self) -> &str {
        self.input.map().set_name(self.input.active_set())
    }
}

/// A file in a folder of this test's own, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("fossps-pause-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        Self(folder)
    }

    fn file(&self) -> PathBuf {
        self.0.join("settings.toml")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn escape_opens_the_menu_over_a_game_that_keeps_running_but_hears_nothing() {
    let mut played = Played::new();
    played.step();
    assert!(played.words().is_empty(), "nothing is drawn while playing");
    assert_eq!(played.live_set(), "gameplay");

    played.input.press(Key::Escape);
    played.step();
    assert_eq!(played.words(), ["Menu", "Resume", "Settings", "Quit"]);
    assert_eq!(played.live_set(), "menu");
    assert_eq!(
        played.input.map().mouse_mode(played.input.active_set()),
        Some(MouseMode::Pointer)
    );

    // Escape is still down from opening the menu: it must not close it.
    played.step();
    played.step();
    assert!(played.is_open());
    played.input.release(Key::Escape);

    // The match goes on, and the player's fire button and mouse do not reach it.
    let before = played.game.game.ticks;
    played.input.press(MouseButton::MouseLeft);
    played.input.move_mouse(40.0, 10.0);
    played.step();
    played.step();
    played.input.release(MouseButton::MouseLeft);
    assert_eq!(played.game.game.ticks, before + 2);
    assert_eq!(played.game.game.fired, 0);
    assert_eq!(played.game.game.looked, [0.0, 0.0]);

    // Escape again is "back", which closes the first screen.
    played.input.press(Key::Escape);
    played.step();
    assert!(!played.is_open());
    assert_eq!(played.live_set(), "gameplay");
    assert!(played.words().is_empty());
    // The same press does not open it again.
    played.step();
    played.step();
    assert!(!played.is_open());

    played.input.release(Key::Escape);
    played.input.press(MouseButton::MouseLeft);
    played.step();
    assert_eq!(played.game.game.fired, 1);
}

#[test]
fn a_gamepad_alone_changes_every_setting_and_the_file_keeps_them() {
    let scratch = Scratch::new("gamepad");
    let mut played = Played::with(
        ActionMap::default_fps(),
        Settings::default(),
        Some(scratch.file()),
    );
    played.step();
    // The window starts with the player's settings; the menu only says
    // when they change.
    assert!(played.applied.is_empty());

    played.tap(GamepadButton::GamepadStart);
    assert!(played.is_open());
    assert_eq!(
        played.menu().ui.focused(),
        Some(played.menu().widgets.resume)
    );
    played.tap(GamepadButton::DpadDown);
    played.tap(GamepadButton::GamepadSouth);
    assert_eq!(
        played.words(),
        [
            "Settings",
            "Volume",
            "100",
            "Look sensitivity",
            "1.0",
            "Invert look",
            "Off",
            "HUD and menu size",
            "100",
            "Reset to defaults",
            "Back"
        ]
    );

    // Volume: two steps down with the d-pad.
    played.tap(GamepadButton::DpadLeft);
    played.tap(GamepadButton::DpadLeft);
    // Sensitivity: up with the stick, which counts once pushed over half way.
    played.tap(GamepadButton::DpadDown);
    for value in [1.0, 0.0, 1.0, 0.0] {
        played
            .input
            .set_stick(Stick::LeftStick, StickAxis::X, value);
        played.step();
    }
    // Invert look: A flips it.
    played.tap(GamepadButton::DpadDown);
    played.tap(GamepadButton::GamepadSouth);
    // Size: one step up.
    played.tap(GamepadButton::DpadDown);
    played.tap(GamepadButton::DpadRight);

    let chosen = played.menu().settings;
    assert_eq!(chosen.volume, 90.0);
    assert!((chosen.sensitivity - 1.2).abs() < 1e-4, "{chosen:?}");
    assert!(chosen.invert_look);
    assert_eq!(chosen.ui_size, 105.0);
    assert_eq!(played.applied.last(), Some(&chosen));
    assert_eq!(played.game.game.ui_scale, 1.05);
    // Nothing is written while the player is still choosing.
    assert!(!scratch.file().exists());

    // B goes back one screen, and that saves.
    played.tap(GamepadButton::GamepadEast);
    assert_eq!(played.words()[0], "Menu");
    let (kept, problems) = settings::load(&scratch.file());
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(kept.volume, 90.0);
    assert!((kept.sensitivity - 1.2).abs() < 1e-4);
    assert!(kept.invert_look);
    assert_eq!(kept.ui_size, 105.0);

    // Start closes the menu from however deep.
    played.tap(GamepadButton::GamepadSouth);
    assert_eq!(played.words()[0], "Settings");
    played.tap(GamepadButton::GamepadStart);
    assert!(!played.is_open());
    assert_eq!(played.live_set(), "gameplay");

    // The view now turns faster, with up and down swapped.
    played.input.move_mouse(10.0, 10.0);
    played.step();
    let looked = played.game.game.looked;
    assert!((looked[0] - 12.0).abs() < 1e-3 && (looked[1] + 12.0).abs() < 1e-3);
}

#[test]
fn a_mouse_alone_works_the_menu() {
    let mut played = Played::new();
    played.tap(Key::Escape);
    played.click("Settings");
    assert_eq!(played.words()[0], "Settings");

    played.click("Invert look");
    assert!(played.menu().settings.invert_look);

    // The slider is dragged: press on its track, move, let go.
    let label = played.place_of("Volume");
    let number = played.place_of("100");
    let track_start = label[0] + 250.0;
    played.pointer = Some([number[0] - 30.0, label[1] + 8.0]);
    played.step();
    played.pointer_down = true;
    played.step();
    played.pointer = Some([(track_start + number[0]) / 2.0, label[1] + 8.0]);
    played.step();
    played.pointer_down = false;
    played.step();
    let volume = played.menu().settings.volume;
    assert!((20.0..=80.0).contains(&volume), "{volume}");

    played.click("Reset to defaults");
    assert_eq!(played.menu().settings, Settings::default());
    assert_eq!(played.applied.last(), Some(&Settings::default()));
    assert!(played.words().contains(&"Off".to_owned()));

    played.click("Back");
    assert_eq!(played.words()[0], "Menu");
    played.click("Resume");
    assert!(!played.is_open());
    assert_eq!(played.live_set(), "gameplay");
}

#[test]
fn quitting_asks_first_and_starts_on_the_safe_answer() {
    let mut played = Played::new();
    played.tap(Key::Escape);
    played.tap(Key::ArrowUp);
    assert_eq!(played.menu().ui.focused(), Some(played.menu().widgets.quit));
    played.tap(Key::Enter);
    assert_eq!(played.words(), ["Quit the game?", "Keep playing", "Quit"]);
    assert_eq!(played.menu().ui.focused(), Some(played.menu().widgets.stay));

    // A second Enter, the easy mistake, keeps the game open.
    played.tap(Key::Enter);
    assert!(!played.closed);
    assert_eq!(played.words()[0], "Menu");

    played.tap(Key::Enter);
    played.tap(Key::S);
    played.tap(Key::Space);
    assert!(played.closed);
}

#[test]
fn a_press_that_came_and_went_between_two_frames_still_counts() {
    let mut played = Played::new();
    // A slow frame: the key goes down and up again in the ticks before it.
    played.input.press(Key::Escape);
    played.tick();
    played.input.release(Key::Escape);
    played.tick();
    played.frame();
    assert!(played.is_open());

    played.input.press(Key::ArrowDown);
    played.tick();
    played.input.release(Key::ArrowDown);
    played.tick();
    played.input.press(Key::Enter);
    played.tick();
    played.input.release(Key::Enter);
    played.tick();
    played.frame();
    played.frame();
    assert_eq!(played.words()[0], "Settings");

    // A fast display: many frames in one tick press a button once.
    played.input.press(Key::Enter);
    played.tick();
    for _ in 0..5 {
        played.frame();
    }
    // Enter was on the volume slider, which it does not change.
    assert_eq!(played.words()[0], "Settings");
    played.input.release(Key::Enter);
    played.input.press(Key::Escape);
    played.tick();
    for _ in 0..5 {
        played.frame();
    }
    assert_eq!(played.words()[0], "Menu", "one press goes back one screen");
}

#[test]
fn the_menu_goes_back_to_the_action_set_the_game_was_using() {
    let mut actions = ActionMap::default_fps();
    let driving = actions.add_set("driving", MouseMode::Look).unwrap();
    actions.button("horn", ["H", "GamepadNorth"]).unwrap();
    let mut played = Played::with(actions, Settings::default(), None);
    played.game.game.ask_for = Some(driving);
    played.step();
    assert_eq!(played.live_set(), "driving");

    // The menu is opened as a game would open it: `pause` is not in this set.
    played.game.menu.as_mut().unwrap().open_asked = true;
    played.step();
    assert_eq!(played.live_set(), "menu");
    played.tap(Key::Escape);
    assert!(!played.is_open());
    assert_eq!(played.live_set(), "driving");
}

#[test]
fn opening_the_menu_stops_a_rumble() {
    let mut played = Played::new();
    played.game.game.rumble = Some(Rumble::new(1.0, 1.0, 2.0));
    played.step();
    assert_eq!(played.rumble, Some(Rumble::new(1.0, 1.0, 2.0)));
    played.input.press(GamepadButton::GamepadStart);
    played.step();
    assert_eq!(played.rumble, Some(Rumble::STOP));
}

#[test]
fn a_game_with_other_controls_or_its_own_menu_is_left_alone() {
    let mut own = ActionMap::new();
    own.button("pause", ["Escape", "GamepadStart"]).unwrap();
    let mut played = Played::with(own, Settings::default(), None);
    assert!(played.game.menu.is_none());
    played.tap(Key::Escape);
    assert!(played.words().is_empty());
    assert_eq!(played.live_set(), "gameplay");
    assert!(played.applied.is_empty());

    // The menu's actions in the wrong set would never be heard.
    let mut muddled = ActionMap::new();
    for name in ["pause", "menu_confirm", "menu_back", "menu_close"] {
        muddled.button(name, ["Escape", "GamepadStart"]).unwrap();
    }
    muddled.axis2d("menu_move", ["WASD", "Dpad"]).unwrap();
    muddled.add_set("menu", MouseMode::Pointer).unwrap();
    for name in ["menu_previous_tab", "menu_next_tab"] {
        muddled.button(name, ["Q", "LeftBumper"]).unwrap();
    }
    assert!(Menu::new(&muddled, Settings::default(), None).is_none());

    // A game with its own menu still gets the HUD size the player chose.
    let bigger = Settings {
        ui_size: 125.0,
        ..Settings::default()
    };
    let mut off = PauseMenu::off(Probe::default(), bigger);
    off.tick(Input::new(ActionMap::default_fps()).tick(), FRAME);
    off.frame(&mut Frame::default());
    assert_eq!(off.game.ticks, 1);
    assert_eq!(off.game.ui_scale, 1.25);
}

#[test]
fn settings_changed_when_the_window_closes_are_still_saved() {
    let scratch = Scratch::new("closing");
    let mut played = Played::with(
        ActionMap::default_fps(),
        Settings::default(),
        Some(scratch.file()),
    );
    played.tap(Key::Escape);
    played.tap(Key::ArrowDown);
    played.tap(Key::Enter);
    played.tap(Key::ArrowLeft);
    assert!(!scratch.file().exists());
    drop(played);
    assert_eq!(settings::load(&scratch.file()).0.volume, 95.0);
}

#[test]
fn the_size_setting_waits_for_the_mouse_to_let_go_of_its_slider() {
    let mut played = Played::new();
    played.tap(Key::Escape);
    played.click("Settings");
    let label = played.place_of("HUD and menu size");
    // Press near the far end of the track and hold.
    let numbers: Vec<[f32; 2]> = (played.drawn.draw().iter())
        .filter_map(|draw| match draw {
            Draw::Text {
                position,
                text: "100",
                ..
            } => Some(position),
            _ => None,
        })
        .collect();
    let end = numbers[1][0] - 20.0;
    played.pointer = Some([end, label[1] + 8.0]);
    played.step();
    played.pointer_down = true;
    played.step();
    played.step();
    assert!(played.menu().settings.ui_size > 100.0);
    // The menu does not grow under the pointer that is dragging it.
    assert_eq!(played.menu().shown_scale, 1.0);
    assert_eq!(played.place_of("HUD and menu size"), label);

    played.pointer_down = false;
    played.step();
    assert!(played.menu().shown_scale > 1.0);
    assert_ne!(played.place_of("HUD and menu size"), label);
}

/// The menu is drawn for real, with no window, and the picture is read
/// back: the game is dimmed, a dark panel sits in the middle, and the
/// button the player is on has the bright outline.
#[test]
fn the_menu_is_drawn_as_a_panel_in_the_middle_of_a_dimmed_game() {
    use fossps_render::{Camera, Headless, RenderError, Upscaling};

    let mut headless = match Headless::new(SCREEN[0], SCREEN[1], Upscaling::Off) {
        Ok(headless) => headless,
        Err(
            error @ (RenderError::NoVulkan(_)
            | RenderError::OldVulkan { .. }
            | RenderError::NoSuitableGpu { .. }),
        ) => {
            let required = std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
            assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
            eprintln!("Skipping the drawing test: {error}");
            return;
        }
        Err(error) => panic!("{error}"),
    };
    headless.set_scene(&crate::standin::scene()).unwrap();
    let camera = Camera {
        position: [0.0, 1.6, 0.0],
        ..Camera::default()
    };
    let bare = headless.screenshot(&camera).unwrap();

    let mut played = Played::new();
    played.tap(GamepadButton::GamepadStart);
    let mut shots = Vec::new();
    headless.set_ui(played.drawn.draw());
    let top = headless.screenshot(&camera).unwrap();
    shots.push(("menu", top.clone()));
    played.tap(GamepadButton::DpadDown);
    played.tap(GamepadButton::GamepadSouth);
    played.tap(GamepadButton::DpadDown);
    headless.set_ui(played.drawn.draw());
    shots.push(("settings", headless.screenshot(&camera).unwrap()));
    played.tap(GamepadButton::GamepadEast);
    played.tap(GamepadButton::DpadDown);
    played.tap(GamepadButton::GamepadSouth);
    headless.set_ui(played.drawn.draw());
    shots.push(("quit", headless.screenshot(&camera).unwrap()));
    assert_eq!(fossps_render::validation_errors(), 0);

    // To look at the pictures: FOSSPS_SAVE_PICTURES=some/folder.
    if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        for (name, picture) in &shots {
            let mut file = format!("P6\n{} {}\n255\n", SCREEN[0], SCREEN[1]).into_bytes();
            file.extend(
                picture
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|pixel| &pixel[..3]),
            );
            fs::write(folder.join(format!("pause-{name}.ppm")), file).unwrap();
        }
    }

    let pixel = |picture: &[u8], x: u32, y: u32| {
        let start = ((y * SCREEN[0] + x) * 4) as usize;
        [picture[start], picture[start + 1], picture[start + 2]]
    };
    let brightness = |[r, g, b]: [u8; 3]| u32::from(r) + u32::from(g) + u32::from(b);

    // The floor in a corner of the picture, far from the panel: the game,
    // darker.
    let corner = [40, SCREEN[1] - 40];
    let before = pixel(&bare, corner[0], corner[1]);
    let after = pixel(&top, corner[0], corner[1]);
    assert!(brightness(before) > 60, "the floor is lit: {before:?}");
    assert!(
        brightness(after) * 2 < brightness(before) + 30,
        "the game is dimmed: {before:?} to {after:?}"
    );

    // The middle column of the picture crosses the panel and its buttons:
    // some of it is the dark panel, and some is the bright outline round
    // "Resume", which is amber (far more red than blue).
    let middle = SCREEN[0] / 2;
    let column: Vec<[u8; 3]> = (0..SCREEN[1]).map(|y| pixel(&top, middle, y)).collect();
    assert!(
        column
            .iter()
            .any(|&[r, g, b]| r > 200 && g > 130 && b < 110),
        "the outline of the button the player is on"
    );
    let dark = column.iter().filter(|&&p| brightness(p) < 110).count();
    assert!(
        dark > 150,
        "the panel and its buttons are dark: {dark} rows"
    );
    // Light text is somewhere on the first button's row.
    let resume = played_place(&mut Played::new(), "Resume");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let row = resume[1] as u32 + 10;
    let lit = (0..SCREEN[0])
        .filter(|&x| brightness(pixel(&top, x, row)) > 600)
        .count();
    assert!(lit > 8, "the button's words are drawn light: {lit} pixels");
}

/// Where `words` is drawn on the first screen of a newly opened menu.
fn played_place(played: &mut Played, words: &str) -> [f32; 2] {
    played.tap(GamepadButton::GamepadStart);
    played.place_of(words)
}
