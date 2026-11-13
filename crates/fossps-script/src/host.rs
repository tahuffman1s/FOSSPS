//! Runs a game's `main.luau` and swaps in a new version when the file is saved.

// A script error carries the whole explanation by value. It is only built when
// a script fails to load, never on the tick path, so its size does not matter.
#![allow(clippy::result_large_err)]

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use fossps_input::{ActionMap, InputFrame};
use mlua::{Function, IntoLuaMulti, Lua, VmState};

use crate::api::{self, Handlers, Shared};
use crate::error::{OpenError, ScriptError, TookTooLong, is_time_limit};

/// The file every game starts from, inside the game's folder.
pub const ENTRY_SCRIPT: &str = "main.luau";

/// How long one call into the script may run before it is stopped. A healthy
/// tick takes well under a millisecond; this only catches loops that never end.
const TIME_LIMIT: Duration = Duration::from_secs(1);

/// How much memory one script may use.
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// How often the script file is checked for a new save.
const POLL_EVERY: Duration = Duration::from_millis(100);

/// How many different runtime errors are printed before the rest are dropped,
/// so a script failing in new ways every tick cannot flood the log.
const MAX_REPORTED: usize = 32;

/// The time limit reads the clock once in this many checks.
const CLOCK_EVERY: u32 = 256;

const KEEPS_RUNNING: &str =
    "The game keeps running. Fix this and save the file: it reloads by itself.";
const KEEPS_PREVIOUS: &str = "The game keeps running the last version that worked. \
     Fix this and save the file: it reloads by itself.";
const NOTHING_RUNNING: &str = "Fix this and save the file: the game picks it up by itself.";
const PAUSED: &str =
    "The script is paused because it never finished. Save the file to start it again.";

/// One loaded version of the script, in its own VM.
struct Version {
    /// Owns every function in `shared`; they stop working once it is dropped.
    _vm: Lua,
    shared: Rc<Shared>,
    source: String,
}

/// What a script file looked like on disk: when it was written and how long it is.
type Stamp = (SystemTime, u64);

/// Decides when a file has been saved and the save has finished.
#[derive(Debug, Default)]
struct Watch {
    /// The file as it was when last loaded.
    loaded: Option<Stamp>,
    /// A newer state seen once, waiting to be seen again.
    pending: Option<Stamp>,
}

impl Watch {
    /// Takes what the file looks like now and says whether to reload it.
    ///
    /// A change must look the same on two checks in a row, so a file caught
    /// half-written by the editor is not loaded. A missing file (some editors
    /// save by deleting and renaming) is never a reason to reload.
    fn should_reload(&mut self, seen: Option<Stamp>) -> bool {
        if seen.is_none() || seen == self.loaded {
            self.pending = None;
            false
        } else if seen == self.pending {
            self.loaded = seen;
            self.pending = None;
            true
        } else {
            self.pending = seen;
            false
        }
    }
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Runs one game's script.
///
/// The script lives in its own sandboxed Luau VM: it can reach the `game`
/// table and Luau's standard library, and nothing else on the computer.
///
/// ```no_run
/// use fossps_input::{ActionMap, InputFrame};
/// use fossps_script::ScriptHost;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut host = ScriptHost::open("my-game", ActionMap::default_fps())?;
/// let input = InputFrame::default();
/// loop {
///     host.poll_reload();
///     host.tick(&input, 1.0 / 60.0);
/// }
/// # }
/// ```
pub struct ScriptHost {
    path: PathBuf,
    file: String,
    actions: Rc<ActionMap>,
    running: Option<Version>,
    /// Whether the `onStart` functions have been called.
    started: bool,
    /// Set when the script was stopped for running too long.
    paused: bool,
    last_error: Option<ScriptError>,
    /// Runtime errors already printed for this version.
    reported: HashSet<String>,
    watch: Watch,
    next_poll: Instant,
}

impl std::fmt::Debug for ScriptHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptHost")
            .field("path", &self.path)
            .field("running", &self.running.is_some())
            .field("last_error", &self.last_error)
            .finish_non_exhaustive()
    }
}

