//! The player: where they are, and how one tick of input moves them.

use parry3d::math::Vector;
use parry3d::shape::Capsule;

use crate::level::Level;
use crate::settings::{MoveInput, MoveSettings, positive};
use crate::slide::{SKIN, Slid, slide, unstick};

/// How far below the feet counts as standing on something when landing.
const LANDING_REACH: f32 = 0.02;

/// How far past an edge, and how far above it, to look for the surface
/// the edge belongs to.
const EDGE_PROBE: f32 = 0.03;

/// A ledge lower than this is not worth a step; the plain move handles it.
const LOWEST_STEP: f32 = 0.02;

/// A player's body in the level.
///
/// This is everything movement remembers between ticks. It is small and
/// `Copy`, so it can be sent over the network, compared with what a server
/// says, and kept to replay from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player {
    /// Where the feet are, in metres.
    pub position: [f32; 3],
    /// How fast the player is moving, in metres per second.
    pub velocity: [f32; 3],
    /// Standing on something, as opposed to jumping, falling or sliding
    /// down a slope too steep to stand on.
    pub grounded: bool,
    /// Crouched. Stays true after crouch is let go for as long as there is
    /// no room to stand up.
    pub crouched: bool,
    /// Sprinting during the last tick.
    pub sprinting: bool,
    /// How far above the feet the eyes are. Eases between the standing and
    /// crouched heights rather than snapping.
    pub eye_height: f32,
    /// Which way the ground underfoot faces. Straight up when in the air.
    pub ground_normal: [f32; 3],
    /// Jump was held during the last tick. A jump needs a fresh press.
    pub jump_held: bool,
}

impl Player {
    /// A player standing still with their feet at `position`.
    ///
    /// The first [`step`](Self::step) settles them onto the ground, or
    /// starts them falling if `position` is in the air.
    #[must_use]
    pub fn at(position: [f32; 3], settings: &MoveSettings) -> Self {
        Self {
            position,
            velocity: [0.0; 3],
            grounded: false,
            crouched: false,
            sprinting: false,
            eye_height: settings.eye_height(false),
            ground_normal: [0.0, 1.0, 0.0],
            jump_held: false,
        }
    }

    /// Where the eyes are: the place to put a first-person camera.
    #[must_use]
    pub fn eye_position(&self) -> [f32; 3] {
        let [x, y, z] = self.position;
        [x, y + self.eye_height, z]
    }

