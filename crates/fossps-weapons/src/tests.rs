//! Weapons fired on a small range built by hand.

use fossps_physics::Level;

use crate::{
    Delivery, FireMode, Flight, Projectile, ProjectileSettings, Shot, Struck, Target, Weapon,
    WeaponEvents, WeaponInput, WeaponSettings, Zone, blast_damage, hitscan, shot_seed,
};

const DT: f32 = 1.0 / 60.0;
const SECOND: u32 = 60;

type Triangle = [[f32; 3]; 3];

/// A flat floor at height 0, 200 metres across.
fn floor() -> Vec<Triangle> {
    let s = 100.0;
    vec![
        [[-s, 0.0, s], [s, 0.0, s], [s, 0.0, -s]],
        [[-s, 0.0, s], [s, 0.0, -s], [-s, 0.0, -s]],
    ]
}

/// A thin wall facing the shooter, `z` metres ahead, 4 m wide and 3 m tall.
fn wall(z: f32) -> Vec<Triangle> {
    vec![
        [[-2.0, 0.0, z], [2.0, 0.0, z], [2.0, 3.0, z]],
        [[-2.0, 0.0, z], [2.0, 3.0, z], [-2.0, 3.0, z]],
    ]
}

fn level(parts: &[Vec<Triangle>]) -> Level {
    Level::from_triangles(&parts.concat())
}

fn nothing() -> Level {
    Level::from_triangles(&[])
}

/// A standing player `ahead` metres in front of the origin.
fn enemy(id: u32, ahead: f32) -> Target {
    Target {
        id,
        feet: [0.0, 0.0, -ahead],
        height: 1.8,
        radius: 0.35,
    }
}

fn held() -> WeaponInput {
    WeaponInput {
        trigger: true,
        ..WeaponInput::default()
    }
}

fn idle() -> WeaponInput {
    WeaponInput::default()
}

fn close(a: f32, b: f32, within: f32) -> bool {
    (a - b).abs() <= within
}

/// A weapon being run tick by tick, remembering every shot it fired.
struct Bench {
    settings: WeaponSettings,
    weapon: Weapon,
    tick: u32,
    shots: Vec<Shot>,
    /// The tick each shot was fired on.
    fired_on: Vec<u32>,
    scratch: Vec<Shot>,
}

impl Bench {
    fn new(settings: WeaponSettings) -> Self {
        Self {
            weapon: Weapon::new(&settings),
            settings,
            tick: 0,
            shots: Vec::new(),
            fired_on: Vec::new(),
            scratch: Vec::new(),
        }
    }

    fn step(&mut self, input: WeaponInput) -> WeaponEvents {
        self.scratch.clear();
        let seed = shot_seed(1, self.tick);
        let events = self
            .weapon
            .step(&self.settings, &input, seed, DT, &mut self.scratch);
        for shot in &self.scratch {
            assert!((0.0..DT).contains(&shot.time), "{shot:?}");
            self.shots.push(*shot);
            self.fired_on.push(self.tick);
        }
        self.tick += 1;
        events
    }

    /// Runs `ticks` ticks of the same input and adds up what happened.
    fn run(&mut self, input: WeaponInput, ticks: u32) -> WeaponEvents {
        let mut all = WeaponEvents::default();
        for _ in 0..ticks {
            let events = self.step(input);
            all.shots += events.shots;
            all.dry_fire |= events.dry_fire;
            all.reload_started |= events.reload_started;
            all.reload_finished |= events.reload_finished;
        }
        all
    }
}

fn endless(settings: WeaponSettings) -> WeaponSettings {
    WeaponSettings {
        magazine_size: 100_000,
        ..settings
    }
}

// ---- Rate of fire ----

#[test]
fn the_rate_of_fire_is_exact_whatever_the_tick_rate() {
    // 800 a minute is a shot every 4.5 ticks: no whole number of ticks.
    let mut bench = Bench::new(endless(WeaponSettings {
        rounds_per_minute: 800.0,
        ..Default::default()
    }));
    let fired = bench.run(held(), 60 * SECOND).shots;
    assert!((800..=801).contains(&fired), "{fired}");
    // The gaps alternate between 4 and 5 ticks rather than settling on one.
    let gaps: Vec<u32> = bench.fired_on.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|gap| *gap == 4 || *gap == 5), "{gaps:?}");
    assert!(gaps.contains(&4) && gaps.contains(&5));
}

#[test]
fn a_very_fast_weapon_fires_several_times_a_tick_but_never_a_whole_magazine() {
    let mut bench = Bench::new(endless(WeaponSettings {
        rounds_per_minute: 7200.0,
        ..Default::default()
    }));
    let fired = bench.run(held(), SECOND).shots;
    assert!((119..=121).contains(&fired), "{fired}");
    let times: Vec<f32> = bench.shots[0..2].iter().map(|shot| shot.time).collect();
    assert!(times[0] < times[1], "spaced out inside the tick: {times:?}");

    // One absurdly long tick does not empty the magazine.
    let mut weapon = Weapon::new(&bench.settings);
    let mut shots = Vec::new();
    let events = weapon.step(&bench.settings, &held(), 0, 30.0, &mut shots);
    assert_eq!(events.shots, 8);
}

#[test]
fn time_spent_not_firing_is_not_saved_up() {
    let mut bench = Bench::new(endless(WeaponSettings::default()));
    bench.run(idle(), 5 * SECOND);
    assert_eq!(bench.step(held()).shots, 1, "one shot, not a stored volley");
    assert_eq!(bench.step(held()).shots, 0);
}

#[test]
fn tapping_quickly_cannot_beat_the_rate_of_fire() {
    let mut bench = Bench::new(endless(WeaponSettings::pistol()));
    let mut fired = 0;
    for tick in 0..10 * SECOND {
        let input = if tick % 2 == 0 { held() } else { idle() };
        fired += bench.step(input).shots;
    }
    // 400 a minute for ten seconds.
    assert!(fired <= 67, "{fired}");
    assert!(fired >= 55, "{fired}");
}

// ---- Fire modes ----

#[test]
fn a_semi_automatic_fires_once_per_pull() {
    let mut bench = Bench::new(WeaponSettings::pistol());
    assert_eq!(bench.run(held(), 2 * SECOND).shots, 1);
    bench.step(idle());
    assert_eq!(bench.run(held(), 2 * SECOND).shots, 1);
    assert_eq!(bench.weapon.magazine, 10);
}

