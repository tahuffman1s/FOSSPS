//! Luau scripting for FOSSPS.
//!
//! A game is a folder with a `main.luau` in it. [`ScriptHost`] runs that file
//! in a sandbox, calls the functions it registers on the `game` table, loads
//! it again whenever it is saved, and turns script mistakes into messages
//! that say where the problem is and what to try.
//!
//! A game can be split over files: `require("./weapons")` runs `weapons.luau`,
//! the file beside the one that asks, once, and gives back what it returned. Only scripts
//! inside the game's folder can be required, and saving any of them loads
//! the game again.
//!
//! ```lua
//! -- main.luau
//! game.onStart(function()
//!     print("Hello from Luau!")
//! end)
//!
//! game.onTick(function(step)
//!     if game.input.justPressed("jump") then
//!         print("Jump!")
//!     end
//! end)
//! ```
//!
//! The script API is described for editors in `types/fossps.d.luau`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod api;
mod audio;
mod error;
mod host;
mod keep;
mod mode_api;
mod modules;

pub use audio::{AudioCommand, SoundId, SoundRequest};
pub use error::{OpenError, ScriptError};
pub use fossps_mode as mode;
pub use fossps_mode::PlayerId;
pub use host::{ENTRY_SCRIPT, ScriptHost};

/// Type definitions for the script API, in the format the Luau language
/// server reads, so editors can autocomplete and check `game`.
pub const TYPE_DEFINITIONS: &str = include_str!("../types/fossps.d.luau");
