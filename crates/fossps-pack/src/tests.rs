use std::fs;
use std::path::PathBuf;

use crate::{FORMAT_VERSION, Pack, PackError, Source, write};

/// An empty folder for one test.
fn scratch(test: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("fossps-pack-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

/// Writes `files` (name, contents) to disk and packs them.
fn packed(test: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let folder = scratch(test);
    let sources: Vec<Source> = files
        .iter()
        .enumerate()
        .map(|(number, (name, contents))| {
            let path = folder.join(format!("file-{number}"));
            fs::write(&path, contents).unwrap();
            Source {
                name: (*name).to_owned(),
                path,
            }
        })
        .collect();
    let out = folder.join("game.fpk");
    let size = write(&out, "Zombie Night", &sources).unwrap();
    assert_eq!(size, fs::metadata(&out).unwrap().len());
    out
}

/// A pack's first bytes, up to and including the count of files, for a game
/// called `g`.
fn head(count: u32) -> Vec<u8> {
    let mut bytes = b"FOSSPSPK".to_vec();
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(b'g');
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes
}

/// One entry of a pack's list.
fn entry(name: &str, position: u64, size: u64) -> Vec<u8> {
    let mut bytes = (name.len() as u16).to_le_bytes().to_vec();
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(&position.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes
}

/// What opening a pack made of `bytes` says.
fn opening(test: &str, bytes: &[u8]) -> String {
    let path = scratch(test).join("game.fpk");
    fs::write(&path, bytes).unwrap();
    Pack::open(&path).unwrap_err().to_string()
}

#[test]
fn a_pack_gives_back_the_files_that_went_in() {
    let out = packed(
        "round-trip",
        &[
            ("main.luau", b"print('hello')"),
            ("levels/dust.glb", &[0, 1, 2, 255]),
            ("empty.txt", b""),
        ],
    );

    let mut pack = Pack::open(&out).unwrap();

    assert_eq!(pack.name(), "Zombie Night");
    assert_eq!(
        pack.files().collect::<Vec<_>>(),
        ["empty.txt", "levels/dust.glb", "main.luau"]
    );
    assert!(pack.contains("main.luau"));
    assert_eq!(pack.read_text("main.luau").unwrap(), "print('hello')");
    assert_eq!(pack.read("levels/dust.glb").unwrap(), [0, 1, 2, 255]);
    assert_eq!(pack.read("empty.txt").unwrap(), b"");
    // Reading one file twice, and out of order, gives the same bytes.
    assert_eq!(pack.read_text("main.luau").unwrap(), "print('hello')");
}

#[test]
fn the_same_files_in_another_order_give_the_same_pack() {
    let forwards = packed("order-a", &[("a.txt", b"one"), ("b.txt", b"two")]);
    let backwards = packed("order-b", &[("b.txt", b"two"), ("a.txt", b"one")]);

    assert_eq!(fs::read(forwards).unwrap(), fs::read(backwards).unwrap());
}

#[test]
fn asking_for_a_file_that_is_not_there_names_it() {
    let out = packed("missing", &[("main.luau", b"")]);

    let said = Pack::open(&out)
        .unwrap()
        .read("levels/dust.glb")
        .unwrap_err()
        .to_string();

    assert!(said.contains("no file called levels/dust.glb"), "{said}");
}

#[test]
fn a_file_that_is_not_text_is_not_read_as_text() {
    let out = packed("not-text", &[("main.luau", &[0xff, 0xfe])]);

    let error = Pack::open(&out)
        .unwrap()
        .read_text("main.luau")
        .unwrap_err();

    assert!(matches!(error, PackError::NotText { .. }), "{error}");
}

#[test]
fn names_that_could_leave_the_game_folder_are_not_packed() {
    let folder = scratch("bad-names");
    let path = folder.join("file");
    fs::write(&path, "x").unwrap();
    for name in [
        "",
        "/etc/passwd",
        "../secret",
        "levels/../../secret",
        "levels//dust.glb",
        "levels/",
        "./main.luau",
        "C:/windows",
        "levels\\dust.glb",
        "bell\u{7}.txt",
    ] {
        let sources = [Source {
            name: name.to_owned(),
            path: path.clone(),
        }];
        let error = write(&folder.join("game.fpk"), "g", &sources).unwrap_err();
        assert!(
            matches!(error, PackError::BadName { .. }),
            "{name}: {error}"
        );
    }
}

#[test]
fn two_files_cannot_have_one_name() {
    let folder = scratch("duplicate");
    let path = folder.join("file");
    fs::write(&path, "x").unwrap();
    let source = Source {
        name: "main.luau".to_owned(),
        path,
    };

    let error = write(&folder.join("game.fpk"), "g", &[source.clone(), source]).unwrap_err();

    assert!(matches!(error, PackError::Duplicate { .. }), "{error}");
}

#[test]
fn a_file_that_is_missing_is_named() {
    let folder = scratch("missing-source");
    let sources = [Source {
        name: "main.luau".to_owned(),
        path: folder.join("not-there.luau"),
    }];

    let said = write(&folder.join("game.fpk"), "g", &sources)
        .unwrap_err()
        .to_string();

    assert!(said.contains("not-there.luau"), "{said}");
}

#[test]
fn something_that_is_not_a_pack_is_refused() {
    for (test, bytes) in [
        ("not-a-pack", b"#!/bin/sh\necho hello\n".as_slice()),
        ("empty", b""),
        ("short", b"FOSS"),
    ] {
        let said = opening(test, bytes);
        assert!(said.contains("is not a FOSSPS game pack"), "{said}");
    }
}

#[test]
fn a_pack_from_another_version_says_which_side_to_update() {
    let mut newer = b"FOSSPSPK".to_vec();
    newer.extend_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    let said = opening("newer", &newer);
    assert!(said.contains("a newer version of FOSSPS"), "{said}");

    let mut older = b"FOSSPSPK".to_vec();
    older.extend_from_slice(&0u32.to_le_bytes());
    let said = opening("older", &older);
    assert!(said.contains("an older version of FOSSPS"), "{said}");
    assert!(said.contains("Package the game again"), "{said}");
}

#[test]
fn a_pack_cut_short_is_damaged_not_a_crash() {
    let out = packed(
        "cut",
        &[("main.luau", b"print('hello')"), ("b.txt", b"two")],
    );
    let whole = fs::read(&out).unwrap();

    // Every shorter length is refused cleanly, wherever the cut falls.
    for length in 12..whole.len() {
        let said = opening("cut-at", &whole[..length]);
        assert!(said.contains("is damaged"), "{length}: {said}");
    }
}

#[test]
fn a_count_larger_than_the_file_is_refused_before_anything_is_read() {
    let said = opening("count", &head(u32::MAX));

    assert!(said.contains("more files than fit"), "{said}");
}

#[test]
fn a_list_that_points_outside_the_pack_is_refused() {
    let list_end = (head(1).len() + entry("a", 0, 0).len()) as u64;
    for (test, position, size) in [
        ("past-end", list_end, 5),
        ("into-list", 0, 4),
        ("overflow", u64::MAX, 2),
        ("far", 1 << 40, 1),
    ] {
        let mut bytes = head(1);
        bytes.extend_from_slice(&entry("a", position, size));
        bytes.extend_from_slice(b"data");
        let said = opening(test, &bytes);
        assert!(said.contains("not inside it"), "{test}: {said}");
    }
}

#[test]
fn a_list_with_unsafe_or_repeated_names_is_refused() {
    let mut escaping = head(1);
    escaping.extend_from_slice(&entry("../../home/secret", 0, 0));
    let said = opening("escaping", &escaping);
    assert!(said.contains("outside the game's folder"), "{said}");

    let mut repeated = head(2);
    let end = (repeated.len() + 2 * entry("a", 0, 0).len()) as u64;
    repeated.extend_from_slice(&entry("a", end, 0));
    repeated.extend_from_slice(&entry("a", end, 0));
    let said = opening("repeated", &repeated);
    assert!(said.contains("same name"), "{said}");

    let mut not_text = head(1);
    not_text.extend_from_slice(&2u16.to_le_bytes());
    not_text.extend_from_slice(&[0xff, 0xfe]);
    not_text.extend_from_slice(&[0; 16]);
    let said = opening("not-text-name", &not_text);
    assert!(said.contains("not readable text"), "{said}");
}
