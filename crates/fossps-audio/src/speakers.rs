//! Plays a mixer through the computer's speakers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::{AudioError, Listener, Mixer, Play, Playing, Sound};

/// How many requests can wait for the sound thread at once. More than a
/// frame could sensibly ask for; past it, new requests are dropped.
const WAITING: usize = 1024;

/// How many moments of sound are mixed in one go.
const BLOCK_FRAMES: usize = 1024;

/// A request from the game to the sound thread.
enum Request {
    Play(Playing, Play),
    Stop(Playing),
    StopAll,
    MoveTo(Playing, [f32; 3]),
    Listener(Listener),
    Volume(f32),
}

/// The computer's speakers or headphones, with a [`Mixer`] playing
/// through them.
///
/// It has the mixer's own methods, and they never wait: each one leaves a
/// note for the sound thread and returns.
///
/// On a machine with no sound card, or when the device is unplugged,
/// nothing breaks: start with [`open_or_silent`](Self::open_or_silent) and
/// the game runs the same without sound.
///
/// ```no_run
/// use fossps_audio::{Listener, Speakers, starter};
///
/// let mut speakers = Speakers::open_or_silent();
/// let mut gunfire = starter::gunfire();
///
/// // Every tick:
/// speakers.set_listener(Listener { position: [0.0, 1.6, 0.0], yaw: 0.0 });
/// // When someone fires:
/// let tick = 0;
/// speakers.play(gunfire.at([10.0, 1.4, -25.0], tick));
/// ```
pub struct Speakers {
    link: Option<Link>,
    next_id: u64,
}

/// The live connection to a sound device.
struct Link {
    /// Kept so the stream keeps playing; dropping it stops the sound.
    _stream: cpal::Stream,
    requests: rtrb::Producer<Request>,
    /// Finished sounds coming back, so this thread is the one to free them.
    finished: rtrb::Consumer<Sound>,
    failed: Arc<AtomicBool>,
    rate: u32,
}

impl Speakers {
    /// Starts playing through the system's default speakers or headphones.
    ///
    /// # Errors
    ///
    /// [`AudioError::NoDevice`] when there is nothing to play through, and
    /// [`AudioError::DeviceFailed`] when the device would not start.
    pub fn open() -> Result<Self, AudioError> {
        Ok(Self {
            link: Some(Link::open()?),
            next_id: 1,
        })
    }

    /// Speakers that play nothing. Every method still works.
    #[must_use]
    pub fn silent() -> Self {
        Self {
            link: None,
            next_id: 1,
        }
    }

    /// [`open`](Self::open), or [`silent`](Self::silent) speakers and a
    /// warning in the log when that fails.
    #[must_use]
    pub fn open_or_silent() -> Self {
        Self::open().unwrap_or_else(|error| {
            log::warn!("{error}");
            Self::silent()
        })
    }

    /// Whether sound is reaching a device right now. False for silent
    /// speakers and after the device was unplugged or stopped.
    #[must_use]
    pub fn is_working(&self) -> bool {
        self.link
            .as_ref()
            .is_some_and(|link| !link.failed.load(Ordering::Relaxed))
    }

    /// Starts again on the system's default device, after
    /// [`is_working`](Self::is_working) turned false. Sounds that were
    /// playing are not carried over.
    ///
    /// # Errors
    ///
    /// The same as [`open`](Self::open). The speakers stay as they were.
    pub fn reconnect(&mut self) -> Result<(), AudioError> {
        self.link = Some(Link::open()?);
        Ok(())
    }

    /// How many samples a second the device plays, or `None` when silent.
    #[must_use]
    pub fn rate(&self) -> Option<u32> {
        self.link.as_ref().map(|link| link.rate)
    }

    /// Starts a sound. See [`Mixer::play`].
    pub fn play(&mut self, play: Play) -> Playing {
        let playing = Playing(self.next_id);
        self.next_id += 1;
        self.send(Request::Play(playing, play));
        playing
    }

    /// Fades a sound out and ends it.
    pub fn stop(&mut self, playing: Playing) {
        self.send(Request::Stop(playing));
    }

    /// Fades out and ends every sound.
    pub fn stop_all(&mut self) {
        self.send(Request::StopAll);
    }

    /// Moves a playing sound.
    pub fn move_to(&mut self, playing: Playing, position: [f32; 3]) {
        self.send(Request::MoveTo(playing, position));
    }

    /// Says where the player's head is. Call it every tick.
    pub fn set_listener(&mut self, listener: Listener) {
        self.send(Request::Listener(listener));
    }

    /// Sets the volume of everything, from 0 (silent) to 1 (full).
    pub fn set_volume(&mut self, volume: f32) {
        self.send(Request::Volume(volume));
    }

