//! Takes of one sound, so it does not come out the same every time.

use crate::{AudioError, Place, Play, Sound};

/// A few takes of the same sound, played with small changes each time.
///
/// The same footstep or gunshot repeated exactly sounds like a machine.
/// A set picks a different take each time, never the same one twice
/// running, and shifts its pitch and volume a little.
///
/// The shift comes from the `seed` alone, and the take from the seed and
/// the take played before, so the same run of sounds always comes out the
/// same. Any number that changes each time will do as a seed: the tick
/// number, or a weapon's shot seed.
///
/// ```
/// use fossps_audio::{Mixer, SoundSet, starter};
///
/// let mut steps: SoundSet = starter::footsteps();
/// let mut mixer = Mixer::new(48_000);
/// let tick = 1234;
/// mixer.play(steps.at([3.0, 0.0, -2.0], tick));
/// ```
#[derive(Debug, Clone)]
pub struct SoundSet {
    sounds: Vec<Sound>,
    last: Option<usize>,
    /// The most the pitch moves either way, as a share of 1.
    pub pitch_spread: f32,
    /// The most the volume drops, as a share of 1.
    pub volume_spread: f32,
}

impl SoundSet {
    /// A set of takes. One is enough; four or more stop the ear noticing.
    ///
    /// # Errors
    ///
    /// [`AudioError::NoSounds`] when `sounds` is empty.
    pub fn new(sounds: Vec<Sound>) -> Result<Self, AudioError> {
        if sounds.is_empty() {
            return Err(AudioError::NoSounds);
        }
        Ok(Self::of(sounds))
    }

    /// A set the engine made itself. `sounds` must not be empty.
    pub(crate) fn of(sounds: Vec<Sound>) -> Self {
        Self {
            sounds,
            last: None,
            pitch_spread: 0.06,
            volume_spread: 0.1,
        }
    }

    /// Sets how far pitch and volume wander: 0.06 is up to 6% either way.
    #[must_use]
    pub fn spread(mut self, pitch: f32, volume: f32) -> Self {
        self.pitch_spread = pitch;
        self.volume_spread = volume;
        self
    }

    /// The next take, to play from a point in the world.
    pub fn at(&mut self, position: [f32; 3], seed: u32) -> Play {
        self.next(Place::At(position), seed)
    }

    /// The next take, to play equally in both ears.
    pub fn everywhere(&mut self, seed: u32) -> Play {
        self.next(Place::Everywhere, seed)
    }

    fn next(&mut self, place: Place, seed: u32) -> Play {
        let count = self.sounds.len().max(1);
        let mut take = scramble(seed) as usize % count;
        if Some(take) == self.last {
            take = (take + 1) % count;
        }
        self.last = Some(take);

        let pitch = 1.0 + clean(self.pitch_spread) * (roll(seed, 1) * 2.0 - 1.0);
        let volume = 1.0 - clean(self.volume_spread) * roll(seed, 2);
        let mut play = Play::everywhere(self.sounds[take].clone())
            .pitch(pitch)
            .volume(volume);
        play.place = place;
        play
    }
}

/// A spread kept between nothing and a half.
fn clean(spread: f32) -> f32 {
    if spread.is_finite() {
        spread.clamp(0.0, 0.5)
    } else {
        0.0
    }
}

/// Scrambles a number so that close inputs give unrelated outputs.
pub(crate) fn scramble(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}

/// Roll number `which` for `seed`, from 0 up to but not including 1.
pub(crate) fn roll(seed: u32, which: u32) -> f32 {
    let mixed = scramble(scramble(seed) ^ which.wrapping_mul(0x9E37_79B9));
    // 24 bits: every one of them fits a float exactly.
    (mixed >> 8) as f32 * (1.0 / 16_777_216.0)
}
