//! Whole matches played through real scripts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use fossps_input::{ActionMap, InputFrame};
use fossps_script::mode::{Phase, TeamId};
use fossps_script::{ENTRY_SCRIPT, PlayerId, ScriptHost};

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
            "fossps-mode-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let folder = Self(folder);
        folder.write(script);
        printed();
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
const SECOND: u32 = 60;

fn run(host: &mut ScriptHost, ticks: u32) {
    let idle = InputFrame::default();
    for _ in 0..ticks {
        host.tick(&idle, STEP);
    }
}

/// Opens `script`, which must set a mode with no warm-up, with Ann and Bo in
/// a round that is being played and nothing printed yet.
fn playing(script: &str) -> (GameFolder, ScriptHost, PlayerId, PlayerId) {
    let game = GameFolder::new(script);
    let mut host = game.open();
    let ann = host.player_joined("Ann");
    let bo = host.player_joined("Bo");
    run(&mut host, 1);
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
    assert_eq!(host.mode().phase(), Phase::Playing);
    printed();
    host.take_spawns().for_each(drop);
    (game, host, ann, bo)
}

/// The message of the error the script just made.
fn error(host: &ScriptHost) -> String {
    host.last_error()
        .unwrap_or_else(|| panic!("the script made no error; it printed {:?}", printed()))
        .message
        .clone()
}

const TEAMS: &str = r#"game.mode { teams = { "Red", "Blue" }, warmup = 0, scoreToWin = 3 }"#;

#[test]
fn one_call_is_a_whole_team_deathmatch() {
    let (_game, mut host, ann, bo) = playing(TEAMS);
    assert_eq!(host.mode().rules().name, "Team Deathmatch");
    for _ in 0..3 {
        host.player_killed(Some(ann), bo);
        run(&mut host, 3 * SECOND);
    }
    let game = host.mode();
    assert_eq!(game.phase(), Phase::Over);
    assert_eq!(game.teams()[0].score, 3);
    assert_eq!(game.player(ann).unwrap().kills, 3);
}

#[test]
fn a_script_without_a_mode_is_a_free_for_all() {
    let game = GameFolder::new(
        r#"
        game.onPlayerJoin(function(player) print(player.name, player.team, #game.teams()) end)
        "#,
    );
    let mut host = game.open();
    host.player_joined("Ann");
    assert_eq!(printed(), ["Ann nil 0"]);
    assert_eq!(host.mode().rules().name, "Deathmatch");
}

#[test]
fn the_script_hears_everything_that_happens_in_order() {
    let game = GameFolder::new(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 1, scoreToWin = 1, intermission = 1 }
        game.onStart(function() print("start") end)
        game.onPlayerJoin(function(player) print("join", player.name, player.team.name) end)
        game.onSpawn(function(player) print("spawn", player.name, player.alive) end)
        game.onRoundStart(function() print("round", game.round.phase(), game.round.timeLeft()) end)
        game.onKill(function(killer, victim)
            print("kill", killer and killer.name, victim.name, victim.alive, victim.deaths)
        end)
        game.onRoundEnd(function(winner) print("end", winner.name, winner.score, game.round.phase()) end)
        game.onPlayerLeave(function(player) print("leave", player.name, player.kills) end)
        "#,
    );
    let mut host = game.open();
    let ann = host.player_joined("Ann");
    let bo = host.player_joined("Bo");
    assert_eq!(
        printed(),
        [
            "start",
            "join Ann Red",
            "spawn Ann true",
            "join Bo Blue",
            "spawn Bo true"
        ]
    );
    assert_eq!(host.take_spawns().collect::<Vec<_>>(), [ann, bo]);

    run(&mut host, SECOND);
    assert_eq!(
        printed(),
        ["round playing 600", "spawn Ann true", "spawn Bo true"]
    );
    assert_eq!(host.take_spawns().collect::<Vec<_>>(), [ann, bo]);

    host.player_killed(Some(ann), bo);
    assert_eq!(printed(), ["kill Ann Bo false 1", "end Red 1 over"]);

    host.player_killed(None, ann);
    assert_eq!(printed(), ["kill nil Ann false 1"]);

    host.player_left(ann);
    assert_eq!(printed(), ["leave Ann 1"]);
    assert!(host.last_error().is_none(), "{:?}", host.last_error());

    run(&mut host, SECOND);
    assert_eq!(printed(), ["spawn Bo true"], "the next warm-up");
}

#[test]
fn a_handler_can_end_the_round_and_the_end_is_told_afterwards() {
    let (_game, mut host, ann, bo) = playing(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 0, killPoints = 0, scoreToWin = 10 }
        game.onKill(function(killer, victim)
            killer.team:addScore(10)
            print("after scoring:", game.round.phase())
        end)
        game.onRoundEnd(function(winner) print(winner.name, "wins with", winner.score) end)
        "#,
    );
    host.player_killed(Some(ann), bo);
    assert_eq!(printed(), ["after scoring: over", "Red wins with 10"]);
}

