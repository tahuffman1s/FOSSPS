//! Script errors written for the person who has to fix them.

use std::fmt;
use std::path::PathBuf;

/// The marker a time-limit error carries so it can be told apart from the
/// script's own errors.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TookTooLong {
    /// The script line that was running when time ran out.
    pub line: Option<u32>,
}

impl fmt::Display for TookTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the script ran for too long without finishing")
    }
}

impl std::error::Error for TookTooLong {}

/// Something went wrong in a game script.
///
/// Printing it shows the file and line, the line's text, what went wrong and,
/// where the engine can tell, what to do about it:
///
/// ```text
/// Script error in main.luau, line 4:
///
///     4 | game.onTik(function(step)
///
///   game.onTik does not exist. Did you mean game.onTick?
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    /// The script's file name, such as `main.luau`.
    pub file: String,
    /// The line the error happened on, counting from 1, when known.
    pub line: Option<u32>,
    /// What went wrong.
    pub message: String,
    /// The text of that line.
    pub source_line: Option<String>,
    /// What to try next.
    pub hint: Option<String>,
    /// What the engine did about it, such as keeping the previous version running.
    pub note: Option<String>,
}

impl ScriptError {
    /// Explains `error`, raised by the script called `file` whose text is `source`.
    pub(crate) fn from_lua(error: &mlua::Error, file: &str, source: &str) -> Self {
        let root = root_cause(error);
        let too_long = match root {
            mlua::Error::ExternalError(inner) => inner.downcast_ref::<TookTooLong>(),
            _ => None,
        };
        let raw = match root {
            mlua::Error::SyntaxError { message, .. } => message.clone(),
            mlua::Error::RuntimeError(message) | mlua::Error::MemoryError(message) => {
                message.clone()
            }
            other => other.to_string(),
        };
        // Anything after the first line is the VM's own stack listing.
        let first_line = raw.lines().next().unwrap_or_default();
        let (line, message) = split_location(first_line, file);
        let line = line.or(too_long.and_then(|limit| limit.line));

        let (message, hint) = if too_long.is_some() {
            (
                "The script ran for too long without finishing, so it was stopped.".to_owned(),
                Some(
                    "Look for a `while` or `repeat` loop near this line that never ends. \
                     Work that takes a long time should be spread over several ticks."
                        .to_owned(),
                ),
            )
        } else if matches!(root, mlua::Error::MemoryError(_)) || message == "not enough memory" {
            (
                "The script used more memory than it is allowed.".to_owned(),
                Some(
                    "Look for a table that keeps growing, such as one that gets a new entry \
                     every tick and is never cleared."
                        .to_owned(),
                ),
            )
        } else {
            let hint = hint_for(message);
            (message.to_owned(), hint)
        };

        Self {
            file: file.to_owned(),
            line,
            message,
            source_line: line.and_then(|line| nth_line(source, line)),
            hint,
            note: None,
        }
    }

    /// The same error, with a line saying what the engine did about it.
    pub(crate) fn with_note(mut self, note: &str) -> Self {
        self.note = Some(note.to_owned());
        self
    }

    /// A problem with the script file itself rather than with a line in it.
    pub(crate) fn file_problem(file: &str, message: String) -> Self {
        Self {
            file: file.to_owned(),
            line: None,
            message,
            source_line: None,
            hint: None,
            note: None,
        }
    }
}

/// Whether `error` is the engine stopping a script that ran too long.
pub(crate) fn is_time_limit(error: &mlua::Error) -> bool {
    matches!(root_cause(error), mlua::Error::ExternalError(inner) if inner.is::<TookTooLong>())
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => writeln!(f, "Script error in {}, line {line}:", self.file)?,
            None => writeln!(f, "Script error in {}:", self.file)?,
        }
        if let (Some(line), Some(text)) = (self.line, &self.source_line) {
            write!(f, "\n  {line:>4} | {}\n", text.trim_end())?;
        }
        write!(f, "\n  {}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  Hint: {hint}")?;
        }
        if let Some(note) = &self.note {
            write!(f, "\n  {note}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ScriptError {}

/// Why a game's script could not be opened at all.
#[derive(Debug)]
pub enum OpenError {
    /// The game folder has no `main.luau`.
    Missing {
        /// Where the script was expected.
        path: PathBuf,
    },
    /// `main.luau` exists but could not be read.
    Unreadable {
        /// The script's path.
        path: PathBuf,
        /// What the operating system reported.
        source: std::io::Error,
    },
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => write!(
                f,
                "There is no game script at {}. Every game starts from a file called \
                 main.luau: create it there, or pass the folder that contains your game.",
                path.display()
            ),
            Self::Unreadable { path, source } => write!(
                f,
                "Could not read {}: {source}. Check that the file is saved as plain text \
                 (UTF-8) and that you are allowed to read it.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Missing { .. } => None,
            Self::Unreadable { source, .. } => Some(source),
        }
    }
}

/// Follows callback wrappers down to the error that started it all.
fn root_cause(mut error: &mlua::Error) -> &mlua::Error {
    loop {
        match error {
            mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => {
                error = cause;
            }
            other => return other,
        }
    }
}

/// Splits `main.luau:12: message` into the line number and the message.
fn split_location<'a>(message: &'a str, file: &str) -> (Option<u32>, &'a str) {
    let parsed = message
        .strip_prefix(file)
        .and_then(|rest| rest.strip_prefix(':'))
        .and_then(|rest| rest.split_once(':'))
        .and_then(|(line, rest)| Some((line.parse().ok()?, rest.trim_start())));
    match parsed {
        Some((line, rest)) => (Some(line), rest),
        None => (None, message),
    }
}

