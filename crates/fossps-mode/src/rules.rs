//! The numbers that make a game mode.

use std::fmt;

/// The most teams a mode can have.
pub const MAX_TEAMS: usize = 8;

/// The longest a mode's or a team's name can be, in characters.
pub const MAX_NAME_LENGTH: usize = 32;

/// The longest any of a mode's times can be: one day, in seconds.
pub const MAX_SECONDS: f32 = 86_400.0;

/// What makes one game mode different from another.
///
/// The default is a free-for-all deathmatch: first player to 50, ten minutes
/// at most. Give it [`teams`](Self::teams) and it is a team deathmatch.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    /// What the mode is called, for server lists and the scoreboard.
    pub name: String,
    /// The teams, by name. Empty means every player for themselves.
    pub teams: Vec<String>,
    /// The score that wins the round at once: a team's score when there are
    /// teams, a player's otherwise. 0 means no score ends the round.
    pub score_to_win: u32,
    /// What a kill is worth. With teams it goes to the killer's team and to
    /// the killer; otherwise to the killer alone. Killing a team-mate or
    /// yourself is worth nothing. 0 means kills do not score, for modes that
    /// score something else.
    pub kill_points: u32,
    /// How long a round lasts, in seconds. When it runs out the highest
    /// score wins. 0 means the round is not timed.
    pub time_limit: f32,
    /// How long a player stays dead before coming back, in seconds.
    pub respawn_delay: f32,
    /// How long players can run around before a round starts counting, in
    /// seconds.
    pub warmup: f32,
    /// How long the final score stays up before the next round, in seconds.
    pub intermission: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            name: "Deathmatch".to_owned(),
            teams: Vec::new(),
            score_to_win: 50,
            kill_points: 1,
            time_limit: 600.0,
            respawn_delay: 3.0,
            warmup: 5.0,
            intermission: 10.0,
        }
    }
}

impl Rules {
    /// Checks that a match can be played with these rules.
    ///
    /// # Errors
    ///
    /// Fails if a name is empty or too long, two teams share a name, there is
    /// one team or more than [`MAX_TEAMS`], or a time is not between 0 and
    /// [`MAX_SECONDS`].
    pub fn check(&self) -> Result<(), RulesError> {
        check_name("the mode's name", &self.name)?;
        if self.teams.len() == 1 {
            return Err(RulesError::OneTeam);
        }
        if self.teams.len() > MAX_TEAMS {
            return Err(RulesError::TooManyTeams {
                count: self.teams.len(),
            });
        }
        for (index, team) in self.teams.iter().enumerate() {
            check_name("a team's name", team)?;
            let repeated = self.teams[..index]
                .iter()
                .any(|earlier| earlier.to_lowercase() == team.to_lowercase());
            if repeated {
                return Err(RulesError::RepeatedTeam { name: team.clone() });
            }
        }
        for (what, seconds) in [
            ("the time limit", self.time_limit),
            ("the respawn delay", self.respawn_delay),
            ("the warm-up", self.warmup),
            ("the intermission", self.intermission),
        ] {
            if !(0.0..=MAX_SECONDS).contains(&seconds) {
                return Err(RulesError::BadTime { what, seconds });
            }
        }
        Ok(())
    }
}

fn check_name(what: &'static str, name: &str) -> Result<(), RulesError> {
    if name.trim().is_empty() {
        Err(RulesError::EmptyName { what })
    } else if name.chars().count() > MAX_NAME_LENGTH {
        Err(RulesError::LongName {
            what,
            name: name.to_owned(),
        })
    } else {
        Ok(())
    }
}

/// Why a set of [`Rules`] cannot be played.
#[derive(Debug, Clone, PartialEq)]
pub enum RulesError {
    /// A name has nothing in it.
    EmptyName {
        /// Which name, such as "a team's name".
        what: &'static str,
    },
    /// A name is longer than [`MAX_NAME_LENGTH`].
    LongName {
        /// Which name, such as "a team's name".
        what: &'static str,
        /// The name.
        name: String,
    },
    /// Two teams have the same name, ignoring capitals.
    RepeatedTeam {
        /// The name used twice.
        name: String,
    },
    /// There is exactly one team, so nobody to play against.
    OneTeam,
    /// There are more than [`MAX_TEAMS`] teams.
    TooManyTeams {
        /// How many there are.
        count: usize,
    },
    /// A time is negative, not a number, or longer than [`MAX_SECONDS`].
    BadTime {
        /// Which time, such as "the time limit".
        what: &'static str,
        /// What it was set to.
        seconds: f32,
    },
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName { what } => {
                write!(f, "{what} is empty. Give it at least one letter.")
            }
            Self::LongName { what, name } => write!(
                f,
                "{what} \"{name}\" is longer than {MAX_NAME_LENGTH} characters. Shorten it."
            ),
            Self::RepeatedTeam { name } => write!(
                f,
                "two teams are called \"{name}\". Give each team its own name."
            ),
            Self::OneTeam => write!(
                f,
                "there is only one team, so it has nobody to play against. List two or \
                 more teams, or none to make it every player for themselves."
            ),
            Self::TooManyTeams { count } => write!(
                f,
                "there are {count} teams, and the most a mode can have is {MAX_TEAMS}."
            ),
            Self::BadTime { what, seconds } => write!(
                f,
                "{what} is {seconds} seconds. It has to be from 0 to {MAX_SECONDS} (one day)."
            ),
        }
    }
}

impl std::error::Error for RulesError {}