#[test]
fn a_pull_just_before_the_weapon_is_ready_still_fires() {
    let mut bench = Bench::new(WeaponSettings::pistol());
    bench.step(held());
    bench.step(idle());
    // Pulled again at once, long before the next shot is allowed, and held.
    let events = bench.run(held(), SECOND);
    assert_eq!(events.shots, 1);
    // 400 a minute is every 9 ticks.
    assert_eq!(bench.fired_on[0], 0);
    assert!(
        (9..=10).contains(&bench.fired_on[1]),
        "{:?}",
        bench.fired_on
    );
}

#[test]
fn a_burst_finishes_after_the_trigger_is_let_go_and_needs_a_new_pull() {
    let mut bench = Bench::new(WeaponSettings {
        fire_mode: FireMode::Burst(3),
        rounds_per_minute: 900.0,
        ..Default::default()
    });
    assert_eq!(bench.step(held()).shots, 1);
    assert_eq!(bench.run(idle(), SECOND).shots, 2, "the burst completes");
    assert_eq!(bench.weapon.burst_left, 0);

    assert_eq!(
        bench.run(held(), 2 * SECOND).shots,
        3,
        "held: one burst only"
    );
    bench.step(idle());
    assert_eq!(bench.run(held(), SECOND).shots, 3);
    assert_eq!(bench.weapon.magazine, 21);
}

#[test]
fn bursts_are_separated_by_the_pause() {
    let settings = WeaponSettings {
        fire_mode: FireMode::Burst(2),
        rounds_per_minute: 600.0,
        burst_pause: 0.5,
        ..Default::default()
    };
    let mut bench = Bench::new(settings);
    for tick in 0..2 * SECOND {
        // A new pull every other tick.
        bench.step(if tick % 2 == 0 { held() } else { idle() });
    }
    // Shots 6 ticks apart inside a burst; 0.6 s (36 ticks) between bursts.
    let gaps: Vec<u32> = bench.fired_on.windows(2).map(|w| w[1] - w[0]).collect();
    assert!((6..=7).contains(&gaps[0]), "{gaps:?}");
    assert!((36..=38).contains(&gaps[1]), "{gaps:?}");
    assert!((6..=7).contains(&gaps[2]), "{gaps:?}");
}

#[test]
fn a_burst_stops_when_the_magazine_runs_dry() {
    let mut bench = Bench::new(WeaponSettings {
        fire_mode: FireMode::Burst(3),
        reserve: 0,
        ..Default::default()
    });
    bench.weapon.magazine = 2;
    assert_eq!(bench.run(held(), SECOND).shots, 2);
    assert_eq!(bench.weapon.burst_left, 0);
    bench.weapon.magazine = 5;
    assert_eq!(bench.run(idle(), SECOND).shots, 0, "the old burst is over");
}

// ---- Ammunition and reloading ----

#[test]
fn an_empty_magazine_clicks_once_and_reloads_by_itself() {
    let mut bench = Bench::new(WeaponSettings::default());
    bench.weapon.magazine = 1;
    let first = bench.step(held());
    assert_eq!(first.shots, 1);
    assert!(!first.dry_fire);

    let mut clicks = 0;
    let mut started = 0;
    let mut finished_at = None;
    for tick in 0..4 * SECOND {
        let events = bench.step(held());
        clicks += u32::from(events.dry_fire);
        started += u32::from(events.reload_started);
        if events.reload_finished && finished_at.is_none() {
            finished_at = Some(tick);
        }
    }
    assert_eq!(clicks, 1);
    assert_eq!(started, 1);
    // An empty reload takes 2.6 s, after the gap that followed the last shot.
    let finished_at = finished_at.unwrap();
    assert!((158..=164).contains(&finished_at), "{finished_at}");
    assert_eq!(bench.weapon.reserve, 90);
    assert!(
        bench.weapon.magazine < 30,
        "and it went straight back to firing"
    );
}

#[test]
fn with_nothing_left_the_trigger_only_clicks() {
    let mut bench = Bench::new(WeaponSettings {
        reserve: 0,
        ..Default::default()
    });
    bench.weapon.magazine = 0;
    let events = bench.run(held(), SECOND);
    assert_eq!(events.shots, 0);
    assert!(events.dry_fire && !events.reload_started);
    bench.step(idle());
    assert!(bench.step(held()).dry_fire, "a new pull clicks again");

    // Without auto reload the player has to ask.
    let mut bench = Bench::new(WeaponSettings {
        auto_reload: false,
        ..Default::default()
    });
    bench.weapon.magazine = 0;
    assert!(!bench.run(held(), SECOND).reload_started);
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    assert!(bench.step(asked).reload_started);
}

#[test]
fn rounds_arrive_part_way_through_a_reload_and_firing_waits_for_the_end() {
    let mut bench = Bench::new(WeaponSettings::default());
    bench.weapon.magazine = 10;
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    assert!(bench.step(asked).reload_started);
    assert!(bench.weapon.reload.is_some_and(|reload| !reload.from_empty));

    // 2 s reload, rounds in at 75%: 1.5 s.
    bench.run(held(), 88);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (10, 120));
    let progress = bench.weapon.reload_progress(&bench.settings).unwrap();
    assert!(close(progress, 0.74, 0.01), "{progress}");
    bench.run(held(), 3);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (30, 100));
    assert!(bench.shots.is_empty(), "no firing during the reload");

    let mut events = WeaponEvents::default();
    while !events.reload_finished {
        events = bench.step(idle());
    }
    assert!((120..=122).contains(&bench.tick), "{}", bench.tick);
    assert!(bench.weapon.reload_progress(&bench.settings).is_none());
    assert_eq!(bench.step(held()).shots, 1);
}

#[test]
fn a_reload_cut_short_before_the_rounds_arrive_gives_nothing() {
    let mut bench = Bench::new(WeaponSettings::default());
    bench.weapon.magazine = 10;
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    bench.run(asked, SECOND);
    bench.weapon.interrupt();
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (10, 120));
    assert!(bench.weapon.reload.is_none());

    // Cut short after they arrive, the rounds are kept.
    bench.run(asked, 100);
    bench.weapon.interrupt();
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (30, 100));
}

