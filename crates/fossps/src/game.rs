//! The interface a game implements to be driven by the engine.

use fossps_input::{ActionSetId, InputFrame, Rumble};
use fossps_ui::{DrawBuffer, Ui};

use crate::sound_output::SoundAsk;

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

/// The size a frame says the window is when there is no window: on a
/// server and in tests. It is the size HUD and menu sizes are written for.
const NO_WINDOW: [u32; 2] = [1920, 1080];

/// What a game can read and change while presenting one frame.
#[derive(Debug)]
pub struct Frame {
    alpha: f32,
    screen: [u32; 2],
    ui: DrawBuffer,
    title: Option<String>,
    close: bool,
    view: Option<View>,
    action_set: Option<ActionSetId>,
    rumble: Option<Rumble>,
    /// The sounds to start and stop. Empty, and so without memory of its
    /// own, in a frame that asks for none.
    sounds: Vec<SoundAsk>,
    /// The editor's panels: what they are made with, and what was made.
    #[cfg(feature = "editor")]
    panels: Option<PanelFrame>,
}

/// One frame of the editor's panels.
#[cfg(feature = "editor")]
struct PanelFrame {
    context: egui::Context,
    /// What the mouse and keyboard did; taken when the panels are made.
    input: Option<egui::RawInput>,
    /// What to draw, once the panels are made.
    output: Option<egui::FullOutput>,
}

#[cfg(feature = "editor")]
impl Drop for PanelFrame {
    fn drop(&mut self) {
        // Panels nobody took are not drawn; `egui` asks to be told their
        // texture changes are thrown away on purpose.
        if let Some(output) = &mut self.output {
            output.textures_delta.clear();
        }
    }
}

#[cfg(feature = "editor")]
impl std::fmt::Debug for PanelFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanelFrame")
            .field("made", &self.output.is_some())
            .finish_non_exhaustive()
    }
}

