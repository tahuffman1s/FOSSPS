//! Mixes every playing sound into one stereo stream.

use std::f32::consts::TAU;

use crate::play::{Heard, hear};
use crate::{Importance, Listener, Place, Play, Range, Sound};

/// How long a sound takes to fade when it is stopped or changes volume.
/// Short enough not to be noticed, long enough not to click.
const FADE_SECONDS: f32 = 0.005;

/// Sounds behind the listener lose what is above this pitch, in hertz.
const BEHIND_CUTOFF: f32 = 4_000.0;

/// The loudest a single sound may be asked to play.
const LOUDEST: f32 = 2.0;

/// Above this the mix is squeezed instead of cut off flat.
const SQUEEZE_FROM: f32 = 0.8;

/// A sound that was started, for stopping or moving it later.
///
/// It is safe to keep after the sound has ended: stopping or moving a
/// sound that is over does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Playing(pub(crate) u64);

/// Mixes every playing sound into one stereo stream.
///
/// It plays nothing by itself: [`mix`](Self::mix) fills a buffer, and
/// [`Speakers`](crate::Speakers) sends that to the sound card. That makes
/// it easy to test and to run where there is no sound card at all.
///
/// After it is made, a mixer never asks for memory.
///
/// ```
/// use fossps_audio::{Listener, Mixer, Play, Sound};
///
/// # fn main() -> Result<(), fossps_audio::AudioError> {
/// let beep = Sound::from_samples(48_000, 1, vec![0.5; 480])?;
/// let mut mixer = Mixer::new(48_000);
/// mixer.set_listener(Listener { position: [0.0, 1.6, 0.0], yaw: 0.0 });
///
/// // Two metres to the player's right.
/// mixer.play(Play::at(beep, [2.0, 1.6, 0.0]));
///
/// let mut buffer = [0.0; 512];
/// mixer.mix(&mut buffer);
/// let (left, right) = (buffer[100], buffer[101]);
/// assert!(right > left);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Mixer {
    rate: u32,
    voices: Box<[Voice]>,
    listener: Listener,
    volume: f32,
    volume_now: f32,
    /// How much a gain may change from one sample to the next.
    slew: f32,
    /// How quickly the dulling filter follows the sound.
    dulling: f32,
    /// The fade-out of sounds that were cut off to make room, left then
    /// right, waiting to be added to the next mix.
    tail: Box<[f32]>,
    tail_frames: usize,
    next_id: u64,
    started: u64,
    /// Finished sounds, kept for another thread to let go of.
    retired: Vec<Sound>,
    keep_retired: bool,
}

impl Mixer {
    /// How many sounds a mixer plays at once unless told otherwise.
    pub const VOICES: usize = 64;