#[test]
fn a_reload_is_refused_when_full_or_with_nothing_spare() {
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    let mut bench = Bench::new(WeaponSettings::default());
    assert!(!bench.step(asked).reload_started, "already full");

    bench.weapon.magazine = 3;
    bench.weapon.reserve = 0;
    assert!(!bench.step(asked).reload_started, "nothing to load");

    // A short reserve loads what there is.
    bench.weapon.reserve = 5;
    assert!(bench.run(asked, 3 * SECOND).reload_finished);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (8, 0));
}

#[test]
fn a_chambered_round_is_kept_only_when_the_magazine_was_not_empty() {
    let settings = WeaponSettings {
        keeps_chambered_round: true,
        ..Default::default()
    };
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    let mut bench = Bench::new(settings);
    bench.weapon.magazine = 4;
    bench.run(asked, 3 * SECOND);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (31, 93));
    assert!(!bench.step(asked).reload_started, "31 is full");

    bench.weapon.magazine = 0;
    bench.run(asked, 3 * SECOND);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (30, 63));
}

#[test]
fn shells_go_in_one_at_a_time_and_a_pull_stops_loading_to_fire() {
    let mut bench = Bench::new(WeaponSettings::shotgun());
    bench.weapon.magazine = 0;
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    bench.step(asked);
    // Half a second a shell; the third goes in 1.375 s after starting.
    bench.run(idle(), 80);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (2, 22));
    bench.run(idle(), 5);
    assert_eq!(bench.weapon.magazine, 3);

    let events = bench.step(held());
    assert_eq!(events.shots, 1, "loading stopped and it fired");
    assert!(bench.weapon.reload.is_none());
    assert_eq!(bench.weapon.magazine, 2);
    assert_eq!(bench.shots.len(), 8, "eight pellets");

    // Left alone, it fills right up and then stops.
    bench.step(idle());
    bench.step(asked);
    assert!(bench.run(idle(), 3 * SECOND).reload_finished);
    assert_eq!((bench.weapon.magazine, bench.weapon.reserve), (6, 17));
}

#[test]
fn holding_the_trigger_through_a_shell_reload_does_not_cut_it_short() {
    let mut bench = Bench::new(WeaponSettings::shotgun());
    bench.weapon.magazine = 1;
    // Fires the last shell, clicks, and starts loading, trigger held throughout.
    bench.run(held(), 4 * SECOND);
    assert_eq!(bench.shots.len(), 8, "only the first blast");
    assert_eq!(bench.weapon.magazine, 6, "filled up while held");
}

#[test]
fn raising_a_weapon_makes_it_wait_and_keeps_its_ammunition() {
    let mut bench = Bench::new(WeaponSettings::pistol());
    bench.weapon.magazine = 5;
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    bench.step(asked);
    bench.weapon.raise(&bench.settings);
    assert!(bench.weapon.reload.is_none());
    assert_eq!(bench.weapon.magazine, 5);

    // A quarter of a second to come up, even with the trigger already held.
    let before = bench.tick;
    assert_eq!(bench.run(held(), SECOND).shots, 1);
    let waited = bench.fired_on[0] - before;
    assert!((15..=16).contains(&waited), "{waited}");
}

// ---- Spread and recoil ----

#[test]
fn firing_widens_the_spread_up_to_a_limit_and_resting_narrows_it() {
    let mut bench = Bench::new(endless(WeaponSettings::default()));
    let resting = bench.weapon.spread(&bench.settings, &idle());
    assert!(close(resting.to_degrees(), 1.0, 1.0e-4));

    bench.run(held(), SECOND);
    let after_a_second = bench.weapon.spread(&bench.settings, &idle());
    assert!(after_a_second > resting);
    bench.run(held(), 4 * SECOND);
    let most = bench.weapon.spread(&bench.settings, &idle());
    assert!(close(most.to_degrees(), 3.0, 1.0e-3), "1 + at most 2");

    bench.run(idle(), SECOND);
    let rested = bench.weapon.spread(&bench.settings, &idle());
    assert!(close(rested, resting, 1.0e-6));
}

#[test]
fn stance_and_movement_change_the_spread() {
    let settings = WeaponSettings::default();
    let weapon = Weapon::new(&settings);
    let degrees = |input: WeaponInput| weapon.spread(&settings, &input).to_degrees();
    let with = |change: fn(&mut WeaponInput)| {
        let mut input = idle();
        change(&mut input);
        degrees(input)
    };
    assert!(close(with(|input| input.aiming = true), 0.1, 1.0e-4));
    assert!(close(with(|input| input.moving = 1.0), 2.0, 1.0e-4));
    assert!(close(with(|input| input.moving = 0.5), 1.5, 1.0e-4));
    assert!(close(with(|input| input.moving = 9.0), 2.0, 1.0e-4));
    assert!(close(with(|input| input.moving = f32::NAN), 1.0, 1.0e-4));
    assert!(close(with(|input| input.crouched = true), 0.7, 1.0e-4));
    assert!(close(with(|input| input.airborne = true), 4.0, 1.0e-4));
}

#[test]
fn every_shot_lands_inside_the_cone_and_they_fill_it() {
    let mut bench = Bench::new(endless(WeaponSettings {
        spread: 2.0,
        spread_per_shot: 0.0,
        kick_up: 0.0,
        kick_sideways: 0.0,
        ..Default::default()
    }));
    bench.run(held(), 30 * SECOND);
    let cone = 2.0_f32.to_radians();
    let mut furthest = 0.0_f32;
    let mut quarters = [0_u32; 4];
    for shot in &bench.shots {
        let [right, up] = shot.spread;
        let off = right.hypot(up);
        assert!(off <= cone * 1.0001, "{shot:?}");
        furthest = furthest.max(off);
        quarters[usize::from(right > 0.0) + 2 * usize::from(up > 0.0)] += 1;
    }
    assert!(furthest > cone * 0.95, "{furthest}");
    let count = bench.shots.len() as u32;
    assert!(quarters.iter().all(|n| *n > count / 5), "{quarters:?}");
}

