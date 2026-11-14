//! The numbers that make one weapon a rifle and another a rocket launcher.

use crate::math::{blend, positive};
use crate::target::Zone;

/// What pulling the trigger does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireMode {
    /// Fires for as long as the trigger is held.
    Automatic,
    /// One shot for each pull of the trigger.
    SemiAutomatic,
    /// This many shots for each pull, finished even if the trigger is let go.
    Burst(u32),
}

/// How a shot reaches what it hits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Delivery {
    /// It lands the moment it is fired, anywhere within `reach` metres.
    /// Right for most guns: see [`hitscan`](crate::hitscan).
    Hitscan {
        /// How far the shot carries, in metres. `f32::INFINITY` is as far
        /// as the level goes.
        reach: f32,
    },
    /// It flies through the level and takes time to arrive: rockets,
    /// grenades, and bullets that should drop over distance. See
    /// [`Projectile`](crate::Projectile).
    Projectile(ProjectileSettings),
}

/// How a rocket, grenade or slow bullet flies and what ends its flight.
///
/// What happens when it touches something is decided in this order: it
/// explodes if `explodes_on_touch`; otherwise it bounces if `bounciness`
/// is above zero; otherwise it stops there and hurts what it struck, like
/// a bullet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileSettings {
    /// How fast it leaves the weapon, in metres per second.
    pub speed: f32,
    /// How fast it speeds up downwards, in metres per second each second.
    /// The player's own gravity is 20; 0 flies dead straight.
    pub gravity: f32,
    /// How quickly the air slows it. 0 is no slowing; at 1 it keeps about
    /// a third of its speed after the first second.
    pub drag: f32,
    /// How much of its speed into a surface a bounce gives back, from 0
    /// (none: it does not bounce) to 1 (all of it). Every bounce also
    /// loses a fifth of the speed along the surface, so nothing rolls
    /// forever.
    pub bounciness: f32,
    /// Seconds until it goes off by itself. It explodes then if it has a
    /// `blast_radius`, and otherwise just disappears. `f32::INFINITY` is
    /// no fuse at all.
    pub fuse: f32,
    /// Whether touching the level or a player sets it off at once.
    pub explodes_on_touch: bool,
    /// How far from the explosion players are hurt, in metres. 0 is no
    /// explosion.
    pub blast_radius: f32,
    /// The damage done at the very edge of the blast. At the centre it is
    /// the weapon's `damage`.
    pub blast_edge_damage: f32,
}

impl Default for ProjectileSettings {
    /// A rocket: fast, straight, and it explodes on the first thing it meets.
    fn default() -> Self {
        Self {
            speed: 40.0,
            gravity: 0.0,
            drag: 0.0,
            bounciness: 0.0,
            fuse: 8.0,
            explodes_on_touch: true,
            blast_radius: 5.0,
            blast_edge_damage: 10.0,
        }
    }
}

