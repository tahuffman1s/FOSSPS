//! A game whose rules are a Luau script.

use std::path::Path;

use fossps_input::{ActionMap, InputFrame};
use fossps_script::{OpenError, ScriptHost};

use crate::game::Game;

/// A [`Game`] that runs the `main.luau` in a game folder and loads it again
/// whenever the file is saved.
///
/// ```no_run
/// use fossps::{Config, ScriptGame};
///
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = Config::default();
///     let game = ScriptGame::open("my-game", &config.actions)?;
///     fossps::run(config, game)?;
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct ScriptGame {
    host: ScriptHost,
}

impl ScriptGame {
    /// Opens the game in `folder`. The script reads input through `actions`,
    /// which should be the same actions the engine is started with.
    ///
    /// A mistake inside the script does not fail here: it is logged, and the
    /// game waits for the file to be fixed and saved.
    ///
    /// # Errors
    ///
    /// Fails if `folder` has no `main.luau` or the file cannot be read.
    pub fn open(folder: impl AsRef<Path>, actions: &ActionMap) -> Result<Self, OpenError> {
        Ok(Self {
            host: ScriptHost::open(folder, actions.clone())?,
        })
    }

    /// The script host, for example to read its latest error.
    #[must_use]
    pub fn host(&self) -> &ScriptHost {
        &self.host
    }
}

impl Game for ScriptGame {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.host.poll_reload();
        self.host.tick(input, step);
    }
}
