//! What can go wrong reading a level or model, worded for the person who made it.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Why a level or model could not be read.
///
/// The message names the file, says what is wrong with it and what to change.
#[derive(Debug)]
#[non_exhaustive]
pub enum AssetError {
    /// The file could not be opened or read.
    Read {
        /// The file that was being read.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// The file is not glTF, or is cut short.
    NotGltf {
        /// The level or model file.
        file: String,
        /// What the glTF reader reported.
        source: gltf::Error,
    },
    /// The file is glTF but breaks the format's rules.
    Invalid {
        /// The level or model file.
        file: String,
        /// Each broken rule, as "where: what".
        problems: Vec<String>,
    },
    /// The file needs a glTF extension FOSSPS does not read.
    UnsupportedExtension {
        /// The level or model file.
        file: String,
        /// The extension's name, such as `KHR_draco_mesh_compression`.
        extension: String,
    },
    /// A file this one points to is not there.
    MissingFile {
        /// The level or model file.
        file: String,
        /// The missing file, as written in the level or model file.
        uri: String,
        /// Where it was looked for.
        path: PathBuf,
        /// Whether the missing file is a picture rather than shape data.
        texture: bool,
    },
    /// The file points to another file, but was read from memory with no folder to look in.
    NoFolder {
        /// The level or model file.
        file: String,
        /// The other file, as written in the level or model file.
        uri: String,
    },
    /// The file points to something outside its own folder.
    OutsideFolder {
        /// The level or model file.
        file: String,
        /// The address, as written in the level or model file.
        uri: String,
    },
    /// The file's data does not match what the file says about it.
    Damaged {
        /// The level or model file.
        file: String,
        /// What does not match.
        problem: String,
    },
    /// A picture in the file could not be decoded.
    BadImage {
        /// The level or model file.
        file: String,
        /// The picture's name.
        image: String,
        /// What the decoder reported.
        problem: String,
    },
    /// The file has nothing placed in it.
    NoScene {
        /// The level or model file.
        file: String,
    },
}

impl AssetError {
    /// A [`Damaged`](Self::Damaged) error for `file`.
    pub(crate) fn damaged(file: &str, problem: impl Into<String>) -> Self {
        Self::Damaged {
            file: file.to_owned(),
            problem: problem.into(),
        }
    }
}

/// What to change in the 3D tool so a file no longer needs `extension`.
fn extension_fix(extension: &str) -> &'static str {
    match extension {
        "KHR_draco_mesh_compression" => {
            "Export it again without Draco compression: in Blender's glTF export \
             options, open Data > Compression and untick it."
        }
        "EXT_meshopt_compression" | "KHR_mesh_quantization" => {
            "Export it again without mesh compression or quantization \
             (with gltfpack, leave out `-c`, `-cc` and add `-noq`)."
        }
        "KHR_texture_basisu" => "Export it again with PNG or JPEG textures instead of KTX2/Basis.",
        "EXT_texture_webp" | "EXT_texture_avif" | "MSFT_texture_dds" => {
            "Export it again with PNG or JPEG textures: in Blender's glTF export \
             options, set Material > Images to Automatic."
        }
        _ => "Export it again without that extension.",
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } if source.kind() == io::ErrorKind::NotFound => write!(
                f,
                "Could not find {}. Check the name and that the file is in that folder.",
                path.display()
            ),
            Self::Read { path, source } => {
                write!(f, "Could not read {}: {source}.", path.display())
            }
            Self::NotGltf { file, source } => write!(
                f,
                "{file} is not a glTF file FOSSPS can read ({source}). \
                 Export the level or model as glTF 2.0: in Blender, \
                 File > Export > glTF 2.0 with the format set to glTF Binary (.glb)."
            ),
            Self::Invalid { file, problems } => {
                write!(f, "{file} breaks the glTF format's rules:")?;
                for problem in problems {
                    write!(f, "\n  - {problem}")?;
                }
                write!(
                    f,
                    "\nExport it again from your 3D tool. If it still fails, \
                     https://github.khronos.org/glTF-Validator/ shows what is wrong with the file."
                )
            }
            Self::UnsupportedExtension { file, extension } => write!(
                f,
                "{file} needs the glTF extension {extension}, which FOSSPS does not read. {}",
                extension_fix(extension)
            ),
            Self::MissingFile {
                file,
                uri,
                path,
                texture,
            } => {
                let what = if *texture {
                    "the texture"
                } else {
                    "the data file"
                };
                write!(
                    f,
                    "{file} uses {what} \"{uri}\", but {} does not exist. \
                     Copy it there, or export as glTF Binary (.glb), which keeps everything in one file.",
                    path.display()
                )
            }
            Self::NoFolder { file, uri } => write!(
                f,
                "{file} uses the separate file \"{uri}\", but was read from memory, \
                 so there is no folder to look in. Say which folder the file came from, \
                 or export as glTF Binary (.glb), which keeps everything in one file."
            ),
            Self::OutsideFolder { file, uri } => write!(
                f,
                "{file} points to \"{uri}\", which is not inside the file's own folder. \
                 FOSSPS only reads files next to the level or model (or in folders below it): \
                 move the file there and export again, or export as glTF Binary (.glb)."
            ),
            Self::Damaged { file, problem } => write!(
                f,
                "{file} is damaged: {problem}. Export it again from your 3D tool; \
                 if it was copied or downloaded, the copy may be incomplete."
            ),
            Self::BadImage {
                file,
                image,
                problem,
            } => write!(
                f,
                "Could not read the texture \"{image}\" in {file}: {problem}. \
                 FOSSPS reads PNG and JPEG textures: save the texture as one of those and export again."
            ),
            Self::NoScene { file } => write!(
                f,
                "{file} has no scene in it, so there is nothing to show. \
                 In your 3D tool, make sure the objects are in the scene before exporting \
                 (in Blender, untick Include > Limit to > Selected Objects)."
            ),
        }
    }
}

impl std::error::Error for AssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::NotGltf { source, .. } => Some(source),
            _ => None,
        }
    }
}