    /// A mixer whose output is `rate` samples a second, playing up to
    /// [`VOICES`](Self::VOICES) sounds at once.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        Self::with_voices(rate, Self::VOICES)
    }

    /// A mixer that plays up to `voices` sounds at once.
    ///
    /// When more are asked for, the least important and quietest give way.
    #[must_use]
    pub fn with_voices(rate: u32, voices: usize) -> Self {
        let rate = rate.clamp(4_000, 384_000);
        let fade_frames = (FADE_SECONDS * rate as f32).ceil();
        let tail_frames = (fade_frames * LOUDEST) as usize + 2;
        let voices = voices.max(1);
        Self {
            rate,
            voices: (0..voices).map(|_| Voice::default()).collect(),
            listener: Listener::default(),
            volume: 1.0,
            volume_now: 1.0,
            slew: 1.0 / fade_frames,
            dulling: 1.0 - (-TAU * BEHIND_CUTOFF / rate as f32).exp(),
            tail: vec![0.0; tail_frames * 2].into_boxed_slice(),
            tail_frames: 0,
            next_id: 1,
            started: 0,
            retired: Vec::with_capacity(voices * 2),
            keep_retired: false,
        }
    }

    /// How many samples a second the mixer produces.
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Starts a sound.
    ///
    /// A sound too far away to hear is not started, and when every voice
    /// is busy with something more important the new sound is skipped.
    /// Either way the answer can be kept and used as normal.
    pub fn play(&mut self, play: Play) -> Playing {
        let id = self.next_id;
        self.next_id += 1;
        self.play_as(Playing(id), play);
        Playing(id)
    }

    /// Fades a sound out and ends it.
    pub fn stop(&mut self, playing: Playing) {
        if let Some(voice) = self.voice_mut(playing) {
            voice.stopping = true;
        }
    }

    /// Fades out and ends every sound.
    pub fn stop_all(&mut self) {
        for voice in &mut self.voices {
            voice.stopping = true;
        }
    }

    /// Moves a playing sound, for one that follows something: a rocket in
    /// flight, a looping engine.
    pub fn move_to(&mut self, playing: Playing, position: [f32; 3]) {
        if let Some(voice) = self.voice_mut(playing) {
            voice.place = Place::At(position);
        }
    }

    /// Whether a sound is still going.
    #[must_use]
    pub fn is_playing(&self, playing: Playing) -> bool {
        self.voices
            .iter()
            .any(|voice| voice.id == playing.0 && voice.sound.is_some() && !voice.stopping)
    }

    /// How many sounds are playing now.
    #[must_use]
    pub fn playing(&self) -> usize {
        self.voices
            .iter()
            .filter(|voice| voice.sound.is_some())
            .count()
    }

    /// Says where the player's head is. Call it every tick.
    pub fn set_listener(&mut self, listener: Listener) {
        self.listener = listener;
    }

    /// Sets the volume of everything, from 0 (silent) to 1 (full).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    /// Fills `out` with the next stretch of sound: left sample, right
    /// sample, left, right, each between -1 and 1.
    ///
    /// The result does not depend on how the stream is cut into buffers:
    /// one call for 1024 samples gives the same numbers as four for 256.
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let whole = out.len() - out.len() % 2;
        let out = &mut out[..whole];

        let carried = self.tail_frames.min(out.len() / 2) * 2;
        for (sample, tail) in out.iter_mut().zip(&self.tail[..carried]) {
            *sample += tail;
        }
        let pending = self.tail_frames * 2;
        self.tail.copy_within(carried..pending, 0);
        self.tail[pending - carried..pending].fill(0.0);
        self.tail_frames -= carried / 2;

        for voice in &mut self.voices {
            if voice.sound.is_none() {
                continue;
            }
            let heard = hear(&self.listener, voice.place, voice.range);
            voice.loudness = heard.loudest() * voice.volume;
            if voice.render(out, heard, self.slew, self.dulling)
                && let Some(sound) = voice.sound.take()
            {
                retire(&mut self.retired, self.keep_retired, sound);
            }
        }

        for frame in out.as_chunks_mut::<2>().0 {
            self.volume_now = approach(self.volume_now, self.volume, self.slew);
            frame[0] = squeeze(frame[0] * self.volume_now);
            frame[1] = squeeze(frame[1] * self.volume_now);
        }
    }

    /// Starts a sound under a handle chosen by the caller.
    pub(crate) fn play_as(&mut self, playing: Playing, play: Play) {
        let volume = if play.volume.is_finite() {
            play.volume.clamp(0.0, LOUDEST)
        } else {
            1.0
        };
        let pitch = if play.pitch.is_finite() {
            play.pitch.clamp(0.25, 4.0)
        } else {
            1.0
        };
        let delay = if play.delay.is_finite() {
            play.delay.clamp(0.0, 60.0)
        } else {
            0.0
        };
        let loudness = hear(&self.listener, play.place, play.range).loudest() * volume;
        let out_of_earshot = loudness <= 0.0 && !play.looping && play.place != Place::Everywhere;
        let Some(slot) = (!out_of_earshot)
            .then(|| self.make_room(play.importance, loudness))
            .flatten()
        else {
            retire(&mut self.retired, self.keep_retired, play.sound);
            return;
        };

        self.started += 1;
        let step = f64::from(play.sound.rate()) / f64::from(self.rate) * f64::from(pitch);
        let voice = &mut self.voices[slot];
        let before = voice.sound.take();
        *voice = Voice {
            id: playing.0,
            sound: Some(play.sound),
            cursor: 0.0,
            step,
            wait: (delay * self.rate as f32) as u32,
            place: play.place,
            range: play.range,
            volume,
            looping: play.looping,
            importance: play.importance,
            gain: [0.0; 2],
            muffle: 0.0,
            low: 0.0,
            fresh: true,
            stopping: false,
            loudness,
            order: self.started,
        };
        if let Some(sound) = before {
            retire(&mut self.retired, self.keep_retired, sound);
        }
    }

    /// Finds a voice for a new sound, cutting one off if that is fair.
    fn make_room(&mut self, importance: Importance, loudness: f32) -> Option<usize> {
        if let Some(free) = self.voices.iter().position(|voice| voice.sound.is_none()) {
            return Some(free);
        }
        // The one to give way: least important, then quietest, then oldest.
        // A sound already fading out counts for nothing.
        let rank = |voice: &Voice| {
            if voice.stopping {
                (Importance::Low, 0.0, voice.order)
            } else {
                (voice.importance, voice.loudness, voice.order)
            }
        };
        let (slot, (least, quietest, _)) = self
            .voices
            .iter()
            .map(rank)
            .enumerate()
            .min_by(|(_, a), (_, b)| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)))?;
        if least > importance || (least == importance && quietest > loudness) {
            return None;
        }

        // Fade the old sound out into the tail, so cutting it does not click.
        let voice = &mut self.voices[slot];
        voice.stopping = true;
        voice.render(&mut self.tail, Heard::SILENT, self.slew, self.dulling);
        self.tail_frames = self.tail.len() / 2;
        if let Some(sound) = voice.sound.take() {
            retire(&mut self.retired, self.keep_retired, sound);
        }
        Some(slot)
    }

    fn voice_mut(&mut self, playing: Playing) -> Option<&mut Voice> {
        self.voices
            .iter_mut()
            .find(|voice| voice.id == playing.0 && voice.sound.is_some())
    }

    /// Keeps finished sounds instead of letting go of them, so the thread
    /// that mixes never gives memory back to the system.
    #[cfg(feature = "device")]
    pub(crate) fn keep_retired(&mut self) {
        self.keep_retired = true;
    }

    /// Hands over finished sounds until `take` gives one back.
    #[cfg(feature = "device")]
    pub(crate) fn hand_over_retired(&mut self, mut take: impl FnMut(Sound) -> Result<(), Sound>) {
        while let Some(sound) = self.retired.pop() {
            if let Err(sound) = take(sound) {
                self.retired.push(sound);
                break;
            }
        }
    }
}