    /// Moves the player through `level` for one tick of `dt` seconds.
    ///
    /// Call this at a fixed rate with the same `dt` every time (for example
    /// `1.0 / 60.0`); that is what makes a move repeatable. A `dt` that is
    /// zero, negative or not a number does nothing, and so does a player
    /// whose position or velocity is not a number.
    pub fn step(&mut self, level: &Level, settings: &MoveSettings, input: &MoveInput, dt: f32) {
        let mut feet = Vector::from_array(self.position);
        let mut velocity = Vector::from_array(self.velocity);
        if !(dt.is_finite() && dt > 0.0 && feet.is_finite() && velocity.is_finite()) {
            return;
        }
        let input = input.cleaned();
        let before = *self;

        // Stance first: it decides the shape everything else moves.
        if input.crouch {
            self.crouched = true;
        } else if self.crouched {
            let standing = settings.shape(false);
            let middle = feet + Vector::Y * (settings.height(false) / 2.0);
            self.crouched = level.overlaps(&standing, middle);
        }
        let body = Body {
            level,
            settings,
            shape: settings.shape(self.crouched),
            steep: settings.flattest_wall(),
            dt,
        };
        let to_middle = Vector::Y * (settings.height(self.crouched) / 2.0);
        let mut middle = unstick(level, &body.shape, feet + to_middle);

        if input.jump && !self.jump_held && self.grounded && !self.crouched {
            velocity.y = settings.jump_speed();
            self.grounded = false;
        }
        self.jump_held = input.jump;

        // Where the player wants to go, and how fast.
        let (sin, cos) = (libm::sinf(input.yaw), libm::cosf(input.yaw));
        let wish = Vector::new(
            cos * input.right - sin * input.forward,
            0.0,
            -sin * input.right - cos * input.forward,
        );
        let push = wish.length().min(1.0);
        let wish = wish.try_normalize().unwrap_or(Vector::ZERO);
        self.sprinting = input.sprint
            && !self.crouched
            && input.forward > 0.0
            && input.forward >= input.right.abs();
        let top_speed = positive(if self.crouched {
            settings.crouch_speed
        } else if self.sprinting {
            settings.sprint_speed
        } else {
            settings.walk_speed
        });
        let wish_speed = top_speed * push;

        let mut ground = Vector::from_array(self.ground_normal);
        if self.grounded {
            velocity.y = 0.0;
            velocity = slowed(velocity, settings, dt);
            velocity = sped_up(
                velocity,
                wish,
                wish_speed,
                top_speed,
                settings.ground_acceleration,
                dt,
            );

            // Follow the ground's tilt at full speed, up ramps and down:
            // same compass heading, tipped to lie along the surface.
            let speed = velocity.length();
            let climb = -(velocity.x * ground.x + velocity.z * ground.z) / ground.y.max(0.1);
            let tilted = Vector::new(velocity.x, climb, velocity.z)
                .try_normalize()
                .map_or(Vector::ZERO, |direction| direction * speed);
            let mut moved = slide(level, &body.shape, middle, tilted, dt, body.steep, true);
            if moved.blocked
                && let Some(stepped) = body.step_up(middle, velocity)
                && flat_distance(stepped.position, middle)
                    > flat_distance(moved.position, middle) + 1.0e-4
            {
                moved = stepped;
            }
            middle = moved.position;
            // Back to level velocity, keeping the speed walls left us.
            velocity = Vector::new(moved.velocity.x, 0.0, moved.velocity.z)
                .try_normalize()
                .map_or(Vector::ZERO, |direction| {
                    direction * moved.velocity.length()
                });

            // Stay on the ground down stairs and ramps; otherwise start falling.
            let reach = positive(settings.step_height).max(LANDING_REACH);
            match body.ground_below(middle, reach) {
                Some(found) => {
                    middle.y -= found.drop;
                    ground = found.normal;
                }
                None => self.grounded = false,
            }
        } else {
            let gravity = positive(settings.gravity);
            velocity = sped_up(
                velocity,
                wish,
                wish_speed.min(positive(settings.air_speed)),
                top_speed,
                settings.air_acceleration,
                dt,
            );
            // Half of gravity before the move and half after: a jump then
            // peaks at the height asked for whatever the tick length.
            velocity.y -= gravity * dt / 2.0;
            let moved = slide(level, &body.shape, middle, velocity, dt, body.steep, false);
            middle = moved.position;
            velocity = moved.velocity;
            velocity.y -= gravity * dt / 2.0;
            velocity.y = velocity.y.max(-positive(settings.fall_speed_limit));

            if velocity.y <= 0.0
                && let Some(found) = body.ground_below(middle, LANDING_REACH)
            {
                middle.y -= found.drop;
                velocity.y = 0.0;
                ground = found.normal;
                self.grounded = true;
            }
        }
        if !self.grounded {
            ground = Vector::Y;
        }

        let eyes = settings.eye_height(self.crouched);
        let reach = positive(settings.crouch_rate) * dt;
        self.eye_height = if self.eye_height.is_finite() {
            self.eye_height + (eyes - self.eye_height).clamp(-reach, reach)
        } else {
            eyes
        };

        feet = middle - to_middle;
        if feet.is_finite() && velocity.is_finite() {
            self.position = feet.to_array();
            self.velocity = velocity.to_array();
            self.ground_normal = ground.to_array();
        } else {
            // Settings wild enough to overflow: leave the player as they were.
            *self = before;
        }
    }
}

/// The player's shape and surroundings during one tick.
struct Body<'a> {
    level: &'a Level,
    settings: &'a MoveSettings,
    shape: Capsule,
    /// Surfaces facing less upward than this cannot be stood on.
    steep: f32,
    dt: f32,
}

