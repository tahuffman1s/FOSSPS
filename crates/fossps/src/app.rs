//! The window and the main loop.

use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fossps_input::{ActionMap, ActionSetId, Input, MouseMode, MouseWheel};
use fossps_render::{Camera, RenderConfig, RenderError, Renderer, Upscaling};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::game::{Frame, Game};
use crate::gamepad::Gamepads;
use crate::pace::{FramePacer, WATCH};
use crate::tick::TickClock;
use crate::translate;

/// Where the eye is before a game says where to look from: as high as a
/// standing player's, in metres.
const STANDING_EYE: f32 = 1.6;

/// A view that moves further than this in one frame, in metres, has jumped
/// rather than moved. Falling at terminal speed at 20 frames a second is
/// under 3 m a frame.
const CUT_METRES: f32 = 5.0;

/// How many pixels of smooth scrolling count as one wheel notch.
const PIXELS_PER_NOTCH: f32 = 40.0;

/// The most wheel notches one scroll event may add up to.
const MOST_NOTCHES_PER_EVENT: f32 = 16.0;

/// How long to leave the window alone after a frame could not be drawn.
///
/// A hidden or minimised window cannot be drawn into, and on some systems
/// finding that out takes a while. Trying only now and then keeps the
/// simulation on time while the window is out of sight.
const PAUSE_AFTER_SKIPPED_FRAME: Duration = Duration::from_millis(250);

/// How the engine should start.
#[derive(Debug, Clone)]
pub struct Config {
    /// Text in the window's title bar.
    pub title: String,
    /// Window width in pixels, before display scaling.
    pub width: u32,
    /// Window height in pixels, before display scaling.
    pub height: u32,
    /// Simulation ticks per second.
    pub tick_rate: u32,
    /// Wait for the display before showing each frame.
    ///
    /// On: no tearing, and the frame rate matches the display. Off: frames are
    /// drawn as fast as possible for the lowest input delay, but this uses more
    /// power and can show a visible split line (tearing).
    ///
    /// ```
    /// let config = fossps::Config {
    ///     vsync: false,
    ///     ..fossps::Config::default()
    /// };
    /// ```
    pub vsync: bool,
    /// The most frames drawn each second. 0 is no limit.
    ///
    /// This matters with `vsync` off, where nothing else holds frames back:
    /// a simple scene can be drawn thousands of times a second, which keeps a
    /// processor core and the graphics card busy for frames nobody sees. With
    /// `vsync` on the display already sets the pace, and this only has an
    /// effect when it is lower than the display's rate.
    ///
    /// A limit from 1 to 29 is treated as 30, so the simulation keeps up.
    ///
    /// ```
    /// let config = fossps::Config {
    ///     vsync: false,
    ///     max_fps: 240,
    ///     ..fossps::Config::default()
    /// };
    /// ```
    pub max_fps: u32,
    /// How much smaller than the window the game is drawn before AMD's
    /// FSR 3.1 makes it full size. Smaller is faster.
    ///
    /// ```
    /// let config = fossps::Config {
    ///     upscaling: fossps::Upscaling::Performance,
    ///     ..fossps::Config::default()
    /// };
    /// ```
    pub upscaling: Upscaling,
    /// The actions the game reads and what they are bound to.
    pub actions: ActionMap,
    /// The level the game is played in. [`level::load`](crate::level::load)
    /// reads the `level.glb` in a game's folder. With `None`, the engine's
    /// stand-in is shown: a chequered floor with a few blocks on it.
    ///
    /// ```no_run
    /// let config = fossps::Config {
    ///     level: fossps::level::load("my-game".as_ref())?,
    ///     ..fossps::Config::default()
    /// };
    /// # Ok::<(), fossps::level::LevelError>(())
    /// ```
    pub level: Option<fossps_assets::Scene>,
}

impl Default for Config {
    /// A 1280x720 window, 60 ticks per second, vsync on, at most 400 frames
    /// per second, FSR upscaling at Quality, the standard FPS controls and
    /// no level.
    fn default() -> Self {
        Self {
            title: "FOSSPS".to_owned(),
            width: 1280,
            height: 720,
            tick_rate: 60,
            vsync: true,
            max_fps: 400,
            upscaling: Upscaling::default(),
            actions: ActionMap::default_fps(),
            level: None,
        }
    }
}

