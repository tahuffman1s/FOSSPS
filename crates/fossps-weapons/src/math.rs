//! The little arithmetic on directions and places that weapons need.

pub(crate) type V3 = [f32; 3];

pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: V3, by: f32) -> V3 {
    [a[0] * by, a[1] * by, a[2] * by]
}

pub(crate) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

pub(crate) fn finite(a: V3) -> bool {
    a.iter().all(|part| part.is_finite())
}

/// `a` made one unit long, and how long it was. `None` when it has no
/// length or is not made of numbers.
pub(crate) fn unit(a: V3) -> Option<(V3, f32)> {
    let long = length(a);
    (long.is_finite() && long > 1.0e-12).then(|| (scale(a, 1.0 / long), long))
}

/// `value`, or zero if it is negative or not a number. Infinity becomes
/// the largest number there is, so "no limit" settings mean what they say.
pub(crate) fn positive(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, f32::MAX)
    }
}

/// From `from` (at 0) to `to` (at 1), with `share` kept inside 0 to 1.
pub(crate) fn blend(from: f32, to: f32, share: f32) -> f32 {
    let share = if share.is_finite() {
        share.clamp(0.0, 1.0)
    } else {
        0.0
    };
    from + (to - from) * share
}
