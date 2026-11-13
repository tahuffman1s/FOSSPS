//! Plays the game in a folder, or with no game shows live input so the action
//! map can be tried by hand.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fossps::input::ActionKind;
use fossps::script::ENTRY_SCRIPT;
use fossps::{ActionId, ActionMap, Config, Frame, Game, InputFrame, ScriptGame};

/// Prints log messages to standard error.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOG: StderrLog = StderrLog;

/// Shows which actions are active in the window title and logs every press.
struct InputDemo {
    map: ActionMap,
    look: Option<ActionId>,
    /// Total look movement, to show that mouse and stick both drive it.
    aim: [f32; 2],
    title: String,
    shown: String,
}

impl InputDemo {
    fn new(map: ActionMap) -> Self {
        Self {
            look: map.action("look").ok(),
            map,
            aim: [0.0; 2],
            title: String::new(),
            shown: String::new(),
        }
    }
}

impl Game for InputDemo {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        const MOUSE_DEGREES_PER_COUNT: f32 = 0.05;
        const STICK_DEGREES_PER_SECOND: f32 = 180.0;

        if let Some(look) = self.look {
            let mouse = input.delta(look);
            let stick = input.axis2d(look);
            for axis in 0..2 {
                self.aim[axis] += mouse[axis] * MOUSE_DEGREES_PER_COUNT
                    + stick[axis] * STICK_DEGREES_PER_SECOND * step;
            }
        }

        self.title.clear();
        // Writing to a String cannot fail.
        let _ = write!(self.title, "aim {:.0}, {:.0}", self.aim[0], self.aim[1]);
        for action in self.map.actions() {
            let name = self.map.name(action);
            if input.just_pressed(action) && self.map.kind(action) == Some(ActionKind::Button) {
                log::info!("tick {}: {name} pressed", input.tick);
            }
            if input.pressed(action) && Some(action) != self.look {
                let [x, y] = input.axis2d(action);
                let _ = match self.map.kind(action) {
                    Some(ActionKind::Axis2D) => write!(self.title, " | {name} {x:.2}, {y:.2}"),
                    Some(ActionKind::Axis1D) => write!(self.title, " | {name} {x:.2}"),
                    _ => write!(self.title, " | {name}"),
                };
            }
        }
    }

    fn frame(&mut self, frame: &mut Frame) {
        if self.title != self.shown {
            self.shown.clone_from(&self.title);
            frame.set_title(format!("FOSSPS input demo | {}", self.title));
        }
    }
}

/// Runs `game` and turns the result into the program's exit code.
fn play(config: Config, game: impl Game) -> ExitCode {
    match fossps::run(config, game) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    // If another logger is already installed, messages go there instead.
    if log::set_logger(&LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }

    // `fossps <folder>` plays the game in that folder; plain `fossps` plays the
    // game in the current folder if there is one.
    let folder = match std::env::args_os().nth(1) {
        Some(folder) => Some(PathBuf::from(folder)),
        None => Path::new(ENTRY_SCRIPT)
            .is_file()
            .then(|| PathBuf::from(".")),
    };
    let mut config = Config::default();

    let Some(folder) = folder else {
        log::info!(
            "No game folder given, so this is the input demo. To play a game, run \
             `fossps <folder>` with a folder that has a {ENTRY_SCRIPT} in it."
        );
        log::info!(
            "Click the window to capture the mouse, then use keyboard, mouse or a gamepad. \
             Escape frees the mouse."
        );
        let demo = InputDemo::new(config.actions.clone());
        return play(config, demo);
    };

    match ScriptGame::open(&folder, &config.actions) {
        Ok(game) => {
            let script = folder.join(ENTRY_SCRIPT);
            log::info!(
                "Playing {}. Edit the file and save it to see changes right away.",
                script.display()
            );
            if let Some(name) = folder
                .canonicalize()
                .ok()
                .as_deref()
                .and_then(Path::file_name)
            {
                config.title = format!("{} | FOSSPS", name.to_string_lossy());
            }
            play(config, game)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