#[test]
fn a_perfectly_accurate_weapon_shoots_dead_straight() {
    let mut bench = Bench::new(WeaponSettings {
        spread: 0.0,
        kick_up: 0.0,
        kick_sideways: 0.0,
        ..Default::default()
    });
    bench.step(held());
    let ahead = bench.shots[0].direction(0.0, 0.0);
    assert!(close(ahead[0], 0.0, 1.0e-6) && close(ahead[1], 0.0, 1.0e-6));
    assert!(close(ahead[2], -1.0, 1.0e-6));
}

#[test]
fn recoil_throws_the_view_up_stops_at_its_limit_and_settles_back() {
    let mut bench = Bench::new(endless(WeaponSettings {
        spread: 0.0,
        spread_per_shot: 0.0,
        ..Default::default()
    }));
    bench.step(held());
    assert_eq!(
        bench.shots[0].kick, [0.0; 2],
        "the first shot goes where aimed"
    );
    assert!(close(bench.weapon.kick[0].to_degrees(), 0.45, 1.0e-4));
    assert!(bench.weapon.kick[1].abs().to_degrees() <= 0.15);

    bench.run(held(), 5 * SECOND);
    assert!(close(bench.weapon.kick[0].to_degrees(), 8.0, 1.0e-3));
    let last = bench.shots[bench.shots.len() - 1];
    assert!(
        last.kick[0] > 0.1,
        "later shots leave with the view thrown up"
    );
    assert!(last.direction(0.0, 0.0)[1] > 0.1, "and so go high");

    bench.run(idle(), 2 * SECOND);
    assert_eq!(bench.weapon.kick, [0.0; 2]);
}

#[test]
fn directions_follow_yaw_pitch_and_the_spread() {
    let shot = |spread: [f32; 2]| Shot {
        time: 0.0,
        kick: [0.0; 2],
        spread,
    };
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| close(*a, b, 1.0e-5));
    let quarter = std::f32::consts::FRAC_PI_2;
    let straight = shot([0.0; 2]);
    assert!(near(straight.direction(0.0, 0.0), [0.0, 0.0, -1.0]));
    assert!(
        near(straight.direction(quarter, 0.0), [-1.0, 0.0, 0.0]),
        "left"
    );
    let up = straight.direction(0.0, 0.5);
    assert!(up[1] > 0.4 && up[2] < 0.0);

    let tenth = 0.1_f32;
    let right = shot([tenth, 0.0]).direction(0.0, 0.0);
    assert!(near(right, [tenth.sin(), 0.0, -tenth.cos()]));
    let high = shot([0.0, tenth]).direction(0.0, 0.0);
    assert!(near(high, [0.0, tenth.sin(), -tenth.cos()]));
    // Turned to face left, "right of the aim" is towards -Z.
    assert!(shot([tenth, 0.0]).direction(quarter, 0.0)[2] < -0.05);

    // Recoil moves the view the shot leaves along: up, then left.
    let kicked = Shot {
        time: 0.0,
        kick: [0.2, 0.3],
        spread: [0.0; 2],
    };
    assert!(near(
        kicked.direction(0.0, 0.0),
        straight.direction(0.3, 0.2)
    ));

    // Always one unit long, even looking straight up with nonsense mixed in.
    let odd = Shot {
        time: 0.0,
        kick: [9.0, f32::NAN],
        spread: [f32::INFINITY, 0.3],
    };
    let direction = odd.direction(f32::NAN, 2.0);
    let long = direction.iter().map(|part| part * part).sum::<f32>().sqrt();
    assert!(close(long, 1.0, 1.0e-5), "{direction:?}");
}

#[test]
fn pellets_share_a_shot_but_each_goes_its_own_way() {
    let mut bench = Bench::new(WeaponSettings::shotgun());
    let events = bench.step(held());
    assert_eq!(events.shots, 1);
    assert_eq!(bench.shots.len(), 8);
    assert_eq!(bench.weapon.magazine, 5, "one shell, not eight");
    for (index, pellet) in bench.shots.iter().enumerate() {
        assert_eq!(pellet.time, bench.shots[0].time);
        for other in &bench.shots[index + 1..] {
            assert_ne!(pellet.spread, other.spread);
        }
    }
}

// ---- Agreement between machines ----

/// A varied few seconds: bursts of fire, reloads and stance changes.
fn script(tick: u32) -> WeaponInput {
    let beat = tick / 7;
    WeaponInput {
        trigger: !beat.is_multiple_of(5),
        reload: tick.is_multiple_of(190),
        aiming: beat.is_multiple_of(3),
        moving: (beat % 4) as f32 / 3.0,
        airborne: beat.is_multiple_of(11),
        crouched: beat.is_multiple_of(6),
    }
}

#[test]
fn the_same_inputs_give_the_same_bullets_bit_for_bit() {
    for settings in [
        WeaponSettings::default(),
        WeaponSettings::shotgun(),
        WeaponSettings {
            fire_mode: FireMode::Burst(3),
            ..Default::default()
        },
    ] {
        let play = || {
            let mut bench = Bench::new(settings);
            for tick in 0..20 * SECOND {
                bench.step(script(tick));
            }
            bench
        };
        let (one, two) = (play(), play());
        assert!(one.shots.len() > 30, "{}", one.shots.len());
        assert_eq!(one.shots, two.shots);
        assert_eq!(one.weapon, two.weapon);
    }
}

#[test]
fn replaying_from_a_kept_weapon_reaches_the_same_place() {
    let mut bench = Bench::new(WeaponSettings::default());
    for tick in 0..300 {
        bench.step(script(tick));
    }
    let kept = bench.weapon;
    let shots_before = bench.shots.len();
    for tick in 300..600 {
        bench.step(script(tick));
    }

    // A client going back to tick 300 and running the same inputs again.
    let mut replay = Bench::new(WeaponSettings::default());
    replay.weapon = kept;
    replay.tick = 300;
    for tick in 300..600 {
        replay.step(script(tick));
    }
    assert_eq!(replay.weapon, bench.weapon);
    assert_eq!(replay.shots, bench.shots[shots_before..]);
}

