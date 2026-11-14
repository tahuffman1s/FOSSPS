//! One weapon in a player's hands: its ammunition, its rhythm and its recoil.

use crate::math::{add, blend, length, positive, scale};
use crate::random::Rolls;
use crate::settings::{FireMode, WeaponSettings};

/// The most shots one tick fires, however fast the weapon and however long
/// the tick, so a stalled game cannot empty a magazine in one go.
const MOST_SHOTS_PER_TICK: u32 = 8;

/// The most pellets one shot throws.
const MOST_PELLETS: u32 = 64;

/// The view never tips further than this from level, in radians, so that
/// straight up and straight down keep a left and a right.
const STEEPEST_PITCH: f32 = 1.55;

/// Which roll of a shot decides the sideways kick; pellets use 0 and up.
const KICK_ROLL: u32 = u32::MAX;

/// What the player holding the weapon is doing during one tick.
///
/// Like movement input this is not tied to keys or sticks: fill it from
/// actions, a recorded demo or a network packet alike.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WeaponInput {
    /// The trigger is held.
    pub trigger: bool,
    /// Reload is asked for.
    pub reload: bool,
    /// Aiming down the sights.
    pub aiming: bool,
    /// How fast the player is moving, as a share of walking speed: 0 is
    /// standing still, 1 is walking or faster.
    pub moving: f32,
    /// In the air: jumping or falling.
    pub airborne: bool,
    /// Crouched.
    pub crouched: bool,
}

/// A reload that is under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reload {
    /// Seconds since it began.
    pub elapsed: f32,
    /// It began with nothing in the magazine.
    pub from_empty: bool,
    /// The rounds are already in; only the end of the motion is left.
    pub loaded: bool,
}

/// What happened to a weapon during one tick, for sounds, animations and
/// the HUD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WeaponEvents {
    /// How many shots were fired (a shotgun blast is one).
    pub shots: u32,
    /// The trigger was pulled on an empty magazine: the click.
    pub dry_fire: bool,
    /// A reload began.
    pub reload_started: bool,
    /// A reload ran to its end.
    pub reload_finished: bool,
}

/// One thing leaving the barrel: a bullet, a pellet or a rocket.
///
/// It says when it left and how far off the aim it went;
/// [`direction`](Self::direction) turns that into where it is headed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    /// When in the tick it was fired, in seconds after the tick began.
    pub time: f32,
    /// How far the recoil had thrown the view when it was fired: up, then
    /// left, in radians. This is before the shot's own kick.
    pub kick: [f32; 2],
    /// How far off the centre of the view it left: right, then up, in
    /// radians.
    pub spread: [f32; 2],
}

impl Shot {
    /// Where the shot is headed, one unit long, for a player looking along
    /// `yaw` and `pitch` (in radians, before any recoil).
    ///
    /// `yaw` turns left from -Z as it grows, as in movement; `pitch` is up
    /// from level.
    #[must_use]
    pub fn direction(&self, yaw: f32, pitch: f32) -> [f32; 3] {
        let number = |value: f32| if value.is_finite() { value } else { 0.0 };
        let yaw = number(yaw) + number(self.kick[1]);
        let pitch = (number(pitch) + number(self.kick[0])).clamp(-STEEPEST_PITCH, STEEPEST_PITCH);
        let (sin_yaw, cos_yaw) = (libm::sinf(yaw), libm::cosf(yaw));
        let (sin_pitch, cos_pitch) = (libm::sinf(pitch), libm::cosf(pitch));
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let right = [cos_yaw, 0.0, -sin_yaw];
        let up = [sin_yaw * sin_pitch, cos_pitch, cos_yaw * sin_pitch];
        let across = libm::tanf(number(self.spread[0]).clamp(-1.5, 1.5));
        let along = libm::tanf(number(self.spread[1]).clamp(-1.5, 1.5));
        let out = add(add(forward, scale(right, across)), scale(up, along));
        let long = length(out);
        out.map(|part| part / long)
    }
}

/// A weapon in a player's hands.
///
/// This is everything a weapon remembers between ticks. Like the player's
/// body it is small and `Copy`: it can be sent over the network, compared
/// with what a server says, and kept to replay from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weapon {
    /// Rounds ready to fire.
    pub magazine: u32,
    /// Spare rounds, moved into the magazine by reloading.
    pub reserve: u32,
    /// Seconds until the next shot can be fired.
    pub cooldown: f32,
    /// The reload under way, if any.
    pub reload: Option<Reload>,
    /// Shots still to come from the burst being fired.
    pub burst_left: u32,
    /// This pull of the trigger has done its work. A semi-automatic or
    /// burst weapon fires again, and an empty one clicks again, only after
    /// the trigger is let go.
    pub trigger_spent: bool,
    /// The trigger was held during the last tick.
    pub trigger_held: bool,
    /// Spread added by recent firing, in radians.
    pub added_spread: f32,
    /// How far recoil has thrown the view: up, then left, in radians. Add
    /// it to where the player looks when placing the camera, so the centre
    /// of the screen is where the next shot goes.
    pub kick: [f32; 2],
    /// Seconds since the last shot.
    pub since_shot: f32,
}

