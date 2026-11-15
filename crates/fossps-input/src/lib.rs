//! Input for FOSSPS: keyboard, mouse and gamepad feeding one action map.
//!
//! Game code never asks "is the space bar down?". It asks "is `jump` pressed?",
//! and an [`ActionMap`] decides which keys, mouse buttons and gamepad controls
//! drive each action. Every action can be bound on a gamepad as well as on
//! keyboard and mouse.
//!
//! Actions come in sets, such as gameplay and menu, and one set is live at a
//! time: see [`ActionMap::add_set`] and [`Input::set_active_set`].
//!
//! This crate has no platform dependencies. A window layer feeds device events
//! into [`Input`], and once per simulation tick [`Input::tick`] produces an
//! [`InputFrame`]: a plain snapshot of every action that game code, replays and
//! the network layer can all consume.
//!
//! One thing goes the other way: a [`Rumble`] is what a game asks the
//! player's gamepad to do.
//!
//! ```
//! use fossps_input::{ActionMap, Input, Key};
//!
//! let mut map = ActionMap::new();
//! let jump = map.button("jump", ["Space", "GamepadSouth"])?;
//! let movement = map.axis2d("move", ["WASD", "LeftStick"])?;
//!
//! let mut input = Input::new(map);
//! input.press(Key::Space);
//! input.press(Key::W);
//!
//! let frame = input.tick();
//! assert!(frame.just_pressed(jump));
//! assert_eq!(frame.axis2d(movement), [0.0, 1.0]);
//! # Ok::<(), fossps_input::MapError>(())
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod binding;
mod devices;
mod file;
mod frame;
mod input;
mod map;
mod names;
mod rumble;
mod shaping;

pub use binding::{ActionKind, Binding, Button};
pub use devices::{GamepadButton, Key, MouseButton, MouseWheel, Stick, StickAxis};
pub use file::{BindingsError, BindingsProblem, ProblemKind};
pub use frame::{ActionState, InputFrame};
pub use input::{Input, RebindError, Rebound};
pub use map::{ActionId, ActionMap, ActionSetId, Conflict, Device, MapError, MouseMode, ParityGap};
pub use names::UnknownName;
pub use rumble::Rumble;
pub use shaping::Deadzone;
