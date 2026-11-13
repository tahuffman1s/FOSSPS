//! The `fossps` command: `fossps new` creates a game and `fossps run` plays it.

mod demo;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use fossps::scaffold::{self, NewGame};
use fossps::script::{ENTRY_SCRIPT, OpenError};
use fossps::{Config, Game, ScriptGame};

use crate::demo::InputDemo;

/// Shown under the help text and when `fossps` is run with nothing after it.
const GETTING_STARTED: &str = "\
Start here:
  fossps new my-game    Create a game in a new folder called my-game
  cd my-game
  fossps run            Play it";

#[derive(Parser)]
#[command(
    name = "fossps",
    // Otherwise help on Windows says `fossps.exe`, unlike every example.
    bin_name = "fossps",
    version,
    about = "Make and play multiplayer first-person shooters.",
    after_help = GETTING_STARTED
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new game that is ready to play
    #[command(
        after_help = "Examples:\n  fossps new my-game\n  fossps new \"Zombie Night\"    \
                            A name with a space needs quotes"
    )]
    New {
        /// The folder to create. Its name becomes the game's name
        folder: PathBuf,
    },
    /// Play a game
    #[command(
        after_help = "Examples:\n  fossps run            Play the game in this folder\n  \
                            fossps run my-game    Play the game in the folder my-game"
    )]
    Run {
        /// The game's folder. Leave it out to play the game in the folder you are in
        folder: Option<PathBuf>,
    },
    /// Test a game's script without opening a window
    #[command(
        after_help = "Loads main.luau, starts the game and runs one second of it with nothing \
                      pressed.\nPrints any script error and fails if there was one, so it can be \
                      used by scripts,\nautomated checks and AI assistants."
    )]
    Check {
        /// The game's folder. Leave it out to check the game in the folder you are in
        folder: Option<PathBuf>,
    },
    /// Open a test window that shows what your keyboard, mouse or gamepad is doing
    Demo,
}

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

fn main() -> ExitCode {
    // If another logger is already installed, messages go there instead.
    if log::set_logger(&LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }

    let outcome = match Cli::parse().command {
        Some(Command::New { folder }) => new(&folder),
        Some(Command::Run { folder }) => run(folder.as_deref().unwrap_or(Path::new("."))),
        Some(Command::Check { folder }) => check(folder.as_deref().unwrap_or(Path::new("."))),
        Some(Command::Demo) => demo(),
        // Asking for nothing is not a mistake: show what there is to ask for.
        None => Cli::command()
            .print_help()
            .map_err(|error| error.to_string()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// `fossps new`: creates the game and says how to play it.
fn new(folder: &Path) -> Result<(), String> {
    let NewGame { name, folder: full } =
        scaffold::new_game(folder).map_err(|error| error.to_string())?;

    println!("Created the game \"{name}\" in {}", full.display());
    println!();
    println!("Play it:");
    // `fossps new .` has no folder name in it: the game is the folder you are in.
    if folder.file_name().is_some() {
        println!("    cd {}", quoted(&folder.display().to_string()));
    }
    println!("    fossps run");
    println!();
    println!(
        "Then open {ENTRY_SCRIPT} in an editor, change something and save: \
         the running game updates by itself."
    );
    Ok(())
}

/// Opens the game in `folder`, or says how to get one.
fn open(folder: &Path, config: &Config) -> Result<ScriptGame, String> {
    if folder.is_file() {
        return Err(format!(
            "{} is a file. This command takes the game's folder, the one with \
             {ENTRY_SCRIPT} in it; inside that folder, no folder name is needed.",
            folder.display()
        ));
    }
    if !folder.is_dir() {
        return Err(format!(
            "There is no folder called {}. Check the spelling, or create a game there \
             with `fossps new {}`.",
            folder.display(),
            quoted(&folder.display().to_string())
        ));
    }
    ScriptGame::open(folder, &config.actions).map_err(|error| match error {
        OpenError::Missing { .. } => format!(
            "There is no game here: {} has no {ENTRY_SCRIPT} in it.\n\
             Run this from inside a game's folder, or name the folder: \
             `fossps run my-game`.\n\
             To make a new game: `fossps new my-game`.",
            if folder == Path::new(".") {
                "this folder".to_owned()
            } else {
                folder.display().to_string()
            }
        ),
        other @ OpenError::Unreadable { .. } => other.to_string(),
    })
}

/// `fossps check`: runs a second of the game with no window and nothing
/// pressed, and fails if the script has a mistake.
fn check(folder: &Path) -> Result<(), String> {
    /// One second of game time.
    const TICKS: u32 = 60;

    let config = Config::default();
    let mut game = open(folder, &config)?;
    let mut input = fossps::input::Input::new(config.actions.clone());
    let step = 1.0 / TICKS as f32;
    // A script that fails to load never runs, so there is nothing to tick.
    if game.host().last_error().is_none() {
        for _ in 0..TICKS {
            game.tick(input.tick(), step);
        }
    }

    // The error itself was printed when it happened.
    if game.host().last_error().is_some() {
        return Err(format!(
            "{ENTRY_SCRIPT} has a mistake (shown above). Fix it and run `fossps check` again."
        ));
    }
    println!(
        "{ENTRY_SCRIPT} is fine: it loaded and ran for {TICKS} ticks (1 second of game time) \
         with no errors."
    );
    println!(
        "Nothing was pressed, so code that only runs on input was not run. \
         Try that with `fossps run`."
    );
    Ok(())
}

/// `fossps run`: plays the game in `folder` until its window is closed.
fn run(folder: &Path) -> Result<(), String> {
    let mut config = Config::default();
    let game = open(folder, &config)?;

    match scaffold::refresh_types(folder) {
        Ok(true) => log::info!(
            "Updated {} to match this version of FOSSPS, so editor autocomplete stays right.",
            scaffold::TYPES_FILE
        ),
        Ok(false) => {}
        // Only autocomplete is affected, so the game still starts.
        Err(error) => log::warn!(
            "Could not update {}: {error}. The game still runs, but editor autocomplete \
             may be out of date.",
            scaffold::TYPES_FILE
        ),
    }

    log::info!(
        "Playing {}. Edit the file and save it to see changes right away.",
        folder.join(ENTRY_SCRIPT).display()
    );
    if let Some(name) = std::path::absolute(folder)
        .ok()
        .as_deref()
        .and_then(Path::file_name)
    {
        config.title = format!("{} | FOSSPS", name.to_string_lossy());
    }
    play(config, game)
}

/// `fossps demo`: shows live input in a window.
fn demo() -> Result<(), String> {
    log::info!(
        "This is the input demo. Click the window to capture the mouse, then use \
         keyboard, mouse or a gamepad. Escape frees the mouse."
    );
    let config = Config::default();
    let demo = InputDemo::new(config.actions.clone());
    play(config, demo)
}

/// Runs `game` until its window is closed.
fn play(config: Config, game: impl Game) -> Result<(), String> {
    fossps::run(config, game).map_err(|error| error.to_string())
}

/// `text` as one word for a command line: in quotes if it has a space.
fn quoted(text: &str) -> String {
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.to_owned()
    }
}
