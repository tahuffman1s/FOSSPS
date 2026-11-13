//! Luau scripting for FOSSPS.
//!
//! A game is a folder with a `main.luau` in it. [`ScriptHost`] runs that file
//! in a sandbox, calls the functions it registers on the `game` table, loads
//! the file again whenever it is saved, and turns script mistakes into
//! messages that say where the problem is and what to try.
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
mod error;
mod host;

pub use error::{OpenError, ScriptError};
pub use host::{ENTRY_SCRIPT, ScriptHost};

/// Type definitions for the script API, in the format the Luau language
/// server reads, so editors can autocomplete and check `game`.
pub const TYPE_DEFINITIONS: &str = include_str!("../types/fossps.d.luau");
