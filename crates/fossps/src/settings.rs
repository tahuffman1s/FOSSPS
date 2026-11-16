//! The player's own settings: how loud, how fast the view turns, which way
//! up, and how big the HUD and menus are. One file on their computer that
//! every game played with FOSSPS reads when it starts, and that the pause
//! menu's settings screen writes.
//!
//! The file is `settings.toml`, beside the bindings file (see
//! [`controls`](crate::controls)): in the `fossps` folder of the place each
//! system keeps settings. Setting `FOSSPS_SETTINGS` to a file's path uses
//! that file instead.
//!
//! ```
//! use fossps::settings::Settings;
//!
//! let (settings, problems) = Settings::read("volume = 40\ninvert_look = true\n");
//! assert!(problems.is_empty());
//! assert_eq!(settings.volume, 40.0);
//! assert!(settings.invert_look);
//! // What the file leaves out keeps its usual value.
//! assert_eq!(settings.sensitivity, 1.0);
//! ```

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml::de::{DeTable, DeValue};

/// Names another settings file to use than the usual one.
pub const FILE_VARIABLE: &str = "FOSSPS_SETTINGS";

/// A setting that is a number: the lowest and highest it can be, and how
/// far one press of left or right moves it in the settings menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    /// The lowest value.
    pub lowest: f32,
    /// The highest value.
    pub highest: f32,
    /// One step in the settings menu.
    pub step: f32,
}

impl Range {
    /// The value in the range nearest to `value`, on a whole step.
    fn nearest(self, value: f32) -> f32 {
        let steps = ((value - self.lowest) / self.step).round();
        (self.lowest + steps * self.step).clamp(self.lowest, self.highest)
    }
}

/// What a player has chosen. `Settings::default()` is what a new player gets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// How loud everything is, from 0 (silent) to 100 (full).
    pub volume: f32,
    /// How fast the view turns with the mouse and the stick: 1 is the
    /// game's own speed, 2 is twice as fast. From 0.1 to 3.
    pub sensitivity: f32,
    /// Whether pushing the mouse or stick up looks down.
    pub invert_look: bool,
    /// How big the HUD and menus are, in percent, from 75 to 150.
    pub ui_size: f32,
}

impl Default for Settings {
    /// Full volume, the game's own turning speed, up is up, and the HUD
    /// and menus at their designed size.
    fn default() -> Self {
        Self {
            volume: 100.0,
            sensitivity: 1.0,
            invert_look: false,
            ui_size: 100.0,
        }
    }
}

impl Settings {
    /// What [`volume`](Self::volume) can be.
    pub const VOLUME: Range = Range {
        lowest: 0.0,
        highest: 100.0,
        step: 5.0,
    };
    /// What [`sensitivity`](Self::sensitivity) can be.
    pub const SENSITIVITY: Range = Range {
        lowest: 0.1,
        highest: 3.0,
        step: 0.1,
    };
    /// What [`ui_size`](Self::ui_size) can be.
    pub const UI_SIZE: Range = Range {
        lowest: 75.0,
        highest: 150.0,
        step: 5.0,
    };

    /// What sound is multiplied by, from 0 to 1.
    ///
    /// Ears hear loudness in ratios, so the slider's number is squared:
    /// half way along sounds about half as loud, where half the signal
    /// would sound only a little quieter.
    #[must_use]
    pub fn gain(&self) -> f32 {
        let share = (self.volume / 100.0).clamp(0.0, 1.0);
        share * share
    }

    /// What the `look` action is multiplied by, sideways and then up and
    /// down, as `ActionMap::set_sensitivity` takes it.
    #[must_use]
    pub fn look(&self) -> [f32; 2] {
        let along = if self.invert_look { -1.0 } else { 1.0 };
        [self.sensitivity, self.sensitivity * along]
    }

    /// What HUD and menu sizes are multiplied by: 1 is the designed size.
    #[must_use]
    pub fn ui_scale(&self) -> f32 {
        self.ui_size / 100.0
    }

