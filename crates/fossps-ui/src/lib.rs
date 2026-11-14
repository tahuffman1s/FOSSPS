//! HUD and menus for FOSSPS, where every menu works with a gamepad.
//!
//! A [`Ui`] holds two things. The HUD is text, bars and boxes pinned to
//! the edges of the screen: ammo, health, a crosshair. Menus are screens
//! of buttons and settings that open on top of each other, and the back
//! button always closes the one on show.
//!
//! Nothing here is keyboard-and-mouse only. The player moves between
//! widgets with the d-pad, stick or arrow keys, and the UI works out
//! which widget each direction leads to, so a menu cannot be built that
//! a gamepad cannot reach. Holding a direction repeats it, left and
//! right change sliders and choices, the shoulder buttons switch tabs,
//! and each screen remembers where the player was. Moving the mouse
//! swaps the outline for a highlight under the pointer.
//!
//! ```
//! use fossps_ui::{Anchor, Ui, UiEvent, UiInput};
//!
//! let mut ui = Ui::default();
//! let ammo = ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
//!
//! let mut resume = None;
//! let pause = ui.add_screen("Paused", |screen| {
//!     resume = Some(screen.button("Resume"));
//!     screen.slider("Volume", 0.0, 100.0, 5.0, 80.0);
//!     screen.button("Quit");
//! })?;
//!
//! // The player pauses, then presses A on the first button.
//! ui.open(pause)?;
//! let pressing = UiInput { confirm: true, ..UiInput::default() };
//! let events = ui.update(&pressing, 1.0 / 60.0, [1920, 1080]);
//! if let [UiEvent::Pressed(button)] = events
//!     && Some(*button) == resume
//! {
//!     ui.close();
//! }
//! assert!(!ui.is_menu_open());
//!
//! // Every frame: new values in, a list of boxes and text out.
//! ui.set_text(ammo, "29 / 90")?;
//! ui.update(&UiInput::default(), 1.0 / 60.0, [1920, 1080]);
//! for draw in ui.draw() {
//!     // hand each box and line of text to the renderer
//!     # let _ = draw;
//! }
//! # Ok::<(), fossps_ui::UiError>(())
//! ```
//!
//! Sizes and offsets are in pixels on a screen 1080 high. On any other
//! screen everything is scaled to match, and the player's own
//! [size setting](Ui::set_ui_scale) is applied on top. The HUD stays
//! inside the middle 90% of the screen, because televisions cut off
//! their edges, and text is never smaller than [`SMALLEST_TEXT`].
//!
//! This crate draws nothing and opens no window. [`Ui::update`] takes a
//! plain [`UiInput`] and [`Ui::draw`] gives back plain boxes and text,
//! so the same menu can be tested without a graphics card. After it is
//! built, a UI does not ask for memory again: not to update, not to
//! draw, and not to change HUD text to something no longer than before.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

mod build;
mod draw;
mod error;
mod focus;
mod input;
mod layout;
mod node;
mod paint;
#[cfg(test)]
mod tests;
mod theme;
mod ui;

pub use build::ScreenBuilder;
pub use draw::{Draw, DrawIter, DrawList};
pub use error::UiError;
pub use input::{Direction, InputMode, UiInput};
pub use node::{Anchor, ScreenId, WidgetId};
pub use theme::{Color, EstimatedText, SMALLEST_TEXT, TextMeasure, Theme};
pub use ui::{Ui, UiEvent, Value};
