//! Playing a packaged game: finding the pack a program was shipped with and
//! opening the game inside it.
//!
//! `fossps package` puts a program and a pack side by side with the same
//! name, `Game` and `Game.fpk`. When the program starts and finds its pack
//! there, it plays that game instead of acting as the `fossps` command.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use fossps_input::ActionMap;
use fossps_pack::{EXTENSION, Pack, PackError};
use fossps_script::ENTRY_SCRIPT;

use crate::scaffold::LEVEL_FILE;
use crate::script_game::ScriptGame;

/// Where the pack for the program at `program` would be: beside it, with the
/// program's name and `.fpk` after it. A Windows program's `.exe` is left
/// out, so `Game.exe` looks for `Game.fpk`.
#[must_use]
pub fn pack_path(program: &Path) -> PathBuf {
    let is_exe = program
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    let mut name: OsString = if is_exe {
        program.file_stem()
    } else {
        program.file_name()
    }
    .unwrap_or_default()
    .to_owned();
    name.push(".");
    name.push(EXTENSION);
    program.with_file_name(name)
}

/// The pack shipped beside the program at `program`, if there is one.
#[must_use]
pub fn pack_beside(program: &Path) -> Option<PathBuf> {
    // A program with no name of its own has no pack named after it.
    program.file_name()?;
    Some(pack_path(program)).filter(|pack| pack.is_file())
}

/// Whether `path` is named like a game pack: it ends in `.fpk`.
#[must_use]
pub fn is_pack(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
}

/// A game opened from a pack.
#[derive(Debug)]
pub struct PackedGame {
    /// The game's name, as its maker wrote it. Empty if the pack has none.
    pub name: String,
    /// The game, running the pack's `main.luau`.
    pub game: ScriptGame,
    /// The pack's `level.glb` as it was packed, if the game has a level.
    pub level: Option<Vec<u8>>,
}

/// Why a packaged game could not be opened.
#[derive(Debug)]
pub enum PlayError {
    /// The pack could not be read.
    Pack(PackError),
    /// The pack holds no `main.luau`.
    NoScript {
        /// The pack.
        pack: PathBuf,
    },
    /// One of the pack's scripts has a mistake. The error itself was logged.
    ScriptMistake {
        /// The pack.
        pack: PathBuf,
        /// The script the mistake is in, such as `main.luau`.
        file: String,
    },
}

impl fmt::Display for PlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pack(error) => error.fmt(f),
            Self::NoScript { pack } => write!(
                f,
                "{} has no {ENTRY_SCRIPT} in it, so there is no game to play. If it is \
                 your game, package it again with `fossps package`.",
                pack.display()
            ),
            Self::ScriptMistake { pack, file } => write!(
                f,
                "The game in {} cannot start: its {file} has a mistake (the error \
                 is shown above). If it is your game, fix the script, try it with \
                 `fossps check`, then package it again. Otherwise, tell the game's maker.",
                pack.display()
            ),
        }
    }
}