#[test]
fn different_players_and_ticks_roll_differently() {
    let settings = WeaponSettings::default();
    let first_shot = |seed: u32| {
        let mut weapon = Weapon::new(&settings);
        let mut shots = Vec::new();
        weapon.step(&settings, &held(), seed, DT, &mut shots);
        shots[0].spread
    };
    assert_eq!(first_shot(shot_seed(1, 5)), first_shot(shot_seed(1, 5)));
    assert_ne!(first_shot(shot_seed(1, 5)), first_shot(shot_seed(1, 6)));
    assert_ne!(first_shot(shot_seed(1, 5)), first_shot(shot_seed(2, 5)));
    assert_ne!(shot_seed(0, 1), shot_seed(1, 0));
}

#[test]
fn nonsense_numbers_do_not_spread() {
    let settings = WeaponSettings {
        rounds_per_minute: f32::NAN,
        spread: f32::NAN,
        spread_per_shot: f32::INFINITY,
        kick_up: f32::NAN,
        kick_sideways: -3.0,
        most_kick: f32::NAN,
        reload_time: f32::NAN,
        empty_reload_time: -1.0,
        reload_ammo_at: f32::NAN,
        magazine_size: 0,
        pellets: 0,
        burst_pause: f32::NAN,
        ..Default::default()
    };
    let mut weapon = Weapon::new(&settings);
    assert_eq!(weapon.magazine, 1, "a magazine holds at least one round");
    let mut shots = Vec::new();
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let before = weapon;
        assert_eq!(weapon.step(&settings, &held(), 0, bad, &mut shots).shots, 0);
        assert_eq!(weapon, before);
    }

    weapon.cooldown = f32::NAN;
    weapon.kick = [f32::NAN, f32::INFINITY];
    weapon.added_spread = f32::NAN;
    weapon.since_shot = f32::NAN;
    let moving = WeaponInput {
        moving: f32::NAN,
        ..held()
    };
    for tick in 0..10 * SECOND {
        weapon.step(&settings, &moving, tick, DT, &mut shots);
    }
    assert!(shots.len() > 3, "it still fires, reloading as it goes");
    assert!(weapon.cooldown.is_finite() && weapon.added_spread.is_finite());
    assert!(weapon.kick.iter().all(|angle| angle.is_finite()));
    for shot in &shots {
        let direction = shot.direction(0.3, 0.1);
        assert!(direction.iter().all(|part| part.is_finite()), "{shot:?}");
    }
}

// ---- What a bullet hits ----

const EYES: [f32; 3] = [0.0, 1.0, 0.0];
const AHEAD: [f32; 3] = [0.0, 0.0, -1.0];

#[test]
fn a_bullet_hits_the_nearest_thing_in_its_way() {
    let rifle = WeaponSettings::default();
    let range = level(&[floor(), wall(-30.0)]);
    let targets = [enemy(2, 20.0), enemy(3, 10.0), enemy(4, 40.0)];

    let hit = hitscan(&range, &targets, EYES, AHEAD, &rifle, None).unwrap();
    assert_eq!(hit.target(), Some(3));
    assert!(close(hit.distance, 10.0 - 0.35, 1.0e-4), "{hit:?}");
    assert!(close(hit.point[2], -9.65, 1.0e-4));
    assert!(close(hit.normal[2], 1.0, 1.0e-4), "faces the shooter");

    // With nobody in front, the wall; the player behind it is safe.
    let hit = hitscan(&range, &targets[2..], EYES, AHEAD, &rifle, None).unwrap();
    assert_eq!(hit.struck, Struck::Level);
    assert_eq!(hit.target(), None);
    assert_eq!(hit.damage, 0.0);
    assert!(close(hit.distance, 30.0, 1.0e-4));

    // Into the sky, nothing.
    assert!(hitscan(&range, &targets, EYES, [0.0, 1.0, 0.0], &rifle, None).is_none());
}

#[test]
fn the_shooter_is_not_hit_by_their_own_bullet() {
    let rifle = WeaponSettings::default();
    let me = Target {
        id: 1,
        feet: [0.0, 0.0, 0.0],
        height: 1.8,
        radius: 0.35,
    };
    let targets = [me, enemy(2, 10.0)];
    let hit = hitscan(&nothing(), &targets, EYES, AHEAD, &rifle, Some(1)).unwrap();
    assert_eq!(hit.target(), Some(2));
    // Not told who is shooting, the bullet starts inside them.
    let hit = hitscan(&nothing(), &targets, EYES, AHEAD, &rifle, None).unwrap();
    assert_eq!(hit.target(), Some(1));
    assert_eq!(hit.distance, 0.0);
}

#[test]
fn head_body_and_legs_are_told_apart_by_height() {
    let rifle = WeaponSettings::default();
    let targets = [enemy(2, 10.0)];
    let at_height = |y: f32| {
        hitscan(&nothing(), &targets, [0.0, y, 0.0], AHEAD, &rifle, None).map(|hit| {
            let Struck::Target { zone, .. } = hit.struck else {
                panic!("{hit:?}");
            };
            (zone, hit.damage)
        })
    };
    assert_eq!(at_height(1.7), Some((Zone::Head, 60.0)));
    assert_eq!(at_height(1.2), Some((Zone::Body, 30.0)));
    assert_eq!(at_height(0.3), Some((Zone::Legs, 24.0)));
    assert_eq!(at_height(1.85), None, "over the head");
    assert_eq!(at_height(-0.1), None, "under the feet");

    // A crouched player's head is lower.
    let crouched = [Target {
        height: 1.1,
        ..enemy(2, 10.0)
    }];
    let hit = hitscan(&nothing(), &crouched, [0.0, 1.0, 0.0], AHEAD, &rifle, None).unwrap();
    assert!(matches!(
        hit.struck,
        Struck::Target {
            zone: Zone::Head,
            ..
        }
    ));
}

#[test]
fn the_round_top_and_the_sides_of_a_body_are_where_they_should_be() {
    let rifle = WeaponSettings::default();
    let targets = [enemy(2, 10.0)];
    let from = |x: f32, y: f32| hitscan(&nothing(), &targets, [x, y, 0.0], AHEAD, &rifle, None);
    assert!(from(0.34, 1.0).is_some());
    assert!(from(0.36, 1.0).is_none());
    // The top is a dome: the middle reaches 1.8 m, the shoulders do not.
    assert!(from(0.0, 1.79).is_some());
    assert!(from(0.3, 1.79).is_none());

    // Straight down onto the head.
    let above = [0.0, 5.0, -10.0];
    let hit = hitscan(&nothing(), &targets, above, [0.0, -1.0, 0.0], &rifle, None).unwrap();
    assert!(close(hit.distance, 3.2, 1.0e-4), "{hit:?}");
    assert!(close(hit.normal[1], 1.0, 1.0e-4));
    // A target behind the shooter is not hit.
    assert!(hitscan(&nothing(), &targets, EYES, [0.0, 0.0, 1.0], &rifle, None).is_none());
}