/// The numbers that shape one kind of weapon. Lengths are metres, times
/// seconds, angles degrees.
///
/// The default is an assault rifle. Start from it, or from
/// [`pistol`](Self::pistol), [`shotgun`](Self::shotgun),
/// [`rocket_launcher`](Self::rocket_launcher) or [`grenade`](Self::grenade),
/// change single fields and keep the rest:
///
/// ```
/// let smg = fossps_weapons::WeaponSettings {
///     rounds_per_minute: 900.0,
///     damage: 22.0,
///     ..Default::default()
/// };
/// # assert_eq!(smg.magazine_size, 30);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponSettings {
    /// What pulling the trigger does.
    pub fire_mode: FireMode,
    /// How fast it fires. The rate is kept exactly, whatever the tick rate.
    pub rounds_per_minute: f32,
    /// The wait after a burst before the next one, on top of the usual gap
    /// between shots. Only [`FireMode::Burst`] uses it.
    pub burst_pause: f32,
    /// How long after the weapon is brought up before it can fire.
    pub raise_time: f32,
    /// How each shot reaches what it hits.
    pub delivery: Delivery,
    /// How many things leave the barrel per shot. A shotgun has several;
    /// each does `damage` on its own, and one shot uses one round.
    pub pellets: u32,

    /// Rounds in a full magazine.
    pub magazine_size: u32,
    /// Spare rounds carried when the weapon is new.
    pub reserve: u32,
    /// How long a reload takes with rounds still in the magazine. For a
    /// weapon loaded `one_at_a_time`, how long each round takes.
    pub reload_time: f32,
    /// How long a reload takes from an empty magazine.
    pub empty_reload_time: f32,
    /// How far through the reload the rounds arrive, from 0 to 1. A reload
    /// cut short before this gives nothing.
    pub reload_ammo_at: f32,
    /// Rounds go in singly, like shells into a shotgun, and pulling the
    /// trigger stops loading to fire what is in.
    pub one_at_a_time: bool,
    /// Reloading before the magazine is empty leaves a round in the
    /// chamber, so the weapon then holds one more than `magazine_size`.
    pub keeps_chambered_round: bool,
    /// Pulling or holding the trigger on an empty magazine starts a reload.
    pub auto_reload: bool,

    /// Damage of one hit to the body at close range.
    pub damage: f32,
    /// Up to this distance a hit does full `damage`.
    pub falloff_start: f32,
    /// From this distance on a hit does `far_damage`; in between it eases
    /// from one to the other.
    pub falloff_end: f32,
    /// Damage of one hit to the body at `falloff_end` and beyond.
    pub far_damage: f32,
    /// What a hit to the head is multiplied by.
    pub head_multiplier: f32,
    /// What a hit to the legs is multiplied by.
    pub leg_multiplier: f32,

    /// How far off the aim a shot can land when firing from the hip,
    /// standing still: the half-width of the cone, in degrees.
    pub spread: f32,
    /// The same while aiming down the sights.
    pub aiming_spread: f32,
    /// Added to the spread by every shot fired.
    pub spread_per_shot: f32,
    /// The most that firing can add.
    pub most_added_spread: f32,
    /// How long after a shot the added spread starts to shrink.
    pub spread_recovery_delay: f32,
    /// How fast the added spread shrinks, in degrees per second.
    pub spread_recovery: f32,
    /// What the spread is multiplied by at walking speed or faster.
    pub moving_spread: f32,
    /// What the spread is multiplied by in the air.
    pub airborne_spread: f32,
    /// What the spread is multiplied by when crouched.
    pub crouched_spread: f32,

    /// How far each shot throws the view upwards, in degrees.
    pub kick_up: f32,
    /// The most each shot throws the view left or right; how much and
    /// which way is rolled per shot.
    pub kick_sideways: f32,
    /// The furthest the view is thrown from where it was aimed, each way.
    pub most_kick: f32,
    /// How long after a shot the view starts to settle back.
    pub kick_recovery_delay: f32,
    /// How fast the view settles back, in degrees per second.
    pub kick_recovery: f32,
}

impl Default for WeaponSettings {
    /// An assault rifle.
    fn default() -> Self {
        Self {
            fire_mode: FireMode::Automatic,
            rounds_per_minute: 750.0,
            burst_pause: 0.15,
            raise_time: 0.4,
            delivery: Delivery::Hitscan { reach: 500.0 },
            pellets: 1,

            magazine_size: 30,
            reserve: 120,
            reload_time: 2.0,
            empty_reload_time: 2.6,
            reload_ammo_at: 0.75,
            one_at_a_time: false,
            keeps_chambered_round: false,
            auto_reload: true,

            damage: 30.0,
            falloff_start: 25.0,
            falloff_end: 60.0,
            far_damage: 21.0,
            head_multiplier: 2.0,
            leg_multiplier: 0.8,

            spread: 1.0,
            aiming_spread: 0.1,
            spread_per_shot: 0.08,
            most_added_spread: 2.0,
            spread_recovery_delay: 0.1,
            spread_recovery: 6.0,
            moving_spread: 2.0,
            airborne_spread: 4.0,
            crouched_spread: 0.7,

            kick_up: 0.45,
            kick_sideways: 0.15,
            most_kick: 8.0,
            kick_recovery_delay: 0.12,
            kick_recovery: 12.0,
        }
    }
}

impl WeaponSettings {
    /// A sidearm: one accurate shot per pull, quick to reload.
    #[must_use]
    pub fn pistol() -> Self {
        Self {
            fire_mode: FireMode::SemiAutomatic,
            rounds_per_minute: 400.0,
            raise_time: 0.25,
            magazine_size: 12,
            reserve: 48,
            reload_time: 1.4,
            empty_reload_time: 1.7,
            damage: 26.0,
            falloff_start: 12.0,
            falloff_end: 35.0,
            far_damage: 14.0,
            spread: 0.6,
            spread_per_shot: 0.5,
            kick_up: 1.2,
            kick_sideways: 0.3,
            ..Self::default()
        }
    }