#[test]
fn scores_only_count_while_a_round_is_played() {
    let game = GameFolder::new(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 1 }
        game.onSpawn(function(player)
            player:addScore(5)
            player.team:addScore(5)
            print(game.round.phase(), player.score, player.team.score)
        end)
        "#,
    );
    let mut host = game.open();
    host.player_joined("Ann");
    run(&mut host, SECOND);
    assert_eq!(printed(), ["warmup 0 0", "playing 5 5"]);
}

#[test]
fn a_player_is_always_the_same_value() {
    let (_game, mut host, ann, bo) = playing(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 0 }
        local kills = {}
        game.onKill(function(killer, victim)
            kills[killer] = (kills[killer] or 0) + 1
            print(kills[killer], killer == game.players()[1], killer == victim,
                killer.team == game.team("red"), killer.team == victim.team,
                tostring(killer), tostring(killer.team))
        end)
        "#,
    );
    host.player_killed(Some(ann), bo);
    run(&mut host, 3 * SECOND);
    host.player_killed(Some(ann), bo);
    assert_eq!(
        printed(),
        [
            "1 true false true false Ann Red",
            "2 true false true false Ann Red"
        ]
    );
}

#[test]
fn teams_list_their_players() {
    let (_game, mut host, _, _) = playing(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 0 }
        game.onPlayerJoin(function()
            for _, team in game.teams() do
                local names = {}
                for _, player in team:players() do table.insert(names, player.name) end
                print(team.name, table.concat(names, " "))
            end
        end)
        "#,
    );
    host.player_joined("Cy");
    assert_eq!(printed(), ["Red Ann Cy", "Blue Bo"]);
}

#[test]
fn the_script_can_end_the_round_itself() {
    let game = GameFolder::new(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 0 }
        game.onTick(function()
            if game.round.phase() == "playing" then game.round.finish(game.team("Blue")) end
        end)
        game.onRoundEnd(function(winner) print(winner and winner.name) end)
        "#,
    );
    let mut host = game.open();
    host.player_joined("Ann");
    run(&mut host, 1);
    assert_eq!(printed(), ["Blue"]);
    assert_eq!(host.mode().phase(), Phase::Over);
}

#[test]
fn a_round_with_no_time_limit_has_endless_time_left() {
    let (_game, mut host, _, _) = playing(
        r#"
        game.mode { warmup = 0, timeLimit = 0 }
        game.onTick(function() print(game.round.timeLeft() == math.huge) end)
        "#,
    );
    run(&mut host, 1);
    assert_eq!(printed(), ["true"]);
}