impl Weapon {
    /// A new weapon with a full magazine and its spare rounds, ready to fire.
    #[must_use]
    pub fn new(settings: &WeaponSettings) -> Self {
        Self {
            magazine: settings.magazine(),
            reserve: settings.reserve,
            cooldown: 0.0,
            reload: None,
            burst_left: 0,
            trigger_spent: false,
            trigger_held: false,
            added_spread: 0.0,
            kick: [0.0; 2],
            since_shot: 0.0,
        }
    }

    /// Brings the weapon up after switching to it: whatever it was doing
    /// when it was put away is forgotten, and it can fire after the
    /// settings' `raise_time`. Its ammunition is as it was left.
    pub fn raise(&mut self, settings: &WeaponSettings) {
        self.interrupt();
        self.cooldown = positive(settings.raise_time);
        self.added_spread = 0.0;
        self.kick = [0.0; 2];
        // A trigger held through the switch fires once the weapon is up.
        self.trigger_spent = false;
        self.trigger_held = false;
    }

    /// Stops a reload or a burst part-way, as sprinting, switching weapons
    /// or dying does. A reload stopped before its rounds arrived gives
    /// nothing.
    pub fn interrupt(&mut self) {
        self.reload = None;
        self.burst_left = 0;
    }

    /// How far through its reload the weapon is, from 0 to 1, or `None`
    /// when it is not reloading.
    #[must_use]
    pub fn reload_progress(&self, settings: &WeaponSettings) -> Option<f32> {
        let reload = self.reload?;
        let whole = settings.reload_duration(reload.from_empty);
        Some(if whole > 0.0 {
            (reload.elapsed / whole).clamp(0.0, 1.0)
        } else {
            1.0
        })
    }

    /// How far off the aim the next shot could land right now, in radians:
    /// the half-width of the cone, and the size to draw a crosshair.
    #[must_use]
    pub fn spread(&self, settings: &WeaponSettings, input: &WeaponInput) -> f32 {
        let resting = positive(if input.aiming {
            settings.aiming_spread
        } else {
            settings.spread
        })
        .to_radians();
        let mut cone = resting + positive(self.added_spread);
        cone *= blend(1.0, positive(settings.moving_spread), input.moving);
        if input.airborne {
            cone *= positive(settings.airborne_spread);
        } else if input.crouched {
            cone *= positive(settings.crouched_spread);
        }
        cone.min(1.0)
    }

    /// Runs the weapon for one tick of `dt` seconds.
    ///
    /// Everything fired during the tick is added to `shots`: one [`Shot`]
    /// per bullet, or per pellet of a shotgun blast. Clear `shots` and use
    /// it again each tick rather than making a new list.
    ///
    /// `seed` decides every roll made this tick; get it from
    /// [`shot_seed`](crate::shot_seed). Call this at a fixed rate with the
    /// same `dt` every time. The same weapon, settings, input, seed and
    /// `dt` always give the same result, so a client can go back to an old
    /// `Weapon` and replay its inputs. A `dt` that is zero, negative or not
    /// a number does nothing.
    pub fn step(
        &mut self,
        settings: &WeaponSettings,
        input: &WeaponInput,
        seed: u32,
        dt: f32,
        shots: &mut Vec<Shot>,
    ) -> WeaponEvents {
        let mut events = WeaponEvents::default();
        if !(dt.is_finite() && dt > 0.0) {
            return events;
        }
        self.repair();
        if !input.trigger {
            self.trigger_spent = false;
        }
        self.settle(settings, dt);
        self.continue_reload(settings, input, dt, &mut events);

        let wants_reload = input.reload && self.reload.is_none() && self.burst_left == 0;
        if wants_reload && self.can_reload(settings) {
            self.start_reload(&mut events);
        }

        let interval = settings.shot_interval();
        while events.shots < MOST_SHOTS_PER_TICK && self.reload.is_none() && self.cooldown < dt {
            let fresh_pull = input.trigger && !self.trigger_spent;
            if self.magazine == 0 {
                self.burst_left = 0;
                if fresh_pull {
                    events.dry_fire = true;
                    self.trigger_spent = true;
                }
                if input.trigger && settings.auto_reload && self.can_reload(settings) {
                    self.start_reload(&mut events);
                }
                break;
            }
            let wants = match settings.fire_mode {
                FireMode::Automatic => input.trigger,
                FireMode::SemiAutomatic => fresh_pull,
                FireMode::Burst(_) => self.burst_left > 0 || fresh_pull,
            };
            if !wants {
                break;
            }
            self.fire(settings, input, seed, events.shots, shots);
            events.shots += 1;
            self.cooldown += interval;
            match settings.fire_mode {
                FireMode::Automatic => {}
                FireMode::SemiAutomatic => self.trigger_spent = true,
                FireMode::Burst(size) => {
                    if self.burst_left == 0 {
                        self.burst_left = size.max(1);
                    }
                    self.burst_left -= 1;
                    self.trigger_spent = true;
                    if self.burst_left == 0 || self.magazine == 0 {
                        self.burst_left = 0;
                        self.cooldown += positive(settings.burst_pause);
                    }
                }
            }
        }
        // What is left over carries into the next tick, which is what keeps
        // the rate of fire exact; time spent not firing is never saved up.
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.trigger_held = input.trigger;
        events
    }