    /// Reads the text of a settings file. Returns the settings, and what
    /// the player should be told about the file; empty when all is well.
    ///
    /// A player's file never stops a game from starting: a line that
    /// cannot be used is left out and reported, and that setting keeps its
    /// usual value. A number outside its range becomes the nearest one
    /// allowed.
    #[must_use]
    pub fn read(text: &str) -> (Self, Vec<String>) {
        let mut settings = Self::default();
        let document = match DeTable::parse(text) {
            Ok(document) => document,
            Err(error) => {
                let line = line_of(text, error.span().map_or(0, |span| span.start));
                return (
                    settings,
                    vec![format!(
                        "line {line}: {}. None of the file was used, so the usual settings \
                         are in place.",
                        error.message()
                    )],
                );
            }
        };
        let mut problems: Vec<(usize, String)> = Vec::new();
        for (key, value) in document.get_ref() {
            let line = line_of(text, key.span().start);
            let name = key.get_ref().as_ref();
            let value = value.get_ref();
            let said = match name {
                "volume" => number(name, value, Self::VOLUME, &mut settings.volume),
                "sensitivity" => number(name, value, Self::SENSITIVITY, &mut settings.sensitivity),
                "ui_size" => number(name, value, Self::UI_SIZE, &mut settings.ui_size),
                "invert_look" => match value {
                    DeValue::Boolean(flag) => {
                        settings.invert_look = *flag;
                        None
                    }
                    _ => {
                        Some("`invert_look` must be `true` or `false`, so it stays `false`.".into())
                    }
                },
                _ => Some(format!(
                    "`{name}` is not a setting, so the line was skipped. The settings are \
                     `volume`, `sensitivity`, `invert_look` and `ui_size`."
                )),
            };
            if let Some(said) = said {
                problems.push((line, said));
            }
        }
        // The reader hands lines over by name; a player reads top to bottom.
        problems.sort_by_key(|(line, _)| *line);
        let problems = problems
            .into_iter()
            .map(|(line, said)| format!("line {line}: {said}"))
            .collect();
        (settings, problems)
    }

    /// The text of a settings file holding these settings.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::from(
            "# Your settings for every game played with FOSSPS.\n\
             # The pause menu's settings screen writes this file. You can change a line\n\
             # here too, and save: games read it when they start.\n\n",
        );
        // Writing to a `String` cannot fail.
        let _ = writeln!(
            text,
            "# How loud everything is, from 0 (silent) to 100.\nvolume = {}\n",
            self.volume.round()
        );
        let _ = writeln!(
            text,
            "# How fast the view turns: 1.0 is the game's own speed. From 0.1 to 3.0.\n\
             sensitivity = {:.1}\n",
            self.sensitivity
        );
        let _ = writeln!(
            text,
            "# true makes pushing the mouse or stick up look down.\ninvert_look = {}\n",
            self.invert_look
        );
        let _ = writeln!(
            text,
            "# How big the HUD and menus are, in percent, from 75 to 150.\nui_size = {}",
            self.ui_size.round()
        );
        text
    }
}

/// Where the player's settings file is kept, whether or not it exists yet.
///
/// `None` when the computer does not say where settings go (no home folder).
#[must_use]
pub fn file() -> Option<PathBuf> {
    crate::controls::player_file(FILE_VARIABLE, "settings.toml")
}

/// The settings in the file at `path`, and what the player should be told
/// about the file; empty when all is well.
///
/// A file that is not there gives the usual settings and is not a problem:
/// the player has not changed anything yet.
#[must_use]
pub fn load(path: &Path) -> (Settings, Vec<String>) {
    match fs::read_to_string(path) {
        Ok(text) => {
            let (settings, problems) = Settings::read(&text);
            let named = problems
                .iter()
                .map(|problem| format!("{}, {problem}", path.display()))
                .collect();
            (settings, named)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Settings::default(), Vec::new()),
        Err(error) => (
            Settings::default(),
            vec![format!(
                "Your settings in {} could not be read ({error}), so the usual settings are \
                 used. Check that the file is yours to read and is plain text.",
                path.display()
            )],
        ),
    }
}

/// Writes `settings` to the file at `path`, making its folder if needed.
///
/// The file is written beside itself and then moved into place, so a crash
/// or a full disk never leaves half a file where the player's settings were.
///
/// # Errors
///
/// Fails if the folder cannot be made or the file cannot be written.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    crate::controls::write_whole(path, &settings.to_text())
}

