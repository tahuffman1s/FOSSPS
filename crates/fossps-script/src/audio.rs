//! `game.audio`: a script asks for a sound by name, and the engine plays it.
//!
//! The script never reads a file. It names a sound, the name is looked up in
//! the list the engine gave ([`ScriptHost::set_sounds`](crate::ScriptHost::set_sounds)),
//! and a request waits here for the engine to take. A name can therefore not
//! reach outside the game, whatever is written in it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::rc::Rc;

use mlua::{AnyUserData, Function, Lua, MetaMethod, Table, UserData, UserDataMethods, Value};

use crate::api::{Shared, describe, fail};
use crate::error::closest;

/// Every function on `game.audio`.
pub(crate) const AUDIO_FUNCTIONS: &[&str] = &["play", "playAt", "stopAll"];

/// Every function on a sound. The type definitions are tested against it.
#[cfg(test)]
pub(crate) const SOUND_METHODS: &[&str] = &["stop"];

/// How many requests wait for the engine before the oldest are dropped. An
/// engine that takes them every frame never comes near it; a server, where
/// nobody listens, drops them all.
const MAX_WAITING: usize = 256;

/// How many sound names an error lists before it says how many more there are.
const MAX_LISTED: usize = 20;

/// The slowest and fastest a sound can be played.
const PITCH: (f32, f32) = (0.25, 4.0);

/// One sound a script started. The engine stops it by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundId(pub u64);

/// A sound a script asked to be played.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRequest {
    /// What a later [`AudioCommand::Stop`] names this sound by.
    pub id: SoundId,
    /// The sound's name: one of the names given to
    /// [`ScriptHost::set_sounds`](crate::ScriptHost::set_sounds), when any were.
    pub name: String,
    /// Where in the world the sound comes from, in metres. `None` for a
    /// sound that is not in the world, such as music.
    pub position: Option<[f32; 3]>,
    /// How loud, from 0 to 1.
    pub volume: f32,
    /// How fast, from 0.25 to 4, where 1 is as recorded.
    pub pitch: f32,
    /// How many metres away the sound can still be heard, when the script
    /// said. Always more than 0.
    pub range: Option<f32>,
    /// Whether the sound starts again when it ends, until it is stopped.
    pub looping: bool,
}

/// What a script asked of the engine's sound.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioCommand {
    /// Play a sound.
    Play(SoundRequest),
    /// Stop a sound that was played. It may have ended already.
    Stop(SoundId),
    /// Stop every sound the script has played.
    StopAll,
}

/// The sound a game has and what its script has asked for. It outlives a
/// version of the script, so a save neither loses a request nor gives two
/// sounds the same id.
#[derive(Debug, Default)]
pub(crate) struct AudioState {
    /// The names a script can play, sorted. `None` until the engine says:
    /// then every name is taken.
    names: RefCell<Option<Vec<String>>>,
    next: Cell<u64>,
    waiting: RefCell<VecDeque<AudioCommand>>,
}

impl AudioState {
    pub(crate) fn set_names(&self, mut names: Vec<String>) {
        names.sort();
        names.dedup();
        self.names.replace(Some(names));
    }

    pub(crate) fn take(&self) -> Option<AudioCommand> {
        self.waiting.borrow_mut().pop_front()
    }

    fn push(&self, command: AudioCommand) {
        let mut waiting = self.waiting.borrow_mut();
        if waiting.len() >= MAX_WAITING {
            waiting.pop_front();
        }
        waiting.push_back(command);
    }

    fn next_id(&self) -> SoundId {
        let id = self.next.get();
        self.next.set(id.wrapping_add(1));
        SoundId(id)
    }
}

/// What a script holds for a sound it started.
#[derive(Clone, Copy)]
struct SoundRef(SoundId);

impl UserData for SoundRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("stop", |lua, this: Value| {
            let stopped = match &this {
                Value::UserData(this) => this.borrow::<Self>().ok().map(|this| this.0),
                _ => None,
            };
            let Some(id) = stopped else {
                return Err(fail(
                    lua,
                    "sound.stop needs a colon, not a dot, because it acts on one sound. \
                     Write it like this: sound:stop()",
                ));
            };
            // Stopping a sound that has ended does nothing, so a script
            // never has to ask whether it is still playing.
            audio_of(lua)?.push(AudioCommand::Stop(id));
            Ok(())
        });
        methods.add_meta_function(MetaMethod::ToString, |_, _: AnyUserData| Ok("a sound"));
        methods.add_meta_function(MetaMethod::Index, |lua, (_, key): (Value, Value)| {
            Err::<(), _>(fail(
                lua,
                format!(
                    "sound.{} does not exist. A sound has one function, which stops it: \
                     sound:stop()",
                    key_text(&key)
                ),
            ))
        });
        methods.add_meta_function(
            MetaMethod::NewIndex,
            |lua, (_, key, _): (Value, Value, Value)| {
                Err::<(), _>(fail(
                    lua,
                    format!(
                        "sound.{} cannot be set. To play the sound louder, lower or from \
                         somewhere else, play it again with other options.",
                        key_text(&key)
                    ),
                ))
            },
        );
    }
}

