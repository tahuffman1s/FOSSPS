//! `game.audio`: what a script asks for is what the engine is handed, and a
//! mistake is explained.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fossps_input::{ActionMap, InputFrame};
use fossps_script::{AudioCommand, ScriptHost, SoundId, SoundRequest};

const SOUNDS: [&str; 5] = ["door", "explosion", "gunshot", "music", "ui/click"];

/// Runs `body` on the first tick of a game that has [`SOUNDS`].
fn started(body: &str) -> ScriptHost {
    let script = format!("game.onStart(function()\n{body}\nend)");
    let mut host = ScriptHost::from_source(script, ActionMap::default_fps());
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    host.set_sounds(SOUNDS);
    host.tick(&InputFrame::default(), 1.0 / 60.0);
    host
}

/// Everything the script asked of the sound.
fn asked(host: &mut ScriptHost) -> Vec<AudioCommand> {
    std::iter::from_fn(|| host.take_sound()).collect()
}

/// The error `body` ends in.
fn mistake(body: &str) -> String {
    let mut host = started(body);
    assert_eq!(asked(&mut host), [], "nothing is played by a mistake");
    let error = host.last_error().expect("the script has a mistake");
    assert_eq!(error.line, Some(2), "{error}");
    error.message.clone()
}

#[test]
fn a_sound_is_played_as_recorded_unless_the_script_says_otherwise() {
    let mut host = started(
        r#"game.audio.play("gunshot")
        game.audio.play("music", { volume = 0.4, pitch = 2, loop = true })
        game.audio.playAt("door", vector.create(10, 1.5, -4))
        game.audio.playAt("ui/click", vector.create(0, 0, 0), { range = 80, volume = 1 })"#,
    );
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    let plain = |id, name: &str| SoundRequest {
        id: SoundId(id),
        name: name.to_owned(),
        position: None,
        volume: 1.0,
        pitch: 1.0,
        range: None,
        looping: false,
    };
    assert_eq!(
        asked(&mut host),
        [
            AudioCommand::Play(plain(0, "gunshot")),
            AudioCommand::Play(SoundRequest {
                volume: 0.4,
                pitch: 2.0,
                looping: true,
                ..plain(1, "music")
            }),
            AudioCommand::Play(SoundRequest {
                position: Some([10.0, 1.5, -4.0]),
                ..plain(2, "door")
            }),
            AudioCommand::Play(SoundRequest {
                position: Some([0.0, 0.0, 0.0]),
                range: Some(80.0),
                ..plain(3, "ui/click")
            }),
        ]
    );
    assert_eq!(host.take_sound(), None);
}

#[test]
fn numbers_out_of_range_are_brought_into_it() {
    let mut host = started(
        r#"game.audio.play("music", { volume = 7, pitch = 100 })
        game.audio.play("music", { volume = -1, pitch = 0 })"#,
    );
    let heard: Vec<(f32, f32)> = asked(&mut host)
        .into_iter()
        .map(|command| match command {
            AudioCommand::Play(sound) => (sound.volume, sound.pitch),
            _ => panic!("nothing was stopped"),
        })
        .collect();
    assert_eq!(heard, [(1.0, 4.0), (0.0, 0.25)]);
}

#[test]
fn a_sound_can_be_stopped_and_stopping_twice_is_fine() {
    let mut host = started(
        r#"local music = game.audio.play("music", { loop = true })
        local shot = game.audio.play("gunshot")
        music:stop()
        music:stop()
        print(tostring(shot))
        game.audio.stopAll()"#,
    );
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    let asked = asked(&mut host);
    assert_eq!(asked.len(), 5);
    assert_eq!(
        asked[2..],
        [
            AudioCommand::Stop(SoundId(0)),
            AudioCommand::Stop(SoundId(0)),
            AudioCommand::StopAll
        ]
    );
}