/// Reads a number setting into `into`, and says what the player should
/// know if it could not be used as written.
fn number(name: &str, value: &DeValue<'_>, range: Range, into: &mut f32) -> Option<String> {
    let read = match value {
        DeValue::Integer(whole) => i64::from_str_radix(whole.as_str(), whole.radix())
            .ok()
            // Any whole number a setting has a use for is exact as `f32`.
            .map(|whole| {
                #[allow(clippy::cast_precision_loss)]
                let read = whole as f32;
                read
            }),
        DeValue::Float(float) => float.as_str().parse::<f32>().ok(),
        _ => None,
    };
    let Range {
        lowest, highest, ..
    } = range;
    let Some(read) = read.filter(|read| read.is_finite()) else {
        return Some(format!(
            "`{name}` must be a number from {lowest} to {highest}. It keeps its usual \
             value, {into}."
        ));
    };
    *into = range.nearest(read);
    (read < lowest || read > highest).then(|| {
        format!("`{name}` goes from {lowest} to {highest}, so {read} was changed to {into}.")
    })
}

/// The line, counted from 1, of the byte at `offset`.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset).unwrap_or(text).matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_player_gets_the_games_own_feel() {
        let settings = Settings::default();
        assert_eq!(settings.gain(), 1.0);
        assert_eq!(settings.look(), [1.0, 1.0]);
        assert_eq!(settings.ui_scale(), 1.0);
        assert_eq!(Settings::read(""), (settings, Vec::new()));
    }

    #[test]
    fn what_is_written_is_read_back_the_same() {
        let chosen = Settings {
            volume: 35.0,
            sensitivity: 1.7,
            invert_look: true,
            ui_size: 125.0,
        };
        let text = chosen.to_text();
        let (read, problems) = Settings::read(&text);
        assert_eq!(problems, Vec::<String>::new());
        assert!((read.sensitivity - 1.7).abs() < 1e-6);
        assert_eq!(
            Settings {
                sensitivity: 1.7,
                ..read
            },
            chosen
        );
        assert_eq!(read.look()[1], -read.sensitivity);
        assert_eq!(read.ui_scale(), 1.25);
        // Half way along the slider is a quarter of the signal.
        let half = Settings {
            volume: 50.0,
            ..chosen
        };
        assert_eq!(half.gain(), 0.25);
    }

    #[test]
    fn a_line_that_cannot_be_used_is_reported_and_the_rest_still_counts() {
        let (read, problems) = Settings::read(
            "volume = \"loud\"\nsensitivity = 9\nfov = 100\ninvert_look = 1\nui_size = 90\n",
        );
        assert_eq!(
            read,
            Settings {
                sensitivity: 3.0,
                ui_size: 90.0,
                ..Settings::default()
            }
        );
        assert_eq!(
            problems,
            [
                "line 1: `volume` must be a number from 0 to 100. It keeps its usual value, 100.",
                "line 2: `sensitivity` goes from 0.1 to 3, so 9 was changed to 3.",
                "line 3: `fov` is not a setting, so the line was skipped. The settings are \
                 `volume`, `sensitivity`, `invert_look` and `ui_size`.",
                "line 4: `invert_look` must be `true` or `false`, so it stays `false`.",
            ]
        );
    }

    #[test]
    fn a_file_that_is_not_toml_changes_nothing_and_says_where() {
        let (read, problems) = Settings::read("volume = 40\nsensitivity = = 2\n");
        assert_eq!(read, Settings::default());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("line 2: "), "{}", problems[0]);
        assert!(problems[0].ends_with("the usual settings are in place."));
    }

    #[test]
    fn settings_are_saved_to_a_file_and_loaded_from_it() {
        let folder =
            std::env::temp_dir().join(format!("fossps-settings-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let path = folder.join("deeper").join("settings.toml");

        // No file yet: nothing to say.
        assert_eq!(load(&path), (Settings::default(), Vec::new()));

        let chosen = Settings {
            volume: 0.0,
            invert_look: true,
            ..Settings::default()
        };
        save(&path, &chosen).unwrap();
        assert_eq!(load(&path), (chosen, Vec::new()));
        assert!(!path.with_extension("toml.part").exists());

        fs::write(&path, "volume = 500\n").unwrap();
        let (read, problems) = load(&path);
        assert_eq!(read.volume, 100.0);
        assert_eq!(
            problems,
            [format!(
                "{}, line 1: `volume` goes from 0 to 100, so 500 was changed to 100.",
                path.display()
            )]
        );
        let _ = fs::remove_dir_all(&folder);
    }
}
