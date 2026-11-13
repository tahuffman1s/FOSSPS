//! A solid-colour window fill, standing in until the Vulkan renderer exists.
//!
//! Wayland does not show a window until something is drawn into it, so
//! without this there would be nothing to click on. The renderer replaces
//! this module.

use std::num::NonZeroU32;
use std::rc::Rc;

use softbuffer::{Context, Surface};
use winit::window::Window;

/// The fill colour, as `0x00RRGGBB`.
const BACKGROUND: u32 = 0x0018_1C24;

/// Fills the window with one colour.
pub(crate) struct Placeholder {
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Set after the first failure so the log is not flooded.
    failed: bool,
}

impl Placeholder {
    /// Prepares to draw into `window`. Returns `None`, after logging why, if that is not possible.
    pub(crate) fn new(window: &Rc<Window>) -> Option<Self> {
        let surface = Context::new(Rc::clone(window))
            .and_then(|context| Surface::new(&context, Rc::clone(window)));
        match surface {
            Ok(surface) => Some(Self {
                surface,
                failed: false,
            }),
            Err(error) => {
                log::warn!(
                    "Could not draw the window background ({error}). \
                     Input still works, but the window may be invisible."
                );
                None
            }
        }
    }

    /// Fills the whole window. Call when the window asks to be redrawn.
    pub(crate) fn draw(&mut self) {
        let size = self.surface.window().inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            // A minimised window has no pixels to fill.
            return;
        };
        let drawn = self.surface.resize(width, height).and_then(|()| {
            let mut buffer = self.surface.buffer_mut()?;
            buffer.fill(BACKGROUND);
            buffer.present()
        });
        if let Err(error) = drawn
            && !self.failed
        {
            self.failed = true;
            log::warn!("Could not draw the window background ({error}).");
        }
    }
}
