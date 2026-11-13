//! FOSSPS: a free, open-source engine for multiplayer FPS games.
//!
//! A game is usually a folder with a Luau script, `main.luau`, run through
//! [`ScriptGame`]. A game can also be written in Rust: implement [`Game`] and
//! hand it to [`run`], which opens a window, draws into it with Vulkan, reads
//! keyboard, mouse and gamepad through one action map, and calls the game at a
//! fixed tick rate.
//!
//! ```no_run
//! use fossps::{Config, Game, InputFrame};
//!
//! struct MyGame;
//!
//! impl Game for MyGame {
//!     fn tick(&mut self, input: &InputFrame, step: f32) {
//!         // Advance the simulation by `step` seconds.
//!     }
//! }
//!
//! fn main() -> Result<(), fossps::AppError> {
//!     fossps::run(Config::default(), MyGame)
//! }
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

#[cfg(feature = "window")]
mod app;
mod game;
#[cfg(feature = "window")]
mod gamepad;
mod script_game;
mod tick;
#[cfg(feature = "window")]
mod translate;

pub use fossps_input as input;
pub use fossps_input::{ActionId, ActionMap, InputFrame};
#[cfg(feature = "window")]
pub use fossps_render as render;

#[cfg(feature = "window")]
pub use app::{AppError, Config, run};
pub use fossps_script as script;
pub use game::{Frame, Game};
pub use script_game::ScriptGame;
pub use tick::TickClock;
