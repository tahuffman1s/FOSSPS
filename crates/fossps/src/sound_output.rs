//! Where the sounds a game asks for in a frame go: to the speakers, or in
//! tests to a mixer whose output can be read.

use fossps_audio::Play;
use fossps_script::SoundId;

#[cfg(any(feature = "window", test))]
pub(crate) use output::{SoundOutput, Voices};

/// One thing a game asks of the sound in a frame.
// A server gathers these and plays none of them.
#[cfg_attr(not(any(feature = "window", test)), allow(dead_code))]
#[derive(Debug, Clone)]
pub(crate) enum SoundAsk {
    /// Start a sound, to be known by this id from now on.
    Play(SoundId, Play),
    /// Stop the sound started with this id, if it is still playing.
    Stop(SoundId),
    /// Stop every sound started with an id.
    StopAll,
}

/// Playing what was asked for. A server has nothing to play through, so
/// it is built without this.
#[cfg(any(feature = "window", test))]
mod output {
    use std::collections::{HashMap, VecDeque};

    use fossps_audio::{Listener, Mixer, Play, Playing};
    use fossps_script::SoundId;

    use super::SoundAsk;

    /// How many sounds that end by themselves are remembered so that they
    /// can be stopped early. The mixer plays 64 at once, so the ones
    /// forgotten have long ended or been played over.
    pub(super) const REMEMBERED: usize = 512;

    /// How many looped sounds play at once: as many as the mixer has
    /// voices. One more ends the oldest, so that none is left playing
    /// with nothing to stop it by.
    pub(super) const LOOPS: usize = 64;

    /// Something that plays sounds: the speakers, or a mixer.
    pub(crate) trait Voices {
        fn play(&mut self, play: Play) -> Playing;
        fn stop(&mut self, playing: Playing);
        fn set_listener(&mut self, listener: Listener);
    }

    impl Voices for Mixer {
        fn play(&mut self, play: Play) -> Playing {
            Mixer::play(self, play)
        }

        fn stop(&mut self, playing: Playing) {
            Mixer::stop(self, playing);
        }

        fn set_listener(&mut self, listener: Listener) {
            Mixer::set_listener(self, listener);
        }
    }

    /// Plays what games ask for, and remembers what was started so that it
    /// can be stopped.
    #[derive(Debug)]
    pub(crate) struct SoundOutput<V> {
        voices: V,
        pub(super) playing: HashMap<SoundId, Playing>,
        /// The sounds in `playing` that end by themselves, oldest first.
        pub(super) once: VecDeque<SoundId>,
        /// The looped sounds in `playing`, oldest first. They are only
        /// forgotten when they are stopped.
        pub(super) loops: VecDeque<SoundId>,
    }

    impl<V: Voices> SoundOutput<V> {
        pub(crate) fn new(voices: V) -> Self {
            Self {
                voices,
                playing: HashMap::new(),
                once: VecDeque::new(),
                loops: VecDeque::new(),
            }
        }

        /// Puts the ears at `position`, facing `yaw`: the player's eyes.
        pub(crate) fn listen_from(&mut self, position: [f32; 3], yaw: f32) {
            self.voices.set_listener(Listener { position, yaw });
        }

        /// Does what a frame asked for, in the order it was asked.
        pub(crate) fn handle(&mut self, asks: impl IntoIterator<Item = SoundAsk>) {
            for ask in asks {
                match ask {
                    SoundAsk::Play(id, play) => {
                        let looping = play.looping;
                        let playing = self.voices.play(play);
                        self.playing.insert(id, playing);
                        if looping {
                            self.loops.push_back(id);
                            if self.loops.len() > LOOPS
                                && let Some(oldest) = self.loops.pop_front()
                                && let Some(playing) = self.playing.remove(&oldest)
                            {
                                self.voices.stop(playing);
                            }
                        } else {
                            self.once.push_back(id);
                            if self.once.len() > REMEMBERED
                                && let Some(oldest) = self.once.pop_front()
                            {
                                self.playing.remove(&oldest);
                            }
                        }
                    }
                    SoundAsk::Stop(id) => {
                        // One that ends by itself stays remembered until it
                        // is pushed out: stopping it again does nothing.
                        if let Some(playing) = self.playing.get(&id).copied() {
                            self.voices.stop(playing);
                            if let Some(place) = self.loops.iter().position(|kept| *kept == id) {
                                self.loops.remove(place);
                                self.playing.remove(&id);
                            }
                        }
                    }
                    SoundAsk::StopAll => {
                        for (_, playing) in self.playing.drain() {
                            self.voices.stop(playing);
                        }
                        self.once.clear();
                        self.loops.clear();
                    }
                }
            }
        }

        /// What plays the sounds.
        #[cfg(test)]
        pub(crate) fn voices_mut(&mut self) -> &mut V {
            &mut self.voices
        }
    }
}