/// Why the engine could not start or keep running.
#[derive(Debug)]
pub enum AppError {
    /// The operating system's event loop failed.
    EventLoop(winit::error::EventLoopError),
    /// The window could not be created.
    Window(winit::error::OsError),
    /// The renderer could not start or stopped working.
    Render(RenderError),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => write!(
                f,
                "Could not start the window system: {error}. \
                 FOSSPS needs a desktop session; on Linux, check that Wayland or X11 is running \
                 and that WAYLAND_DISPLAY or DISPLAY is set."
            ),
            Self::Window(error) => write!(
                f,
                "Could not open the game window: {error}. \
                 Check that your graphics drivers are installed and try a smaller window size."
            ),
            Self::Render(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
            Self::Render(error) => error.source(),
        }
    }
}

/// Opens a window and runs `game` until the window is closed.
///
/// Keyboard, mouse and gamepad all feed `config.actions`, and
/// [`Game::tick`] is called `config.tick_rate` times per second.
///
/// The live action set decides what the mouse is for. While it is one the
/// player looks around in (as the first set, `gameplay`, is), clicking the
/// window captures the mouse; Escape or switching to another window always
/// lets it go. When the game makes a menu's set live with
/// [`Frame::use_action_set`], the cursor is shown and freed, and it is
/// captured again when the game goes back.
///
/// Call this from the main thread.
///
/// # Errors
///
/// Fails if the window system cannot be reached, the window cannot be created,
/// or no graphics card here can draw with Vulkan 1.3. The error's message says
/// what to do about it.
pub fn run(config: Config, game: impl Game) -> Result<(), AppError> {
    start(config, game, false)
}

/// Runs `game` as [`run`] does, as the editor: a ground grid is drawn over
/// the level, and holding the right mouse button captures the mouse to look
/// around with, whatever the live action set does with it.
#[cfg(feature = "editor")]
pub(crate) fn run_editor(config: Config, game: impl Game) -> Result<(), AppError> {
    start(config, game, true)
}

/// Opens the window and runs `game` in it, as the editor if `editing`.
fn start(config: Config, game: impl Game, editing: bool) -> Result<(), AppError> {
    for gap in config.actions.parity_gaps() {
        log::warn!("{gap}");
    }
    let event_loop = EventLoop::new().map_err(AppError::EventLoop)?;
    let mut app = App {
        clock: TickClock::new(config.tick_rate),
        pacer: FramePacer::new(config.max_fps, Instant::now()),
        input: Input::new(config.actions.clone()),
        gamepads: Gamepads::new(),
        config,
        game,
        editing,
        renderer: None,
        window: None,
        occluded: false,
        draw_after: Instant::now(),
        focused: false,
        captured: false,
        scroll: 0.0,
        last_frame: Instant::now(),
        last_view: None,
        ui: fossps_ui::DrawBuffer::default(),
        #[cfg(feature = "editor")]
        panels: None,
        error: None,
    };
    event_loop.run_app(&mut app).map_err(AppError::EventLoop)?;
    app.error.map_or(Ok(()), Err)
}

/// Everything the main loop owns.
struct App<G> {
    config: Config,
    game: G,
    /// Whether this is the editor: see [`run_editor`].
    editing: bool,
    input: Input,
    clock: TickClock,
    /// Holds frames back to the frame rate limit.
    pacer: FramePacer,
    gamepads: Gamepads,
    /// Draws into the window. Declared before it so it is dropped first.
    renderer: Option<Renderer>,
    /// Created once the window system is ready for it.
    window: Option<Rc<Window>>,
    /// Whether the window is completely hidden, so drawing would be wasted.
    occluded: bool,
    /// Drawing is not tried before this time. Pushed back when a frame is skipped.
    draw_after: Instant,
    focused: bool,
    /// Whether the mouse is captured for looking around.
    captured: bool,
    /// Smooth-scroll pixels that have not yet added up to a whole notch.
    scroll: f32,
    last_frame: Instant,
    /// Where the game was last seen from, to notice the view jumping.
    last_view: Option<[f32; 3]>,
    /// The HUD and menus the game asked for this frame. Kept from frame to
    /// frame so that gathering them stops asking for memory.
    ui: fossps_ui::DrawBuffer,
    /// What the editor's panels are made with; `None` in a game.
    #[cfg(feature = "editor")]
    panels: Option<PanelInput>,
    /// A failure from inside the event loop, reported once it exits.
    error: Option<AppError>,
}

