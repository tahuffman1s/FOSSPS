//! Gamepad rumble: what a game asks the player's gamepad to do.

/// One burst of gamepad rumble: how hard each of the two motors shakes, and
/// for how long.
///
/// A gamepad has a heavy motor that thumps (`strong`) and a light one that
/// buzzes (`weak`). A hit taken is mostly the strong one; a shot fired is
/// mostly the weak one.
///
/// ```
/// use fossps_input::Rumble;
///
/// let shot = Rumble::new(0.2, 0.6, 0.1);
/// assert_eq!(shot.seconds(), 0.1);
///
/// // Numbers out of range are brought into it, so a rumble can never be
/// // stronger than the gamepad's full strength or left on for good.
/// let hit = Rumble::new(3.0, -1.0, 60.0);
/// assert_eq!((hit.strong(), hit.weak()), (1.0, 0.0));
/// assert_eq!(hit.seconds(), Rumble::MAX_SECONDS);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rumble {
    strong: f32,
    weak: f32,
    seconds: f32,
}

impl Rumble {
    /// The longest one rumble lasts. A longer one is cut to this, so a
    /// mistake in a game cannot leave a gamepad shaking.
    pub const MAX_SECONDS: f32 = 5.0;

    /// How long a rumble lasts when a script does not say: long enough to
    /// feel, short enough for a single shot.
    pub const DEFAULT_SECONDS: f32 = 0.2;

    /// No rumble. Asking for it stops the one that is playing.
    pub const STOP: Self = Self {
        strong: 0.0,
        weak: 0.0,
        seconds: 0.0,
    };

    /// A rumble of `strong` and `weak`, each from 0 (off) to 1 (full), for
    /// `seconds`.
    ///
    /// Strengths outside 0 to 1 are brought to the nearest end, `seconds` is
    /// kept between 0 and [`MAX_SECONDS`](Self::MAX_SECONDS), and a number
    /// that is not a number counts as 0.
    #[must_use]
    pub fn new(strong: f32, weak: f32, seconds: f32) -> Self {
        // `clamp` passes NaN through, so it is turned into 0 first.
        let within = |value: f32, most: f32| {
            if value.is_nan() {
                0.0
            } else {
                value.clamp(0.0, most)
            }
        };
        Self {
            strong: within(strong, 1.0),
            weak: within(weak, 1.0),
            seconds: within(seconds, Self::MAX_SECONDS),
        }
    }

    /// How hard the heavy motor shakes, from 0 to 1.
    #[must_use]
    pub fn strong(self) -> f32 {
        self.strong
    }

    /// How hard the light motor shakes, from 0 to 1.
    #[must_use]
    pub fn weak(self) -> f32 {
        self.weak
    }

    /// How long the rumble lasts, in seconds. A gamepad is told in steps of
    /// a twentieth of a second, so 0.12 is felt as 0.15.
    #[must_use]
    pub fn seconds(self) -> f32 {
        self.seconds
    }

    /// Whether this rumble would not be felt: both motors are off, or it
    /// lasts no time. Playing it only stops the rumble before it.
    #[must_use]
    pub fn is_still(self) -> bool {
        self.seconds <= 0.0 || (self.strong <= 0.0 && self.weak <= 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_range_are_kept() {
        let rumble = Rumble::new(0.25, 0.75, 0.5);
        assert_eq!(
            (rumble.strong(), rumble.weak(), rumble.seconds()),
            (0.25, 0.75, 0.5)
        );
        assert!(!rumble.is_still());
    }

    #[test]
    fn numbers_out_of_range_are_brought_into_it() {
        let rumble = Rumble::new(7.0, -2.0, 1000.0);
        assert_eq!(
            (rumble.strong(), rumble.weak(), rumble.seconds()),
            (1.0, 0.0, Rumble::MAX_SECONDS)
        );
        let endless = Rumble::new(f32::INFINITY, 0.0, f32::INFINITY);
        assert_eq!(
            (endless.strong(), endless.seconds()),
            (1.0, Rumble::MAX_SECONDS)
        );
    }

    #[test]
    fn not_a_number_is_no_rumble() {
        let rumble = Rumble::new(f32::NAN, f32::NAN, f32::NAN);
        assert_eq!(rumble, Rumble::STOP);
    }

    #[test]
    fn nothing_to_feel_is_still() {
        assert!(Rumble::STOP.is_still());
        assert!(Rumble::new(1.0, 1.0, 0.0).is_still());
        assert!(Rumble::new(0.0, 0.0, 1.0).is_still());
        assert!(Rumble::new(1.0, 1.0, -1.0).is_still());
        assert!(!Rumble::new(0.0, 0.1, 0.1).is_still());
    }
}
