//! Moving a shape through the level: stop at what it touches, then carry
//! on along the surface.

use parry3d::math::Vector;
use parry3d::shape::Capsule;

use crate::level::Level;

/// The gap kept between the player and every surface, in metres. Resting a
/// hair away instead of exactly touching keeps the arithmetic well behaved.
pub(crate) const SKIN: f32 = 0.01;

/// How many surfaces one move may be turned aside by. Four covers a floor,
/// two walls of a corner and a spare, as in Quake.
const MOST_TURNS: usize = 4;

/// How far out of a surface a stuck shape is pushed in one tick, at most.
const MOST_PUSHES: usize = 4;

/// Where a move ended up.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slid {
    /// The shape's centre after the move.
    pub position: Vector,
    /// The velocity left after being turned aside.
    pub velocity: Vector,
    /// Something too steep to stand on got in the way.
    pub blocked: bool,
}

/// Moves `shape` from `position` at `velocity` for `dt` seconds, sliding
/// along whatever it meets.
///
/// Surfaces whose direction has a y below `steep` are too steep to stand
/// on. With `upright_walls` they are also treated as vertical, so walking
/// into a steep bank does not carry the player up it.
pub(crate) fn slide(
    level: &Level,
    shape: &Capsule,
    mut position: Vector,
    mut velocity: Vector,
    dt: f32,
    steep: f32,
    upright_walls: bool,
) -> Slid {
    let heading = velocity;
    let mut surfaces = [Vector::ZERO; MOST_TURNS];
    let mut time = dt;
    let mut blocked = false;

    for met in 0..MOST_TURNS {
        let travel = velocity * time;
        if travel.length_squared() < 1.0e-12 {
            break;
        }
        let Some(touch) = level.sweep(shape, position, travel, SKIN) else {
            position += travel;
            break;
        };
        position += travel * touch.fraction;
        time *= 1.0 - touch.fraction;

        let mut normal = touch.normal;
        if normal.y < steep {
            blocked = true;
            if upright_walls && normal.y > 0.0 {
                let Some(upright) = Vector::new(normal.x, 0.0, normal.z).try_normalize() else {
                    break;
                };
                normal = upright;
            }
        }
        surfaces[met] = normal;
        let surfaces = &surfaces[..=met];

        // Prefer a direction along one surface that enters none of the others.
        let along_one = surfaces.iter().enumerate().find_map(|(i, &surface)| {
            let turned = along(velocity, surface);
            surfaces
                .iter()
                .enumerate()
                .all(|(j, other)| i == j || turned.dot(*other) >= -1.0e-5)
                .then_some(turned)
        });
        velocity = match (along_one, surfaces) {
            (Some(turned), _) => turned,
            // Two surfaces meeting in a crease: follow the crease.
            (None, [first, second]) => match first.cross(*second).try_normalize() {
                Some(crease) => crease * crease.dot(velocity),
                None => Vector::ZERO,
            },
            // Boxed in by three or more.
            (None, _) => Vector::ZERO,
        };
        // Never turn back on the way we set out: that is how corners jitter.
        if velocity.dot(heading) <= 0.0 {
            velocity = Vector::ZERO;
            break;
        }
    }

    Slid {
        position,
        velocity,
        blocked,
    }
}

/// `velocity` with the part heading into a surface facing `normal` removed.
fn along(velocity: Vector, normal: Vector) -> Vector {
    velocity - normal * velocity.dot(normal)
}

/// Moves a shape that is inside something solid, or much nearer to it
/// than the gap it should keep, back out to that gap. This is what frees a
/// player who spawned in a floor.
pub(crate) fn unstick(level: &Level, shape: &Capsule, mut centre: Vector) -> Vector {
    for _ in 0..MOST_PUSHES {
        match level.nearest(shape, centre, SKIN) {
            Some((normal, distance)) if distance < SKIN / 2.0 => {
                centre += normal * (SKIN - distance);
            }
            _ => break,
        }
    }
    centre
}
