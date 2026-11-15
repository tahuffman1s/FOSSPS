//! Game packs: one `.fpk` file that holds a packaged game's files.
//!
//! `fossps package` writes a pack with [`write`] and a packaged game reads
//! its files back out of it with [`Pack`]. This is the first version of the
//! format: a list of names, then every file as it was on disk.
//!
//! ```no_run
//! use fossps_pack::Pack;
//!
//! let mut pack = Pack::open("my-game.fpk".as_ref())?;
//! println!("{} has {} files", pack.name(), pack.files().count());
//! let script = pack.read_text("main.luau")?;
//! # Ok::<(), fossps_pack::PackError>(())
//! ```
//!
//! A pack is read as untrusted input: it may have been downloaded. Every
//! length and position in it is checked against the size of the file, and a
//! name that could point outside a game's folder is refused.
//!
//! # Layout
//!
//! All numbers are little-endian.
//!
//! | Bytes | What |
//! |---|---|
//! | 8 | `FOSSPSPK` |
//! | 4 | format version, [`FORMAT_VERSION`] |
//! | 2 + n | the game's name: length, then UTF-8 |
//! | 4 | how many files |
//! | per file | name (2 + n, with `/` between folders), position (8), size (8) |
//! | rest | the files' bytes |

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub use error::PackError;

/// What a pack file's name ends in, without the dot.
pub const EXTENSION: &str = "fpk";

/// The version of the pack layout this crate writes and reads.
pub const FORMAT_VERSION: u32 = 1;

/// What every pack starts with.
const MAGIC: &[u8; 8] = b"FOSSPSPK";

/// The longest game name or file name in a pack, in bytes.
const MAX_NAME_BYTES: usize = 1024;

/// The fewest bytes one file takes in the list: an empty name is refused, so
/// a name is at least one byte after its length.
const SMALLEST_ENTRY: u64 = 2 + 1 + 8 + 8;

/// One file to put in a pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The file's name in the pack, with `/` between folders, such as
    /// `levels/dust.glb`.
    pub name: String,
    /// Where the file is on disk now.
    pub path: PathBuf,
}

/// Writes a pack for the game called `game` to `out`, holding `files`.
/// Returns the pack's size in bytes.
///
/// The files are stored in the order of their names, so the same game always
/// gives the same pack.
///
/// # Errors
///
/// Fails if a name cannot be stored (see [`PackError::BadName`]), two files
/// have the same name, or a file cannot be read or `out` cannot be written.
/// A pack that could only be partly written is left on disk for the caller to
/// remove.
pub fn write(out: &Path, game: &str, files: &[Source]) -> Result<u64, PackError> {
    if game.len() > MAX_NAME_BYTES {
        return Err(PackError::BadName {
            name: game.to_owned(),
            reason: "it is longer than 1024 bytes",
        });
    }
    let mut sorted: Vec<&Source> = files.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for pair in sorted.windows(2) {
        if let [a, b] = pair
            && a.name == b.name
        {
            return Err(PackError::Duplicate {
                name: a.name.clone(),
            });
        }
    }
    let count = u32::try_from(sorted.len()).map_err(|_| PackError::TooManyFiles)?;

    let mut sizes = Vec::with_capacity(sorted.len());
    let mut list_size = (MAGIC.len() + 4 + 2 + game.len() + 4) as u64;
    for source in &sorted {
        if let Some(reason) = name_problem(&source.name) {
            return Err(PackError::BadName {
                name: source.name.clone(),
                reason,
            });
        }
        let size = std::fs::metadata(&source.path)
            .map_err(|error| PackError::read(&source.path, error))?
            .len();
        sizes.push(size);
        list_size += (2 + source.name.len() + 8 + 8) as u64;
    }

    let writing = |error| PackError::Write {
        path: out.to_owned(),
        source: error,
    };
    let mut file = BufWriter::new(File::create(out).map_err(writing)?);
    let mut head = Vec::with_capacity(usize::try_from(list_size).unwrap_or(0));
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    put_text(&mut head, game);
    head.extend_from_slice(&count.to_le_bytes());
    let mut position = list_size;
    for (source, size) in sorted.iter().zip(&sizes) {
        put_text(&mut head, &source.name);
        head.extend_from_slice(&position.to_le_bytes());
        head.extend_from_slice(&size.to_le_bytes());
        position += size;
    }
    file.write_all(&head).map_err(writing)?;

    for (source, &size) in sorted.iter().zip(&sizes) {
        let mut from = File::open(&source.path)
            .map_err(|error| PackError::read(&source.path, error))?
            // One byte more than expected, so a file that grew is noticed.
            .take(size.saturating_add(1));
        // `io::copy` reports one error for both sides; the output is the
        // likelier one to fail (a full disk), and the message names both.
        let copied = io::copy(&mut from, &mut file).map_err(|error| PackError::Copy {
            from: source.path.clone(),
            to: out.to_owned(),
            source: error,
        })?;
        if copied != size {
            return Err(PackError::Changed {
                path: source.path.clone(),
            });
        }
    }
    file.flush().map_err(writing)?;
    Ok(position)
}

/// Adds `text` to `to` as its length in two bytes and then its bytes.
/// Callers have checked that it is no longer than [`MAX_NAME_BYTES`].
fn put_text(to: &mut Vec<u8>, text: &str) {
    let length = u16::try_from(text.len()).unwrap_or(u16::MAX);
    to.extend_from_slice(&length.to_le_bytes());
    to.extend_from_slice(text.as_bytes());
}

