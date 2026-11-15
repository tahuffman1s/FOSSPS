//! What the `fossps` and `fossps-server` commands share.

use std::path::Path;
use std::time::Duration;

use fossps_input::ActionMap;
use fossps_script::{ENTRY_SCRIPT, OpenError};

use crate::online::{DEFAULT_PORT, HostedGame};
use crate::script_game::ScriptGame;
use crate::server::{ServerConfig, StopSignal, serve};

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

/// Sends log messages to standard error, unless a logger is already installed.
pub fn log_to_stderr() {
    if log::set_logger(&LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Opens the game in `folder`, or says how to get one. `command` is what the
/// person typed before the folder, such as `fossps run`, and is used in the
/// advice.
///
/// # Errors
///
/// Fails with a message for the person if `folder` is not a game's folder.
pub fn open_game(folder: &Path, actions: &ActionMap, command: &str) -> Result<ScriptGame, String> {
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
    ScriptGame::open(folder, actions).map_err(|error| match error {
        OpenError::Missing { .. } => format!(
            "There is no game here: {} has no {ENTRY_SCRIPT} in it.\n\
             Run this from inside a game's folder, or name the folder: \
             `{command} my-game`.\n\
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

/// Runs the game in `folder` as a dedicated server until Ctrl-C, taking
/// players on the UDP port `port`. `command` is what the person typed before
/// the folder, such as `fossps serve`.
///
/// # Errors
///
/// Fails with a message for the person if `folder` is not a game's folder or
/// the server could not start.
pub fn serve_game(folder: &Path, command: &str, port: u16) -> Result<(), String> {
    let config = ServerConfig::default();
    let game = open_game(folder, &config.actions, command)?;
    let mut game =
        HostedGame::open(game, &format!("0.0.0.0:{port}"), config.tick_rate).map_err(|error| {
            format!(
                "The server could not start: {error}\nTo use another port: \
                 `{command} --port {}`.",
                port.checked_add(1).unwrap_or(DEFAULT_PORT)
            )
        })?;
    let stop = StopSignal::on_ctrl_c().map_err(|error| error.to_string())?;

    log::info!(
        "Serving {} at {} ticks a second, with no window. Press Ctrl-C to stop.",
        folder.join(ENTRY_SCRIPT).display(),
        config.tick_rate
    );
    log::info!("{}", how_to_join(port));
    let report = serve(&config, &mut game, &stop);
    log::info!(
        "Server stopped after {}. It ran {} ticks{}.",
        long_time(report.uptime),
        report.ticks,
        if report.skipped == 0 {
            String::new()
        } else {
            format!(" and skipped {}", report.skipped)
        }
    );
    Ok(())
}

/// What to tell the person hosting about getting players in.
fn how_to_join(port: u16) -> String {
    let suffix = if port == DEFAULT_PORT {
        String::new()
    } else {
        format!(":{port}")
    };
    format!(
        "Players can join on UDP port {port}. On this computer: `fossps join localhost{suffix}`. \
         From another computer: `fossps join` and this computer's address, such as \
         `fossps join 192.168.1.20{suffix}`. Over the internet, the router has to pass \
         UDP port {port} on to this computer."
    )
}

/// `text` as one word for a command line: in quotes if it has a space.
#[must_use]
pub fn quoted(text: &str) -> String {
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.to_owned()
    }
}

/// `time` in hours, minutes and seconds, leaving out the parts that are zero
/// at the front: `2 h 5 min 9 s`, `5 min 9 s`, `9 s`.
fn long_time(time: Duration) -> String {
    let seconds = time.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours} h {minutes} min {seconds} s")
    } else if minutes > 0 {
        format!("{minutes} min {seconds} s")
    } else {
        format!("{seconds} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_times_leave_out_leading_zero_parts() {
        assert_eq!(long_time(Duration::from_millis(9_400)), "9 s");
        assert_eq!(long_time(Duration::from_secs(309)), "5 min 9 s");
        assert_eq!(long_time(Duration::from_secs(7_509)), "2 h 5 min 9 s");
        assert_eq!(long_time(Duration::from_secs(7_200)), "2 h 0 min 0 s");
    }

    #[test]
    fn joining_advice_names_the_port_only_when_it_is_not_the_usual_one() {
        let usual = how_to_join(DEFAULT_PORT);
        assert!(usual.contains("`fossps join localhost`"), "{usual}");
        let other = how_to_join(7000);
        assert!(other.contains("`fossps join localhost:7000`"), "{other}");
        assert!(other.contains("UDP port 7000"), "{other}");
    }

    #[test]
    fn names_with_spaces_are_quoted() {
        assert_eq!(quoted("my-game"), "my-game");
        assert_eq!(quoted("Zombie Night"), "\"Zombie Night\"");
    }
}