impl ScriptHost {
    /// Opens the game in `folder` and runs its `main.luau`.
    ///
    /// `actions` are the input actions the script can read by name.
    ///
    /// A mistake inside the script is not a failure here: it is logged, kept in
    /// [`last_error`](Self::last_error), and the host waits for the file to be
    /// fixed and saved.
    ///
    /// # Errors
    ///
    /// Fails if `folder` has no `main.luau` or the file cannot be read.
    pub fn open(folder: impl AsRef<Path>, actions: ActionMap) -> Result<Self, OpenError> {
        let path = folder.as_ref().join(ENTRY_SCRIPT);
        let seen = stamp(&path);
        let source = std::fs::read_to_string(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                OpenError::Missing { path: path.clone() }
            } else {
                OpenError::Unreadable {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let mut host = Self {
            path,
            file: ENTRY_SCRIPT.to_owned(),
            actions: Rc::new(actions),
            running: None,
            started: false,
            paused: false,
            last_error: None,
            reported: HashSet::new(),
            watch: Watch {
                loaded: seen,
                pending: None,
            },
            next_poll: Instant::now() + POLL_EVERY,
        };
        // A script error is already logged and kept in `last_error`.
        let _ = host.install(source);
        Ok(host)
    }

    /// The most recent script error, until a reload succeeds.
    #[must_use]
    pub fn last_error(&self) -> Option<&ScriptError> {
        self.last_error.as_ref()
    }

    /// Runs one simulation tick of `step` seconds with this tick's `input`.
    ///
    /// The first tick calls the script's `game.onStart` functions before its
    /// `game.onTick` ones. A function that fails is reported once and the
    /// others still run.
    pub fn tick(&mut self, input: &InputFrame, step: f32) {
        let Some(version) = &self.running else { return };
        if self.paused {
            return;
        }
        version.shared.input.borrow_mut().clone_from(input);
        if !self.started {
            self.started = true;
            self.call_all(|handlers| &handlers.start, ());
        }
        self.call_all(|handlers| &handlers.tick, step);
    }

    /// Reloads the script if it was saved since it was last loaded.
    ///
    /// Call this every tick or frame; it looks at the file at most ten times a
    /// second. See [`reload`](Self::reload) for what a reload does.
    pub fn poll_reload(&mut self) {
        let now = Instant::now();
        if now < self.next_poll {
            return;
        }
        self.next_poll = now + POLL_EVERY;
        if self.watch.should_reload(stamp(&self.path)) {
            // A script error is already logged and kept in `last_error`.
            let _ = self.reload();
        }
    }

    /// Loads the script from disk again, now.
    ///
    /// The new version starts fresh: its variables are reset and its handlers
    /// replace the old ones, then its `game.onReload` functions are called.
    /// `game.onStart` functions are not called again.
    ///
    /// # Errors
    ///
    /// If the new version has a mistake, the error is returned (and logged) and
    /// the previous version keeps running.
    pub fn reload(&mut self) -> Result<(), ScriptError> {
        self.watch.loaded = stamp(&self.path).or(self.watch.loaded);
        self.watch.pending = None;
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) => {
                let error = ScriptError::file_problem(
                    &self.file,
                    format!(
                        "Could not read {}: {error}. Check that the file still exists and \
                         is saved as plain text (UTF-8).",
                        self.path.display()
                    ),
                );
                return Err(self.failed_to_load(error));
            }
        };
        let had_version = self.running.is_some();
        self.install(source)?;
        if had_version {
            log::info!("Reloaded {}.", self.file);
        }
        if self.started {
            self.call_all(|handlers| &handlers.reload, ());
        }
        Ok(())
    }

    /// Builds a version from `source` and makes it the running one.
    fn install(&mut self, source: String) -> Result<(), ScriptError> {
        match self.build(source) {
            Ok(version) => {
                self.running = Some(version);
                self.paused = false;
                self.last_error = None;
                self.reported.clear();
                Ok(())
            }
            Err(error) => Err(self.failed_to_load(error)),
        }
    }

    /// Records and logs an error that stopped a new version from loading.
    fn failed_to_load(&mut self, error: ScriptError) -> ScriptError {
        let error = error.with_note(if self.running.is_some() {
            KEEPS_PREVIOUS
        } else {
            NOTHING_RUNNING
        });
        log::error!("{error}");
        self.last_error = Some(error.clone());
        error
    }

