//! Runs the built `fossps-server` command the way a person hosting would.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// An empty folder for one test.
fn scratch(test: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("server-{test}"));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

fn server(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fossps-server"))
        .args(args)
        .current_dir(folder)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn help_shows_how_to_serve_and_how_to_stop() {
    let output = server(&scratch("help"), &["--help"]);

    assert!(output.status.success());
    let said = text(&output.stdout);
    assert!(said.contains("Usage: fossps-server [FOLDER]"), "{said}");
    assert!(said.contains("fossps-server my-game"), "{said}");
    assert!(said.contains("Press Ctrl-C to stop the server."), "{said}");
}

#[test]
fn a_folder_with_no_game_says_what_to_do() {
    let output = server(&scratch("empty"), &[]);

    assert!(!output.status.success());
    let said = text(&output.stderr);
    assert!(
        said.contains("There is no game here: this folder has no main.luau"),
        "{said}"
    );
    assert!(said.contains("`fossps-server my-game`"), "{said}");
    assert!(said.contains("`fossps new my-game`"), "{said}");
}

/// The server must stay free of windowing, graphics, gamepad and sound
/// libraries, so it runs on a bare machine and stays small.
#[test]
fn links_no_window_graphics_or_gamepad_libraries() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "fossps-server",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    let tree = text(&output.stdout);
    assert!(tree.contains("fossps-script"), "{tree}");
    for unwanted in ["winit", "ash", "gilrs", "cpal", "fossps-render"] {
        assert!(
            !tree
                .lines()
                .any(|line| line.split(' ').next() == Some(unwanted)),
            "fossps-server depends on {unwanted}:\n{tree}"
        );
    }
}

/// Ctrl-C is sent with `kill`, which only Linux and macOS have.
#[cfg(unix)]
#[test]
fn runs_the_script_until_ctrl_c_then_stops_cleanly() {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;

    let folder = scratch("ctrl-c");
    fs::write(
        folder.join("main.luau"),
        "local ticks = 0\n\
         game.onStart(function() print(\"started\") end)\n\
         game.onTick(function(step)\n\
             ticks += 1\n\
             if ticks == 5 then print(\"five ticks\") end\n\
         end)\n",
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_fossps-server"))
        .current_dir(&folder)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait until the game has really ticked before asking it to stop. What a
    // script prints goes to standard error with the rest of the log.
    let mut log = BufReader::new(child.stderr.take().unwrap());
    let mut said = String::new();
    while !said.contains("five ticks") {
        assert_ne!(log.read_line(&mut said).unwrap(), 0, "ended early: {said}");
    }
    assert!(said.contains("started"), "{said}");
    assert!(
        said.contains("at 60 ticks a second, with no window"),
        "{said}"
    );

    let killed = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());

    log.read_to_string(&mut said).unwrap();
    assert!(child.wait().unwrap().success(), "{said}");
    assert!(said.contains("Server stopped after"), "{said}");
}
