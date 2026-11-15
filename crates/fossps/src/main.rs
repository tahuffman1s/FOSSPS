//! The `fossps` command: `fossps new` creates a game, `fossps run` plays it,
//! `fossps serve` hosts it, `fossps join` joins a hosted one,
//! `fossps servers` lists the ones on this network, `fossps package` makes a
//! download for players, `fossps controls` shows and keeps the player's own
//! controls and `fossps bench` measures how fast this computer draws.
//!
//! The same program is the player inside a packaged game: started with a
//! game pack of its own name beside it, it plays that game.

mod demo;

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use fossps::Upscaling;
use fossps::bench::Bench;
use fossps::cli::{self, quoted};
use fossps::online::{DEFAULT_PORT, FoundServer, JoinedGame, LocalGame, SEARCH_TIME, find_servers};
use fossps::package;
use fossps::player;
use fossps::scaffold::{self, NewGame};
use fossps::script::ENTRY_SCRIPT;
use fossps::script::mode::Phase;
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
                            fossps run my-game    Play the game in the folder my-game\n  \
                            fossps run --open     Play it, and let others on your network join \
                            with `fossps join`\n  \
                            fossps run dist/my-game.fpk    Play a game pack, as a packaged game does"
    )]
    Run {
        /// The game's folder, or a game pack (.fpk). Leave it out to play the game in the
        /// folder you are in
        folder: Option<PathBuf>,
        /// Let other players on your network join this game with `fossps join`
        #[arg(long)]
        open: bool,
    },
    /// Test a game's script without opening a window
    #[command(
        after_help = "Loads main.luau, starts the game and runs one second of it with nothing \
                      pressed.\nAnything the script prints is shown first. Prints any script error and fails if there was one, so it can be \
                      used by scripts,\nautomated checks and AI assistants."
    )]
    Check {
        /// The game's folder. Leave it out to check the game in the folder you are in
        folder: Option<PathBuf>,
    },
    /// Run a game as a server, with no window
    #[command(
        after_help = "Examples:\n  fossps serve                Serve the game in this folder\n  \
                            fossps serve my-game        Serve the game in the folder my-game\n  \
                            fossps serve --port 7000    Take players on UDP port 7000\n\n\
                      Players on your network join with `fossps join`, which finds the \
                      server by itself.\nFrom elsewhere they add this computer's address.\n\
                      Press Ctrl-C to stop the server. For a machine that only hosts, the \
                      separate\nfossps-server program does the same and needs no graphics \
                      card or desktop."
    )]
    Serve {
        /// The game's folder. Leave it out to serve the game in the folder you are in
        folder: Option<PathBuf>,
        /// The UDP port players join through
        #[arg(long, default_value_t = DEFAULT_PORT, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
    },
    /// Join a game someone is serving
    #[command(
        after_help = "Examples:\n  fossps join                     Find a game on this network or this \
                            computer and join it\n  \
                            fossps join 192.168.1.20        Join the server at that address\n  \
                            fossps join example.com:7000    Join a server on another port than 24680\n\n\
                      With no address, `fossps join` joins the game it finds. If it finds \
                      several, it lists\nthem and asks which one."
    )]
    Join {
        /// The server's address. Leave it out to find a game on this network
        address: Option<String>,
    },
    /// List the games being served on this network
    #[command(
        after_help = "Examples:\n  fossps servers                Games on this network and this computer\n  \
                            fossps servers --port 7000    Games served with `--port 7000`\n\n\
                      Only games on the same network are found. `fossps join` uses the same \
                      search and\njoins what it finds."
    )]
    Servers {
        /// The UDP port the games are served on
        #[arg(long, default_value_t = DEFAULT_PORT, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
    },
    /// Make a download of a game that players can unpack and run
    #[command(
        after_help = "Examples:\n  fossps package                   Package the game in this folder \
                            for this kind of computer\n  \
                            fossps package my-game           Package the game in the folder my-game\n  \
                            fossps package --target linux    Package it for Linux\n\n\
                      The package is written to the folder dist inside the game's folder. \
                      For Linux it is a .tar.gz\nholding the game's program, its files in \
                      one game pack (.fpk), and its icon if the game's\nfolder has an \
                      icon.png. Nothing is compiled. Packages for Windows and macOS are \
                      not built yet."
    )]
    Package {
        /// The game's folder. Leave it out to package the game in the folder you are in
        folder: Option<PathBuf>,
        /// The kind of computer to package for. Leave it out to package for this kind
        #[arg(long, value_enum, value_name = "SYSTEM")]
        target: Option<Target>,
        /// A folder holding ready-made FOSSPS programs to put in packages, instead of
        /// this one: fossps-linux-x86_64 for Linux
        #[arg(long, value_name = "FOLDER")]
        runtime_dir: Option<PathBuf>,
    },
    /// Show your controls and the file to change them in
    #[command(
        after_help = "Your controls are kept in one text file on this computer, which every game \
                      played with FOSSPS\nreads when it starts. This command prints where it \
                      is, and makes it if it is not there yet.\nOpen it in a text editor, \
                      change a line such as jump = [\"F\", \"GamepadSouth\"] and save.\n\n\
                      Names are a key (Space, F, LeftShift), a mouse button (MouseLeft, \
                      MouseWheelUp) or a gamepad\nbutton (GamepadSouth, RightTrigger). To \
                      go back to a game's own controls, delete the file.\nTo keep the file \
                      somewhere else, set FOSSPS_BINDINGS to its path."
    )]
    Controls,
    /// Open a test window that shows what your keyboard, mouse or gamepad is doing
    Demo,
    /// Measure how fast this computer draws a built-in test scene
    #[command(
        after_help = "Examples:\n  fossps bench                       Draw the test scene and say how \
                            long its frames took\n  \
                            fossps bench --size 1920x1080      Draw a bigger picture\n  \
                            fossps bench --upscaling off       Draw every pixel, with no FSR\n  \
                            fossps bench --budget-ms 16.6      Fail if a slow frame took longer \
                            than 16.6 ms\n  \
                            fossps bench --json bench.json     Also save the numbers for a program \
                            to read\n\n\
                      No window opens. The same frames are drawn on every run, so two runs \
                      differ only in how\nfast the computer drew them. Close other programs \
                      first for steadier numbers."
    )]
    Bench {
        /// How many frames to time
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
        frames: u32,
        /// How many frames to draw first without timing them
        #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(..=1_000_000))]
        warmup: u32,
        /// The size of the picture, in pixels
        #[arg(long, default_value = "1280x720", value_name = "WIDTHxHEIGHT", value_parser = cli::picture_size)]
        size: (u32, u32),
        /// How much smaller the scene is drawn before FSR makes it full size:
        /// off, native, quality, balanced, performance or ultra-performance
        #[arg(long, default_value = "quality", value_name = "NAME", value_parser = cli::upscaling)]
        upscaling: Upscaling,
        /// Fail if a slow frame (99 of 100 are faster) took longer than this many milliseconds
        #[arg(long, value_name = "MS", value_parser = cli::budget)]
        budget_ms: Option<f64>,
        /// Also write the numbers to this file as JSON
        #[arg(long, value_name = "FILE")]
        json: Option<PathBuf>,
    },
}

