//! Plain-English names for inputs, and "did you mean" suggestions for typos.

use std::fmt;

/// A name that did not match any known input or action.
///
/// The message lists what was typed, what kind of thing was expected and,
/// when a close match exists, the name the author probably meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownName {
    /// What kind of name was expected, for example `"key"`.
    pub kind: &'static str,
    /// The text that was not recognised.
    pub given: String,
    /// The closest valid name, if one is close enough to be a likely typo.
    pub suggestion: Option<String>,
}

impl fmt::Display for UnknownName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\" is not a known {}.", self.given, self.kind)?;
        match &self.suggestion {
            Some(name) => write!(f, " Did you mean \"{name}\"?"),
            None if matches!(self.kind, "binding" | "key or button") => f.write_str(
                " Names look like \"Space\", \"MouseLeft\", \"GamepadSouth\" or \"WASD\".",
            ),
            None => Ok(()),
        }
    }
}

impl std::error::Error for UnknownName {}

/// Builds the error for `given`, suggesting the closest of `candidates`.
pub(crate) fn unknown<'a>(
    kind: &'static str,
    given: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> UnknownName {
    UnknownName {
        kind,
        given: given.to_owned(),
        suggestion: closest(given, candidates).map(str::to_owned),
    }
}

/// The candidate nearest to `given`, if it is near enough to be a typo.
fn closest<'a>(given: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let given = given.to_ascii_lowercase();
    // Allow roughly one mistake per three characters, and always at least one.
    let limit = (given.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|name| (edit_distance(&given, &name.to_ascii_lowercase()), name))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, name)| name)
}

/// Edit distance between two strings, counted in characters.
///
/// Swapping two neighbouring characters counts as one edit, because that is
/// the most common typo ("jmup" for "jump").
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let width = b.len() + 1;
    // A flat (a.len() + 1) by (b.len() + 1) table of distances between prefixes.
    let mut table = vec![0_usize; (a.len() + 1) * width];
    for (j, cell) in table.iter_mut().take(width).enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        table[i * width] = i;
        for j in 1..=b.len() {
            let substitute = table[(i - 1) * width + j - 1] + usize::from(a[i - 1] != b[j - 1]);
            let insert_or_delete = table[(i - 1) * width + j].min(table[i * width + j - 1]) + 1;
            let mut best = substitute.min(insert_or_delete);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[(i - 2) * width + j - 2] + 1);
            }
            table[i * width + j] = best;
        }
    }
    table[a.len() * width + b.len()]
}

/// Declares a field-less enum whose variants have plain-English names.
///
/// Generates `ALL`, `COUNT`, `name`, `index`, `Display` and a case-insensitive
/// `FromStr` that suggests the closest name on a typo.
macro_rules! named_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident ($kind:literal) {
            $( $(#[$vmeta:meta])* $variant:ident ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// How many values there are.
            pub const COUNT: usize = Self::ALL.len();

            /// The name used in bindings and shown to players.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => stringify!($variant)),+ }
            }

            /// Position in [`Self::ALL`], for use as an array index.
            #[must_use]
            pub const fn index(self) -> usize {
                self as usize
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::names::UnknownName;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|value| value.name().eq_ignore_ascii_case(text))
                    .ok_or_else(|| {
                        $crate::names::unknown($kind, text, Self::ALL.iter().map(|v| v.name()))
                    })
            }
        }
    };
}

pub(crate) use named_enum;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_counts_single_edits() {
        assert_eq!(edit_distance("space", "space"), 0);
        assert_eq!(edit_distance("spcae", "space"), 1);
        assert_eq!(edit_distance("spxxe", "space"), 2);
        assert_eq!(edit_distance("spac", "space"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn suggests_only_near_matches() {
        let names = ["Space", "LeftShift", "Enter"];
        assert_eq!(closest("spase", names), Some("Space"));
        assert_eq!(closest("leftshfit", names), Some("LeftShift"));
        assert_eq!(closest("banana", names), None);
    }

    #[test]
    fn message_says_what_to_do_next() {
        let error = unknown("key", "Spase", ["Space", "Enter"]);
        assert_eq!(
            error.to_string(),
            "\"Spase\" is not a known key. Did you mean \"Space\"?"
        );
    }
}
