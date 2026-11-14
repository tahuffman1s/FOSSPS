//! The game-mode part of `game`: `game.mode`, players, teams and the round.
//!
//! The match itself is a [`fossps_mode::Match`] that the host owns, outside
//! the script's VM, so saving the script does not reset the score. Everything
//! here borrows it for one operation and lets go before returning; no script
//! code ever runs while it is borrowed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use fossps_mode::{Match, Phase, Player, PlayerId, Rules, TeamId, Winner};
use mlua::{
    AnyUserData, Function, Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, Value,
};

use crate::api::{Shared, describe, fail, strict};
use crate::error::closest;

/// Every setting `game.mode` takes.
pub(crate) const MODE_SETTINGS: &[&str] = &[
    "name",
    "teams",
    "scoreToWin",
    "killPoints",
    "timeLimit",
    "respawnDelay",
    "warmup",
    "intermission",
];

/// Every function on `game.round`.
pub(crate) const ROUND_FUNCTIONS: &[&str] = &["phase", "timeLeft", "finish"];

/// What can be read from a player.
pub(crate) const PLAYER_FIELDS: &[&str] = &["name", "team", "score", "kills", "deaths", "alive"];

/// What a player can be told to do.
pub(crate) const PLAYER_METHODS: &[&str] = &["addScore"];

/// What can be read from a team.
pub(crate) const TEAM_FIELDS: &[&str] = &["name", "score"];

/// What a team can be asked or told to do.
pub(crate) const TEAM_METHODS: &[&str] = &["players", "addScore"];

/// The most points one call can add or take away.
const MAX_POINTS: f64 = 1_000_000.0;

const MODE_EXAMPLE: &str =
    "game.mode { name = \"Team Deathmatch\", teams = { \"Red\", \"Blue\" }, scoreToWin = 50 }";

/// The match, shared by the host and every version of the script.
pub(crate) struct ModeState {
    pub game: RefCell<Match>,
    /// Players who left while their leaving is still being told to the
    /// script, so `game.onPlayerLeave` can read who they were.
    pub gone: RefCell<Vec<Player>>,
}

impl ModeState {
    pub fn new() -> Self {
        Self {
            game: RefCell::new(Match::default()),
            gone: RefCell::new(Vec::new()),
        }
    }
}

/// The script's values for players and teams. One player is always the same
/// value, so players work as table keys and compare equal with `==`.
#[derive(Default)]
pub(crate) struct Handles {
    players: RefCell<HashMap<PlayerId, AnyUserData>>,
    teams: RefCell<HashMap<TeamId, AnyUserData>>,
}

impl Handles {
    /// The script's value for a player.
    pub fn player(&self, lua: &Lua, id: PlayerId) -> mlua::Result<AnyUserData> {
        if let Some(handle) = self.players.borrow().get(&id) {
            return Ok(handle.clone());
        }
        let handle = lua.create_userdata(PlayerRef(id))?;
        self.players.borrow_mut().insert(id, handle.clone());
        Ok(handle)
    }

    /// The script's value for a team.
    pub fn team(&self, lua: &Lua, id: TeamId) -> mlua::Result<AnyUserData> {
        if let Some(handle) = self.teams.borrow().get(&id) {
            return Ok(handle.clone());
        }
        let handle = lua.create_userdata(TeamRef(id))?;
        self.teams.borrow_mut().insert(id, handle.clone());
        Ok(handle)
    }

    /// The script's value for whoever won, or nothing for a draw.
    pub fn winner(&self, lua: &Lua, winner: Option<Winner>) -> mlua::Result<Option<AnyUserData>> {
        match winner {
            Some(Winner::Team(id)) => self.team(lua, id).map(Some),
            Some(Winner::Player(id)) => self.player(lua, id).map(Some),
            None => Ok(None),
        }
    }

    /// Stops remembering a player who has left for good.
    pub fn forget(&self, id: PlayerId) {
        self.players.borrow_mut().remove(&id);
    }
}

