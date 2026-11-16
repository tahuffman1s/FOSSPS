//! The map file: what is placed in a game's level, as text beside it.
//!
//! A game's `level.glb` is its shapes, made in a 3D tool and never written
//! by the engine. Its `map.toml` lists what stands in it for the game's
//! rules: spawn points, zones, objectives. `fossps edit` writes it, it can
//! be written by hand, and a running game picks up a save by itself.
//!
//! ```toml
//! version = 1
//!
//! [[entity]]
//! id = "k7f3q2xa"
//! type = "spawn"
//! name = "Red spawn"
//! position = [4.0, 0.0, -12.5]
//! facing = 180.0
//!
//! [entity.properties]
//! team = "red"
//! ```
//!
//! Each entity keeps its `id` for good, so scripts and other files can refer
//! to it. IDs are made by chance, not counted, and entities are written in
//! the order of their IDs: two people who each place something get two
//! separate changes that merge without a conflict.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use toml::de::{DeTable, DeValue};

/// What a game's map file is called. It sits beside `main.luau` and
/// `level.glb` at the top of the game's folder.
pub const MAP_FILE: &str = "map.toml";

/// The format this version of the engine writes, and the newest it reads.
pub const VERSION: i64 = 1;

/// The type of an entity where players come into the world.
pub const SPAWN: &str = "spawn";

/// What a save is written to before it takes the map file's place. Hidden,
/// so one left behind by a crash is not packaged with the game.
const SAVING_FILE: &str = ".map.toml.saving";

/// The longest a map file may be, in bytes. A map this size holds far more
/// than a level has room for; anything longer is some other file.
const LONGEST_FILE: u64 = 4 * 1024 * 1024;
/// The most entities one map holds.
const MOST_ENTITIES: usize = 10_000;
/// The most properties one entity holds.
const MOST_PROPERTIES: usize = 64;
/// The longest an ID, a type or a property's name may be, in characters.
pub const LONGEST_WORD: usize = 64;
/// The longest a name or a property's text may be, in characters.
pub const LONGEST_TEXT: usize = 1024;
/// How far from the middle of the level anything may stand, in metres.
pub const FURTHEST: f32 = 100_000.0;

/// How many characters an ID made by the engine has, and what they are
/// drawn from: no `i`, `l`, `o`, `0` or `1`, which read alike.
const ID_LENGTH: usize = 8;
const ID_LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// How often a map file is looked at for changes.
const POLL_EVERY: Duration = Duration::from_millis(100);

/// What a written file says before its entities.
const HEADER: &str = "\
# What is placed in this game's level: spawn points, zones and objectives.
# `fossps edit` writes this file. It can be changed by hand as well: a game
# that is running, and the editor, pick up a save by themselves.
#
# Every entity keeps its id for good. To add one by hand, give it any id no
# other entity here has.
";

/// What an entity's property holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Words, such as a team's name.
    Text(String),
    /// A number.
    Number(f64),
    /// Yes or no.
    Flag(bool),
}

impl fmt::Display for Value {
    /// The value as it is written in the map file.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::new();
        write_value(&mut text, self);
        f.write_str(&text)
    }
}

/// One thing placed in the level: a spawn point, a zone, an objective.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// What it is known by for good. No two entities of a map share one.
    pub id: String,
    /// What sort of thing it is, such as `spawn`.
    pub kind: String,
    /// What its maker calls it. May be empty.
    pub name: String,
    /// Where it stands, in metres.
    pub position: [f32; 3],
    /// How far it is turned to the left, in degrees. At 0 it faces -Z, as a
    /// player who has not turned does.
    pub facing: f32,
    /// Anything else the game's rules want to know about it, by name.
    pub properties: BTreeMap<String, Value>,
}

/// Everything placed in a level. The entities are kept in the order of
/// their IDs, which is the order they are written in.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Map {
    entities: Vec<Entity>,
}

/// Why a map file could not be read or written.
#[derive(Debug)]
pub enum MapError {
    /// The file could not be read from disk.
    Read {
        /// The map file.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The file could not be written to disk.
    Write {
        /// The map file.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The file is far longer than any map.
    TooLong {
        /// What messages call the file.
        file: String,
    },
    /// The file was written by a newer version of the engine.
    Newer {
        /// What messages call the file.
        file: String,
        /// The version the file says it is.
        version: i64,
    },
    /// Something in the file is not as a map file has it.
    Mistake {
        /// What messages call the file.
        file: String,
        /// The line of the mistake, counted from 1.
        line: usize,
        /// What is wrong and what to write instead.
        problem: String,
    },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(
                f,
                "{} could not be read: {source}. Check that the file can be opened by you.",
                path.display()
            ),
            Self::Write { path, source } => write!(
                f,
                "{} could not be saved: {source}. Check that the game's folder can be \
                 written to and the disk is not full.",
                path.display()
            ),
            Self::TooLong { file } => write!(
                f,
                "{file} is longer than 4 MB, which no map file is. Check that it is the \
                 map file and not something else saved under its name."
            ),
            Self::Newer { file, version } => write!(
                f,
                "{file} says version = {version}, but this version of FOSSPS reads map \
                 files up to version {VERSION}. It was saved by a newer FOSSPS: update \
                 FOSSPS to open it."
            ),
            Self::Mistake {
                file,
                line,
                problem,
            } => write!(f, "{file}, line {line}: {problem}"),
        }
    }
}

