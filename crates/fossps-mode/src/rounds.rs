//! One match being played: who is in it, the score, and the round's clock.

use std::collections::VecDeque;

use crate::rules::{Rules, RulesError};

/// How many [`Event`]s wait to be read before the oldest are dropped. A
/// caller that reads them every tick never comes near it.
const MAX_WAITING_EVENTS: usize = 1024;

/// Names one player for as long as the match lasts. A number is never given
/// to a second player, so one kept after its player left cannot point at
/// somebody else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

/// Names one team: its place in [`Rules::teams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TeamId(pub u8);

/// Where a round is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nobody is on the server.
    Waiting,
    /// Players can move and shoot, and nothing counts yet.
    Warmup,
    /// The round is on and scores count.
    Playing,
    /// The round has ended and the final score is on show.
    Over,
}

/// Who won a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    /// A team, in a mode with teams.
    Team(TeamId),
    /// A player, when it is every player for themselves.
    Player(PlayerId),
}

/// A player in the match.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    /// The player's number.
    pub id: PlayerId,
    /// The name they play under.
    pub name: String,
    /// Their team, in a mode with teams.
    pub team: Option<TeamId>,
    /// Their own score this round.
    pub score: i32,
    /// How many players they have killed this round.
    pub kills: u32,
    /// How many times they have died this round.
    pub deaths: u32,
    /// Whether they are in the world, as opposed to dead and waiting.
    pub alive: bool,
    /// How long until they come back, in seconds, while they are dead and
    /// the round is still going.
    pub respawn_in: Option<f32>,
}

/// A team in the match.
#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    /// The team's name.
    pub name: String,
    /// The team's score this round.
    pub score: i32,
}

/// Something that happened in the match, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A player joined.
    Joined(PlayerId),
    /// A player left. They are no longer in the match, so this carries how
    /// they stood when they went.
    Left(Player),
    /// A player died.
    Killed {
        /// Who killed them, if anyone did.
        killer: Option<PlayerId>,
        /// Who died.
        victim: PlayerId,
    },
    /// A player came into the world and needs a place to stand: see
    /// [`choose_spawn`](crate::choose_spawn).
    Spawned(PlayerId),
    /// The warm-up ended and scores now count. Everyone's score is back to 0.
    RoundStarted,
    /// The round ended.
    RoundEnded {
        /// Who won. `None` is a draw.
        winner: Option<Winner>,
    },
}

/// One match being played under a set of [`Rules`].
#[derive(Debug, Clone)]
pub struct Match {
    rules: Rules,
    teams: Vec<Team>,
    /// In the order they joined.
    players: Vec<Player>,
    /// Seconds until each player comes back; lines up with `players`.
    respawns: Vec<Option<f64>>,
    next_id: u32,
    phase: Phase,
    /// Seconds left in the phase. Kept in more digits than a tick's length so
    /// that a 5-second warm-up at 60 ticks a second is 300 ticks exactly.
    time_left: f64,
    round: u32,
    events: VecDeque<Event>,
}

impl Default for Match {
    /// A match under the default rules, with nobody in it.
    fn default() -> Self {
        Self::unchecked(Rules::default())
    }
}

impl Match {
    /// Starts a match with nobody in it.
    ///
    /// # Errors
    ///
    /// Fails if the rules cannot be played: see [`Rules::check`].
    pub fn new(rules: Rules) -> Result<Self, RulesError> {
        rules.check()?;
        Ok(Self::unchecked(rules))
    }

    fn unchecked(rules: Rules) -> Self {
        Self {
            teams: teams_of(&rules),
            rules,
            players: Vec::new(),
            respawns: Vec::new(),
            next_id: 1,
            phase: Phase::Waiting,
            time_left: 0.0,
            round: 0,
            events: VecDeque::new(),
        }
    }