    /// Replaces numbers that are not numbers, which only a corrupted
    /// snapshot could hold, so they cannot spread.
    fn repair(&mut self) {
        self.cooldown = positive(self.cooldown);
        self.added_spread = positive(self.added_spread);
        self.since_shot = positive(self.since_shot);
        for angle in &mut self.kick {
            if !angle.is_finite() {
                *angle = 0.0;
            }
        }
    }

    /// Lets the spread shrink and the view settle back once the weapon has
    /// been quiet for long enough.
    fn settle(&mut self, settings: &WeaponSettings, dt: f32) {
        self.since_shot = (self.since_shot + dt).min(3600.0);
        if self.since_shot > positive(settings.spread_recovery_delay) {
            let less = positive(settings.spread_recovery).to_radians() * dt;
            self.added_spread = (self.added_spread - less).max(0.0);
        }
        if self.since_shot > positive(settings.kick_recovery_delay) {
            let [up, left] = self.kick;
            let far = (up * up + left * left).sqrt();
            let less = positive(settings.kick_recovery).to_radians() * dt;
            self.kick = if far > less {
                let keep = (far - less) / far;
                [up * keep, left * keep]
            } else {
                [0.0; 2]
            };
        }
    }

    /// Whether there is room in the magazine and something to put in it.
    fn can_reload(&self, settings: &WeaponSettings) -> bool {
        self.reserve > 0 && self.magazine < settings.magazine()
    }

    fn start_reload(&mut self, events: &mut WeaponEvents) {
        self.burst_left = 0;
        self.reload = Some(Reload {
            elapsed: 0.0,
            from_empty: self.magazine == 0,
            loaded: false,
        });
        events.reload_started = true;
    }

    /// Moves a reload along: the rounds arrive part-way through, and the
    /// weapon can fire again at the end.
    fn continue_reload(
        &mut self,
        settings: &WeaponSettings,
        input: &WeaponInput,
        dt: f32,
        events: &mut WeaponEvents,
    ) {
        let Some(mut reload) = self.reload else {
            return;
        };
        // A fresh pull stops loading shells to fire the ones already in.
        let pulled = input.trigger && !self.trigger_held;
        if settings.one_at_a_time && pulled && self.magazine > 0 {
            self.reload = None;
            return;
        }
        reload.elapsed = positive(reload.elapsed) + dt;
        let whole = settings.reload_duration(reload.from_empty);
        if !reload.loaded && reload.elapsed >= whole * blend(0.0, 1.0, settings.reload_ammo_at) {
            reload.loaded = true;
            let full = settings.magazine().saturating_add(u32::from(
                settings.keeps_chambered_round && !reload.from_empty && !settings.one_at_a_time,
            ));
            let wanted = if settings.one_at_a_time {
                1
            } else {
                full.saturating_sub(self.magazine)
            };
            let moved = wanted.min(self.reserve);
            self.magazine = self.magazine.saturating_add(moved);
            self.reserve -= moved;
        }
        self.reload = if reload.elapsed < whole {
            Some(reload)
        } else if settings.one_at_a_time && self.can_reload(settings) {
            // Straight on to the next shell.
            Some(Reload {
                elapsed: reload.elapsed - whole,
                from_empty: false,
                loaded: false,
            })
        } else {
            events.reload_finished = true;
            None
        };
    }

    /// Fires one shot: uses a round, adds its bullet or pellets to `shots`,
    /// then kicks the view and widens the spread for the shot after.
    fn fire(
        &mut self,
        settings: &WeaponSettings,
        input: &WeaponInput,
        seed: u32,
        shot: u32,
        shots: &mut Vec<Shot>,
    ) {
        self.magazine -= 1;
        let cone = self.spread(settings, input);
        for pellet in 0..settings.pellets.clamp(1, MOST_PELLETS) {
            let mut rolls = Rolls::new(seed, shot, pellet);
            // The square root spreads shots evenly over the circle instead
            // of bunching them in the middle.
            let off = cone * rolls.next().sqrt();
            let around = rolls.next() * std::f32::consts::TAU;
            shots.push(Shot {
                time: self.cooldown,
                kick: self.kick,
                spread: [off * libm::cosf(around), off * libm::sinf(around)],
            });
        }

        let most = positive(settings.most_kick).to_radians();
        let sideways = Rolls::new(seed, shot, KICK_ROLL).next() * 2.0 - 1.0;
        let up = self.kick[0] + positive(settings.kick_up).to_radians();
        let left = self.kick[1] + positive(settings.kick_sideways).to_radians() * sideways;
        self.kick = [up.min(most), left.clamp(-most, most)];

        let more = positive(settings.spread_per_shot).to_radians();
        let most = positive(settings.most_added_spread).to_radians();
        self.added_spread = (self.added_spread + more).min(most);
        self.since_shot = 0.0;
    }
}