/// The computer's speakers, opened the first time a sound is played, so a
/// game that makes no sound asks the system for nothing.
#[cfg(feature = "window")]
#[derive(Debug, Default)]
pub(crate) struct LazySpeakers {
    speakers: Option<fossps_audio::Speakers>,
    /// Where the ears are, to tell the speakers once they are open.
    listener: fossps_audio::Listener,
    /// When to next try a device that is not working.
    retry_at: Option<std::time::Instant>,
}

#[cfg(feature = "window")]
impl LazySpeakers {
    /// How long to wait before trying again to reach a sound device, such
    /// as headphones that were unplugged.
    const RETRY_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

    /// The speakers, opened if they were not, and started again if the
    /// device went away and it is time to try.
    fn ready(&mut self) -> &mut fossps_audio::Speakers {
        let listener = self.listener;
        let now = std::time::Instant::now();
        let retry_at = &mut self.retry_at;
        let speakers = self.speakers.get_or_insert_with(|| {
            // Says why in the log when there is nothing to play through.
            let mut speakers = fossps_audio::Speakers::open_or_silent();
            speakers.set_listener(listener);
            *retry_at = Some(now + Self::RETRY_EVERY);
            speakers
        });
        if !speakers.is_working() && retry_at.is_none_or(|at| now >= at) {
            *retry_at = Some(now + Self::RETRY_EVERY);
            if speakers.reconnect().is_ok() {
                log::info!("Sound is playing again.");
                speakers.set_listener(listener);
            }
        }
        speakers
    }
}

#[cfg(feature = "window")]
impl Voices for LazySpeakers {
    fn play(&mut self, play: Play) -> fossps_audio::Playing {
        self.ready().play(play)
    }

    fn stop(&mut self, playing: fossps_audio::Playing) {
        if let Some(speakers) = &mut self.speakers {
            speakers.stop(playing);
        }
    }

    fn set_listener(&mut self, listener: fossps_audio::Listener) {
        self.listener = listener;
        if let Some(speakers) = &mut self.speakers {
            speakers.set_listener(listener);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fossps_audio::Mixer;
    use fossps_input::{ActionMap, GamepadButton, Input};

    use super::*;
    use crate::game::{Frame, Game};
    use crate::online::LocalGame;
    use crate::script_game::ScriptGame;
    use crate::sounds::test_wav;

    const RATE: u32 = 48_000;

    /// A game played alone whose sounds go to a mixer, not to speakers.
    struct Played {
        game: LocalGame,
        input: Input,
        output: SoundOutput<Mixer>,
    }

    impl Played {
        /// Plays the game in `folder`, which is then removed.
        fn open(folder: &std::path::Path) -> Self {
            let actions = ActionMap::default_fps();
            let script = ScriptGame::open(folder, &actions).unwrap();
            assert!(script.host().last_error().is_none());
            let game = LocalGame::start(script, &actions, 60).unwrap();
            Self {
                game,
                input: Input::new(actions),
                output: SoundOutput::new(Mixer::new(RATE)),
            }
        }

        /// One tick and one frame, as the window's loop runs them, and the
        /// sound that comes out over that sixtieth of a second: the
        /// loudest sample in the left ear and in the right.
        fn step(&mut self) -> [f32; 2] {
            self.game.tick(self.input.tick(), 1.0 / 60.0);
            let mut frame = Frame::new(0.0);
            self.game.frame(&mut frame);
            if let Some(view) = frame.view() {
                self.output.listen_from(view.position, view.yaw);
            }
            self.output.handle(frame.take_sounds());
            let mut out = vec![0.0; 2 * (RATE / 60) as usize];
            self.output.voices_mut().mix(&mut out);
            let loudest = |ear: usize| {
                out.iter()
                    .skip(ear)
                    .step_by(2)
                    .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()))
            };
            [loudest(0), loudest(1)]
        }

        fn error(&self) -> Option<String> {
            let error = self.game.hosted().game().host().last_error()?;
            Some(error.message.clone())
        }
    }