#[test]
fn a_name_that_is_not_a_sound_is_a_mistake_that_names_the_nearest() {
    assert_eq!(
        mistake(r#"game.audio.play("explosoin")"#),
        "There is no sound called \"explosoin\". Did you mean \"explosion\"?"
    );
    assert_eq!(
        mistake(r#"game.audio.playAt("Gunshot", vector.create(0, 0, 0))"#),
        "There is no sound called \"Gunshot\". Did you mean \"gunshot\"?"
    );
    assert_eq!(
        mistake(r#"game.audio.play("victory")"#),
        "There is no sound called \"victory\". The sounds are: door, explosion, gunshot, \
         music, ui/click. To add one, put victory.wav in a folder called sounds inside the \
         game's folder."
    );
    // The file's name and its path are slips of their own.
    for written in ["door.wav", "sounds/door", "sounds/door.WAV"] {
        assert_eq!(
            mistake(&format!(r#"game.audio.play("{written}")"#)),
            format!(
                "There is no sound called \"{written}\". A sound is named by its file in \
                 the sounds folder, without the folder and without .wav: write \"door\"."
            )
        );
    }
}

#[test]
fn a_name_cannot_reach_outside_the_games_sounds() {
    for written in [
        "../secret",
        "..\\\\secret",
        "/etc/passwd",
        "C:/Windows/Media/tada",
        "sounds/../../secret.wav",
        "",
    ] {
        let said = mistake(&format!(r#"game.audio.play("{written}")"#));
        assert!(said.starts_with("There is no sound called"), "{said}");
    }
}

#[test]
fn many_sounds_are_not_all_listed() {
    let script = r#"game.onStart(function() game.audio.play("zzzzzzzzzzzz") end)"#;
    let mut host = ScriptHost::from_source(script, ActionMap::default_fps());
    host.set_sounds((0..30).map(|number| format!("take-{number:02}")));
    host.tick(&InputFrame::default(), 1.0 / 60.0);
    let said = &host.last_error().unwrap().message;
    assert!(said.contains("take-19 and 10 more."), "{said}");
    assert!(!said.contains("take-20"), "{said}");
}

#[test]
fn wrong_arguments_say_how_to_write_the_call() {
    for (body, expected) in [
        (
            "game.audio.play(3)",
            "game.audio.play needs the name of a sound in quotes, but it was given a number. \
             Write it like this: game.audio.play(\"gunshot\")",
        ),
        (
            r#"game.audio.playAt("door")"#,
            "game.audio.playAt needs the place the sound comes from, as \
             vector.create(x, y, z) in metres, but it was given nothing. Write it like \
             this: game.audio.playAt(\"door\", vector.create(0, 1, -5)). For a sound that \
             is not in the world, such as music, use game.audio.play.",
        ),
        (
            r#"game.audio.playAt("door", { volume = 1 })"#,
            "game.audio.playAt needs the place the sound comes from, as \
             vector.create(x, y, z) in metres, but it was given a table: the place goes \
             before the options. Write it like this: game.audio.playAt(\"door\", vector.create(0, 1, -5)). For a sound that \
             is not in the world, such as music, use game.audio.play.",
        ),
        (
            r#"game.audio.play("door", vector.create(1, 2, 3))"#,
            "game.audio.play plays a sound that is not in the world, so it takes no place. \
             To play it from a place, write: \
             game.audio.playAt(\"door\", vector.create(0, 1, -5))",
        ),
        (
            r#"game.audio.playAt("door", vector.create(0 / 0, 2, 3))"#,
            "game.audio.playAt was given a place that is not a real point: one of its \
             numbers is infinite or not a number (often from dividing by zero). Check how \
             the place is worked out.",
        ),
        (
            r#"game.audio.play("door", 0.5)"#,
            "game.audio.play takes its options in a table, but it was given a number. \
             Write them like this: { volume = 0.5, pitch = 1.2 }",
        ),
        (
            r#"game.audio.play("door", { volum = 0.5 })"#,
            "game.audio.play has no option called `volum`. Did you mean `volume`?",
        ),
        (
            r#"game.audio.playAt("door", vector.create(0, 0, 0), { distance = 5 })"#,
            "game.audio.playAt has no option called `distance`. Its options are: volume, \
             pitch, loop, range.",
        ),
        (
            r#"game.audio.play("door", { volume = "loud" })"#,
            "game.audio.play needs a number for `volume`, from 0 (silent) to 1 (as \
             recorded), but it was given text. Write it like this: { volume = 0.5 }",
        ),
        (
            r#"game.audio.play("door", { pitch = 0 / 0 })"#,
            "game.audio.play needs a number for `pitch`, where 1 is as recorded, 2 is \
             twice as fast and high, but it was given a number that is infinite or not a \
             number. Write it like this: { pitch = 1.2 }",
        ),
        (
            r#"game.audio.play("door", { loop = 1 })"#,
            "game.audio.play needs true or false for `loop`, but it was given a number. \
             Write it like this: { loop = true }",
        ),
        (
            r#"game.audio.play("door", { range = 10 })"#,
            "game.audio.play has no `range`, because its sound is not in the world: it is \
             as loud wherever the player stands. To play a sound from a place, use \
             game.audio.playAt.",
        ),
        (
            r#"game.audio.playAt("door", vector.create(0, 0, 0), { range = 0 })"#,
            "game.audio.playAt needs a `range` above 0: it is how many metres away the \
             sound can still be heard. It was given 0.",
        ),
        (
            r#"game.audio.plya("door")"#,
            "game.audio.plya does not exist. Did you mean game.audio.play?",
        ),
        (
            r#"game.audio.play("door").stop()"#,
            "sound.stop needs a colon, not a dot, because it acts on one sound. Write it \
             like this: sound:stop()",
        ),
        (
            r#"local sound = game.audio.play("door"); print(sound.volume)"#,
            "sound.volume does not exist. A sound has one function, which stops it: \
             sound:stop()",
        ),
        (
            r#"local sound = game.audio.play("door"); sound.volume = 1"#,
            "sound.volume cannot be set. To play the sound louder, lower or from somewhere \
             else, play it again with other options.",
        ),
    ] {
        // The last three play a sound before they go wrong.
        let mut host = started(body);
        let error = host.last_error().unwrap_or_else(|| panic!("{body}"));
        assert_eq!(error.message, expected, "{body}");
        assert_eq!(error.line, Some(2), "{body}");
        let plays = asked(&mut host).len();
        assert!(plays <= 1, "{body}");
    }
}

#[test]
fn a_sound_cannot_be_played_while_the_script_loads() {
    let host = ScriptHost::from_source(
        "local music = game.audio.play(\"music\")",
        ActionMap::default_fps(),
    );
    let error = host.last_error().unwrap();
    assert_eq!(
        error.message,
        "game.audio.play cannot be called while the script is loading: nobody is in the \
         game to hear it yet, and it would play again every time the file is saved. Put it \
         inside a function that runs later, such as game.onStart(function() ... end)."
    );
    assert_eq!(error.line, Some(1));
}

#[test]
fn a_host_that_names_no_sounds_takes_any_name() {
    let script = r#"game.onStart(function() game.audio.play("anything/at all") end)"#;
    let mut host = ScriptHost::from_source(script, ActionMap::default_fps());
    host.tick(&InputFrame::default(), 1.0 / 60.0);
    assert!(host.last_error().is_none());
    assert_eq!(asked(&mut host).len(), 1);
}

#[test]
fn only_the_latest_requests_wait_when_nobody_takes_them() {
    let script = r#"game.onTick(function() game.audio.play("gunshot") end)"#;
    let mut host = ScriptHost::from_source(script, ActionMap::default_fps());
    for _ in 0..1000 {
        host.tick(&InputFrame::default(), 1.0 / 60.0);
    }
    let waiting = asked(&mut host);
    assert_eq!(waiting.len(), 256);
    let AudioCommand::Play(last) = waiting.last().unwrap() else {
        panic!("a sound was played");
    };
    assert_eq!(last.id, SoundId(999));
}
