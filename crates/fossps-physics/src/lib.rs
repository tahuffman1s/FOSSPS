//! Collision with a level, and the player's movement through it.
//!
//! A [`Level`] holds the triangles of a loaded scene. A [`Player`] is moved
//! through it one tick at a time by [`Player::step`]: walking, sprinting,
//! crouching and jumping, sliding along walls, climbing stairs and ramps.
//!
//! ```no_run
//! use fossps_physics::{Level, MoveInput, MoveSettings, Player};
//!
//! # fn main() -> Result<(), fossps_assets::AssetError> {
//! let scene = fossps_assets::Scene::load("level.glb")?;
//! let level = Level::from_scene(&scene);
//! let settings = MoveSettings::default();
//! let mut player = Player::at([0.0, 1.0, 0.0], &settings);
//!
//! // Once per tick, with what the player is holding down:
//! let input = MoveInput { forward: 1.0, sprint: true, ..MoveInput::default() };
//! player.step(&level, &settings, &input, 1.0 / 60.0);
//! # Ok(())
//! # }
//! ```
//!
//! Directions and sizes are the engine's: +Y is up, -Z is forward, +X is
//! right, and one unit is one metre.
//!
//! Nothing here reads a clock, a random number or anything else outside its
//! arguments: the same player, level, settings and inputs always give the
//! same result. A server and a predicting client can therefore run the same
//! moves and agree, and a client can go back to an old [`Player`] (it is a
//! small `Copy` value) and replay its inputs.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod level;
mod player;
mod settings;
mod slide;
#[cfg(test)]
mod tests;

pub use level::Level;
pub use player::Player;
pub use settings::{MoveInput, MoveSettings};
