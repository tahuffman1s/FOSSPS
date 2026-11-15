//! Runs real scripts from real files through the host.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use fossps_input::{ActionMap, GamepadButton, Input, InputFrame, Key, Rumble};
use fossps_script::{ENTRY_SCRIPT, OpenError, ScriptHost};

thread_local! {
    /// What scripts on this test's thread printed.
    static PRINTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The errors the engine logged on this test's thread.
    static ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Collects `print` output per thread, so tests can run side by side.
struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        if record.target() == "script" {
            PRINTED.with_borrow_mut(|lines| lines.push(record.args().to_string()));
        } else if record.level() == log::Level::Error {
            ERRORS.with_borrow_mut(|lines| lines.push(record.args().to_string()));
        }
    }

    fn flush(&self) {}
}

fn printed() -> Vec<String> {
    PRINTED.with_borrow_mut(std::mem::take)
}

fn errors() -> Vec<String> {
    ERRORS.with_borrow_mut(std::mem::take)
}

/// A game folder that is deleted when the test ends.
struct GameFolder(PathBuf);

impl GameFolder {
    fn new(script: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Info);
        let folder = std::env::temp_dir().join(format!(
            "fossps-script-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let folder = Self(folder);
        folder.write(script);
        folder
    }

    fn write(&self, script: &str) {
        std::fs::write(self.0.join(ENTRY_SCRIPT), script).unwrap();
    }

    /// Writes another script into the game, making its folder if needed.
    fn write_file(&self, name: &str, script: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, script).unwrap();
    }

    fn open(&self) -> ScriptHost {
        ScriptHost::open(&self.0, ActionMap::default_fps()).unwrap()
    }
}

impl Drop for GameFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const STEP: f32 = 1.0 / 60.0;

fn idle() -> InputFrame {
    InputFrame::default()
}

#[test]
fn start_runs_once_before_the_first_tick() {
    let game = GameFolder::new(
        r#"
        print("loaded")
        game.onStart(function() print("start") end)
        game.onTick(function(step) print("tick", step) end)
        "#,
    );
    let mut host = game.open();
    assert_eq!(printed(), ["loaded"]);
    host.tick(&idle(), 0.5);
    host.tick(&idle(), 0.5);
    assert_eq!(printed(), ["start", "tick 0.5", "tick 0.5"]);
    assert!(host.last_error().is_none());
}

#[test]
fn several_handlers_run_in_the_order_they_were_added() {
    let game = GameFolder::new(
        r#"
        game.onTick(function() print("first") end)
        game.onTick(function() print("second") end)
        "#,
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["first", "second"]);
}

#[test]
fn scripts_read_keyboard_and_gamepad_through_the_same_action() {
    let game = GameFolder::new(
        r#"
        game.onTick(function()
            if game.input.justPressed("jump") then print("jump") end
            if game.input.pressed("jump") then print("held") end
            if game.input.justReleased("jump") then print("released") end
            local x, y = game.input.axis("move")
            if y ~= 0 then print("move", x, y) end
        end)
        "#,
    );
    let mut host = game.open();
    let mut input = Input::new(ActionMap::default_fps());

    input.press(Key::Space);
    host.tick(input.tick(), STEP);
    assert_eq!(printed(), ["jump", "held"]);
    input.release(Key::Space);
    host.tick(input.tick(), STEP);
    assert_eq!(printed(), ["released"]);

    input.press(GamepadButton::GamepadSouth);
    host.tick(input.tick(), STEP);
    assert_eq!(printed(), ["jump", "held"]);
    input.release(GamepadButton::GamepadSouth);
    host.tick(input.tick(), STEP);
    printed();

    input.press(Key::W);
    host.tick(input.tick(), STEP);
    assert_eq!(printed(), ["move 0 1"]);
}

#[test]
fn a_script_rumbles_the_gamepad_and_the_latest_call_wins() {
    let game = GameFolder::new(
        r#"
        game.onTick(function()
            if game.input.justPressed("fire") then
                game.input.rumble(0.9, 0.1, 2)
                game.input.rumble(0.25, 0.5, 0.125)
            end
            if game.input.justPressed("jump") then
                game.input.rumble(50, -1, 600)
            end
        end)
        "#,
    );
    let mut host = game.open();
    let mut input = Input::new(ActionMap::default_fps());
    host.tick(input.tick(), STEP);
    assert_eq!(host.take_rumble(), None);

    input.press(GamepadButton::RightTrigger);
    host.tick(input.tick(), STEP);
    assert_eq!(host.take_rumble(), Some(Rumble::new(0.25, 0.5, 0.125)));
    // Taken once, not played again every frame.
    host.tick(input.tick(), STEP);
    assert_eq!(host.take_rumble(), None);

    // Numbers out of range are brought into it instead of failing mid-game.
    input.press(Key::Space);
    host.tick(input.tick(), STEP);
    let rumble = host.take_rumble().unwrap();
    assert_eq!(
        (rumble.strong(), rumble.weak(), rumble.seconds()),
        (1.0, 0.0, Rumble::MAX_SECONDS)
    );
    assert!(host.last_error().is_none());
}

