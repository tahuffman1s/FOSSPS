//! Runs the built `fossps` command the way a person would.
//!
//! Playing a game needs a window and a graphics card, so `fossps run` is only
//! tested up to the point where it would open one.

#![cfg(all(feature = "window", feature = "cli"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// An empty folder for one test.
fn scratch(test: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{test}"));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

/// Runs `fossps` with `args` from inside `folder`.
fn fossps(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fossps"))
        .args(args)
        .current_dir(folder)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

#[test]
fn new_creates_a_game_and_says_how_to_play_it() {
    let folder = scratch("new");

    let output = fossps(&folder, &["new", "my-game"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("Created the game \"my-game\""), "{said}");
    assert!(said.contains("    cd my-game\n"), "{said}");
    assert!(said.contains("    fossps run\n"), "{said}");
    for file in [
        "main.luau",
        "README.md",
        "fossps.d.luau",
        ".vscode/settings.json",
        "CLAUDE.md",
        ".claude/settings.json",
    ] {
        assert!(folder.join("my-game").join(file).is_file(), "{file}");
    }
}

#[test]
fn new_in_the_current_folder_does_not_say_cd() {
    let folder = scratch("new-here");

    let output = fossps(&folder, &["new", "."]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains("cd "), "{}", stdout(&output));
    assert!(folder.join("main.luau").is_file());
}

#[test]
fn new_refuses_a_folder_that_has_files() {
    let folder = scratch("new-taken");
    fs::create_dir(folder.join("taken")).unwrap();
    fs::write(folder.join("taken/notes.txt"), "mine").unwrap();

    let output = fossps(&folder, &["new", "taken"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("already has files in it"),
        "{}",
        stderr(&output)
    );
    assert_eq!(fs::read_dir(folder.join("taken")).unwrap().count(), 1);
}

#[test]
fn new_refuses_a_name_that_breaks_on_windows() {
    let folder = scratch("new-reserved");

    let output = fossps(&folder, &["new", "con"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("Try a name like my-game"),
        "{}",
        stderr(&output)
    );
    assert!(!folder.join("con").exists());
}

#[test]
fn new_without_a_name_shows_how_to_use_it() {
    let folder = scratch("new-nameless");

    let output = fossps(&folder, &["new"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("fossps new <FOLDER>"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn run_outside_a_game_says_how_to_get_one() {
    let folder = scratch("run-empty");

    let output = fossps(&folder, &["run"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("There is no game here"), "{said}");
    assert!(said.contains("fossps new my-game"), "{said}");
}

#[test]
fn run_with_a_folder_that_does_not_exist_says_so() {
    let folder = scratch("run-missing");

    let output = fossps(&folder, &["run", "nowhere"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("There is no folder called nowhere"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn run_with_a_file_asks_for_the_folder() {
    let folder = scratch("run-file");
    fs::write(folder.join("main.luau"), "").unwrap();

    let output = fossps(&folder, &["run", "main.luau"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("main.luau is a file"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn no_arguments_shows_help_and_succeeds() {
    let folder = scratch("bare");

    let output = fossps(&folder, &[]);

    assert!(output.status.success());
    let said = stdout(&output);
    assert!(said.contains("Usage: fossps"), "{said}");
    assert!(said.contains("fossps new my-game"), "{said}");
}

#[test]
fn a_misspelled_command_suggests_the_right_one() {
    let folder = scratch("typo");

    let output = fossps(&folder, &["runn"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("'run'"), "{}", stderr(&output));
}

#[test]
fn check_plays_a_pretend_match_through_the_team_deathmatch_example() {
    let folder = scratch("check-mode");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team-deathmatch");

    let output = fossps(&folder, &["check", &example.display().to_string()]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("a pretend match was played"),
        "{}",
        stdout(&output)
    );
    // The mode's own functions really ran.
    for line in [
        "Check Bot 1 joined Red.",
        "Check Bot 1 killed Check Bot 2.",
        "Red wins the round!",
        "Check Bot 2 left.",
    ] {
        assert!(stderr(&output).contains(line), "{}", stderr(&output));
    }
}

#[test]
fn check_reports_a_mistake_that_only_a_match_reaches() {
    let folder = scratch("check-mode-mistake");
    assert!(fossps(&folder, &["new", "game"]).status.success());
    std::fs::write(
        folder.join("game").join("main.luau"),
        "game.onKill(function(killer, victim) print(victim.nmae) end)",
    )
    .unwrap();

    let output = fossps(&folder, &["check", "game"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("player.nmae does not exist. Did you mean player.name?"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn check_passes_a_new_game_without_a_window() {
    let folder = scratch("check-ok");
    assert!(fossps(&folder, &["new", "game"]).status.success());

    let output = fossps(&folder, &["check", "game"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("main.luau is fine"),
        "{}",
        stdout(&output)
    );
    // The start handler really ran.
    assert!(
        stderr(&output).contains("Welcome to game!"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn check_fails_on_a_mistake_that_only_shows_while_running() {
    let folder = scratch("check-broken");
    fs::write(
        folder.join("main.luau"),
        "game.onTick(function(step)\n    local nothing = nil\n    print(nothing.x)\nend)\n",
    )
    .unwrap();

    let output = fossps(&folder, &["check"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("Script error in main.luau, line 3"), "{said}");
    assert!(said.contains("run `fossps check` again"), "{said}");
}

#[test]
fn check_fails_on_a_script_that_does_not_load() {
    let folder = scratch("check-typo");
    fs::write(folder.join("main.luau"), "game.onTik(function() end)\n").unwrap();

    let output = fossps(&folder, &["check"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("Did you mean game.onTick?"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn serve_in_a_folder_with_no_game_says_what_to_do() {
    let folder = scratch("serve-empty");

    let output = fossps(&folder, &["serve"]);

    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(said.contains("There is no game here"), "{said}");
    assert!(said.contains("`fossps serve my-game`"), "{said}");
}

/// A UDP port nothing is using right now.
fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn join_with_no_server_there_says_what_to_check() {
    let address = format!("127.0.0.1:{}", free_port());

    let output = fossps(&scratch("join-nobody"), &["join", &address]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(
        said.contains(&format!("Could not join {address}: no server answered.")),
        "{said}"
    );
    assert!(said.contains("`fossps serve`"), "{said}");
    assert!(said.contains("UDP (not TCP)"), "{said}");
}

#[test]
fn join_with_an_address_that_cannot_be_read_says_how_to_write_one() {
    let output = fossps(&scratch("join-unreadable"), &["join", "localhost:port"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("Could not join"), "{said}");
    assert!(said.contains("`localhost:port`"), "{said}");
    assert!(said.contains("The port can be left out"), "{said}");
}

#[test]
fn serve_on_a_port_that_is_taken_says_how_to_choose_another() {
    let folder = scratch("serve-taken");
    assert!(fossps(&folder, &["new", "."]).status.success());
    let taken = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    let port = taken.local_addr().unwrap().port();

    let output = fossps(&folder, &["serve", "--port", &port.to_string()]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("The server could not start"), "{said}");
    assert!(said.contains("`fossps serve --port "), "{said}");
}

#[test]
fn a_player_joins_a_running_serve_and_walks() {
    use fossps::Game;
    use fossps::input::{ActionMap, Input, Key};
    use fossps::online::JoinedGame;
    use std::time::Duration;

    /// Stops the server even when the test fails.
    struct Running(std::process::Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let folder = scratch("serve-join");
    assert!(fossps(&folder, &["new", "."]).status.success());
    let port = free_port();
    let _server = Running(
        Command::new(env!("CARGO_BIN_EXE_fossps"))
            .args(["serve", "--port", &port.to_string()])
            .current_dir(&folder)
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );

    let actions = ActionMap::default_fps();
    let address = format!("127.0.0.1:{port}");
    // The server may still be starting: each try asks for five seconds.
    let mut tries = 0;
    let mut joined = loop {
        let mut joined = JoinedGame::connect(&address, &actions, 60).unwrap();
        match joined.wait_until_joined(|| {}) {
            Ok(_) => break joined,
            Err(error) if tries == 5 => panic!("{error}"),
            Err(_) => tries += 1,
        }
    };

    let mut input = Input::new(actions);
    input.press(Key::W);
    let step = Duration::from_secs_f32(1.0 / 60.0);
    for _ in 0..90 {
        joined.tick(input.tick(), step.as_secs_f32());
        std::thread::sleep(step);
    }

    let here = joined.player().position;
    assert!(here[2] < -2.0, "{here:?}");
    assert!(joined.closed().is_none());
}

#[test]
fn servers_with_nothing_being_served_says_what_to_check() {
    let output = fossps(
        &scratch("servers-none"),
        &["servers", "--port", &free_port().to_string()],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("No games were found on this network."),
        "{said}"
    );
    assert!(said.contains("`fossps serve`"), "{said}");
    assert!(said.contains("`fossps run --open`"), "{said}");
    assert!(said.contains("UDP (not TCP)"), "{said}");
}

#[test]
fn servers_lists_a_running_serve_by_its_folder_name() {
    /// Stops the server even when the test fails.
    struct Running(std::process::Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let folder = scratch("servers-found").join("zombie-night");
    assert!(
        fossps(folder.parent().unwrap(), &["new", "zombie-night"])
            .status
            .success()
    );
    let port = free_port();
    let _server = Running(
        Command::new(env!("CARGO_BIN_EXE_fossps"))
            .args(["serve", "--port", &port.to_string()])
            .current_dir(&folder)
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );

    // The server may still be starting: each try looks for a second.
    let mut said = String::new();
    for _ in 0..10 {
        let output = fossps(&folder, &["servers", "--port", &port.to_string()]);
        assert!(output.status.success(), "{}", stderr(&output));
        said = stdout(&output);
        if said.contains("zombie-night") {
            break;
        }
    }
    assert!(
        said.contains(&format!(
            "  1. zombie-night   localhost:{port}   0 of 64 players (on this computer)"
        )),
        "{said}"
    );
    assert!(
        said.contains(&format!("`fossps join localhost:{port}`")),
        "{said}"
    );
}

#[test]
fn bench_says_how_to_write_a_size_an_upscaling_and_a_budget() {
    let folder = scratch("bench-wrong");
    for (args, expected) in [
        (["bench", "--size", "big"], "such as 1920x1080"),
        (["bench", "--size", "0x720"], "such as 1920x1080"),
        (["bench", "--upscaling", "best"], "ultra-performance"),
        (["bench", "--budget-ms", "0"], "such as 16.6"),
        (["bench", "--frames", "0"], "0 is not in"),
    ] {
        let output = fossps(&folder, &args);
        assert!(!output.status.success(), "{args:?}");
        let said = stderr(&output);
        assert!(said.contains(expected), "{args:?}: {said}");
    }
}

/// Draws for real where there is a graphics card or a software one, as on
/// the Linux machines of the automated checks; elsewhere `fossps bench`
/// must say why it cannot.
#[test]
fn bench_draws_and_reports_or_says_why_it_cannot() {
    let folder = scratch("bench");
    let args = [
        "bench",
        "--frames",
        "4",
        "--warmup",
        "1",
        "--size",
        "96x64",
        "--json",
        "bench.json",
    ];

    let output = fossps(&folder, &args);

    if !output.status.success() {
        let said = stderr(&output);
        assert!(said.contains("The benchmark could not be drawn"), "{said}");
        let required = std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
        assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {said}");
        return;
    }
    let said = stdout(&output);
    assert!(said.contains("Drew 4 frames"), "{said}");
    assert!(said.contains("Slow frame"), "{said}");
    let json = fs::read_to_string(folder.join("bench.json")).unwrap();
    assert!(json.contains("\"frames\": 4"), "{json}");
    assert!(json.contains("\"within_budget\": true"), "{json}");

    // No frame is drawn in a millionth of a millisecond.
    let output = fossps(
        &folder,
        &[&args[..7], &["--budget-ms", "0.000001"]].concat(),
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Over budget"),
        "{}",
        stderr(&output)
    );
}

/// A game called `name` in `folder`, and beside it a folder of pretend
/// engine programs to package it with. Returns the runtime folder.
fn game_to_package(folder: &Path, name: &str) -> PathBuf {
    assert!(fossps(folder, &["new", name]).status.success());
    let runtime = folder.join("runtime");
    fs::create_dir(&runtime).unwrap();
    fs::write(
        runtime.join("fossps-linux-x86_64"),
        b"\x7fELF pretend program",
    )
    .unwrap();
    runtime
}

#[test]
fn package_makes_a_linux_download_and_says_where_it_is() {
    let folder = scratch("package");
    let runtime = game_to_package(&folder, "Zombie Night");
    let game = folder.join("Zombie Night");

    let output = fossps(
        &game,
        &[
            "package",
            "--target",
            "linux",
            "--runtime-dir",
            runtime.to_str().unwrap(),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("Packaged \"Zombie Night\" for Linux"),
        "{said}"
    );
    assert!(said.contains("Zombie-Night-linux-x86_64.tar.gz"), "{said}");
    assert!(said.contains("runs Zombie-Night/Zombie-Night."), "{said}");
    assert!(said.contains("called icon.png"), "{said}");
    let archive = game.join("dist/Zombie-Night-linux-x86_64.tar.gz");
    // A gzip file, with something in it.
    let bytes = fs::read(archive).unwrap();
    assert!(bytes.starts_with(&[0x1f, 0x8b]) && bytes.len() > 100);
}

#[test]
fn package_for_every_system_says_which_were_not_made() {
    let folder = scratch("package-all");
    let runtime = game_to_package(&folder, "my-game");

    let output = fossps(
        &folder,
        &[
            "package",
            "my-game",
            "--target",
            "all",
            "--runtime-dir",
            runtime.to_str().unwrap(),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("Packaged \"my-game\" for Linux"), "{said}");
    assert!(said.contains("Windows and macOS were not made"), "{said}");
    assert!(
        folder
            .join("my-game/dist/my-game-linux-x86_64.tar.gz")
            .is_file()
    );
}

#[test]
fn package_for_a_system_that_is_not_built_yet_says_what_works() {
    let folder = scratch("package-windows");
    game_to_package(&folder, "my-game");

    for system in ["windows", "macos"] {
        let output = fossps(&folder, &["package", "my-game", "--target", system]);

        assert_eq!(output.status.code(), Some(1));
        let said = stderr(&output);
        assert!(said.contains("is not built yet"), "{said}");
        assert!(said.contains("`fossps package --target linux`"), "{said}");
    }
    assert!(!folder.join("my-game/dist").exists());
}

#[test]
fn package_refuses_a_game_whose_script_has_a_mistake() {
    let folder = scratch("package-mistake");
    let runtime = game_to_package(&folder, "my-game");
    fs::write(folder.join("my-game/main.luau"), "game.onTik()").unwrap();

    let output = fossps(
        &folder,
        &[
            "package",
            "my-game",
            "--target",
            "linux",
            "--runtime-dir",
            runtime.to_str().unwrap(),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("Did you mean game.onTick?"), "{said}");
    assert!(said.contains("so the game was not packaged"), "{said}");
    assert!(!folder.join("my-game/dist").exists());
}

#[test]
fn package_outside_a_game_says_how_to_get_one() {
    let folder = scratch("package-nothing");

    let output = fossps(&folder, &["package", "--target", "linux"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("There is no game here"), "{said}");
    assert!(said.contains("`fossps package my-game`"), "{said}");
}

#[test]
fn a_player_given_something_that_is_not_a_pack_says_so() {
    let folder = scratch("player-not-a-pack");
    fs::write(folder.join("Game.fpk"), "not a pack").unwrap();

    // As a packaged game is started, and as its maker tries a pack.
    for args in [["--pack", "Game.fpk"], ["run", "Game.fpk"]] {
        let output = fossps(&folder, &args);

        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let said = stderr(&output);
        assert!(
            said.contains("is not a FOSSPS game pack"),
            "{args:?}: {said}"
        );
    }
}

#[test]
fn a_player_shows_help_under_the_games_own_options() {
    let folder = scratch("player-help");

    let output = fossps(&folder, &["--pack", "Game.fpk", "--help"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("Plays the game packed with this program."),
        "{said}"
    );
    assert!(said.contains("--pack <FILE>"), "{said}");
    // None of the `fossps` command's own commands are offered.
    assert!(!said.contains("serve"), "{said}");
}
