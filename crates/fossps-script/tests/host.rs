//! Runs real scripts from real files through the host.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use fossps_input::{ActionMap, GamepadButton, Input, InputFrame, Key};
use fossps_script::{ENTRY_SCRIPT, OpenError, ScriptHost};

thread_local! {
    /// What scripts on this test's thread printed.
    static PRINTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
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
        }
    }

    fn flush(&self) {}
}

fn printed() -> Vec<String> {
    PRINTED.with_borrow_mut(std::mem::take)
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
        ("require(\"./other\")", "require() is not available yet"),
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
