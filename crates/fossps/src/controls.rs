//! The player's own controls: one bindings file on their computer that every
//! game played with FOSSPS reads when it starts.
//!
//! The file is `bindings.toml` in the `fossps` folder of the place each
//! system keeps settings: `~/.config` on Linux (or `XDG_CONFIG_HOME`),
//! `%APPDATA%` on Windows, `~/Library/Application Support` on macOS. Setting
//! `FOSSPS_BINDINGS` to a file's path uses that file instead.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fossps_input::ActionMap;

/// Names another bindings file to use than the usual one.
pub const FILE_VARIABLE: &str = "FOSSPS_BINDINGS";

/// Where the player's bindings file is kept, whether or not it exists yet.
///
/// `None` when the computer does not say where settings go (no home folder).
#[must_use]
pub fn file() -> Option<PathBuf> {
    player_file(FILE_VARIABLE, "bindings.toml")
}

/// Where a file of the player's own is kept: the file the environment
/// variable `variable` names, or else `name` in the `fossps` folder of the
/// place this system keeps settings.
pub(crate) fn player_file(variable: &str, name: &str) -> Option<PathBuf> {
    let set = |name: &str| env::var_os(name).filter(|value| !value.is_empty());
    if let Some(path) = set(variable) {
        return Some(PathBuf::from(path));
    }
    let settings = if cfg!(target_os = "windows") {
        PathBuf::from(set("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(set("HOME")?).join("Library/Application Support")
    } else {
        // The XDG rules: a path that is not absolute is to be ignored.
        match set("XDG_CONFIG_HOME").map(PathBuf::from) {
            Some(folder) if folder.is_absolute() => folder,
            _ => PathBuf::from(set("HOME")?).join(".config"),
        }
    };
    Some(settings.join("fossps").join(name))
}

/// Gives `actions` the bindings in the file at `path`, and returns what the
/// player should be told: every part of the file that was not used, then
/// every key or button that now does two things. Empty when all is well.
///
/// A file that is not there changes nothing and is not a problem: the player
/// has not changed their controls. No problem stops a game from starting.
#[must_use]
pub fn load(path: &Path, actions: &mut ActionMap) -> Vec<String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            return vec![format!(
                "Your controls in {} could not be read ({error}), so the game's own controls \
                 are used. Check that the file is yours to read and is plain text.",
                path.display()
            )];
        }
    };
    let mut said = match actions.read_bindings(&text) {
        Ok(problems) => problems
            .iter()
            .map(|problem| format!("{}: {problem}", path.display()))
            .collect(),
        Err(error) => vec![format!(
            "{}: {error} None of the file was used, so the game's own controls are in place.",
            path.display()
        )],
    };
    said.extend(actions.conflicts().iter().map(ToString::to_string));
    said
}

/// Writes the bindings file for `actions` to `path`, making its folder if
/// needed. Actions bound as in `defaults` are written as comments.
///
/// The file is written beside itself and then moved into place, so a crash
/// or a full disk never leaves half a file where the player's controls were.
///
/// # Errors
///
/// Fails if the folder cannot be made or the file cannot be written.
pub fn save(path: &Path, actions: &ActionMap, defaults: &ActionMap) -> io::Result<()> {
    write_whole(path, &actions.write_bindings(defaults))
}

/// Writes `text` to the file at `path`, making its folder if needed. The
/// file is written beside itself and then moved into place, so there is
/// never half a file where the old one was.
pub(crate) fn write_whole(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        fs::create_dir_all(folder)?;
    }
    let mut beside = path.as_os_str().to_owned();
    beside.push(".part");
    let beside = PathBuf::from(beside);
    fs::write(&beside, text)
        .and_then(|()| fs::rename(&beside, path))
        .inspect_err(|_| {
            let _ = fs::remove_file(&beside);
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(test: &str) -> PathBuf {
        let folder = env::temp_dir().join(format!("fossps-controls-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn a_missing_file_is_not_a_problem() {
        let mut actions = ActionMap::default_fps();
        let said = load(&scratch("missing").join("bindings.toml"), &mut actions);
        assert_eq!(said, Vec::<String>::new());
        assert_eq!(actions, ActionMap::default_fps());
    }

    #[test]
    fn saved_controls_are_loaded_again() {
        let path = scratch("saved").join("deeper").join("bindings.toml");
        let defaults = ActionMap::default_fps();
        let mut changed = defaults.clone();
        changed
            .read_bindings("[bindings]\njump = [\"F\", \"GamepadSouth\"]")
            .unwrap();

        save(&path, &changed, &defaults).unwrap();
        save(&path, &changed, &defaults).unwrap();
        let mut loaded = defaults.clone();
        assert_eq!(load(&path, &mut loaded), Vec::<String>::new());

        assert_eq!(loaded, changed);
        let left: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["bindings.toml"], "no half-written file stays");
    }

    #[test]
    fn problems_name_the_file_and_shared_keys_are_pointed_out() {
        let path = scratch("problems").join("bindings.toml");
        fs::write(
            &path,
            "[bindings]\njmup = [\"F\"]\nreload = [\"E\", \"GamepadWest\"]\n",
        )
        .unwrap();
        let mut actions = ActionMap::default_fps();

        let said = load(&path, &mut actions);

        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].starts_with(&format!("{}: Line 2: \"jmup\"", path.display())),
            "{said:?}"
        );
        assert!(
            said[1].starts_with("\"E\" is bound to both \"reload\" and \"interact\""),
            "{said:?}"
        );
    }

    #[test]
    fn a_file_that_is_not_toml_leaves_the_games_own_controls() {
        let path = scratch("broken").join("bindings.toml");
        fs::write(&path, "[bindings\njump = [\"F\"]\n").unwrap();
        let mut actions = ActionMap::default_fps();

        let said = load(&path, &mut actions);

        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("bindings.toml: Line 1, column "),
            "{said:?}"
        );
        assert!(said[0].ends_with("the game's own controls are in place."));
        assert_eq!(actions, ActionMap::default_fps());
    }
}