#[test]
fn a_rumble_missing_a_number_says_which_and_how_to_write_it() {
    let game = GameFolder::new("game.input.rumble(0.5)");
    let host = game.open();
    let error = host.last_error().unwrap();
    assert_eq!(error.line, Some(1));
    assert!(
        error
            .message
            .starts_with("game.input.rumble needs a number for `weak`, but it was given nothing.")
            && error
                .message
                .ends_with("Write it like this: game.input.rumble(0.2, 0.6, 0.1)"),
        "{error}"
    );

    let game = GameFolder::new(r#"game.input.rumble(0.5, 0.5, "long")"#);
    let host = game.open();
    let error = host.last_error().unwrap();
    assert!(
        error
            .message
            .contains("a number for `seconds`, but it was given text"),
        "{error}"
    );
}

#[test]
fn a_rumble_with_no_time_given_is_a_short_one() {
    let game = GameFolder::new("game.onStart(function() game.input.rumble(1, 0.5) end)");
    let mut host = game.open();
    host.tick(&idle(), STEP);
    assert_eq!(
        host.take_rumble(),
        Some(Rumble::new(1.0, 0.5, Rumble::DEFAULT_SECONDS))
    );
}

#[test]
fn a_missing_script_says_where_to_put_one() {
    let game = GameFolder::new("");
    std::fs::remove_file(game.0.join(ENTRY_SCRIPT)).unwrap();
    let error = ScriptHost::open(&game.0, ActionMap::default_fps()).unwrap_err();
    assert!(matches!(error, OpenError::Missing { .. }));
    let text = error.to_string();
    assert!(text.contains("main.luau"), "{text}");
    assert!(text.contains(&*game.0.to_string_lossy()), "{text}");
}

#[test]
fn a_syntax_error_names_the_line_and_waits_for_a_fix() {
    let game = GameFolder::new("game.onTick(function()\n    print(\"hi\")\n");
    let mut host = game.open();
    let error = host.last_error().expect("the script is broken").clone();
    assert_eq!(error.file, "main.luau");
    assert!(error.line.is_some(), "{error}");
    assert!(
        error.hint.as_deref().unwrap_or_default().contains("`end`"),
        "{error}"
    );
    assert!(error.to_string().contains("save the file"), "{error}");

    // Nothing runs, and nothing crashes, until the file is fixed.
    host.tick(&idle(), STEP);
    game.write("game.onStart(function() print(\"fixed\") end)");
    host.reload().unwrap();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["fixed"]);
    assert!(host.last_error().is_none());
}