/// Where the world is seen from: the eye's position, then which way it
/// faces, as [`Frame::look_from`] takes them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub(crate) position: [f32; 3],
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Frame {
    /// A frame that is `alpha` of the way from the last tick to the next.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn new(alpha: f32) -> Self {
        Self {
            alpha,
            screen: NO_WINDOW,
            ui: DrawBuffer::default(),
            title: None,
            close: false,
            view: None,
            action_set: None,
            rumble: None,
            sounds: Vec::new(),
            #[cfg(feature = "editor")]
            panels: None,
        }
    }

    /// Gives the frame what the editor's panels are made with: `context`,
    /// and what the mouse and keyboard did since the last frame.
    #[cfg(feature = "editor")]
    pub(crate) fn with_panels(mut self, context: egui::Context, input: egui::RawInput) -> Self {
        self.panels = Some(PanelFrame {
            context,
            input: Some(input),
            output: None,
        });
        self
    }

    /// Makes the editor's panels for this frame: `build` is given an area
    /// that covers the window. Without a window there is nothing to make
    /// them with and `build` is not called; called twice in a frame, only
    /// the first call counts.
    #[cfg(feature = "editor")]
    pub(crate) fn panels(&mut self, build: impl FnMut(&mut egui::Ui)) {
        if let Some(panels) = &mut self.panels
            && let Some(input) = panels.input.take()
        {
            panels.output = Some(panels.context.run_ui(input, build));
        }
    }

    /// The panels made this frame, if any were.
    #[cfg(feature = "editor")]
    pub(crate) fn take_panels(&mut self) -> Option<egui::FullOutput> {
        self.panels.as_mut()?.output.take()
    }

    /// How far the frame is between the last tick and the next, from 0 to 1.
    ///
    /// Blend the previous and current simulation states by this much for
    /// smooth motion at any frame rate.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// The size of the window in pixels, `[width, height]`: what a
    /// [`Ui`] is updated with so that it fills the window.
    ///
    /// A game on a server has no window; there this is 1920 by 1080.
    #[must_use]
    pub fn screen_size(&self) -> [u32; 2] {
        self.screen
    }

    /// Draws `ui` over the game this frame: its HUD, and its menu if one
    /// is open.
    ///
    /// Call it every frame the UI should be seen, after
    /// [`Ui::update`]; a frame without the call shows the game alone. Called
    /// with several UIs in one frame, each is drawn on top of the ones
    /// before it. The UI is drawn at the window's full size after
    /// upscaling, so its text stays sharp.
    ///
    /// ```
    /// use fossps::ui::{Anchor, Ui, UiInput};
    /// use fossps::{Frame, Game, InputFrame};
    ///
    /// struct Hud {
    ///     ui: Ui,
    /// }
    ///
    /// impl Hud {
    ///     fn new() -> Self {
    ///         let mut ui = Ui::default();
    ///         ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
    ///         Self { ui }
    ///     }
    /// }
    ///
    /// impl Game for Hud {
    ///     fn tick(&mut self, _input: &InputFrame, _step: f32) {}
    ///
    ///     fn frame(&mut self, frame: &mut Frame) {
    ///         self.ui
    ///             .update(&UiInput::default(), 1.0 / 60.0, frame.screen_size());
    ///         frame.draw_ui(&self.ui);
    ///     }
    /// }
    /// ```
    ///
    /// A new `Ui` guesses how wide text is. In a game with a window, give
    /// it the font text is drawn in, so that centred text is centred
    /// exactly: `ui.set_text_measure(Box::new(fossps::render::Font::bundled()))`.
    ///
    /// A game on a server has no window, so nothing is drawn there.
    pub fn draw_ui(&mut self, ui: &Ui) {
        self.ui.add(ui.draw());
    }

    /// Starts the frame for a window of `screen` pixels, if there is one yet, with `buffer` to
    /// gather the UI in: the last frame's, so its memory is used again.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn for_window(mut self, screen: Option<[u32; 2]>, mut buffer: DrawBuffer) -> Self {
        buffer.clear();
        // A minimised window has no size; a UI still needs one to lay out in.
        self.screen = screen.map_or(NO_WINDOW, |[width, height]| [width.max(1), height.max(1)]);
        self.ui = buffer;
        self
    }

    /// Everything the game asked to be drawn over it this frame.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn take_ui(&mut self) -> DrawBuffer {
        std::mem::take(&mut self.ui)
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

    /// Shakes the gamepad the player is holding: `Rumble::new(strong, weak,
    /// seconds)`.
    ///
    /// It replaces the rumble that is playing, so
    /// [`Rumble::STOP`](fossps_input::Rumble::STOP) ends one early, and it
    /// stops by itself when its time is up, when the window is no longer in
    /// front and when the gamepad is unplugged. Called twice in one frame,
    /// the later call wins.
    ///
    /// Nothing happens when the player is on keyboard and mouse, when the
    /// gamepad has no rumble motors, or on macOS, where rumble is not
    /// supported yet. Never make rumble the only sign of something: show or
    /// play it as well. A game on a server has no gamepad, so nothing happens
    /// there.
    pub fn rumble(&mut self, rumble: Rumble) {
        self.rumble = Some(rumble);
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

    /// The rumble requested this frame, if any.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn rumble_requested(&self) -> Option<Rumble> {
        self.rumble
    }

    /// Starts or stops a sound for the player at this computer. A game on
    /// a server has no speakers, so nothing happens there.
    pub(crate) fn sound(&mut self, ask: SoundAsk) {
        self.sounds.push(ask);
    }

    /// The sounds asked for this frame, in order.
    #[cfg_attr(not(any(feature = "window", test)), allow(dead_code))]
    pub(crate) fn take_sounds(&mut self) -> Vec<SoundAsk> {
        std::mem::take(&mut self.sounds)
    }

    /// The title requested this frame, if any.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    pub(crate) fn take_title(&mut self) -> Option<String> {
        self.title.take()
    }
}

#[cfg(test)]
mod tests {
    use fossps_ui::{Anchor, Draw, UiInput};

    use super::*;

    /// A UI with one line of HUD text, laid out for `frame`.
    fn hud(words: &str, frame: &Frame) -> Ui {
        let mut ui = Ui::default();
        ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, words);
        ui.update(&UiInput::default(), 1.0 / 60.0, frame.screen_size());
        ui
    }

    fn words(buffer: &DrawBuffer) -> Vec<String> {
        buffer
            .draw()
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text { text, .. } => Some(text.to_owned()),
                Draw::Rect { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_frame_gathers_each_ui_in_the_order_it_was_drawn() {
        let mut frame = Frame::new(0.5);
        assert_eq!(frame.screen_size(), [1920, 1080]);
        frame.draw_ui(&hud("30 / 90", &frame));
        frame.draw_ui(&hud("Paused", &frame));
        let drawn = frame.take_ui();
        assert_eq!(words(&drawn), ["30 / 90", "Paused"]);

        // The next frame starts with nothing, in the window's size.
        let mut next = Frame::new(0.5).for_window(Some([1280, 720]), drawn);
        assert_eq!(next.screen_size(), [1280, 720]);
        assert!(next.take_ui().draw().is_empty());
    }

    #[test]
    fn a_window_with_no_size_still_gives_a_ui_room() {
        let minimised = Frame::new(0.0).for_window(Some([0, 0]), DrawBuffer::default());
        assert_eq!(minimised.screen_size(), [1, 1]);
        let not_open_yet = Frame::new(0.0).for_window(None, DrawBuffer::default());
        assert_eq!(not_open_yet.screen_size(), [1920, 1080]);
    }
}
