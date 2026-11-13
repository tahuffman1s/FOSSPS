//! The window and the main loop.

use std::fmt;
use std::rc::Rc;
use std::time::Instant;

use fossps_input::{ActionMap, Input, MouseWheel};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::game::{Frame, Game};
use crate::gamepad::Gamepads;
use crate::placeholder::Placeholder;
use crate::tick::TickClock;
use crate::translate;

/// How many pixels of smooth scrolling count as one wheel notch.
const PIXELS_PER_NOTCH: f32 = 40.0;

/// The most wheel notches one scroll event may add up to.
const MOST_NOTCHES_PER_EVENT: f32 = 16.0;

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
    /// The actions the game reads and what they are bound to.
    pub actions: ActionMap,
}

impl Default for Config {
    /// A 1280x720 window, 60 ticks per second and the standard FPS controls.
    fn default() -> Self {
        Self {
            title: "FOSSPS".to_owned(),
            width: 1280,
            height: 720,
            tick_rate: 60,
            actions: ActionMap::default_fps(),
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
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
        }
    }
}

/// Opens a window and runs `game` until the window is closed.
///
/// Keyboard, mouse and gamepad all feed `config.actions`, and
/// [`Game::tick`] is called `config.tick_rate` times per second. Clicking the
/// window captures the mouse for looking around; Escape or switching to
/// another window lets it go.
///
/// Call this from the main thread.
///
/// # Errors
///
/// Fails if the window system cannot be reached or the window cannot be created.
pub fn run(config: Config, game: impl Game) -> Result<(), AppError> {
    for gap in config.actions.parity_gaps() {
        log::warn!("{gap}");
    }
    let event_loop = EventLoop::new().map_err(AppError::EventLoop)?;
    let mut app = App {
        clock: TickClock::new(config.tick_rate),
        input: Input::new(config.actions.clone()),
        gamepads: Gamepads::new(),
        config,
        game,
        window: None,
        placeholder: None,
        focused: false,
        captured: false,
        scroll: 0.0,
        last_frame: Instant::now(),
        error: None,
    };
    event_loop.run_app(&mut app).map_err(AppError::EventLoop)?;
    app.error.map_or(Ok(()), Err)
}

/// Everything the main loop owns.
struct App<G> {
    config: Config,
    game: G,
    input: Input,
    clock: TickClock,
    gamepads: Gamepads,
    /// Created once the window system is ready for it.
    window: Option<Rc<Window>>,
    /// Fills the window until the renderer exists.
    placeholder: Option<Placeholder>,
    focused: bool,
    /// Whether the mouse is captured for looking around.
    captured: bool,
    /// Smooth-scroll pixels that have not yet added up to a whole notch.
    scroll: f32,
    last_frame: Instant,
    /// A failure from inside the event loop, reported once it exits.
    error: Option<AppError>,
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

    /// Gives the cursor back to the desktop.
    fn release_mouse(&mut self) {
        if let Some(window) = &self.window {
            // Releasing a grab that was never taken is not an error worth reporting.
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.captured = false;
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

    /// Runs the ticks that are due, then presents one frame.
    fn update(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame);
        self.last_frame = now;

        self.gamepads.poll(&mut self.input, self.focused);
        let step = self.clock.step().as_secs_f32();
        for _ in 0..self.clock.advance(elapsed) {
            self.game.tick(self.input.tick(), step);
        }

        let mut frame = Frame::new(self.clock.alpha());
        self.game.frame(&mut frame);
        if let (Some(title), Some(window)) = (frame.take_title(), &self.window) {
            window.set_title(&title);
        }
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
                self.placeholder = Placeholder::new(&window);
                window.request_redraw();
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Some(placeholder) = &mut self.placeholder {
                    placeholder.draw();
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if focused {
                    self.gamepads.resync(&mut self.input);
                } else {
                    self.release_mouse();
                    self.input.release_all();
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
                // Escape always frees the mouse, so a player can never be
                // trapped in the window. It still reaches the action map.
                if pressed && code == KeyCode::Escape {
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
                if pressed && !self.captured {
                    // This click only captures the mouse. Passing it on would
                    // fire a shot the player did not mean.
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
            WindowEvent::MouseWheel { delta, .. } if self.captured => self.scroll(delta),
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
        self.update();
        // Until a renderer paces the loop, sleep until the next tick is due
        // instead of spinning a CPU core.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + self.clock.until_next_tick(),
        ));
    }
}