/// The text of line `line` (counting from 1), unless it is blank.
fn nth_line(source: &str, line: u32) -> Option<String> {
    let index = usize::try_from(line).ok()?.checked_sub(1)?;
    let text = source.lines().nth(index)?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Advice for the errors beginners meet most often.
fn hint_for(message: &str) -> Option<String> {
    const HINTS: &[(&str, &str)] = &[
        (
            "attempt to call a nil value",
            "This line calls a function that does not exist. Check how its name is spelled \
             and that it is defined above this line.",
        ),
        (
            "attempt to index nil",
            "The value in front of the `.` or `[` is nil (empty) here. Check that it was \
             given a value before this line and that its name is spelled correctly.",
        ),
        (
            "attempt to perform arithmetic",
            "One of the values in this calculation is not a number. Check that every \
             variable in it was given a value first.",
        ),
        (
            "attempt to compare",
            "The two values being compared are different kinds of thing, or one is nil. \
             Check that both were given a value first.",
        ),
        (
            "attempt to concatenate",
            "`..` joins text. Wrap other values in tostring(), like tostring(score).",
        ),
        (
            "attempt to modify a readonly table",
            "Built-in tables such as `game` and `math` cannot be changed. Keep your own \
             values in a `local` variable instead.",
        ),
        (
            "to close",
            "Every `function`, `if`, `for` and `while` needs its own `end`. The message \
             names the line where the unclosed block starts: add the missing `end` after \
             that block's last line.",
        ),
        (
            "Incomplete statement",
            "A line here names something without doing anything with it. To call a \
             function, add brackets: `name()`.",
        ),
    ];
    HINTS
        .iter()
        .find(|(pattern, _)| message.contains(pattern))
        .map(|(_, hint)| (*hint).to_owned())
}

/// The candidate closest to `name`, if one is close enough to be a likely typo.
pub(crate) fn closest<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let wanted = name.to_lowercase();
    let allowed = (wanted.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|candidate| (edit_distance(&wanted, &candidate.to_lowercase()), candidate))
        .filter(|(distance, _)| *distance <= allowed)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

/// How many single-character edits turn `a` into `b`.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, from) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, to) in b.iter().enumerate() {
            let substitute = diagonal + usize::from(from != *to);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_is_split_from_the_message() {
        assert_eq!(
            split_location("main.luau:12: attempt to call a nil value", "main.luau"),
            (Some(12), "attempt to call a nil value")
        );
        assert_eq!(
            split_location("something else", "main.luau"),
            (None, "something else")
        );
    }

    #[test]
    fn close_names_are_suggested_and_distant_ones_are_not() {
        let names = ["onStart", "onTick", "onReload", "input"];
        assert_eq!(closest("onTik", names), Some("onTick"));
        assert_eq!(closest("ontick", names), Some("onTick"));
        assert_eq!(closest("spawnBot", names), None);
    }

    #[test]
    fn edit_distance_counts_edits() {
        assert_eq!(edit_distance("fire", "fire"), 0);
        assert_eq!(edit_distance("fier", "fire"), 2);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn display_shows_place_line_message_and_hint() {
        let error = ScriptError {
            file: "main.luau".to_owned(),
            line: Some(4),
            message: "attempt to call a nil value".to_owned(),
            source_line: Some("spwanBot()".to_owned()),
            hint: Some("Check the spelling.".to_owned()),
            note: Some("The game keeps running.".to_owned()),
        };
        assert_eq!(
            error.to_string(),
            "Script error in main.luau, line 4:\n\n     4 | spwanBot()\n\n  \
             attempt to call a nil value\n  Hint: Check the spelling.\n  The game keeps running."
        );
    }
}