    fn send(&mut self, request: Request) {
        let Some(link) = &mut self.link else {
            return;
        };
        // Let go of sounds the sound thread has finished with.
        while link.finished.pop().is_ok() {}
        // When the line is full the request is dropped: a missed sound is
        // better than a game that waits on its sound card.
        let _ = link.requests.push(request);
    }
}

impl std::fmt::Debug for Speakers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speakers")
            .field("working", &self.is_working())
            .field("rate", &self.rate())
            .finish()
    }
}

impl Link {
    fn open() -> Result<Self, AudioError> {
        let failed = |error: cpal::Error| AudioError::DeviceFailed {
            reason: error.to_string(),
        };
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;
        let supported = device.default_output_config().map_err(failed)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate;
        let channels = usize::from(config.channels).max(1);

        let (requests, incoming) = rtrb::RingBuffer::new(WAITING);
        let (outgoing, finished) = rtrb::RingBuffer::new(WAITING);
        let mut mixer = Mixer::new(rate);
        mixer.keep_retired();
        let feed = Feed {
            mixer,
            incoming,
            outgoing,
            channels,
            block: [0.0; BLOCK_FRAMES * 2],
        };
        let failed_flag = Arc::new(AtomicBool::new(false));

        let stream = match format {
            SampleFormat::F32 => feed.start::<f32>(&device, config, &failed_flag),
            SampleFormat::I16 => feed.start::<i16>(&device, config, &failed_flag),
            SampleFormat::U16 => feed.start::<u16>(&device, config, &failed_flag),
            SampleFormat::I32 => feed.start::<i32>(&device, config, &failed_flag),
            SampleFormat::U8 => feed.start::<u8>(&device, config, &failed_flag),
            other => {
                return Err(AudioError::DeviceFailed {
                    reason: format!("it wants samples as {other}, which the engine cannot make"),
                });
            }
        }
        .map_err(failed)?;
        stream.play().map_err(failed)?;

        Ok(Self {
            _stream: stream,
            requests,
            finished,
            failed: failed_flag,
            rate,
        })
    }
}

/// What the sound thread owns: it takes requests and fills the device's
/// buffers. Nothing here locks, waits or asks for memory.
struct Feed {
    mixer: Mixer,
    incoming: rtrb::Consumer<Request>,
    outgoing: rtrb::Producer<Sound>,
    channels: usize,
    block: [f32; BLOCK_FRAMES * 2],
}

impl Feed {
    fn start<T>(
        mut self,
        device: &cpal::Device,
        config: cpal::StreamConfig,
        failed: &Arc<AtomicBool>,
    ) -> Result<cpal::Stream, cpal::Error>
    where
        T: SizedSample + FromSample<f32>,
    {
        let failed = Arc::clone(failed);
        device.build_output_stream(
            config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| self.fill(out),
            move |error| match error.kind() {
                // A hiccup, or the system moved the stream to another
                // device by itself: the sound carries on.
                cpal::ErrorKind::Xrun
                | cpal::ErrorKind::DeviceChanged
                | cpal::ErrorKind::RealtimeDenied => log::debug!("Sound device: {error}"),
                _ => {
                    log::warn!(
                        "{}",
                        AudioError::DeviceFailed {
                            reason: error.to_string()
                        }
                    );
                    failed.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
    }

    fn fill<T>(&mut self, out: &mut [T])
    where
        T: SizedSample + FromSample<f32>,
    {
        while let Ok(request) = self.incoming.pop() {
            match request {
                Request::Play(playing, play) => self.mixer.play_as(playing, play),
                Request::Stop(playing) => self.mixer.stop(playing),
                Request::StopAll => self.mixer.stop_all(),
                Request::MoveTo(playing, position) => self.mixer.move_to(playing, position),
                Request::Listener(listener) => self.mixer.set_listener(listener),
                Request::Volume(volume) => self.mixer.set_volume(volume),
            }
        }

        for part in out.chunks_mut(BLOCK_FRAMES * self.channels) {
            let frames = part.len() / self.channels;
            let mixed = &mut self.block[..frames * 2];
            self.mixer.mix(mixed);
            for (frame, pair) in part
                .chunks_mut(self.channels)
                .zip(mixed.as_chunks::<2>().0.iter())
            {
                match frame {
                    [only] => *only = T::from_sample((pair[0] + pair[1]) * 0.5),
                    [left, right, others @ ..] => {
                        *left = T::from_sample(pair[0]);
                        *right = T::from_sample(pair[1]);
                        others.fill(T::EQUILIBRIUM);
                    }
                    [] => {}
                }
            }
        }

        let outgoing = &mut self.outgoing;
        self.mixer.hand_over_retired(|sound| {
            outgoing
                .push(sound)
                .map_err(|rtrb::PushError::Full(sound)| sound)
        });
    }
}