impl std::error::Error for MapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl Map {
    /// Reads the text of a map file. `file` is what messages call it.
    ///
    /// ```
    /// use fossps::map::Map;
    ///
    /// let map = Map::parse(
    ///     "map.toml",
    ///     "version = 1\n\n[[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [4, 0, -12.5]\n",
    /// )?;
    /// assert_eq!(map.entities()[0].position, [4.0, 0.0, -12.5]);
    /// # Ok::<(), fossps::map::MapError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Fails if the text is not a map file this version of the engine can
    /// read. The message gives the line and says what to write instead.
    pub fn parse(file: &str, text: &str) -> Result<Self, MapError> {
        if text.len() as u64 > LONGEST_FILE {
            return Err(MapError::TooLong {
                file: file.to_owned(),
            });
        }
        let mistake = |offset: usize, problem: String| MapError::Mistake {
            file: file.to_owned(),
            line: line_of(text, offset),
            problem,
        };
        let document = DeTable::parse(text).map_err(|error| {
            mistake(
                error.span().map_or(0, |span| span.start),
                format!(
                    "{}. The file is TOML: an entity starts with a line saying [[entity]], \
                     followed by lines like type = \"spawn\".",
                    error.message().trim_end_matches('.')
                ),
            )
        })?;

        // The version first, whatever else the file holds: a newer file may
        // have things in it this reader would call mistakes.
        let mut version = None;
        for (key, value) in document.get_ref() {
            if key.get_ref() != "version" {
                continue;
            }
            let at = key.span().start;
            let read = match value.get_ref() {
                DeValue::Integer(number) => {
                    i64::from_str_radix(number.as_str(), number.radix()).ok()
                }
                _ => None,
            };
            match read {
                Some(read) if read > VERSION => {
                    return Err(MapError::Newer {
                        file: file.to_owned(),
                        version: read,
                    });
                }
                Some(read) if read >= 1 => version = Some(read),
                _ => {
                    return Err(mistake(
                        at,
                        format!(
                            "version has to be a whole number from 1 to {VERSION}, like \
                             version = {VERSION}"
                        ),
                    ));
                }
            }
        }
        if version.is_none() {
            return Err(mistake(
                0,
                format!(
                    "The file does not say which version of the map format it is. Add \
                     this as its first line: version = {VERSION}"
                ),
            ));
        }

        let mut entities: Vec<Entity> = Vec::new();
        // Where each ID was first written, to point at both of a pair.
        let mut lines: BTreeMap<String, usize> = BTreeMap::new();
        for (key, value) in document.get_ref() {
            let at = key.span().start;
            match (key.get_ref().as_ref(), value.get_ref()) {
                ("version", _) => {}
                ("entity", DeValue::Array(list)) => {
                    if list.len() > MOST_ENTITIES {
                        return Err(mistake(
                            at,
                            format!(
                                "There are {} entities, and a map holds {MOST_ENTITIES} at \
                                 most. Take some out.",
                                list.len()
                            ),
                        ));
                    }
                    for item in list.iter() {
                        let DeValue::Table(table) = item.get_ref() else {
                            return Err(mistake(
                                item.span().start,
                                "An entity starts with a line saying [[entity]].".to_owned(),
                            ));
                        };
                        let (entity, line_at) = read_entity(table, item.span().start)
                            .map_err(|(offset, problem)| mistake(offset, problem))?;
                        if let Some(&first) = lines.get(&entity.id) {
                            // Lines are counted only for a mistake: counting
                            // one for every entity would read the file once
                            // for each of them.
                            let first = line_of(text, first);
                            return Err(mistake(
                                line_at,
                                format!(
                                    "The id \"{}\" is also used on line {first}. Every \
                                     entity needs an id of its own: change one of the two.",
                                    entity.id
                                ),
                            ));
                        }
                        lines.insert(entity.id.clone(), line_at);
                        entities.push(entity);
                    }
                }
                ("entity", _) => {
                    return Err(mistake(
                        at,
                        "An entity starts with a line saying [[entity]], in double square \
                         brackets."
                            .to_owned(),
                    ));
                }
                (other, _) => {
                    return Err(mistake(
                        at,
                        format!(
                            "\"{other}\" is not something a map file has. It holds a line \
                             saying version = {VERSION} and entities, each under a line \
                             saying [[entity]]."
                        ),
                    ));
                }
            }
        }
        entities.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self { entities })
    }

    /// The text of the map file for this map. Reading it back gives the same
    /// map, with positions to the millimetre and facings to the hundredth of
    /// a degree.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER}\nversion = {VERSION}\n");
        for entity in &self.entities {
            text.push_str("\n[[entity]]\nid = ");
            write_string(&mut text, &entity.id);
            text.push_str("\ntype = ");
            write_string(&mut text, &entity.kind);
            if !entity.name.is_empty() {
                text.push_str("\nname = ");
                write_string(&mut text, &entity.name);
            }
            let [x, y, z] = entity
                .position
                .map(|part| rounded(part.clamp(-FURTHEST, FURTHEST), 1000.0));
            // Writing to a `String` cannot fail.
            let _ = write!(
                text,
                "\nposition = [{x:?}, {y:?}, {z:?}]\nfacing = {:?}\n",
                rounded(wrapped(entity.facing), 100.0)
            );
            if !entity.properties.is_empty() {
                text.push_str("\n[entity.properties]\n");
                for (name, value) in &entity.properties {
                    write_key(&mut text, name);
                    text.push_str(" = ");
                    write_value(&mut text, value);
                    text.push('\n');
                }
            }
        }
        text
    }

    /// Reads the map file of the game in `folder`. A game with no map file
    /// has an empty map.
    ///
    /// # Errors
    ///
    /// Fails if the file is there but cannot be read as a map file.
    pub fn load(folder: &Path) -> Result<Self, MapError> {
        read_file(&folder.join(MAP_FILE)).map(Option::unwrap_or_default)
    }

    /// Everything placed, in the order of their IDs.
    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Whether nothing is placed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The entity with this ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Entity> {
        self.index_of(id).map(|index| &self.entities[index])
    }

    /// The entity with this ID, to change it. Its `id` must be left alone.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Entity> {
        self.index_of(id).map(|index| &mut self.entities[index])
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.entities
            .binary_search_by(|entity| entity.id.as_str().cmp(id))
            .ok()
    }

    /// Places something of type `kind` and gives the ID made for it. `None`
    /// if the map is full or `kind` is not a word a type can be.
    pub fn place(&mut self, kind: &str, position: [f32; 3], facing: f32) -> Option<&str> {
        if self.entities.len() >= MOST_ENTITIES || check_word(kind).is_err() {
            return None;
        }
        let mut seed = RandomState::new().build_hasher();
        let id = loop {
            seed.write_usize(self.entities.len());
            let mut number = seed.finish();
            let id: String = (0..ID_LENGTH)
                .map(|_| {
                    let letter = ID_LETTERS[(number % ID_LETTERS.len() as u64) as usize];
                    number /= ID_LETTERS.len() as u64;
                    char::from(letter)
                })
                .collect();
            if self.index_of(&id).is_none() {
                break id;
            }
        };
        let finite = |part: f32| if part.is_finite() { part } else { 0.0 };
        let entity = Entity {
            id,
            kind: kind.to_owned(),
            name: String::new(),
            position: position.map(|part| finite(part).clamp(-FURTHEST, FURTHEST)),
            facing: wrapped(finite(facing)),
            properties: BTreeMap::new(),
        };
        let index = self.entities.partition_point(|other| other.id < entity.id);
        self.entities.insert(index, entity);
        Some(self.entities[index].id.as_str())
    }

    /// Takes the entity with this ID out of the map.
    pub fn remove(&mut self, id: &str) -> Option<Entity> {
        self.index_of(id).map(|index| self.entities.remove(index))
    }

    /// Where players come into the world: every entity of type `spawn`.
    pub fn spawns(&self) -> impl Iterator<Item = &Entity> + '_ {
        self.entities.iter().filter(|entity| entity.kind == SPAWN)
    }

    /// A line about the map for the person who made it: how many of each
    /// type are placed, such as "3 placed: 2 spawn, 1 zone".
    #[must_use]
    pub fn summary(&self) -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for entity in &self.entities {
            *counts.entry(entity.kind.as_str()).or_default() += 1;
        }
        let mut words = format!("{} placed", self.entities.len());
        for (index, (kind, count)) in counts.iter().enumerate() {
            words.push_str(if index == 0 { ": " } else { ", " });
            // Writing to a `String` cannot fail.
            let _ = write!(words, "{count} {kind}");
        }
        words
    }
}

