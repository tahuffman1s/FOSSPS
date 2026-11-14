//! Fixed-step timing: turns uneven frame times into a whole number of ticks.

use std::time::Duration;

/// The longest stretch of real time one frame may account for.
///
/// After a stall (a breakpoint, a window drag on Windows, a suspended laptop)
/// the simulation resumes instead of fast-forwarding through the gap.
const LONGEST_FRAME: Duration = Duration::from_millis(250);

/// The most ticks one frame may run to catch up.
///
/// If ticks ever cost more than their step, catching up without a limit would
/// make every frame slower than the last.
const MOST_TICKS_PER_FRAME: u32 = 5;

/// Decides how many simulation ticks to run each frame.
///
/// The simulation always advances in steps of the same length, which keeps it
/// deterministic. Frames arrive at uneven intervals, so the clock banks the
/// elapsed time and pays it out in whole ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickClock {
    step: Duration,
    banked: Duration,
}

/// What [`TickClock::advance_counting`] decided for one pass of the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Due {
    /// Ticks to run now.
    pub(crate) ticks: u32,
    /// Ticks that were due but will never be run.
    pub(crate) skipped: u64,
}

impl TickClock {
    /// A clock ticking `rate` times per second. A rate of 0 is treated as 1.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        Self {
            step: Duration::from_secs(1) / rate.max(1),
            banked: Duration::ZERO,
        }
    }

    /// The length of one tick.
    #[must_use]
    pub fn step(&self) -> Duration {
        self.step
    }

    /// Banks `elapsed` real time and returns how many ticks to run now.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.advance_counting(elapsed).ticks
    }

    /// Like [`advance`](Self::advance), and also says how many ticks were
    /// given up because the caller was too far behind to catch up.
    pub(crate) fn advance_counting(&mut self, elapsed: Duration) -> Due {
        let step = self.step.as_nanos().max(1);
        // What would be due if nothing were ever given up.
        let owed = self.banked.saturating_add(elapsed).as_nanos() / step;
        self.banked += elapsed.min(LONGEST_FRAME);
        let ticks = match u32::try_from(self.banked.as_nanos() / step) {
            Ok(ticks) if ticks <= MOST_TICKS_PER_FRAME => {
                self.banked -= self.step * ticks;
                ticks
            }
            // Too far behind: run the limit and forget the rest.
            _ => {
                self.banked = Duration::ZERO;
                MOST_TICKS_PER_FRAME
            }
        };
        Due {
            ticks,
            skipped: u64::try_from(owed)
                .unwrap_or(u64::MAX)
                .saturating_sub(u64::from(ticks)),
        }
    }

    /// How far the present moment is between the last tick and the next, from 0 to 1.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        (self.banked.as_secs_f32() / self.step.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Real time left until the next tick is due.
    #[must_use]
    pub fn until_next_tick(&self) -> Duration {
        self.step.saturating_sub(self.banked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn pays_out_whole_ticks_and_banks_the_rest() {
        let mut clock = TickClock::new(100);
        assert_eq!(clock.step(), 10 * MS);
        assert_eq!(clock.advance(4 * MS), 0);
        assert_eq!(clock.advance(4 * MS), 0);
        assert_eq!(clock.advance(4 * MS), 1);
        assert_eq!(clock.until_next_tick(), 8 * MS);
        assert!((clock.alpha() - 0.2).abs() < 1e-4);
        assert_eq!(clock.advance(28 * MS), 3);
        assert_eq!(clock.until_next_tick(), 10 * MS);
    }

    #[test]
    fn uneven_frames_add_up_to_the_right_number_of_ticks() {
        let mut clock = TickClock::new(60);
        let mut ticks = 0;
        for frame in 0..6_000_u32 {
            let elapsed = if frame % 2 == 0 { 3 * MS } else { 17 * MS };
            ticks += clock.advance(elapsed);
        }
        // 60 seconds of real time at 60 ticks per second.
        assert_eq!(ticks, 3_600);
    }

    #[test]
    fn a_long_stall_does_not_fast_forward() {
        let mut clock = TickClock::new(60);
        assert_eq!(clock.advance(Duration::from_secs(30)), MOST_TICKS_PER_FRAME);
        assert_eq!(clock.advance(Duration::ZERO), 0);
        assert_eq!(clock.alpha(), 0.0);
    }

    #[test]
    fn counts_the_ticks_it_gives_up() {
        let mut clock = TickClock::new(100);
        assert_eq!(
            clock.advance_counting(30 * MS),
            Due {
                ticks: 3,
                skipped: 0
            }
        );
        // 80 ms is 8 ticks: 5 are run and 3 are given up.
        assert_eq!(
            clock.advance_counting(80 * MS),
            Due {
                ticks: 5,
                skipped: 3
            }
        );
        // A two-second stall is 200 ticks, and only 5 are run.
        assert_eq!(
            clock.advance_counting(Duration::from_secs(2)),
            Due {
                ticks: 5,
                skipped: 195
            }
        );
        assert_eq!(
            clock.advance_counting(10 * MS),
            Due {
                ticks: 1,
                skipped: 0
            }
        );
    }

    #[test]
    fn zero_rate_is_safe() {
        let mut clock = TickClock::new(0);
        assert_eq!(clock.step(), Duration::from_secs(1));
        assert_eq!(clock.advance(Duration::from_millis(200)), 0);
    }

    #[test]
    fn same_frame_times_give_the_same_ticks() {
        let run = || {
            let mut clock = TickClock::new(60);
            (0..500_u32)
                .map(|frame| clock.advance(Duration::from_micros(u64::from(frame % 37) * 700)))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
