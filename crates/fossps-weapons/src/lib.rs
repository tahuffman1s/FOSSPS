//! Weapons: firing, recoil, reloading, and what the shots hit.
//!
//! [`WeaponSettings`] are the numbers that make a kind of weapon: an
//! assault rifle by default, with a pistol, a shotgun, a rocket launcher
//! and a grenade ready to start from. A [`Weapon`] is one of them in a
//! player's hands. Each tick, [`Weapon::step`] takes what the player is
//! doing and gives back the [`Shot`]s fired; [`hitscan`] finds what a
//! bullet hit, and a [`Projectile`] flies a rocket or grenade until it
//! lands.
//!
//! ```
//! use fossps_physics::Level;
//! use fossps_weapons::{hitscan, shot_seed, Target, Weapon, WeaponInput, WeaponSettings};
//!
//! let level = Level::from_triangles(&[]);
//! let enemy = Target { id: 2, feet: [0.0, 0.0, -20.0], height: 1.8, radius: 0.35 };
//!
//! let rifle = WeaponSettings::default();
//! let mut weapon = Weapon::new(&rifle);
//! let mut shots = Vec::new();
//!
//! // Once per tick, with what the player is holding down:
//! let (player, tick) = (1, 0);
//! let input = WeaponInput { trigger: true, aiming: true, ..WeaponInput::default() };
//! shots.clear();
//! weapon.step(&rifle, &input, shot_seed(player, tick), 1.0 / 60.0, &mut shots);
//!
//! let (eyes, yaw, pitch) = ([0.0, 1.6, 0.0], 0.0, 0.0);
//! for shot in &shots {
//!     let direction = shot.direction(yaw, pitch);
//!     if let Some(hit) = hitscan(&level, &[enemy], eyes, direction, &rifle, Some(player)) {
//!         println!("hit {:?} for {}", hit.struck, hit.damage);
//!     }
//! }
//! assert_eq!(weapon.magazine, 29);
//! ```
//!
//! Directions and sizes are the engine's: +Y is up, -Z is forward, +X is
//! right, and one unit is one metre. Settings give angles in degrees;
//! everything worked out each tick is in radians.
//!
//! Nothing here reads a clock or keeps a random number generator: chance
//! comes from the seed handed to [`Weapon::step`]. The same weapon,
//! settings, input, seed and tick length always give the same result, so
//! a server and a predicting client fire the same bullets, and a client
//! can go back to an old [`Weapon`] and replay its inputs.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod math;
mod projectile;
mod random;
mod settings;
mod target;
#[cfg(test)]
mod tests;
mod weapon;

pub use projectile::{Flight, Projectile, blast_damage};
pub use random::shot_seed;
pub use settings::{Delivery, FireMode, ProjectileSettings, WeaponSettings};
pub use target::{Hit, Struck, Target, Zone, hitscan};
pub use weapon::{Reload, Shot, Weapon, WeaponEvents, WeaponInput};