fn key_text(key: &Value) -> String {
    match key {
        Value::String(key) => key.to_string_lossy(),
        other => other.to_string().unwrap_or_default(),
    }
}

fn audio_of(lua: &Lua) -> mlua::Result<Rc<AudioState>> {
    lua.app_data_ref::<Rc<Shared>>()
        .map(|shared| Rc::clone(&shared.audio))
        .ok_or_else(|| mlua::Error::runtime("the game is not ready yet"))
}

/// Which of the two functions was called.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Call {
    Play,
    PlayAt,
}

impl Call {
    fn name(self) -> &'static str {
        match self {
            Self::Play => "game.audio.play",
            Self::PlayAt => "game.audio.playAt",
        }
    }

    /// The call written out, for the end of an error.
    fn example(self, sound: &str) -> String {
        match self {
            Self::Play => format!("game.audio.play(\"{sound}\")"),
            Self::PlayAt => {
                format!("game.audio.playAt(\"{sound}\", vector.create(0, 1, -5))")
            }
        }
    }

    fn options(self) -> &'static [&'static str] {
        match self {
            Self::Play => &["volume", "pitch", "loop"],
            Self::PlayAt => &["volume", "pitch", "loop", "range"],
        }
    }
}

/// Builds the `game.audio` table.
pub(crate) fn table(lua: &Lua, shared: &Rc<Shared>) -> mlua::Result<Table> {
    let audio = lua.create_table()?;
    audio.set("play", function(lua, shared, Call::Play)?)?;
    audio.set("playAt", function(lua, shared, Call::PlayAt)?)?;
    let state = Rc::clone(&shared.audio);
    audio.set(
        "stopAll",
        lua.create_function(move |_, ()| {
            state.push(AudioCommand::StopAll);
            Ok(())
        })?,
    )?;
    Ok(audio)
}

/// `game.audio.play(name, options)` or `game.audio.playAt(name, position,
/// options)`: leaves a request for the engine and gives the script a sound
/// it can stop.
fn function(lua: &Lua, shared: &Rc<Shared>, call: Call) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, (name, second, third): (Value, Value, Value)| {
        let function = call.name();
        if shared.loading.get() {
            return Err(fail(
                lua,
                format!(
                    "{function} cannot be called while the script is loading: nobody is \
                     in the game to hear it yet, and it would play again every time the \
                     file is saved. Put it inside a function that runs later, such as \
                     game.onStart(function() ... end)."
                ),
            ));
        }
        let name = sound_name(lua, &shared.audio, call, &name)?;
        let (position, options) = match call {
            Call::Play => {
                if matches!(second, Value::Vector(_)) {
                    return Err(fail(
                        lua,
                        format!(
                            "game.audio.play plays a sound that is not in the world, so it \
                             takes no place. To play it from a place, write: {}",
                            Call::PlayAt.example(&name)
                        ),
                    ));
                }
                (None, second)
            }
            Call::PlayAt => {
                let Value::Vector(place) = second else {
                    let given = match &second {
                        Value::Table(_) => "a table: the place goes before the options",
                        other => describe(other),
                    };
                    return Err(fail(
                        lua,
                        format!(
                            "game.audio.playAt needs the place the sound comes from, as \
                             vector.create(x, y, z) in metres, but it was given {given}. \
                             Write it like this: {}. For a sound that is not in the world, \
                             such as music, use game.audio.play.",
                            call.example(&name)
                        ),
                    ));
                };
                let place = [place.x(), place.y(), place.z()];
                if !place.iter().all(|part| part.is_finite()) {
                    return Err(fail(
                        lua,
                        "game.audio.playAt was given a place that is not a real point: one \
                         of its numbers is infinite or not a number (often from dividing by \
                         zero). Check how the place is worked out.",
                    ));
                }
                (Some(place), third)
            }
        };
        let mut request = SoundRequest {
            id: shared.audio.next_id(),
            name,
            position,
            volume: 1.0,
            pitch: 1.0,
            range: None,
            looping: false,
        };
        read_options(lua, call, &options, &mut request)?;
        let id = request.id;
        shared.audio.push(AudioCommand::Play(request));
        lua.create_userdata(SoundRef(id))
    })
}