/// Whether `word` can be an entity's type: not empty, at most 64 characters,
/// no space at either end, and nothing in it that cannot be seen.
#[must_use]
pub fn is_type(word: &str) -> bool {
    check_word(word).is_ok()
}

/// Whether `word` can be an ID, a type or a property's name: not empty, not
/// too long, and nothing in it that cannot be seen.
fn check_word(word: &str) -> Result<(), &'static str> {
    if word.trim().is_empty() {
        Err("is empty")
    } else if word.trim() != word {
        Err("starts or ends with a space")
    } else if word.chars().count() > LONGEST_WORD {
        Err("is longer than 64 characters")
    } else if word.chars().any(char::is_control) {
        Err("has a character in it that cannot be seen")
    } else {
        Ok(())
    }
}

/// Whether `text` can be a name or a property's words.
fn check_text(text: &str) -> Result<(), &'static str> {
    if text.chars().count() > LONGEST_TEXT {
        Err("is longer than 1024 characters")
    } else {
        Ok(())
    }
}

/// A number written in the file, whole or not.
fn number(value: &DeValue<'_>) -> Option<f64> {
    let read = match value {
        DeValue::Integer(whole) => {
            // Whole numbers a map has any use for are exact as `f64`.
            #[allow(clippy::cast_precision_loss)]
            let read = i64::from_str_radix(whole.as_str(), whole.radix()).ok()? as f64;
            read
        }
        DeValue::Float(float) => float.as_str().parse::<f64>().ok()?,
        _ => return None,
    };
    read.is_finite().then_some(read)
}

