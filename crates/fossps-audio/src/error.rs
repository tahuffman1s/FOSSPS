//! What can go wrong while loading or playing sound.

use std::fmt;
use std::path::PathBuf;

/// A problem with a sound file or with the computer's sound output.
///
/// Every message says what to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioError {
    /// A sound file could not be opened or read.
    CannotRead {
        /// The file that was asked for.
        path: PathBuf,
        /// What the operating system said.
        reason: String,
    },
    /// The data is not a WAV file this crate can read.
    NotWav {
        /// What was wrong with it.
        reason: String,
    },
    /// A sound with more than two channels.
    TooManyChannels {
        /// How many channels the sound has.
        found: u32,
    },
    /// A sound with nothing in it.
    Empty,
    /// A sample rate no sound card plays.
    BadRate {
        /// The rate that was given, in samples a second.
        found: u32,
    },
    /// A [`SoundSet`](crate::SoundSet) made from an empty list.
    NoSounds,
    /// The computer has nothing to play sound through.
    NoDevice,
    /// The sound device refused to start or stopped working.
    DeviceFailed {
        /// What the sound system said.
        reason: String,
    },
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CannotRead { path, reason } => write!(
                f,
                "Could not read the sound file \"{}\": {reason}. Check the name and that the \
                 file is in the game's folder.",
                path.display()
            ),
            Self::NotWav { reason } => write!(
                f,
                "This is not a WAV file the engine can read: {reason}. Export the sound as a \
                 16-bit WAV and try again."
            ),
            Self::TooManyChannels { found } => write!(
                f,
                "This sound has {found} channels. Export it as mono (best for sounds in the \
                 world) or stereo (for music and menus)."
            ),
            Self::Empty => {
                f.write_str("This sound has no samples in it. Export it again from your editor.")
            }
            Self::BadRate { found } => write!(
                f,
                "This sound says it plays {found} samples a second. Export it at 44100 or 48000."
            ),
            Self::NoSounds => f.write_str(
                "A sound set needs at least one sound. Give it one, or a few takes of the same \
                 sound so it does not repeat.",
            ),
            Self::NoDevice => f.write_str(
                "No speakers or headphones were found, so the game will run without sound. Plug \
                 some in or check the system's sound settings, then start the game again.",
            ),
            Self::DeviceFailed { reason } => write!(
                f,
                "The sound device stopped working: {reason}. The game will run without sound. \
                 Check the system's sound settings, then start the game again."
            ),
        }
    }
}

impl std::error::Error for AudioError {}
