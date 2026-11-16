//! The sounds a game can play by name: the engine's starter sounds, and the
//! WAV files in the game's `sounds` folder.
//!
//! `sounds/door.wav` is the sound `door`, and `sounds/ui/click.wav` is
//! `ui/click`. A file named like a starter sound (`gunshot.wav`) takes its
//! place. A script only ever names a sound from this list, so nothing a
//! script writes decides which file is read.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use fossps_audio::{Play, Range, Sound, SoundSet, starter};
use fossps_pack::Pack;
use fossps_script::SoundRequest;

/// The folder inside a game's folder that holds its sounds.
pub const SOUNDS_FOLDER: &str = "sounds";

/// The sounds every game has without a file of its own.
pub const STARTER_SOUNDS: [&str; 5] = ["dry-fire", "footstep", "gunshot", "landing", "reload"];

/// How often a game's folder is looked at for new, changed or removed sounds.
const LOOK_EVERY: Duration = Duration::from_millis(250);

/// How many sound files a game can have. Far more than a game needs; it
/// bounds what a look at the folder costs.
const MAX_FILES: usize = 4096;

/// How many folders deep inside `sounds` files are looked for.
const MAX_DEPTH: usize = 8;

/// Sound files that are not WAV. Anything else beside the sounds, such as a
/// licence text, is nobody's mistake.
const OTHER_SOUND_ENDINGS: [&str; 9] = [
    "ogg", "mp3", "flac", "opus", "m4a", "aac", "aiff", "aif", "wma",
];

/// One take, or several to choose among.
#[derive(Debug)]
enum Source {
    One(Sound),
    Takes(SoundSet),
}

#[derive(Debug)]
struct Entry {
    source: Source,
    /// How far it carries when the script does not say.
    range: Range,
}

/// A sound file in a game's folder, as it was when last looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    /// The sound's name: `ui/click` for `sounds/ui/click.wav`.
    name: String,
    path: PathBuf,
    changed: Option<SystemTime>,
    bytes: u64,
}

/// The sounds one game can play.
#[derive(Debug)]
pub struct Sounds {
    /// The game's `sounds` folder, for a game that is a folder.
    folder: Option<PathBuf>,
    /// The sound files that were in that folder at the last look, or
    /// `None` before the first.
    seen: Option<Vec<Found>>,
    /// The files among them that are, or may yet be, sounds to play.
    found: Vec<Found>,
    /// How many of the game's own files are sounds to play.
    own: usize,
    /// The sounds that are ready to play.
    entries: BTreeMap<String, Entry>,
    /// What is wrong with the files that cannot be played.
    problems: Vec<String>,
    /// The files in `found` that have not been read yet.
    waiting: Vec<Found>,
    next_look: Instant,
}

impl Sounds {
    /// The starter sounds alone: what a game with no `sounds` folder has.
    #[must_use]
    pub fn starter() -> Self {
        Self {
            folder: None,
            seen: None,
            found: Vec::new(),
            own: 0,
            entries: starters(),
            problems: Vec::new(),
            waiting: Vec::new(),
            next_look: Instant::now(),
        }
    }

    /// The sounds of the game in the folder `game`. The files are only
    /// listed here; [`load`](Self::load) reads them, which a server, where
    /// nobody listens, never needs to do.
    #[must_use]
    pub fn in_folder(game: &Path) -> Self {
        let mut sounds = Self::starter();
        sounds.folder = Some(game.join(SOUNDS_FOLDER));
        sounds.look();
        sounds
    }

    /// The sounds of the game in `pack`, read and ready to play.
    #[must_use]
    pub fn in_pack(pack: &mut Pack) -> Self {
        let mut sounds = Self::starter();
        let prefix = format!("{SOUNDS_FOLDER}/");
        let mut files: Vec<String> = pack
            .files()
            .filter(|file| file.starts_with(&prefix))
            .map(str::to_owned)
            .collect();
        files.sort();
        for file in files {
            let Some(name) = sounds.name_of(&file[prefix.len()..]) else {
                continue;
            };
            let read = pack
                .read(&file)
                .map_err(|error| error.to_string())
                .and_then(|bytes| Sound::from_wav(&bytes).map_err(|error| error.to_string()));
            if sounds.add(name, read) {
                sounds.own += 1;
            }
        }
        sounds
    }

