//! The level a game is played in: the `level.glb` in its folder.
//!
//! A level is one glTF Binary file, which Blender and most other 3D tools
//! export, holding everything itself. `fossps new` puts a small one in a new
//! game; a game without the file is shown the engine's stand-in floor.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fossps_assets::{AssetError, LoadOptions, Scene};

pub use crate::scaffold::LEVEL_FILE;

/// What a level exported as separate files is called. It is not read, but
/// finding it says what went wrong.
const SEPARATE_FILE: &str = "level.gltf";

/// Why a game's level could not be loaded.
#[derive(Debug)]
pub enum LevelError {
    /// The level file is there but could not be read as a level.
    Load(AssetError),
    /// The level points to a file outside itself, such as a texture.
    NotOneFile {
        /// The level file.
        file: String,
        /// The other file, as written in the level file.
        uses: String,
    },
    /// The folder has a `level.gltf` and no `level.glb`.
    SeparateFiles {
        /// The `level.gltf` that was found.
        found: PathBuf,
    },
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => error.fmt(f),
            Self::NotOneFile { file, uses } => write!(
                f,
                "{file} uses the separate file \"{uses}\", but a level has to hold \
                 everything itself. Export it again as glTF Binary (.glb): in Blender, \
                 File > Export > glTF 2.0 with the format set to glTF Binary (.glb)."
            ),
            Self::SeparateFiles { found } => write!(
                f,
                "Found {}, but a game's level is one file called {LEVEL_FILE}. Export it \
                 again as glTF Binary (.glb): in Blender, File > Export > glTF 2.0 with the \
                 format set to glTF Binary (.glb), and save it as {LEVEL_FILE} in the same \
                 folder.",
                found.display()
            ),
        }
    }
}

impl std::error::Error for LevelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) => Some(error),
            _ => None,
        }
    }
}

/// Loads the level of the game in `folder`: its `level.glb`. `None` if the
/// game has no level of its own.
///
/// ```no_run
/// use fossps::Config;
///
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = Config {
///         level: fossps::level::load("my-game".as_ref())?,
///         ..Config::default()
///     };
///     # let _ = config;
///     Ok(())
/// }
/// ```
///
/// # Errors
///
/// Fails if the file is there but is not a level FOSSPS can read, or if the
/// level was exported as `level.gltf` with its data in separate files. The
/// message says what to change in the 3D tool.
pub fn load(folder: &Path) -> Result<Option<Scene>, LevelError> {
    let path = folder.join(LEVEL_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let separate = folder.join(SEPARATE_FILE);
            return if separate.is_file() {
                Err(LevelError::SeparateFiles { found: separate })
            } else {
                Ok(None)
            };
        }
        Err(source) => return Err(LevelError::Load(AssetError::Read { path, source })),
    };
    from_bytes(&path.display().to_string(), &bytes).map(Some)
}

/// Reads a level that is already in memory, as a packaged game's is. `name`
/// is what messages call the file.
///
/// # Errors
///
/// Fails if the bytes are not a level FOSSPS can read. The message says what
/// to change in the 3D tool.
pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Scene, LevelError> {
    // No folder to look in: a level that needs a second file would play
    // from a folder and then fail once the game is packaged.
    Scene::from_bytes(name, bytes, None, LoadOptions::default()).map_err(|error| match error {
        AssetError::NoFolder { file, uri } => LevelError::NotOneFile { file, uses: uri },
        other => LevelError::Load(other),
    })
}

/// A line about `level` for the person who made it: how much is in it.
#[must_use]
pub fn summary(level: &Scene) -> String {
    let triangles: usize = level
        .instances
        .iter()
        .filter_map(|instance| level.meshes.get(instance.mesh as usize))
        .flat_map(|mesh| &mesh.parts)
        .map(|part| part.indices.len() / 3)
        .sum();
    let count = |number: usize, one: &str, many: &str| {
        format!("{number} {}", if number == 1 { one } else { many })
    };
    format!(
        "{} placed, {}",
        count(level.instances.len(), "shape", "shapes"),
        count(triangles, "triangle", "triangles")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scaffold::STARTER_LEVEL;

    /// An empty folder for one test.
    fn scratch(test: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("fossps-level-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn a_folder_with_no_level_has_none() {
        let folder = scratch("none");
        assert_eq!(load(&folder).unwrap(), None);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_level_in_a_folder_is_loaded_and_summed_up() {
        let folder = scratch("starter");
        fs::write(folder.join(LEVEL_FILE), STARTER_LEVEL).unwrap();
        let level = load(&folder).unwrap().unwrap();
        assert_eq!(level.warnings, Vec::<String>::new());
        assert_eq!(summary(&level), "22 shapes placed, 744 triangles");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_that_is_not_a_level_says_how_to_export_one() {
        let folder = scratch("broken");
        fs::write(folder.join(LEVEL_FILE), "not a level").unwrap();
        let said = load(&folder).unwrap_err().to_string();
        assert!(said.contains("level.glb is not a glTF file"), "{said}");
        assert!(said.contains("glTF Binary (.glb)"), "{said}");

        // Cut short, as a file still being copied is.
        fs::write(folder.join(LEVEL_FILE), &STARTER_LEVEL[..2000]).unwrap();
        let said = load(&folder).unwrap_err().to_string();
        assert!(said.contains("level.glb"), "{said}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_level_exported_as_separate_files_says_to_export_one_file() {
        let folder = scratch("separate");
        fs::write(folder.join("level.gltf"), "{}").unwrap();
        let said = load(&folder).unwrap_err().to_string();
        assert!(
            said.contains("level.gltf, but a game's level is one file"),
            "{said}"
        );
        assert!(said.contains("save it as level.glb"), "{said}");
        // With both, the one file is the level.
        fs::write(folder.join(LEVEL_FILE), STARTER_LEVEL).unwrap();
        assert!(load(&folder).unwrap().is_some());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_level_that_needs_a_second_file_is_refused() {
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[]}],"buffers":[{"byteLength":4,"uri":"level.bin"}]}"#;
        let said = from_bytes("level.glb", json.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(
            said.starts_with("level.glb uses the separate file \"level.bin\""),
            "{said}"
        );
        assert!(said.contains("hold everything itself"), "{said}");
    }
}
