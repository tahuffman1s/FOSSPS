//! When a moving player's feet hit the ground.

/// What a player's feet are doing this tick. Fill it in from the player.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Feet {
    /// How fast the player moves along the ground, in metres per second.
    /// Leave out the up-and-down part of the velocity.
    pub speed: f32,
    /// How fast the player is dropping, in metres per second. Zero or
    /// less when standing or rising.
    pub falling: f32,
    /// Standing on something.
    pub grounded: bool,
    /// Crouched.
    pub crouched: bool,
    /// Sprinting.
    pub sprinting: bool,
}

/// How far apart and how loud footsteps are.
///
/// Steps come by distance covered, not by time, so they always match how
/// fast the player really moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootstepSettings {
    /// Metres between steps when walking.
    pub walk_stride: f32,
    /// Metres between steps when sprinting.
    pub sprint_stride: f32,
    /// Metres between steps when crouched.
    pub crouch_stride: f32,
    /// How loud a walking step is.
    pub walk_volume: f32,
    /// How loud a sprinting step is.
    pub sprint_volume: f32,
    /// How loud a crouched step is. Quiet, so crouching is how to sneak.
    pub crouch_volume: f32,
    /// Below this speed, in metres per second, the player makes no steps.
    pub slowest: f32,
    /// A drop shorter than this many seconds makes no landing sound, so
    /// stairs and kerbs do not thump.
    pub shortest_fall: f32,
    /// Landing at this speed, in metres per second, or faster is as loud
    /// as a landing gets.
    pub hardest_landing: f32,
}

impl Default for FootstepSettings {
    fn default() -> Self {
        Self {
            walk_stride: 2.0,
            sprint_stride: 2.2,
            crouch_stride: 1.3,
            walk_volume: 0.6,
            sprint_volume: 1.0,
            crouch_volume: 0.25,
            slowest: 0.5,
            shortest_fall: 0.15,
            hardest_landing: 10.0,
        }
    }
}

/// A foot hitting the ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Footfall {
    /// One step.
    Step {
        /// How loud it should play, from 0 to 1.
        volume: f32,
        /// The left foot. Feet take turns.
        left: bool,
    },
    /// Both feet, after a jump or a fall.
    Landing {
        /// How loud it should play, from 0 to 1. Harder landings are louder.
        volume: f32,
    },
}

/// Decides when one player's feet hit the ground. Keep one per player.
///
/// It reads no clock and rolls no dice: the same movement always gives
/// the same steps, on a server, a client or in a replay. It is a small
/// `Copy` value, so it can be kept with the player and rolled back.
///
/// ```
/// use fossps_audio::{Feet, Footfall, FootstepSettings, Footsteps};
///
/// let settings = FootstepSettings::default();
/// let mut footsteps = Footsteps::default();
///
/// // Once per tick, from the player's movement:
/// let feet = Feet { speed: 4.5, grounded: true, ..Feet::default() };
/// let mut steps = 0;
/// for _tick in 0..60 {
///     if let Some(Footfall::Step { .. }) = footsteps.step(&settings, &feet, 1.0 / 60.0) {
///         steps += 1;
///     }
/// }
/// // A second of walking covers 4.5 metres. The first step lands a metre
/// // in and the next two metres later.
/// assert_eq!(steps, 2);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Footsteps {
    /// Metres covered since the last step.
    travelled: f32,
    /// Seconds spent off the ground. Zero when standing.
    airborne: f32,
    /// The fastest the player dropped while off the ground.
    fastest_fall: f32,
    /// The next step is the left foot's.
    left_next: bool,
    /// Was moving last tick. When setting off, the first step comes early.
    moving: bool,
}

impl Footsteps {
    /// Advances by one tick of `dt` seconds. Gives back a footfall when a
    /// foot came down during it.
    pub fn step(&mut self, settings: &FootstepSettings, feet: &Feet, dt: f32) -> Option<Footfall> {
        let dt = positive(dt);
        if !feet.grounded {
            self.airborne += dt;
            self.fastest_fall = self.fastest_fall.max(positive(feet.falling));
            return None;
        }

        let fell_for = std::mem::take(&mut self.airborne);
        let fall = std::mem::take(&mut self.fastest_fall);
        if fell_for > positive(settings.shortest_fall) {
            self.travelled = 0.0;
            self.moving = true;
            let hardest = positive(settings.hardest_landing).max(0.1);
            return Some(Footfall::Landing {
                volume: (fall / hardest).clamp(0.3, 1.0),
            });
        }

        let speed = positive(feet.speed);
        let (stride, volume) = if feet.crouched {
            (settings.crouch_stride, settings.crouch_volume)
        } else if feet.sprinting {
            (settings.sprint_stride, settings.sprint_volume)
        } else {
            (settings.walk_stride, settings.walk_volume)
        };
        let stride = positive(stride).max(0.1);
        if speed < positive(settings.slowest) {
            self.moving = false;
            return None;
        }
        if !std::mem::replace(&mut self.moving, true) {
            // Setting off: the first foot lands half a stride in.
            self.travelled = stride * 0.5;
        }

        self.travelled += speed * dt;
        if self.travelled < stride {
            return None;
        }
        // One step a tick at most: a huge tick does not owe a drum roll.
        self.travelled = (self.travelled - stride).min(stride);
        self.left_next = !self.left_next;
        Some(Footfall::Step {
            volume: positive(volume).min(1.0),
            left: !self.left_next,
        })
    }
}

/// `value`, or zero if it is negative or not a number.
fn positive(value: f32) -> f32 {
    if value.is_nan() { 0.0 } else { value.max(0.0) }
}