    /// The rules being played.
    #[must_use]
    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// Changes the rules while the match goes on.
    ///
    /// New numbers take effect at once and the score is kept. If the teams
    /// themselves changed, the match starts again from the warm-up with
    /// everyone shared out over the new teams.
    ///
    /// # Errors
    ///
    /// Fails, changing nothing, if the rules cannot be played.
    pub fn set_rules(&mut self, rules: Rules) -> Result<(), RulesError> {
        rules.check()?;
        let same_teams = rules.teams == self.rules.teams;
        self.rules = rules;
        if same_teams {
            // A countdown already running is not made longer than its new length.
            let length = match self.phase {
                Phase::Waiting => 0.0,
                Phase::Warmup => self.rules.warmup,
                Phase::Playing => self.rules.time_limit,
                Phase::Over => self.rules.intermission,
            };
            self.time_left = self.time_left.min(f64::from(length));
            if self.phase == Phase::Playing && self.rules.time_limit > 0.0 && self.time_left <= 0.0
            {
                // The round had no time limit until now: start counting.
                self.time_left = f64::from(self.rules.time_limit);
            }
            self.end_if_won();
        } else {
            self.teams = teams_of(&self.rules);
            for index in 0..self.players.len() {
                self.players[index].team = None;
                self.players[index].team = self.smallest_team();
            }
            self.round = 0;
            if !self.players.is_empty() {
                self.begin_warmup();
            }
        }
        Ok(())
    }

    /// Where the round is in its life.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Seconds until the phase ends by itself: the warm-up, the round's time
    /// limit or the intermission. `None` while waiting for players and in a
    /// round with no time limit.
    #[must_use]
    pub fn time_left(&self) -> Option<f32> {
        match self.phase {
            Phase::Waiting => None,
            Phase::Playing if self.rules.time_limit <= 0.0 => None,
            _ => Some(self.time_left.max(0.0) as f32),
        }
    }

    /// Which round this is, counting from 1. 0 while waiting for players.
    #[must_use]
    pub fn round(&self) -> u32 {
        self.round
    }