    /// Every sound a script can play, by name, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        let found = self.found.iter().map(|found| found.name.as_str());
        let ready = self.entries.keys().map(String::as_str);
        let mut names: Vec<&str> = found.chain(ready).collect();
        names.sort_unstable();
        names.dedup();
        names.into_iter()
    }

    /// How many of the game's own files are sounds to play.
    #[must_use]
    pub fn own(&self) -> usize {
        self.own
    }

    /// What is wrong with the sound files that cannot be played, one
    /// sentence each, saying what to do. Files that have not been read yet
    /// ([`load`](Self::load)) have had no chance to be wrong.
    #[must_use]
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Reads the sound files that have not been read yet. Returns whether
    /// there were any.
    pub fn load(&mut self) -> bool {
        if self.waiting.is_empty() {
            return false;
        }
        for file in std::mem::take(&mut self.waiting) {
            let read = Sound::load(&file.path).map_err(|error| error.to_string());
            if !self.add(file.name.clone(), read) {
                self.found.retain(|found| *found != file);
            }
        }
        self.own = self.found.len();
        true
    }

    /// Looks at the game's folder again if it is time to, and says whether
    /// the sounds in it have changed since the last look. A game from a
    /// pack never changes.
    pub fn poll(&mut self) -> bool {
        let now = Instant::now();
        if self.folder.is_none() || now < self.next_look {
            return false;
        }
        self.next_look = now + LOOK_EVERY;
        self.look()
    }

    /// Lists the sound files in the game's folder. Returns whether they
    /// are not the ones that were there at the last look.
    fn look(&mut self) -> bool {
        let Some(folder) = self.folder.clone() else {
            return false;
        };
        let mut problems = Vec::new();
        let mut found = Vec::new();
        let mut folders = vec![(folder, String::new(), 0)];
        while let Some((folder, prefix, depth)) = folders.pop() {
            // No folder is a game with no sounds of its own.
            let Ok(entries) = fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file) = entry.file_name().into_string() else {
                    continue;
                };
                // Not followed, so a link is seen as a link and left alone:
                // it could lead anywhere.
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if file.starts_with('.') {
                    continue;
                }
                if kind.is_dir() {
                    if depth < MAX_DEPTH {
                        folders.push((entry.path(), format!("{prefix}{file}/"), depth + 1));
                    }
                } else if kind.is_file() {
                    let within = format!("{prefix}{file}");
                    let Some(name) = name_of(&within, &mut problems) else {
                        continue;
                    };
                    if found.len() >= MAX_FILES {
                        continue;
                    }
                    let about = entry.metadata().ok();
                    found.push(Found {
                        name,
                        path: entry.path(),
                        changed: about.as_ref().and_then(|about| about.modified().ok()),
                        bytes: about.map_or(0, |about| about.len()),
                    });
                }
            }
        }
        if found.len() >= MAX_FILES {
            problems.push(format!(
                "Only the first {MAX_FILES} files in the {SOUNDS_FOLDER} folder are sounds \
                 to play. Remove the ones the game does not use."
            ));
        }
        found.sort_by(|a, b| (&a.name, &a.path).cmp(&(&b.name, &b.path)));
        // `door.wav` beside `door.WAV`: the same name twice.
        let mut kept: Vec<Found> = Vec::with_capacity(found.len());
        for file in found {
            match kept.last().filter(|kept| kept.name == file.name) {
                Some(first) => problems.push(format!(
                    "{} was not loaded: {} has the same name, \"{}\". Rename one of them.",
                    file.path.display(),
                    first.path.display(),
                    file.name
                )),
                None => kept.push(file),
            }
        }
        // Compared by what is in the folder, not by what loaded: a file
        // that could not be read is not read again until it is saved again.
        let first = self.seen.is_none();
        if self.seen.as_ref() == Some(&kept) {
            return false;
        }
        self.seen = Some(kept.clone());
        // A file that was read and is as it was keeps its sound, so saving
        // one file reads one file, and the others go on playing.
        let mut entries = starters();
        let mut waiting = Vec::new();
        for file in &kept {
            let read = self.found.contains(file) && !self.waiting.contains(file);
            match self.entries.remove(&file.name) {
                Some(entry) if read => {
                    entries.insert(file.name.clone(), entry);
                }
                _ => waiting.push(file.clone()),
            }
        }
        self.entries = entries;
        self.waiting = waiting;
        self.own = kept.len();
        self.found = kept;
        self.problems = problems;
        !first
    }

    /// The sound's name for a file inside the sounds folder, or `None`
    /// for a file that is not a sound to play.
    fn name_of(&mut self, within: &str) -> Option<String> {
        name_of(within, &mut self.problems)
    }

    /// Makes `name` playable, or notes why it is not. Returns whether it is.
    fn add(&mut self, name: String, read: Result<Sound, String>) -> bool {
        match read {
            Ok(sound) => {
                let entry = Entry {
                    source: Source::One(sound),
                    range: Range::default(),
                };
                self.entries.insert(name, entry);
                true
            }
            Err(error) => {
                self.problems.push(format!(
                    "{SOUNDS_FOLDER}/{name}.wav was not loaded. {error}"
                ));
                false
            }
        }
    }

    /// What to hand the speakers for `request`, or `None` when the game has
    /// no sound of that name ready.
    pub(crate) fn play(&mut self, request: &SoundRequest) -> Option<Play> {
        let entry = self.entries.get_mut(&request.name)?;
        let mut play = match &mut entry.source {
            Source::One(sound) => Play::everywhere(sound.clone()),
            // Which take plays only has to differ from sound to sound.
            #[allow(clippy::cast_possible_truncation)]
            Source::Takes(takes) => takes.everywhere(request.id.0 as u32),
        };
        if let Some(position) = request.position {
            play.place = fossps_audio::Place::At(position);
        }
        play.volume *= request.volume;
        play.pitch *= request.pitch;
        play.looping = request.looping;
        // Music and the like: a burst of gunfire must not take its voice,
        // because nothing would start it again.
        if request.looping && request.position.is_none() {
            play.importance = fossps_audio::Importance::High;
        }
        play.range = match request.range {
            // The whole range is stretched, so the sound falls away the
            // same way over a longer or shorter distance.
            Some(far) => Range {
                near: entry.range.near * far / entry.range.far,
                far,
            },
            None => entry.range,
        };
        Some(play)
    }
}

