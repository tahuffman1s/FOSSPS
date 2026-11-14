//! What the player is doing to the UI this frame.

/// What the player is holding this frame, as far as menus care.
///
/// The window layer fills this in from the game's actions, so a menu
/// never knows which key or button was used. Everything is "held right
/// now"; the UI works out for itself what was just pressed, and repeats
/// a held direction.
///
/// ```
/// use fossps_ui::UiInput;
///
/// // The player is pushing the stick down and has just pressed A.
/// let input = UiInput { direction: [0.0, -1.0], confirm: true, ..UiInput::default() };
/// # let _ = input;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UiInput {
    /// The d-pad, arrow keys or left stick: `[right, up]`, each from -1 to 1.
    /// A stick counts once it is pushed more than halfway.
    pub direction: [f32; 2],
    /// Choose the widget the player is on (A or Cross, Enter).
    pub confirm: bool,
    /// Go back one screen (B or Circle, Escape).
    pub back: bool,
    /// Open the tab to the left (left shoulder button, Q).
    pub previous_tab: bool,
    /// Open the tab to the right (right shoulder button, E).
    pub next_tab: bool,
    /// Where the mouse pointer is, in pixels from the top-left corner of
    /// the screen, or `None` when there is no pointer.
    pub pointer: Option<[f32; 2]>,
    /// The main mouse button.
    pub pointer_down: bool,
}

/// A direction the player can move between widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Towards the top of the screen.
    Up,
    /// Towards the bottom of the screen.
    Down,
    /// Towards the left edge of the screen.
    Left,
    /// Towards the right edge of the screen.
    Right,
}

impl Direction {
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Up => 0,
            Self::Down => 1,
            Self::Left => 2,
            Self::Right => 3,
        }
    }
}

/// What the player last used, which decides what is highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// A gamepad or keyboard: the widget the player is on is outlined.
    #[default]
    Focus,
    /// The mouse: the widget under the pointer is highlighted and the
    /// outline is hidden.
    Pointer,
}

/// How far a stick must be pushed to count as a direction.
const STICK_THRESHOLD: f32 = 0.5;
/// How long a direction is held before it starts repeating, in seconds.
pub(crate) const REPEAT_DELAY: f32 = 0.4;
/// How often a held direction repeats after that, in seconds.
pub(crate) const REPEAT_EVERY: f32 = 0.1;

/// Which way `direction` points, if it is pushed far enough to count.
pub(crate) fn held_direction(direction: [f32; 2]) -> Option<Direction> {
    let [right, up] = direction;
    if right.abs() > up.abs() {
        if right > STICK_THRESHOLD {
            Some(Direction::Right)
        } else if right < -STICK_THRESHOLD {
            Some(Direction::Left)
        } else {
            None
        }
    } else if up > STICK_THRESHOLD {
        Some(Direction::Up)
    } else if up < -STICK_THRESHOLD {
        Some(Direction::Down)
    } else {
        None
    }
}

/// Turns a held direction into single steps: one at once, then repeats.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Repeater {
    held: Option<Direction>,
    /// Seconds until the next repeat.
    wait: f32,
}

impl Repeater {
    /// Notes what is held without stepping, for while no menu is open. A
    /// direction still held when a menu opens does nothing until it is
    /// let go, so walking forwards into a pause does not move the menu.
    pub(crate) fn wait_for_release(&mut self, held: Option<Direction>) {
        self.held = held;
        self.wait = f32::INFINITY;
    }

    /// The step to take this frame, if any.
    pub(crate) fn step(&mut self, held: Option<Direction>, dt: f32) -> Option<Direction> {
        if held != self.held {
            self.held = held;
            self.wait = REPEAT_DELAY;
            return held;
        }
        held?;
        self.wait -= dt.max(0.0);
        if self.wait <= 0.0 {
            // One step a frame at most: a frame that took far too long
            // does not leave a queue of steps behind it.
            let next = self.wait + REPEAT_EVERY;
            self.wait = if next > 0.0 { next } else { REPEAT_EVERY };
            return held;
        }
        None
    }
}