/// A player, as a script holds one.
#[derive(Clone, Copy)]
struct PlayerRef(PlayerId);

/// A team, as a script holds one.
#[derive(Clone, Copy)]
struct TeamRef(TeamId);

fn shared_of(lua: &Lua) -> mlua::Result<Rc<Shared>> {
    lua.app_data_ref::<Rc<Shared>>()
        .map(|shared| Rc::clone(&shared))
        .ok_or_else(|| mlua::Error::runtime("the game is not ready yet"))
}

impl PlayerRef {
    /// Reads something about the player, who may have just left.
    fn read<R>(self, lua: &Lua, field: &str, read: impl FnOnce(&Player) -> R) -> mlua::Result<R> {
        let shared = shared_of(lua)?;
        if let Some(player) = shared.mode.game.borrow().player(self.0) {
            return Ok(read(player));
        }
        if let Some(player) = shared
            .mode
            .gone
            .borrow()
            .iter()
            .find(|player| player.id == self.0)
        {
            return Ok(read(player));
        }
        Err(fail(
            lua,
            format!(
                "This player has left the match, so player.{field} cannot be read. Stop \
                 keeping a player in game.onPlayerLeave; game.players() lists who is here now."
            ),
        ))
    }
}

impl UserData for PlayerRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |lua, this| {
            this.read(lua, "name", |player| player.name.clone())
        });
        fields.add_field_method_get("team", |lua, this| {
            match this.read(lua, "team", |player| player.team)? {
                Some(team) => shared_of(lua)?.handles.team(lua, team).map(Some),
                None => Ok(None),
            }
        });
        fields.add_field_method_get("score", |lua, this| {
            this.read(lua, "score", |player| player.score)
        });
        fields.add_field_method_get("kills", |lua, this| {
            this.read(lua, "kills", |player| player.kills)
        });
        fields.add_field_method_get("deaths", |lua, this| {
            this.read(lua, "deaths", |player| player.deaths)
        });
        fields.add_field_method_get("alive", |lua, this| {
            this.read(lua, "alive", |player| player.alive)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("addScore", |lua, (this, points): (Value, Value)| {
            let this = receiver::<Self>(lua, &this, "player", "addScore", "(1)")?;
            let points = whole_points(lua, &points, "player")?;
            // Does nothing for a player who has left, like every score
            // outside a round.
            shared_of(lua)?
                .mode
                .game
                .borrow_mut()
                .add_score(this.0, points);
            Ok(())
        });
        methods.add_meta_function(MetaMethod::ToString, |lua, this: AnyUserData| {
            let this = *this.borrow::<Self>()?;
            Ok(this
                .read(lua, "name", |player| player.name.clone())
                .unwrap_or_else(|_| "a player who has left".to_owned()))
        });
        methods.add_meta_function(MetaMethod::Index, |lua, (_, key): (Value, Value)| {
            Err::<(), _>(missing(lua, "player", &key, PLAYER_FIELDS, PLAYER_METHODS))
        });
        methods.add_meta_function(
            MetaMethod::NewIndex,
            |lua, (_, key, _): (Value, Value, Value)| Err::<(), _>(read_only(lua, "player", &key)),
        );
    }
}