/// The sound's name for a file inside the sounds folder: without `.wav`.
/// `None` for a file that is not a sound to play; `problems` is told when
/// it looks like one that was meant to be.
fn name_of(within: &str, problems: &mut Vec<String>) -> Option<String> {
    let (name, ending) = within.rsplit_once('.')?;
    if ending.eq_ignore_ascii_case("wav") && !name.is_empty() {
        return Some(name.to_owned());
    }
    if OTHER_SOUND_ENDINGS
        .iter()
        .any(|other| ending.eq_ignore_ascii_case(other))
    {
        problems.push(format!(
            "{SOUNDS_FOLDER}/{within} was not loaded: only WAV files can be played for \
             now. Export it as a 16-bit WAV file (in Audacity: File, Export Audio, WAV) \
             and save it as {SOUNDS_FOLDER}/{name}.wav."
        ));
    }
    None
}

/// The engine's own sounds, made from arithmetic.
fn starters() -> BTreeMap<String, Entry> {
    let one = |sound, range| Entry {
        source: Source::One(sound),
        range,
    };
    let takes = |takes, range| Entry {
        source: Source::Takes(takes),
        range,
    };
    let close = Range::default();
    let entries = [
        ("dry-fire", one(starter::dry_fire(), close)),
        ("footstep", takes(starter::footsteps(), Range::FOOTSTEP)),
        ("gunshot", takes(starter::gunfire(), Range::GUNFIRE)),
        ("landing", one(starter::landing(), Range::FOOTSTEP)),
        ("reload", one(starter::reload(), close)),
    ];
    entries
        .into_iter()
        .map(|(name, entry)| (name.to_owned(), entry))
        .collect()
}

