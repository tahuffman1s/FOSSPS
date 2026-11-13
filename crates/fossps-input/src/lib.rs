//! Input for FOSSPS: keyboard, mouse and gamepad feeding one action map.
//!
//! Game code never asks "is the space bar down?". It asks "is `jump` pressed?",
//! and an [`ActionMap`] decides which keys, mouse buttons and gamepad controls
//! drive each action. Every action can be bound on a gamepad as well as on
//! keyboard and mouse.
//!
//! This crate has no platform dependencies. A window layer feeds device events
//! into [`Input`], and once per simulation tick [`Input::tick`] produces an
//! [`InputFrame`]: a plain snapshot of every action that game code, replays and
//! the network layer can all consume.
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
mod frame;
mod input;
mod map;
mod names;
mod shaping;

pub use binding::{ActionKind, Binding, Button};
pub use devices::{GamepadButton, Key, MouseButton, MouseWheel, Stick, StickAxis};
pub use frame::{ActionState, InputFrame};
pub use input::Input;
pub use map::{ActionId, ActionMap, Device, MapError, ParityGap};
pub use names::UnknownName;
pub use shaping::Deadzone;