    /// Starts a fresh VM and runs `source` in it.
    fn build(&self, source: String) -> Result<Version, ScriptError> {
        let shared = Rc::new(Shared {
            file: self.file.clone(),
            actions: Rc::clone(&self.actions),
            handlers: RefCell::new(Handlers::default()),
            input: RefCell::new(InputFrame::default()),
            deadline: Cell::new(None),
            stopped: Cell::new(None),
            checks: Cell::new(0),
        });
        let lua = Lua::new();
        let run = || -> mlua::Result<()> {
            lua.set_memory_limit(MEMORY_LIMIT)?;
            api::install(&lua, &shared)?;
            lua.sandbox(true)?;
            let limit = Rc::clone(&shared);
            lua.set_interrupt(move |lua| {
                let Some(deadline) = limit.deadline.get() else {
                    return Ok(VmState::Continue);
                };
                // Once time is up every check fails, so a script that catches
                // the error with pcall still cannot carry on.
                let mut stopped = limit.stopped.get();
                if stopped.is_none() {
                    // This runs at every function call and loop turn, so the
                    // clock is only read now and then.
                    let checks = limit.checks.get().wrapping_add(1);
                    limit.checks.set(checks);
                    if !checks.is_multiple_of(CLOCK_EVERY) || Instant::now() < deadline {
                        return Ok(VmState::Continue);
                    }
                    let line = lua
                        .inspect_stack(0, |running| running.current_line())
                        .flatten()
                        .and_then(|line| u32::try_from(line).ok());
                    stopped = Some(TookTooLong { line });
                    limit.stopped.set(stopped);
                }
                match stopped {
                    Some(stopped) => Err(mlua::Error::external(stopped)),
                    None => Ok(VmState::Continue),
                }
            });
            let chunk = lua.load(&source).set_name(format!("@{}", self.file));
            within_time_limit(&shared, || chunk.exec())
        };
        match run() {
            Ok(()) => Ok(Version {
                _vm: lua,
                shared,
                source,
            }),
            Err(error) => Err(ScriptError::from_lua(&error, &self.file, &source)),
        }
    }

    /// Calls every function in one handler list, reporting the ones that fail.
    fn call_all(&mut self, list: fn(&Handlers) -> &Vec<Function>, args: impl IntoLuaMulti + Copy) {
        let Some(version) = &self.running else { return };
        // Handlers added while these run are first called on the next pass,
        // so a handler that keeps adding handlers cannot hold up the tick.
        let count = list(&version.shared.handlers.borrow()).len();
        for index in 0..count {
            // The list is borrowed only for this lookup, so a handler may
            // register more handlers while it runs.
            let handler = list(&version.shared.handlers.borrow()).get(index).cloned();
            let Some(handler) = handler else { break };
            let Err(error) = within_time_limit(&version.shared, || handler.call::<()>(args)) else {
                continue;
            };
            let too_long = is_time_limit(&error);
            let error = ScriptError::from_lua(&error, &self.file, &version.source)
                .with_note(if too_long { PAUSED } else { KEEPS_RUNNING });
            let text = error.to_string();
            if self.reported.len() < MAX_REPORTED && self.reported.insert(text.clone()) {
                log::error!("{text}");
            }
            self.last_error = Some(error);
            if too_long {
                // Calling it again would freeze the game for the whole time
                // limit on every tick.
                self.paused = true;
                break;
            }
        }
    }
}

/// Runs `call` with the time limit armed.
///
/// Running out of time is an error even if the script caught it with `pcall`
/// and returned normally.
fn within_time_limit<R>(
    shared: &Shared,
    call: impl FnOnce() -> mlua::Result<R>,
) -> mlua::Result<R> {
    shared.stopped.set(None);
    shared.deadline.set(Some(Instant::now() + TIME_LIMIT));
    let result = call();
    shared.deadline.set(None);
    match shared.stopped.take() {
        Some(stopped) => Err(mlua::Error::external(stopped)),
        None => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64, len: u64) -> Option<Stamp> {
        Some((SystemTime::UNIX_EPOCH + Duration::from_secs(seconds), len))
    }

    #[test]
    fn a_save_is_loaded_once_it_has_settled() {
        let mut watch = Watch {
            loaded: at(1, 10),
            pending: None,
        };
        assert!(!watch.should_reload(at(1, 10)), "nothing changed");
        assert!(
            !watch.should_reload(at(2, 4)),
            "seen once: may be half-written"
        );
        assert!(!watch.should_reload(at(2, 20)), "still being written");
        assert!(
            watch.should_reload(at(2, 20)),
            "same twice: the save is done"
        );
        assert!(!watch.should_reload(at(2, 20)), "already loaded");
    }

    #[test]
    fn a_file_that_vanishes_mid_save_is_not_a_reload() {
        let mut watch = Watch {
            loaded: at(1, 10),
            pending: None,
        };
        assert!(!watch.should_reload(at(2, 20)));
        assert!(!watch.should_reload(None), "deleted before the rename");
        assert!(!watch.should_reload(at(2, 20)));
        assert!(watch.should_reload(at(2, 20)));
    }
}