/// Reads one `[[entity]]`, which starts at byte `start`. Gives the entity
/// and where its ID is written, or where a mistake is and what it is.
fn read_entity(table: &DeTable<'_>, start: usize) -> Result<(Entity, usize), (usize, String)> {
    let mut id = None;
    let mut kind = None;
    let mut name = String::new();
    let mut position = None;
    let mut facing = 0.0;
    let mut properties = BTreeMap::new();
    // The first line of the entity, for what is said about all of it.
    let mut first = usize::MAX;
    for (key, value) in table {
        let at = key.span().start;
        first = first.min(at);
        let key = key.get_ref().as_ref();
        let word = |what: &str| -> Result<String, (usize, String)> {
            let text = value.get_ref().as_str().ok_or_else(|| {
                (
                    at,
                    format!("{what} has to be text in quotes, like {key} = \"spawn\"."),
                )
            })?;
            Ok(text.to_owned())
        };
        match key {
            "id" => {
                let text = word("An id")?;
                check_word(&text).map_err(|why| (at, format!("This id {why}. Give it a short one of its own, like id = \"red-spawn-1\".")))?;
                id = Some((text, at));
            }
            "type" => {
                let text = word("A type")?;
                check_word(&text).map_err(|why| {
                    (
                        at,
                        format!("This type {why}. Say what the entity is, like type = \"spawn\"."),
                    )
                })?;
                kind = Some(text);
            }
            "name" => {
                name = word("A name")?;
                check_text(&name).map_err(|why| (at, format!("This name {why}. Shorten it.")))?;
            }
            "position" => {
                let parts: Option<Vec<f64>> = value.get_ref().as_array().map(|list| {
                    list.iter()
                        .filter_map(|part| number(part.get_ref()))
                        .collect()
                });
                let listed = value.get_ref().as_array().map_or(0, |list| list.len());
                match parts.as_deref() {
                    Some(&[x, y, z])
                        if listed == 3
                            && [x, y, z]
                                .iter()
                                .all(|part| part.abs() <= f64::from(FURTHEST)) =>
                    {
                        // Within 100 km, so each fits an `f32`.
                        #[allow(clippy::cast_possible_truncation)]
                        let place = [x as f32, y as f32, z as f32];
                        position = Some(place);
                    }
                    _ => {
                        return Err((
                            at,
                            "A position is three numbers in metres, each within 100000 of \
                             zero: across, up and along, like position = [4.0, 0.0, -12.5]."
                                .to_owned(),
                        ));
                    }
                }
            }
            "facing" => {
                let Some(turned) = number(value.get_ref()) else {
                    return Err((
                        at,
                        "facing is how far the entity is turned to the left, in degrees, \
                         like facing = 90.0."
                            .to_owned(),
                    ));
                };
                // Wrapped to half a turn either way first, so it fits.
                #[allow(clippy::cast_possible_truncation)]
                let turned = turned.rem_euclid(360.0) as f32;
                facing = wrapped(turned);
            }
            "properties" => {
                let Some(entries) = value.get_ref().as_table() else {
                    return Err((
                        at,
                        "Properties go under a line saying [entity.properties], one to a \
                         line, like team = \"red\"."
                            .to_owned(),
                    ));
                };
                if entries.len() > MOST_PROPERTIES {
                    return Err((
                        at,
                        format!(
                            "This entity has {} properties, and one holds \
                             {MOST_PROPERTIES} at most. Take some out.",
                            entries.len()
                        ),
                    ));
                }
                for (name, value) in entries {
                    let at = name.span().start;
                    let name = name.get_ref().as_ref();
                    check_word(name).map_err(|why| {
                        (
                            at,
                            format!("This property's name {why}. Give it a short name, like team."),
                        )
                    })?;
                    let value = match value.get_ref() {
                        DeValue::String(text) => {
                            check_text(text).map_err(|why| {
                                (at, format!("The text of \"{name}\" {why}. Shorten it."))
                            })?;
                            Value::Text(text.to_string())
                        }
                        DeValue::Boolean(flag) => Value::Flag(*flag),
                        other => match number(other) {
                            Some(read) => Value::Number(read),
                            None => {
                                return Err((
                                    at,
                                    format!(
                                        "The property \"{name}\" has to be text in quotes, \
                                         a number, or true or false, like {name} = \"red\"."
                                    ),
                                ));
                            }
                        },
                    };
                    properties.insert(name.to_owned(), value);
                }
            }
            other => {
                return Err((
                    at,
                    format!(
                        "\"{other}\" is not something an entity has. An entity has id, \
                         type, name, position and facing; anything else goes under a line \
                         saying [entity.properties]."
                    ),
                ));
            }
        }
    }
    let first = if first == usize::MAX { start } else { first };
    let Some((id, id_at)) = id else {
        return Err((
            first,
            "This entity has no id. Give it one no other entity here has, like \
             id = \"red-spawn-1\"."
                .to_owned(),
        ));
    };
    let Some(kind) = kind else {
        return Err((
            id_at,
            format!("The entity \"{id}\" has no type. Say what it is, like type = \"spawn\"."),
        ));
    };
    let Some(position) = position else {
        return Err((
            id_at,
            format!(
                "The entity \"{id}\" has no position. Say where it stands, like \
                 position = [4.0, 0.0, -12.5]."
            ),
        ));
    };
    let entity = Entity {
        id,
        kind,
        name,
        position,
        facing,
        properties,
    };
    Ok((entity, id_at))
}

/// The line, counted from 1, of the byte at `offset`.
fn line_of(text: &str, offset: usize) -> usize {
    let before = text.as_bytes().get(..offset).unwrap_or(text.as_bytes());
    before.iter().filter(|&&byte| byte == b'\n').count() + 1
}

/// `value` to the nearest `1 / per`, and never "-0". Something that is not
/// a number is written as 0, so what is written can always be read back.
fn rounded(value: f32, per: f64) -> f64 {
    if value.is_finite() {
        (f64::from(value) * per).round() / per + 0.0
    } else {
        0.0
    }
}

