//! The interface a game implements to be driven by the engine.

use fossps_input::{ActionSetId, InputFrame};

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

/// A borrowed game is a game, so one can be run and looked at afterwards.
impl<G: Game + ?Sized> Game for &mut G {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        (**self).tick(input, step);
    }

    fn frame(&mut self, frame: &mut Frame) {
        (**self).frame(frame);
    }
}

/// What a game can read and change while presenting one frame.
#[derive(Debug, Default)]
pub struct Frame {
    alpha: f32,
    title: Option<String>,
    close: bool,
    view: Option<View>,
    action_set: Option<ActionSetId>,
}

/// Where the world is seen from: the eye's position, then which way it
/// faces, as [`Frame::look_from`] takes them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub(crate) position: [f32; 3],
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
}

impl Frame {
    /// A frame that is `alpha` of the way from the last tick to the next.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn new(alpha: f32) -> Self {
        Self {
            alpha,
            title: None,
            close: false,
            view: None,
            action_set: None,
        }
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

    /// Draws the world as seen from `position`, in metres, from this frame
    /// until it is called again.
    ///
    /// `yaw` is which way the eye faces, in radians: 0 looks along -Z and it
    /// turns left as the number grows. `pitch` is how far it looks up
    /// (positive) or down. These are the angles the player physics uses. A
    /// game on a server has no window, so nothing happens there.
    pub fn look_from(&mut self, position: [f32; 3], yaw: f32, pitch: f32) {
        self.view = Some(View {
            position,
            yaw,
            pitch,
        });
    }

    /// Makes `set` the live action set from the next tick on: call it with
    /// the menu's set when a menu opens, and with
    /// [`ActionMap::GAMEPLAY`](fossps_input::ActionMap::GAMEPLAY) when it
    /// closes.
    ///
    /// Only the live set's actions hear the player. The set also says what
    /// the mouse is for: one made with
    /// [`MouseMode::Pointer`](fossps_input::MouseMode::Pointer) shows the
    /// cursor and lets it go, and one made with `MouseMode::Look` hides it
    /// and holds it in the window again. The key that was held to open the
    /// menu does nothing in the menu until it is pressed again; see
    /// [`Input::set_active_set`](fossps_input::Input::set_active_set).
    ///
    /// Asking for the set that is already live does nothing, so a game can
    /// call this every frame. A game on a server has no player at the
    /// keyboard, so nothing happens there.
    pub fn use_action_set(&mut self, set: ActionSetId) {
        self.action_set = Some(set);
    }

    /// Closes the window and ends the game after this frame, as if the
    /// player had closed it. A game on a server has no window, so nothing
    /// happens there.
    pub fn close(&mut self) {
        self.close = true;
    }

    /// Whether the game asked to end this frame.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn closing(&self) -> bool {
        self.close
    }

    /// Where the game asked the world to be seen from this frame, if it did.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn view(&self) -> Option<View> {
        self.view
    }

    /// The action set requested this frame, if any.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn action_set(&self) -> Option<ActionSetId> {
        self.action_set
    }

    /// The title requested this frame, if any.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn take_title(&mut self) -> Option<String> {
        self.title.take()
    }
}
