//! Runs a game's `main.luau` and the scripts it requires, and swaps in a new
//! version when any of them is saved.

// A script error carries the whole explanation by value. It is only built when
// a script fails to load, never on the tick path, so its size does not matter.
#![allow(clippy::result_large_err)]

use std::cell::{Cell, Ref, RefCell};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use fossps_input::{ActionMap, InputFrame, Rumble};
use fossps_mode::{Event, Match, PlayerId};
use mlua::{AnyUserData, Function, IntoLuaMulti, Lua, VmState};

use crate::api::{self, Handlers, Shared};
use crate::error::{OpenError, ScriptError, TookTooLong, is_time_limit};
use crate::keep::{Kept, Store};
use crate::mode_api::{Handles, ModeState};
use crate::modules::{Files, Modules, Stamp, Touched, stamp};

/// The file every game starts from, inside the game's folder.
pub const ENTRY_SCRIPT: &str = "main.luau";

/// How long one call into the script may run before it is stopped. A healthy
/// tick takes well under a millisecond; this only catches loops that never end.
const TIME_LIMIT: Duration = Duration::from_secs(1);

/// How much memory one script may use.
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// How often the script files are checked for a new save.
const POLL_EVERY: Duration = Duration::from_millis(100);

/// How many different runtime errors are printed before the rest are dropped,
/// so a script failing in new ways every tick cannot flood the log.
const MAX_REPORTED: usize = 32;

/// How many match events are handled in one go. A kill is one event and a
/// round starting with 64 players is 65, so reaching this means handlers keep
/// setting each other off.
const MAX_EVENTS_AT_ONCE: usize = 10_000;

/// How many spawns wait for the engine to place them before the oldest are
/// dropped. An engine that takes them every tick never comes near it.
const MAX_WAITING_SPAWNS: usize = 1024;

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
    vm: Lua,
    shared: Rc<Shared>,
}

/// A script file the game looked for, found or not.
#[derive(Debug)]
struct Watched {
    path: PathBuf,
    watch: Watch,
}

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

/// Runs one game's script.
///
/// The script lives in its own sandboxed Luau VM: it can reach the `game`
/// table and Luau's standard library, and nothing else on the computer.
///
/// ```no_run
/// use fossps_input::{ActionMap, InputFrame, Rumble};
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
    /// Where the game's scripts are read from.
    files: Rc<Files>,
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
    /// Every file the running version looked for, and the ones the latest
    /// version that failed to load did. A save of any of them is a reload.
    watched: Vec<Watched>,
    next_poll: Instant,
    /// The match. Kept here, outside the script's VM, so that saving the
    /// script does not reset the score.
    mode: Rc<ModeState>,
    /// What the script asked to keep with `game.keep`, as it was at the
    /// last reload, for the next version to pick up.
    kept: Rc<RefCell<Store>>,
    /// Players who have come into the world and not yet been given a place.
    spawns: Vec<PlayerId>,
}

impl std::fmt::Debug for ScriptHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptHost")
            .field("files", &self.files)
            .field("running", &self.running.is_some())
            .field("last_error", &self.last_error)
            .finish_non_exhaustive()
    }
}

