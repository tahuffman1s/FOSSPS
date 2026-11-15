//! The `fossps-server` command: runs a game as a dedicated server.
//!
//! It does what `fossps serve` does, in a program with no windowing, graphics
//! or gamepad code in it, for machines that only ever host.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "fossps-server",
    // Otherwise help on Windows says `fossps-server.exe`, unlike every example.
    bin_name = "fossps-server",
    version,
    about = "Run a FOSSPS game as a dedicated server: no window, no graphics card needed.",
    after_help = "Examples:\n  fossps-server                Serve the game in this folder\n  \
                  fossps-server my-game        Serve the game in the folder my-game\n  \
                  fossps-server --port 7000    Take players on UDP port 7000\n\n\
                  Press Ctrl-C to stop the server."
)]
struct Cli {
    /// The game's folder. Leave it out to serve the game in the folder you are in
    folder: Option<PathBuf>,
    /// The UDP port players join through
    #[arg(long, default_value_t = fossps::online::DEFAULT_PORT, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}

fn main() -> ExitCode {
    fossps::cli::log_to_stderr();
    let cli = Cli::parse();
    let folder = cli.folder.as_deref().unwrap_or(Path::new("."));
    match fossps::cli::serve_game(folder, "fossps-server", cli.port) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