/// What the editor's panels are made with, and what tells them about the
/// mouse and keyboard.
#[cfg(feature = "editor")]
struct PanelInput {
    context: egui::Context,
    state: egui_winit::State,
}

/// Whether `event` is one the panels can keep from the camera: a press or
/// the wheel. Letting go always reaches the camera, so nothing stays held.
#[cfg(feature = "editor")]
fn starts_something(event: &WindowEvent) -> bool {
    match event {
        WindowEvent::MouseWheel { .. } => true,
        WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
        WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed,
        _ => false,
    }
}

impl<G: Game> App<G> {
    /// Hides the cursor and holds it in the window so the mouse can look around.
    fn capture_mouse(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        // Locking keeps the cursor still and is what Wayland and macOS offer;
        // confining it to the window is what X11 and Windows offer.
        let held = window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
        if let Err(error) = held {
            log::warn!(
                "Could not capture the mouse ({error}). Looking around still works, \
                 but the cursor may leave the window."
            );
        }
        window.set_cursor_visible(false);
        self.captured = true;
    }

    /// What the mouse is for in the live action set.
    fn mouse_mode(&self) -> MouseMode {
        self.input
            .map()
            .mouse_mode(self.input.active_set())
            .unwrap_or_default()
    }

    /// Makes `set` live and gives the mouse to it: freed for a set that
    /// points, captured for one that looks around.
    fn use_action_set(&mut self, set: ActionSetId) {
        let before = self.input.active_set();
        self.input.set_active_set(set);
        // Asking again for the live set, or for one this map does not have,
        // must not take back a mouse the player freed with Escape.
        if self.input.active_set() == before {
            return;
        }
        match self.mouse_mode() {
            MouseMode::Pointer => self.release_mouse(),
            // The player is already in the game, so no click is asked for.
            // After Escape or a switch of windows, a click still is.
            MouseMode::Look if self.focused && !self.captured => self.capture_mouse(),
            MouseMode::Look => {}
        }
    }

