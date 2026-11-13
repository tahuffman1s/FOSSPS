//! The interface a game implements to be driven by the engine.

use fossps_input::InputFrame;

/// A game the engine can run.
pub trait Game {
    /// Advances the simulation by one fixed step of `step` seconds.
    ///
    /// Called at the configured tick rate no matter how fast frames are drawn.
    /// Everything that affects gameplay belongs here.
    fn tick(&mut self, input: &InputFrame, step: f32);

    /// Called once per displayed frame, after that frame's ticks.
    ///
    /// Use it for presentation only. The default does nothing.
    fn frame(&mut self, frame: &mut Frame) {
        let _ = frame;
    }
}

/// What a game can read and change while presenting one frame.
#[derive(Debug, Default)]
pub struct Frame {
    alpha: f32,
    title: Option<String>,
}

impl Frame {
    /// A frame that is `alpha` of the way from the last tick to the next.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn new(alpha: f32) -> Self {
        Self { alpha, title: None }
    }

    /// How far the frame is between the last tick and the next, from 0 to 1.
    ///
    /// Blend the previous and current simulation states by this much for
    /// smooth motion at any frame rate.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Changes the window title.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    /// The title requested this frame, if any.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn take_title(&mut self) -> Option<String> {
        self.title.take()
    }
}
