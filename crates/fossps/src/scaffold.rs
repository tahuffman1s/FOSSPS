//! Creates a new game folder from the built-in template.
//!
//! This is what `fossps new` runs. A new game is a `main.luau` that already
//! plays, a short README, and the files an editor needs for autocomplete.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use fossps_script::{ENTRY_SCRIPT, TYPE_DEFINITIONS};

/// The file in a game folder that describes the script API to editors.
pub const TYPES_FILE: &str = "fossps.d.luau";

/// The longest game name, in characters.
const MAX_NAME_LENGTH: usize = 64;

/// Where the template's game name goes.
const NAME_PLACEHOLDER: &str = "{name}";

const MAIN_SCRIPT: &str = include_str!("../template/main.luau");
const README: &str = include_str!("../template/README.md");
const EDITOR_SETTINGS: &str = include_str!("../template/vscode-settings.json");

/// A game folder that [`new_game`] just created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGame {
    /// The game's name: the last part of the folder's path.
    pub name: String,
    /// The folder the game is in, as a full path.
    pub folder: PathBuf,
}

/// Why [`new_game`] did not create a game.
#[derive(Debug)]
pub enum NewGameError {
    /// The folder's name cannot be used as a game name.
    InvalidName {
        /// The name that was asked for.
        name: String,
        /// What is wrong with it, as the end of a sentence.
        reason: String,
    },
    /// The folder already has something in it.
    NotEmpty {
        /// The folder.
        folder: PathBuf,
    },
    /// The path is taken by a file.
    NotAFolder {
        /// The path.
        path: PathBuf,
    },
    /// The operating system refused to create or write something.
    Io {
        /// What was being created.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
}

impl fmt::Display for NewGameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { name, reason } => write!(
                f,
                "\"{name}\" cannot be used as a game name: {reason}. \
                 Try a name like my-game."
            ),
            Self::NotEmpty { folder } => write!(
                f,
                "{} already has files in it, so nothing was changed. Pick a name that is \
                 not taken yet. If that folder is already a game, play it with \
                 `fossps run`.",
                folder.display()
            ),
            Self::NotAFolder { path } => write!(
                f,
                "{} is a file, not a folder, so no game was created. Pick a different name.",
                path.display()
            ),
            Self::Io { path, source } => write!(
                f,
                "Could not create {}: {source}. Check that you are allowed to write \
                 there and that the disk is not full.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for NewGameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Creates a game at `path` that plays as it is. The last part of the path
/// becomes the game's name. The folder may exist already if it is empty;
/// nothing is ever overwritten.
///
/// ```no_run
/// let game = fossps::scaffold::new_game("my-game".as_ref())?;
/// println!("Created {} in {}", game.name, game.folder.display());
/// # Ok::<(), fossps::scaffold::NewGameError>(())
/// ```
///
/// # Errors
///
/// Fails if the name cannot be used on every platform, if the folder already
/// has files in it, or if it cannot be written. A game that could only be
/// partly written is removed again.
pub fn new_game(path: &Path) -> Result<NewGame, NewGameError> {
    // The name is checked as it was typed where possible: making a reserved
    // Windows name such as `con` absolute turns it into a device path.
    if path.file_name().is_some() {
        game_name(path)?;
    }
    // Gives `.` a name: the folder it stands for. Reads nothing from disk.
    let folder = std::path::absolute(path).map_err(|source| NewGameError::Io {
        path: path.to_owned(),
        source,
    })?;
    let name = game_name(&folder)?;

    if folder.is_file() {
        return Err(NewGameError::NotAFolder { path: folder });
    }
    let existed = folder.is_dir();
    if existed {
        let mut entries = fs::read_dir(&folder).map_err(|source| NewGameError::Io {
            path: folder.clone(),
            source,
        })?;
        if entries.next().is_some() {
            return Err(NewGameError::NotEmpty { folder });
        }
    }

    let editor_folder = folder.join(".vscode");
    let files = [
        (
            folder.join(ENTRY_SCRIPT),
            MAIN_SCRIPT.replace(NAME_PLACEHOLDER, &name),
        ),
        (
            folder.join("README.md"),
            README.replace(NAME_PLACEHOLDER, &name),
        ),
        (folder.join(TYPES_FILE), TYPE_DEFINITIONS.to_owned()),
        (
            editor_folder.join("settings.json"),
            EDITOR_SETTINGS.to_owned(),
        ),
    ];

    let written = fs::create_dir_all(&editor_folder)
        .map_err(|source| NewGameError::Io {
            path: editor_folder.clone(),
            source,
        })
        .and_then(|()| {
            files
                .iter()
                .try_for_each(|(path, text)| write_new(path, text))
        });

    if let Err(error) = written {
        // Leave nothing half-made behind. Removing is best effort: the error
        // worth reporting is the one that stopped the game being created.
        for (path, _) in &files {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&editor_folder);
        if !existed {
            let _ = fs::remove_dir(&folder);
        }
        return Err(error);
    }

    Ok(NewGame { name, folder })
}

/// Brings a game folder's [`TYPES_FILE`] up to date with this version of the
/// engine, so editor autocomplete matches what scripts can really call.
/// Returns whether the file was rewritten.
///
/// A folder without the file is left alone: not every game wants one.
///
/// # Errors
///
/// Fails if the file exists but cannot be read or written.
pub fn refresh_types(folder: &Path) -> io::Result<bool> {
    let path = folder.join(TYPES_FILE);
    let current = match fs::read_to_string(&path) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    // Compared by lines so a checkout that changed the line endings still counts
    // as up to date.
    if current.lines().eq(TYPE_DEFINITIONS.lines()) {
        return Ok(false);
    }
    fs::write(&path, TYPE_DEFINITIONS)?;
    Ok(true)
}

/// Writes `text` to a file that must not exist yet.
fn write_new(path: &Path, text: &str) -> Result<(), NewGameError> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|source| NewGameError::Io {
            path: path.to_owned(),
            source,
        })
}