/// What stops `name` from being a file's name in a pack, as the end of a
/// sentence. A name is steps of plain text with `/` between them, and no step
/// may lead out of the game's folder or mean something else on Windows.
fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("it is empty");
    }
    if name.len() > MAX_NAME_BYTES {
        return Some("it is longer than 1024 bytes");
    }
    for step in name.split('/') {
        match step {
            "" => return Some("it starts or ends with `/`, or has two in a row"),
            "." | ".." => {
                return Some("it has a `.` or `..` step, which could lead out of the game");
            }
            _ => {}
        }
        if step.contains(['\\', ':']) {
            return Some("it has a `\\` or `:`, which mean another place on Windows");
        }
        if step.chars().any(char::is_control) {
            return Some("it has a character that cannot be shown");
        }
    }
    None
}

/// Where one file's bytes are in a pack.
#[derive(Debug, Clone, Copy)]
struct Place {
    position: u64,
    size: u64,
}

/// An open pack: a packaged game's files, read from one file on disk.
#[derive(Debug)]
pub struct Pack {
    path: PathBuf,
    file: File,
    game: String,
    files: BTreeMap<String, Place>,
}

impl Pack {
    /// Opens the pack at `path` and reads its list of files. No file's
    /// contents are read yet.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read, is not a pack, was written by a
    /// newer version of FOSSPS, or is damaged: cut short, or with a list that
    /// points outside the file.
    pub fn open(path: &Path) -> Result<Self, PackError> {
        let file = File::open(path).map_err(|error| PackError::read(path, error))?;
        let length = file
            .metadata()
            .map_err(|error| PackError::read(path, error))?
            .len();
        let damaged = |what: &'static str| PackError::Damaged {
            path: path.to_owned(),
            what,
        };
        // A read that runs out of file means the pack was cut short; any
        // other failure is the disk's.
        let short = |error: io::Error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                damaged("it ends before its list of files does")
            } else {
                PackError::read(path, error)
            }
        };
        let mut reader = BufReader::new(&file);

        let mut magic = [0; 8];
        let is_pack = match reader.read_exact(&mut magic) {
            Ok(()) => &magic == MAGIC,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => false,
            Err(error) => return Err(PackError::read(path, error)),
        };
        if !is_pack {
            return Err(PackError::NotAPack {
                path: path.to_owned(),
            });
        }
        let version = read_u32(&mut reader).map_err(short)?;
        if version != FORMAT_VERSION {
            return Err(PackError::OtherVersion {
                path: path.to_owned(),
                version,
            });
        }
        let game = read_text(&mut reader)
            .map_err(short)?
            .ok_or_else(|| damaged("the game's name is not readable text"))?;
        let count = read_u32(&mut reader).map_err(short)?;
        // Checked before anything is set aside for the list, so a made-up
        // count cannot ask for more memory than the file is long.
        if u64::from(count).saturating_mul(SMALLEST_ENTRY) > length {
            return Err(damaged("it says it holds more files than fit in it"));
        }

        let mut files = BTreeMap::new();
        for _ in 0..count {
            let name = read_text(&mut reader)
                .map_err(short)?
                .ok_or_else(|| damaged("a file's name is not readable text"))?;
            if name_problem(&name).is_some() {
                return Err(damaged(
                    "a file's name could lead outside the game's folder",
                ));
            }
            let position = read_u64(&mut reader).map_err(short)?;
            let size = read_u64(&mut reader).map_err(short)?;
            if files.insert(name, Place { position, size }).is_some() {
                return Err(damaged("two of its files have the same name"));
            }
        }
        let list_end = reader.stream_position().map_err(short)?;
        for place in files.values() {
            let inside = place.position >= list_end
                && place
                    .position
                    .checked_add(place.size)
                    .is_some_and(|end| end <= length);
            if !inside {
                return Err(damaged(
                    "its list points to a file that is not inside it (the pack may have \
                     been cut short while it was copied or downloaded)",
                ));
            }
        }
        drop(reader);
        Ok(Self {
            path: path.to_owned(),
            file,
            game,
            files,
        })
    }

    /// The name of the game this pack holds, as its maker wrote it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.game
    }

    /// The names of the files in the pack, in order, with `/` between
    /// folders.
    pub fn files(&self) -> impl ExactSizeIterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Whether the pack holds a file called `name`.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.files.contains_key(name)
    }

    /// Reads the whole file called `name`.
    ///
    /// # Errors
    ///
    /// Fails if the pack has no such file or it cannot be read.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, PackError> {
        let Some(&place) = self.files.get(name) else {
            return Err(PackError::NoSuchFile {
                path: self.path.clone(),
                name: name.to_owned(),
            });
        };
        let reading = |error| PackError::read(&self.path, error);
        // `open` checked that the file's bytes are inside the pack, so the
        // size is no more than the pack's own.
        let size = usize::try_from(place.size).map_err(|_| PackError::Damaged {
            path: self.path.clone(),
            what: "one of its files is too big for this computer to hold in memory",
        })?;
        let mut bytes = vec![0; size];
        let mut file = &self.file;
        file.seek(SeekFrom::Start(place.position))
            .map_err(reading)?;
        file.read_exact(&mut bytes).map_err(reading)?;
        Ok(bytes)
    }

    /// Reads the whole file called `name` as text.
    ///
    /// # Errors
    ///
    /// Fails if the pack has no such file, it cannot be read, or it is not
    /// plain text (UTF-8).
    pub fn read_text(&mut self, name: &str) -> Result<String, PackError> {
        String::from_utf8(self.read(name)?).map_err(|_| PackError::NotText {
            path: self.path.clone(),
            name: name.to_owned(),
        })
    }
}

fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Reads a length and then that many bytes of text. `Ok(None)` if the bytes
/// are not UTF-8.
fn read_text(reader: &mut impl Read) -> io::Result<Option<String>> {
    let mut length = [0; 2];
    reader.read_exact(&mut length)?;
    let mut bytes = vec![0; usize::from(u16::from_le_bytes(length))];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes).ok())
}
