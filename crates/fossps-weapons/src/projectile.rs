//! Things that take time to arrive: rockets, grenades and slow bullets.

use fossps_physics::Level;

use crate::math::{V3, add, dot, finite, length, positive, scale, sub, unit};
use crate::settings::{Delivery, ProjectileSettings, WeaponSettings};
use crate::target::{Hit, Struck, Target, distance_to, nearest_on_spine, trace};

/// How long after launch a projectile cannot touch whoever fired it, so it
/// gets clear of their own body.
const CLEAR_OF_OWNER: f32 = 0.25;

/// How far off a surface a bounce or an explosion is placed, in metres, so
/// the next look from there does not start inside the surface.
const OFF_SURFACE: f32 = 0.01;

/// How much of its speed along a surface a bounce keeps.
const BOUNCE_GRIP: f32 = 0.8;

/// A bounce off the ground slower than this, in metres per second, ends
/// with the projectile lying still.
const REST_SPEED: f32 = 0.5;

/// A surface facing at least this far upwards (the y of its direction) is
/// ground a projectile can come to rest on.
const RESTING_GROUND: f32 = 0.7;

/// What a projectile did during one tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flight {
    /// Still on its way (or lying still, waiting for its fuse).
    Flying,
    /// It bounced and carries on.
    Bounced {
        /// Where it bounced.
        point: [f32; 3],
        /// Which way the surface faces there.
        normal: [f32; 3],
    },
    /// It stopped in what it struck, like a bullet, and its flight is over.
    /// The hit's damage is the weapon's at the distance flown.
    Struck(Hit),
    /// It exploded and its flight is over. Use
    /// [`blast_damage`] to find out who was hurt.
    Exploded {
        /// The centre of the explosion.
        point: [f32; 3],
        /// The target it went off against, if it touched one.
        touched: Option<u32>,
    },
    /// Its time ran out with nothing to show for it; its flight is over.
    Expired,
}

impl Flight {
    /// Whether the flight is over and the projectile should be removed.
    #[must_use]
    pub fn is_over(&self) -> bool {
        matches!(
            self,
            Self::Struck(_) | Self::Exploded { .. } | Self::Expired
        )
    }
}

/// A rocket, grenade or slow bullet on its way.
///
/// Like a weapon it is small and `Copy`, and the same projectile, level,
/// targets, settings and `dt` always give the same next step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projectile {
    /// Where it is, in metres.
    pub position: [f32; 3],
    /// How fast it is moving, in metres per second.
    pub velocity: [f32; 3],
    /// Seconds since it was launched.
    pub age: f32,
    /// How far it has flown, in metres.
    pub travelled: f32,
    /// The `id` of the target that launched it, who is not hit by it in
    /// the first quarter of a second.
    pub owner: Option<u32>,
}

impl Projectile {
    /// A projectile leaving `from` along `direction` at the weapon's speed.
    ///
    /// `direction` normally comes from
    /// [`Shot::direction`](crate::Shot::direction). Returns `None` if the
    /// weapon's delivery is not [`Delivery::Projectile`] or `direction`
    /// has no length.
    #[must_use]
    pub fn launch(
        settings: &WeaponSettings,
        from: [f32; 3],
        direction: [f32; 3],
        owner: Option<u32>,
    ) -> Option<Self> {
        let Delivery::Projectile(flight) = settings.delivery else {
            return None;
        };
        let (heading, _) = unit(direction)?;
        finite(from).then(|| Self {
            position: from,
            velocity: scale(heading, positive(flight.speed)),
            age: 0.0,
            travelled: 0.0,
            owner,
        })
    }

