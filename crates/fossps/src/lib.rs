//! FOSSPS: a free, open-source engine for multiplayer FPS games.
//!
//! A game implements [`Game`] and hands it to [`run`], which opens a window,
//! draws into it with Vulkan, reads keyboard, mouse and gamepad through one
//! action map, and calls the game at a fixed tick rate.
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
mod tick;
#[cfg(feature = "window")]
mod translate;

pub use fossps_input as input;
pub use fossps_input::{ActionId, ActionMap, InputFrame};
#[cfg(feature = "window")]
pub use fossps_render as render;

#[cfg(feature = "window")]
pub use app::{AppError, Config, run};
pub use game::{Frame, Game};
pub use tick::TickClock;