    /// Gives the cursor back to the desktop.
    fn release_mouse(&mut self) {
        if let Some(window) = &self.window {
            // Releasing a grab that was never taken is not an error worth reporting.
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.captured = false;
    }

    /// Tells the editor's panels about `event`, and says whether it was
    /// meant for them and not for the camera: a click or the wheel over a
    /// panel, or a key while a text field is being typed in.
    #[cfg(feature = "editor")]
    fn offer_to_panels(&mut self, event: &WindowEvent) -> bool {
        let (Some(panels), Some(window)) = (&mut self.panels, &self.window) else {
            return false;
        };
        if self.captured {
            // The mouse is looking around, and its hidden cursor must not
            // brush over the panels. They still hear the button let go, and
            // Shift let go or the window left, so nothing stays held there.
            if matches!(
                event,
                WindowEvent::MouseInput { .. }
                    | WindowEvent::ModifiersChanged(_)
                    | WindowEvent::Focused(_)
            ) {
                let _ = panels.state.on_window_event(window, event);
            }
            return false;
        }
        let consumed = panels.state.on_window_event(window, event).consumed;
        if matches!(event, WindowEvent::KeyboardInput { .. }) {
            // A button that was clicked may keep the keyboard's attention;
            // only typing takes the flying keys away.
            panels.context.text_edit_focused()
        } else {
            consumed
        }
    }

    /// Hands the panels made this frame to the renderer, and lets them
    /// change the cursor.
    #[cfg(feature = "editor")]
    fn show_panels(&mut self, mut output: egui::FullOutput) {
        let (Some(panels), Some(renderer)) = (&mut self.panels, &mut self.renderer) else {
            // Nothing will draw these; `egui` asks to be told that its
            // texture changes are thrown away on purpose.
            output.textures_delta.clear();
            return;
        };
        // While the mouse looks around, the cursor stays hidden.
        if !self.captured
            && let Some(window) = &self.window
        {
            panels
                .state
                .handle_platform_output(window, output.platform_output);
        }
        renderer.set_panels(fossps_render::Panels {
            shapes: panels
                .context
                .tessellate(output.shapes, output.pixels_per_point),
            textures: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
        });
    }

    /// Feeds whole wheel notches into the input state.
    fn scroll(&mut self, delta: MouseScrollDelta) {
        self.scroll += match delta {
            MouseScrollDelta::LineDelta(_, lines) => lines,
            // Pixel deltas are f64; a wheel notch needs nowhere near that precision.
            #[allow(clippy::cast_possible_truncation)]
            MouseScrollDelta::PixelDelta(position) => position.y as f32 / PIXELS_PER_NOTCH,
        };
        // A faulty device can report absurd distances. Without a limit, a
        // value too large for subtracting 1 to change would loop forever below.
        self.scroll = if self.scroll.is_finite() {
            self.scroll
                .clamp(-MOST_NOTCHES_PER_EVENT, MOST_NOTCHES_PER_EVENT)
        } else {
            0.0
        };
        while self.scroll >= 1.0 {
            self.input.press(MouseWheel::MouseWheelUp);
            self.scroll -= 1.0;
        }
        while self.scroll <= -1.0 {
            self.input.press(MouseWheel::MouseWheelDown);
            self.scroll += 1.0;
        }
    }

    /// Draws one frame. A renderer failure ends the main loop.
    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if !self.can_draw() {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render() {
            Ok(true) => {}
            Ok(false) => self.draw_after = Instant::now() + PAUSE_AFTER_SKIPPED_FRAME,
            Err(error) => {
                self.error = Some(AppError::Render(error));
                event_loop.exit();
            }
        }
    }

    /// Whether it is worth trying to draw a frame right now.
    fn can_draw(&self) -> bool {
        !self.occluded && Instant::now() >= self.draw_after
    }

    /// Runs the ticks that are due and lets the game prepare the next frame.
    /// Returns whether the game asked to end.
    fn update(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame);
        self.last_frame = now;

        self.gamepads.poll(&mut self.input, self.focused);
        let step = self.clock.step().as_secs_f32();
        for _ in 0..self.clock.advance(elapsed) {
            self.game.tick(self.input.tick(), step);
        }

        let screen = self.window.as_ref().map(|window| {
            let size = window.inner_size();
            [size.width, size.height]
        });
        let mut frame =
            Frame::new(self.clock.alpha()).for_window(screen, std::mem::take(&mut self.ui));
        #[cfg(feature = "editor")]
        if let (Some(panels), Some(window)) = (&mut self.panels, &self.window) {
            let input = panels.state.take_egui_input(window);
            frame = frame.with_panels(panels.context.clone(), input);
        }
        self.game.frame(&mut frame);
        #[cfg(feature = "editor")]
        if let Some(output) = frame.take_panels() {
            self.show_panels(output);
        }
        self.ui = frame.take_ui();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_ui(self.ui.draw());
        }
        if let (Some(view), Some(renderer)) = (frame.view(), &mut self.renderer) {
            let camera = Camera {
                position: view.position,
                yaw: view.yaw,
                pitch: view.pitch,
                ..Camera::default()
            };
            // Nobody walks or falls this far in one frame: it is a respawn
            // or a teleport, and the frames from before it do not belong
            // in the picture.
            let jumped = self.last_view.is_some_and(|last| {
                let moved: f32 = (0..3)
                    .map(|axis| (view.position[axis] - last[axis]).powi(2))
                    .sum();
                moved > CUT_METRES * CUT_METRES
            });
            if jumped {
                renderer.cut_to(camera);
            } else {
                renderer.set_camera(camera);
            }
            self.last_view = Some(view.position);
        }
        if let Some(set) = frame.action_set() {
            self.use_action_set(set);
        }
        // Like every other gamepad event, rumble is for the window in front.
        if let Some(rumble) = frame.rumble_requested()
            && self.focused
        {
            self.gamepads.rumble(rumble);
        }
        if let (Some(title), Some(window)) = (frame.take_title(), &self.window) {
            window.set_title(&title);
        }
        frame.closing()
    }
}