#[test]
fn a_runtime_error_shows_the_line_and_is_reported_once() {
    let game = GameFolder::new(
        "local player = nil\n\
         game.onTick(function()\n\
         \x20   player.addScore(1)\n\
         end)\n\
         game.onTick(function() print(\"still running\") end)\n",
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    host.tick(&idle(), STEP);

    let error = host.last_error().expect("the tick failed");
    assert_eq!(error.line, Some(3));
    assert_eq!(error.source_line.as_deref(), Some("player.addScore(1)"));
    assert!(error.message.contains("attempt to index nil"), "{error}");
    assert!(error.hint.is_some(), "{error}");
    // The other handler is not taken down with it.
    assert_eq!(printed(), ["still running", "still running"]);
}

#[test]
fn a_misspelled_api_name_suggests_the_right_one() {
    let game = GameFolder::new("\ngame.onTik(function() end)\n");
    let host = game.open();
    let error = host.last_error().expect("onTik does not exist");
    assert_eq!(error.line, Some(2));
    assert_eq!(
        error.message,
        "game.onTik does not exist. Did you mean game.onTick?"
    );
}

#[test]
fn a_misspelled_action_suggests_the_right_one() {
    let game = GameFolder::new(
        "game.onTick(function()\n    game.input.pressed(\"jmp\")\nend)\n\
         game.onTick(function()\n    game.input.pressed(\"teleport\")\nend)\n",
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    let error = host.last_error().unwrap();
    assert_eq!(error.line, Some(5));
    assert!(
        error.message.contains("\"teleport\" is not a known action"),
        "{error}"
    );
    assert!(error.message.contains("jump"), "{error}");

    game.write("game.onTick(function()\n    game.input.pressed(\"jmp\")\nend)\n");
    host.reload().unwrap();
    host.tick(&idle(), STEP);
    let error = host.last_error().unwrap();
    assert_eq!(error.line, Some(2));
    assert!(error.message.contains("Did you mean \"jump\"?"), "{error}");
}

#[test]
fn a_wrong_argument_explains_what_was_expected() {
    let game = GameFolder::new("game.onTick(5)");
    let host = game.open();
    let error = host.last_error().unwrap();
    assert_eq!(error.line, Some(1));
    assert!(
        error
            .message
            .starts_with("game.onTick needs a function, but it was given a number."),
        "{error}"
    );
}

#[test]
fn reload_replaces_handlers_and_resets_variables() {
    let game = GameFolder::new(
        r#"
        local count = 0
        game.onStart(function() print("start one") end)
        game.onReload(function() print("reload one") end)
        game.onTick(function() count += 1; print("one", count) end)
        "#,
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["start one", "one 1", "one 2"]);

    game.write(
        r#"
        local count = 0
        game.onStart(function() print("start two") end)
        game.onReload(function() print("reload two") end)
        game.onTick(function() count += 1; print("two", count) end)
        "#,
    );
    host.reload().unwrap();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["reload two", "two 1"]);
}

#[test]
fn a_broken_reload_keeps_the_previous_version_running() {
    let game = GameFolder::new("game.onTick(function() print(\"good\") end)");
    let mut host = game.open();
    host.tick(&idle(), STEP);

    game.write("game.onTick(function() print(\"bad\") end)\nthis is not luau\n");
    let error = host.reload().unwrap_err();
    assert_eq!(error.line, Some(2));
    assert!(
        error.to_string().contains("last version that worked"),
        "{error}"
    );
    assert_eq!(host.last_error(), Some(&error));

    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["good", "good"]);
}

#[test]
fn saving_the_file_reloads_it_without_being_asked() {
    let game = GameFolder::new("game.onTick(function() print(\"before\") end)");
    let mut host = game.open();
    game.write("game.onTick(function() print(\"after the save\") end)");

    let started = Instant::now();
    loop {
        host.poll_reload();
        host.tick(&idle(), STEP);
        if printed().iter().any(|line| line == "after the save") {
            break;
        }
        assert!(
            started.elapsed().as_secs() < 10,
            "the save was never picked up"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn an_endless_loop_is_stopped_and_the_script_paused() {
    let game = GameFolder::new(
        "game.onTick(function()\n    print(\"tick\")\n    while true do end\nend)\n",
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    let error = host.last_error().expect("the loop was stopped").clone();
    assert_eq!(error.line, Some(3));
    assert!(error.message.contains("too long"), "{error}");
    assert!(error.to_string().contains("paused"), "{error}");

    // Paused: the loop does not freeze the game again on the next tick.
    let before = Instant::now();
    host.tick(&idle(), STEP);
    assert!(before.elapsed().as_millis() < 500);
    assert_eq!(printed(), ["tick"]);

    game.write("game.onTick(function() print(\"fixed\") end)");
    host.reload().unwrap();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["fixed"]);
}

#[test]
fn scripts_cannot_reach_the_computer_or_change_the_api() {
    for (script, expected) in [
        ("local f = io.open(\"/etc/passwd\")", "attempt to index nil"),
        ("os.execute(\"true\")", "attempt to call a nil value"),
        ("game.onTick = nil", "readonly"),
        ("math.pi = 3", "readonly"),
        ("require(\"../other\")", "outside the game's folder"),
    ] {
        let game = GameFolder::new(script);
        let host = game.open();
        let error = host
            .last_error()
            .unwrap_or_else(|| panic!("`{script}` should fail"));
        assert!(error.message.contains(expected), "`{script}`: {error}");
    }
}

#[test]
fn the_example_game_runs_cleanly() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello");
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Info);
    let mut host = ScriptHost::open(example, ActionMap::default_fps()).unwrap();
    let mut input = Input::new(ActionMap::default_fps());
    input.press(Key::Space);
    host.tick(input.tick(), STEP);
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    assert!(
        printed().len() >= 2,
        "the example greets and reacts to jump"
    );
}

#[test]
fn reading_an_action_the_wrong_way_says_which_function_to_use() {
    for (call, expected) in [
        (
            "game.input.value(\"move\")",
            "Write: local x, y = game.input.axis(\"move\")",
        ),
        ("game.input.axis(\"jump\")", "game.input.pressed(\"jump\")"),
        ("game.input.mouse(\"fire\")", "has no x and y"),
    ] {
        let game = GameFolder::new(call);
        let host = game.open();
        let error = host
            .last_error()
            .unwrap_or_else(|| panic!("`{call}` should fail"));
        assert!(error.message.contains(expected), "`{call}`: {error}");
    }
    let game = GameFolder::new(
        "local x, y = game.input.mouse(\"look\")\nprint(x, y, game.input.value(\"fire\"))",
    );
    let host = game.open();
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    assert_eq!(printed(), ["0 0 0"]);
}

#[test]
fn catching_the_time_limit_with_pcall_does_not_hide_it() {
    let game = GameFolder::new(
        "game.onTick(function()\n    pcall(function()\n        while true do end\n    end)\n    print(\"got away\")\nend)\n",
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    let error = host.last_error().expect("the loop was stopped").clone();
    assert_eq!(error.line, Some(3));
    assert!(error.to_string().contains("paused"), "{error}");
    assert!(printed().is_empty(), "nothing runs after time is up");
}

#[test]
fn a_handler_that_keeps_adding_handlers_cannot_hold_up_a_tick() {
    let game = GameFolder::new(
        "local function again()\n    game.onTick(again)\nend\ngame.onTick(again)\n",
    );
    let mut host = game.open();
    let before = Instant::now();
    for _ in 0..12 {
        host.tick(&idle(), STEP);
    }
    assert!(before.elapsed().as_millis() < 500);
    let error = host.last_error().expect("the pile-up is reported");
    assert_eq!(error.line, Some(2));
    assert!(error.message.contains("1024 functions already"), "{error}");
}

#[test]
fn a_script_given_as_text_runs_and_is_never_loaded_again() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Info);
    let _ = printed();
    let mut host = ScriptHost::from_source(
        "game.onStart(function() print('packed') end)",
        ActionMap::default_fps(),
    );

    host.tick(&InputFrame::default(), 1.0 / 60.0);
    host.poll_reload();

    assert!(host.last_error().is_none());
    assert_eq!(printed(), ["packed"]);
    // There is no file to read, and that is not a mistake in the script.
    assert!(host.reload().is_ok());
    assert!(host.last_error().is_none());

    let broken = ScriptHost::from_source("game.onTik()", ActionMap::default_fps());
    assert!(broken.last_error().is_some());
}

