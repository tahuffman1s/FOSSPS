//! Game-mode rules: teams, rounds, scoring, respawning and spawn points.
//!
//! [`Rules`] are the numbers that make a mode: a ten-minute deathmatch to 50
//! by default, a team deathmatch once it is given teams. A [`Match`] is one
//! of them being played. The server tells it who joined, who left and who
//! killed whom, and calls [`Match::tick`] once per tick; the match keeps the
//! score, counts down the round, brings dead players back, and says what
//! happened as [`Event`]s read with [`Match::next_event`].
//!
//! ```
//! use fossps_mode::{Event, Match, Phase, Rules, Winner};
//!
//! let rules = Rules {
//!     teams: vec!["Red".to_owned(), "Blue".to_owned()],
//!     score_to_win: 1,
//!     warmup: 0.0,
//!     ..Rules::default()
//! };
//! let mut game = Match::new(rules)?;
//! let ann = game.join("Ann");
//! let bo = game.join("Bo");
//! game.tick(1.0 / 60.0);
//! assert_eq!(game.phase(), Phase::Playing);
//!
//! game.kill(Some(ann), bo);
//! let red = game.player(ann).and_then(|player| player.team).unwrap();
//! game.add_team_score(red, 1);
//! assert_eq!(game.phase(), Phase::Over);
//!
//! let mut won = None;
//! while let Some(event) = game.next_event() {
//!     if let Event::RoundEnded { winner } = event {
//!         won = winner;
//!     }
//! }
//! assert_eq!(won, Some(Winner::Team(red)));
//! # Ok::<(), fossps_mode::RulesError>(())
//! ```
//!
//! A round goes through four [`Phase`]s. With nobody on the server the match
//! is `Waiting`. The first player to join starts the `Warmup`: everyone can
//! move and shoot, and nothing counts. Then the round is `Playing` until a
//! side reaches the score to win or time runs out, stays `Over` for the
//! intermission with the final score on show, and the next round warms up.
//!
//! Where a player appears is not decided here, because that needs the map.
//! The match says [`Event::Spawned`], and [`choose_spawn`] picks the place
//! from the map's spawn points and where everyone is standing.
//!
//! Nothing here reads a clock or rolls dice: the same calls in the same
//! order always give the same match.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod rounds;
mod rules;
mod spawn;
#[cfg(test)]
mod tests;

pub use rounds::{Event, Match, Phase, Player, PlayerId, Team, TeamId, Winner};
pub use rules::{MAX_NAME_LENGTH, MAX_SECONDS, MAX_TEAMS, Rules, RulesError};
pub use spawn::{SpawnPoint, Stander, choose_spawn};