/// `degrees` as the same facing from -180 (not included) to 180.
fn wrapped(degrees: f32) -> f32 {
    let turned = degrees.rem_euclid(360.0);
    if turned > 180.0 {
        turned - 360.0
    } else {
        turned
    }
}

/// Writes `text` as TOML text in quotes.
fn write_string(into: &mut String, text: &str) {
    into.push('"');
    for c in text.chars() {
        match c {
            '"' => into.push_str("\\\""),
            '\\' => into.push_str("\\\\"),
            '\n' => into.push_str("\\n"),
            '\t' => into.push_str("\\t"),
            c if c.is_control() => {
                // Writing to a `String` cannot fail.
                let _ = write!(into, "\\u{:04X}", u32::from(c));
            }
            c => into.push(c),
        }
    }
    into.push('"');
}

/// Writes a property's name as a TOML key, in quotes only when it needs them.
fn write_key(into: &mut String, name: &str) {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        into.push_str(name);
    } else {
        write_string(into, name);
    }
}

/// Writes a property's value as TOML.
fn write_value(into: &mut String, value: &Value) {
    match value {
        Value::Text(text) => write_string(into, text),
        Value::Flag(flag) => into.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => {
            // Writing to a `String` cannot fail. A whole number is written
            // as one, so `rounds = 3` stays as it was typed.
            let whole = number.fract() == 0.0 && number.abs() < 1e15;
            let _ = if whole {
                write!(into, "{number:.0}")
            } else if number.is_finite() {
                write!(into, "{number:?}")
            } else {
                // Not something the reader lets in; written as nothing.
                write!(into, "0")
            };
        }
    }
}

/// What a file looked like at one moment: when it was last written and how
/// long it is.
type Stamp = (SystemTime, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    let about = fs::metadata(path).ok()?;
    Some((about.modified().ok()?, about.len()))
}

/// Reads the map file at `path`, or `None` if there is no such file.
fn read_file(path: &Path) -> Result<Option<Map>, MapError> {
    let failed = |source| MapError::Read {
        path: path.to_owned(),
        source,
    };
    let name = path.display().to_string();
    match fs::metadata(path) {
        Ok(about) if about.len() > LONGEST_FILE => return Err(MapError::TooLong { file: name }),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    }
    match fs::read_to_string(path) {
        Ok(text) => Map::parse(&name, &text).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failed(error)),
    }
}

/// A game's map file on disk: saves to it, and notices when something else
/// has saved it.
#[derive(Debug)]
pub struct MapFile {
    path: PathBuf,
    /// The file as it was when last read or written here.
    known: Option<Stamp>,
    /// A newer state seen once, waiting to be seen again.
    pending: Option<Stamp>,
    next_poll: Instant,
}