/// A kind of computer `fossps package` can be asked to package for.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Target {
    /// Linux, Windows and macOS
    All,
    /// A .tar.gz for Linux (x86-64)
    Linux,
    /// Not built yet
    Windows,
    /// Not built yet
    Macos,
}

impl Target {
    /// The kind of computer this program runs on.
    fn here() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// What a packaged game can be started with. The game's own name stands in
/// for `game` in its help.
#[derive(Parser)]
#[command(name = "game", about = "Plays the game packed with this program.")]
struct Player {
    /// Play this game pack (.fpk) instead of the one beside the program
    #[arg(long, value_name = "FILE")]
    pack: Option<PathBuf>,
    /// Show your controls and the file to change them in, instead of playing
    #[arg(long)]
    controls: bool,
}

/// What a packaged game was started to do.
enum Asked {
    /// Play the game in this pack.
    Play(PathBuf),
    /// Show the player's controls.
    Controls,
}

/// The game pack this program was started to play, if it was started as a
/// packaged game: with `--pack` first, or with a pack of its own name beside
/// it. `Err` is clap's own message, for `--help` or a mistyped option.
fn pack_to_play() -> Option<Result<Asked, clap::Error>> {
    let program = std::env::current_exe().ok();
    let beside = program.as_deref().and_then(player::pack_beside);
    let asked = std::env::args_os().nth(1).is_some_and(|first| {
        let first = first.to_string_lossy();
        first == "--pack" || first.starts_with("--pack=")
    });
    if beside.is_none() && !asked {
        return None;
    }
    let name = program.as_deref().and_then(Path::file_stem).map_or_else(
        || "game".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let matches = Player::command()
        .name(name.clone())
        .bin_name(name)
        .try_get_matches();
    match matches {
        Ok(matches) if matches.get_flag("controls") => Some(Ok(Asked::Controls)),
        Ok(matches) => (matches.get_one::<PathBuf>("pack").cloned())
            .or(beside)
            .map(|pack| Ok(Asked::Play(pack))),
        Err(error) => Some(Err(error)),
    }
}

fn main() -> ExitCode {
    cli::log_to_stderr();

    let outcome = match pack_to_play() {
        Some(Ok(Asked::Play(pack))) => play_pack(&pack),
        Some(Ok(Asked::Controls)) => controls(),
        // Prints help to standard output and mistakes to standard error,
        // with the exit code each should have.
        Some(Err(error)) => error.exit(),
        None => command(),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Does what the `fossps` command was asked to.
fn command() -> Result<(), String> {
    match Cli::parse().command {
        Some(Command::New { folder }) => new(&folder),
        Some(Command::Run { folder, open }) => {
            run(folder.as_deref().unwrap_or(Path::new(".")), open)
        }
        Some(Command::Check { folder }) => check(folder.as_deref().unwrap_or(Path::new("."))),
        Some(Command::Serve { folder, port }) => cli::serve_game(
            folder.as_deref().unwrap_or(Path::new(".")),
            "fossps serve",
            port,
        ),
        Some(Command::Join { address }) => match address {
            Some(address) => join(&address),
            None => find_and_join(),
        },
        Some(Command::Servers { port }) => servers(port),
        Some(Command::Package {
            folder,
            target,
            runtime_dir,
        }) => package(
            folder.as_deref().unwrap_or(Path::new(".")),
            target.unwrap_or_else(Target::here),
            runtime_dir.as_deref(),
        ),
        Some(Command::Controls) => controls(),
        Some(Command::Demo) => demo(),
        Some(Command::Bench {
            frames,
            warmup,
            size: (width, height),
            upscaling,
            budget_ms,
            json,
        }) => cli::bench(
            &Bench {
                frames,
                warmup,
                width,
                height,
                upscaling,
            },
            budget_ms,
            json.as_deref(),
        ),
        // Asking for nothing is not a mistake: show what there is to ask for.
        None => Cli::command()
            .print_help()
            .map_err(|error| error.to_string()),
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

/// Opens the game in `folder` for `fossps {verb}`, or says how to get one.
fn open(folder: &Path, config: &Config, verb: &str) -> Result<ScriptGame, String> {
    cli::open_game(folder, &config.actions, &format!("fossps {verb}"))
}

/// `fossps check`: runs a second of the game and a pretend match with no
/// window and nothing pressed, and fails if the script has a mistake.
fn check(folder: &Path) -> Result<(), String> {
    /// One second of game time.
    const TICKS: u32 = 60;
    /// The longest warm-up the pretend match sits through: ten minutes.
    const LONGEST_WARMUP: u32 = 600 * TICKS;

    let config = Config::default();
    let mut game = open(folder, &config, "check")?;
    let mut input = fossps::input::Input::new(config.actions.clone());
    let step = 1.0 / TICKS as f32;
    let mut played = false;
    // A script that fails to load never runs, so there is nothing to tick.
    if game.host().last_error().is_none() {
        for _ in 0..TICKS {
            game.tick(input.tick(), step);
        }
        // A pretend match, so the functions that wait for players run too.
        let first = game.host_mut().player_joined("Check Bot 1");
        let second = game.host_mut().player_joined("Check Bot 2");
        let mut waited = 0;
        while game.host().mode().phase() != Phase::Playing && waited < LONGEST_WARMUP {
            game.tick(input.tick(), step);
            waited += 1;
        }
        played = game.host().mode().phase() == Phase::Playing;
        game.host_mut().player_killed(Some(first), second);
        game.host_mut().player_killed(None, first);
        game.tick(input.tick(), step);
        game.host_mut().end_round();
        game.tick(input.tick(), step);
        game.host_mut().player_left(second);
        game.host_mut().player_left(first);
        game.tick(input.tick(), step);
    }

    // The error itself was printed when it happened.
    if game.host().last_error().is_some() {
        return Err(format!(
            "{ENTRY_SCRIPT} has a mistake (the error is shown above). No game is running \
             during a check, so nothing reloads: fix it, then run `fossps check` again."
        ));
    }
    println!(
        "{ENTRY_SCRIPT} is fine: it loaded and ran for {TICKS} ticks (1 second of game time) \
         with no errors."
    );
    if played {
        println!(
            "Then a pretend match was played: two players joined, the round started, one \
             killed the other, one died alone, the round ended and both left. Each \
             game.on... function in the script was called at least once, with no errors. \
             Code the pretend match does not reach, such as what happens on a team kill, \
             was not run."
        );
    } else {
        println!(
            "A pretend match was started with two players, but its warm-up is longer than \
             ten minutes, so the round itself was not played. To have it played, set a \
             shorter warm-up in game.mode, such as warmup = 5."
        );
    }
    println!(
        "Nothing was pressed, so code that only runs on input was not run. \
         Read that code carefully, and try it by playing with `fossps run`."
    );
    Ok(())
}

/// `fossps run`: plays the game in `folder` until its window is closed.
/// With `others_may_join`, other players can join it meanwhile.
fn run(folder: &Path, others_may_join: bool) -> Result<(), String> {
    if folder.is_file() && player::is_pack(folder) {
        return play_packed(folder, others_may_join);
    }
    let mut config = player_config();
    let game = open(folder, &config, "run")?;

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
    let name = cli::game_name(folder);
    if !name.is_empty() {
        config.title = format!("{name} | FOSSPS");
    }
    start(config, game, &name, others_may_join)
}

/// Starts `game`, called `name`, and plays it until its window is closed.
/// With `others_may_join`, other players can join it meanwhile.
fn start(
    config: Config,
    game: ScriptGame,
    name: &str,
    others_may_join: bool,
) -> Result<(), String> {
    let game = if others_may_join {
        let game = LocalGame::open(game, name, DEFAULT_PORT, &config.actions, config.tick_rate)
            .map_err(|error| {
                format!(
                    "The game could not be opened for others to join: {error}\nIf another \
                     game is already open or served on this computer, close it first. To \
                     play alone, leave out `--open`."
                )
            })?;
        log::info!(
            "Open for others to join. {}",
            cli::how_to_join(DEFAULT_PORT, fossps::online::lan_address())
        );
        game
    } else {
        LocalGame::start(game, &config.actions, config.tick_rate).map_err(|error| {
            format!(
                "The game could not start: {error}\nIt is played over this computer's own \
                 network (127.0.0.1), which a firewall or security program may be blocking."
            )
        })?
    };
    play(config.clone(), game.with_title(config.title))
}

/// A packaged game started by its player: plays the game in the pack at
/// `pack` until its window is closed.
fn play_pack(pack: &Path) -> Result<(), String> {
    play_packed(pack, false)
}

/// Plays the game in the pack at `pack` until its window is closed. With
/// `others_may_join`, other players can join it meanwhile.
fn play_packed(pack: &Path, others_may_join: bool) -> Result<(), String> {
    let mut config = player_config();
    let packed = player::open(pack, &config.actions).map_err(|error| error.to_string())?;
    // The window carries the game's own name; one with none keeps the usual title.
    if !packed.name.is_empty() {
        config.title.clone_from(&packed.name);
    }
    start(config, packed.game, &packed.name, others_may_join)
}

/// `fossps package`: makes a download of the game in `folder` for `target`.
fn package(folder: &Path, target: Target, runtime_dir: Option<&Path>) -> Result<(), String> {
    /// What to do about a system that cannot be packaged for yet.
    const NOT_YET: &str = "This version of FOSSPS packages for Linux only: \
                           `fossps package --target linux`.";
    match target {
        Target::Windows => {
            return Err(format!("Packaging for Windows is not built yet. {NOT_YET}"));
        }
        Target::Macos => return Err(format!("Packaging for macOS is not built yet. {NOT_YET}")),
        Target::All | Target::Linux => {}
    }

    // Loading the script is the cheapest check there is: a game that does not
    // load for its maker will not load for a player.
    let game = open(folder, &Config::default(), "package")?;
    if game.host().last_error().is_some() {
        return Err(format!(
            "{ENTRY_SCRIPT} has a mistake (the error is shown above), so the game was not \
             packaged. Fix it, try it with `fossps check`, then run `fossps package` again."
        ));
    }
    drop(game);

    let packaged =
        package::package_linux(folder, runtime_dir).map_err(|error| error.to_string())?;
    print!("{}", cli::packaged(&packaged));
    if target == Target::All {
        println!(
            "\nPackages for Windows and macOS were not made: this version of FOSSPS \
             packages for Linux only."
        );
    }
    Ok(())
}

/// `fossps join`: plays on the server at `address` until the window is
/// closed or the server lets go.
fn join(address: &str) -> Result<(), String> {
    let mut config = player_config();
    let mut game = JoinedGame::connect(address, &config.actions, config.tick_rate)
        .map_err(|error| error.to_string())?;
    log::info!("Joining {address}...");
    let id = game
        .wait_until_joined(|| {})
        .map_err(|error| error.to_string())?;
    log::info!(
        "Joined as Player {}. Close the window to leave.",
        u32::from(id) + 1
    );
    config.title = format!("{address} | FOSSPS");
    let mut game = game.with_title(config.title.clone());
    fossps::run(config, &mut game).map_err(|error| error.to_string())?;
    // The window closing by itself needs a reason; the player closing it does not.
    game.closed().map_or(Ok(()), |error| Err(error.to_string()))
}

/// Looks for games on this network for a second.
fn search(port: u16) -> Result<Vec<FoundServer>, String> {
    log::info!("Looking for games on this network...");
    find_servers(port, SEARCH_TIME).map_err(|error| {
        let mut said = error.to_string();
        if let Some(first) = said.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        said
    })
}

/// `fossps servers`: lists the games found on this network.
fn servers(port: u16) -> Result<(), String> {
    let found = search(port)?;
    if found.is_empty() {
        println!("{}", cli::no_servers_found(port));
        return Ok(());
    }
    print!("{}", cli::server_list(&found));
    if let Some(server) = found.iter().find(|server| server.same_version) {
        println!(
            "Join one with its address, like `fossps join {}`.",
            cli::join_address(server.address)
        );
    }
    Ok(())
}

/// `fossps join` with no address: joins the game found on this network,
/// asking which when there are several.
fn find_and_join() -> Result<(), String> {
    let found = search(DEFAULT_PORT)?;
    let server = match found.as_slice() {
        [] => return Err(cli::no_servers_found(DEFAULT_PORT)),
        [only] => only,
        several => {
            let list = cli::server_list(several);
            if !std::io::stdin().is_terminal() {
                return Err(format!(
                    "Found {} games:\n{list}Say which one by its address, like \
                     `fossps join {}`.",
                    several.len(),
                    cli::join_address(several.first().map_or_else(
                        || std::net::SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
                        |server| server.address
                    ))
                ));
            }
            println!("Found {} games:\n{list}", several.len());
            match ask_which(several)? {
                Some(server) => server,
                None => return Ok(()),
            }
        }
    };
    if !server.same_version {
        return Err(format!(
            "Found {} at {}, but it runs another version of FOSSPS, so it cannot be joined. \
             Update FOSSPS on both computers to the same version.",
            cli::server_name(server),
            cli::join_address(server.address)
        ));
    }
    log::info!(
        "Found {} ({} of {} players).",
        cli::server_name(server),
        server.players,
        server.places
    );
    join(&cli::join_address(server.address))
}

/// Asks which of `found` to join until the answer is one of them, or
/// nothing.
fn ask_which(found: &[FoundServer]) -> Result<Option<&FoundServer>, String> {
    let mut line = String::new();
    loop {
        print!("Which one? Type its number and press Enter: ");
        std::io::stdout()
            .flush()
            .map_err(|error| error.to_string())?;
        line.clear();
        let read = std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        // Nothing to read at all: the terminal was closed.
        if read == 0 {
            return Ok(None);
        }
        match cli::chosen(&line, found.len()) {
            Ok(choice) => return Ok(choice.and_then(|place| found.get(place))),
            Err(advice) => println!("{advice}"),
        }
    }
}

/// `fossps controls`: shows the player's controls and where they are kept.
fn controls() -> Result<(), String> {
    let (said, problems) = cli::controls()?;
    print!("{said}");
    for problem in problems {
        log::warn!("{problem}");
    }
    Ok(())
}

/// The settings a game is played with: the usual ones, with the player's own
/// controls.
fn player_config() -> Config {
    let mut config = Config::default();
    cli::use_player_controls(&mut config.actions);
    config
}

/// `fossps demo`: shows live input in a window.
fn demo() -> Result<(), String> {
    log::info!(
        "This is the input demo. Click the window to capture the mouse, then use \
         keyboard, mouse or a gamepad. Escape or Start pauses: the cursor comes back \
         and the game hears nothing until you press it again."
    );
    let mut config = player_config();
    let demo = InputDemo::new(config.actions.clone());
    config.actions.clone_from(demo.actions());
    play(config, demo)
}

/// Runs `game` until its window is closed.
fn play(config: Config, game: impl Game) -> Result<(), String> {
    fossps::run(config, game).map_err(|error| error.to_string())
}