impl UserData for TeamRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |lua, this| {
            let shared = shared_of(lua)?;
            let game = shared.mode.game.borrow();
            Ok(game.team(this.0).map(|team| team.name.clone()))
        });
        fields.add_field_method_get("score", |lua, this| {
            let shared = shared_of(lua)?;
            let game = shared.mode.game.borrow();
            Ok(game.team(this.0).map_or(0, |team| team.score))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("players", |lua, this: Value| {
            let this = receiver::<Self>(lua, &this, "team", "players", "()")?;
            players(lua, Some(this.0))
        });
        methods.add_function("addScore", |lua, (this, points): (Value, Value)| {
            let this = receiver::<Self>(lua, &this, "team", "addScore", "(1)")?;
            let points = whole_points(lua, &points, "team")?;
            shared_of(lua)?
                .mode
                .game
                .borrow_mut()
                .add_team_score(this.0, points);
            Ok(())
        });
        methods.add_meta_function(MetaMethod::ToString, |lua, this: AnyUserData| {
            let this = *this.borrow::<Self>()?;
            let shared = shared_of(lua)?;
            let game = shared.mode.game.borrow();
            Ok(game
                .team(this.0)
                .map_or_else(|| "a team".to_owned(), |team| team.name.clone()))
        });
        methods.add_meta_function(MetaMethod::Index, |lua, (_, key): (Value, Value)| {
            Err::<(), _>(missing(lua, "team", &key, TEAM_FIELDS, TEAM_METHODS))
        });
        methods.add_meta_function(
            MetaMethod::NewIndex,
            |lua, (_, key, _): (Value, Value, Value)| Err::<(), _>(read_only(lua, "team", &key)),
        );
    }
}

/// The player or team a method was called on, or an error saying to use a
/// colon if it was called with a dot.
fn receiver<T: UserData + Copy + 'static>(
    lua: &Lua,
    this: &Value,
    what: &str,
    method: &str,
    arguments: &str,
) -> mlua::Result<T> {
    if let Value::UserData(this) = this
        && let Ok(this) = this.borrow::<T>()
    {
        return Ok(*this);
    }
    Err(fail(
        lua,
        format!(
            "{what}.{method} needs a colon, not a dot, because it acts on one {what}. \
             Write it like this: {what}:{method}{arguments}"
        ),
    ))
}

/// The points handed to `addScore`.
fn whole_points(lua: &Lua, value: &Value, what: &str) -> mlua::Result<i32> {
    let points = match value {
        Value::Integer(points) => *points as f64,
        Value::Number(points) => *points,
        other => {
            return Err(fail(
                lua,
                format!(
                    "{what}:addScore needs a number of points, but it was given {}. \
                     Write it like this: {what}:addScore(1)",
                    describe(other)
                ),
            ));
        }
    };
    if points.fract() != 0.0 || points.abs() > MAX_POINTS {
        return Err(fail(
            lua,
            format!(
                "{what}:addScore needs a whole number of points between -1000000 and \
                 1000000, but it was given {points}. Scores are whole numbers: \
                 round it first with math.floor."
            ),
        ));
    }
    Ok(points as i32)
}