    /// Flies the projectile for one tick of `dt` seconds and says what
    /// became of it.
    ///
    /// The whole path of the tick is checked, so a fast projectile does not
    /// pass through a thin wall or a player. Once the result
    /// [`is_over`](Flight::is_over), remove the projectile. The fuse is
    /// checked once per tick, so it goes off up to one tick late.
    ///
    /// A weapon whose delivery is not [`Delivery::Projectile`], or a `dt`
    /// that is zero, negative or not a number, gives [`Flight::Expired`].
    pub fn step(
        &mut self,
        level: &Level,
        targets: &[Target],
        settings: &WeaponSettings,
        dt: f32,
    ) -> Flight {
        let Delivery::Projectile(flight) = settings.delivery else {
            return Flight::Expired;
        };
        let sound = dt.is_finite() && dt > 0.0 && finite(self.position) && finite(self.velocity);
        if !sound {
            return Flight::Expired;
        }
        let explodes = positive(flight.blast_radius) > 0.0;
        self.age = positive(self.age) + dt;
        if self.age >= positive(flight.fuse) {
            return if explodes {
                Flight::Exploded {
                    point: self.position,
                    touched: None,
                }
            } else {
                Flight::Expired
            };
        }
        // Lying still after its last bounce: only the fuse matters now.
        if self.velocity == [0.0; 3] && self.travelled > 0.0 {
            return Flight::Flying;
        }

        self.velocity[1] -= positive(flight.gravity) * dt;
        self.velocity = scale(self.velocity, 1.0 / (1.0 + positive(flight.drag) * dt));
        let travel = scale(self.velocity, dt);
        let Some((heading, far)) = unit(travel) else {
            return Flight::Flying;
        };
        let skip = self.owner.filter(|_| self.age < CLEAR_OF_OWNER);
        let Some(mut hit) = trace(level, targets, self.position, heading, far, skip) else {
            self.position = add(self.position, travel);
            self.travelled += far;
            return Flight::Flying;
        };
        self.travelled += hit.distance;
        self.position = add(hit.point, scale(hit.normal, OFF_SURFACE));

        if flight.explodes_on_touch && explodes {
            return Flight::Exploded {
                point: self.position,
                touched: hit.target(),
            };
        }
        let bounciness = positive(flight.bounciness).min(1.0);
        if bounciness > 0.0 {
            self.bounce(&flight, hit.normal, bounciness);
            return Flight::Bounced {
                point: hit.point,
                normal: hit.normal,
            };
        }
        if let Struck::Target { zone, .. } = hit.struck {
            hit.damage = settings.damage_at(self.travelled, zone);
        }
        hit.distance = self.travelled;
        Flight::Struck(hit)
    }

    /// Turns the projectile's speed around off a surface. What is left of
    /// the tick after the bounce is not flown.
    fn bounce(&mut self, flight: &ProjectileSettings, normal: V3, bounciness: f32) {
        let into = dot(self.velocity, normal);
        let along = sub(self.velocity, scale(normal, into));
        self.velocity = add(scale(along, BOUNCE_GRIP), scale(normal, -into * bounciness));
        let on_ground = normal[1] >= RESTING_GROUND && positive(flight.gravity) > 0.0;
        if on_ground && length(self.velocity) < REST_SPEED {
            self.velocity = [0.0; 3];
        }
    }
}

/// The damage an explosion at `centre` does to `target`.
///
/// It is the weapon's `damage` for a target touching the centre, easing
/// down to the projectile's `blast_edge_damage` at its `blast_radius`, and
/// nothing beyond that or when the level is in the way of both the
/// target's middle and its head. Zero for a weapon that does not explode.
#[must_use]
pub fn blast_damage(
    level: &Level,
    settings: &WeaponSettings,
    centre: [f32; 3],
    target: &Target,
) -> f32 {
    let Delivery::Projectile(flight) = settings.delivery else {
        return 0.0;
    };
    let radius = positive(flight.blast_radius);
    let distance = distance_to(target, centre);
    if !(finite(centre) && distance < radius) {
        return 0.0;
    }
    let middle = nearest_on_spine(target, centre);
    let head = nearest_on_spine(target, [centre[0], f32::MAX, centre[2]]);
    let in_sight = |point: V3| {
        unit(sub(point, centre))
            .is_none_or(|(heading, far)| level.raycast(centre, heading, far).is_none())
    };
    if !(in_sight(middle) || in_sight(head)) {
        return 0.0;
    }
    let (near, edge) = (
        positive(settings.damage),
        positive(flight.blast_edge_damage),
    );
    near + (edge - near) * (distance / radius)
}