    /// A game folder with `script` and a steady, half-loud second of sound
    /// called `tone`.
    fn game(test: &str, script: &str) -> std::path::PathBuf {
        let folder =
            std::env::temp_dir().join(format!("fossps-heard-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("sounds")).unwrap();
        fs::write(folder.join("main.luau"), script).unwrap();
        fs::write(
            folder.join("sounds/tone.wav"),
            test_wav(&vec![i16::MAX / 2; RATE as usize]),
        )
        .unwrap();
        folder
    }

    #[test]
    fn a_sound_the_script_plays_comes_out_of_the_speakers_until_it_is_stopped() {
        let folder = game(
            "played",
            r#"local tone
            game.onTick(function()
                if game.input.justPressed("fire") then
                    tone = game.audio.play("tone", { volume = 0.5, loop = true })
                end
                if game.input.justReleased("fire") then
                    tone:stop()
                    tone:stop()
                end
            end)"#,
        );
        let mut played = Played::open(&folder);
        assert_eq!(played.step(), [0.0, 0.0]);

        played.input.press(GamepadButton::RightTrigger);
        played.step();
        // Half-loud file at half volume, the same in both ears, and still
        // going two seconds later because it loops.
        for _ in 0..120 {
            let [left, right] = played.step();
            assert!((left - 0.25).abs() < 0.01, "{left}");
            assert!((left - right).abs() < 1e-6, "{left} {right}");
        }

        played.input.release(GamepadButton::RightTrigger);
        played.step();
        assert_eq!(played.step(), [0.0, 0.0]);
        assert_eq!(played.error(), None);
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_sound_played_at_a_place_is_louder_in_the_nearer_ear_and_fades_with_distance() {
        // The player starts at the origin facing -Z, so +X is to the right.
        let folder = game(
            "placed",
            r#"local places = { vector.create(3, 1.6, 0), vector.create(-3, 1.6, 0),
                vector.create(30, 1.6, 0), vector.create(300, 1.6, 0) }
            local turn = 0
            game.onTick(function()
                if game.input.justPressed("fire") then
                    turn += 1
                    game.audio.playAt("tone", places[turn])
                end
            end)"#,
        );
        let mut played = Played::open(&folder);
        let mut heard = Vec::new();
        for _ in 0..4 {
            played.input.press(GamepadButton::RightTrigger);
            played.step();
            heard.push(played.step());
            played.input.release(GamepadButton::RightTrigger);
            // The one-second sound ends before the next is played.
            for _ in 0..70 {
                played.step();
            }
            assert_eq!(played.step(), [0.0, 0.0]);
        }
        let [right, left, far, out_of_earshot] = heard[..] else {
            panic!("four sounds were played");
        };
        assert!(right[1] > right[0] * 1.5, "{right:?}");
        assert!(left[0] > left[1] * 1.5, "{left:?}");
        assert!((right[1] - left[0]).abs() < 1e-4, "{right:?} {left:?}");
        assert!(far[1] > 0.0 && far[1] < right[1] / 5.0, "{far:?}");
        assert_eq!(out_of_earshot, [0.0, 0.0]);
        assert_eq!(played.error(), None);
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_sound_that_is_not_in_the_game_is_a_script_error_and_plays_nothing() {
        let folder = game(
            "missing",
            r#"game.onStart(function() game.audio.play("tune") end)"#,
        );
        let mut played = Played::open(&folder);
        assert_eq!(played.step(), [0.0, 0.0]);
        assert_eq!(
            played.error().as_deref(),
            Some("There is no sound called \"tune\". Did you mean \"tone\"?")
        );
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_looped_sound_can_still_be_stopped_after_many_other_sounds() {
        use super::output::{LOOPS, REMEMBERED};

        let mut output = SoundOutput::new(Mixer::new(RATE));
        let sound = fossps_audio::Sound::from_samples(RATE, 1, vec![0.5; 64]).unwrap();
        let once = |id| SoundAsk::Play(SoundId(id), Play::everywhere(sound.clone()));
        let looped = |id| SoundAsk::Play(SoundId(id), Play::everywhere(sound.clone()).looping());
        let heard = |output: &mut SoundOutput<Mixer>| {
            let mut out = vec![0.0; 2 * RATE as usize];
            output.voices_mut().mix(&mut out);
            out[out.len() - 2..].iter().any(|sample| *sample != 0.0)
        };

        output.handle([looped(0)]);
        // A few at a time, each lot ending before the next, as in a game:
        // all at once they would take the loop's voice.
        for lot in 0..100 {
            output.handle((1..=20).map(|id| once(lot * 20 + id)));
            assert!(heard(&mut output), "the loop outlasts the others");
        }
        assert_eq!(output.once.len(), REMEMBERED);
        assert_eq!(output.playing.len(), REMEMBERED + 1);
        assert!(heard(&mut output), "the loop outlasts the others");
        // Stopping a forgotten sound, or one never played, does nothing.
        output.handle([SoundAsk::Stop(SoundId(1)), SoundAsk::Stop(SoundId(9999))]);
        assert!(heard(&mut output));
        output.handle([SoundAsk::Stop(SoundId(0)), SoundAsk::Stop(SoundId(0))]);
        assert!(!heard(&mut output));
        assert!(output.loops.is_empty());

        // One loop too many ends the oldest, and everything can be stopped
        // at once.
        output.handle((3000..=3000 + LOOPS as u64).map(looped));
        assert_eq!(output.loops.len(), LOOPS);
        assert!(!output.playing.contains_key(&SoundId(3000)));
        assert!(heard(&mut output));
        output.handle([SoundAsk::StopAll]);
        assert!(!heard(&mut output));
        assert!(output.playing.is_empty() && output.once.is_empty() && output.loops.is_empty());
    }
}