#[test]
fn a_game_is_split_over_files_with_require() {
    let game = GameFolder::new(
        r#"local weapons = require("./weapons")
        local again = require("./weapons.luau")
        require("./ui/hud")
        print(weapons.rifle.damage, weapons == again)"#,
    );
    game.write_file(
        "weapons.luau",
        r#"print("weapons loaded")
        return { rifle = require("./weapons/rifle") }"#,
    );
    game.write_file(
        "weapons/rifle.luau",
        r#"local names = require("../names")
        return { damage = 30, name = names.rifle }"#,
    );
    game.write_file("names.luau", "return { rifle = \"Rifle\" }");
    game.write_file(
        "ui/hud.luau",
        r#"local names = require("../names")
        game.onStart(function() print("hud for " .. names.rifle) end)"#,
    );
    let mut host = game.open();
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    // Each file runs once, however often it is required.
    assert_eq!(printed(), ["weapons loaded", "30 true"]);
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["hud for Rifle"]);
}

#[test]
fn an_error_in_a_required_script_names_that_file_and_line() {
    let game = GameFolder::new(
        r#"local weapons = require("./weapons")
        game.onTick(function() weapons.fire() end)"#,
    );
    game.write_file(
        "weapons.luau",
        "local weapons = {}\nfunction weapons.fire()\n    local ammo = nil\n    \
         return ammo.count\nend\nreturn weapons",
    );
    let mut host = game.open();
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    host.tick(&idle(), STEP);
    let error = host.last_error().expect("the tick failed").clone();
    assert_eq!(error.file, "weapons.luau");
    assert_eq!(error.line, Some(4));
    assert_eq!(error.source_line.as_deref(), Some("return ammo.count"));
    assert!(
        error
            .to_string()
            .starts_with("Script error in weapons.luau, line 4:"),
        "{error}"
    );

    // A mistake that stops the file from being read at all.
    game.write_file("weapons.luau", "local weapons = {}\nreturn weapons +");
    let error = host.reload().unwrap_err();
    assert_eq!(error.file, "weapons.luau");
    assert_eq!(error.line, Some(2), "{error}");

    // The engine's own complaints point at the file that made the call.
    game.write_file("weapons.luau", "\ngame.onTik(function() end)");
    let error = host.reload().unwrap_err();
    assert_eq!((error.file.as_str(), error.line), ("weapons.luau", Some(2)));
    assert!(
        error.message.contains("Did you mean game.onTick?"),
        "{error}"
    );

    // A loop that never ends is found in the file it is in.
    game.write_file(
        "weapons.luau",
        "local n = 0\nwhile true do\n    n += 1\nend",
    );
    let error = host.reload().unwrap_err();
    assert_eq!(error.file, "weapons.luau");
    assert!(error.message.contains("too long"), "{error}");
    assert!(matches!(error.line, Some(2..=4)), "{error}");
}