/// The game name a folder path gives, if it is one that works everywhere.
fn game_name(folder: &Path) -> Result<String, NewGameError> {
    let Some(name) = folder.file_name() else {
        return Err(NewGameError::InvalidName {
            name: folder.display().to_string(),
            reason: "it does not end in a folder name".to_owned(),
        });
    };
    let Some(name) = name.to_str() else {
        return Err(NewGameError::InvalidName {
            name: name.to_string_lossy().into_owned(),
            reason: "it has characters that are not readable text".to_owned(),
        });
    };
    match name_problem(name) {
        Some(reason) => Err(NewGameError::InvalidName {
            name: name.to_owned(),
            reason,
        }),
        None => Ok(name.to_owned()),
    }
}

/// What stops `name` from being a folder name on Linux, Windows and macOS
/// alike, so a game made on one can be copied to the others.
fn name_problem(name: &str) -> Option<String> {
    const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];

    if name.is_empty() {
        return Some("it is empty".to_owned());
    }
    if name.chars().count() > MAX_NAME_LENGTH {
        return Some(format!("it is longer than {MAX_NAME_LENGTH} characters"));
    }
    if name.starts_with(['-', '.']) {
        return Some("it starts with a dash or a dot".to_owned());
    }
    if name.starts_with(' ') || name.ends_with([' ', '.']) {
        return Some("it starts with a space, or ends with a space or a dot".to_owned());
    }
    if let Some(bad) = name
        .chars()
        .find(|c| FORBIDDEN.contains(c) || c.is_control())
    {
        return Some(if bad.is_control() {
            "it has a character that cannot be typed, such as a tab or a line break".to_owned()
        } else {
            format!("it has a {bad} in it, which Windows does not allow in folder names")
        });
    }
    // Windows reserves these with any extension too: `nul.txt` is still `nul`.
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if RESERVED.contains(&stem.to_ascii_lowercase().as_str()) {
        return Some(format!("{stem} is a name Windows keeps for itself"));
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use fossps_input::ActionMap;
    use fossps_script::ScriptHost;

    use super::*;

    /// A folder for one test, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let folder = std::env::temp_dir().join(format!(
                "fossps-scaffold-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&folder).unwrap();
            Self(folder)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creates_every_file_with_the_name_filled_in() {
        let scratch = Scratch::new();
        let game = new_game(&scratch.path("my-game")).unwrap();

        assert_eq!(game.name, "my-game");
        assert_eq!(game.folder, scratch.path("my-game"));
        let script = fs::read_to_string(game.folder.join("main.luau")).unwrap();
        assert!(script.contains("Welcome to my-game!"));
        assert!(!script.contains(NAME_PLACEHOLDER));
        let readme = fs::read_to_string(game.folder.join("README.md")).unwrap();
        assert!(readme.starts_with("# my-game\n") || readme.starts_with("# my-game\r\n"));
        assert_eq!(
            fs::read_to_string(game.folder.join(TYPES_FILE)).unwrap(),
            TYPE_DEFINITIONS
        );
        let settings = fs::read_to_string(game.folder.join(".vscode/settings.json")).unwrap();
        assert!(settings.contains(&format!("\"fossps\": \"{TYPES_FILE}\"")));
    }

    #[test]
    fn the_new_game_runs_without_a_script_error() {
        let scratch = Scratch::new();
        let game = new_game(&scratch.path("Zombie Night 2")).unwrap();

        let host = ScriptHost::open(&game.folder, ActionMap::default()).unwrap();
        assert_eq!(host.last_error(), None);
    }

    #[test]
    fn the_example_is_the_template() {
        let example = include_str!("../../../examples/hello/main.luau");
        assert_eq!(MAIN_SCRIPT.replace(NAME_PLACEHOLDER, "hello"), example);
    }

    #[test]
    fn an_empty_folder_and_missing_parents_are_fine() {
        let scratch = Scratch::new();
        fs::create_dir(scratch.path("empty")).unwrap();
        new_game(&scratch.path("empty")).unwrap();
        new_game(&scratch.path("a/b/deep")).unwrap();

        assert!(scratch.path("empty/main.luau").is_file());
        assert!(scratch.path("a/b/deep/main.luau").is_file());
    }

    #[test]
    fn a_folder_with_files_is_left_untouched() {
        let scratch = Scratch::new();
        fs::create_dir(scratch.path("taken")).unwrap();
        fs::write(scratch.path("taken/main.luau"), "mine").unwrap();

        let error = new_game(&scratch.path("taken")).unwrap_err();

        assert!(matches!(error, NewGameError::NotEmpty { .. }), "{error}");
        assert_eq!(
            fs::read_to_string(scratch.path("taken/main.luau")).unwrap(),
            "mine"
        );
        assert_eq!(fs::read_dir(scratch.path("taken")).unwrap().count(), 1);
    }

    #[test]
    fn a_file_in_the_way_is_reported() {
        let scratch = Scratch::new();
        fs::write(scratch.path("notes"), "text").unwrap();

        let error = new_game(&scratch.path("notes")).unwrap_err();

        assert!(matches!(error, NewGameError::NotAFolder { .. }), "{error}");
    }

    #[test]
    fn names_that_work_everywhere_are_accepted() {
        for name in ["my-game", "Zombie Night 2", "héros", "v1.2", "console", "a"] {
            assert_eq!(name_problem(name), None, "{name}");
        }
    }

    #[test]
    fn names_that_break_somewhere_are_refused() {
        let long = "x".repeat(MAX_NAME_LENGTH + 1);
        for name in [
            "", "-rf", ".hidden", " lead", "trail ", "dot.", "a:b", "a\"b", "a\\b", "what?",
            "a\tb", "CON", "nul.txt", "Lpt1", &long,
        ] {
            assert!(name_problem(name).is_some(), "{name:?}");
        }
    }

    #[test]
    fn a_path_without_a_name_is_refused() {
        let error = new_game(Path::new("..")).unwrap_err();
        assert!(matches!(error, NewGameError::InvalidName { .. }), "{error}");
    }

    #[test]
    fn stale_types_are_refreshed_and_missing_ones_are_not_created() {
        let scratch = Scratch::new();
        let game = new_game(&scratch.path("game")).unwrap();
        let types = game.folder.join(TYPES_FILE);

        assert!(!refresh_types(&game.folder).unwrap());
        fs::write(&types, TYPE_DEFINITIONS.replace('\n', "\r\n")).unwrap();
        assert!(!refresh_types(&game.folder).unwrap());

        fs::write(&types, "-- from an older engine").unwrap();
        assert!(refresh_types(&game.folder).unwrap());
        assert_eq!(fs::read_to_string(&types).unwrap(), TYPE_DEFINITIONS);

        fs::remove_file(&types).unwrap();
        assert!(!refresh_types(&game.folder).unwrap());
        assert!(!types.exists());
    }
}
