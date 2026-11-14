//! How a sound is asked for, and how it reaches the listener's ears.

use std::f32::consts::FRAC_PI_4;

use crate::Sound;

/// Where the player's head is and which way it faces.
///
/// Give the mixer a new one every tick, from the player's eyes and the
/// direction they look.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Listener {
    /// Where the ears are, in metres. Use the player's eye position.
    pub position: [f32; 3],
    /// Which way the head faces, in radians, turning left from -Z as it
    /// grows: the same yaw the player moves with.
    pub yaw: f32,
}

/// Where a sound comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// Not in the world: equally loud in both ears wherever the player
    /// stands. For menus, music, and the player's own gun and feet.
    Everywhere,
    /// A point in the world, in metres. Quieter with distance, and louder
    /// in the ear it is nearer to.
    At([f32; 3]),
}

/// How far a sound carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    /// Inside this many metres the sound is at full volume. Past it, the
    /// volume halves each time the distance doubles.
    pub near: f32,
    /// Past this many metres the sound cannot be heard at all. It fades
    /// out over the last tenth of the way.
    pub far: f32,
}

impl Range {
    /// Footsteps: clear across a room, gone across a street.
    pub const FOOTSTEP: Self = Self {
        near: 1.5,
        far: 30.0,
    };
    /// Gunfire: heard across the whole of a mid-sized map.
    pub const GUNFIRE: Self = Self {
        near: 4.0,
        far: 150.0,
    };
}

impl Default for Range {
    /// Most things: voices, doors, pickups, a magazine hitting the floor.
    fn default() -> Self {
        Self {
            near: 1.0,
            far: 50.0,
        }
    }
}

/// Which sounds give way when too many play at once.
///
/// Among sounds that matter equally, the quietest one gives way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Importance {
    /// The first to go: shell casings, far-off impacts.
    Low,
    /// Most sounds.
    #[default]
    Normal,
    /// Never cut off by anything less important: the player's own gun,
    /// an announcer, an enemy's footsteps.
    High,
}

/// A request to play a sound: what, where and how.
///
/// ```
/// use fossps_audio::{Importance, Play, Range, Sound};
///
/// # fn main() -> Result<(), fossps_audio::AudioError> {
/// # let shot = Sound::from_samples(48_000, 1, vec![0.0; 64])?;
/// let play = Play::at(shot, [12.0, 1.4, -30.0])
///     .range(Range::GUNFIRE)
///     .importance(Importance::High);
/// # let _ = play;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Play {
    /// The sound to play.
    pub sound: Sound,
    /// Where it comes from.
    pub place: Place,
    /// How loud, from 0 (silent) through 1 (as recorded) to 2 at most.
    pub volume: f32,
    /// How fast and high: 1 is as recorded, 2 is twice as fast and an
    /// octave up. Kept between a quarter and four.
    pub pitch: f32,
    /// How far it carries. Only matters for a sound at a place.
    pub range: Range,
    /// Start again from the beginning each time it ends, until stopped.
    pub looping: bool,
    /// Whether it gives way when too many sounds play at once.
    pub importance: Importance,
    /// How long to wait before it starts, in seconds.
    pub delay: f32,
}

impl Play {
    /// Plays `sound` equally in both ears, wherever the player stands.
    #[must_use]
    pub fn everywhere(sound: Sound) -> Self {
        Self {
            sound,
            place: Place::Everywhere,
            volume: 1.0,
            pitch: 1.0,
            range: Range::default(),
            looping: false,
            importance: Importance::Normal,
            delay: 0.0,
        }
    }

    /// Plays `sound` from a point in the world.
    #[must_use]
    pub fn at(sound: Sound, position: [f32; 3]) -> Self {
        Self {
            place: Place::At(position),
            ..Self::everywhere(sound)
        }
    }

    /// Sets how loud it is: 1 is as recorded.
    #[must_use]
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Sets how fast and high it plays: 1 is as recorded.
    #[must_use]
    pub fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    /// Sets how far it carries.
    #[must_use]
    pub fn range(mut self, range: Range) -> Self {
        self.range = range;
        self
    }

    /// Makes it repeat until it is stopped.
    #[must_use]
    pub fn looping(mut self) -> Self {
        self.looping = true;
        self
    }

    /// Sets whether it gives way when too many sounds play at once.
    #[must_use]
    pub fn importance(mut self, importance: Importance) -> Self {
        self.importance = importance;
        self
    }

    /// Makes it start `seconds` from now.
    ///
    /// A weapon that fires partway through a tick says when; passing that
    /// time here keeps automatic fire evenly spaced.
    #[must_use]
    pub fn after(mut self, seconds: f32) -> Self {
        self.delay = seconds;
        self
    }
}

/// How much quieter a sound straight behind the listener is.
const BEHIND_QUIETER: f32 = 0.3;

/// How loud a sound is in each ear, before its own volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Heard {
    pub left: f32,
    pub right: f32,
    /// How dulled it is for being behind the listener, from 0 to 1.
    pub muffle: f32,
}

impl Heard {
    pub(crate) const SILENT: Self = Self {
        left: 0.0,
        right: 0.0,
        muffle: 0.0,
    };

    pub(crate) fn loudest(&self) -> f32 {
        self.left.max(self.right)
    }
}

/// Works out how a sound at `place` reaches `listener`.
pub(crate) fn hear(listener: &Listener, place: Place, range: Range) -> Heard {
    let position = match place {
        Place::Everywhere => {
            return Heard {
                left: 1.0,
                right: 1.0,
                muffle: 0.0,
            };
        }
        Place::At(position) => position,
    };
    let offset = [
        position[0] - listener.position[0],
        position[1] - listener.position[1],
        position[2] - listener.position[2],
    ];
    let distance = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
    let near = if range.near.is_finite() {
        range.near.max(0.01)
    } else {
        0.01
    };
    let far = if range.far.is_nan() {
        near
    } else {
        range.far.max(near)
    };
    if !distance.is_finite() || distance >= far {
        return Heard::SILENT;
    }

    // Half as loud each time the distance doubles, fading to nothing over
    // the last tenth so a sound never switches off at the edge.
    let mut gain = near / distance.max(near);
    let fade_from = far * 0.9;
    if distance > fade_from {
        gain *= (far - distance) / (far - fade_from);
    }

    // Right on top of the listener a sound has no side: without this it
    // would jump from ear to ear as the player walks through it.
    let sided = (distance / near).min(1.0);
    let (toward_right, toward_front) = if distance > 1.0e-6 {
        let yaw = if listener.yaw.is_finite() {
            listener.yaw
        } else {
            0.0
        };
        let (sin, cos) = yaw.sin_cos();
        (
            (offset[0] * cos - offset[2] * sin) / distance,
            (-offset[0] * sin - offset[2] * cos) / distance,
        )
    } else {
        (0.0, 0.0)
    };
    let behind = (-toward_front).clamp(0.0, 1.0) * sided;
    gain *= 1.0 - BEHIND_QUIETER * behind;

    // Equal power: the two ears always add up to the same loudness.
    let angle = (toward_right.clamp(-1.0, 1.0) * sided + 1.0) * FRAC_PI_4;
    Heard {
        left: gain * angle.cos(),
        right: gain * angle.sin(),
        muffle: behind,
    }
}
