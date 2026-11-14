//! Netcode: commands to the server, snapshots back, and a client that
//! does not wait for either.
//!
//! The server is the only one whose word counts. Each tick a [`Client`]
//! moves its own player straight away ([`Client::predict`]) and sends the
//! [`Command`] it ran. The [`Server`] runs the same command a little later
//! ([`Server::tick`]) and sends back a snapshot of where everyone is
//! ([`Server::write_snapshot`]). When a snapshot disagrees with what the
//! client predicted, the client takes the server's answer and runs its
//! newer commands again on top of it ([`Client::receive`]).
//!
//! ```
//! use fossps_net::{Client, ClientSettings, PlayerInput, Server, ServerSettings};
//! use fossps_physics::{Level, MoveInput, MoveSettings, Player};
//!
//! # fn main() -> Result<(), fossps_net::NetError> {
//! let level = Level::from_triangles(&[]);
//! let rules = MoveSettings::default();
//! let step = 1.0 / 60.0;
//! let spawn = Player::at([0.0, 1.0, 0.0], &rules);
//!
//! let mut server = Server::new(step, ServerSettings::default());
//! let id = server.join(spawn)?;
//! let mut client = Client::new(spawn, step, ClientSettings::default());
//!
//! let (mut to_server, mut to_client) = (Vec::new(), Vec::new());
//! for _ in 0..10 {
//!     // On the player's machine, once per tick:
//!     let input = PlayerInput {
//!         movement: MoveInput { forward: 1.0, ..MoveInput::default() },
//!         ..PlayerInput::default()
//!     };
//!     client.predict(&level, &rules, &input, &mut to_server);
//!
//!     // On the server, once per tick:
//!     server.receive(id, &to_server)?;
//!     server.tick(&level, &rules);
//!     server.write_snapshot(id, &mut to_client)?;
//!
//!     // Back on the player's machine:
//!     client.receive(&level, &rules, &to_client)?;
//! }
//! assert_eq!(client.id(), Some(id));
//! # Ok(())
//! # }
//! ```
//!
//! A player sees the others a little in the past, so when one fires, shoot
//! at what [`Server::seen_by`] gives: everyone else where the shooter had
//! them on screen when pulling the trigger (lag compensation).
//!
//! This crate only makes and reads packets. It opens no socket: how the
//! bytes travel is up to the caller, and packets may be lost, repeated or
//! arrive out of order.
//!
//! Nothing here reads a clock or a random number. The same packets in the
//! same order always give the same result, and a client and a server given
//! the same commands work out the same moves to the last digit.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod client;
mod command;
mod server;
mod snapshot;
#[cfg(test)]
mod tests;
mod wire;

pub use client::{Client, ClientSettings, Received};
pub use command::{Command, PlayerInput};
pub use server::{Server, ServerSettings};
pub use snapshot::{MAX_PLAYERS, PlayerId, RemotePlayer};
pub use wire::{MAX_PACKET_BYTES, NetError};