impl<G: Game> ApplicationHandler for App<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Rc::new(window);
                let size = window.inner_size();
                let config = RenderConfig {
                    vsync: self.config.vsync,
                    upscaling: self.config.upscaling,
                };
                let renderer = Renderer::new(Rc::clone(&window), size.width, size.height, config)
                    .and_then(|mut renderer| {
                        // The graphics card has its own copy from here on.
                        match self.config.level.take() {
                            Some(level) => renderer.set_scene(&level)?,
                            None => renderer.set_scene(&crate::standin::scene())?,
                        }
                        renderer.set_grid(self.editing);
                        renderer.set_camera(Camera {
                            position: [0.0, STANDING_EYE, 0.0],
                            ..Camera::default()
                        });
                        Ok(renderer)
                    });
                match renderer {
                    Ok(renderer) => self.renderer = Some(renderer),
                    Err(error) => {
                        self.error = Some(AppError::Render(error));
                        event_loop.exit();
                        return;
                    }
                }
                window.request_redraw();
                #[cfg(feature = "editor")]
                if self.editing {
                    let context = crate::editor::panel_context();
                    // Text is drawn as large as the display's scaling asks.
                    #[allow(clippy::cast_possible_truncation)]
                    let scale = window.scale_factor() as f32;
                    let state = egui_winit::State::new(
                        context.clone(),
                        egui::ViewportId::ROOT,
                        &*window,
                        Some(scale),
                        None,
                        None,
                    );
                    self.panels = Some(PanelInput { context, state });
                }
                self.window = Some(window);
                self.last_frame = Instant::now();
            }
            Err(error) => {
                self.error = Some(AppError::Window(error));
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        #[cfg(feature = "editor")]
        if self.offer_to_panels(&event) && starts_something(&event) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                // A window that was minimised may be back.
                self.draw_after = Instant::now();
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                self.draw_after = Instant::now();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if focused {
                    self.gamepads.resync(&mut self.input);
                } else {
                    self.release_mouse();
                    self.input.release_all();
                    self.gamepads.stop_rumble();
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                // Escape always frees the mouse, whatever the live action
                // set does with it, so a player can never be trapped in the
                // window. It still reaches the action map.
                // Only a new press does: a key that repeats while held
                // would free a mouse the game has just captured again.
                if pressed && !event.repeat && !is_synthetic && code == KeyCode::Escape {
                    self.release_mouse();
                }
                let Some(key) = translate::key(code) else {
                    return;
                };
                if !pressed {
                    self.input.release(key);
                } else if !event.repeat && !is_synthetic {
                    // Synthetic presses describe keys already down when the
                    // window gained focus; the player did not just press them.
                    self.input.press(key);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                // In the editor the right button is held to look around, as
                // in the viewports of other editors, and the cursor comes
                // back when it is let go.
                if self.editing && button == winit::event::MouseButton::Right {
                    if pressed {
                        self.capture_mouse();
                    } else {
                        self.release_mouse();
                    }
                }
                if pressed && !self.captured && self.mouse_mode() == MouseMode::Look {
                    // This click only captures the mouse. Passing it on would
                    // fire a shot the player did not mean. A set that points
                    // wants its clicks.
                    self.capture_mouse();
                    return;
                }
                let Some(button) = translate::mouse_button(button) else {
                    return;
                };
                if pressed {
                    self.input.press(button);
                } else {
                    self.input.release(button);
                }
            }
            WindowEvent::MouseWheel { delta, .. }
                if self.captured || self.mouse_mode() == MouseMode::Pointer =>
            {
                self.scroll(delta);
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (x, y) } = event
            && self.captured
        {
            // Mouse counts are small numbers, so f32 holds them exactly. The
            // window system reports down as positive; FOSSPS uses up.
            #[allow(clippy::cast_possible_truncation)]
            self.input.move_mouse(x as f32, -y as f32);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }
        // The frame rate limit is waited out here, before input is read and
        // the game is asked for a frame, so what is drawn is as fresh as it
        // can be. A hidden window draws nothing and is paced by the ticks.
        if self.can_draw() {
            let now = Instant::now();
            let wait = self.pacer.wait(now);
            if wait > WATCH {
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + wait - WATCH));
                return;
            }
            let due = now + wait;
            while Instant::now() < due {
                std::hint::spin_loop();
            }
            self.pacer.start(Instant::now());
        }
        if self.update() {
            event_loop.exit();
            return;
        }
        // Drawing happens when the window system delivers the redraw. With
        // vsync a frame takes as long as the display allows, and that paces
        // the loop; without it, the frame rate limit above does.
        if self.can_draw()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
        // While nothing is drawn (a hidden or minimised window), sleep until
        // the next tick is due instead of spinning a CPU core.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + self.clock.until_next_tick(),
        ));
    }
}