/// The name of the sound to play, checked against the sounds the game has.
fn sound_name(lua: &Lua, audio: &AudioState, call: Call, value: &Value) -> mlua::Result<String> {
    let function = call.name();
    let Value::String(name) = value else {
        return Err(fail(
            lua,
            format!(
                "{function} needs the name of a sound in quotes, but it was given {}. \
                 Write it like this: {}",
                describe(value),
                call.example("gunshot")
            ),
        ));
    };
    let name = name.to_string_lossy();
    let names = audio.names.borrow();
    let Some(names) = names.as_deref() else {
        if name.is_empty() {
            return Err(fail(
                lua,
                format!("{function} was given an empty name. Name the sound to play."),
            ));
        }
        return Ok(name);
    };
    if names.binary_search(&name).is_ok() {
        return Ok(name);
    }

    let known = |candidate: &str| names.iter().any(|name| name == candidate);
    // The two slips that are not typos: the file's name, or its whole path.
    let bare = name.strip_prefix("sounds/").unwrap_or(&name);
    let bare = match bare.len().checked_sub(4) {
        Some(cut) if bare.is_char_boundary(cut) && bare[cut..].eq_ignore_ascii_case(".wav") => {
            &bare[..cut]
        }
        _ => bare,
    };
    let mut message = format!("There is no sound called \"{name}\".");
    if bare != name && known(bare) {
        let _ = write!(
            message,
            " A sound is named by its file in the sounds folder, without the folder \
             and without .wav: write \"{bare}\"."
        );
    } else if let Some(near) = closest(bare, names.iter().map(String::as_str)) {
        let _ = write!(message, " Did you mean \"{near}\"?");
    } else {
        message.push_str(" The sounds are: ");
        for (index, name) in names.iter().take(MAX_LISTED).enumerate() {
            if index > 0 {
                message.push_str(", ");
            }
            message.push_str(name);
        }
        if names.len() > MAX_LISTED {
            let _ = write!(message, " and {} more", names.len() - MAX_LISTED);
        }
        let _ = write!(
            message,
            ". To add one, put {bare}.wav in a folder called sounds inside the game's folder."
        );
    }
    Err(fail(lua, message))
}

/// Reads the table of options into `request`.
fn read_options(
    lua: &Lua,
    call: Call,
    options: &Value,
    request: &mut SoundRequest,
) -> mlua::Result<()> {
    let function = call.name();
    let options = match options {
        Value::Nil => return Ok(()),
        Value::Table(options) => options,
        other => {
            return Err(fail(
                lua,
                format!(
                    "{function} takes its options in a table, but it was given {}. Write \
                     them like this: {{ volume = 0.5, pitch = 1.2 }}",
                    describe(other)
                ),
            ));
        }
    };
    for pair in options.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = key_text(&key);
        let number = |meaning: &str, example: &str| -> mlua::Result<f32> {
            // A sound needs no more precision than an f32 holds.
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let number = match &value {
                Value::Number(number) => *number as f32,
                Value::Integer(number) => *number as f32,
                _ => f32::NAN,
            };
            if number.is_finite() {
                return Ok(number);
            }
            Err(fail(
                lua,
                format!(
                    "{function} needs a number for `{key}`, {meaning}, but it was given \
                     {}. Write it like this: {{ {key} = {example} }}",
                    match &value {
                        Value::Number(_) => "a number that is infinite or not a number",
                        other => describe(other),
                    }
                ),
            ))
        };
        match key.as_str() {
            "volume" => {
                request.volume =
                    number("from 0 (silent) to 1 (as recorded)", "0.5")?.clamp(0.0, 1.0);
            }
            "pitch" => {
                request.pitch =
                    number("where 1 is as recorded, 2 is twice as fast and high", "1.2")?
                        .clamp(PITCH.0, PITCH.1);
            }
            "loop" => match value {
                Value::Boolean(looping) => request.looping = looping,
                other => {
                    return Err(fail(
                        lua,
                        format!(
                            "{function} needs true or false for `loop`, but it was given \
                             {}. Write it like this: {{ loop = true }}",
                            describe(&other)
                        ),
                    ));
                }
            },
            "range" if call == Call::PlayAt => {
                let range = number("how many metres away it can still be heard", "30")?;
                if range <= 0.0 {
                    return Err(fail(
                        lua,
                        format!(
                            "{function} needs a `range` above 0: it is how many metres \
                             away the sound can still be heard. It was given {range}."
                        ),
                    ));
                }
                request.range = Some(range);
            }
            "range" => {
                return Err(fail(
                    lua,
                    "game.audio.play has no `range`, because its sound is not in the world: \
                     it is as loud wherever the player stands. To play a sound from a \
                     place, use game.audio.playAt.",
                ));
            }
            _ => {
                let mut message = format!("{function} has no option called `{key}`.");
                match closest(&key, call.options().iter().copied()) {
                    Some(near) => {
                        let _ = write!(message, " Did you mean `{near}`?");
                    }
                    None => {
                        let _ = write!(message, " Its options are: {}.", call.options().join(", "));
                    }
                }
                return Err(fail(lua, message));
            }
        }
    }
    Ok(())
}
