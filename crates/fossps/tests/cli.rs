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
