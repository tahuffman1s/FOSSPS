//! A sound held in memory, ready to play.

use std::fmt;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use crate::AudioError;

/// The slowest and fastest sample rates a sound may have.
const RATES: std::ops::RangeInclusive<u32> = 4_000..=384_000;

/// A sound held in memory, ready to play any number of times at once.
///
/// Cloning is cheap: every clone shares the same samples. Load each sound
/// once when the game starts and keep it.
///
/// Sounds that come from a place in the world should be mono. A stereo
/// sound played at a place is folded down to mono first.
#[derive(Clone)]
pub struct Sound(Arc<Samples>);

struct Samples {
    /// One number per channel per moment, left before right.
    data: Box<[f32]>,
    channels: usize,
    rate: u32,
}

impl Sound {
    /// A sound made from samples between -1 and 1.
    ///
    /// `rate` is how many moments there are a second, and `channels` is 1
    /// for mono or 2 for stereo (left sample, then right, for each moment).
    ///
    /// # Errors
    ///
    /// [`AudioError::TooManyChannels`], [`AudioError::BadRate`] or
    /// [`AudioError::Empty`] when the sound cannot be played.
    pub fn from_samples(
        rate: u32,
        channels: u32,
        mut samples: Vec<f32>,
    ) -> Result<Self, AudioError> {
        let channels = match channels {
            1 => 1,
            2 => 2,
            found => return Err(AudioError::TooManyChannels { found }),
        };
        if !RATES.contains(&rate) {
            return Err(AudioError::BadRate { found: rate });
        }
        samples.truncate(samples.len() - samples.len() % channels);
        if samples.is_empty() {
            return Err(AudioError::Empty);
        }
        Ok(Self::checked(rate, channels, samples))
    }

    /// A mono sound the engine made itself. Silence if `samples` is empty.
    pub(crate) fn mono(rate: u32, mut samples: Vec<f32>) -> Self {
        if samples.is_empty() {
            samples.push(0.0);
        }
        Self::checked(rate, 1, samples)
    }

    fn checked(rate: u32, channels: usize, mut samples: Vec<f32>) -> Self {
        for sample in &mut samples {
            *sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
        }
        Self(Arc::new(Samples {
            data: samples.into_boxed_slice(),
            channels,
            rate,
        }))
    }

    /// Reads a sound from the bytes of a WAV file.
    ///
    /// # Errors
    ///
    /// [`AudioError::NotWav`] when the bytes are not a WAV file, and the
    /// errors of [`from_samples`](Self::from_samples).
    pub fn from_wav(bytes: &[u8]) -> Result<Self, AudioError> {
        let not_wav = |error: hound::Error| AudioError::NotWav {
            reason: error.to_string(),
        };
        let mut reader = hound::WavReader::new(Cursor::new(bytes)).map_err(not_wav)?;
        let spec = reader.spec();
        let samples = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<Vec<f32>, _>>()
                .map_err(not_wav)?,
            hound::SampleFormat::Int => {
                // The largest value a sample of this many bits can hold.
                let full = 2.0_f32.powi(i32::from(spec.bits_per_sample.clamp(1, 32)) - 1);
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|sample| sample as f32 / full))
                    .collect::<Result<Vec<f32>, _>>()
                    .map_err(not_wav)?
            }
        };
        Self::from_samples(spec.sample_rate, u32::from(spec.channels), samples)
    }

    /// Reads a sound from a WAV file.
    ///
    /// # Errors
    ///
    /// [`AudioError::CannotRead`] when the file cannot be opened, and the
    /// errors of [`from_wav`](Self::from_wav).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| AudioError::CannotRead {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        Self::from_wav(&bytes)
    }

    /// How long the sound lasts at normal pitch, in seconds.
    #[must_use]
    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.0.rate as f32
    }

    /// How many samples a second the sound was recorded at.
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.0.rate
    }

    /// 1 for mono, 2 for stereo.
    #[must_use]
    pub fn channels(&self) -> u32 {
        self.0.channels as u32
    }

    /// How many moments of sound there are.
    pub(crate) fn frames(&self) -> usize {
        self.0.data.len() / self.0.channels
    }

    pub(crate) fn is_stereo(&self) -> bool {
        self.0.channels == 2
    }

    pub(crate) fn data(&self) -> &[f32] {
        &self.0.data
    }
}

impl fmt::Debug for Sound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sound")
            .field("seconds", &self.seconds())
            .field("rate", &self.0.rate)
            .field("channels", &self.0.channels)
            .finish()
    }
}
