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
        // Never the controls of whoever runs the tests.
        .env("FOSSPS_BINDINGS", folder.join("bindings.toml"))
        .output()
        .unwrap()
}

#[test]
fn controls_makes_the_file_then_shows_what_the_player_changed_in_it() {
    let folder = scratch("controls");
    let file = folder.join("bindings.toml");

    let output = fossps(&folder, &["controls"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.starts_with("Your controls:\n"), "{said}");
    assert!(
        said.contains("\n  jump             Space, GamepadSouth\n"),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "\nThey are kept in {}, made just now.\n",
            file.display()
        )),
        "{said}"
    );
    assert_eq!(stderr(&output), "");
    let written = fs::read_to_string(&file).unwrap();
    assert!(
        written.contains("\n# jump = [\"Space\", \"GamepadSouth\"]\n"),
        "{written}"
    );

    // The player takes the # away, as the file says to, and picks keys.
    let edited = written
        .replace(
            "# jump = [\"Space\", \"GamepadSouth\"]",
            "jump = [\"F\", \"GamepadSouth\"]",
        )
        .replace(
            "# reload = [\"R\", \"GamepadWest\"]",
            "reload = [\"F\", \"GamepadWset\"]",
        )
        .replace("# melee = [\"V\", \"RightStickPress\"]", "melee = []");
    fs::write(&file, &edited).unwrap();

    let output = fossps(&folder, &["controls"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("\n  jump             F, GamepadSouth  (changed by you)\n"),
        "{said}"
    );
    assert!(
        said.contains("\n  melee            nothing  (changed by you)\n"),
        "{said}"
    );
    assert!(
        said.contains("\n  reload           R, GamepadWest\n"),
        "{said}"
    );
    assert!(
        said.contains(&format!("\nThey are kept in {}.\n", file.display())),
        "{said}"
    );
    let warned = stderr(&output);
    assert!(
        warned.contains(&format!(
            "{}: Line 20: \"GamepadWset\" is not a known binding. Did you mean \
             \"GamepadWest\"? Action \"reload\" keeps the game's own controls.",
            file.display()
        )),
        "{warned}"
    );
    assert_eq!(warned.lines().count(), 1, "{warned}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        edited,
        "left as written"
    );
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
        "level.glb",
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
fn edit_outside_a_game_says_how_to_get_one_and_opens_no_window() {
    let folder = scratch("edit-empty");

    let output = fossps(&folder, &["edit"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("There is no game here"), "{said}");
    assert!(said.contains("`fossps edit my-game`"), "{said}");

    let output = fossps(&folder, &["edit", "nowhere"]);
    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("There is no folder called nowhere"), "{said}");
}

#[test]
fn edit_with_a_packaged_game_asks_for_its_folder() {
    let folder = scratch("edit-pack");
    fs::write(folder.join("Game.fpk"), "").unwrap();

    let output = fossps(&folder, &["edit", "Game.fpk"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(
        said.contains("Game.fpk is a packaged game, which cannot be edited"),
        "{said}"
    );
}

#[test]
fn edit_help_says_how_to_fly_with_a_mouse_and_with_a_gamepad() {
    let folder = scratch("edit-help");

    let output = fossps(&folder, &["edit", "--help"]);

    assert!(output.status.success());
    let said = stdout(&output);
    assert!(said.contains("Hold the right mouse button"), "{said}");
    assert!(said.contains("On a gamepad"), "{said}");
    assert!(
        said.contains("saved in\nmap.toml in the game's folder"),
        "{said}"
    );
    let listed = stdout(&fossps(&folder, &[]));
    assert!(listed.contains("edit "), "{listed}");
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
fn check_reports_a_mistake_that_only_a_press_reaches_and_names_the_action() {
    let folder = scratch("check-press-mistake");
    let script = [
        "game.onTick(function()",
        "    if game.input.justPressed(\"fire\") then",
        "        local weapon = nil",
        "        print(weapon.ammo)",
        "    end",
        "    local x, y = game.input.axis(\"move\")",
        "    if y < -0.5 then",
        "        print(missing.speed)",
        "    end",
        "end)",
        // Wrong on every tick, pressed or not: no action is blamed for it.
        "game.onTick(function() print(always.wrong) end)",
    ];
    fs::write(folder.join("main.luau"), script.join("\n")).unwrap();

    let output = fossps(&folder, &["check"]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let said = stderr(&output);
    // Both are found in one run: the check carries on after the first.
    let at = |words: &str| {
        said.find(words)
            .unwrap_or_else(|| panic!("{words}: {said}"))
    };
    let moved = at("Script error in main.luau, line 8");
    let pushing = at("That happened while the check was pushing \"move\".");
    let fired = at("Script error in main.luau, line 4");
    let pressing = at("That happened while the check was pressing \"fire\".");
    assert!(
        moved < pushing && pushing < fired && fired < pressing,
        "{said}"
    );
    assert!(
        said.contains("Script error in main.luau, line 11"),
        "{said}"
    );
    assert_eq!(said.matches("That happened while").count(), 2, "{said}");
    assert!(said.contains("run `fossps check` again"), "{said}");
}

#[test]
fn check_presses_every_action_of_a_new_game_and_says_so() {
    let folder = scratch("check-press");
    assert!(fossps(&folder, &["new", "game"]).status.success());

    let output = fossps(&folder, &["check", "game"]);

    assert!(output.status.success(), "{}", stderr(&output));
    // Jump and fire are each bound to a key and to a gamepad button, and
    // the new game's script prints on both.
    for line in ["Jump number 2", "Fire! Moving 0, 0"] {
        assert!(stderr(&output).contains(line), "{}", stderr(&output));
    }
    assert!(!stderr(&output).contains("Jump number 3"));
    let said = stdout(&output);
    assert!(
        said.contains("Each of the 15 actions was also pressed, held and let go"),
        "{said}"
    );
    assert!(!said.contains("Nothing was pressed"), "{said}");
    assert!(!said.contains("Not pressed"), "{said}");
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
    // The level `fossps new` made was loaded too.
    assert!(
        stdout(&output).contains("level.glb is fine: it loaded with 22 shapes placed"),
        "{}",
        stdout(&output)
    );
}

/// A new game whose level was replaced by something that is not one.
fn game_with_a_broken_level(test: &str) -> PathBuf {
    let folder = scratch(test);
    assert!(fossps(&folder, &["new", "game"]).status.success());
    fs::write(folder.join("game/level.glb"), "not a level").unwrap();
    folder
}

#[test]
fn check_fails_on_a_level_that_does_not_load_and_says_how_to_export_one() {
    let folder = game_with_a_broken_level("check-level");

    let output = fossps(&folder, &["check", "game"]);

    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(said.contains("level.glb is not a glTF file"), "{said}");
    assert!(said.contains("glTF Binary (.glb)"), "{said}");
    assert!(said.contains("level.glb could not be loaded"), "{said}");
    assert!(said.contains("run `fossps check` again"), "{said}");
    // The script still had its turn.
    assert!(said.contains("Welcome to game!"), "{said}");
    assert!(!stdout(&output).contains("level.glb is fine"));
}

#[test]
fn check_passes_a_game_with_no_level_and_says_what_it_is_shown() {
    let folder = scratch("check-no-level");
    assert!(fossps(&folder, &["new", "game"]).status.success());
    fs::remove_file(folder.join("game/level.glb")).unwrap();

    let output = fossps(&folder, &["check", "game"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stderr(&output);
    assert!(said.contains("This game has no level.glb"), "{said}");
    assert!(
        said.contains("To give it a level, export one from Blender"),
        "{said}"
    );
    assert!(said.contains("and save it as level.glb"), "{said}");
    assert!(!stdout(&output).contains("level.glb is fine"));
}

#[test]
fn run_and_edit_open_nothing_when_the_level_does_not_load() {
    let folder = game_with_a_broken_level("run-level");

    for verb in ["run", "edit"] {
        let output = fossps(&folder, &[verb, "game"]);

        assert!(!output.status.success(), "{verb}");
        let said = stderr(&output);
        assert!(said.contains("level.glb is not a glTF file"), "{said}");
        assert!(said.contains("Nothing was opened"), "{said}");
        assert!(
            said.contains(&format!("run `fossps {verb}` again")),
            "{said}"
        );
    }
}

#[test]
fn check_reads_the_map_file_and_says_what_is_placed() {
    let folder = scratch("check-map");
    assert!(fossps(&folder, &["new", "game"]).status.success());
    // A new game has no map file, and nothing is said about one.
    let output = fossps(&folder, &["check", "game"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains("map.toml"));

    fs::write(
        folder.join("game/map.toml"),
        "version = 1\n\n[[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [4, 0, -12]\n\n\
         [[entity]]\nid = \"b\"\ntype = \"spawn\"\nposition = [-4, 0, 12]\n\n\
         [[entity]]\nid = \"c\"\ntype = \"zone\"\nposition = [0, 0, 0]\n",
    )
    .unwrap();
    let output = fossps(&folder, &["check", "game"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains(
            "map.toml is fine: 3 placed: 2 spawn, 1 zone. Players come into the world at \
             its spawn points."
        ),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_map_file_with_a_mistake_fails_check_and_package_and_is_not_opened_for_editing() {
    let folder = scratch("map-mistake");
    assert!(fossps(&folder, &["new", "game"]).status.success());
    let map = folder.join("game/map.toml");
    let written =
        "version = 1\n\n[[entity]]\nid = \"a\"\ntype = \"spawn\"\npositon = [4, 0, -12]\n";
    fs::write(&map, written).unwrap();

    let output = fossps(&folder, &["check", "game"]);
    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(
        said.contains("map.toml, line 6: \"positon\" is not something an entity has"),
        "{said}"
    );
    assert!(said.contains("map.toml has a mistake"), "{said}");
    assert!(said.contains("run `fossps check` again"), "{said}");
    assert!(!stdout(&output).contains("map.toml is fine"));

    let output = fossps(&folder, &["package", "game", "--target", "linux"]);
    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(said.contains("line 6: \"positon\""), "{said}");
    assert!(said.contains("the game was not packaged"), "{said}");
    assert!(!folder.join("game/dist").exists());

    // The editor saves what it holds, so it does not open a file it could
    // not read: the file is left exactly as it was.
    let output = fossps(&folder, &["edit", "game"]);
    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(said.contains("line 6: \"positon\""), "{said}");
    assert!(said.contains("Nothing was opened"), "{said}");
    assert!(
        said.contains("move map.toml out of the game's folder"),
        "{said}"
    );
    assert_eq!(fs::read_to_string(&map).unwrap(), written);
}

#[test]
fn package_refuses_a_game_whose_level_does_not_load() {
    let folder = game_with_a_broken_level("package-level");

    let output = fossps(&folder, &["package", "game", "--target", "linux"]);

    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(said.contains("level.glb is not a glTF file"), "{said}");
    assert!(said.contains("The game was not packaged"), "{said}");
    assert!(!folder.join("game/dist").exists());
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

#[test]
fn check_names_the_required_script_a_mistake_is_in() {
    let folder = scratch("check-required");
    assert!(fossps(&folder, &["new", "game"]).status.success());
    let game = folder.join("game");
    std::fs::write(
        game.join("main.luau"),
        "local weapons = require(\"./weapons\")\nprint(weapons.rifle)",
    )
    .unwrap();
    std::fs::write(game.join("weapons.luau"), "return { rifle = 30 }").unwrap();

    let output = fossps(&folder, &["check", "game"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("30"), "{}", stderr(&output));

    std::fs::write(
        game.join("weapons.luau"),
        "local weapons = {}\nweapons.rifle = nothing.here\nreturn weapons",
    )
    .unwrap();

    let output = fossps(&folder, &["check", "game"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(
        said.contains("Script error in weapons.luau, line 2:"),
        "{said}"
    );
    assert!(said.contains("weapons.luau has a mistake"), "{said}");
}

#[test]
fn a_packed_game_whose_level_does_not_load_says_so_and_opens_no_window() {
    let folder = scratch("pack-level");
    fs::write(folder.join("main.luau"), "").unwrap();
    fs::write(folder.join("level.glb"), "not a level").unwrap();
    let sources = ["main.luau", "level.glb"].map(|name| fossps_pack::Source {
        name: name.to_owned(),
        path: folder.join(name),
    });
    fossps_pack::write(&folder.join("Game.fpk"), "Game", &sources).unwrap();

    let output = fossps(&folder, &["run", "Game.fpk"]);

    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(
        said.contains("cannot start: its level could not be loaded"),
        "{said}"
    );
    assert!(
        said.contains("level.glb in Game.fpk is not a glTF file"),
        "{said}"
    );
    assert!(said.contains("tell the game's maker"), "{said}");
}

/// A 16-bit, one-channel WAV file: a tenth of a second of a steady level.
fn wav() -> Vec<u8> {
    let samples = 4800_u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&96_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(samples * 2).to_le_bytes());
    for _ in 0..samples {
        bytes.extend_from_slice(&8000_i16.to_le_bytes());
    }
    bytes
}

#[test]
fn check_loads_the_games_sounds_and_says_so() {
    let folder = scratch("check-sounds");
    fs::create_dir_all(folder.join("sounds/ui")).unwrap();
    fs::write(folder.join("sounds/ui/click.wav"), wav()).unwrap();
    fs::write(folder.join("sounds/gunshot.wav"), wav()).unwrap();
    fs::write(folder.join("sounds/LICENCE.txt"), "CC0").unwrap();
    fs::write(
        folder.join("main.luau"),
        "game.onTick(function()\n    if game.input.justPressed(\"fire\") then\n        \
         game.audio.play(\"ui/click\", { volume = 0.5 })\n        \
         game.audio.playAt(\"gunshot\", vector.create(0, 1, -5), { range = 80 })\n    \
         end\nend)\n",
    )
    .unwrap();

    let output = fossps(&folder, &["check"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("The 2 sound files in the sounds folder are fine"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn check_fails_on_a_sound_that_is_not_in_the_game_and_names_the_nearest() {
    let folder = scratch("check-sound-name");
    fs::create_dir_all(folder.join("sounds")).unwrap();
    fs::write(folder.join("sounds/door.wav"), wav()).unwrap();
    fs::write(
        folder.join("main.luau"),
        "game.onTick(function()\n    if game.input.justPressed(\"interact\") then\n        \
         game.audio.play(\"dor\")\n    end\nend)\n",
    )
    .unwrap();

    let output = fossps(&folder, &["check"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("Script error in main.luau, line 3"), "{said}");
    assert!(
        said.contains("There is no sound called \"dor\". Did you mean \"door\"?"),
        "{said}"
    );
    assert!(said.contains("pressing \"interact\""), "{said}");
}

#[test]
fn check_fails_on_a_sound_file_that_cannot_be_played_and_says_what_to_do() {
    let folder = scratch("check-sound-file");
    fs::create_dir_all(folder.join("sounds")).unwrap();
    fs::write(folder.join("sounds/door.wav"), "not a sound").unwrap();
    fs::write(folder.join("sounds/music.mp3"), "ID3").unwrap();
    fs::write(folder.join("main.luau"), "game.onStart(function() end)\n").unwrap();

    let output = fossps(&folder, &["check"]);

    assert_eq!(output.status.code(), Some(1));
    let said = stderr(&output);
    assert!(said.contains("sounds/door.wav was not loaded."), "{said}");
    assert!(
        said.contains("sounds/music.mp3 was not loaded: only WAV files can be played"),
        "{said}"
    );
    assert!(
        said.contains("2 of the files in the game's sounds folder cannot be played"),
        "{said}"
    );
    assert!(said.contains("run `fossps check` again"), "{said}");
}
