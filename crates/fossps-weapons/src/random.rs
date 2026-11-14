//! Chance that a server and a client roll the same way.

/// The number to hand [`Weapon::step`](crate::Weapon::step) for one player's
/// tick.
///
/// Every roll a weapon makes during that tick (where in the spread a bullet
/// goes, which way the recoil pulls) is worked out from this number alone,
/// so a server and a client that agree on who fired and on which tick see
/// the same bullets. Nothing is remembered between ticks.
#[must_use]
pub fn shot_seed(player: u32, tick: u32) -> u32 {
    mix(mix(player ^ 0x9E37_79B9) ^ tick)
}

/// Scrambles a number so that close inputs give unrelated outputs.
pub(crate) fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}

/// A short run of rolls that follows from one starting number.
pub(crate) struct Rolls(u32);

impl Rolls {
    /// The rolls for one thing (`which`) in one shot (`shot`) of a tick.
    pub(crate) fn new(seed: u32, shot: u32, which: u32) -> Self {
        Self(mix(mix(seed ^ shot.wrapping_mul(0x9E37_79B9)) ^ which))
    }

    /// The next roll, from 0 up to but not including 1.
    pub(crate) fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9);
        // 24 bits: every one of them fits a float exactly.
        (mix(self.0) >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}