impl std::error::Error for PlayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pack(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PackError> for PlayError {
    fn from(error: PackError) -> Self {
        Self::Pack(error)
    }
}

/// The ending of a script's file name.
const SCRIPT_ENDING: &str = ".luau";

/// Opens the game in the pack at `pack`. The script reads input through
/// `actions`, which should be the same actions the engine is started with.
///
/// The level is handed over as bytes and not read here, so a server can
/// open a pack without any level-reading code.
///
/// # Errors
///
/// Fails if the pack cannot be read, holds no `main.luau`, or its script has
/// a mistake: a packaged game has no file to fix and save, so a script that
/// does not load is not worth opening a window for.
pub fn open(pack: &Path, actions: &ActionMap) -> Result<PackedGame, PlayError> {
    let mut opened = Pack::open(pack)?;
    if !opened.contains(ENTRY_SCRIPT) {
        return Err(PlayError::NoScript {
            pack: pack.to_owned(),
        });
    }
    // Every script, so that `main.luau` can require the others.
    let names: Vec<String> = opened
        .files()
        .filter(|name| name.ends_with(SCRIPT_ENDING))
        .map(str::to_owned)
        .collect();
    let mut scripts = Vec::with_capacity(names.len());
    for name in names {
        let source = opened.read_text(&name)?;
        scripts.push((name, source));
    }
    let game = ScriptGame::from_sources(scripts, actions);
    if let Some(error) = game.host().last_error() {
        return Err(PlayError::ScriptMistake {
            pack: pack.to_owned(),
            file: error.file.clone(),
        });
    }
    let level = if opened.contains(LEVEL_FILE) {
        Some(opened.read(LEVEL_FILE)?)
    } else {
        None
    };
    Ok(PackedGame {
        name: opened.name().to_owned(),
        game,
        level,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fossps_pack::Source;

    use super::*;
    use crate::game::Game;

    /// An empty folder for one test.
    fn scratch(test: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("fossps-player-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A pack in `folder` holding `files` (name, contents).
    fn pack(folder: &Path, files: &[(&str, &str)]) -> PathBuf {
        let sources: Vec<Source> = files
            .iter()
            .map(|(name, contents)| {
                let path = folder.join(format!("source-{}", name.replace('/', "-")));
                fs::write(&path, contents).unwrap();
                Source {
                    name: (*name).to_owned(),
                    path,
                }
            })
            .collect();
        let out = folder.join("Game.fpk");
        fossps_pack::write(&out, "Zombie Night", &sources).unwrap();
        out
    }

    #[test]
    fn a_programs_pack_has_its_name() {
        for (program, pack) in [
            ("/games/Zombie/Zombie", "/games/Zombie/Zombie.fpk"),
            ("/games/Zombie/Zombie.exe", "/games/Zombie/Zombie.fpk"),
            ("/games/Zombie/Zombie.EXE", "/games/Zombie/Zombie.fpk"),
            // Only `.exe` is a program's ending; a version is part of the name.
            ("/games/Zombie-1.2", "/games/Zombie-1.2.fpk"),
            ("fossps", "fossps.fpk"),
        ] {
            assert_eq!(pack_path(Path::new(program)), Path::new(pack), "{program}");
        }
    }

    #[test]
    fn a_pack_is_found_only_when_it_is_beside_the_program() {
        let folder = scratch("beside");
        let program = folder.join("Game");
        assert_eq!(pack_beside(&program), None);

        fs::create_dir(folder.join("Other.fpk")).unwrap();
        assert_eq!(pack_beside(&folder.join("Other")), None, "a folder");

        let made = pack(&folder, &[("main.luau", "")]);
        assert_eq!(pack_beside(&program), Some(made));
        assert_eq!(pack_beside(Path::new("/")), None);
    }

    #[test]
    fn packs_are_told_by_their_ending() {
        assert!(is_pack(Path::new("dist/Game.fpk")));
        assert!(is_pack(Path::new("GAME.FPK")));
        assert!(!is_pack(Path::new("my-game")));
        assert!(!is_pack(Path::new("main.luau")));
    }

    #[test]
    fn a_packed_game_plays_its_script() {
        let folder = scratch("plays");
        let made = pack(
            &folder,
            &[(
                "main.luau",
                "game.mode { warmup = 0 }\ngame.onTick(function() end)",
            )],
        );
        let actions = ActionMap::default_fps();

        let mut packed = open(&made, &actions).unwrap();

        assert_eq!(packed.name, "Zombie Night");
        let mut input = fossps_input::Input::new(actions);
        packed.game.tick(input.tick(), 1.0 / 60.0);
        assert!(packed.game.host().last_error().is_none());
    }

    #[test]
    fn a_packed_game_hands_over_its_level_as_it_was_packed() {
        let folder = scratch("level");
        let without = pack(&folder, &[("main.luau", "")]);
        let packed = open(&without, &ActionMap::default()).unwrap();
        assert_eq!(packed.level, None);

        let with = pack(&folder, &[("main.luau", ""), ("level.glb", "the level")]);
        let packed = open(&with, &ActionMap::default()).unwrap();
        assert_eq!(packed.level.as_deref(), Some(b"the level".as_slice()));
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_packed_game_requires_its_other_scripts() {
        let folder = scratch("requires");
        let made = pack(
            &folder,
            &[
                (
                    "main.luau",
                    "local rifle = require(\"./weapons/rifle\")\nassert(rifle.damage == 30)",
                ),
                (
                    "weapons/rifle.luau",
                    "return { damage = require(\"../balance\").rifle }",
                ),
                ("balance.luau", "return { rifle = 30 }"),
                ("notes.txt", "not a script"),
            ],
        );
        let actions = ActionMap::default_fps();

        let packed = open(&made, &actions).unwrap();

        assert!(packed.game.host().last_error().is_none());

        // A mistake in a required script is put down to that script.
        let folder = scratch("requires-mistake");
        let made = pack(
            &folder,
            &[
                ("main.luau", "require(\"./weapons\")"),
                ("weapons.luau", "game.onTik()"),
            ],
        );
        let said = open(&made, &actions).unwrap_err().to_string();
        assert!(said.contains("its weapons.luau has a mistake"), "{said}");
    }

    #[test]
    fn a_pack_that_cannot_be_played_says_why() {
        let actions = ActionMap::default_fps();

        let folder = scratch("no-script");
        let said = open(&pack(&folder, &[("notes.txt", "hello")]), &actions)
            .unwrap_err()
            .to_string();
        assert!(said.contains("has no main.luau in it"), "{said}");

        let folder = scratch("mistake");
        let said = open(&pack(&folder, &[("main.luau", "game.onTik()")]), &actions)
            .unwrap_err()
            .to_string();
        assert!(said.contains("has a mistake"), "{said}");

        let folder = scratch("not-a-pack");
        fs::write(folder.join("Game.fpk"), "hello").unwrap();
        let said = open(&folder.join("Game.fpk"), &actions)
            .unwrap_err()
            .to_string();
        assert!(said.contains("is not a FOSSPS game pack"), "{said}");
    }
}
