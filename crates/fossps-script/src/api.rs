//! What a script can see: the `game` table, `print`, and nothing else of the
//! computer it runs on.

use std::cell::{Cell, RefCell};
use std::fmt::{Display, Write as _};
use std::rc::Rc;
use std::time::Instant;

use fossps_input::{ActionKind, ActionMap, ActionState, InputFrame};
use mlua::{Function, Lua, MultiValue, Table, Value, Variadic};

use crate::error::{TookTooLong, closest};

/// Every function directly on `game`. The type definitions shipped for editors
/// are tested against this list.
pub(crate) const GAME_FUNCTIONS: &[&str] = &["onStart", "onTick", "onReload"];

/// Every function on `game.input`.
pub(crate) const INPUT_FUNCTIONS: &[&str] = &[
    "pressed",
    "justPressed",
    "justReleased",
    "value",
    "axis",
    "mouse",
];

/// How many functions one event accepts. Far more than a game needs: reaching
/// it means handlers are being added over and over by mistake.
const MAX_HANDLERS: usize = 1024;

/// The functions a script has asked the engine to call.
#[derive(Default)]
pub(crate) struct Handlers {
    pub start: Vec<Function>,
    pub tick: Vec<Function>,
    pub reload: Vec<Function>,
}

/// State the engine and one script VM both reach.
pub(crate) struct Shared {
    /// The script's file name, used to say where an error happened.
    pub file: String,
    pub actions: Rc<ActionMap>,
    pub handlers: RefCell<Handlers>,
    /// The input for the tick being run.
    pub input: RefCell<InputFrame>,
    /// When the script call in progress must have finished.
    pub deadline: Cell<Option<Instant>>,
    /// Set once the call in progress has run out of time.
    pub stopped: Cell<Option<TookTooLong>>,
    /// How many times the time limit has been checked.
    pub checks: Cell<u32>,
}

/// An error raised by the engine on the script's behalf, pointing at the
/// script line that made the call.
fn fail(lua: &Lua, message: impl Display) -> mlua::Error {
    let line = lua
        .inspect_stack(1, |caller| caller.current_line())
        .flatten();
    let file = lua
        .app_data_ref::<Rc<Shared>>()
        .map(|shared| shared.file.clone());
    match (file, line) {
        (Some(file), Some(line)) => mlua::Error::runtime(format!("{file}:{line}: {message}")),
        _ => mlua::Error::runtime(message),
    }
}

/// How to name a value's kind in an error message.
fn describe(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nothing",
        Value::Boolean(_) => "true or false",
        Value::Integer(_) | Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Table(_) => "a table",
        Value::Function(_) => "a function",
        _ => "something else",
    }
}

/// Puts `game`, `print` and a placeholder `require` into the VM's globals.
pub(crate) fn install(lua: &Lua, shared: &Rc<Shared>) -> mlua::Result<()> {
    lua.set_app_data(Rc::clone(shared));
    let globals = lua.globals();

    let game = lua.create_table()?;
    game.set(
        "onStart",
        on_event(
            lua,
            shared,
            "onStart",
            "game.onStart(function() ... end)",
            |h| &mut h.start,
        )?,
    )?;
    game.set(
        "onTick",
        on_event(
            lua,
            shared,
            "onTick",
            "game.onTick(function(step) ... end)",
            |h| &mut h.tick,
        )?,
    )?;
    game.set(
        "onReload",
        on_event(
            lua,
            shared,
            "onReload",
            "game.onReload(function() ... end)",
            |h| &mut h.reload,
        )?,
    )?;

    let input = lua.create_table()?;
    let any = Reads::Any;
    input.set(
        "pressed",
        reader(lua, shared, "pressed", any, |s| s.pressed)?,
    )?;
    input.set(
        "justPressed",
        reader(lua, shared, "justPressed", any, |s| s.just_pressed)?,
    )?;
    input.set(
        "justReleased",
        reader(lua, shared, "justReleased", any, |s| s.just_released)?,
    )?;
    input.set(
        "value",
        reader(lua, shared, "value", Reads::OneDirection, |s| s.axis[0])?,
    )?;
    let two = Reads::TwoDirections;
    input.set(
        "axis",
        reader(lua, shared, "axis", two, |s| (s.axis[0], s.axis[1]))?,
    )?;
    input.set(
        "mouse",
        reader(lua, shared, "mouse", two, |s| (s.delta[0], s.delta[1]))?,
    )?;
    strict(lua, &input, "game.input", INPUT_FUNCTIONS)?;
    game.set("input", input)?;

    let mut members = GAME_FUNCTIONS.to_vec();
    members.push("input");
    strict(lua, &game, "game", &members)?;
    globals.set("game", game)?;

    globals.set("print", lua.create_function(print)?)?;
    globals.set(
        "require",
        lua.create_function(|lua, _: MultiValue| -> mlua::Result<()> {
            Err(fail(
                lua,
                "require() is not available yet. Put all of your code in main.luau for now; \
                 `game` is ready to use without require.",
            ))
        })?,
    )?;
    Ok(())
}