#[test]
fn damage_falls_off_between_two_distances() {
    let rifle = WeaponSettings::default();
    assert_eq!(rifle.damage_at(0.0, Zone::Body), 30.0);
    assert_eq!(rifle.damage_at(25.0, Zone::Body), 30.0);
    assert!(close(rifle.damage_at(42.5, Zone::Body), 25.5, 1.0e-4));
    assert_eq!(rifle.damage_at(60.0, Zone::Body), 21.0);
    assert_eq!(rifle.damage_at(500.0, Zone::Body), 21.0);
    assert_eq!(rifle.damage_at(f32::NAN, Zone::Body), 30.0);

    let hit = hitscan(&nothing(), &[enemy(2, 60.35)], EYES, AHEAD, &rifle, None).unwrap();
    assert!(close(hit.damage, 21.0, 1.0e-3));

    // Start and end at the same place: a plain step.
    let step = WeaponSettings {
        falloff_start: 10.0,
        falloff_end: 10.0,
        ..rifle
    };
    assert_eq!(step.damage_at(10.0, Zone::Body), 30.0);
    assert_eq!(step.damage_at(10.1, Zone::Body), 21.0);
}

#[test]
fn a_bullet_does_not_carry_past_its_reach() {
    let short = WeaponSettings {
        delivery: Delivery::Hitscan { reach: 9.0 },
        ..Default::default()
    };
    assert!(hitscan(&nothing(), &[enemy(2, 10.0)], EYES, AHEAD, &short, None).is_none());
    assert!(hitscan(&nothing(), &[enemy(2, 9.0)], EYES, AHEAD, &short, None).is_some());
    let none = WeaponSettings {
        delivery: Delivery::Hitscan { reach: f32::NAN },
        ..Default::default()
    };
    assert!(hitscan(&nothing(), &[enemy(2, 5.0)], EYES, AHEAD, &none, None).is_none());
    let rifle = WeaponSettings::default();
    assert!(hitscan(&nothing(), &[enemy(2, 5.0)], EYES, [0.0; 3], &rifle, None).is_none());
}

#[test]
fn a_shotgun_blast_up_close_lands_most_of_its_pellets() {
    let shotgun = WeaponSettings::shotgun();
    let mut weapon = Weapon::new(&shotgun);
    let mut shots = Vec::new();
    weapon.step(&shotgun, &held(), shot_seed(1, 0), DT, &mut shots);
    let targets = [enemy(2, 5.0)];
    let damage: f32 = shots
        .iter()
        .filter_map(|shot| {
            let direction = shot.direction(0.0, 0.0);
            hitscan(
                &nothing(),
                &targets,
                [0.0, 1.2, 0.0],
                direction,
                &shotgun,
                Some(1),
            )
        })
        .map(|hit| hit.damage)
        .sum();
    assert!(damage >= 6.0 * 14.0, "{damage}");
}

// ---- Rockets and grenades ----

fn fly(
    projectile: &mut Projectile,
    level: &Level,
    targets: &[Target],
    settings: &WeaponSettings,
    most_ticks: u32,
) -> (Flight, u32) {
    for tick in 1..=most_ticks {
        let flight = projectile.step(level, targets, settings, DT);
        if flight.is_over() {
            return (flight, tick);
        }
    }
    (Flight::Flying, most_ticks)
}

#[test]
fn a_rocket_flies_straight_and_explodes_on_the_wall() {
    let launcher = WeaponSettings::rocket_launcher();
    let range = level(&[floor(), wall(-20.0)]);
    let mut rocket = Projectile::launch(&launcher, EYES, AHEAD, Some(1)).unwrap();
    let (flight, ticks) = fly(&mut rocket, &range, &[], &launcher, 10 * SECOND);
    let Flight::Exploded { point, touched } = flight else {
        panic!("{flight:?}");
    };
    assert_eq!(touched, None);
    // Just off the wall, at the height it was fired from.
    assert!(close(point[2], -19.99, 1.0e-3), "{point:?}");
    assert!(close(point[1], 1.0, 1.0e-4));
    // 20 m at 40 m/s is half a second.
    assert!((30..=31).contains(&ticks), "{ticks}");
    assert!(close(rocket.travelled, 20.0, 1.0e-3));
}

#[test]
fn a_rocket_goes_off_against_a_player_but_not_its_owner() {
    let launcher = WeaponSettings::rocket_launcher();
    let me = Target {
        id: 1,
        feet: [0.0, 0.0, 0.0],
        height: 1.8,
        radius: 0.35,
    };
    let targets = [me, enemy(2, 12.0)];
    let mut rocket = Projectile::launch(&launcher, EYES, AHEAD, Some(1)).unwrap();
    let (flight, _) = fly(&mut rocket, &nothing(), &targets, &launcher, 10 * SECOND);
    assert!(
        matches!(
            flight,
            Flight::Exploded {
                touched: Some(2),
                ..
            }
        ),
        "{flight:?}"
    );

    // A rocket nobody owns, launched inside a player, goes off at once.
    let mut loose = Projectile::launch(&launcher, EYES, AHEAD, None).unwrap();
    let flight = loose.step(&nothing(), &targets, &launcher, DT);
    assert!(
        matches!(
            flight,
            Flight::Exploded {
                touched: Some(1),
                ..
            }
        ),
        "{flight:?}"
    );
}

#[test]
fn a_rocket_that_meets_nothing_runs_out_of_fuse() {
    let launcher = WeaponSettings::rocket_launcher();
    let mut rocket = Projectile::launch(&launcher, EYES, [0.0, 1.0, 0.0], None).unwrap();
    let (flight, ticks) = fly(&mut rocket, &nothing(), &[], &launcher, 20 * SECOND);
    assert!(matches!(flight, Flight::Exploded { touched: None, .. }));
    assert!((479..=481).contains(&ticks), "eight seconds: {ticks}");
}