/// A 16-bit, one-channel WAV file of `samples`, 48,000 a second.
#[cfg(test)]
pub(crate) fn test_wav(samples: &[i16]) -> Vec<u8> {
    let data = u32::try_from(samples.len() * 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    // PCM, one channel, 48 kHz, 96,000 bytes a second, 2 bytes a frame, 16 bits.
    for field in [1_u16, 1] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&96_000_u32.to_le_bytes());
    for field in [2_u16, 16] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use fossps_audio::Place;
    use fossps_script::SoundId;

    use super::*;

    /// A folder for one test's game, with `files` (name, contents) in it.
    fn game(test: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("fossps-sounds-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        for (name, contents) in files {
            let path = folder.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn names(sounds: &Sounds) -> Vec<&str> {
        sounds.names().collect()
    }

    fn request(name: &str) -> SoundRequest {
        SoundRequest {
            id: SoundId(7),
            name: name.to_owned(),
            position: None,
            volume: 1.0,
            pitch: 1.0,
            range: None,
            looping: false,
        }
    }

    #[test]
    fn a_game_with_no_sounds_folder_has_the_starter_sounds() {
        let folder = game("none", &[]);
        let mut sounds = Sounds::in_folder(&folder);
        assert_eq!(names(&sounds), STARTER_SOUNDS);
        assert_eq!(sounds.own(), 0);
        assert!(!sounds.load(), "there is nothing to read");
        assert!(sounds.problems().is_empty());
        for name in STARTER_SOUNDS {
            assert!(sounds.play(&request(name)).is_some(), "{name}");
        }
        assert!(sounds.play(&request("door")).is_none());
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_wav_file_in_the_sounds_folder_is_a_sound_named_without_its_ending() {
        let wav = test_wav(&[1000; 480]);
        let folder = game(
            "named",
            &[
                ("sounds/door.wav", &wav),
                ("sounds/ui/Click.WAV", &wav),
                ("sounds/LICENCE.txt", b"CC0"),
                ("sounds/.hidden.wav", &wav),
                ("elsewhere/not-a-sound.wav", &wav),
            ],
        );
        let mut sounds = Sounds::in_folder(&folder);
        // Known by name before a single file is read.
        assert!(names(&sounds).contains(&"door"));
        assert!(names(&sounds).contains(&"ui/Click"));
        assert_eq!(sounds.own(), 2);
        assert_eq!(names(&sounds).len(), STARTER_SOUNDS.len() + 2);
        assert!(sounds.play(&request("door")).is_none(), "not read yet");

        assert!(sounds.load());
        assert!(sounds.problems().is_empty(), "{:?}", sounds.problems());
        let play = sounds.play(&request("door")).unwrap();
        assert_eq!(play.sound.seconds(), 0.01);
        assert_eq!(play.place, Place::Everywhere);
        assert!(!sounds.load(), "read once");
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_file_that_cannot_be_played_is_explained_and_is_not_a_sound() {
        let folder = game(
            "broken",
            &[
                ("sounds/music.ogg", b"OggS"),
                ("sounds/door.wav", b"this is not a WAV file"),
                ("sounds/gunshot.wav", b"neither is this"),
            ],
        );
        let mut sounds = Sounds::in_folder(&folder);
        sounds.load();
        let problems = sounds.problems().join("\n");
        assert!(
            problems.contains(
                "sounds/music.ogg was not loaded: only WAV files can be played for now. \
                 Export it as a 16-bit WAV file (in Audacity: File, Export Audio, WAV) and \
                 save it as sounds/music.wav."
            ),
            "{problems}"
        );
        assert!(
            problems.contains("sounds/door.wav was not loaded."),
            "{problems}"
        );
        assert_eq!(sounds.problems().len(), 3, "{problems}");
        // The starter gunshot is still there; the door is not.
        assert_eq!(names(&sounds), STARTER_SOUNDS);
        assert!(sounds.play(&request("gunshot")).is_some());
        assert_eq!(sounds.own(), 0);
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_file_named_like_a_starter_sound_takes_its_place() {
        let folder = game("own", &[("sounds/gunshot.wav", &test_wav(&[1000; 4800]))]);
        let mut sounds = Sounds::in_folder(&folder);
        let starter = sounds.play(&request("gunshot")).unwrap().sound.seconds();
        sounds.load();
        assert_eq!(names(&sounds), STARTER_SOUNDS);
        assert_eq!(sounds.own(), 1);
        let own = sounds.play(&request("gunshot")).unwrap();
        assert_eq!(own.sound.seconds(), 0.1);
        assert_ne!(starter, 0.1);
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_request_says_where_how_loud_and_how_far() {
        let mut sounds = Sounds::starter();
        let play = sounds
            .play(&SoundRequest {
                position: Some([1.0, 2.0, 3.0]),
                volume: 0.5,
                pitch: 2.0,
                range: Some(100.0),
                looping: true,
                ..request("reload")
            })
            .unwrap();
        assert_eq!(play.place, Place::At([1.0, 2.0, 3.0]));
        assert_eq!((play.volume, play.pitch, play.looping), (0.5, 2.0, true));
        assert_eq!(play.importance, fossps_audio::Importance::Normal);
        let music = SoundRequest {
            looping: true,
            ..request("reload")
        };
        assert_eq!(
            sounds.play(&music).unwrap().importance,
            fossps_audio::Importance::High
        );
        assert_eq!(
            play.range,
            Range {
                near: 2.0,
                far: 100.0
            }
        );

        // Left to itself, a starter sound carries as far as the real thing.
        let shot = sounds.play(&request("gunshot")).unwrap();
        assert_eq!(shot.range, Range::GUNFIRE);
        assert!(shot.volume <= 1.0 && shot.volume > 0.9, "{}", shot.volume);
    }

    #[test]
    fn a_sound_saved_while_the_game_runs_is_picked_up() {
        let folder = game("saved", &[("sounds/door.wav", &test_wav(&[1000; 480]))]);
        let mut sounds = Sounds::in_folder(&folder);
        sounds.load();
        sounds.next_look = Instant::now();
        assert!(!sounds.poll(), "nothing has changed");

        fs::write(folder.join("sounds/bell.wav"), test_wav(&[1000; 960])).unwrap();
        sounds.next_look = Instant::now();
        assert!(sounds.poll());
        assert!(names(&sounds).contains(&"bell"));
        // The door was read and has not changed: it plays on, and only
        // the bell is waiting to be read.
        assert!(sounds.play(&request("door")).is_some());
        assert_eq!(sounds.waiting.len(), 1);
        assert!(sounds.load());
        assert_eq!(sounds.play(&request("bell")).unwrap().sound.seconds(), 0.02);
        assert!(sounds.play(&request("door")).is_some());

        fs::remove_file(folder.join("sounds/door.wav")).unwrap();
        sounds.next_look = Instant::now();
        assert!(sounds.poll());
        assert!(!names(&sounds).contains(&"door"));
        sounds.load();
        assert!(sounds.play(&request("door")).is_none());
        let _ = fs::remove_dir_all(folder);
    }

    /// A link could lead to any file on the computer, so it is not a sound.
    #[cfg(unix)]
    #[test]
    fn a_link_in_the_sounds_folder_is_left_alone() {
        let folder = game("link", &[("secret.wav", &test_wav(&[1000; 480]))]);
        fs::create_dir_all(folder.join("sounds")).unwrap();
        std::os::unix::fs::symlink(folder.join("secret.wav"), folder.join("sounds/secret.wav"))
            .unwrap();
        std::os::unix::fs::symlink(&folder, folder.join("sounds/up")).unwrap();
        let mut sounds = Sounds::in_folder(&folder);
        sounds.load();
        assert_eq!(names(&sounds), STARTER_SOUNDS);
        assert!(sounds.problems().is_empty());
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_packed_game_plays_the_sounds_in_its_pack() {
        use fossps_pack::Source;

        let folder = game(
            "pack",
            &[
                ("door.wav", &test_wav(&[1000; 480])),
                ("bad.wav", b"not a WAV file"),
                ("main.luau", b""),
            ],
        );
        let source = |name: &str, file: &str| Source {
            name: name.to_owned(),
            path: folder.join(file),
        };
        let out = folder.join("Game.fpk");
        fossps_pack::write(
            &out,
            "Game",
            &[
                source("main.luau", "main.luau"),
                source("sounds/doors/front.wav", "door.wav"),
                source("sounds/bad.wav", "bad.wav"),
                source("music/not-in-sounds.wav", "door.wav"),
            ],
        )
        .unwrap();
        let mut pack = Pack::open(&out).unwrap();
        let mut sounds = Sounds::in_pack(&mut pack);
        assert_eq!(sounds.own(), 1);
        assert_eq!(names(&sounds).len(), STARTER_SOUNDS.len() + 1);
        assert_eq!(
            sounds
                .play(&request("doors/front"))
                .unwrap()
                .sound
                .seconds(),
            0.01
        );
        assert_eq!(sounds.problems().len(), 1, "{:?}", sounds.problems());
        assert!(sounds.problems()[0].starts_with("sounds/bad.wav was not loaded."));
        let _ = fs::remove_dir_all(folder);
    }
}
