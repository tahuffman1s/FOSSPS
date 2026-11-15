//! What the `fossps` and `fossps-server` commands share.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use fossps_input::ActionMap;
use fossps_script::{ENTRY_SCRIPT, OpenError};

use crate::online::{DEFAULT_PORT, FoundServer, HostedGame};
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
    let mut game = HostedGame::open(game, &format!("0.0.0.0:{port}"), config.tick_rate)
        .map_err(|error| {
            format!(
                "The server could not start: {error}\nTo use another port: \
                 `{command} --port {}`.",
                port.checked_add(1).unwrap_or(DEFAULT_PORT)
            )
        })?
        .named(&game_name(folder));
    let stop = StopSignal::on_ctrl_c().map_err(|error| error.to_string())?;

    log::info!(
        "Serving {} at {} ticks a second, with no window. Press Ctrl-C to stop.",
        folder.join(ENTRY_SCRIPT).display(),
        config.tick_rate
    );
    log::info!("{}", how_to_join(port, fossps_net::lan_address()));
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

/// The name players looking for a game are shown for the game in `folder`:
/// the folder's own name.
#[must_use]
pub fn game_name(folder: &Path) -> String {
    std::path::absolute(folder)
        .ok()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What to tell the person hosting about getting players in. `lan` is this
/// computer's address on its network, when it has one.
#[must_use]
pub fn how_to_join(port: u16, lan: Option<Ipv4Addr>) -> String {
    let suffix = if port == DEFAULT_PORT {
        String::new()
    } else {
        format!(":{port}")
    };
    let by_address = match lan {
        Some(lan) => format!("`fossps join {lan}{suffix}`"),
        None => format!(
            "`fossps join` and this computer's address, such as \
             `fossps join 192.168.1.20{suffix}`"
        ),
    };
    let on_network = if port == DEFAULT_PORT {
        format!(
            "Players on this network join with just `fossps join`, which finds this game \
             by itself, or by address: {by_address}."
        )
    } else {
        format!(
            "Players on this network join with {by_address}. (`fossps join` alone only \
             finds games on the usual port, {DEFAULT_PORT}.)"
        )
    };
    format!(
        "{on_network} On this computer: `fossps join localhost{suffix}`. Over the internet, \
         the router has to pass UDP port {port} on to this computer."
    )
}

/// `address` as a player would type it after `fossps join`: `localhost` for
/// this computer, and no port when it is the usual one.
#[must_use]
pub fn join_address(address: SocketAddr) -> String {
    match (address.ip().is_loopback(), address.port() == DEFAULT_PORT) {
        (true, true) => "localhost".to_owned(),
        (true, false) => format!("localhost:{}", address.port()),
        (false, true) => address.ip().to_string(),
        (false, false) => address.to_string(),
    }
}

/// The name of a found game, or its address when it gave no name.
#[must_use]
pub fn server_name(server: &FoundServer) -> String {
    if server.name.is_empty() {
        join_address(server.address)
    } else {
        server.name.clone()
    }
}

/// A numbered list of found games, one to a line: name, address to join,
/// how full it is.
#[must_use]
pub fn server_list(found: &[FoundServer]) -> String {
    let names: Vec<String> = found.iter().map(server_name).collect();
    let addresses: Vec<String> = found
        .iter()
        .map(|server| join_address(server.address))
        .collect();
    let width = |texts: &[String]| texts.iter().map(|text| text.chars().count()).max();
    let (name_width, address_width) = (width(&names).unwrap_or(0), width(&addresses).unwrap_or(0));
    let mut list = String::new();
    for (number, ((server, name), address)) in (1..).zip(found.iter().zip(&names).zip(&addresses)) {
        // Writing to a `String` cannot fail.
        let _ = write!(
            list,
            "  {number}. {name:name_width$}   {address:address_width$}   {} of {} players",
            server.players, server.places
        );
        if server.address.ip().is_loopback() {
            list.push_str(" (on this computer)");
        }
        if !server.same_version {
            list.push_str(" (another version of FOSSPS: update to join)");
        }
        list.push('\n');
    }
    list
}

/// What to say when a search on the UDP port `port` found no game.
#[must_use]
pub fn no_servers_found(port: u16) -> String {
    format!(
        "No games were found on this network.\n\
         - Is one running? Start one with `fossps serve` inside a game's folder, or \
         `fossps run --open` to play in it too.\n\
         - Are both computers on the same network? Games elsewhere on the internet are \
         not found: join those by address, like `fossps join example.com`.\n\
         - The server's firewall has to let UDP (not TCP) through on port {port}. \
         If the search is being blocked, the address still works: `fossps join 192.168.1.20`, \
         with the address the server showed when it started."
    )
}

/// What a person meant by `line`, typed to pick one of `count` listed
/// games: `Ok(None)` for nothing (an empty line), otherwise the place in
/// the list, counted from zero.
///
/// # Errors
///
/// Fails with what to type instead when `line` is not a number on the list.
pub fn chosen(line: &str, count: usize) -> Result<Option<usize>, String> {
    let line = line.trim().trim_end_matches('.');
    if line.is_empty() {
        return Ok(None);
    }
    match line.parse::<usize>() {
        Ok(number) if (1..=count).contains(&number) => Ok(Some(number - 1)),
        _ => Err(format!(
            "`{line}` is not on the list. Type a number from 1 to {count}, or press Enter \
             alone to join nothing."
        )),
    }
}

/// Reads a picture size typed as `1280x720`.
///
/// # Errors
///
/// Fails with what to type instead.
#[cfg(feature = "window")]
pub fn picture_size(text: &str) -> Result<(u32, u32), String> {
    /// The largest picture every Vulkan 1.3 graphics card can draw.
    const LARGEST: u32 = 4096;
    let wrong = || {
        format!(
            "a size is a width and a height in pixels with an x between them, each from 1 \
             to {LARGEST}, such as 1920x1080"
        )
    };
    let (width, height) = text.split_once(['x', 'X']).ok_or_else(wrong)?;
    let side = |side: &str| {
        side.trim()
            .parse()
            .ok()
            .filter(|pixels| (1..=LARGEST).contains(pixels))
            .ok_or_else(wrong)
    };
    Ok((side(width)?, side(height)?))
}

/// Reads an upscaling setting typed by its name, such as `quality`.
///
/// # Errors
///
/// Fails with the names there are.
#[cfg(feature = "window")]
pub fn upscaling(text: &str) -> Result<fossps_render::Upscaling, String> {
    use fossps_render::Upscaling;
    const ALL: [Upscaling; 6] = [
        Upscaling::Off,
        Upscaling::Native,
        Upscaling::Quality,
        Upscaling::Balanced,
        Upscaling::Performance,
        Upscaling::UltraPerformance,
    ];
    let typed = text.trim().to_lowercase().replace('_', "-");
    ALL.into_iter()
        .find(|&upscaling| crate::bench::upscaling_name(upscaling) == typed)
        .ok_or_else(|| {
            let names: Vec<&str> = ALL.into_iter().map(crate::bench::upscaling_name).collect();
            format!("the upscaling settings are {}", names.join(", "))
        })
}

/// Reads a frame-time budget typed in milliseconds, such as `16.6`.
///
/// # Errors
///
/// Fails with what to type instead.
#[cfg(feature = "window")]
pub fn budget(text: &str) -> Result<f64, String> {
    text.trim()
        .parse()
        .ok()
        .filter(|budget: &f64| budget.is_finite() && *budget > 0.0)
        .ok_or_else(|| {
            "a budget is a number of milliseconds above 0, such as 16.6 (one frame at 60 \
             frames a second)"
                .to_owned()
        })
}

/// `fossps bench`: draws the benchmark with no window, prints how long its
/// frames took, and writes the numbers to `json` if a file is named.
///
/// # Errors
///
/// Fails with a message for the person if the frames could not be drawn,
/// the file could not be written, or a slow frame took longer than
/// `budget` milliseconds.
#[cfg(feature = "window")]
pub fn bench(
    bench: &crate::bench::Bench,
    budget: Option<f64>,
    json: Option<&Path>,
) -> Result<(), String> {
    log::info!(
        "Drawing {} frames with no window. This takes a few seconds on a graphics card \
         and a few minutes without one.",
        u64::from(bench.frames) + u64::from(bench.warmup)
    );
    let report = crate::bench::run(bench)
        .map_err(|error| format!("The benchmark could not be drawn: {error}"))?;
    print!("{}", report.text());
    if let Some(file) = json {
        std::fs::write(file, report.json(budget)).map_err(|error| {
            format!(
                "The numbers could not be written to {}: {error}\nName a file in a folder \
                 that exists and that you may write to, such as `--json bench.json`.",
                file.display()
            )
        })?;
        println!("\nThe numbers are in {} as JSON.", file.display());
    }
    match budget {
        None => Ok(()),
        Some(budget) if report.within(Some(budget)) => {
            println!(
                "\nWithin budget: a slow frame took {:.2} ms and may take {budget} ms.",
                report.frame.slow
            );
            Ok(())
        }
        Some(budget) => Err(format!(
            "\nOver budget: a slow frame took {:.2} ms and may take {budget} ms.\nIf other \
             programs were busy, close them and run it again. If it stays over, something \
             made drawing slower: compare with the last version that was within budget.",
            report.frame.slow
        )),
    }
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
        let usual = how_to_join(DEFAULT_PORT, None);
        assert!(usual.contains("`fossps join localhost`"), "{usual}");
        assert!(usual.contains("just `fossps join`"), "{usual}");
        let other = how_to_join(7000, None);
        assert!(other.contains("`fossps join localhost:7000`"), "{other}");
        assert!(other.contains("`fossps join 192.168.1.20:7000`"), "{other}");
        assert!(other.contains("UDP port 7000"), "{other}");
        assert!(!other.contains("just `fossps join`"), "{other}");
    }

    #[test]
    fn joining_advice_gives_this_computers_own_address_when_it_has_one() {
        let lan = Some(Ipv4Addr::new(10, 0, 0, 7));
        let usual = how_to_join(DEFAULT_PORT, lan);
        assert!(usual.contains("`fossps join 10.0.0.7`"), "{usual}");
        assert!(!usual.contains("192.168.1.20"), "{usual}");
        let other = how_to_join(7000, lan);
        assert!(other.contains("`fossps join 10.0.0.7:7000`"), "{other}");
    }

    fn server(name: &str, address: &str, players: u8) -> FoundServer {
        FoundServer {
            address: address.parse().unwrap(),
            name: name.to_owned(),
            players,
            places: 64,
            same_version: true,
        }
    }

    #[test]
    fn found_games_are_listed_in_columns_with_the_address_to_type() {
        let mut old = server("old", "192.168.1.9:24680", 0);
        old.same_version = false;
        let found = [
            server("zombie-night", "192.168.1.20:24680", 2),
            server("", "127.0.0.1:7000", 0),
            old,
        ];
        assert_eq!(
            server_list(&found),
            "  1. zombie-night     192.168.1.20     2 of 64 players\n  \
             2. localhost:7000   localhost:7000   0 of 64 players (on this computer)\n  \
             3. old              192.168.1.9      0 of 64 players (another version of \
             FOSSPS: update to join)\n"
        );
    }

    #[test]
    fn a_typed_choice_is_a_number_on_the_list_or_nothing() {
        assert_eq!(chosen("2\n", 3), Ok(Some(1)));
        assert_eq!(chosen(" 1. ", 3), Ok(Some(0)));
        assert_eq!(chosen("\n", 3), Ok(None));
        for wrong in ["0", "4", "two", "-1"] {
            let said = chosen(wrong, 3).unwrap_err();
            assert!(said.contains("from 1 to 3"), "{said}");
        }
    }

    #[test]
    fn names_with_spaces_are_quoted() {
        assert_eq!(quoted("my-game"), "my-game");
        assert_eq!(quoted("Zombie Night"), "\"Zombie Night\"");
    }
}