#[test]
fn a_fast_projectile_does_not_pass_through_a_thin_wall_or_a_player() {
    // A sniper's bullet: 900 m/s is 15 m a tick.
    let sniper = WeaponSettings {
        delivery: Delivery::Projectile(ProjectileSettings {
            speed: 900.0,
            gravity: 10.0,
            drag: 0.0,
            bounciness: 0.0,
            fuse: 3.0,
            explodes_on_touch: false,
            blast_radius: 0.0,
            blast_edge_damage: 0.0,
        }),
        damage: 90.0,
        far_damage: 60.0,
        falloff_start: 50.0,
        falloff_end: 150.0,
        ..Default::default()
    };
    let range = level(&[wall(-37.0)]);
    let mut bullet = Projectile::launch(&sniper, EYES, AHEAD, Some(1)).unwrap();
    let (flight, ticks) = fly(&mut bullet, &range, &[], &sniper, SECOND);
    let Flight::Struck(hit) = flight else {
        panic!("{flight:?}");
    };
    assert_eq!(hit.struck, Struck::Level);
    assert_eq!(ticks, 3);

    let mut bullet = Projectile::launch(&sniper, EYES, AHEAD, Some(1)).unwrap();
    let (flight, _) = fly(&mut bullet, &nothing(), &[enemy(2, 100.0)], &sniper, SECOND);
    let Flight::Struck(hit) = flight else {
        panic!("{flight:?}");
    };
    assert_eq!(hit.target(), Some(2));
    // Half-way down the falloff, and it dropped a little on the way.
    assert!(close(hit.distance, 99.65, 0.05), "{hit:?}");
    assert!(close(hit.damage, 75.0, 0.5), "{hit:?}");
    assert!(hit.point[1] < 1.0 && hit.point[1] > 0.8, "{hit:?}");

    // With nothing in the way it just disappears when its time is up.
    let mut bullet = Projectile::launch(&sniper, EYES, AHEAD, Some(1)).unwrap();
    let (flight, _) = fly(&mut bullet, &nothing(), &[], &sniper, 10 * SECOND);
    assert_eq!(flight, Flight::Expired);
}

#[test]
fn a_grenade_arcs_bounces_comes_to_rest_and_goes_off_on_time() {
    let grenade = WeaponSettings::grenade();
    let range = level(&[floor()]);
    // Thrown level from head height.
    let mut thrown = Projectile::launch(&grenade, [0.0, 1.6, 0.0], AHEAD, Some(1)).unwrap();
    let mut bounces = Vec::new();
    let mut highest_after_bounce = 0.0_f32;
    let mut ended = None;
    for tick in 1..=5 * SECOND {
        match thrown.step(&range, &[], &grenade, DT) {
            Flight::Flying => {}
            Flight::Bounced { point, normal } => {
                assert!(close(point[1], 0.0, 1.0e-4) && close(normal[1], 1.0, 1.0e-4));
                bounces.push(tick);
            }
            over => {
                ended = Some((over, tick));
                break;
            }
        }
        if !bounces.is_empty() {
            highest_after_bounce = highest_after_bounce.max(thrown.position[1]);
        }
        assert!(
            thrown.position[1] > 0.0,
            "fell through the floor: {thrown:?}"
        );
    }
    // It falls 1.6 m in 0.4 s, having gone about 7 m.
    assert!((23..=26).contains(&bounces[0]), "{bounces:?}");
    assert!(bounces.len() >= 2, "{bounces:?}");
    // The bounce is lower than the throw, and it ends up lying still.
    assert!(highest_after_bounce > 0.1 && highest_after_bounce < 0.6);
    assert_eq!(thrown.velocity, [0.0; 3]);

    let (over, tick) = ended.unwrap();
    assert!((180..=181).contains(&tick), "a three second fuse: {tick}");
    let Flight::Exploded { point, touched } = over else {
        panic!("{over:?}");
    };
    assert_eq!(touched, None);
    assert!(point[2] < -7.0 && point[2] > -20.0, "{point:?}");
    assert!(close(point[1], 0.01, 1.0e-3), "on the floor: {point:?}");
}

#[test]
fn a_grenade_bounces_off_a_wall_back_towards_the_thrower() {
    let grenade = WeaponSettings::grenade();
    let range = level(&[floor(), wall(-3.0)]);
    let mut thrown = Projectile::launch(&grenade, [0.0, 1.6, 0.0], AHEAD, Some(1)).unwrap();
    let mut first = None;
    for _ in 0..SECOND {
        if let Flight::Bounced { normal, .. } = thrown.step(&range, &[], &grenade, DT) {
            first.get_or_insert(normal);
        }
    }
    assert!(close(first.unwrap()[2], 1.0, 1.0e-4));
    assert!(thrown.position[2] > -3.0, "{thrown:?}");
    assert!(thrown.velocity[2] >= 0.0, "{thrown:?}");
}

#[test]
fn drag_slows_a_projectile_down() {
    let with_drag = |drag: f32| {
        let settings = WeaponSettings {
            delivery: Delivery::Projectile(ProjectileSettings {
                drag,
                ..ProjectileSettings::default()
            }),
            ..WeaponSettings::rocket_launcher()
        };
        let mut rocket = Projectile::launch(&settings, EYES, AHEAD, None).unwrap();
        for _ in 0..SECOND {
            rocket.step(&nothing(), &[], &settings, DT);
        }
        rocket
    };
    assert!(close(with_drag(0.0).travelled, 40.0, 1.0e-2));
    let slowed = with_drag(1.0);
    assert!(
        slowed.travelled < 30.0 && slowed.travelled > 20.0,
        "{slowed:?}"
    );
    assert!(close(-slowed.velocity[2], 40.0 / 2.7, 1.0), "{slowed:?}");
}

