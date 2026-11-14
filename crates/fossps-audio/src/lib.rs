//! Sound for FOSSPS: where it comes from, footsteps and gunfire.
//!
//! A [`Sound`] is loaded once and played any number of times. A [`Play`]
//! says how: [everywhere](Play::everywhere) for menus and the player's own
//! gun, or [at a place](Play::at) in the world, where it gets quieter with
//! distance, louder in the nearer ear and duller behind the player. The
//! [`Mixer`] adds up everything that is playing, and [`Speakers`] send
//! that to the sound card.
//!
//! ```no_run
//! use fossps_audio::{Feet, Footfall, FootstepSettings, Footsteps, Listener, Play, Range};
//! use fossps_audio::{Speakers, starter};
//!
//! let mut speakers = Speakers::open_or_silent();
//!
//! // Sounds that come with the engine. Load your own with `Sound::load`.
//! let mut steps = starter::footsteps();
//! let mut gunfire = starter::gunfire();
//! let settings = FootstepSettings::default();
//! let mut footsteps = Footsteps::default();
//!
//! // Once per tick:
//! # let (tick, eyes, yaw, shots_fired) = (0, [0.0, 1.6, 0.0], 0.0, 1);
//! # let (enemy_feet, enemy_muzzle) = ([4.0, 0.0, -9.0], [4.0, 1.4, -9.0]);
//! speakers.set_listener(Listener { position: eyes, yaw });
//!
//! // An enemy running nearby: a step each time a foot comes down.
//! let feet = Feet { speed: 6.5, grounded: true, sprinting: true, ..Feet::default() };
//! if let Some(Footfall::Step { volume, .. }) = footsteps.step(&settings, &feet, 1.0 / 60.0) {
//!     let step = steps.at(enemy_feet, tick);
//!     let volume = step.volume * volume;
//!     speakers.play(step.volume(volume).range(Range::FOOTSTEP));
//! }
//!
//! // Their gun: one sound per shot the weapon reports.
//! for _ in 0..shots_fired {
//!     speakers.play(gunfire.at(enemy_muzzle, tick).range(Range::GUNFIRE));
//! }
//! ```
//!
//! Directions and sizes are the engine's: +Y is up, -Z is forward, +X is
//! right, and one unit is one metre.
//!
//! Everything but [`Speakers`] is plain arithmetic. The mixer fills a
//! buffer and can be tested, or run on a server, with no sound card; once
//! made it never asks for memory. [`Footsteps`] and [`SoundSet`] read no
//! clock and keep no random number generator, so the same movement and
//! the same seeds always give the same sounds.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
mod footsteps;
mod mixer;
mod play;
mod sound;
#[cfg(feature = "device")]
mod speakers;
pub mod starter;
#[cfg(test)]
mod tests;
mod vary;

pub use error::AudioError;
pub use footsteps::{Feet, Footfall, FootstepSettings, Footsteps};
pub use mixer::{Mixer, Playing};
pub use play::{Importance, Listener, Place, Play, Range};
pub use sound::Sound;
#[cfg(feature = "device")]
pub use speakers::Speakers;
pub use vary::SoundSet;