    /// The players, in the order they joined.
    #[must_use]
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// One player, if they are still in the match.
    #[must_use]
    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|player| player.id == id)
    }

    /// The teams, in the order the rules list them. Empty when it is every
    /// player for themselves.
    #[must_use]
    pub fn teams(&self) -> &[Team] {
        &self.teams
    }

    /// One team.
    #[must_use]
    pub fn team(&self, id: TeamId) -> Option<&Team> {
        self.teams.get(usize::from(id.0))
    }

    /// The team with this name, ignoring capitals.
    #[must_use]
    pub fn team_named(&self, name: &str) -> Option<TeamId> {
        let name = name.to_lowercase();
        let index = self
            .teams
            .iter()
            .position(|team| team.name.to_lowercase() == name)?;
        u8::try_from(index).ok().map(TeamId)
    }

    /// Who is ahead right now: the team or player with the highest score.
    /// `None` when the top score is shared, or nobody is playing.
    #[must_use]
    pub fn leader(&self) -> Option<Winner> {
        if self.teams.is_empty() {
            sole_highest(self.players.iter().map(|p| (Winner::Player(p.id), p.score)))
        } else {
            sole_highest(
                self.teams
                    .iter()
                    .zip(0u8..)
                    .map(|(team, index)| (Winner::Team(TeamId(index)), team.score)),
            )
        }
    }

    /// Takes the oldest thing that happened and has not been read yet.
    ///
    /// Read until this gives `None` after every call that can change the
    /// match, or at least once a tick.
    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Adds a player. With teams, they go to the one with the fewest players
    /// (then the lowest score, then the first listed).
    ///
    /// The first player in starts the warm-up. Someone who joins during a
    /// warm-up or a round comes into the world at once; someone who joins
    /// while the final score is up waits for the next round.
    pub fn join(&mut self, name: impl Into<String>) -> PlayerId {
        let id = PlayerId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let team = self.smallest_team();
        self.players.push(Player {
            id,
            name: name.into(),
            team,
            score: 0,
            kills: 0,
            deaths: 0,
            alive: false,
            respawn_in: None,
        });
        self.respawns.push(None);
        self.push(Event::Joined(id));
        match self.phase {
            Phase::Waiting => self.begin_warmup(),
            Phase::Warmup | Phase::Playing => self.spawn(self.players.len() - 1),
            Phase::Over => {}
        }
        id
    }

    /// Removes a player. Returns `false` if they were not in the match.
    ///
    /// When the last player leaves, the match goes back to waiting and the
    /// next player to join starts round 1.
    pub fn leave(&mut self, id: PlayerId) -> bool {
        let Some(index) = self.index(id) else {
            return false;
        };
        let player = self.players.remove(index);
        self.respawns.remove(index);
        self.push(Event::Left(player));
        if self.players.is_empty() {
            self.phase = Phase::Waiting;
            self.time_left = 0.0;
            self.round = 0;
            for team in &mut self.teams {
                team.score = 0;
            }
        }
        true
    }

    /// Records that `victim` died, and that `killer` did it if anyone did.
    /// Returns `false`, changing nothing, if the victim is not alive.
    ///
    /// The victim comes back after [`Rules::respawn_delay`]. While a round is
    /// being played, a kill of an enemy scores [`Rules::kill_points`].
    pub fn kill(&mut self, killer: Option<PlayerId>, victim: PlayerId) -> bool {
        let Some(index) = self.index(victim).filter(|&i| self.players[i].alive) else {
            return false;
        };
        // Killing yourself is dying alone.
        let killer = killer
            .filter(|&killer| killer != victim)
            .and_then(|killer| self.index(killer));
        let waits = matches!(self.phase, Phase::Warmup | Phase::Playing);
        let delay = self.rules.respawn_delay;
        let player = &mut self.players[index];
        player.alive = false;
        player.deaths = player.deaths.saturating_add(1);
        player.respawn_in = waits.then_some(delay);
        self.respawns[index] = waits.then_some(f64::from(delay));
        let victim_team = self.players[index].team;
        let killer = killer.map(|killer| {
            let killer = &mut self.players[killer];
            killer.kills = killer.kills.saturating_add(1);
            (killer.id, killer.team)
        });
        self.push(Event::Killed {
            killer: killer.map(|(id, _)| id),
            victim,
        });
        if let Some((id, team)) = killer {
            let points = i32::try_from(self.rules.kill_points).unwrap_or(i32::MAX);
            let enemy = team.is_none() || team != victim_team;
            if enemy && points > 0 {
                self.add_score(id, points);
                if let Some(team) = team {
                    self.add_team_score(team, points);
                }
            }
        }
        true
    }

    /// Adds to a player's own score (or takes away, with a negative number).
    /// Returns `false`, changing nothing, unless a round is being played.
    ///
    /// When it is every player for themselves, reaching
    /// [`Rules::score_to_win`] wins the round.
    pub fn add_score(&mut self, id: PlayerId, points: i32) -> bool {
        if self.phase != Phase::Playing {
            return false;
        }
        let Some(index) = self.index(id) else {
            return false;
        };
        let player = &mut self.players[index];
        player.score = player.score.saturating_add(points);
        self.end_if_won();
        true
    }

    /// Adds to a team's score (or takes away, with a negative number).
    /// Returns `false`, changing nothing, unless a round is being played.
    ///
    /// Reaching [`Rules::score_to_win`] wins the round.
    pub fn add_team_score(&mut self, id: TeamId, points: i32) -> bool {
        if self.phase != Phase::Playing {
            return false;
        }
        let Some(team) = self.teams.get_mut(usize::from(id.0)) else {
            return false;
        };
        team.score = team.score.saturating_add(points);
        self.end_if_won();
        true
    }

    /// Brings a player into the world now, without waiting out the respawn
    /// delay. A player who is alive is sent back to a spawn point. Returns
    /// `false` if they are not in the match, or no warm-up or round is on.
    pub fn respawn(&mut self, id: PlayerId) -> bool {
        if !matches!(self.phase, Phase::Warmup | Phase::Playing) {
            return false;
        }
        let Some(index) = self.index(id) else {
            return false;
        };
        self.spawn(index);
        true
    }

    /// Moves a player to another team. They keep their own score. Returns
    /// `false` if the player or the team is not in the match.
    pub fn set_team(&mut self, id: PlayerId, team: TeamId) -> bool {
        if self.team(team).is_none() {
            return false;
        }
        let Some(index) = self.index(id) else {
            return false;
        };
        self.players[index].team = Some(team);
        true
    }

    /// Ends the round now with this winner, or as a draw with `None`.
    /// Returns `false`, changing nothing, unless a round is being played.
    pub fn finish(&mut self, winner: Option<Winner>) -> bool {
        if self.phase != Phase::Playing {
            return false;
        }
        self.phase = Phase::Over;
        self.time_left = f64::from(self.rules.intermission);
        for (player, respawn) in self.players.iter_mut().zip(&mut self.respawns) {
            player.respawn_in = None;
            *respawn = None;
        }
        self.push(Event::RoundEnded { winner });
        true
    }

    /// Moves the match on by one tick of `step` seconds: counts the phase
    /// down and brings back players whose wait is over.
    pub fn tick(&mut self, step: f32) {
        let step = f64::from(step.max(0.0));
        // A countdown is over when less than a thousandth of a tick is left,
        // so a tick length that is a hair short of 1/100 s still ends a
        // 5-second warm-up on tick 500.
        let done = step * 0.001;
        match self.phase {
            Phase::Waiting => {}
            Phase::Warmup => {
                self.count_down_respawns(step, done);
                self.time_left -= step;
                if self.time_left <= done {
                    self.begin_round();
                }
            }
            Phase::Playing => {
                self.count_down_respawns(step, done);
                if self.rules.time_limit > 0.0 {
                    self.time_left -= step;
                    if self.time_left <= done {
                        self.finish(self.leader());
                    }
                }
            }
            Phase::Over => {
                self.time_left -= step;
                if self.time_left <= done {
                    self.begin_warmup();
                }
            }
        }
    }

    fn index(&self, id: PlayerId) -> Option<usize> {
        self.players.iter().position(|player| player.id == id)
    }

    fn push(&mut self, event: Event) {
        if self.events.len() >= MAX_WAITING_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// The team a new player should go to, if there are teams.
    fn smallest_team(&self) -> Option<TeamId> {
        (0u8..)
            .zip(&self.teams)
            .min_by_key(|&(index, team)| {
                let size = self
                    .players
                    .iter()
                    .filter(|player| player.team == Some(TeamId(index)))
                    .count();
                (size, team.score, index)
            })
            .map(|(index, _)| TeamId(index))
    }

    /// Puts a player in the world.
    fn spawn(&mut self, index: usize) {
        let player = &mut self.players[index];
        player.alive = true;
        player.respawn_in = None;
        self.respawns[index] = None;
        let id = player.id;
        self.push(Event::Spawned(id));
    }

    fn count_down_respawns(&mut self, step: f64, done: f64) {
        for index in 0..self.players.len() {
            let Some(left) = self.respawns[index] else {
                continue;
            };
            let left = left - step;
            if left <= done {
                self.spawn(index);
            } else {
                self.respawns[index] = Some(left);
                self.players[index].respawn_in = Some(left as f32);
            }
        }
    }

    /// Starts the next round's warm-up, or the round itself if the rules
    /// have no warm-up.
    fn begin_warmup(&mut self) {
        self.round = self.round.saturating_add(1);
        if self.rules.warmup <= 0.0 {
            self.begin_round();
            return;
        }
        self.phase = Phase::Warmup;
        self.time_left = f64::from(self.rules.warmup);
        self.clear_scores();
        for index in 0..self.players.len() {
            if !self.players[index].alive {
                self.spawn(index);
            }
        }
    }

    /// Starts counting: scores go back to 0 and everyone starts from a spawn
    /// point.
    fn begin_round(&mut self) {
        self.phase = Phase::Playing;
        self.time_left = f64::from(self.rules.time_limit);
        self.clear_scores();
        self.push(Event::RoundStarted);
        for index in 0..self.players.len() {
            self.spawn(index);
        }
    }

    fn clear_scores(&mut self) {
        for team in &mut self.teams {
            team.score = 0;
        }
        for player in &mut self.players {
            player.score = 0;
            player.kills = 0;
            player.deaths = 0;
        }
    }

    /// Ends the round if a score has reached the score to win.
    fn end_if_won(&mut self) {
        if self.phase != Phase::Playing || self.rules.score_to_win == 0 {
            return;
        }
        let target = i64::from(self.rules.score_to_win);
        let reached = |score: &i32| i64::from(*score) >= target;
        let won = if self.teams.is_empty() {
            self.players.iter().map(|player| &player.score).any(reached)
        } else {
            self.teams.iter().map(|team| &team.score).any(reached)
        };
        if won {
            // Two sides can get there in the same moment, when a script
            // hands out points: the higher score wins, and equal is a draw.
            self.finish(self.leader());
        }
    }
}

fn teams_of(rules: &Rules) -> Vec<Team> {
    rules
        .teams
        .iter()
        .map(|name| Team {
            name: name.clone(),
            score: 0,
        })
        .collect()
}

/// The entry with the highest score, unless that score is shared.
fn sole_highest(scores: impl Iterator<Item = (Winner, i32)>) -> Option<Winner> {
    let mut best: Option<(Winner, i32)> = None;
    let mut shared = false;
    for (who, score) in scores {
        match best {
            Some((_, top)) if score < top => {}
            Some((_, top)) if score == top => shared = true,
            _ => {
                best = Some((who, score));
                shared = false;
            }
        }
    }
    best.filter(|_| !shared).map(|(who, _)| who)
}