#[test]
fn saving_the_script_keeps_the_score_and_applies_new_numbers() {
    let (game, mut host, ann, bo) = playing(TEAMS);
    host.player_killed(Some(ann), bo);
    game.write(
        r#"
        game.mode { teams = { "Red", "Blue" }, warmup = 0, scoreToWin = 2, respawnDelay = 1 }
        game.onReload(function() print("reloaded", game.team("Red").score, #game.players()) end)
        game.onRoundEnd(function(winner) print(winner.name, "wins") end)
        "#,
    );
    host.reload().unwrap();
    assert_eq!(printed(), ["reloaded 1 2"]);
    run(&mut host, 3 * SECOND);
    host.player_killed(Some(ann), bo);
    assert_eq!(printed(), ["Red wins"]);
}

#[test]
fn a_save_with_a_mistake_leaves_the_rules_alone() {
    let (game, mut host, _, _) = playing(TEAMS);
    game.write(r#"game.mode { teams = { "Green", "Gold" }, warmup = 0 } oops()"#);
    assert!(host.reload().is_err());
    assert_eq!(host.mode().teams()[0].name, "Red");
    assert_eq!(host.mode().rules().score_to_win, 3);
}

#[test]
fn changing_the_teams_starts_the_match_again() {
    let (game, mut host, ann, bo) = playing(TEAMS);
    host.player_killed(Some(ann), bo);
    game.write(
        r#"
        game.mode { teams = { "Green", "Gold", "Grey" }, warmup = 0 }
        game.onRoundStart(function() print("again", game.players()[2].team.name) end)
        "#,
    );
    host.reload().unwrap();
    assert_eq!(printed(), ["again Gold"]);
    let game = host.mode();
    assert_eq!(game.teams().len(), 3);
    assert_eq!(game.player(ann).unwrap().team, Some(TeamId(0)));
    assert_eq!(game.player(ann).unwrap().kills, 0);
}

#[test]
fn a_player_kept_after_they_left_says_so() {
    let (_game, mut host, ann, _) = playing(
        r#"
        game.mode { warmup = 0 }
        local kept
        game.onPlayerLeave(function(player) kept = player print("bye", player.name) end)
        game.onTick(function() if kept then print(tostring(kept)) print(kept.score) end end)
        "#,
    );
    host.player_left(ann);
    assert_eq!(printed(), ["bye Ann"]);
    run(&mut host, 1);
    assert_eq!(printed(), ["a player who has left"]);
    assert_eq!(
        error(&host),
        "This player has left the match, so player.score cannot be read. Stop keeping a \
         player in game.onPlayerLeave; game.players() lists who is here now."
    );
    assert_eq!(host.last_error().unwrap().line, Some(5));
}

/// Runs `body` inside an `onKill` handler and gives the error it makes.
fn kill_error(body: &str) -> String {
    let script = format!(
        "game.mode {{ teams = {{ \"Red\", \"Blue\" }}, warmup = 0 }}\n\
         game.onKill(function(killer, victim)\n{body}\nend)"
    );
    let (_game, mut host, ann, bo) = playing(&script);
    host.player_killed(Some(ann), bo);
    let message = error(&host);
    assert_eq!(host.last_error().unwrap().line, Some(3), "{message}");
    message
}

#[test]
fn mistakes_with_players_and_teams_say_what_to_write() {
    assert_eq!(
        kill_error("print(killer.scor)"),
        "player.scor does not exist. Did you mean player.score?"
    );
    assert_eq!(
        kill_error("killer:addScor(1)"),
        "player.addScor does not exist. Did you mean player:addScore?"
    );
    assert_eq!(
        kill_error("print(killer.health)"),
        "player.health does not exist. A player has: name, team, score, kills, deaths, alive, \
         and the functions player:addScore."
    );
    assert_eq!(
        kill_error("print(killer.team.colour)"),
        "team.colour does not exist. A team has: name, score, and the functions \
         team:players, team:addScore."
    );
    assert_eq!(
        kill_error("killer.addScore(1)"),
        "player.addScore needs a colon, not a dot, because it acts on one player. \
         Write it like this: player:addScore(1)"
    );
    assert_eq!(
        kill_error("killer.team.players()"),
        "team.players needs a colon, not a dot, because it acts on one team. \
         Write it like this: team:players()"
    );
    assert_eq!(
        kill_error("killer.team:addScore(\"one\")"),
        "team:addScore needs a number of points, but it was given text. \
         Write it like this: team:addScore(1)"
    );
    assert_eq!(
        kill_error("killer:addScore(0.5)"),
        "player:addScore needs a whole number of points between -1000000 and 1000000, but \
         it was given 0.5. Scores are whole numbers: round it first with math.floor."
    );
    assert_eq!(
        kill_error("killer.score = 10"),
        "player.score cannot be set. To change a score, write: player:addScore(1)"
    );
    assert_eq!(
        kill_error("victim.team.name = \"Losers\""),
        "team.name cannot be set. The engine keeps a team's details; to remember something \
         of your own about a team, use a table of your own: notes[team] = \"something\""
    );
    assert_eq!(
        kill_error("game.team(\"Bleu\")"),
        "There is no team called \"Bleu\". Did you mean \"Blue\"?"
    );
    assert_eq!(
        kill_error("game.team(\"Yellow\")"),
        "There is no team called \"Yellow\". The teams are: Red, Blue."
    );
    assert_eq!(
        kill_error("game.round.finish(killer)"),
        "game.round.finish was given a player, but this mode has teams, so a team wins the \
         round. Write it like this: game.round.finish(player.team)"
    );
    assert_eq!(
        kill_error("game.round.finish(\"Red\")"),
        "game.round.finish needs the team that won, or nothing for a draw, but it was given \
         text. Write it like this: game.round.finish(game.team(\"Red\"))"
    );
    assert_eq!(
        kill_error("game.round.stop()"),
        "game.round.stop does not exist. game.round has: phase, timeLeft, finish."
    );
    assert_eq!(
        kill_error("game.mode { scoreToWin = 5 }"),
        "game.mode sets up the whole match, so it cannot be called inside a function that \
         runs later, such as game.onTick. Call it once, near the top of the file."
    );
}

#[test]
fn scripts_cannot_reach_inside_players() {
    let message = kill_error("getmetatable(killer).__index = nil");
    assert!(!message.is_empty());
    let message = kill_error("setmetatable(killer, {})");
    assert!(!message.is_empty());
}

/// The error a script makes while it loads.
fn load_error(script: &str) -> String {
    let game = GameFolder::new(script);
    let host = game.open();
    error(&host)
}

#[test]
fn mistakes_in_the_mode_settings_say_what_to_write() {
    const EXAMPLE: &str =
        "game.mode { name = \"Team Deathmatch\", teams = { \"Red\", \"Blue\" }, scoreToWin = 50 }";
    assert_eq!(
        load_error("game.mode(50)"),
        format!(
            "game.mode needs a list of settings in curly brackets, but it was given a \
             number. Write it like this: {EXAMPLE}"
        )
    );
    assert_eq!(
        load_error("game.mode { scoreTowin = 50 }"),
        "game.mode has no setting called scoreTowin. Did you mean scoreToWin?"
    );
    assert_eq!(
        load_error("game.mode { gravity = 2 }"),
        "game.mode has no setting called gravity. The settings are: name, teams, scoreToWin, \
         killPoints, timeLimit, respawnDelay, warmup, intermission."
    );
    assert_eq!(
        load_error("game.mode { \"Team Deathmatch\" }"),
        format!(
            "Every setting in game.mode needs a name, such as scoreToWin = 50, but one was \
             given without a name. Write it like this: {EXAMPLE}"
        )
    );
    assert_eq!(
        load_error("game.mode { timeLimit = \"ten minutes\" }"),
        "game.mode: timeLimit needs a number, but it was given text. \
         Write it like this: timeLimit = 600"
    );
    assert_eq!(
        load_error("game.mode { timeLimit = -1 }"),
        "game.mode: timeLimit is -1, and it cannot be less than 0. Use 0 for no time limit."
    );
    assert_eq!(
        load_error("game.mode { scoreToWin = 2.5 }"),
        "game.mode: scoreToWin is 2.5, and it has to be a whole number, at most 1000000. \
         Write it like this: scoreToWin = 50"
    );
    assert_eq!(
        load_error("game.mode { name = 5 }"),
        "game.mode: name needs text in quotes, but it was given a number. \
         Write it like this: name = \"Team Deathmatch\""
    );
    assert_eq!(
        load_error("game.mode { teams = \"Red\" }"),
        "game.mode: teams needs a list of names, but it was given text. \
         Write it like this: teams = { \"Red\", \"Blue\" }"
    );
    assert_eq!(
        load_error("game.mode { teams = { 1, 2 } }"),
        "game.mode: teams needs a list of names, but it was given a number. \
         Write it like this: teams = { \"Red\", \"Blue\" }"
    );
    assert_eq!(
        load_error("game.mode { teams = { red = \"Red\" } }"),
        "game.mode: teams is a plain list of names, with no \"=\" inside it. \
         Write it like this: teams = { \"Red\", \"Blue\" }"
    );
    assert_eq!(
        load_error("game.mode { teams = {} }"),
        "game.mode: teams is an empty list. Name at least two teams, or leave teams out to \
         make it every player for themselves."
    );
    assert_eq!(
        load_error("game.mode { teams = { \"Red\" } }"),
        "game.mode cannot use these settings: there is only one team, so it has nobody to \
         play against. List two or more teams, or none to make it every player for themselves."
    );
    assert_eq!(
        load_error("game.mode { teams = { \"Red\", \"RED\" } }"),
        "game.mode cannot use these settings: two teams are called \"RED\". Give each team \
         its own name."
    );
    assert_eq!(
        load_error("game.mode { timeLimit = 1e9 }"),
        "game.mode cannot use these settings: the time limit is 1000000000 seconds. It has \
         to be from 0 to 86400 (one day)."
    );
    assert_eq!(
        load_error("game.mode { scoreToWin = 5 }\n\ngame.mode { scoreToWin = 6 }"),
        "game.mode was already called on line 1. A game has one mode: put all of the \
         settings in that one call."
    );
}

#[test]
fn handlers_that_set_each_other_off_do_not_hang_the_game() {
    let game = GameFolder::new(
        r#"
        game.mode { warmup = 0, intermission = 0, timeLimit = 0 }
        local ends = 0
        game.onRoundEnd(function() ends += 1 end)
        game.onRoundStart(function() game.round.finish() end)
        game.onTick(function() if ends > 0 then print(ends) ends = 0 end end)
        "#,
    );
    let mut host = game.open();
    host.player_joined("Ann");
    run(&mut host, 3);
    // Each round ends as it starts, but only one round starts per tick: the
    // intermission, however short, is a tick long.
    assert_eq!(printed(), ["1", "1", "1"]);
    assert!(host.last_error().is_none());
}

#[test]
fn the_match_goes_on_while_the_script_is_broken() {
    let game = GameFolder::new("this is not luau");
    let mut host = game.open();
    assert!(host.last_error().is_some());
    let ann = host.player_joined("Ann");
    let bo = host.player_joined("Bo");
    run(&mut host, 5 * SECOND);
    host.player_killed(Some(ann), bo);
    assert_eq!(host.mode().phase(), Phase::Playing);
    assert_eq!(host.mode().player(ann).unwrap().score, 1);
    assert_eq!(host.take_spawns().count(), 4);
}

#[test]
fn the_team_deathmatch_example_plays_a_whole_round() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team-deathmatch");
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Info);
    let mut host = ScriptHost::open(&example, ActionMap::default_fps()).unwrap();
    printed();
    let players: Vec<_> = ["Ann", "Bo", "Cy", "Di"]
        .into_iter()
        .map(|name| host.player_joined(name))
        .collect();
    assert_eq!(
        printed(),
        [
            "Ann joined Red.",
            "Bo joined Blue.",
            "Cy joined Red.",
            "Di joined Blue."
        ]
    );
    run(&mut host, 5 * SECOND);
    assert_eq!(printed(), ["The round is on! First team to 50 wins."]);

    let [ann, bo, cy, di] = players[..] else {
        panic!()
    };
    // Ann kills Bo five times in a row: 5 points and a streak bonus.
    for _ in 0..5 {
        host.player_killed(Some(ann), bo);
        run(&mut host, 3 * SECOND);
    }
    assert_eq!(
        printed()[4..],
        ["Ann killed Bo.", "Ann is on a streak of 5! Bonus point."]
    );
    assert_eq!(host.mode().teams()[0].score, 6);

    // A team kill costs a point, and a fall is nobody's kill.
    host.player_killed(Some(ann), cy);
    host.player_killed(None, di);
    assert_eq!(
        printed(),
        ["Ann killed a team-mate. Red loses a point.", "Di died."]
    );
    assert_eq!(host.mode().teams()[0].score, 5);

    host.player_left(cy);
    assert_eq!(printed(), ["Cy left."]);

    // Bo's streak ended each time he died, so 45 more kills by Ann bring 9
    // bonus points: Red reaches 50 before the last of them.
    let mut kills = 0;
    while host.mode().phase() == Phase::Playing {
        host.player_killed(Some(ann), bo);
        run(&mut host, 3 * SECOND);
        kills += 1;
    }
    assert_eq!(kills, 38);
    let lines = printed();
    assert_eq!(
        lines[lines.len() - 3..],
        ["Red wins the round!", "Red: 50", "Blue: 0"]
    );
    assert!(host.last_error().is_none(), "{:?}", host.last_error());
}