    /// A pump shotgun: eight pellets a shot, loaded one shell at a time.
    #[must_use]
    pub fn shotgun() -> Self {
        Self {
            fire_mode: FireMode::SemiAutomatic,
            rounds_per_minute: 70.0,
            delivery: Delivery::Hitscan { reach: 60.0 },
            pellets: 8,
            magazine_size: 6,
            reserve: 24,
            reload_time: 0.5,
            empty_reload_time: 0.5,
            one_at_a_time: true,
            damage: 14.0,
            falloff_start: 6.0,
            falloff_end: 20.0,
            far_damage: 3.0,
            head_multiplier: 1.5,
            spread: 3.0,
            aiming_spread: 2.5,
            spread_per_shot: 0.0,
            kick_up: 4.0,
            kick_sideways: 1.0,
            ..Self::default()
        }
    }

    /// One rocket at a time that explodes where it lands.
    #[must_use]
    pub fn rocket_launcher() -> Self {
        Self {
            fire_mode: FireMode::SemiAutomatic,
            rounds_per_minute: 60.0,
            raise_time: 0.8,
            delivery: Delivery::Projectile(ProjectileSettings::default()),
            magazine_size: 1,
            reserve: 3,
            reload_time: 2.8,
            empty_reload_time: 2.8,
            damage: 120.0,
            spread: 0.0,
            aiming_spread: 0.0,
            spread_per_shot: 0.0,
            kick_up: 3.0,
            kick_sideways: 0.5,
            ..Self::default()
        }
    }

    /// A hand grenade: thrown in an arc, it bounces and goes off after
    /// three seconds.
    #[must_use]
    pub fn grenade() -> Self {
        Self {
            fire_mode: FireMode::SemiAutomatic,
            rounds_per_minute: 60.0,
            raise_time: 0.3,
            delivery: Delivery::Projectile(ProjectileSettings {
                speed: 18.0,
                gravity: 20.0,
                drag: 0.0,
                bounciness: 0.45,
                fuse: 3.0,
                explodes_on_touch: false,
                blast_radius: 6.0,
                blast_edge_damage: 10.0,
            }),
            magazine_size: 1,
            reserve: 1,
            reload_time: 0.6,
            empty_reload_time: 0.6,
            damage: 140.0,
            spread: 0.0,
            aiming_spread: 0.0,
            spread_per_shot: 0.0,
            kick_up: 0.0,
            kick_sideways: 0.0,
            ..Self::default()
        }
    }

    /// The damage of one hit from `distance` metres away on `zone`.
    ///
    /// ```
    /// use fossps_weapons::{WeaponSettings, Zone};
    ///
    /// let rifle = WeaponSettings::default();
    /// assert_eq!(rifle.damage_at(10.0, Zone::Body), 30.0);
    /// assert_eq!(rifle.damage_at(10.0, Zone::Head), 60.0);
    /// assert_eq!(rifle.damage_at(100.0, Zone::Body), 21.0);
    /// ```
    #[must_use]
    pub fn damage_at(&self, distance: f32, zone: Zone) -> f32 {
        let (near, far) = (positive(self.damage), positive(self.far_damage));
        let start = positive(self.falloff_start);
        let span = positive(self.falloff_end) - start;
        let base = if span > 0.0 {
            blend(near, far, (distance - start) / span)
        } else if distance > start {
            far
        } else {
            near
        };
        base * match zone {
            Zone::Head => positive(self.head_multiplier),
            Zone::Body => 1.0,
            Zone::Legs => positive(self.leg_multiplier),
        }
    }

    /// Seconds between two shots.
    pub(crate) fn shot_interval(&self) -> f32 {
        let rate = if self.rounds_per_minute.is_finite() {
            self.rounds_per_minute.clamp(1.0, 60_000.0)
        } else {
            600.0
        };
        60.0 / rate
    }

    /// How long one reload (or one round of a one-at-a-time reload) takes.
    pub(crate) fn reload_duration(&self, from_empty: bool) -> f32 {
        positive(if from_empty && !self.one_at_a_time {
            self.empty_reload_time
        } else {
            self.reload_time
        })
    }

    /// Rounds in a full magazine: never none.
    pub(crate) fn magazine(&self) -> u32 {
        self.magazine_size.max(1)
    }
}
