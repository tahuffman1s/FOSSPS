//! What the player holds down, and the numbers that shape movement.

use parry3d::shape::Capsule;

/// What the player is asking for during one tick.
///
/// This is deliberately not tied to keys or sticks: fill it from actions,
/// from a recorded demo or from a network packet alike.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MoveInput {
    /// Forward (1) or back (-1). A stick pushed halfway gives 0.5.
    pub forward: f32,
    /// Right (1) or left (-1).
    pub right: f32,
    /// Which way the player faces, in radians, turning left from -Z as it
    /// grows. Looking up and down does not change where walking goes.
    pub yaw: f32,
    /// Jump is held. Holding it does not jump again on landing, and a
    /// crouched player does not jump.
    pub jump: bool,
    /// Crouch is held.
    pub crouch: bool,
    /// Sprint is held. It only counts while standing and heading forward.
    pub sprint: bool,
}

impl MoveInput {
    /// The input with numbers that are not numbers taken as zero and the
    /// two axes kept inside -1 to 1.
    pub(crate) fn cleaned(self) -> Self {
        let axis = |value: f32| {
            if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        Self {
            forward: axis(self.forward),
            right: axis(self.right),
            yaw: if self.yaw.is_finite() { self.yaw } else { 0.0 },
            ..self
        }
    }
}

/// The numbers that shape how a player moves. Lengths are metres, speeds
/// metres per second.
///
/// The defaults suit a modern military shooter. Change single fields and
/// keep the rest:
///
/// ```
/// let floaty = fossps_physics::MoveSettings {
///     gravity: 8.0,
///     ..Default::default()
/// };
/// # assert_eq!(floaty.walk_speed, 4.5);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveSettings {
    /// Half the player's width.
    pub radius: f32,
    /// Feet to top of head when standing.
    pub standing_height: f32,
    /// Feet to top of head when crouched.
    pub crouched_height: f32,
    /// Feet to eyes when standing.
    pub standing_eye_height: f32,
    /// Feet to eyes when crouched.
    pub crouched_eye_height: f32,
    /// How fast the eyes move between the two heights.
    pub crouch_rate: f32,
    /// Top speed on foot.
    pub walk_speed: f32,
    /// Top speed while sprinting.
    pub sprint_speed: f32,
    /// Top speed while crouched.
    pub crouch_speed: f32,
    /// How high a jump lifts the feet.
    pub jump_height: f32,
    /// How fast falling speeds up, in metres per second each second.
    pub gravity: f32,
    /// The fastest a player can fall.
    pub fall_speed_limit: f32,
    /// How quickly the player reaches top speed on the ground. Higher is snappier.
    pub ground_acceleration: f32,
    /// How quickly the player stops on the ground when nothing is held.
    pub friction: f32,
    /// Below this speed friction acts as if the player moved this fast, so
    /// stopping ends crisply instead of creeping.
    pub stop_speed: f32,
    /// How quickly the player can change direction in the air.
    pub air_acceleration: f32,
    /// The most speed steering in the air can add in a new direction.
    pub air_speed: f32,
    /// The tallest ledge the player walks up without jumping, such as a stair.
    pub step_height: f32,
    /// The steepest ground the player can stand on, in degrees from flat.
    /// Anything steeper is slid down.
    pub steepest_slope: f32,
}

impl Default for MoveSettings {
    fn default() -> Self {
        Self {
            radius: 0.35,
            standing_height: 1.8,
            crouched_height: 1.1,
            standing_eye_height: 1.6,
            crouched_eye_height: 1.0,
            crouch_rate: 4.0,
            walk_speed: 4.5,
            sprint_speed: 6.5,
            crouch_speed: 2.0,
            jump_height: 1.1,
            gravity: 20.0,
            fall_speed_limit: 50.0,
            ground_acceleration: 10.0,
            friction: 6.0,
            stop_speed: 1.5,
            air_acceleration: 10.0,
            air_speed: 0.75,
            step_height: 0.4,
            steepest_slope: 45.0,
        }
    }
}

impl MoveSettings {
    /// Feet to top of head in the given stance.
    pub(crate) fn height(&self, crouched: bool) -> f32 {
        let height = if crouched {
            self.crouched_height
        } else {
            self.standing_height
        };
        // Never shorter than a ball of the player's width.
        positive(height).max(2.0 * self.radius())
    }

    /// Feet to eyes in the given stance.
    pub(crate) fn eye_height(&self, crouched: bool) -> f32 {
        positive(if crouched {
            self.crouched_eye_height
        } else {
            self.standing_eye_height
        })
    }

    /// The player's collision shape in the given stance, centred on its middle.
    pub(crate) fn shape(&self, crouched: bool) -> Capsule {
        let radius = self.radius();
        Capsule::new_y(self.height(crouched) / 2.0 - radius, radius)
    }

    pub(crate) fn radius(&self) -> f32 {
        positive(self.radius).max(0.01)
    }

    /// How far up a surface must face (the y of its direction) to be stood on.
    pub(crate) fn flattest_wall(&self) -> f32 {
        let degrees = if self.steepest_slope.is_finite() {
            self.steepest_slope.clamp(0.0, 89.0)
        } else {
            45.0
        };
        // A hair of slack, so a ramp built at exactly the limit is walkable.
        libm::cosf(degrees.to_radians()) - 1.0e-3
    }

    /// The upward speed that lifts a jump to `jump_height`.
    pub(crate) fn jump_speed(&self) -> f32 {
        (2.0 * positive(self.gravity) * positive(self.jump_height)).sqrt()
    }
}

/// `value`, or zero if it is negative or not a number.
pub(crate) fn positive(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