/// The error for reading something a player or team does not have.
fn missing(lua: &Lua, what: &str, key: &Value, fields: &[&str], methods: &[&str]) -> mlua::Error {
    let key = match key {
        Value::String(key) => key.to_string_lossy(),
        other => other.to_string().unwrap_or_default(),
    };
    let mut message = format!("{what}.{key} does not exist.");
    match closest(&key, fields.iter().chain(methods).copied()) {
        Some(near) if methods.contains(&near) => {
            let _ = write!(message, " Did you mean {what}:{near}?");
        }
        Some(near) => {
            let _ = write!(message, " Did you mean {what}.{near}?");
        }
        None => {
            let _ = write!(
                message,
                " A {what} has: {}, and the functions {}.",
                fields.join(", "),
                methods
                    .iter()
                    .map(|method| format!("{what}:{method}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    fail(lua, message)
}

/// The error for assigning to something on a player or team.
fn read_only(lua: &Lua, what: &str, key: &Value) -> mlua::Error {
    let key = match key {
        Value::String(key) => key.to_string_lossy(),
        other => other.to_string().unwrap_or_default(),
    };
    let advice = if key == "score" {
        format!("To change a score, write: {what}:addScore(1)")
    } else {
        format!(
            "The engine keeps a {what}'s details; to remember something of your own about \
             a {what}, use a table of your own: notes[{what}] = \"something\""
        )
    };
    fail(lua, format!("{what}.{key} cannot be set. {advice}"))
}

/// A list of the players on `team`, or of everyone.
fn players(lua: &Lua, team: Option<TeamId>) -> mlua::Result<Table> {
    let shared = shared_of(lua)?;
    let list = lua.create_table()?;
    // The ids are copied out first: making a value can run the garbage
    // collector, and nothing of the script's may run while the match is
    // borrowed.
    let ids: Vec<PlayerId> = shared
        .mode
        .game
        .borrow()
        .players()
        .iter()
        .filter(|player| team.is_none() || player.team == team)
        .map(|player| player.id)
        .collect();
    for id in ids {
        list.raw_push(shared.handles.player(lua, id)?)?;
    }
    Ok(list)
}

/// Adds the game-mode functions to `game`.
pub(crate) fn install(lua: &Lua, shared: &Rc<Shared>, game: &Table) -> mlua::Result<()> {
    game.set("mode", mode_function(lua, shared)?)?;

    game.set(
        "players",
        lua.create_function(|lua, ()| players(lua, None))?,
    )?;
    game.set(
        "teams",
        lua.create_function(|lua, ()| {
            let shared = shared_of(lua)?;
            let list = lua.create_table()?;
            let count = shared.mode.game.borrow().teams().len();
            for index in 0..count {
                if let Ok(index) = u8::try_from(index) {
                    list.raw_push(shared.handles.team(lua, TeamId(index))?)?;
                }
            }
            Ok(list)
        })?,
    )?;
    game.set("team", lua.create_function(team_by_name)?)?;

    // These are functions, not values read like `player.score`: the VM looks
    // up `game.round.phase` once, when the file loads, and would never see
    // it change.
    let round = lua.create_table()?;
    round.set(
        "phase",
        lua.create_function(|lua, ()| {
            Ok(match shared_of(lua)?.mode.game.borrow().phase() {
                Phase::Waiting => "waiting",
                Phase::Warmup => "warmup",
                Phase::Playing => "playing",
                Phase::Over => "over",
            })
        })?,
    )?;
    round.set(
        "timeLeft",
        lua.create_function(|lua, ()| {
            let left = shared_of(lua)?.mode.game.borrow().time_left();
            Ok(left.map_or(f64::INFINITY, f64::from))
        })?,
    )?;
    round.set("finish", lua.create_function(finish)?)?;
    strict(lua, &round, "game.round", ROUND_FUNCTIONS)?;
    game.set("round", round)?;
    Ok(())
}

/// `game.team("Red")`.
fn team_by_name(lua: &Lua, name: Value) -> mlua::Result<AnyUserData> {
    let Value::String(name) = &name else {
        return Err(fail(
            lua,
            format!(
                "game.team needs a team's name in quotes, but it was given {}. \
                 Write it like this: game.team(\"Red\")",
                describe(&name)
            ),
        ));
    };
    let name = name.to_str()?;
    let shared = shared_of(lua)?;
    let found = shared.mode.game.borrow().team_named(&name);
    if let Some(id) = found {
        return shared.handles.team(lua, id);
    }
    let game = shared.mode.game.borrow();
    let names: Vec<&str> = game.teams().iter().map(|team| team.name.as_str()).collect();
    let mut message = format!("There is no team called \"{name}\".");
    if names.is_empty() {
        message.push_str(
            " This mode has no teams. List them when you set up the mode: \
             game.mode { teams = { \"Red\", \"Blue\" } }",
        );
    } else if let Some(near) = closest(&name, names.iter().copied()) {
        let _ = write!(message, " Did you mean \"{near}\"?");
    } else {
        let _ = write!(message, " The teams are: {}.", names.join(", "));
    }
    Err(fail(lua, message))
}

/// `game.round.finish(winner)`.
fn finish(lua: &Lua, winner: Value) -> mlua::Result<()> {
    let shared = shared_of(lua)?;
    let has_teams = !shared.mode.game.borrow().teams().is_empty();
    let winner = match &winner {
        Value::Nil => None,
        Value::UserData(handle) => {
            if let Ok(team) = handle.borrow::<TeamRef>() {
                Some(Winner::Team(team.0))
            } else if let Ok(player) = handle.borrow::<PlayerRef>() {
                if has_teams {
                    return Err(fail(
                        lua,
                        "game.round.finish was given a player, but this mode has teams, so a \
                         team wins the round. Write it like this: game.round.finish(player.team)",
                    ));
                }
                Some(Winner::Player(player.0))
            } else {
                return Err(wrong_winner(lua, &winner, has_teams));
            }
        }
        other => return Err(wrong_winner(lua, other, has_teams)),
    };
    shared.mode.game.borrow_mut().finish(winner);
    Ok(())
}

fn wrong_winner(lua: &Lua, given: &Value, has_teams: bool) -> mlua::Error {
    let (who, example) = if has_teams {
        ("the team that won", "game.round.finish(game.team(\"Red\"))")
    } else {
        ("the player who won", "game.round.finish(player)")
    };
    fail(
        lua,
        format!(
            "game.round.finish needs {who}, or nothing for a draw, but it was given {}. \
             Write it like this: {example}",
            describe(given)
        ),
    )
}

/// `game.mode { ... }`: remembers the rules the script asks for. The host
/// puts them into effect once the whole file has loaded without a mistake.
fn mode_function(lua: &Lua, shared: &Rc<Shared>) -> mlua::Result<Function> {
    let shared = Rc::clone(shared);
    lua.create_function(move |lua, settings: Value| {
        if !shared.loading.get() {
            return Err(fail(
                lua,
                "game.mode sets up the whole match, so it cannot be called inside a function \
                 that runs later, such as game.onTick. Call it once, near the top of the file.",
            ));
        }
        if let Some((_, line)) = &*shared.rules.borrow() {
            let earlier = line.map_or_else(String::new, |line| format!(" on line {line}"));
            return Err(fail(
                lua,
                format!(
                    "game.mode was already called{earlier}. A game has one mode: put all \
                     of the settings in that one call."
                ),
            ));
        }
        let Value::Table(settings) = &settings else {
            return Err(fail(
                lua,
                format!(
                    "game.mode needs a list of settings in curly brackets, but it was given \
                     {}. Write it like this: {MODE_EXAMPLE}",
                    describe(&settings)
                ),
            ));
        };
        let rules = read_rules(lua, settings)?;
        let line = lua
            .inspect_stack(1, |caller| caller.current_line())
            .flatten();
        *shared.rules.borrow_mut() = Some((rules, line));
        Ok(())
    })
}

/// Turns the table given to `game.mode` into rules, or says what is wrong
/// with it.
fn read_rules(lua: &Lua, settings: &Table) -> mlua::Result<Rules> {
    let mut rules = Rules::default();
    let mut named = false;
    for pair in settings.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let Value::String(key) = &key else {
            return Err(fail(
                lua,
                format!(
                    "Every setting in game.mode needs a name, such as scoreToWin = 50, but \
                     one was given without a name. Write it like this: {MODE_EXAMPLE}"
                ),
            ));
        };
        let key = key.to_str()?;
        match &*key {
            "name" => {
                rules.name = text(lua, "name", &value, "\"Team Deathmatch\"")?;
                named = true;
            }
            "teams" => rules.teams = team_names(lua, &value)?,
            "scoreToWin" => {
                rules.score_to_win =
                    whole(lua, "scoreToWin", &value, "50", "Use 0 for no score limit.")?;
            }
            "killPoints" => {
                rules.kill_points = whole(
                    lua,
                    "killPoints",
                    &value,
                    "1",
                    "Use 0 if kills should not score.",
                )?;
            }
            "timeLimit" => {
                rules.time_limit =
                    seconds(lua, "timeLimit", &value, "600", "Use 0 for no time limit.")?;
            }
            "respawnDelay" => {
                rules.respawn_delay = seconds(
                    lua,
                    "respawnDelay",
                    &value,
                    "3",
                    "Use 0 to respawn at once.",
                )?;
            }
            "warmup" => {
                rules.warmup = seconds(lua, "warmup", &value, "5", "Use 0 for no warm-up.")?;
            }
            "intermission" => {
                rules.intermission = seconds(
                    lua,
                    "intermission",
                    &value,
                    "10",
                    "Use 0 to start the next round at once.",
                )?;
            }
            unknown => {
                let mut message = format!("game.mode has no setting called {unknown}.");
                match closest(unknown, MODE_SETTINGS.iter().copied()) {
                    Some(near) => {
                        let _ = write!(message, " Did you mean {near}?");
                    }
                    None => {
                        let _ = write!(message, " The settings are: {}.", MODE_SETTINGS.join(", "));
                    }
                }
                return Err(fail(lua, message));
            }
        }
    }
    if !named && !rules.teams.is_empty() {
        rules.name = "Team Deathmatch".to_owned();
    }
    rules.check().map_err(|problem| {
        fail(
            lua,
            format!("game.mode cannot use these settings: {problem}"),
        )
    })?;
    Ok(rules)
}

fn wrong_kind(lua: &Lua, key: &str, needs: &str, value: &Value, example: &str) -> mlua::Error {
    fail(
        lua,
        format!(
            "game.mode: {key} needs {needs}, but it was given {}. \
             Write it like this: {key} = {example}",
            describe(value)
        ),
    )
}

fn text(lua: &Lua, key: &str, value: &Value, example: &str) -> mlua::Result<String> {
    match value {
        Value::String(text) => Ok(text.to_str()?.to_owned()),
        other => Err(wrong_kind(lua, key, "text in quotes", other, example)),
    }
}

fn number(lua: &Lua, key: &str, value: &Value, example: &str, zero: &str) -> mlua::Result<f64> {
    let number = match value {
        Value::Integer(number) => *number as f64,
        Value::Number(number) => *number,
        other => return Err(wrong_kind(lua, key, "a number", other, example)),
    };
    if number.is_nan() || number < 0.0 {
        return Err(fail(
            lua,
            format!("game.mode: {key} is {number}, and it cannot be less than 0. {zero}"),
        ));
    }
    Ok(number)
}

fn seconds(lua: &Lua, key: &str, value: &Value, example: &str, zero: &str) -> mlua::Result<f32> {
    // Times that are too long are reported by `Rules::check`.
    number(lua, key, value, example, zero).map(|seconds| seconds as f32)
}

fn whole(lua: &Lua, key: &str, value: &Value, example: &str, zero: &str) -> mlua::Result<u32> {
    let number = number(lua, key, value, example, zero)?;
    if number.fract() != 0.0 || number > MAX_POINTS {
        return Err(fail(
            lua,
            format!(
                "game.mode: {key} is {number}, and it has to be a whole number, at most \
                 1000000. Write it like this: {key} = {example}"
            ),
        ));
    }
    Ok(number as u32)
}

fn team_names(lua: &Lua, value: &Value) -> mlua::Result<Vec<String>> {
    const EXAMPLE: &str = "{ \"Red\", \"Blue\" }";
    let Value::Table(list) = value else {
        return Err(wrong_kind(lua, "teams", "a list of names", value, EXAMPLE));
    };
    let mut names = Vec::new();
    for pair in list.pairs::<Value, Value>() {
        let (key, name) = pair?;
        let in_order = matches!(key, Value::Integer(index) if index == names.len() as i64 + 1);
        let Value::String(name) = &name else {
            return Err(wrong_kind(lua, "teams", "a list of names", &name, EXAMPLE));
        };
        if !in_order {
            return Err(fail(
                lua,
                format!(
                    "game.mode: teams is a plain list of names, with no \"=\" inside it. \
                     Write it like this: teams = {EXAMPLE}"
                ),
            ));
        }
        names.push(name.to_str()?.to_owned());
    }
    if names.is_empty() {
        return Err(fail(
            lua,
            "game.mode: teams is an empty list. Name at least two teams, or leave teams \
             out to make it every player for themselves.",
        ));
    }
    Ok(names)
}