impl MapFile {
    /// The map file of the game in `folder`, and the map read from it: an
    /// empty one if the game has no map file yet.
    ///
    /// When the file cannot be read as a map the mistake is handed back in
    /// place of a map, and the file is still watched, so fixing and saving
    /// it is picked up by [`changed`](Self::changed).
    pub fn open(folder: &Path) -> (Self, Result<Map, MapError>) {
        let path = folder.join(MAP_FILE);
        let known = stamp(&path);
        let map = read_file(&path).map(Option::unwrap_or_default);
        let file = Self {
            path,
            known,
            pending: None,
            next_poll: Instant::now() + POLL_EVERY,
        };
        (file, map)
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The map as it now is on disk, if the file was saved by something
    /// else since it was last read or written here. `None` when nothing
    /// changed.
    ///
    /// Call this every tick or frame; it looks at the file at most ten times
    /// a second. A change must look the same on two looks in a row, so a
    /// file caught half-written is not read, and a file that is gone (some
    /// tools save by deleting and renaming) is not a change: the map stays
    /// as it was.
    pub fn changed(&mut self) -> Option<Result<Map, MapError>> {
        let now = Instant::now();
        if now < self.next_poll {
            return None;
        }
        self.next_poll = now + POLL_EVERY;
        let seen = stamp(&self.path);
        if seen.is_none() || seen == self.known {
            self.pending = None;
            None
        } else if seen == self.pending {
            self.known = seen;
            self.pending = None;
            // Gone between the look and the read: the next look decides.
            read_file(&self.path).transpose()
        } else {
            self.pending = seen;
            None
        }
    }

    /// Whether the file was saved by something else since it was last read
    /// or written here, so that a [`save`](Self::save) now would replace
    /// that. Unlike [`changed`](Self::changed) this looks at once and reads
    /// nothing.
    #[must_use]
    pub fn saved_elsewhere(&self) -> bool {
        stamp(&self.path).is_some_and(|seen| Some(seen) != self.known)
    }

    /// Writes `map` to the file. The text goes to a second file first, which
    /// then takes the map file's place in one step: something reading the
    /// map file meanwhile sees the old one or the new one, never half of
    /// either.
    ///
    /// # Errors
    ///
    /// Fails if the game's folder cannot be written to. The map file is
    /// then as it was.
    pub fn save(&mut self, map: &Map) -> Result<(), MapError> {
        let failed = |source| MapError::Write {
            path: self.path.clone(),
            source,
        };
        let saving = self.path.with_file_name(SAVING_FILE);
        let written = fs::File::create(&saving).and_then(|mut file| {
            file.write_all(map.to_text().as_bytes())?;
            // On disk before it takes the map file's place, so a power cut
            // cannot leave an empty map file.
            file.sync_all()
        });
        if let Err(error) = written.and_then(|()| fs::rename(&saving, &self.path)) {
            let _ = fs::remove_file(&saving);
            return Err(failed(error));
        }
        // Our own save is not a change to pick up.
        self.known = stamp(&self.path);
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "\
version = 1

[[entity]]
id = \"red-1\"
type = \"spawn\"
name = \"Red spawn\"
position = [4, 0.0, -12.5]
facing = 180

[entity.properties]
team = \"red\"
weight = 2
\"needs key\" = true
ratio = 0.25

[[entity]]
id = \"a-zone\"
type = \"zone\"
position = [0.1, 0.0004, 1e2]
";

    /// An empty folder for one test.
    fn scratch(test: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("fossps-map-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// What reading `text` fails with.
    fn said(text: &str) -> String {
        Map::parse("map.toml", text).unwrap_err().to_string()
    }

    #[test]
    fn a_map_file_is_read_into_entities_in_the_order_of_their_ids() {
        let map = Map::parse("map.toml", TWO).unwrap();
        let [zone, spawn] = map.entities() else {
            panic!("{map:?}");
        };
        assert_eq!(
            (zone.id.as_str(), zone.kind.as_str(), zone.name.as_str()),
            ("a-zone", "zone", "")
        );
        assert_eq!(zone.position, [0.1, 0.0004, 100.0]);
        assert_eq!(zone.facing, 0.0);
        assert!(zone.properties.is_empty());
        assert_eq!(spawn.id, "red-1");
        assert_eq!(spawn.name, "Red spawn");
        assert_eq!(spawn.position, [4.0, 0.0, -12.5]);
        assert_eq!(spawn.facing, 180.0);
        let properties: Vec<(&str, &Value)> = spawn
            .properties
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect();
        assert_eq!(
            properties,
            [
                ("needs key", &Value::Flag(true)),
                ("ratio", &Value::Number(0.25)),
                ("team", &Value::Text("red".to_owned())),
                ("weight", &Value::Number(2.0)),
            ]
        );
        assert_eq!(map.spawns().count(), 1);
        assert_eq!(map.summary(), "2 placed: 1 spawn, 1 zone");
        assert_eq!(map.get("red-1"), Some(spawn));
        assert_eq!(map.get("red-2"), None);

        // A file with nothing placed is a map with nothing in it.
        let empty = Map::parse("map.toml", "version = 1\n").unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.summary(), "0 placed");
    }

    #[test]
    fn the_text_written_is_the_same_each_time_and_reads_back_the_same() {
        let map = Map::parse("map.toml", TWO).unwrap();
        let text = map.to_text();
        let expected = "\
version = 1

[[entity]]
id = \"a-zone\"
type = \"zone\"
position = [0.1, 0.0, 100.0]
facing = 0.0

[[entity]]
id = \"red-1\"
type = \"spawn\"
name = \"Red spawn\"
position = [4.0, 0.0, -12.5]
facing = 180.0

[entity.properties]
\"needs key\" = true
ratio = 0.25
team = \"red\"
weight = 2
";
        assert_eq!(text, format!("{HEADER}\n{expected}"));
        // Written again from what was read, nothing moves: a save that
        // changed nothing changes no line.
        let again = Map::parse("map.toml", &text).unwrap();
        assert_eq!(again.to_text(), text);
        assert_eq!(again.entities()[1], map.entities()[1]);
    }

    #[test]
    fn numbers_are_written_without_noise_and_text_with_its_marks_kept() {
        let mut map = Map::default();
        let id = map
            .place("spawn", [0.1 + 0.2, -0.0, 1.0 / 3.0], 450.0)
            .unwrap()
            .to_owned();
        let entity = map.get_mut(&id).unwrap();
        entity.name = "The \"big\" one\\\n\u{7}".to_owned();
        entity
            .properties
            .insert("tiny".to_owned(), Value::Number(1.0e-7));
        entity
            .properties
            .insert("huge".to_owned(), Value::Number(1.0e20));
        entity
            .properties
            .insert("say \"hi\"".to_owned(), Value::Text("tab\there".to_owned()));
        let text = map.to_text();
        assert!(text.contains("position = [0.3, 0.0, 0.333]\n"), "{text}");
        assert!(text.contains("facing = 90.0\n"), "{text}");
        assert!(
            text.contains("name = \"The \\\"big\\\" one\\\\\\n\\u0007\"\n"),
            "{text}"
        );
        let read = Map::parse("map.toml", &text).unwrap();
        let back = read.get(&id).unwrap();
        assert_eq!(back.name, "The \"big\" one\\\n\u{7}");
        assert_eq!(back.properties["tiny"], Value::Number(1.0e-7));
        assert_eq!(back.properties["huge"], Value::Number(1.0e20));
        assert_eq!(
            back.properties["say \"hi\""],
            Value::Text("tab\there".to_owned())
        );
        assert_eq!(read.to_text(), text);
        assert_eq!(Value::Number(3.0).to_string(), "3");
        assert_eq!(Value::Flag(false).to_string(), "false");

        // A facing is the same facing however many turns it was given in.
        for (given, kept) in [
            (180.0, 180.0),
            (-180.0, 180.0),
            (270.0, -90.0),
            (-45.0, -45.0),
        ] {
            assert_eq!(wrapped(given), kept, "{given}");
        }
    }

    #[test]
    fn placing_makes_an_id_of_its_own_and_keeps_the_order() {
        let mut map = Map::default();
        let mut ids = Vec::new();
        for number in 0..200_u8 {
            let id = map
                .place("spawn", [f32::from(number), 0.0, 0.0], 0.0)
                .unwrap()
                .to_owned();
            assert_eq!(id.len(), ID_LENGTH);
            assert!(
                id.bytes().all(|letter| ID_LETTERS.contains(&letter)),
                "{id}"
            );
            ids.push(id);
        }
        assert_eq!(map.entities().len(), 200);
        assert!(
            map.entities()
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
        );
        // Each is found by its ID, where it was put.
        for (number, id) in (0..200_u8).zip(&ids) {
            assert_eq!(map.get(id).unwrap().position[0], f32::from(number));
        }
        let gone = map.remove(&ids[7]).unwrap();
        assert_eq!(gone.position[0], 7.0);
        assert_eq!(map.get(&ids[7]), None);
        assert_eq!(map.remove(&ids[7]), None);

        // Numbers that are not numbers, and places off the map, are tamed.
        let id = map
            .place("zone", [f32::NAN, f32::INFINITY, 1.0e9], f32::NAN)
            .unwrap()
            .to_owned();
        let placed = map.get(&id).unwrap();
        assert_eq!(placed.position, [0.0, 0.0, FURTHEST]);
        assert_eq!(placed.facing, 0.0);
        assert_eq!(map.place("", [0.0; 3], 0.0), None);
        assert_eq!(map.place("a\nb", [0.0; 3], 0.0), None);
        assert!(Map::parse("map.toml", &map.to_text()).is_ok());
    }

    #[test]
    fn a_mistake_names_its_line_and_says_what_to_write() {
        let text = "version = 1\n\n[[entity]]\nid = \"a\"\ntype = \"spawn\"\npositon = [0, 0, 0]\n";
        assert_eq!(
            said(text),
            "map.toml, line 6: \"positon\" is not something an entity has. An entity has \
             id, type, name, position and facing; anything else goes under a line saying \
             [entity.properties]."
        );
        let entity = |lines: &str| format!("version = 1\n[[entity]]\n{lines}");
        for (lines, line, expected) in [
            ("type = \"spawn\"\nposition = [0, 0, 0]\n", 3, "has no id"),
            ("id = \"a\"\nposition = [0, 0, 0]\n", 3, "\"a\" has no type"),
            ("id = \"a\"\ntype = \"spawn\"\n", 3, "\"a\" has no position"),
            ("id = 7\n", 3, "An id has to be text in quotes"),
            ("id = \"\"\n", 3, "This id is empty"),
            ("id = \"a\"\ntype = \" \"\n", 4, "This type is empty"),
            (
                "id = \"a\"\ntype = \"spawn \"\n",
                4,
                "This type starts or ends with a space",
            ),
            ("id = \" a\"\n", 3, "This id starts or ends with a space"),
            (
                "id = \"a\"\nposition = [0, 0]\n",
                4,
                "three numbers in metres",
            ),
            (
                "id = \"a\"\nposition = [0, \"0\", 0, 0]\n",
                4,
                "three numbers",
            ),
            ("id = \"a\"\nposition = [0, nan, 0]\n", 4, "three numbers"),
            ("id = \"a\"\nposition = [0, 1e9, 0]\n", 4, "within 100000"),
            ("id = \"a\"\nfacing = \"north\"\n", 4, "in degrees"),
            ("id = \"a\"\nfacing = inf\n", 4, "in degrees"),
            ("id = \"a\"\nproperties = 3\n", 4, "[entity.properties]"),
            (
                "id = \"a\"\n[entity.properties]\nteam = [1]\n",
                5,
                "\"team\" has to be text in quotes, a number, or true or false",
            ),
            (
                "id = \"a\"\n[entity.properties]\nwhen = 1979-05-27\n",
                5,
                "\"when\" has to be text",
            ),
        ] {
            let said = said(&entity(lines));
            assert!(
                said.starts_with(&format!("map.toml, line {line}: ")) && said.contains(expected),
                "{lines:?} gave: {said}"
            );
        }

        let twice = "version = 1\n[[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [0, 0, 0]\n\
                     [[entity]]\nid = \"a\"\ntype = \"zone\"\nposition = [1, 0, 0]\n";
        assert_eq!(
            said(twice),
            "map.toml, line 7: The id \"a\" is also used on line 3. Every entity needs an \
             id of its own: change one of the two."
        );
    }

    #[test]
    fn a_file_that_is_not_a_map_says_so() {
        let syntax = said("version = 1\n[[entity]\n");
        assert!(syntax.starts_with("map.toml, line 2: "), "{syntax}");
        assert!(syntax.contains("The file is TOML"), "{syntax}");

        let no_version = said("[[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [0, 0, 0]\n");
        assert!(
            no_version.contains("Add this as its first line: version = 1"),
            "{no_version}"
        );
        let newer = said("version = 2\nsky = \"dusk\"\n");
        assert_eq!(
            newer,
            "map.toml says version = 2, but this version of FOSSPS reads map files up to \
             version 1. It was saved by a newer FOSSPS: update FOSSPS to open it."
        );
        for text in ["version = 0\n", "version = \"1\"\n", "version = 1.0\n"] {
            assert!(
                said(text).contains("version has to be a whole number"),
                "{text}"
            );
        }
        assert!(said("version = 1\nentity = 3\n").contains("[[entity]]"));
        assert!(said("version = 1\n[entity]\nid = \"a\"\n").contains("double square"));
        assert!(said("version = 1\nsky = 1\n").contains("\"sky\" is not something a map file has"));

        let many = format!("version = 1\n{}", "[[entity]]\n".repeat(MOST_ENTITIES + 1));
        assert!(said(&many).contains("a map holds 10000 at most"));
        let long = "#".repeat(LONGEST_FILE as usize + 1);
        assert!(said(&long).contains("longer than 4 MB"));
        let word = "w".repeat(LONGEST_WORD + 1);
        let long_id = format!("version = 1\n[[entity]]\nid = \"{word}\"\n");
        assert!(said(&long_id).contains("longer than 64 characters"));
    }

    #[test]
    fn the_biggest_map_there_can_be_is_read_quickly() {
        // As long as a map file may be, with as many entities as it may
        // hold at its end: read in one pass, not once for each entity.
        let mut text = String::from("version = 1\n");
        let entities: String = (0..MOST_ENTITIES)
            .map(|number| {
                format!("[[entity]]\nid = \"e{number}\"\ntype = \"spawn\"\nposition = [0, 0, 0]\n")
            })
            .collect();
        let padding = LONGEST_FILE as usize - text.len() - entities.len();
        text.push_str(&"# padding\n".repeat(padding / 10));
        text.push_str(&entities);
        let started = Instant::now();
        let map = Map::parse("map.toml", &text).unwrap();
        assert_eq!(map.entities().len(), MOST_ENTITIES);
        // Seconds even unoptimised; counting lines for each entity took
        // minutes.
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_game_with_no_map_file_has_an_empty_map_and_a_save_makes_one() {
        let folder = scratch("save");
        assert!(Map::load(&folder).unwrap().is_empty());
        let (mut file, map) = MapFile::open(&folder);
        let mut map = map.unwrap();
        assert!(map.is_empty());
        assert!(!file.path().exists(), "nothing is written until asked");

        map.place("spawn", [1.0, 0.0, 2.0], 90.0).unwrap();
        file.save(&map).unwrap();
        assert_eq!(fs::read_to_string(file.path()).unwrap(), map.to_text());
        assert_eq!(Map::load(&folder).unwrap(), map);
        // Nothing is left beside it.
        let names: Vec<_> = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [MAP_FILE]);

        // A save over a file that is there replaces all of it.
        map.place("zone", [0.0; 3], 0.0).unwrap();
        file.save(&map).unwrap();
        assert_eq!(Map::load(&folder).unwrap().entities().len(), 2);

        // A folder that is not there cannot be saved into, and says so.
        let (mut nowhere, _) = MapFile::open(&folder.join("missing"));
        let said = nowhere.save(&map).unwrap_err().to_string();
        assert!(said.contains("could not be saved"), "{said}");
        assert!(said.contains("can be written to"), "{said}");
        let _ = fs::remove_dir_all(&folder);
    }

    /// Looks at `file` until it reports a change, for twenty seconds at most.
    fn wait_for_change(file: &mut MapFile) -> Result<Map, MapError> {
        let give_up = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(change) = file.changed() {
                return change;
            }
            assert!(Instant::now() < give_up, "the save was never noticed");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_save_by_something_else_is_noticed_and_our_own_is_not() {
        let folder = scratch("watch");
        let (mut file, map) = MapFile::open(&folder);
        let mut map = map.unwrap();

        // Our own save is not news.
        map.place("spawn", [1.0, 0.0, 2.0], 0.0).unwrap();
        file.save(&map).unwrap();
        for _ in 0..4 {
            std::thread::sleep(POLL_EVERY);
            assert!(file.changed().is_none());
        }

        // Someone else's is, whether the file is written in place...
        fs::write(file.path(), TWO).unwrap();
        let read = wait_for_change(&mut file).unwrap();
        assert_eq!(read.entities().len(), 2);
        assert!(file.changed().is_none(), "it is told once");

        // ...or replaced, as our own save does it, from another window.
        let (mut other, _) = MapFile::open(&folder);
        other.save(&map).unwrap();
        assert_eq!(wait_for_change(&mut file).unwrap(), map);

        // A save with a mistake in it is handed over as the mistake.
        fs::write(file.path(), "version = 1\n[[entity]]\nid = 3\n").unwrap();
        let said = wait_for_change(&mut file).unwrap_err().to_string();
        assert!(
            said.contains("map.toml, line 3: An id has to be text"),
            "{said}"
        );

        // Looked at without waiting, a save from elsewhere shows at once,
        // and our own save over it settles it.
        assert!(!file.saved_elsewhere());
        fs::write(file.path(), TWO).unwrap();
        assert!(file.saved_elsewhere());
        file.save(&map).unwrap();
        assert!(!file.saved_elsewhere());

        // A file that is gone is not a change.
        fs::remove_file(file.path()).unwrap();
        for _ in 0..3 {
            std::thread::sleep(POLL_EVERY);
            assert!(file.changed().is_none());
        }
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_map_file_with_a_mistake_is_still_watched() {
        let folder = scratch("broken");
        fs::write(folder.join(MAP_FILE), "version = 1\nsky = 1\n").unwrap();
        assert!(Map::load(&folder).is_err());
        let (mut file, map) = MapFile::open(&folder);
        assert!(map.is_err());
        fs::write(file.path(), TWO).unwrap();
        assert_eq!(wait_for_change(&mut file).unwrap().entities().len(), 2);
        let _ = fs::remove_dir_all(&folder);
    }
}
