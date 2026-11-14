//! The dedicated server: runs a game at a fixed tick rate with no window.
//!
//! Nothing here opens a window, draws or reads a keyboard, so a server runs on
//! a machine with no screen and no graphics card. [`serve`] calls
//! [`Game::tick`] at the configured rate until its [`StopSignal`] is set, which
//! [`StopSignal::on_ctrl_c`] ties to Ctrl-C and to the operating system asking
//! the program to end.
//!
//! ```no_run
//! use fossps::server::{ServerConfig, StopSignal, serve};
//! use fossps::{Game, InputFrame};
//!
//! struct MyGame;
//!
//! impl Game for MyGame {
//!     fn tick(&mut self, input: &InputFrame, step: f32) {
//!         // Advance the simulation by `step` seconds.
//!     }
//! }
//!
//! fn main() -> Result<(), fossps::server::ServerError> {
//!     let stop = StopSignal::on_ctrl_c()?;
//!     let report = serve(&ServerConfig::default(), &mut MyGame, &stop);
//!     println!("Ran {} ticks.", report.ticks);
//!     Ok(())
//! }
//! ```

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fossps_input::{ActionMap, Input};

use crate::game::Game;
use crate::tick::TickClock;

/// The longest the loop sleeps in one go, so a stop request is noticed quickly
/// even at a very low tick rate.
const LONGEST_SLEEP: Duration = Duration::from_millis(100);

/// The shortest time between two warnings that the server is falling behind.
const WARNING_GAP: Duration = Duration::from_secs(10);

/// How a dedicated server should run.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Simulation ticks per second.
    pub tick_rate: u32,
    /// The actions the game reads. Nothing is ever pressed on a server; the
    /// game still needs the names to exist.
    pub actions: ActionMap,
    /// How often to log a line saying how the server is doing. Zero turns the
    /// line off; a warning is still logged when the server falls behind.
    pub status_every: Duration,
}

impl Default for ServerConfig {
    /// 60 ticks per second, the standard FPS actions and a status line every
    /// minute.
    fn default() -> Self {
        Self {
            tick_rate: 60,
            actions: ActionMap::default_fps(),
            status_every: Duration::from_secs(60),
        }
    }
}

/// Tells a running server to stop.
///
/// Clones share one signal, so a clone can be handed to another thread or kept
/// by the game itself.
#[derive(Debug, Clone, Default)]
pub struct StopSignal(Arc<AtomicBool>);

impl StopSignal {
    /// A signal that has not been set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A signal that is set when the person running the server presses Ctrl-C
    /// or the operating system asks the program to end (`SIGTERM` and `SIGHUP`
    /// on Linux and macOS, closing the console on Windows).
    ///
    /// # Errors
    ///
    /// Fails if this was already called in this program, or the operating
    /// system refuses to tell the program about those requests.
    pub fn on_ctrl_c() -> Result<Self, ServerError> {
        let signal = Self::new();
        let handler = signal.clone();
        ctrlc::set_handler(move || handler.stop()).map_err(ServerError::Signals)?;
        Ok(signal)
    }

    /// Asks the server to stop. It finishes the tick it is in first.
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether a stop has been asked for.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Why a dedicated server could not start.
#[derive(Debug)]
pub enum ServerError {
    /// The program could not arrange to be told about Ctrl-C.
    Signals(ctrlc::Error),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signals(ctrlc::Error::MultipleHandlers) => write!(
                f,
                "Something else in this program already handles Ctrl-C, so the server cannot. \
                 Call `StopSignal::on_ctrl_c` once and share the signal it returns, or make a \
                 `StopSignal::new()` and call `stop` on it from the other handler."
            ),
            Self::Signals(error) => write!(
                f,
                "Could not set up a clean stop on Ctrl-C: {error}. \
                 Try starting the server again; if this keeps happening, please report it."
            ),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Signals(error) => Some(error),
        }
    }
}

/// What a server did between starting and stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerReport {
    /// Ticks that were run.
    pub ticks: u64,
    /// Ticks that were due but never run because the server was too far
    /// behind to catch up.
    pub skipped: u64,
    /// How long the server ran.
    pub uptime: Duration,
}

/// Runs `game` with no window until `stop` is set.
///
/// [`Game::tick`] is called `config.tick_rate` times per second with nothing
/// pressed; [`Game::frame`] is never called, because nothing is drawn. Between
/// ticks the thread sleeps, so an idle server uses almost no processor time.
///
/// A server that cannot keep up runs at most a few ticks in a row to catch up,
/// gives up the rest and logs a warning that says why.
pub fn serve(config: &ServerConfig, game: &mut impl Game, stop: &StopSignal) -> ServerReport {
    run(config, game, stop, &mut RealTime(Instant::now()))
}