#[test]
fn projectiles_fly_the_same_way_every_time_and_shrug_off_nonsense() {
    let grenade = WeaponSettings::grenade();
    let range = level(&[floor(), wall(-6.0)]);
    let throw = || {
        let mut thrown =
            Projectile::launch(&grenade, [0.3, 1.6, 0.0], [0.1, 0.4, -1.0], Some(1)).unwrap();
        let mut path = Vec::new();
        for _ in 0..2 * SECOND {
            thrown.step(&range, &[enemy(2, 4.0)], &grenade, DT);
            path.push(thrown);
        }
        path
    };
    assert_eq!(throw(), throw());

    let rifle = WeaponSettings::default();
    assert!(
        Projectile::launch(&rifle, EYES, AHEAD, None).is_none(),
        "a rifle launches nothing"
    );
    assert!(Projectile::launch(&grenade, EYES, [0.0; 3], None).is_none());
    assert!(Projectile::launch(&grenade, [f32::NAN; 3], AHEAD, None).is_none());
    let mut thrown = Projectile::launch(&grenade, EYES, AHEAD, None).unwrap();
    assert_eq!(
        thrown.step(&range, &[], &grenade, f32::NAN),
        Flight::Expired
    );
    assert_eq!(thrown.step(&range, &[], &rifle, DT), Flight::Expired);
    thrown.velocity = [f32::NAN; 3];
    assert_eq!(thrown.step(&range, &[], &grenade, DT), Flight::Expired);
}

#[test]
fn an_explosion_hurts_less_further_away_and_not_through_walls() {
    let launcher = WeaponSettings::rocket_launcher();
    let open = level(&[floor()]);
    let centre = [0.0, 1.0, 0.0];
    let at = |ahead: f32| blast_damage(&open, &launcher, centre, &enemy(2, ahead));
    assert_eq!(at(0.2), 120.0, "touching the centre");
    // 2.5 m from the body's surface is half-way to the 5 m edge.
    assert!(close(at(2.85), 65.0, 1.0e-3), "{}", at(2.85));
    assert!(close(at(5.3), 11.1, 0.2), "{}", at(5.3));
    assert_eq!(at(5.36), 0.0);
    assert_eq!(at(50.0), 0.0);

    // A wall taller than the player shields them; one they can see over
    // does not.
    let tall = level(&[floor(), wall(-1.5)]);
    assert_eq!(blast_damage(&tall, &launcher, centre, &enemy(2, 3.0)), 0.0);
    let low: Vec<Triangle> = wall(-1.5)
        .into_iter()
        .map(|corners| corners.map(|[x, y, z]| [x, y * 0.4, z]))
        .collect();
    let low = level(&[floor(), low]);
    let behind_cover = blast_damage(&low, &launcher, [0.0, 1.7, 0.0], &enemy(2, 3.0));
    assert!(behind_cover > 0.0);

    // A rifle does not explode.
    let rifle = WeaponSettings::default();
    assert_eq!(blast_damage(&open, &rifle, centre, &enemy(2, 1.0)), 0.0);
    assert_eq!(
        blast_damage(&open, &launcher, [f32::NAN; 3], &enemy(2, 1.0)),
        0.0
    );
}

#[test]
fn a_rocket_at_someones_feet_hurts_them_from_where_it_went_off() {
    let launcher = WeaponSettings::rocket_launcher();
    let range = level(&[floor()]);
    let victim = enemy(2, 10.0);
    // Aimed at the ground one metre in front of them.
    let mut rocket =
        Projectile::launch(&launcher, [0.0, 1.6, 0.0], [0.0, -1.6, -9.0], Some(1)).unwrap();
    let (flight, _) = fly(&mut rocket, &range, &[victim], &launcher, 5 * SECOND);
    let Flight::Exploded {
        point,
        touched: None,
    } = flight
    else {
        panic!("{flight:?}");
    };
    let damage = blast_damage(&range, &launcher, point, &victim);
    // 0.65 m from their body: most of the 120.
    assert!(damage > 95.0 && damage < 115.0, "{damage}");
}

#[test]
fn odd_targets_and_endless_settings_are_taken_in_stride() {
    let rifle = WeaponSettings {
        delivery: Delivery::Hitscan {
            reach: f32::INFINITY,
        },
        falloff_end: f32::INFINITY,
        ..Default::default()
    };
    let far = hitscan(&nothing(), &[enemy(2, 5000.0)], EYES, AHEAD, &rifle, None).unwrap();
    assert_eq!(far.target(), Some(2));
    assert!(close(far.damage, 30.0, 1.0e-2), "no falloff: {far:?}");

    // Targets with no height, or sizes that are not numbers, are a ball of
    // their own width (or a speck) wherever they stand.
    let launcher = WeaponSettings::rocket_launcher();
    for step in 0..2000_u16 {
        let y = 1.0 + f32::from(step) * 1.192_092_9e-7;
        for (height, radius) in [(0.0, 0.35), (f32::NAN, 0.35), (-3.0, f32::NAN), (0.5, 0.3)] {
            let odd = Target {
                id: 2,
                feet: [0.0, y, -10.0],
                height,
                radius,
            };
            let from = [0.0, y + 0.005, 0.0];
            let hit = hitscan(&nothing(), &[odd], from, AHEAD, &rifle, None).unwrap();
            assert!(hit.distance > 9.0 && hit.distance < 10.0, "{hit:?}");
            assert!(blast_damage(&nothing(), &launcher, from, &odd) == 0.0);
        }
    }

    // A projectile with no fuse flies on.
    let endless_fuse = WeaponSettings {
        delivery: Delivery::Projectile(ProjectileSettings {
            fuse: f32::INFINITY,
            ..ProjectileSettings::default()
        }),
        ..launcher
    };
    let mut rocket = Projectile::launch(&endless_fuse, EYES, AHEAD, None).unwrap();
    let (flight, _) = fly(&mut rocket, &nothing(), &[], &endless_fuse, 20 * SECOND);
    assert_eq!(flight, Flight::Flying);

    // A magazine as big as numbers go does not overflow on a reload.
    let huge = WeaponSettings {
        magazine_size: u32::MAX,
        keeps_chambered_round: true,
        reserve: u32::MAX,
        ..Default::default()
    };
    let mut bench = Bench::new(huge);
    bench.weapon.magazine = 5;
    let asked = WeaponInput {
        reload: true,
        ..idle()
    };
    assert!(bench.run(asked, 3 * SECOND).reload_finished);
    assert_eq!(bench.weapon.magazine, u32::MAX);
}