/// Lets go of a finished sound, or keeps it for another thread to.
fn retire(retired: &mut Vec<Sound>, keep: bool, sound: Sound) {
    if keep && retired.len() < retired.capacity() {
        retired.push(sound);
    }
}

/// One playing sound.
#[derive(Debug)]
struct Voice {
    id: u64,
    /// `None` when the voice is free.
    sound: Option<Sound>,
    /// How far through the sound it is, in the sound's own samples.
    cursor: f64,
    /// How far the cursor moves for each sample of output.
    step: f64,
    /// Samples of output still to pass before it starts.
    wait: u32,
    place: Place,
    range: Range,
    volume: f32,
    looping: bool,
    importance: Importance,
    /// How loud it is in each ear right now.
    gain: [f32; 2],
    muffle: f32,
    /// What the dulling filter last let through.
    low: f32,
    /// Nothing has been mixed yet, so there is nothing to fade from.
    fresh: bool,
    stopping: bool,
    /// How loud it was at the last mix, for choosing what gives way.
    loudness: f32,
    /// Counts up with each sound started: lower is older.
    order: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            id: 0,
            sound: None,
            cursor: 0.0,
            step: 1.0,
            wait: 0,
            place: Place::Everywhere,
            range: Range::default(),
            volume: 1.0,
            looping: false,
            importance: Importance::Normal,
            gain: [0.0; 2],
            muffle: 0.0,
            low: 0.0,
            fresh: true,
            stopping: false,
            loudness: 0.0,
            order: 0,
        }
    }
}

impl Voice {
    /// Adds this voice to `out`. True when the sound is over.
    fn render(&mut self, out: &mut [f32], heard: Heard, slew: f32, dulling: f32) -> bool {
        let Some(sound) = &self.sound else {
            return true;
        };
        let target = if self.stopping {
            [0.0; 2]
        } else {
            [heard.left * self.volume, heard.right * self.volume]
        };
        if self.fresh {
            if self.stopping {
                return true;
            }
            self.gain = target;
            self.muffle = heard.muffle;
        }

        let data = sound.data();
        let frames = sound.frames();
        let length = frames as f64;
        let stereo = sound.is_stereo();
        let placed = self.place != Place::Everywhere;

        for frame in out.as_chunks_mut::<2>().0 {
            if self.wait > 0 {
                if self.stopping {
                    return true;
                }
                self.wait -= 1;
                continue;
            }
            self.fresh = false;
            if self.cursor >= length {
                if !self.looping {
                    return true;
                }
                self.cursor %= length;
            }
            let index = self.cursor as usize;
            let next = if index + 1 < frames {
                index + 1
            } else if self.looping {
                0
            } else {
                index
            };
            let between = (self.cursor - index as f64) as f32;
            self.cursor += self.step;

            self.gain[0] = approach(self.gain[0], target[0], slew);
            self.gain[1] = approach(self.gain[1], target[1], slew);
            self.muffle = approach(self.muffle, heard.muffle, slew);

            if stereo && !placed {
                let left = data[index * 2];
                let right = data[index * 2 + 1];
                frame[0] += (left + (data[next * 2] - left) * between) * self.gain[0];
                frame[1] += (right + (data[next * 2 + 1] - right) * between) * self.gain[1];
            } else {
                let (from, to) = if stereo {
                    (
                        (data[index * 2] + data[index * 2 + 1]) * 0.5,
                        (data[next * 2] + data[next * 2 + 1]) * 0.5,
                    )
                } else {
                    (data[index], data[next])
                };
                let clear = from + (to - from) * between;
                self.low += (clear - self.low) * dulling;
                let sample = clear + (self.low - clear) * self.muffle;
                frame[0] += sample * self.gain[0];
                frame[1] += sample * self.gain[1];
            }

            if self.stopping && self.gain == [0.0; 2] {
                return true;
            }
        }
        false
    }
}

/// `from` moved toward `to` by `step` at most.
fn approach(from: f32, to: f32, step: f32) -> f32 {
    if from < to {
        (from + step).min(to)
    } else {
        (from - step).max(to)
    }
}

/// Keeps a sample between -1 and 1, bending the loudest part of the mix
/// instead of cutting it off flat.
fn squeeze(sample: f32) -> f32 {
    let size = sample.abs();
    if size <= SQUEEZE_FROM {
        return sample;
    }
    let room = 1.0 - SQUEEZE_FROM;
    (SQUEEZE_FROM + room * ((size - SQUEEZE_FROM) / room).tanh()).copysign(sample)
}