/// `game.onTick` and friends: remember a function to call later.
fn on_event(
    lua: &Lua,
    shared: &Rc<Shared>,
    name: &'static str,
    example: &'static str,
    list: fn(&mut Handlers) -> &mut Vec<Function>,
) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, value: Value| match value {
        Value::Function(handler) => {
            let mut handlers = shared.handlers.borrow_mut();
            let list = list(&mut handlers);
            if list.len() >= MAX_HANDLERS {
                return Err(fail(
                    lua,
                    format!(
                        "game.{name} has been given {MAX_HANDLERS} functions already. Call \
                         game.{name} once, at the top of the file, not inside another \
                         game.{name} function."
                    ),
                ));
            }
            list.push(handler);
            Ok(())
        }
        other => Err(fail(
            lua,
            format!(
                "game.{name} needs a function, but it was given {}. Write it like this: {example}",
                describe(&other)
            ),
        )),
    })
}

/// Which kinds of action a `game.input` function can read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reads {
    /// Every action can be asked whether it is pressed.
    Any,
    /// Buttons and one-direction axes: a single number.
    OneDirection,
    /// Two-direction axes such as move and look: an x and a y.
    TwoDirections,
}

/// Why `function` cannot read an action of this `kind`, if it cannot. Reading
/// the wrong shape would give a believable but wrong number, so it is an error.
fn wrong_shape(
    reads: Reads,
    kind: Option<ActionKind>,
    function: &str,
    action: &str,
) -> Option<String> {
    let two = kind == Some(ActionKind::Axis2D);
    match reads {
        Reads::OneDirection if two => Some(format!(
            "\"{action}\" has two directions, so game.input.{function} cannot read it. \
             Write: local x, y = game.input.axis(\"{action}\")"
        )),
        Reads::TwoDirections if !two => Some(format!(
            "\"{action}\" has no x and y, so game.input.{function} cannot read it. \
             Write: game.input.pressed(\"{action}\") or game.input.value(\"{action}\")"
        )),
        _ => None,
    }
}

/// A `game.input` function that reads one thing about a named action.
fn reader<R: mlua::IntoLuaMulti + 'static>(
    lua: &Lua,
    shared: &Rc<Shared>,
    name: &'static str,
    reads: Reads,
    read: fn(ActionState) -> R,
) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, value: Value| {
        let Value::String(action) = &value else {
            return Err(fail(
                lua,
                format!(
                    "game.input.{name} needs the name of an action in quotes, but it was \
                     given {}. Write it like this: game.input.{name}(\"jump\")",
                    describe(&value)
                ),
            ));
        };
        let action = action.to_str()?;
        match shared.actions.action(&action) {
            Ok(id) => match wrong_shape(reads, shared.actions.kind(id), name, &action) {
                Some(message) => Err(fail(lua, message)),
                None => Ok(read(shared.input.borrow().get(id))),
            },
            Err(unknown) => {
                let mut message = unknown.to_string();
                if unknown.suggestion.is_none() {
                    message.push_str(" The actions are: ");
                    for (index, id) in shared.actions.actions().enumerate() {
                        if index > 0 {
                            message.push_str(", ");
                        }
                        message.push_str(shared.actions.name(id));
                    }
                    message.push('.');
                }
                Err(fail(lua, message))
            }
        }
    })
}

/// Makes reading a missing member of `table` an error that names the closest
/// real one, and stops scripts from changing the table.
fn strict(
    lua: &Lua,
    table: &Table,
    path: &'static str,
    members: &[&'static str],
) -> mlua::Result<()> {
    let members = members.to_vec();
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Value, Value)| -> mlua::Result<()> {
            let key = match &key {
                Value::String(key) => key.to_string_lossy(),
                other => other.to_string()?,
            };
            let mut message = format!("{path}.{key} does not exist.");
            match closest(&key, members.iter().copied()) {
                Some(near) => {
                    let _ = write!(message, " Did you mean {path}.{near}?");
                }
                None => {
                    let _ = write!(message, " {path} has: {}.", members.join(", "));
                }
            }
            Err(fail(lua, message))
        })?,
    )?;
    meta.set("__metatable", false)?;
    table.set_metatable(Some(meta))?;
    table.set_readonly(true);
    Ok(())
}

/// `print(...)`: writes to the engine's log, like a console line.
fn print(_: &Lua, values: Variadic<Value>) -> mlua::Result<()> {
    let mut line = String::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            line.push(' ');
        }
        line.push_str(&value.to_string()?);
    }
    log::info!(target: "script", "{line}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_definitions_describe_exactly_the_real_api() {
        let declared: Vec<&str> = crate::TYPE_DEFINITIONS
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .filter_map(|line| line.trim().split_once(": (").map(|(name, _)| name))
            .collect();
        let real: Vec<&str> = GAME_FUNCTIONS
            .iter()
            .chain(INPUT_FUNCTIONS)
            .copied()
            .collect();
        assert_eq!(declared, real);
    }
}