#[test]
fn a_require_that_cannot_work_says_what_to_write() {
    /// A `main.luau`, the other files as name and text, and what the error says.
    type Case = (
        &'static str,
        &'static [(&'static str, &'static str)],
        &'static str,
    );
    let cases: &[Case] = &[
        (
            r#"require("weapons")"#,
            &[("weapons.luau", "")],
            r#"Write require("./weapons")"#,
        ),
        (
            r#"require("./wepons")"#,
            &[("weapons.luau", "")],
            r#"There is no script called wepons.luau in the game's folder. Did you mean require("./weapons")?"#,
        ),
        (
            r#"require("./guns/rifel")"#,
            &[("guns/rifle.luau", "")],
            r#"Did you mean require("./guns/rifle")?"#,
        ),
        (
            r#"require("./Weapons")"#,
            &[("weapons.luau", "")],
            // Found under its real name on Windows and macOS, missing on Linux.
            "weapons",
        ),
        (
            r#"require("./guns")"#,
            &[("guns/rifle.luau", "")],
            "guns is a folder",
        ),
        (r#"require("./nothing")"#, &[], "or create the file"),
        (
            r#"require("../../etc/passwd")"#,
            &[],
            "outside the game's folder",
        ),
        (r#"require("/etc/passwd")"#, &[], "is a full path"),
        (
            r#"require("./guns.luau")"#,
            &[("guns/rifle.luau", ""), ("guns/pistol.luau", "")],
            r#"like require("./guns/pistol")"#,
        ),
        (r#"require(42)"#, &[], "it was given a number"),
        (r#"require()"#, &[], "it was given nothing"),
        (
            r#"require("./a")"#,
            &[
                ("a.luau", r#"require("./b")"#),
                ("b.luau", r#"require("./a")"#),
            ],
            "in a circle: a.luau, which requires b.luau, which requires a.luau.",
        ),
        (
            r#"require("./a")"#,
            &[("a.luau", r#"require("./a")"#)],
            "a.luau requires itself.",
        ),
        (r#"require("./main")"#, &[], "where the game starts"),
    ];
    for (script, files, expected) in cases {
        let game = GameFolder::new(script);
        for (name, text) in *files {
            game.write_file(name, text);
        }
        let host = game.open();
        let error = host
            .last_error()
            .unwrap_or_else(|| panic!("`{script}` should fail"));
        assert!(error.message.contains(expected), "`{script}`: {error}");
        assert!(error.line.is_some(), "`{script}`: {error}");
    }
}

#[test]
fn a_required_script_cannot_be_reached_through_a_link() {
    #[cfg(unix)]
    {
        let outside = GameFolder::new("");
        outside.write_file("secret.luau", "return \"secret\"");
        let game = GameFolder::new("print(require(\"./secret\"))");
        std::os::unix::fs::symlink(outside.0.join("secret.luau"), game.0.join("secret.luau"))
            .unwrap();
        let host = game.open();
        let error = host.last_error().expect("a link is refused");
        assert!(error.message.contains("is a link"), "{error}");
        assert_eq!(printed(), [] as [&str; 0]);

        let game = GameFolder::new("print(require(\"./lib/secret\"))");
        std::os::unix::fs::symlink(&outside.0, game.0.join("lib")).unwrap();
        let host = game.open();
        let error = host.last_error().expect("a linked folder is refused");
        assert!(error.message.contains("is a link"), "{error}");
    }
}

#[test]
fn a_script_cannot_pretend_to_be_a_file_somewhere_else() {
    // `loadstring` lets a script name its own chunk. A name that is not one
    // of the game's files must not move where `./` starts from.
    let outside = GameFolder::new("");
    outside.write_file("secret.luau", "return \"secret\"");
    let above = outside.0.file_name().unwrap().to_str().unwrap().to_owned();
    for chunk in [
        format!("@../{above}/x.luau"),
        format!("@{}/x.luau", outside.0.display()),
        "@../x.luau".to_owned(),
        "@a/../../x.luau".to_owned(),
    ] {
        let game = GameFolder::new(&format!(
            r#"local load = loadstring or function() return function() return "no loadstring" end end
            local run = load("return require('./secret')", "{}")
            print(select(2, pcall(run)))"#,
            chunk.replace('\\', "/")
        ));
        game.write_file("inside.luau", "return 1");
        let _host = game.open();
        let said = printed().join("\n");
        assert!(
            !said.contains("secret\n") && said != "secret",
            "{chunk}: {said}"
        );
        assert!(
            said.contains("There is no script called secret.luau") || said == "no loadstring",
            "{chunk}: {said}"
        );
    }

    // From a chunk with a made-up name, `./` is the game's own folder.
    let game = GameFolder::new(
        r#"local load = loadstring or function() return function() return 1 end end
        print(load("return require('./inside')", "@deep/down/x.luau")())"#,
    );
    game.write_file("inside.luau", "return 1");
    let host = game.open();
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    assert_eq!(printed(), ["1"]);
}

#[test]
fn a_require_that_fails_is_not_tried_again_every_tick() {
    let game = GameFolder::new(
        r#"local n = 0
        game.onTick(function()
            n += 1
            local found = pcall(require, "./late")
            local _, problem = pcall(require, "./made-up-" .. n)
            local full = string.find(tostring(problem), "the most it can", 1, true) ~= nil
            print(n, found, full)
        end)"#,
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["1 false false"]);
    // The file appears, but this version already knows the answer: only a
    // reload, which the new file sets off, asks again.
    game.write_file("late.luau", "return 1");
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["2 false false"]);
    host.reload().unwrap();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["1 true false"]);

    // Names made up as the game runs come to an end.
    for _ in 0..1100 {
        host.tick(&idle(), STEP);
    }
    let said = printed();
    assert_eq!(said[1000], "1002 true false");
    assert_eq!(said.last().unwrap(), "1101 true true");
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
}

/// Polls until the host has loaded a save, or gives up.
fn wait_for_reload(host: &mut ScriptHost, done: impl Fn(&ScriptHost) -> bool) {
    let start = Instant::now();
    while start.elapsed().as_secs() < 10 {
        host.poll_reload();
        if done(host) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the save was not picked up");
}

#[test]
fn saving_a_required_script_reloads_the_game() {
    let game = GameFolder::new(
        r#"local weapons = require("./weapons")
        game.onTick(function() print(weapons.damage) end)"#,
    );
    game.write_file("weapons.luau", "return { damage = 30 }");
    let mut host = game.open();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["30"]);

    // A longer file, so the save shows even if the clock has not moved.
    game.write_file("weapons.luau", "return { damage = 45 } -- more");
    let start = Instant::now();
    loop {
        host.poll_reload();
        host.tick(&idle(), STEP);
        if printed().last().is_some_and(|line| line == "45") {
            break;
        }
        assert!(start.elapsed().as_secs() < 10, "the save was not picked up");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // A mistake in it keeps the last version, and fixing it is picked up.
    game.write_file("weapons.luau", "return {");
    wait_for_reload(&mut host, |host| host.last_error().is_some());
    assert_eq!(host.last_error().unwrap().file, "weapons.luau");
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["45"]);
    game.write_file("weapons.luau", "return { damage = 60 }");
    wait_for_reload(&mut host, |host| host.last_error().is_none());
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["60"]);
}

#[test]
fn a_script_that_was_missing_is_picked_up_once_it_is_made() {
    let game = GameFolder::new(
        r#"local weapons = require("./weapons")
        print(weapons.damage)"#,
    );
    let mut host = game.open();
    assert!(host.last_error().is_some());
    game.write_file("weapons.luau", "return { damage = 30 }");
    wait_for_reload(&mut host, |host| host.last_error().is_none());
    assert_eq!(printed(), ["30"]);
}

#[test]
fn a_script_required_while_the_game_runs_is_watched_too() {
    let game = GameFolder::new(r#"game.onTick(function() print(require("./late").text) end)"#);
    game.write_file("late.luau", "return { text = \"first\" }");
    let mut host = game.open();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["first"]);
    game.write_file("late.luau", "return { text = \"second one\" }");
    let start = Instant::now();
    loop {
        host.poll_reload();
        host.tick(&idle(), STEP);
        if printed().last().is_some_and(|line| line == "second one") {
            break;
        }
        assert!(start.elapsed().as_secs() < 10, "the save was not picked up");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn scripts_given_as_text_require_each_other() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Info);
    let scripts = |main: &str| {
        [
            ("main.luau", main),
            (
                "weapons/rifle.luau",
                "return { damage = require(\"../balance\").rifle }",
            ),
            ("balance.luau", "return { rifle = 30 }"),
        ]
        .map(|(name, text)| (name.to_owned(), text.to_owned()))
    };
    let host = ScriptHost::from_sources(
        scripts("print(require(\"./weapons/rifle\").damage)"),
        ActionMap::default_fps(),
    );
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    assert_eq!(printed(), ["30"]);

    let host = ScriptHost::from_sources(
        scripts("require(\"./weapons/Rifle\")"),
        ActionMap::default_fps(),
    );
    let error = host.last_error().expect("the name's capitals differ");
    assert!(
        error.message.contains("saved as weapons/rifle.luau"),
        "{error}"
    );

    let host = ScriptHost::from_sources(
        [("weapons.luau".to_owned(), String::new())],
        ActionMap::default_fps(),
    );
    let error = host.last_error().expect("there is no main.luau");
    assert!(
        error.message.contains("no script called main.luau"),
        "{error}"
    );
}

#[test]
fn a_kept_table_is_still_there_after_a_reload() {
    let script = r#"
        local saved = game.keep("saved", { jumps = 0, name = "none", list = {}, on = false })
        local plain = 0
        game.onTick(function()
            saved.jumps += 1
            saved.name = "Ann"
            saved.on = true
            saved.half = 0.5
            table.insert(saved.list, { at = saved.jumps })
            plain += 1
            print(saved.jumps, plain, saved.name, saved.on, saved.half, #saved.list,
                saved.list[#saved.list].at)
        end)
        "#;
    let game = GameFolder::new(script);
    let mut host = game.open();
    host.tick(&idle(), STEP);
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["1 1 Ann true 0.5 1 1", "2 2 Ann true 0.5 2 2"]);

    host.reload().unwrap();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["3 1 Ann true 0.5 3 3"]);
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    assert_eq!(errors(), [] as [&str; 0]);
}

#[test]
fn a_kept_table_gains_what_the_script_now_starts_with() {
    let game = GameFolder::new(
        r#"
        local saved = game.keep("saved", { best = 1, gone = 5 })
        saved.best = 7
        saved.gone = nil
        "#,
    );
    let mut host = game.open();
    game.write(
        r#"
        local saved = game.keep("saved", { best = 0, lives = 3, gone = 5 })
        print(saved.best, saved.lives, saved.gone)
        "#,
    );
    host.reload().unwrap();
    assert_eq!(printed(), ["7 3 5"]);
}

#[test]
fn a_kept_table_can_be_asked_for_with_no_start_and_from_two_files() {
    let game = GameFolder::new(
        r#"
        local mine = game.keep("counts")
        local theirs = require("./other")
        mine.total = (mine.total or 0) + 1
        print(mine == theirs, theirs.total)
        "#,
    );
    game.write_file(
        "other.luau",
        "return game.keep(\"counts\", { extra = true })",
    );
    let mut host = game.open();
    host.reload().unwrap();
    assert_eq!(printed(), ["true 1", "true 2"]);
}

#[test]
fn players_are_kept_as_keys_and_values_while_they_are_in_the_match() {
    let game = GameFolder::new(
        r#"
        local streaks = game.keep("streaks")
        local saved = game.keep("saved", { last = false })
        game.onPlayerJoin(function(player)
            streaks[player] = #player.name
            saved.last = player
        end)
        game.onReload(function()
            local names = {}
            for player, streak in streaks do
                table.insert(names, player.name .. "=" .. streak)
            end
            table.sort(names)
            print(table.concat(names, " "), saved.last and saved.last.name)
            for _, player in game.players() do
                print(player.name, streaks[player])
            end
        end)
        "#,
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    let ann = host.player_joined("Ann");
    host.player_joined("Bob");
    host.reload().unwrap();
    assert_eq!(printed(), ["Ann=3 Bob=3 Bob", "Ann 3", "Bob 3"]);

    // A player who left is not handed back as a key that cannot be read.
    let cy = host.player_joined("Cyril");
    host.player_left(ann);
    host.reload().unwrap();
    assert_eq!(printed(), ["Bob=3 Cyril=5 Cyril", "Bob 3", "Cyril 5"]);
    host.player_left(cy);
    host.reload().unwrap();
    assert_eq!(printed(), ["Bob=3 false", "Bob 3"]);
    assert_eq!(errors(), [] as [&str; 0]);
}

#[test]
fn what_cannot_be_kept_is_left_out_and_named() {
    let game = GameFolder::new(
        r#"
        game.mode { teams = { "Red", "Blue" } }
        local saved = game.keep("saved", { best = 4 })
        game.onStart(function()
            saved.inner = { call = function() end, fine = 1 }
            saved.team = game.team("Red")
            saved["odd key"] = print
            saved[{}] = 2
            saved.loop = {}
            saved.loop.again = saved.loop
        end)
        "#,
    );
    let mut host = game.open();
    host.tick(&idle(), STEP);
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    host.reload().unwrap();
    let mut problems = errors();
    problems.sort();
    let all = problems.join("\n");
    assert_eq!(problems.len(), 5, "{all}");
    assert!(
        all.contains(
            "`inner.call` in game.keep(\"saved\") was not kept across the reload: it is a \
             function. A kept table can hold numbers, text, true and false, players, and \
             tables of those. Take this out of the table, or keep a name or a number in \
             its place."
        ),
        "{all}"
    );
    assert!(
        all.contains("`team` in game.keep(\"saved\") was not kept across the reload: it is a team. Keep the team's name instead: team.name."),
        "{all}"
    );
    assert!(all.contains("`[\"odd key\"]` in game.keep"), "{all}");
    assert!(all.contains("its key is a table"), "{all}");
    assert!(
        all.contains("A table that holds itself goes on for ever"),
        "{all}"
    );
    assert!(host.last_error().is_none(), "{:?}", host.last_error());

    game.write(
        r#"
        local saved = game.keep("saved")
        local keys = {}
        for key in saved do table.insert(keys, key) end
        table.sort(keys)
        print(table.concat(keys, " "), saved.inner.fine, next(saved.inner, "fine"))
        "#,
    );
    host.reload().unwrap();
    assert_eq!(printed(), ["best inner loop 1 nil"]);
    assert_eq!(errors(), [] as [&str; 0]);
}

#[test]
fn a_wrong_keep_says_how_to_write_it() {
    for (script, expected) in [
        (
            "local jumps = game.keep(\"jumps\", 0)",
            "game.keep keeps a table, but it was given a number. A number or text on its \
             own is copied each time it is used, so a kept one could never change. Put it in \
             a table and keep the table: local saved = game.keep(\"saved\", { count = 0 }), \
             then saved.count += 1",
        ),
        (
            "local saved = game.keep({ best = 0 })",
            "game.keep needs a name in quotes first, but it was given a table.",
        ),
        (
            "for i = 1, 1000 do game.keep(\"table\" .. i) end",
            "game.keep has been given 256 different names already.",
        ),
    ] {
        let game = GameFolder::new(script);
        let host = game.open();
        let error = host
            .last_error()
            .unwrap_or_else(|| panic!("`{script}` should fail"));
        assert_eq!(error.line, Some(1));
        assert!(error.message.contains(expected), "`{script}`: {error}");
    }
}

#[test]
fn a_save_with_a_mistake_does_not_lose_what_was_kept() {
    let script = r#"
        local saved = game.keep("saved", { ticks = 0 })
        game.onTick(function() saved.ticks += 1; print(saved.ticks) end)
        "#;
    let game = GameFolder::new(script);
    let mut host = game.open();
    host.tick(&idle(), STEP);

    game.write("local saved = game.keep(\"saved\", { ticks = 0 })\nsaved.ticks = 100\noops()");
    assert!(host.reload().is_err());
    host.tick(&idle(), STEP);

    game.write(script);
    host.reload().unwrap();
    host.tick(&idle(), STEP);
    assert_eq!(printed(), ["1", "2", "3"]);
}

#[test]
fn a_table_the_script_stops_asking_for_starts_over() {
    let script = "local saved = game.keep(\"saved\", { n = 0 })\nsaved.n += 1\nprint(saved.n)";
    let game = GameFolder::new(script);
    let mut host = game.open();
    host.reload().unwrap();
    game.write("print(\"not kept\")");
    host.reload().unwrap();
    game.write(script);
    host.reload().unwrap();
    assert_eq!(printed(), ["1", "2", "not kept", "1"]);
}

#[test]
fn keeping_too_much_is_cut_short_and_said_once() {
    let game = GameFolder::new(
        r#"
        local big = game.keep("big")
        for i = 1, 150000 do big[i] = i end
        print(#big)
        "#,
    );
    let mut host = game.open();
    game.write("local big = game.keep(\"big\")\nprint(#big > 1000, #big < 100000)");
    host.reload().unwrap();
    assert_eq!(printed(), ["150000", "true true"]);
    let problems = errors();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("game.keep ran out of room at the reload"),
        "{problems:?}"
    );
}

#[test]
fn a_kept_table_cannot_carry_anything_but_plain_values_into_the_new_version() {
    // Whatever a script hangs on a kept table stays behind in the old VM.
    let game = GameFolder::new(
        r#"
        local saved = game.keep("saved", { n = 1 })
        setmetatable(saved, { __index = function() return "sneaky" end, __iter = error })
        saved.text = "a\0b\255"
        "#,
    );
    let mut host = game.open();
    game.write(
        r#"
        local saved = game.keep("saved")
        print(saved.n, getmetatable(saved), saved.other, #saved.text, saved.text == "a\0b\255")
        "#,
    );
    host.reload().unwrap();
    assert_eq!(printed(), ["1 nil nil 4 true"]);
    assert_eq!(errors(), [] as [&str; 0]);
}