impl Body<'_> {
    /// Looks for ground to stand on within `reach` below a shape centred
    /// at `middle`: how far down it is, and which way it faces.
    fn ground_below(&self, middle: Vector, reach: f32) -> Option<Ground> {
        let touch = self
            .level
            .sweep(&self.shape, middle, Vector::NEG_Y * reach, SKIN)?;
        let drop = reach * touch.fraction;
        let ground = |normal| Ground {
            drop,
            normal,
            height: touch.point.y,
        };
        if touch.normal.y >= self.steep {
            return Some(ground(touch.normal));
        }
        if touch.normal.y <= 0.0 {
            return None;
        }
        // The round underside is resting on an edge or in a crease, so the
        // touch itself looks steep. What matters is the surface it belongs
        // to: flat just past the edge (a stair, a ledge), or flat right
        // under the feet (the floor at the foot of a bank).
        let standable = |normal: &Vector| normal.y >= self.steep;
        let outward = Vector::new(touch.point.x - middle.x, 0.0, touch.point.z - middle.z)
            .try_normalize()
            .unwrap_or(Vector::ZERO);
        let past_edge = touch.point + outward * EDGE_PROBE + Vector::Y * EDGE_PROBE;
        let feet = middle - Vector::Y * (drop + self.shape.half_height() + self.shape.radius);
        self.level
            .surface_below(past_edge, 2.0 * EDGE_PROBE)
            .filter(standable)
            .or_else(|| {
                self.level
                    .surface_below(feet + Vector::Y * EDGE_PROBE, EDGE_PROBE + 3.0 * SKIN)
                    .filter(standable)
            })
            .map(ground)
    }

    /// Tries the move again from a step higher: up, across, and back down.
    /// Gives the result only if it ends standing on something.
    fn step_up(&self, middle: Vector, velocity: Vector) -> Option<Slid> {
        let step = positive(self.settings.step_height);
        let headroom = self
            .level
            .sweep(&self.shape, middle, Vector::Y * step, SKIN)
            .map_or(1.0, |touch| touch.fraction);
        let rise = step * headroom;
        if rise < LOWEST_STEP {
            return None;
        }
        let raised = middle + Vector::Y * rise;
        // Go far enough to get over the lip even when barely moving: the
        // plain move stopped a gap short of the step's face.
        let speed = velocity.length();
        let time = self.dt.max(3.0 * SKIN / speed.max(1.0e-3));
        let across = slide(
            self.level,
            &self.shape,
            raised,
            velocity,
            time,
            self.steep,
            true,
        );
        let landing = self.ground_below(across.position, rise)?;
        // Only a real step up counts. Landing back on the same floor would
        // just be the blocked move again, pressed closer to the wall.
        let feet = middle.y - self.shape.half_height() - self.shape.radius;
        let climbed = landing.height - feet;
        (LOWEST_STEP..=step + SKIN)
            .contains(&climbed)
            .then_some(Slid {
                position: across.position - Vector::Y * landing.drop,
                ..across
            })
    }
}

/// Ground found under the player.
struct Ground {
    /// How far down the player can move to rest on it.
    drop: f32,
    /// Which way it faces.
    normal: Vector,
    /// How high up the place touched is.
    height: f32,
}

/// `velocity` after ground friction has worked on it for `dt` seconds.
fn slowed(velocity: Vector, settings: &MoveSettings, dt: f32) -> Vector {
    let speed = velocity.length();
    if speed < 1.0e-4 {
        return Vector::ZERO;
    }
    let grip = speed.max(positive(settings.stop_speed));
    let lost = grip * positive(settings.friction) * dt;
    velocity * ((speed - lost).max(0.0) / speed)
}

/// `velocity` pushed toward moving at `wish_speed` along `wish`, Quake
/// style: only the speed along `wish` is counted, so turning stays sharp.
/// `drive` is the speed that sets how hard the push is.
fn sped_up(
    velocity: Vector,
    wish: Vector,
    wish_speed: f32,
    drive: f32,
    acceleration: f32,
    dt: f32,
) -> Vector {
    let missing = wish_speed - velocity.dot(wish);
    if missing <= 0.0 {
        return velocity;
    }
    velocity + wish * (positive(acceleration) * dt * drive).min(missing)
}

/// How far apart two points are, ignoring height.
fn flat_distance(a: Vector, b: Vector) -> f32 {
    let (x, z) = (a.x - b.x, a.z - b.z);
    (x * x + z * z).sqrt()
}
