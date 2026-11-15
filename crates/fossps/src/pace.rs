//! The frame rate limit: decides when the next frame may start.

use std::time::{Duration, Instant};

/// The lowest frame rate limit that is honoured; a lower one is raised to it.
///
/// The simulation catches up a few ticks per frame at most, so frames much
/// further apart than this would leave ticks behind and slow the game down.
pub(crate) const LOWEST_LIMIT: u32 = 30;

/// How close to the moment a frame is due the loop stops sleeping and watches
/// the clock instead.
///
/// The operating system wakes a sleeper a little late, by more than a frame
/// is allowed to be out. A fifth of a millisecond of watching costs a few
/// percent of one processor core at the highest limits and nothing worth
/// measuring at the usual ones.
pub(crate) const WATCH: Duration = Duration::from_micros(200);

/// Spaces frames out so no more than a set number start each second.
///
/// Every frame is due a fixed time after the one before was due, not after it
/// started, so a frame that starts a little late does not make the rest late
/// too and the rate over a second comes out as asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FramePacer {
    /// The time from one frame to the next, or `None` for no limit.
    interval: Option<Duration>,
    /// When the next frame may start.
    due: Instant,
}

impl FramePacer {
    /// A pacer allowing `max_fps` frames per second, the first of them at
    /// `now`. 0 is no limit, and a limit under [`LOWEST_LIMIT`] is raised to it.
    pub(crate) fn new(max_fps: u32, now: Instant) -> Self {
        Self {
            interval: (max_fps > 0).then(|| Duration::from_secs(1) / max_fps.max(LOWEST_LIMIT)),
            due: now,
        }
    }

    /// How long from `now` until the next frame may start: zero if it may
    /// start at once.
    pub(crate) fn wait(&self, now: Instant) -> Duration {
        match self.interval {
            Some(_) => self.due.saturating_duration_since(now),
            None => Duration::ZERO,
        }
    }

    /// Notes that a frame starts at `now` and works out when the next is due.
    pub(crate) fn start(&mut self, now: Instant) {
        let Some(interval) = self.interval else {
            return;
        };
        let next = self.due + interval;
        // A frame that took longer than the limit allows has nothing to wait
        // for, and the time it lost is not made up with a burst of fast ones.
        self.due = next.max(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn no_limit_never_waits() {
        let now = Instant::now();
        let mut pacer = FramePacer::new(0, now);
        for _ in 0..3 {
            assert_eq!(pacer.wait(now), Duration::ZERO);
            pacer.start(now);
        }
    }

    #[test]
    fn the_first_frame_starts_at_once_and_the_next_waits_its_turn() {
        let now = Instant::now();
        let mut pacer = FramePacer::new(100, now);
        assert_eq!(pacer.wait(now), Duration::ZERO);
        pacer.start(now);
        assert_eq!(pacer.wait(now), 10 * MS);
        assert_eq!(pacer.wait(now + 4 * MS), 6 * MS);
        assert_eq!(pacer.wait(now + 10 * MS), Duration::ZERO);
        assert_eq!(pacer.wait(now + 11 * MS), Duration::ZERO);
    }

    #[test]
    fn a_frame_that_starts_late_does_not_make_the_next_one_late() {
        let begin = Instant::now();
        let mut pacer = FramePacer::new(100, begin);
        pacer.start(begin);
        // Due at 10 ms, started at 12: the one after is still due at 20.
        pacer.start(begin + 12 * MS);
        assert_eq!(pacer.wait(begin + 12 * MS), 8 * MS);
    }

    #[test]
    fn frames_started_when_due_come_out_at_the_limit() {
        let begin = Instant::now();
        let mut pacer = FramePacer::new(250, begin);
        let mut now = begin;
        let mut frames = 0;
        loop {
            now += pacer.wait(now);
            if now >= begin + Duration::from_secs(1) {
                break;
            }
            pacer.start(now);
            frames += 1;
            // Each frame takes a little while, and starts a little late.
            now += Duration::from_micros(300);
        }
        assert_eq!(frames, 250);
    }

    #[test]
    fn slow_frames_are_not_made_up_for_with_a_burst() {
        let begin = Instant::now();
        let mut pacer = FramePacer::new(100, begin);
        pacer.start(begin);
        // A stall of half a second, where fifty frames were due.
        let after = begin + 500 * MS;
        assert_eq!(pacer.wait(after), Duration::ZERO);
        pacer.start(after);
        assert_eq!(pacer.wait(after), Duration::ZERO);
        pacer.start(after);
        assert_eq!(pacer.wait(after), 10 * MS);
    }

    #[test]
    fn a_limit_too_low_for_the_simulation_is_raised() {
        let now = Instant::now();
        let mut pacer = FramePacer::new(5, now);
        pacer.start(now);
        assert_eq!(pacer.wait(now), Duration::from_secs(1) / LOWEST_LIMIT);
    }
}
