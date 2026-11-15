//! Pack errors written for the person who has to fix them.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::FORMAT_VERSION;

/// Why a pack could not be written or read.
#[derive(Debug)]
pub enum PackError {
    /// A file could not be read from disk: the pack itself, or a file going
    /// into one.
    Read {
        /// The file.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// The pack could not be written.
    Write {
        /// The pack.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A file could not be copied into the pack.
    Copy {
        /// The file being copied.
        from: PathBuf,
        /// The pack.
        to: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A file changed size while it was being copied into the pack.
    Changed {
        /// The file.
        path: PathBuf,
    },
    /// A name cannot be stored in a pack.
    BadName {
        /// The name.
        name: String,
        /// What is wrong with it, as the end of a sentence.
        reason: &'static str,
    },
    /// Two files were given the same name.
    Duplicate {
        /// The name.
        name: String,
    },
    /// More files than a pack can list.
    TooManyFiles,
    /// The file is not a pack at all.
    NotAPack {
        /// The file.
        path: PathBuf,
    },
    /// The pack was written in a layout this version does not read.
    OtherVersion {
        /// The pack.
        path: PathBuf,
        /// The layout version it was written in.
        version: u32,
    },
    /// The pack is cut short or its contents contradict each other.
    Damaged {
        /// The pack.
        path: PathBuf,
        /// What is wrong with it, as the end of a sentence.
        what: &'static str,
    },
    /// The pack has no file of that name.
    NoSuchFile {
        /// The pack.
        path: PathBuf,
        /// The name that was asked for.
        name: String,
    },
    /// A file in the pack was read as text and is not UTF-8.
    NotText {
        /// The pack.
        path: PathBuf,
        /// The file in it.
        name: String,
    },
}

impl PackError {
    pub(crate) fn read(path: &Path, source: io::Error) -> Self {
        Self::Read {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(
                f,
                "Could not read {}: {source}. Check that the file exists and that you are \
                 allowed to read it.",
                path.display()
            ),
            Self::Write { path, source } => write!(
                f,
                "Could not write {}: {source}. Check that you are allowed to write there \
                 and that the disk is not full.",
                path.display()
            ),
            Self::Copy { from, to, source } => write!(
                f,
                "Could not copy {} into {}: {source}. Check that the file can be read and \
                 that the disk is not full.",
                from.display(),
                to.display()
            ),
            Self::Changed { path } => write!(
                f,
                "{} changed while it was being packed. Wait until nothing is saving it \
                 (an editor or an export, for example), then package the game again.",
                path.display()
            ),
            Self::BadName { name, reason } => write!(
                f,
                "\"{name}\" cannot be a name in a game pack: {reason}. Rename it, then \
                 package the game again."
            ),
            Self::Duplicate { name } => write!(
                f,
                "Two files would both be called \"{name}\" in the game pack. Rename one \
                 of them, then package the game again."
            ),
            Self::TooManyFiles => f.write_str(
                "The game has more files than a game pack can list (about four thousand \
                 million). Remove files the game does not use, then package it again.",
            ),
            Self::NotAPack { path } => write!(
                f,
                "{} is not a FOSSPS game pack. A game pack is the .fpk file that \
                 `fossps package` makes; if this is meant to be one, it was replaced or \
                 damaged, so get the game again.",
                path.display()
            ),
            Self::OtherVersion { path, version } => {
                let (age, fix) = if *version > FORMAT_VERSION {
                    (
                        "a newer",
                        "Get the game again whole, so its program and its pack match",
                    )
                } else {
                    (
                        "an older",
                        "Package the game again with this version of FOSSPS",
                    )
                };
                write!(
                    f,
                    "{} was made by {age} version of FOSSPS than this program (its pack \
                     layout is version {version}; this program reads version \
                     {FORMAT_VERSION}). {fix}.",
                    path.display()
                )
            }
            Self::Damaged { path, what } => write!(
                f,
                "{} is damaged: {what}. Get the game again, or if it is your game, \
                 package it again with `fossps package`.",
                path.display()
            ),
            Self::NoSuchFile { path, name } => write!(
                f,
                "{} has no file called {name} in it. If it is your game, check that the \
                 file is in the game's folder, then package it again with `fossps package`.",
                path.display()
            ),
            Self::NotText { path, name } => write!(
                f,
                "{name} in {} is not plain text (UTF-8). If it is your game, save the \
                 file as UTF-8, then package it again with `fossps package`.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } | Self::Copy { source, .. } => {
                Some(source)
            }
            _ => None,
        }
    }
}
