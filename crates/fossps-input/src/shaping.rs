//! Turning raw analog readings into clean values: dead zones, curves and thresholds.

/// A trigger counts as pressed once it is pulled this far.
pub(crate) const PRESS_AT: f32 = 0.5;

/// A pressed trigger counts as released once it falls back below this.
///
/// Being lower than [`PRESS_AT`] stops a trigger held near the threshold from
/// flickering between pressed and released.
pub(crate) const RELEASE_AT: f32 = 0.4;

/// The part of a thumbstick's travel that is ignored.
///
/// Sticks rarely rest at exactly zero or reach exactly one. Readings shorter
/// than `inner` become zero, readings longer than `outer` become full tilt, and
/// everything between is rescaled so the output still runs smoothly from 0 to 1.
/// The dead zone is circular, so aiming is not pulled toward the axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Deadzone {
    /// Tilt below this is treated as a stick at rest. Default `0.15`.
    pub inner: f32,
    /// Tilt above this is treated as fully pushed. Default `0.95`.
    pub outer: f32,
}

impl Default for Deadzone {
    fn default() -> Self {
        Self {
            inner: 0.15,
            outer: 0.95,
        }
    }
}

impl Deadzone {
    /// Applies the dead zone, then raises the remaining tilt to `curve`.
    ///
    /// A `curve` of 1 is linear. Higher values give finer control near the
    /// centre, which suits aiming. The result never exceeds length 1.
    pub(crate) fn apply(self, stick: [f32; 2], curve: f32) -> [f32; 2] {
        let length = stick[0].hypot(stick[1]);
        // Settings that are not numbers fall back to the defaults: clamping
        // against NaN would panic.
        let fallback = Self::default();
        let inner = if self.inner.is_finite() {
            self.inner.clamp(0.0, 0.99)
        } else {
            fallback.inner
        };
        let outer = if self.outer.is_finite() {
            self.outer.clamp(inner + 0.01, 1.0)
        } else {
            fallback.outer.max(inner + 0.01)
        };
        if length <= inner {
            return [0.0, 0.0];
        }
        let tilt = ((length - inner) / (outer - inner)).min(1.0);
        let curve = if curve.is_finite() {
            curve.max(0.01)
        } else {
            1.0
        };
        let scale = tilt.powf(curve) / length;
        [stick[0] * scale, stick[1] * scale]
    }
}

/// Makes a device reading safe to store: finite and within `-1..=1`.
pub(crate) fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// The length of a 2D value.
pub(crate) fn length(value: [f32; 2]) -> f32 {
    value[0].hypot(value[1])
}

/// Shrinks a 2D value to length 1 if it is longer, so diagonals are not faster.
pub(crate) fn clamp_length(value: [f32; 2]) -> [f32; 2] {
    let length = length(value);
    if length > 1.0 {
        [value[0] / length, value[1] / length]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn resting_stick_reads_zero() {
        let zone = Deadzone::default();
        assert_eq!(zone.apply([0.14, 0.0], 1.0), [0.0, 0.0]);
        assert_eq!(zone.apply([0.1, 0.1], 1.0), [0.0, 0.0]);
    }

    #[test]
    fn output_starts_at_zero_at_the_edge() {
        let zone = Deadzone::default();
        let just_outside = zone.apply([0.16, 0.0], 1.0);
        assert!(just_outside[0] > 0.0 && just_outside[0] < 0.02);
    }

    #[test]
    fn full_tilt_reaches_one() {
        let zone = Deadzone::default();
        assert!(close(zone.apply([0.95, 0.0], 1.0)[0], 1.0));
        assert!(close(zone.apply([1.0, 0.0], 1.0)[0], 1.0));
        assert!(close(zone.apply([0.0, -1.0], 2.0)[1], -1.0));
    }

    #[test]
    fn diagonals_keep_their_direction_and_never_exceed_one() {
        let zone = Deadzone::default();
        let out = zone.apply([1.0, 1.0], 1.0);
        assert!(close(out[0], out[1]));
        assert!(close(length(out), 1.0));
    }

    #[test]
    fn curve_softens_small_tilts() {
        let zone = Deadzone::default();
        let linear = zone.apply([0.5, 0.0], 1.0)[0];
        let curved = zone.apply([0.5, 0.0], 2.0)[0];
        assert!(curved < linear);
        assert!(close(curved, linear * linear));
    }

    #[test]
    fn nonsense_settings_do_not_produce_nonsense() {
        let zone = Deadzone {
            inner: 2.0,
            outer: -1.0,
        };
        let out = zone.apply([0.5, 0.5], 0.0);
        assert!(out[0].is_finite() && out[1].is_finite());

        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for zone in [
                Deadzone {
                    inner: bad,
                    outer: 0.95,
                },
                Deadzone {
                    inner: 0.15,
                    outer: bad,
                },
                Deadzone {
                    inner: bad,
                    outer: bad,
                },
            ] {
                let out = zone.apply([0.5, 0.5], bad);
                assert!(out[0].is_finite() && out[1].is_finite(), "{zone:?}");
            }
        }
    }

    #[test]
    fn sanitize_rejects_bad_readings() {
        assert_eq!(sanitize(f32::NAN), 0.0);
        assert_eq!(sanitize(f32::INFINITY), 0.0);
        assert_eq!(sanitize(3.0), 1.0);
        assert_eq!(sanitize(-0.25), -0.25);
    }

    #[test]
    fn clamp_length_normalises_only_long_values() {
        let out = clamp_length([1.0, 1.0]);
        assert!(close(length(out), 1.0));
        assert_eq!(clamp_length([0.3, 0.4]), [0.3, 0.4]);
    }
}
