//! `game.keep`: tables a script asks to have handed back after a reload.
//!
//! A reload runs the script in a fresh VM, and nothing can be carried from
//! one VM into another. So just before a reload the kept tables are copied
//! out of the old VM as plain data, and the new version's `game.keep` calls
//! build new tables from the copies.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use fossps_mode::PlayerId;
use mlua::{Function, Lua, Table, Value};

use crate::api::{Shared, describe, fail};
use crate::mode_api;

/// How many tables one version of a script can keep. Far more than a game
/// needs: reaching it means `game.keep` is being called with made-up names.
const MAX_KEPT: usize = 256;

/// How deep tables inside a kept table are followed.
const MAX_DEPTH: usize = 32;

/// How many keys and values one reload carries over, all kept tables
/// together, so that copying them cannot hold the game up.
const MAX_VALUES: usize = 100_000;

/// How many bytes of text one reload carries over.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// How many things that could not be kept are named at one reload.
const MAX_PROBLEMS: usize = 8;

/// How much of an odd key is shown when saying where a problem is.
const MAX_KEY_SHOWN: usize = 40;

const EXAMPLE: &str = "local saved = game.keep(\"saved\", { best = 0 })";

const KEEPS: &str = "A kept table can hold numbers, text, true and false, players, and \
     tables of those. Take this out of the table, or keep a name or a number in its place.";

/// A value that can outlive the VM it was made in.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Plain {
    Bool(bool),
    Integer(mlua::Integer),
    Number(f64),
    /// Luau text is bytes, and need not be UTF-8.
    Text(Box<[u8]>),
    Vector(mlua::Vector),
    /// A player, who is given back if they are still in the match.
    Player(PlayerId),
    /// The keys and values, in the order the table gave them.
    Table(Vec<(Plain, Plain)>),
}

/// What the last version of the script kept, by the name it gave each table.
pub(crate) type Store = HashMap<String, Plain>;

/// The tables one version of the script has asked to keep.
#[derive(Default)]
pub(crate) struct Kept {
    tables: RefCell<Vec<(String, Table)>>,
}

impl Kept {
    /// Copies every kept table out of the VM. Also gives a message for each
    /// thing that could not be kept and was left out.
    pub fn snapshot(&self) -> (Store, Vec<String>) {
        let mut copier = Copier {
            values_left: MAX_VALUES,
            bytes_left: MAX_BYTES,
            full: false,
            name: String::new(),
            path: String::new(),
            problems: Vec::new(),
        };
        let mut store = Store::new();
        for (name, table) in self.tables.borrow().iter() {
            copier.name.clone_from(name);
            copier.path.clear();
            let copy = copier.table(table, 0);
            store.insert(name.clone(), Plain::Table(copy));
        }
        (store, copier.problems)
    }
}

/// Copies tables out of a VM, within the limits above.
struct Copier {
    values_left: usize,
    bytes_left: usize,
    /// Set once a limit is reached: nothing more is copied.
    full: bool,
    /// The kept table being copied.
    name: String,
    /// Where in that table the copier is, such as `.best[2]`.
    path: String,
    problems: Vec<String>,
}

impl Copier {
    fn table(&mut self, table: &Table, depth: usize) -> Vec<(Plain, Plain)> {
        let mut copy = Vec::new();
        // This reads the table as it is, without calling anything the script
        // may have attached to it.
        let walked = table.for_each(|key: Value, value: Value| {
            if self.full {
                // Stops the walk: there is no room for anything more.
                return Err(mlua::Error::runtime("full"));
            }
            let before = self.path.len();
            self.step_into(&key);
            if let Some(key) = self.value(&key, depth, true)
                && let Some(value) = self.value(&value, depth, false)
            {
                copy.push((key, value));
            }
            self.path.truncate(before);
            Ok(())
        });
        if let Err(error) = walked
            && !self.full
        {
            self.problem(format_args!("it could not be read ({error})"));
        }
        copy
    }

    /// The copy of one key or value, or nothing if it cannot be kept.
    fn value(&mut self, value: &Value, depth: usize, is_key: bool) -> Option<Plain> {
        if self.values_left == 0 {
            return self.out_of_room();
        }
        self.values_left -= 1;
        let plain = match value {
            Value::Boolean(value) => Plain::Bool(*value),
            Value::Integer(value) => Plain::Integer(*value),
            Value::Number(value) => Plain::Number(*value),
            Value::Vector(value) => Plain::Vector(*value),
            Value::String(text) => {
                let bytes = text.as_bytes();
                if bytes.len() > self.bytes_left {
                    return self.out_of_room();
                }
                self.bytes_left -= bytes.len();
                Plain::Text(Box::from(&*bytes))
            }
            Value::UserData(handle) if let Some(id) = mode_api::player_id(handle) => {
                Plain::Player(id)
            }
            Value::Table(_) if is_key => {
                self.problem(format_args!("its key is a table"));
                return None;
            }
            Value::Table(_) if depth + 1 >= MAX_DEPTH => {
                self.problem(format_args!(
                    "it is more than {MAX_DEPTH} tables deep, so it was cut off there. A \
                     table that holds itself goes on for ever"
                ));
                return None;
            }
            Value::Table(table) => Plain::Table(self.table(table, depth + 1)),
            other => {
                let what = match other {
                    Value::UserData(handle) if mode_api::is_team(handle) => {
                        "a team. Keep the team's name instead: team.name"
                    }
                    other => describe(other),
                };
                if is_key {
                    self.problem(format_args!("its key is {what}"));
                } else {
                    self.problem(format_args!("it is {what}"));
                }
                return None;
            }
        };
        Some(plain)
    }

