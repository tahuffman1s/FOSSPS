//! Levels fetched from servers, kept on the player's computer so the same
//! level is only fetched once.
//!
//! They are in the `fossps/levels` folder of the place each system keeps
//! what can be made again: `~/.cache` on Linux (or `XDG_CACHE_HOME`),
//! `%LOCALAPPDATA%` on Windows, `~/Library/Caches` on macOS. Setting
//! `FOSSPS_CACHE` to a folder's path keeps them there instead.
//!
//! A file is named after the SHA-256 hash of what is in it and nothing
//! else, so a server has no say in where anything is written, and a file
//! that no longer matches its name is not used. Only the newest few are
//! kept.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use fossps_net::{SharedFile, hash_of};

/// Names another folder to keep fetched levels in than the usual one.
pub const FOLDER_VARIABLE: &str = "FOSSPS_CACHE";

/// How many fetched levels are kept. The ones used longest ago go first.
const KEPT: usize = 8;

/// How old a half-written file is before it is taken to be abandoned.
const ABANDONED_AFTER: Duration = Duration::from_secs(60 * 60);

/// The folder fetched levels are kept in, whether or not it exists yet.
///
/// `None` when the computer does not say where such things go (no home
/// folder); levels are then fetched each time.
#[must_use]
pub fn folder() -> Option<PathBuf> {
    let set = |name: &str| env::var_os(name).filter(|value| !value.is_empty());
    if let Some(path) = set(FOLDER_VARIABLE) {
        return Some(PathBuf::from(path));
    }
    let caches = if cfg!(target_os = "windows") {
        PathBuf::from(set("LOCALAPPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(set("HOME")?).join("Library/Caches")
    } else {
        // The XDG rules: a path that is not absolute is to be ignored.
        match set("XDG_CACHE_HOME").map(PathBuf::from) {
            Some(folder) if folder.is_absolute() => folder,
            _ => PathBuf::from(set("HOME")?).join(".cache"),
        }
    };
    Some(caches.join("fossps").join("levels"))
}

/// What a file with `hash` is called: the hash as 64 letters and digits.
fn name_of(hash: &[u8; 32]) -> String {
    use std::fmt::Write;

    let mut name = String::with_capacity(64);
    for byte in hash {
        // Writing to a string cannot fail.
        let _ = write!(name, "{byte:02x}");
    }
    name
}

/// Whether `name` is what [`name_of`] gives: the only files here that are
/// ever read or removed.
fn is_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The file in `folder` that is what a server offers as `file`, if it is
/// there and still whole. One that has been changed is removed.
pub(crate) fn find(folder: &Path, file: SharedFile) -> Option<Vec<u8>> {
    let path = folder.join(name_of(&file.hash));
    if fs::metadata(&path).ok()?.len() != u64::from(file.len) {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    if hash_of(&bytes) != file.hash {
        let _ = fs::remove_file(&path);
        return None;
    }
    // Used now, so it is the last to be cleared out. If the time cannot be
    // set it is cleared out sooner, which only costs a fetch.
    if let Ok(kept) = fs::OpenOptions::new().write(true).open(&path) {
        let _ = kept.set_modified(SystemTime::now());
    }
    Some(bytes)
}

/// Keeps `bytes` in `folder` under their own hash, then clears out all but
/// the newest few files kept there. Gives where they were written.
pub(crate) fn keep(folder: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let name = name_of(&hash_of(bytes));
    let path = folder.join(&name);
    // Written beside it and moved into place, so a half-written file never
    // has the name of a whole one.
    let unfinished = folder.join(format!("{name}.{}.part", std::process::id()));
    let written = fs::write(&unfinished, bytes).and_then(|()| fs::rename(&unfinished, &path));
    if let Err(error) = written {
        let _ = fs::remove_file(&unfinished);
        return Err(error);
    }
    clear_out(folder);
    Ok(path)
}

/// Whether `name` is what a file is called while it is being written:
/// the name it will have, the number of the program writing it, `part`.
fn is_unfinished(name: &str) -> bool {
    let mut parts = name.split('.');
    parts.next().is_some_and(is_name)
        && parts
            .next()
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        && parts.next() == Some("part")
        && parts.next().is_none()
}

/// Removes all but the [`KEPT`] newest fetched files in `folder`, and any
/// file left half-written over an hour ago by a program that was stopped.
/// Nothing else in the folder is touched.
fn clear_out(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut kept: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(about) = entry.metadata() else {
            continue;
        };
        let Ok(written) = about.modified() else {
            continue;
        };
        if !about.is_file() {
            continue;
        }
        if is_name(name) {
            kept.push((written, entry.path()));
        } else if is_unfinished(name) && written.elapsed().is_ok_and(|age| age > ABANDONED_AFTER) {
            let _ = fs::remove_file(entry.path());
        }
    }
    kept.sort_by(|a, b| b.cmp(a));
    for (_, path) in kept.iter().skip(KEPT) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_for(test: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("fossps-cache-test-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    fn offered(bytes: &[u8]) -> SharedFile {
        SharedFile {
            len: bytes.len() as u32,
            hash: hash_of(bytes),
        }
    }

    #[test]
    fn a_kept_file_is_found_by_its_hash_and_a_changed_one_is_not() {
        let folder = folder_for("find");
        let level = b"a level".to_vec();
        assert_eq!(find(&folder, offered(&level)), None);

        let path = keep(&folder, &level).unwrap();
        assert_eq!(path.parent(), Some(&*folder));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(is_name(name), "{name}");
        assert_eq!(find(&folder, offered(&level)), Some(level.clone()));
        assert_eq!(find(&folder, offered(b"another")), None);
        // Nothing unfinished is left behind.
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);

        // Changed on disk, same length: not used, and removed.
        fs::write(&path, b"b level").unwrap();
        assert_eq!(find(&folder, offered(&level)), None);
        assert!(!path.exists());
        // Changed on disk, another length: not used.
        fs::write(&path, b"a longer level").unwrap();
        assert_eq!(find(&folder, offered(&level)), None);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn only_the_newest_few_are_kept_and_other_files_are_left_alone() {
        let folder = folder_for("clear");
        fs::create_dir_all(&folder).unwrap();
        let mine = folder.join("notes.txt");
        fs::write(&mine, "mine").unwrap();
        // Named like a fetched file but a folder: left alone too.
        let odd = folder.join("0".repeat(64));
        fs::create_dir(&odd).unwrap();

        let long_ago = SystemTime::UNIX_EPOCH;
        let mut paths = Vec::new();
        for number in 0..KEPT + 3 {
            let path = keep(&folder, format!("level {number}").as_bytes()).unwrap();
            // Each older than the next, whatever the clock's grain.
            let at = long_ago + Duration::from_secs(1000 + number as u64);
            let kept = fs::OpenOptions::new().write(true).open(&path).unwrap();
            kept.set_modified(at).unwrap();
            paths.push(path);
        }
        // One file left half-written long ago, one just now.
        let name = name_of(&hash_of(b"x"));
        let (stale, fresh) = (
            folder.join(format!("{name}.123.part")),
            folder.join(format!("{name}.456.part")),
        );
        fs::write(&stale, "half").unwrap();
        fs::write(&fresh, "half").unwrap();
        let kept = fs::OpenOptions::new().write(true).open(&stale).unwrap();
        kept.set_modified(long_ago).unwrap();
        drop(kept);

        keep(&folder, b"the newest").unwrap();
        assert!(!stale.exists() && fresh.exists());
        assert!(!is_unfinished("notes.part") && !is_unfinished(&format!("{name}.x.part")));
        assert!(!is_unfinished(&format!("{name}.1.part.txt")));
        let left = paths.iter().filter(|path| path.exists()).count();
        assert_eq!(left, KEPT - 1);
        assert!(!paths[0].exists() && paths[KEPT + 2].exists());
        assert!(mine.exists() && odd.is_dir());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_name_is_the_hash_and_nothing_a_server_chose() {
        let name = name_of(&hash_of(b""));
        assert_eq!(
            name,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(is_name(&name));
        for other in ["", "notes.txt", "../x", &name[1..], &name.to_uppercase()] {
            assert!(!is_name(other), "{other}");
        }
    }
}