impl ScriptHost {
    /// Opens the game in `folder` and runs its `main.luau`, which can
    /// `require` the other scripts in the folder.
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
        let folder = folder.as_ref();
        let path = folder.join(ENTRY_SCRIPT);
        if let Err(source) = std::fs::File::open(&path) {
            return Err(if source.kind() == io::ErrorKind::NotFound {
                OpenError::Missing { path }
            } else {
                OpenError::Unreadable { path, source }
            });
        }
        Ok(Self::with_files(Files::Folder(folder.to_owned()), actions))
    }

    /// Runs `source` as a game's `main.luau` without a file behind it. The
    /// game has no other scripts, so a `require` in it finds nothing.
    ///
    /// Nothing is watched, so [`poll_reload`](Self::poll_reload) and
    /// [`reload`](Self::reload) do nothing. A mistake inside the script is
    /// logged and kept in [`last_error`](Self::last_error).
    #[must_use]
    pub fn from_source(source: impl Into<String>, actions: ActionMap) -> Self {
        Self::from_sources([(ENTRY_SCRIPT.to_owned(), source.into())], actions)
    }

    /// Runs a game whose scripts are given as text, as a packaged game
    /// does: they came out of the game's pack.
    ///
    /// `scripts` are pairs of a name and the script's text. A name is the
    /// file's place in the game with `/` between folders, such as
    /// `main.luau` or `weapons/rifle.luau`; `main.luau` is the one that is
    /// run, and the others are what it can `require`.
    ///
    /// Nothing is watched, so [`poll_reload`](Self::poll_reload) and
    /// [`reload`](Self::reload) do nothing. A mistake inside a script, or
    /// leaving `main.luau` out, is logged and kept in
    /// [`last_error`](Self::last_error).
    ///
    /// ```
    /// use fossps_input::ActionMap;
    /// use fossps_script::ScriptHost;
    ///
    /// let host = ScriptHost::from_sources(
    ///     [
    ///         ("main.luau".to_owned(), "print(require('./weapons').rifle)".to_owned()),
    ///         ("weapons.luau".to_owned(), "return { rifle = 30 }".to_owned()),
    ///     ],
    ///     ActionMap::default_fps(),
    /// );
    /// assert!(host.last_error().is_none());
    /// ```
    #[must_use]
    pub fn from_sources(
        scripts: impl IntoIterator<Item = (String, String)>,
        actions: ActionMap,
    ) -> Self {
        Self::with_files(Files::Memory(scripts.into_iter().collect()), actions)
    }

    /// A host running the `main.luau` in `files`.
    fn with_files(files: Files, actions: ActionMap) -> Self {
        let mut host = Self {
            files: Rc::new(files),
            file: ENTRY_SCRIPT.to_owned(),
            actions: Rc::new(actions),
            running: None,
            started: false,
            paused: false,
            last_error: None,
            reported: HashSet::new(),
            watched: Vec::new(),
            next_poll: Instant::now() + POLL_EVERY,
            mode: Rc::new(ModeState::new()),
            kept: Rc::default(),
            spawns: Vec::new(),
        };
        // A script error is already logged and kept in `last_error`.
        let _ = host.install();
        host
    }

    /// The most recent script error, until a reload succeeds.
    #[must_use]
    pub fn last_error(&self) -> Option<&ScriptError> {
        self.last_error.as_ref()
    }

    /// Runs one simulation tick of `step` seconds with this tick's `input`.
    ///
    /// The first tick calls the script's `game.onStart` functions before its
    /// `game.onTick` ones. Then the match moves on by one tick, and the
    /// script is told what happened in it. A function that fails is reported
    /// once and the others still run.
    pub fn tick(&mut self, input: &InputFrame, step: f32) {
        if let Some(version) = &self.running {
            version.shared.input.borrow_mut().clone_from(input);
        }
        self.start();
        self.call_all(|handlers| &handlers.tick, step);
        self.mode.game.borrow_mut().tick(step);
        self.tell_script();
    }

    /// The rumble the script asked for with `game.input.rumble` since this
    /// was last called, for whoever owns the gamepad to play. When the script
    /// asked more than once, this is the latest.
    pub fn take_rumble(&mut self) -> Option<Rumble> {
        self.running.as_ref()?.shared.rumble.take()
    }

    /// The match being played: the rules the script set with `game.mode`,
    /// who is in it, the score and where the round is.
    ///
    /// ```no_run
    /// # use fossps_input::ActionMap;
    /// # use fossps_script::ScriptHost;
    /// # let host = ScriptHost::open("my-game", ActionMap::default_fps())?;
    /// for team in host.mode().teams() {
    ///     println!("{}: {}", team.name, team.score);
    /// }
    /// # Ok::<(), fossps_script::OpenError>(())
    /// ```
    #[must_use]
    pub fn mode(&self) -> Ref<'_, Match> {
        self.mode.game.borrow()
    }

    /// Adds a player to the match and tells the script. With teams, the
    /// player is put on the smallest one.
    pub fn player_joined(&mut self, name: &str) -> PlayerId {
        let id = self.mode.game.borrow_mut().join(name);
        self.tell_script();
        id
    }

    /// Takes a player out of the match and tells the script. Does nothing if
    /// they were not in it.
    pub fn player_left(&mut self, player: PlayerId) {
        self.mode.game.borrow_mut().leave(player);
        self.tell_script();
    }

    /// Records that `victim` died, by `killer`'s hand if there was one, and
    /// tells the script. Does nothing if the victim is not alive.
    pub fn player_killed(&mut self, killer: Option<PlayerId>, victim: PlayerId) {
        self.mode.game.borrow_mut().kill(killer, victim);
        self.tell_script();
    }

    /// Ends the round now, as if time had run out: the highest score wins.
    /// Does nothing unless a round is being played.
    pub fn end_round(&mut self) {
        {
            let mut game = self.mode.game.borrow_mut();
            let leader = game.leader();
            game.finish(leader);
        }
        self.tell_script();
    }

    /// The players who have come into the world since this was last called,
    /// oldest first: each needs putting at a spawn point (see
    /// [`fossps_mode::choose_spawn`]). Call it every tick.
    pub fn take_spawns(&mut self) -> std::vec::Drain<'_, PlayerId> {
        self.spawns.drain(..)
    }

    /// Reloads the script if `main.luau` or a script it required was saved
    /// since it was last loaded.
    ///
    /// Call this every tick or frame; it looks at the files at most ten times
    /// a second. See [`reload`](Self::reload) for what a reload does.
    pub fn poll_reload(&mut self) {
        let now = Instant::now();
        if now < self.next_poll {
            return;
        }
        self.next_poll = now + POLL_EVERY;
        // A script required for the first time while the game runs.
        if let Some(version) = &self.running {
            let touched = version.shared.modules.take_touched();
            if !touched.is_empty() {
                self.watch(touched, false);
            }
        }
        let mut saved = false;
        for file in &mut self.watched {
            // Every file is asked, so each one's save settles on its own.
            saved |= file.watch.should_reload(stamp(&file.path));
        }
        if saved {
            // A script error is already logged and kept in `last_error`.
            let _ = self.reload();
        }
    }

    /// Loads the script from disk again, now: `main.luau` and, as it
    /// requires them, the other scripts.
    ///
    /// The new version starts fresh: every script's variables are reset and
    /// its handlers replace the old ones, then its `game.onReload` functions
    /// are called. The tables the script asked for with `game.keep` are
    /// handed to the new version with what was in them.
    /// `game.onStart` functions are not called again. A script that was given
    /// as text ([`from_source`](Self::from_source)) has no file, and nothing
    /// happens.
    ///
    /// # Errors
    ///
    /// If the new version has a mistake, the error is returned (and logged) and
    /// the previous version keeps running.
    pub fn reload(&mut self) -> Result<(), ScriptError> {
        if !self.files.on_disk() {
            return Ok(());
        }
        let had_version = self.running.is_some();
        if let Some(version) = &self.running {
            // The new version asks for these while it loads. If it fails to
            // load, the old one carries on with its own tables.
            let (kept, problems) = version.shared.kept.snapshot();
            for problem in problems {
                log::error!("{problem}");
            }
            self.kept.replace(kept);
        }
        self.install()?;
        if had_version {
            log::info!("Reloaded {}.", self.file);
        }
        if self.started {
            self.call_all(|handlers| &handlers.reload, ());
        }
        self.tell_script();
        Ok(())
    }

    /// Builds a version from the game's scripts and makes it the running one.
    fn install(&mut self) -> Result<(), ScriptError> {
        let (touched, built) = self.build();
        // A version that failed is fixed by saving one of the files it read,
        // or one the running version did.
        self.watch(touched, built.is_ok());
        match built {
            Ok(version) => {
                // A script with no `game.mode` plays the default rules.
                let rules = version.shared.rules.borrow_mut().take();
                let rules = rules.map(|(rules, _)| rules).unwrap_or_default();
                // `game.mode` only keeps rules that can be played.
                if let Err(problem) = self.mode.game.borrow_mut().set_rules(rules) {
                    log::error!("The rules from game.mode were not used: {problem}");
                }
                self.running = Some(version);
                self.paused = false;
                self.last_error = None;
                self.reported.clear();
                Ok(())
            }
            Err(error) => Err(self.failed_to_load(error)),
        }
    }

    /// Notes what the files a version looked for were like when it did.
    /// `only` forgets the files that are not among them.
    fn watch(&mut self, touched: Touched, only: bool) {
        if only {
            self.watched.clear();
        }
        for (path, seen) in touched {
            let watch = Watch {
                loaded: seen,
                pending: None,
            };
            match self.watched.iter_mut().find(|file| file.path == path) {
                Some(file) => file.watch = watch,
                None => self.watched.push(Watched { path, watch }),
            }
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

    /// Starts a fresh VM and runs the game's `main.luau` in it. Also gives
    /// the files that were looked for, whether or not it worked.
    fn build(&self) -> (Touched, Result<Version, ScriptError>) {
        let shared = Rc::new(Shared {
            actions: Rc::clone(&self.actions),
            handlers: RefCell::new(Handlers::default()),
            input: RefCell::new(InputFrame::default()),
            deadline: Cell::new(None),
            stopped: RefCell::new(None),
            checks: Cell::new(0),
            mode: Rc::clone(&self.mode),
            handles: Handles::default(),
            loading: Cell::new(true),
            rules: RefCell::new(None),
            rumble: Cell::new(None),
            modules: Modules::new(Rc::clone(&self.files)),
            kept: Kept::default(),
            store: Rc::clone(&self.kept),
        });
        let built = self.run(&shared);
        (shared.modules.take_touched(), built)
    }

    /// Runs the game's `main.luau` in a fresh VM that reaches `shared`.
    fn run(&self, shared: &Rc<Shared>) -> Result<Version, ScriptError> {
        let source = shared.modules.read(&self.file).map_err(|problem| {
            let message = shared.modules.explain(&problem, &self.file, None);
            ScriptError::file_problem(&self.file, message)
        })?;
        let lua = Lua::new();
        let run = || -> mlua::Result<()> {
            lua.set_memory_limit(MEMORY_LIMIT)?;
            api::install(&lua, shared)?;
            lua.sandbox(true)?;
            let limit = Rc::clone(shared);
            lua.set_interrupt(move |lua| {
                let Some(deadline) = limit.deadline.get() else {
                    return Ok(VmState::Continue);
                };
                // Once time is up every check fails, so a script that catches
                // the error with pcall still cannot carry on.
                let mut stopped = limit.stopped.borrow().clone();
                if stopped.is_none() {
                    // This runs at every function call and loop turn, so the
                    // clock is only read now and then.
                    let checks = limit.checks.get().wrapping_add(1);
                    limit.checks.set(checks);
                    if !checks.is_multiple_of(CLOCK_EVERY) || Instant::now() < deadline {
                        return Ok(VmState::Continue);
                    }
                    let place = lua.inspect_stack(0, |running| {
                        let file = running.source().source;
                        let file = file.and_then(|file| Some(file.strip_prefix('@')?.to_owned()));
                        (file, running.current_line())
                    });
                    let (file, line) = place.unwrap_or_default();
                    let line = line.and_then(|line| u32::try_from(line).ok());
                    stopped = Some(TookTooLong { file, line });
                    limit.stopped.borrow_mut().clone_from(&stopped);
                }
                match stopped {
                    Some(stopped) => Err(mlua::Error::external(stopped)),
                    None => Ok(VmState::Continue),
                }
            });
            let chunk = lua.load(&source).set_name(format!("@{}", self.file));
            let result = within_time_limit(shared, || chunk.exec());
            shared.loading.set(false);
            result
        };
        match run() {
            Ok(()) => Ok(Version {
                vm: lua,
                shared: Rc::clone(shared),
            }),
            Err(error) => Err(ScriptError::from_lua(
                &error,
                &self.file,
                &shared.modules.sources.borrow(),
            )),
        }
    }

    /// Calls the `game.onStart` functions if they have not been called yet.
    fn start(&mut self) {
        if !self.started && self.running.is_some() && !self.paused {
            self.started = true;
            self.call_all(|handlers| &handlers.start, ());
        }
    }

    /// Tells the script everything that has happened in the match and has
    /// not been told yet, in order, and notes who needs a spawn point.
    ///
    /// What a handler does to the match (a score that ends the round, say)
    /// is only queued while the handler runs, and told here afterwards, so
    /// handlers never run inside one another.
    fn tell_script(&mut self) {
        let mut told = 0;
        loop {
            let event = self.mode.game.borrow_mut().next_event();
            let Some(event) = event else { break };
            told += 1;
            if told == MAX_EVENTS_AT_ONCE {
                log::error!(
                    "The script's game.on... functions keep setting each other off \
                     ({MAX_EVENTS_AT_ONCE} events in one tick), so the rest of this tick's \
                     events were skipped. Look for a function that causes the event it \
                     listens to."
                );
            }
            let quiet = told >= MAX_EVENTS_AT_ONCE;
            if !quiet {
                self.start();
            }
            match event {
                Event::Joined(id) if !quiet => {
                    if let Some(player) = self.player(id) {
                        self.call_all(|handlers| &handlers.join, player);
                    }
                }
                Event::Left(player) => {
                    let id = player.id;
                    self.mode.gone.borrow_mut().push(player);
                    if !quiet && let Some(player) = self.player(id) {
                        self.call_all(|handlers| &handlers.leave, player);
                    }
                }
                Event::Killed { killer, victim } if !quiet => {
                    let killer = killer.and_then(|killer| self.player(killer));
                    if let Some(victim) = self.player(victim) {
                        self.call_all(|handlers| &handlers.kill, (killer, victim));
                    }
                }
                Event::Spawned(id) => {
                    if !quiet && let Some(player) = self.player(id) {
                        self.call_all(|handlers| &handlers.spawn, player);
                    }
                    if self.spawns.len() >= MAX_WAITING_SPAWNS {
                        self.spawns.remove(0);
                    }
                    self.spawns.push(id);
                }
                Event::RoundStarted if !quiet => {
                    self.call_all(|handlers| &handlers.round_start, ());
                }
                Event::RoundEnded { winner } if !quiet => {
                    let winner = self.running.as_ref().and_then(|version| {
                        version.shared.handles.winner(&version.vm, winner).ok()
                    });
                    if let Some(winner) = winner {
                        self.call_all(|handlers| &handlers.round_end, winner);
                    }
                }
                _ => {}
            }
        }
        // Everyone has been told, so those who left can be forgotten.
        for player in self.mode.gone.borrow_mut().drain(..) {
            if let Some(version) = &self.running {
                version.shared.handles.forget(player.id);
            }
        }
    }

    /// The running script's value for a player.
    fn player(&self, id: PlayerId) -> Option<AnyUserData> {
        let version = self.running.as_ref()?;
        version.shared.handles.player(&version.vm, id).ok()
    }

    /// Calls every function in one handler list, reporting the ones that fail.
    fn call_all(&mut self, list: fn(&Handlers) -> &Vec<Function>, args: impl IntoLuaMulti + Clone) {
        if self.paused {
            return;
        }
        let Some(version) = &self.running else { return };
        // Handlers added while these run are first called on the next pass,
        // so a handler that keeps adding handlers cannot hold up the tick.
        let count = list(&version.shared.handlers.borrow()).len();
        for index in 0..count {
            // The list is borrowed only for this lookup, so a handler may
            // register more handlers while it runs.
            let handler = list(&version.shared.handlers.borrow()).get(index).cloned();
            let Some(handler) = handler else { break };
            let Err(error) =
                within_time_limit(&version.shared, || handler.call::<()>(args.clone()))
            else {
                continue;
            };
            let too_long = is_time_limit(&error);
            let sources = version.shared.modules.sources.borrow();
            let error = ScriptError::from_lua(&error, &self.file, &sources)
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
    shared.stopped.replace(None);
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
    use std::time::SystemTime;

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