    /// Adds a key to the path, as a script would write it.
    fn step_into(&mut self, key: &Value) {
        let path = &mut self.path;
        let _ = match key {
            Value::String(text) => {
                let text = text.to_string_lossy();
                let mut letters = text.chars();
                let is_name = letters
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                    && letters.all(|letter| letter.is_ascii_alphanumeric() || letter == '_');
                if is_name && text.len() <= MAX_KEY_SHOWN {
                    write!(path, ".{text}")
                } else {
                    let shown: String = text.chars().take(MAX_KEY_SHOWN).collect();
                    write!(path, "[{shown:?}]")
                }
            }
            Value::Integer(number) => write!(path, "[{number}]"),
            Value::Number(number) => write!(path, "[{number}]"),
            Value::Boolean(value) => write!(path, "[{value}]"),
            Value::UserData(handle) if mode_api::player_id(handle).is_some() => {
                write!(path, "[a player]")
            }
            _ => write!(path, "[...]"),
        };
    }

    /// Notes that the value at the current path was left out, and why.
    fn problem(&mut self, why: std::fmt::Arguments<'_>) {
        if self.problems.len() >= MAX_PROBLEMS {
            return;
        }
        let name = &self.name;
        let place = if self.path.is_empty() {
            format!("game.keep(\"{name}\")")
        } else {
            format!(
                "`{}` in game.keep(\"{name}\")",
                self.path.trim_start_matches('.')
            )
        };
        let mut message = format!("{place} was not kept across the reload: {why}.");
        if self.problems.len() + 1 == MAX_PROBLEMS {
            message.push_str(" Any more like this are not listed.");
        } else {
            let _ = write!(message, " {KEEPS}");
        }
        self.problems.push(message);
    }

    fn out_of_room(&mut self) -> Option<Plain> {
        self.full = true;
        // This one is always said, however many came before it.
        self.problems.truncate(MAX_PROBLEMS - 1);
        self.problems.push(format!(
            "game.keep ran out of room at the reload: all kept tables together can hold \
             {MAX_VALUES} keys and values and {} MB of text. What did not fit was not \
             kept. Keep less: only what must not start over when the script is saved.",
            MAX_BYTES / (1024 * 1024)
        ));
        None
    }
}

/// Builds a kept value in this VM. Gives nothing for a player who has left
/// the match since.
fn restore(lua: &Lua, shared: &Shared, plain: &Plain) -> mlua::Result<Option<Value>> {
    Ok(Some(match plain {
        Plain::Bool(value) => Value::Boolean(*value),
        Plain::Integer(value) => Value::Integer(*value),
        Plain::Number(value) => Value::Number(*value),
        Plain::Vector(value) => Value::Vector(*value),
        Plain::Text(bytes) => Value::String(lua.create_string(bytes)?),
        Plain::Player(id) => {
            if shared.mode.game.borrow().player(*id).is_none() {
                return Ok(None);
            }
            Value::UserData(shared.handles.player(lua, *id)?)
        }
        Plain::Table(pairs) => Value::Table(restore_table(lua, shared, pairs)?),
    }))
}

fn restore_table(lua: &Lua, shared: &Shared, pairs: &[(Plain, Plain)]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (key, value) in pairs {
        if let Some(key) = restore(lua, shared, key)?
            && let Some(value) = restore(lua, shared, value)?
        {
            table.raw_set(key, value)?;
        }
    }
    Ok(table)
}

/// `game.keep(name, start)`: a table that is still there, with what was put
/// in it, after the script is saved and loaded again.
pub(crate) fn function(lua: &Lua, shared: &Rc<Shared>) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, (name, start): (Value, Value)| {
        let Value::String(name) = &name else {
            return Err(fail(
                lua,
                format!(
                    "game.keep needs a name in quotes first, but it was given {}. The name \
                     is how the table is found again after a reload. Write it like this: \
                     {EXAMPLE}",
                    describe(&name)
                ),
            ));
        };
        let name = name.to_string_lossy();
        let start = match start {
            Value::Table(start) => Some(start),
            Value::Nil => None,
            other => {
                return Err(fail(
                    lua,
                    format!(
                        "game.keep keeps a table, but it was given {}. A number or text \
                         on its own is copied each time it is used, so a kept one could \
                         never change. Put it in a table and keep the table: local saved = \
                         game.keep(\"saved\", {{ count = 0 }}), then saved.count += 1",
                        describe(&other)
                    ),
                ));
            }
        };
        let asked_before = shared
            .kept
            .tables
            .borrow()
            .iter()
            .find(|(kept, _)| *kept == name)
            .map(|(_, table)| table.clone());
        let table = if let Some(table) = asked_before {
            // Two files that keep the same name share one table.
            table
        } else {
            if shared.kept.tables.borrow().len() >= MAX_KEPT {
                return Err(fail(
                    lua,
                    format!(
                        "game.keep has been given {MAX_KEPT} different names already. Call \
                         it at the top of the file with a name that stays the same, and \
                         put what changes inside the table."
                    ),
                ));
            }
            let kept = match shared.store.borrow().get(&name) {
                Some(Plain::Table(pairs)) => Some(restore_table(lua, &shared, pairs)?),
                _ => None,
            };
            let table = match (kept, &start) {
                (Some(kept), _) => kept,
                // Nothing was kept: the script's own table is the one.
                (None, Some(start)) => start.clone(),
                (None, None) => lua.create_table()?,
            };
            shared.kept.tables.borrow_mut().push((name, table.clone()));
            table
        };
        // What the script starts with now and the kept table lacks is added,
        // so a new setting shows up without starting everything over.
        if let Some(start) = start
            && start != table
        {
            start.for_each(|key: Value, value: Value| {
                if table.raw_get::<Value>(key.clone())?.is_nil() {
                    table.raw_set(key, value)?;
                }
                Ok(())
            })?;
        }
        Ok(table)
    })
}
