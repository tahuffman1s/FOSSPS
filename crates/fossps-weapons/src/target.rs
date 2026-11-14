//! What can be shot: players, and the level behind them.

use fossps_physics::Level;

use crate::math::{V3, add, dot, finite, positive, scale, sub, unit};
use crate::settings::{Delivery, WeaponSettings};

/// Hits at or above this share of a target's height are to the head.
const HEAD_FROM: f32 = 0.85;

/// Hits below this share of a target's height are to the legs.
const LEGS_BELOW: f32 = 0.45;

/// Something that can be shot: a player's body, as an upright capsule.
///
/// To shoot at where players were a moment ago (lag compensation), pass
/// targets built from their old positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// Your own number for who this is; it comes back in the [`Hit`].
    pub id: u32,
    /// Where the feet are, in metres.
    pub feet: [f32; 3],
    /// Feet to top of head: 1.8 standing and 1.1 crouched with the default
    /// movement settings.
    pub height: f32,
    /// Half the body's width: 0.35 with the default movement settings.
    pub radius: f32,
}

/// Where on a body a hit landed.
///
/// Until characters have skeletons this goes by height alone: the top 15%
/// of a target is its head and the bottom 45% its legs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// The head.
    Head,
    /// The chest, stomach and arms.
    Body,
    /// The legs and feet.
    Legs,
}

/// What a shot ran into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Struck {
    /// The level: a wall, the floor.
    Level,
    /// One of the targets.
    Target {
        /// The target's `id`.
        id: u32,
        /// Where on the body.
        zone: Zone,
    },
}

/// Where a shot landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// What was hit.
    pub struck: Struck,
    /// Where, in metres.
    pub point: [f32; 3],
    /// Which way the surface faces there, one unit long.
    pub normal: [f32; 3],
    /// How far the shot travelled, in metres.
    pub distance: f32,
    /// The damage done: nothing for the level; for a target, the weapon's
    /// damage at that distance on that part of the body.
    pub damage: f32,
}

impl Hit {
    /// The target that was hit, if it was one.
    #[must_use]
    pub fn target(&self) -> Option<u32> {
        match self.struck {
            Struck::Level => None,
            Struck::Target { id, .. } => Some(id),
        }
    }
}

/// Fires a shot that lands at once: the first thing along `direction` from
/// `from`, whether the level or one of `targets`, and the damage done.
///
/// `from` is normally the shooter's eyes and `direction` comes from
/// [`Shot::direction`](crate::Shot::direction). `shooter` is the `id` of
/// the target that fired, so they do not shoot themselves. The shot carries
/// as far as the weapon's [`Delivery::Hitscan`](crate::Delivery::Hitscan)
/// `reach`.
///
/// Returns `None` when nothing is in the way. A shotgun calls this once per
/// pellet and adds the damage up.
///
/// ```
/// use fossps_physics::Level;
/// use fossps_weapons::{hitscan, Target, WeaponSettings, Zone, Struck};
///
/// let level = Level::from_triangles(&[]);
/// let enemy = Target { id: 7, feet: [0.0, 0.0, -10.0], height: 1.8, radius: 0.35 };
/// let rifle = WeaponSettings::default();
///
/// let eyes = [0.0, 1.0, 0.0];
/// let ahead = [0.0, 0.0, -1.0];
/// let hit = hitscan(&level, &[enemy], eyes, ahead, &rifle, None).unwrap();
/// assert_eq!(hit.struck, Struck::Target { id: 7, zone: Zone::Body });
/// assert_eq!(hit.damage, 30.0);
/// ```
#[must_use]
pub fn hitscan(
    level: &Level,
    targets: &[Target],
    from: [f32; 3],
    direction: [f32; 3],
    settings: &WeaponSettings,
    shooter: Option<u32>,
) -> Option<Hit> {
    let reach = match settings.delivery {
        Delivery::Hitscan { reach } => reach,
        // A weapon that launches things has no reach of its own; asked
        // anyway, look as far as its damage is defined.
        Delivery::Projectile(_) => settings.falloff_end,
    };
    let mut hit = trace(level, targets, from, direction, reach, shooter)?;
    if let Struck::Target { zone, .. } = hit.struck {
        hit.damage = settings.damage_at(hit.distance, zone);
    }
    Some(hit)
}