/// Where the loop gets the time from, so tests can run it without waiting.
trait Time {
    /// Time since the server started.
    fn now(&mut self) -> Duration;
    /// Waits for about `duration`.
    fn sleep(&mut self, duration: Duration);
}

struct RealTime(Instant);

impl Time for RealTime {
    fn now(&mut self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn run(
    config: &ServerConfig,
    game: &mut impl Game,
    stop: &StopSignal,
    time: &mut impl Time,
) -> ServerReport {
    let mut clock = TickClock::new(config.tick_rate);
    let mut input = Input::new(config.actions.clone());
    let step = clock.step();
    let started = time.now();
    let mut last = started;
    let mut stats = Stats::new(started);

    while !stop.is_stopped() {
        let now = time.now();
        // Real time is banked, not counted in sleeps, so a sleep that runs
        // long is made up for by a shorter one and the rate never drifts.
        let due = clock.advance_counting(now.saturating_sub(last));
        last = now;

        let mut tick_began = now;
        for _ in 0..due.ticks {
            game.tick(input.tick(), step.as_secs_f32());
            let tick_ended = time.now();
            stats.ticked(tick_ended.saturating_sub(tick_began));
            tick_began = tick_ended;
            if stop.is_stopped() {
                break;
            }
        }
        stats.skipped(due.skipped);
        stats.report(tick_began, step, config.status_every);

        let spent = time.now().saturating_sub(now);
        let wait = clock.until_next_tick().saturating_sub(spent);
        if !wait.is_zero() && !stop.is_stopped() {
            time.sleep(wait.min(LONGEST_SLEEP));
        }
    }

    ServerReport {
        ticks: stats.ticks,
        skipped: stats.skipped,
        uptime: time.now().saturating_sub(started),
    }
}

/// How the ticks went over one stretch of time.
#[derive(Debug, Clone, Copy)]
struct Stretch {
    began: Duration,
    ticks: u64,
    skipped: u64,
    /// Time spent inside ticks, added up.
    busy: Duration,
    longest: Duration,
}

impl Stretch {
    fn new(began: Duration) -> Self {
        Self {
            began,
            ticks: 0,
            skipped: 0,
            busy: Duration::ZERO,
            longest: Duration::ZERO,
        }
    }
}

/// Counts ticks and decides when to say something about them.
#[derive(Debug)]
struct Stats {
    ticks: u64,
    skipped: u64,
    /// Since the last status line.
    status: Stretch,
    /// Since the last falling-behind warning.
    behind: Stretch,
    warned: bool,
}

impl Stats {
    fn new(now: Duration) -> Self {
        Self {
            ticks: 0,
            skipped: 0,
            status: Stretch::new(now),
            behind: Stretch::new(now),
            warned: false,
        }
    }

    fn ticked(&mut self, took: Duration) {
        self.ticks += 1;
        for stretch in [&mut self.status, &mut self.behind] {
            stretch.ticks += 1;
            stretch.busy += took;
            stretch.longest = stretch.longest.max(took);
        }
    }

    fn skipped(&mut self, ticks: u64) {
        self.skipped = self.skipped.saturating_add(ticks);
        self.status.skipped = self.status.skipped.saturating_add(ticks);
        self.behind.skipped = self.behind.skipped.saturating_add(ticks);
    }

    /// Logs the status line and the falling-behind warning when they are due.
    fn report(&mut self, now: Duration, step: Duration, status_every: Duration) {
        let since_warning = now.saturating_sub(self.behind.began);
        // The first warning is not held back: the operator should hear at once.
        if self.behind.skipped > 0 && (!self.warned || since_warning >= WARNING_GAP) {
            log::warn!("{}", behind_text(&self.behind, since_warning, step));
            self.warned = true;
            self.behind = Stretch::new(now);
        } else if self.behind.skipped == 0 && since_warning >= WARNING_GAP {
            self.behind = Stretch::new(now);
        }

        let since_status = now.saturating_sub(self.status.began);
        if !status_every.is_zero() && since_status >= status_every {
            log::info!("{}", status_text(&self.status, since_status, step));
            self.status = Stretch::new(now);
        }
    }
}

/// `duration` in milliseconds, for a message.
fn ms(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
}

fn status_text(stretch: &Stretch, lasted: Duration, step: Duration) -> String {
    let rate = stretch.ticks as f64 / lasted.as_secs_f64().max(f64::EPSILON);
    let average = stretch
        .busy
        .checked_div(u32::try_from(stretch.ticks).unwrap_or(u32::MAX))
        .unwrap_or_default();
    let mut text = format!(
        "Running at {rate:.1} ticks a second. A tick takes {} on average and {} at most; \
         {} is allowed.",
        ms(average),
        ms(stretch.longest),
        ms(step)
    );
    if stretch.skipped > 0 {
        text.push_str(&format!(" Skipped {} ticks.", stretch.skipped));
    }
    text
}

fn behind_text(stretch: &Stretch, lasted: Duration, step: Duration) -> String {
    let lost = step.as_secs_f64() * stretch.skipped as f64;
    let cause = if stretch.longest > step {
        format!(
            "Its slowest tick took {} and only {} is allowed, so the game does more work \
             each tick than this machine can do in time. Make the game do less each tick, or \
             use a faster machine.",
            ms(stretch.longest),
            ms(step)
        )
    } else {
        format!(
            "Its ticks are fast enough ({} at most, {} allowed), so the machine itself \
             paused or was busy with something else. Check what else is running on it.",
            ms(stretch.longest),
            ms(step)
        )
    };
    let span = match lasted.as_secs_f64().round() {
        seconds if seconds < 2.0 => "second".to_owned(),
        seconds => format!("{seconds:.0} seconds"),
    };
    format!(
        "The server is falling behind: it skipped {} ticks ({lost:.1} s of game time) in the \
         last {span}. {cause}",
        stretch.skipped
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use fossps_input::InputFrame;

    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// A clock the test moves by hand. Sleeping moves it forward by exactly
    /// the time asked for, plus `oversleep`.
    #[derive(Clone, Default)]
    struct FakeTime {
        now: Rc<Cell<Duration>>,
        oversleep: Duration,
        sleeps: Rc<Cell<u32>>,
    }

    impl Time for FakeTime {
        fn now(&mut self) -> Duration {
            self.now.get()
        }

        fn sleep(&mut self, duration: Duration) {
            self.now.set(self.now.get() + duration + self.oversleep);
            self.sleeps.set(self.sleeps.get() + 1);
        }
    }

    /// A game whose ticks each take `cost` and that stops the server once the
    /// clock reaches `until`.
    struct Busy {
        time: FakeTime,
        cost: Duration,
        until: Duration,
        stop: StopSignal,
        ticks: u64,
        steps: Vec<f32>,
        pressed: bool,
    }

    impl Busy {
        fn new(time: &FakeTime, stop: &StopSignal, cost: Duration, until: Duration) -> Self {
            Self {
                time: time.clone(),
                cost,
                until,
                stop: stop.clone(),
                ticks: 0,
                steps: Vec::new(),
                pressed: false,
            }
        }
    }

    impl Game for Busy {
        fn tick(&mut self, input: &InputFrame, step: f32) {
            self.ticks += 1;
            self.steps.push(step);
            self.pressed |= ActionMap::default_fps()
                .actions()
                .any(|action| input.pressed(action));
            self.time.now.set(self.time.now.get() + self.cost);
            if self.time.now.get() >= self.until {
                self.stop.stop();
            }
        }
    }

    fn serve_for(
        rate: u32,
        cost: Duration,
        oversleep: Duration,
        until: Duration,
    ) -> (ServerReport, Busy, FakeTime) {
        let mut time = FakeTime {
            oversleep,
            ..FakeTime::default()
        };
        let stop = StopSignal::new();
        let mut game = Busy::new(&time, &stop, cost, until);
        let config = ServerConfig {
            tick_rate: rate,
            ..ServerConfig::default()
        };
        let report = run(&config, &mut game, &stop, &mut time);
        (report, game, time)
    }

    #[test]
    fn runs_the_tick_rate_and_sleeps_in_between() {
        let (report, game, time) = serve_for(60, MS, Duration::ZERO, Duration::from_secs(10));

        assert_eq!(report.ticks, 600);
        assert_eq!(report.skipped, 0);
        assert_eq!(game.ticks, 600);
        assert!(
            game.steps
                .iter()
                .all(|step| (step - 1.0 / 60.0).abs() < 1e-6)
        );
        assert!(!game.pressed, "nothing is ever pressed on a server");
        // One sleep per tick: the loop never spins while it waits.
        assert!(
            (599..=601).contains(&time.sleeps.get()),
            "{}",
            time.sleeps.get()
        );
        assert!(report.uptime >= Duration::from_secs(10));
    }

    #[test]
    fn sleeping_too_long_does_not_slow_the_game_down() {
        // Every sleep runs 3 ms over, as on a busy machine.
        let (report, ..) = serve_for(60, MS, 3 * MS, Duration::from_secs(10));

        assert!((599..=601).contains(&report.ticks), "{}", report.ticks);
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn a_game_too_slow_for_the_rate_skips_ticks_and_does_not_spiral() {
        // Each tick takes 25 ms and only 16.7 ms is allowed.
        let (report, game, time) = serve_for(60, 25 * MS, Duration::ZERO, Duration::from_secs(10));

        // It runs back to back: 10 s / 25 ms.
        assert!((395..=405).contains(&report.ticks), "{}", report.ticks);
        assert_eq!(report.ticks, game.ticks);
        assert!(
            report.ticks + report.skipped <= 601,
            "ticks {} skipped {}",
            report.ticks,
            report.skipped
        );
        assert!(report.skipped > 150, "{}", report.skipped);
        // Only the wait for the very first tick.
        assert!(time.sleeps.get() <= 1, "{}", time.sleeps.get());
    }

    #[test]
    fn stops_within_a_tick_of_being_asked() {
        let mut time = FakeTime::default();
        let stop = StopSignal::new();
        stop.stop();
        let mut game = Busy::new(&time, &stop, MS, Duration::MAX);

        let report = run(&ServerConfig::default(), &mut game, &stop, &mut time);

        assert_eq!(report.ticks, 0);
        assert_eq!(time.sleeps.get(), 0);
    }

    #[test]
    fn a_very_low_rate_still_notices_a_stop_quickly() {
        let (report, _, time) = serve_for(1, MS, Duration::ZERO, Duration::from_secs(3));

        assert_eq!(report.ticks, 3);
        // Sleeps are cut into pieces of at most a tenth of a second.
        assert!(time.sleeps.get() >= 29, "{}", time.sleeps.get());
    }

    #[test]
    fn the_same_run_gives_the_same_ticks() {
        let run = || serve_for(64, 2 * MS, MS, Duration::from_secs(5)).0;
        assert_eq!(run(), run());
    }

    #[test]
    fn the_status_line_gives_the_rate_and_the_tick_times() {
        let stretch = Stretch {
            began: Duration::ZERO,
            ticks: 3600,
            skipped: 0,
            busy: Duration::from_millis(1800),
            longest: 4 * MS,
        };
        assert_eq!(
            status_text(
                &stretch,
                Duration::from_secs(60),
                Duration::from_secs(1) / 60
            ),
            "Running at 60.0 ticks a second. A tick takes 0.50 ms on average and 4.00 ms at \
             most; 16.67 ms is allowed."
        );
    }

    #[test]
    fn the_warning_says_whether_the_game_or_the_machine_is_slow() {
        let step = Duration::from_secs(1) / 60;
        let mut stretch = Stretch {
            began: Duration::ZERO,
            ticks: 400,
            skipped: 120,
            busy: Duration::from_secs(10),
            longest: 25 * MS,
        };
        let slow_game = behind_text(&stretch, Duration::from_secs(10), step);
        assert!(
            slow_game.starts_with(
                "The server is falling behind: it skipped 120 ticks (2.0 s of game time) in \
                 the last 10 seconds. Its slowest tick took 25.00 ms and only 16.67 ms is \
                 allowed"
            ),
            "{slow_game}"
        );
        assert!(
            slow_game.contains("Make the game do less each tick"),
            "{slow_game}"
        );

        assert!(
            behind_text(&stretch, Duration::from_millis(300), step).contains("in the last second."),
        );

        stretch.longest = MS;
        let slow_machine = behind_text(&stretch, Duration::from_secs(10), step);
        assert!(
            slow_machine.contains("Check what else is running on it"),
            "{slow_machine}"
        );
    }

    #[test]
    fn warnings_are_spaced_out() {
        let step = Duration::from_secs(1) / 60;
        let mut stats = Stats::new(Duration::ZERO);

        stats.skipped(3);
        stats.report(Duration::from_secs(1), step, Duration::ZERO);
        assert!(stats.warned);
        assert_eq!(stats.behind.skipped, 0);

        // More skips soon after are held until the gap has passed.
        stats.skipped(4);
        stats.report(Duration::from_secs(2), step, Duration::ZERO);
        assert_eq!(stats.behind.skipped, 4);
        stats.report(Duration::from_secs(12), step, Duration::ZERO);
        assert_eq!(stats.behind.skipped, 0);
        assert_eq!(stats.skipped, 7);
    }
}
