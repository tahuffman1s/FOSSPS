//! Finds the data a glTF file points to: inside itself, or in files beside it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::error::AssetError;

/// The file being read and where to look for the files it points to.
#[derive(Clone, Copy)]
pub(crate) struct Origin<'a> {
    /// The file's name, for messages.
    pub(crate) file: &'a str,
    /// The folder the file is in, if it came from disk.
    pub(crate) folder: Option<&'a Path>,
}

impl Origin<'_> {
    /// Reads what `uri` points to. `texture` says what it is, for messages.
    pub(crate) fn read(&self, uri: &str, texture: bool) -> Result<Vec<u8>, AssetError> {
        if let Some(data) = uri.strip_prefix("data:") {
            return decode_data(data).ok_or_else(|| {
                AssetError::damaged(self.file, "data stored inside the file is not valid base64")
            });
        }
        let outside = || AssetError::OutsideFolder {
            file: self.file.to_owned(),
            uri: uri.to_owned(),
        };
        let relative = relative_path(uri).ok_or_else(outside)?;
        let Some(folder) = self.folder else {
            return Err(AssetError::NoFolder {
                file: self.file.to_owned(),
                uri: uri.to_owned(),
            });
        };
        let path = folder.join(relative);
        fs::read(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                AssetError::MissingFile {
                    file: self.file.to_owned(),
                    uri: uri.to_owned(),
                    path,
                    texture,
                }
            } else {
                AssetError::Read { path, source }
            }
        })
    }
}

/// Decodes what follows `data:` in a data address.
fn decode_data(data: &str) -> Option<Vec<u8>> {
    let (kind, encoded) = data.split_once(',')?;
    if !kind.ends_with(";base64") {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
}

/// Turns a glTF address into a path below the file's folder.
///
/// `None` if it could lead anywhere else: a web address, an absolute path,
/// a drive letter or a step up with `..`.
fn relative_path(uri: &str) -> Option<PathBuf> {
    let decoded = percent_decode(uri)?;
    let mut path = PathBuf::new();
    // Windows tools sometimes write backslashes; both mean a folder step.
    for (index, step) in decoded.split(['/', '\\']).enumerate() {
        match step {
            // A leading separator is an absolute path.
            "" if index == 0 => return None,
            "" | "." => {}
            ".." => return None,
            // A colon is a web address (`https:`), a drive (`C:`) or, on
            // Windows, a hidden stream; a NUL ends the name early on some systems.
            step if step.contains([':', '\0']) => return None,
            // Windows drops a trailing dot or space, and treats some names
            // as devices (`NUL`, `COM1`) whatever follows the first dot.
            step if step.ends_with(['.', ' ']) || is_device(step) => return None,
            step => path.push(step),
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

/// Whether Windows would open a device instead of a file called `step`.
fn is_device(step: &str) -> bool {
    let stem = step.split('.').next().unwrap_or(step).trim_end();
    let numbered = |prefix: &str| {
        stem.get(..3)
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            && matches!(stem.as_bytes().get(3..), Some([b'1'..=b'9']))
    };
    ["con", "prn", "aux", "nul"]
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
        || numbered("com")
        || numbered("lpt")
}

/// Replaces each `%41`-style escape with the byte it stands for.
fn percent_decode(uri: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(uri.len());
    let mut rest = uri.as_bytes().iter().copied();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let high = char::from(rest.next()?).to_digit(16)?;
            let low = char::from(rest.next()?).to_digit(16)?;
            bytes.push(u8::try_from(high * 16 + low).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_stay_inside_the_folder() {
        assert_eq!(relative_path("level.bin"), Some(PathBuf::from("level.bin")));
        assert_eq!(
            relative_path("textures/my%20wall.png"),
            Some(Path::new("textures").join("my wall.png"))
        );
        assert_eq!(
            relative_path("./a//b.bin"),
            Some(Path::new("a").join("b.bin"))
        );
        for escape in [
            "../secret.bin",
            "a/../../b.bin",
            "%2e%2e/b.bin",
            "/etc/passwd",
            "\\\\server\\share",
            "C:\\Windows\\x.bin",
            "c:/x.bin",
            "https://example.com/x.bin",
            "file:///etc/passwd",
            "a\\..\\..\\b",
            "",
            "bad%zzescape",
            "NUL",
            "textures/con.png",
            "Com1.bin",
            "lpt9 .txt",
            "name.",
            "name%20",
        ] {
            assert_eq!(relative_path(escape), None, "{escape}");
        }
    }

    #[test]
    fn data_addresses_decode() {
        assert_eq!(
            decode_data("application/octet-stream;base64,AQID"),
            Some(vec![1, 2, 3])
        );
        assert_eq!(decode_data("text/plain,hello"), None);
        assert_eq!(decode_data("application/octet-stream;base64,!!"), None);
    }
}