/// The first thing along a line, with no damage worked out.
pub(crate) fn trace(
    level: &Level,
    targets: &[Target],
    from: V3,
    direction: V3,
    reach: f32,
    skip: Option<u32>,
) -> Option<Hit> {
    let (heading, _) = unit(direction)?;
    let mut reach = positive(reach);
    if !finite(from) || reach <= 0.0 {
        return None;
    }
    let mut first = level.raycast(from, heading, reach).map(|wall| Hit {
        struck: Struck::Level,
        point: wall.point,
        normal: wall.normal,
        distance: wall.distance,
        damage: 0.0,
    });
    if let Some(wall) = &first {
        reach = wall.distance;
    }
    for target in targets {
        if skip == Some(target.id) {
            continue;
        }
        let Some((distance, normal)) = ray_capsule(target, from, heading) else {
            continue;
        };
        // Strictly nearer: of two targets at the same distance the earlier
        // in the list is hit, on every machine.
        if distance < reach {
            reach = distance;
            let point = add(from, scale(heading, distance));
            let height = target_height(target);
            let share = (point[1] - target.feet[1]) / height;
            let zone = if share >= HEAD_FROM {
                Zone::Head
            } else if share < LEGS_BELOW {
                Zone::Legs
            } else {
                Zone::Body
            };
            first = Some(Hit {
                struck: Struck::Target {
                    id: target.id,
                    zone,
                },
                point,
                normal,
                distance,
                damage: 0.0,
            });
        }
    }
    first
}

fn target_radius(target: &Target) -> f32 {
    positive(target.radius).max(0.01)
}

/// A target is never shorter than a ball of its own width.
fn target_height(target: &Target) -> f32 {
    positive(target.height).max(2.0 * target_radius(target))
}

/// The heights of the two ends of a target's spine, the line down its
/// middle: the lower first, and the upper never below it.
fn spine(target: &Target) -> (f32, f32) {
    let radius = target_radius(target);
    let low = target.feet[1] + radius;
    (low, low + (target_height(target) - 2.0 * radius).max(0.0))
}

/// The point on a target's spine nearest `point`.
pub(crate) fn nearest_on_spine(target: &Target, point: V3) -> V3 {
    let (low, high) = spine(target);
    // Not `clamp`, which refuses a height that is not a number.
    [target.feet[0], point[1].max(low).min(high), target.feet[2]]
}

/// How far `point` is from the outside of a target; zero when inside it.
pub(crate) fn distance_to(target: &Target, point: V3) -> f32 {
    let gap = sub(point, nearest_on_spine(target, point));
    (dot(gap, gap).sqrt() - target_radius(target)).max(0.0)
}

/// Where a ray from `from` along the unit `heading` first meets a target:
/// how far along, and which way the body's surface faces there.
fn ray_capsule(target: &Target, from: V3, heading: V3) -> Option<(f32, V3)> {
    if !finite(target.feet) {
        return None;
    }
    let radius = target_radius(target);
    let (low, high) = spine(target);

    // Starting inside is a hit at once, facing back at the shooter.
    let inside = sub(from, nearest_on_spine(target, from));
    if dot(inside, inside) <= radius * radius {
        return Some((0.0, scale(heading, -1.0)));
    }

    let mut nearest: Option<f32> = None;
    let mut keep = |distance: f32| {
        if distance >= 0.0 && nearest.is_none_or(|best| distance < best) {
            nearest = Some(distance);
        }
    };
    // Each test finds where the ray passes closest to the middle and steps
    // back from there to the surface; subtracting two large squares
    // instead loses a far-off target to rounding.

    // The straight side: a circle seen from above.
    let (mx, mz) = (from[0] - target.feet[0], from[2] - target.feet[2]);
    let flat = heading[0] * heading[0] + heading[2] * heading[2];
    if flat > 1.0e-12 {
        let closest = -(mx * heading[0] + mz * heading[2]) / flat;
        let (px, pz) = (mx + heading[0] * closest, mz + heading[2] * closest);
        let inner = radius * radius - (px * px + pz * pz);
        if inner >= 0.0 {
            let distance = closest - (inner / flat).sqrt();
            let y = from[1] + heading[1] * distance;
            if (low..=high).contains(&y) {
                keep(distance);
            }
        }
    }
    // The round ends: a ball at the top and one at the bottom.
    for y in [low, high] {
        let m = sub(from, [target.feet[0], y, target.feet[2]]);
        let closest = -dot(m, heading);
        let passing = add(m, scale(heading, closest));
        let inner = radius * radius - dot(passing, passing);
        if inner >= 0.0 {
            keep(closest - inner.sqrt());
        }
    }

    let distance = nearest?;
    let point = add(from, scale(heading, distance));
    let (normal, _) = unit(sub(point, nearest_on_spine(target, point)))?;
    Some((distance, normal))
}
