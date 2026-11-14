//! Small levels built by hand, and a player walked through them.

use fossps_assets::{Instance, Mesh, MeshPart, Scene};

use crate::slide::SKIN;
use crate::{Level, MoveInput, MoveSettings, Player};

const DT: f32 = 1.0 / 60.0;
const SECOND: usize = 60;

type Triangle = [[f32; 3]; 3];

/// A four-cornered face, corners given anticlockwise seen from the front.
fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> [Triangle; 2] {
    [[a, b, c], [a, c, d]]
}

/// A flat floor at height 0, 80 metres across.
fn floor() -> Vec<Triangle> {
    let s = 40.0;
    quad([-s, 0.0, s], [s, 0.0, s], [s, 0.0, -s], [-s, 0.0, -s]).to_vec()
}

/// A solid box between two corners.
fn block(min: [f32; 3], max: [f32; 3]) -> Vec<Triangle> {
    let corner = |x: bool, y: bool, z: bool| {
        [
            if x { max[0] } else { min[0] },
            if y { max[1] } else { min[1] },
            if z { max[2] } else { min[2] },
        ]
    };
    let (f, t) = (false, true);
    [
        quad(
            corner(f, t, t),
            corner(t, t, t),
            corner(t, t, f),
            corner(f, t, f),
        ), // top
        quad(
            corner(f, f, f),
            corner(t, f, f),
            corner(t, f, t),
            corner(f, f, t),
        ), // bottom
        quad(
            corner(f, f, t),
            corner(t, f, t),
            corner(t, t, t),
            corner(f, t, t),
        ), // +z
        quad(
            corner(t, f, f),
            corner(f, f, f),
            corner(f, t, f),
            corner(t, t, f),
        ), // -z
        quad(
            corner(t, f, t),
            corner(t, f, f),
            corner(t, t, f),
            corner(t, t, t),
        ), // +x
        quad(
            corner(f, f, f),
            corner(f, f, t),
            corner(f, t, t),
            corner(f, t, f),
        ), // -x
    ]
    .concat()
}

/// A ramp 6 metres wide that starts at z = -2 and climbs toward -z at
/// `degrees`, for 10 metres along the ground.
fn ramp(degrees: f32) -> Vec<Triangle> {
    let top = 10.0 * degrees.to_radians().tan();
    quad(
        [-3.0, 0.0, -2.0],
        [3.0, 0.0, -2.0],
        [3.0, top, -12.0],
        [-3.0, top, -12.0],
    )
    .to_vec()
}

fn level(parts: &[Vec<Triangle>]) -> Level {
    Level::from_triangles(&parts.concat())
}

/// A player who has settled onto the ground at `position`.
fn standing_at(level: &Level, position: [f32; 3]) -> Player {
    let settings = MoveSettings::default();
    let mut player = Player::at(position, &settings);
    run(&mut player, level, MoveInput::default(), SECOND / 2);
    assert!(player.grounded, "did not settle: {player:?}");
    player
}

fn run(player: &mut Player, level: &Level, input: MoveInput, ticks: usize) {
    let settings = MoveSettings::default();
    for _ in 0..ticks {
        player.step(level, &settings, &input, DT);
    }
}

fn forward() -> MoveInput {
    MoveInput {
        forward: 1.0,
        ..MoveInput::default()
    }
}

fn speed(player: &Player) -> f32 {
    let [x, _, z] = player.velocity;
    x.hypot(z)
}

fn close(a: f32, b: f32, within: f32) -> bool {
    (a - b).abs() <= within
}

#[test]
fn a_player_dropped_above_the_floor_lands_on_it() {
    let level = level(&[floor()]);
    let mut player = Player::at([0.0, 3.0, 0.0], &MoveSettings::default());
    run(&mut player, &level, MoveInput::default(), 2 * SECOND);
    assert!(player.grounded);
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
    assert_eq!(player.velocity, [0.0; 3]);
    assert_eq!(player.eye_position()[1], player.position[1] + 1.6);
}

#[test]
fn walking_reaches_walk_speed_and_goes_forward() {
    let level = level(&[floor()]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    run(&mut player, &level, forward(), SECOND);
    assert!(close(speed(&player), 4.5, 0.01), "{player:?}");
    assert!(player.position[2] < -3.0, "forward is -z: {player:?}");
    assert!(close(player.position[0], 0.0, 1.0e-3));
    assert!(close(player.position[1], 0.0, 0.02));
}

#[test]
fn yaw_turns_left_and_strafing_goes_right() {
    let level = level(&[floor()]);
    let mut turned = standing_at(&level, [0.0, 0.0, 0.0]);
    let left = MoveInput {
        yaw: std::f32::consts::FRAC_PI_2,
        ..forward()
    };
    run(&mut turned, &level, left, SECOND);
    assert!(turned.position[0] < -3.0, "{turned:?}");
    assert!(close(turned.position[2], 0.0, 1.0e-3), "{turned:?}");

    let mut strafing = standing_at(&level, [0.0, 0.0, 0.0]);
    let right = MoveInput {
        right: 1.0,
        ..MoveInput::default()
    };
    run(&mut strafing, &level, right, SECOND);
    assert!(strafing.position[0] > 3.0, "{strafing:?}");
}

#[test]
fn a_diagonal_is_no_faster_and_a_half_pushed_stick_is_slower() {
    let level = level(&[floor()]);
    let mut diagonal = standing_at(&level, [0.0, 0.0, 0.0]);
    let both = MoveInput {
        right: 1.0,
        ..forward()
    };
    run(&mut diagonal, &level, both, SECOND);
    assert!(close(speed(&diagonal), 4.5, 0.01), "{diagonal:?}");

    let mut gentle = standing_at(&level, [0.0, 0.0, 0.0]);
    let half = MoveInput {
        forward: 0.5,
        ..MoveInput::default()
    };
    run(&mut gentle, &level, half, SECOND);
    assert!(close(speed(&gentle), 2.25, 0.01), "{gentle:?}");
}

#[test]
fn sprinting_is_faster_but_only_forward() {
    let level = level(&[floor()]);
    let sprint = MoveInput {
        sprint: true,
        ..forward()
    };
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    run(&mut player, &level, sprint, SECOND);
    assert!(player.sprinting);
    assert!(close(speed(&player), 6.5, 0.01), "{player:?}");

    let mut backward = standing_at(&level, [0.0, 0.0, 0.0]);
    let back = MoveInput {
        forward: -1.0,
        sprint: true,
        ..MoveInput::default()
    };
    run(&mut backward, &level, back, SECOND);
    assert!(!backward.sprinting);
    assert!(close(speed(&backward), 4.5, 0.01), "{backward:?}");
}

#[test]
fn letting_go_stops_the_player_quickly() {
    let level = level(&[floor()]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    run(&mut player, &level, forward(), SECOND);
    run(&mut player, &level, MoveInput::default(), SECOND / 2);
    assert_eq!(player.velocity, [0.0; 3], "{player:?}");
}

#[test]
fn crouching_is_slower_and_lowers_the_eyes() {
    let level = level(&[floor()]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let crouch = MoveInput {
        crouch: true,
        sprint: true,
        ..forward()
    };
    player.step(&level, &MoveSettings::default(), &crouch, DT);
    assert!(player.crouched);
    assert!(
        player.eye_height < 1.6 && player.eye_height > 1.0,
        "the eyes ease down: {player:?}"
    );
    run(&mut player, &level, crouch, SECOND);
    assert!(!player.sprinting);
    assert!(close(speed(&player), 2.0, 0.01), "{player:?}");
    assert_eq!(player.eye_height, 1.0);
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");

    run(&mut player, &level, MoveInput::default(), SECOND);
    assert!(!player.crouched);
    assert_eq!(player.eye_height, 1.6);
}

#[test]
fn a_crouched_player_fits_under_a_low_ceiling_and_cannot_stand_there() {
    // A slab whose underside is 1.3 m up, over z from -2 to -8.
    let level = level(&[floor(), block([-3.0, 1.3, -8.0], [3.0, 2.0, -2.0])]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);

    // Standing, the slab is a wall.
    run(&mut player, &level, forward(), 2 * SECOND);
    assert!(player.position[2] > -2.0, "{player:?}");

    // Crouched, the player goes under it.
    let crouch = MoveInput {
        crouch: true,
        ..forward()
    };
    run(&mut player, &level, crouch, 2 * SECOND);
    assert!(
        player.position[2] < -3.0 && player.position[2] > -7.0,
        "{player:?}"
    );

    // Letting go of crouch under the slab does not stand the player up.
    run(&mut player, &level, MoveInput::default(), SECOND / 2);
    assert!(player.crouched, "{player:?}");

    // Walking out the far side does.
    run(&mut player, &level, forward(), 4 * SECOND);
    assert!(player.position[2] < -8.5, "{player:?}");
    assert!(!player.crouched, "{player:?}");
}

#[test]
fn a_jump_peaks_at_the_jump_height_and_lands() {
    let level = level(&[floor()]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let start = player.position[1];
    let jump = MoveInput {
        jump: true,
        ..MoveInput::default()
    };
    let mut peak = start;
    let mut airborne = 0;
    for _ in 0..2 * SECOND {
        player.step(&level, &MoveSettings::default(), &jump, DT);
        peak = peak.max(player.position[1]);
        airborne += usize::from(!player.grounded);
    }
    assert!(close(peak - start, 1.1, 0.02), "peaked at {peak}");
    assert!(player.grounded, "{player:?}");
    assert!(close(player.position[1], start, 0.02), "{player:?}");
    // Up and down once (about two thirds of a second), not bouncing for
    // as long as jump is held.
    assert!((35..=45).contains(&airborne), "{airborne} ticks in the air");

    // Letting go and pressing again jumps again.
    player.step(&level, &MoveSettings::default(), &MoveInput::default(), DT);
    player.step(&level, &MoveSettings::default(), &jump, DT);
    assert!(!player.grounded);
}

#[test]
fn jumping_into_a_ceiling_stops_the_rise() {
    let level = level(&[floor(), block([-3.0, 2.2, -3.0], [3.0, 3.0, 3.0])]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let jump = MoveInput {
        jump: true,
        ..MoveInput::default()
    };
    let mut peak = 0.0_f32;
    for _ in 0..SECOND {
        player.step(&level, &MoveSettings::default(), &jump, DT);
        peak = peak.max(player.position[1] + 1.8);
    }
    assert!(peak <= 2.2 && peak > 2.1, "head reached {peak}");
    assert!(player.grounded);
}

#[test]
fn steering_in_the_air_is_weak() {
    let level = level(&[floor()]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let jump = MoveInput {
        jump: true,
        ..forward()
    };
    run(&mut player, &level, jump, SECOND / 4);
    assert!(!player.grounded);
    assert!(speed(&player) <= 0.76, "{player:?}");
}

#[test]
fn a_wall_stops_the_player_and_lets_them_slide_along_it() {
    // A wall facing +z at z = -2.
    let level = level(&[floor(), block([-20.0, 0.0, -3.0], [20.0, 3.0, -2.0])]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    run(&mut player, &level, forward(), 2 * SECOND);
    let gap = player.position[2] - 0.35 - -2.0;
    assert!((0.0..0.03).contains(&gap), "stopped {gap} m from the wall");
    assert!(speed(&player) < 0.05, "{player:?}");

    // Pushing into it at an angle slides along it.
    let angled = MoveInput {
        right: 1.0,
        ..forward()
    };
    run(&mut player, &level, angled, 2 * SECOND);
    assert!(player.position[0] > 4.0, "{player:?}");
    let gap = player.position[2] - 0.35 - -2.0;
    assert!((0.0..0.03).contains(&gap), "slid {gap} m from the wall");
    assert!(player.grounded);
}

#[test]
fn an_inside_corner_holds_the_player() {
    let level = level(&[
        floor(),
        block([-20.0, 0.0, -3.0], [20.0, 3.0, -2.0]),
        block([2.0, 0.0, -20.0], [3.0, 3.0, 20.0]),
    ]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let into_corner = MoveInput {
        right: 1.0,
        ..forward()
    };
    run(&mut player, &level, into_corner, 5 * SECOND);
    let [x, y, z] = player.position;
    assert!(close(x, 2.0 - 0.35, 0.03), "{player:?}");
    assert!(close(z, -2.0 + 0.35, 0.03), "{player:?}");
    assert!(close(y, 0.0, 0.02), "{player:?}");
}

#[test]
fn stairs_are_climbed_and_descended_without_jumping() {
    // Five stairs, each 0.3 m higher and 0.5 m deeper, then a landing.
    let mut parts = vec![floor()];
    for stair in 0..5 {
        let (n, top) = (stair as f32, 0.3 * (stair + 1) as f32);
        parts.push(block([-3.0, 0.0, -20.0], [3.0, top, -2.0 - 0.5 * n]));
    }
    let level = level(&parts);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let mut left_ground = 0;
    for _ in 0..3 * SECOND {
        player.step(&level, &MoveSettings::default(), &forward(), DT);
        left_ground += usize::from(!player.grounded);
    }
    assert!(close(player.position[1], 1.5, 0.02), "{player:?}");
    assert!(player.position[2] < -6.0, "{player:?}");
    assert_eq!(left_ground, 0, "climbing stairs is walking, not hopping");

    // And back down, staying on the ground the whole way.
    let back = MoveInput {
        forward: -1.0,
        ..MoveInput::default()
    };
    for _ in 0..4 * SECOND {
        player.step(&level, &MoveSettings::default(), &back, DT);
        left_ground += usize::from(!player.grounded);
    }
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
    assert!(player.position[2] > -1.0, "{player:?}");
    assert_eq!(left_ground, 0, "descending stairs keeps the feet down");
}

#[test]
fn a_slow_walker_still_climbs_stairs() {
    let level = level(&[floor(), block([-3.0, 0.0, -20.0], [3.0, 0.3, -2.0])]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let creep = MoveInput {
        forward: 0.2,
        crouch: true,
        ..MoveInput::default()
    };
    run(&mut player, &level, creep, 12 * SECOND);
    assert!(close(player.position[1], 0.3, 0.02), "{player:?}");
    assert!(player.position[2] < -2.5, "{player:?}");
}

#[test]
fn a_ledge_taller_than_a_step_needs_a_jump() {
    let level = level(&[floor(), block([-3.0, 0.0, -20.0], [3.0, 0.6, -2.0])]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    run(&mut player, &level, forward(), 2 * SECOND);
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
    assert!(player.position[2] > -2.0, "{player:?}");

    let jump = MoveInput {
        jump: true,
        ..forward()
    };
    run(&mut player, &level, jump, SECOND);
    run(&mut player, &level, forward(), SECOND);
    assert!(close(player.position[1], 0.6, 0.02), "{player:?}");
    assert!(player.grounded);
}

#[test]
fn a_gentle_ramp_is_walked_at_full_speed_both_ways() {
    let level = level(&[floor(), ramp(30.0)]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let mut left_ground = 0;
    for _ in 0..2 * SECOND {
        player.step(&level, &MoveSettings::default(), &forward(), DT);
        left_ground += usize::from(!player.grounded);
    }
    assert!(player.position[1] > 2.0, "{player:?}");
    assert!(close(speed(&player), 4.5, 0.05), "{player:?}");
    assert!(player.ground_normal[1] < 0.9, "{player:?}");

    // Standing still on it does not slide.
    run(&mut player, &level, MoveInput::default(), SECOND / 2);
    let rest = player.position;
    run(&mut player, &level, MoveInput::default(), SECOND);
    assert_eq!(player.position, rest);

    let back = MoveInput {
        forward: -1.0,
        ..MoveInput::default()
    };
    for _ in 0..3 * SECOND {
        player.step(&level, &MoveSettings::default(), &back, DT);
        left_ground += usize::from(!player.grounded);
    }
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
    assert_eq!(left_ground, 0, "the feet stay down on a ramp");
}

#[test]
fn a_steep_bank_cannot_be_climbed_or_stood_on() {
    let level = level(&[floor(), ramp(60.0)]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    run(&mut player, &level, forward(), 3 * SECOND);
    assert!(player.position[1] < 0.2, "{player:?}");
    assert!(player.position[2] > -3.0, "{player:?}");

    // Dropped onto it, the player slides to the bottom.
    let mut dropped = Player::at([0.0, 9.0, -6.0], &MoveSettings::default());
    run(&mut dropped, &level, MoveInput::default(), 5 * SECOND);
    assert!(dropped.grounded, "{dropped:?}");
    assert!(close(dropped.position[1], 0.0, 0.05), "{dropped:?}");
    assert!(dropped.position[2] > -3.0, "{dropped:?}");
}

#[test]
fn walking_off_a_ledge_falls() {
    let level = level(&[floor(), block([-3.0, 0.0, -3.0], [3.0, 2.0, 3.0])]);
    let mut player = standing_at(&level, [0.0, 2.0, 0.0]);
    assert!(close(player.position[1], 2.0, 0.02));
    run(&mut player, &level, forward(), SECOND);
    assert!(!player.grounded || player.position[1] < 1.0, "{player:?}");
    run(&mut player, &level, MoveInput::default(), SECOND);
    assert!(player.grounded);
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
}

#[test]
fn a_player_spawned_inside_the_floor_is_pushed_out() {
    let level = level(&[floor()]);
    let mut player = Player::at([0.0, -0.2, 0.0], &MoveSettings::default());
    run(&mut player, &level, MoveInput::default(), SECOND);
    assert!(player.grounded, "{player:?}");
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
}

#[test]
fn a_fast_fall_does_not_pass_through_a_thin_floor() {
    let level = level(&[floor()]);
    let mut player = Player::at([0.0, 300.0, 0.0], &MoveSettings::default());
    run(&mut player, &level, MoveInput::default(), 12 * SECOND);
    assert!(player.grounded, "{player:?}");
    assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
}

#[test]
fn the_same_inputs_replay_to_the_same_bits() {
    let mut parts = vec![floor(), ramp(30.0)];
    parts.push(block([4.0, 0.0, -6.0], [6.0, 0.3, -4.0]));
    parts.push(block([-8.0, 0.0, -9.0], [-5.0, 3.0, 9.0]));
    let level = level(&parts);
    let settings = MoveSettings::default();
    // A wander that turns, strafes, sprints, crouches and jumps.
    let input = |tick: usize| MoveInput {
        forward: if tick % 97 < 80 { 1.0 } else { -0.5 },
        right: if tick % 53 < 20 { 0.7 } else { 0.0 },
        yaw: tick as f32 * 0.013,
        jump: tick.is_multiple_of(45),
        crouch: tick % 200 > 170,
        sprint: tick % 120 < 60,
    };
    let play = |mut player: Player, ticks: std::ops::Range<usize>| {
        let mut history = Vec::new();
        for tick in ticks {
            player.step(&level, &settings, &input(tick), DT);
            history.push(player);
        }
        history
    };
    let bits = |player: &Player| {
        [player.position, player.velocity, player.ground_normal].map(|v| v.map(f32::to_bits))
    };

    let start = Player::at([0.0, 0.5, 0.0], &settings);
    let first = play(start, 0..600);
    let second = play(start, 0..600);
    assert!(first.iter().map(bits).eq(second.iter().map(bits)));
    assert!(first.iter().any(|p| !p.grounded) && first.iter().any(|p| p.crouched));

    // A client that goes back to tick 300 and replays ends in the same place.
    let replayed = play(first[299], 300..600);
    assert_eq!(bits(&replayed[299]), bits(&first[599]));
    assert_eq!(replayed[299], first[599]);
}

#[test]
fn nonsense_input_and_settings_do_no_harm() {
    let level = level(&[floor()]);
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let nonsense = MoveInput {
        forward: f32::NAN,
        right: f32::INFINITY,
        yaw: f32::NAN,
        ..MoveInput::default()
    };
    run(&mut player, &level, nonsense, SECOND);
    assert!(player.position.iter().all(|n| n.is_finite()));
    assert_eq!(
        player.velocity, [0.0; 3],
        "a push that is not a number is no push"
    );

    let before = player;
    player.step(&level, &MoveSettings::default(), &forward(), f32::NAN);
    player.step(&level, &MoveSettings::default(), &forward(), 0.0);
    assert_eq!(player, before);

    let broken = MoveSettings {
        radius: f32::NAN,
        standing_height: -1.0,
        gravity: f32::NAN,
        walk_speed: f32::INFINITY,
        steepest_slope: f32::NAN,
        ..MoveSettings::default()
    };
    for _ in 0..SECOND {
        player.step(&level, &broken, &forward(), DT);
    }
    assert!(player.position.iter().all(|n| n.is_finite()), "{player:?}");
}

#[test]
fn extreme_settings_keep_the_player_on_the_floor() {
    let level = level(&[floor()]);
    let odd = [
        MoveSettings {
            step_height: 0.0,
            ..MoveSettings::default()
        },
        MoveSettings {
            radius: 0.0,
            standing_height: 0.001,
            ..MoveSettings::default()
        },
        MoveSettings {
            standing_height: 30.0,
            friction: 1.0e30,
            ground_acceleration: 1.0e30,
            ..MoveSettings::default()
        },
    ];
    for settings in odd {
        let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
        for tick in 0..2 * SECOND {
            player.step(&level, &settings, &forward(), DT);
            assert!(player.grounded, "tick {tick} with {settings:?}: {player:?}");
        }
        assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
        assert!(player.position[2] < -3.0, "{player:?}");
        assert!(player.velocity.iter().all(|n| n.is_finite()), "{player:?}");
    }

    // Numbers large enough to overflow leave the player exactly as they were.
    let overflowing = MoveSettings {
        jump_height: 1.0e30,
        gravity: 1.0e30,
        fall_speed_limit: f32::MAX,
        ..MoveSettings::default()
    };
    let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
    let jump = MoveInput {
        jump: true,
        ..MoveInput::default()
    };
    for _ in 0..SECOND {
        let before = player;
        player.step(&level, &overflowing, &jump, DT);
        let finite = player
            .position
            .iter()
            .chain(&player.velocity)
            .all(|n| n.is_finite());
        assert!(finite, "{player:?}");
        if player.position == before.position {
            assert_eq!(player, before);
        }
    }
}

#[test]
fn an_empty_level_is_empty_space() {
    let empty = Level::from_triangles(&[]);
    assert_eq!(empty.triangle_count(), 0);
    let flat = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let broken = [[f32::NAN; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    assert_eq!(Level::from_triangles(&[flat, broken]).triangle_count(), 0);

    let mut player = Player::at([0.0; 3], &MoveSettings::default());
    run(&mut player, &empty, forward(), SECOND);
    assert!(!player.grounded);
    assert!(player.position[1] < -5.0);
}

#[test]
fn a_scene_becomes_a_level_where_its_meshes_stand() {
    let s = 5.0;
    let part = MeshPart {
        positions: vec![[-s, 0.0, s], [s, 0.0, s], [s, 0.0, -s], [-s, 0.0, -s]],
        indices: vec![0, 1, 2, 0, 2, 3],
        ..MeshPart::default()
    };
    let placed = |transform| Instance {
        mesh: 0,
        transform,
        mirrored: false,
        name: None,
        extras: None,
    };
    // The same floor twice: raised 2 m, and raised 5 m, moved 20 m along x
    // and scaled to half size.
    let scene = Scene {
        meshes: vec![Mesh {
            name: None,
            parts: vec![part],
        }],
        instances: vec![
            placed([
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 2.0, 0.0, 1.0],
            ]),
            Instance {
                mirrored: true,
                ..placed([
                    [-0.5, 0.0, 0.0, 0.0],
                    [0.0, 0.5, 0.0, 0.0],
                    [0.0, 0.0, 0.5, 0.0],
                    [20.0, 5.0, 0.0, 1.0],
                ])
            },
        ],
        ..Scene::default()
    };
    let level = Level::from_scene(&scene);
    assert_eq!(level.triangle_count(), 4);

    let settings = MoveSettings::default();
    let mut first = Player::at([0.0, 4.0, 0.0], &settings);
    let mut second = Player::at([20.0, 8.0, 0.0], &settings);
    let mut beside = Player::at([23.0, 8.0, 0.0], &settings);
    for player in [&mut first, &mut second, &mut beside] {
        run(player, &level, MoveInput::default(), 2 * SECOND);
    }
    assert!(
        first.grounded && close(first.position[1], 2.0, 0.02),
        "{first:?}"
    );
    assert!(
        second.grounded && close(second.position[1], 5.0, 0.02),
        "{second:?}"
    );
    assert!(
        !beside.grounded,
        "the half-size floor ends at 22.5: {beside:?}"
    );
}

#[test]
fn crossing_a_ramp_at_an_angle_keeps_its_heading() {
    let level = level(&[floor(), ramp(30.0)]);
    let mut player = standing_at(&level, [-2.0, 0.0, -1.0]);
    // Up and to the right, at 30 degrees off straight up the ramp.
    let angled = MoveInput {
        yaw: -std::f32::consts::FRAC_PI_6,
        ..forward()
    };
    run(&mut player, &level, angled, SECOND);
    let [x, _, z] = player.velocity;
    assert!(
        close(x / -z, (30.0_f32).to_radians().tan(), 0.01),
        "{player:?}"
    );
    assert!(close(speed(&player), 4.5, 0.05), "{player:?}");
    assert!(player.grounded && player.position[1] > 0.5, "{player:?}");
}

#[test]
fn a_floor_of_many_small_triangles_is_as_smooth_as_one_big_one() {
    // A 10 m square of 25 cm squares, each two triangles.
    let mut tiles = Vec::new();
    for i in 0..40 {
        for j in 0..40 {
            let (x, z) = (-5.0 + 0.25 * i as f32, -5.0 + 0.25 * j as f32);
            tiles.extend(quad(
                [x, 0.0, z + 0.25],
                [x + 0.25, 0.0, z + 0.25],
                [x + 0.25, 0.0, z],
                [x, 0.0, z],
            ));
        }
    }
    let level = level(&[tiles]);
    for turn in 0..16 {
        let yaw = turn as f32 * std::f32::consts::TAU / 16.0 + 0.1;
        let input = MoveInput { yaw, ..forward() };
        let mut player = standing_at(&level, [0.0, 0.0, 0.0]);
        run(&mut player, &level, input, SECOND / 2);
        for _ in 0..SECOND / 2 {
            player.step(&level, &MoveSettings::default(), &input, DT);
            assert!(player.grounded, "{player:?}");
            assert!(close(speed(&player), 4.5, 0.01), "{player:?}");
            assert!(close(player.position[1], 0.0, 0.02), "{player:?}");
        }
        // Still heading where it was pointed.
        let [x, _, z] = player.position;
        assert!(
            close(x.atan2(z), (-yaw.sin()).atan2(-yaw.cos()), 0.005),
            "{player:?}"
        );
    }
}

#[test]
fn a_long_erratic_run_never_ends_up_inside_or_outside_the_level() {
    // A walled room, 20 m square, with a floor of small triangles and one
    // of everything in it.
    let mut parts = Vec::new();
    for i in 0..20 {
        for j in 0..20 {
            let (x, z) = (-10.0 + i as f32, -10.0 + j as f32);
            parts.push(
                quad(
                    [x, 0.0, z + 1.0],
                    [x + 1.0, 0.0, z + 1.0],
                    [x + 1.0, 0.0, z],
                    [x, 0.0, z],
                )
                .to_vec(),
            );
        }
    }
    parts.push(block([-11.0, 0.0, -11.0], [11.0, 6.0, -10.0]));
    parts.push(block([-11.0, 0.0, 10.0], [11.0, 6.0, 11.0]));
    parts.push(block([-11.0, 0.0, -11.0], [-10.0, 6.0, 11.0]));
    parts.push(block([10.0, 0.0, -11.0], [11.0, 6.0, 11.0]));
    parts.push(block([-11.0, 6.0, -11.0], [11.0, 7.0, 11.0])); // roof
    parts.push(block([3.0, 0.0, 3.0], [4.0, 6.0, 4.0])); // pillar
    parts.push(block([-6.0, 0.0, 2.0], [-3.0, 1.0, 5.0])); // crate to jump on
    parts.push(block([-2.0, 1.2, 4.0], [2.0, 1.6, 8.0])); // table to crouch under
    for stair in 0..6 {
        let n = stair as f32;
        parts.push(block(
            [5.0, 0.0, -10.0],
            [9.0, 0.3 * (n + 1.0), -4.0 - 0.5 * n],
        ));
    }
    parts.push(
        ramp(30.0)
            .iter()
            .map(|t| t.map(|[x, y, z]| [x - 6.0, y, z + 1.0]))
            .collect(),
    );
    parts.push(
        quad(
            [-1.0, 0.0, -6.0],
            [2.0, 0.0, -6.0],
            [2.0, 4.0, -8.0],
            [-1.0, 4.0, -8.0],
        )
        .to_vec(), // a bank too steep to stand on
    );
    let level = level(&parts);
    let settings = MoveSettings::default();

    // A small repeatable source of erratic choices.
    let mut seed = 0x2545_f491_u32;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };

    let mut player = Player::at([0.0, 0.5, 0.0], &settings);
    let mut input = MoveInput::default();
    let mut furthest = 0.0_f32;
    let mut highest = 0.0_f32;
    for tick in 0..40_000_u32 {
        if tick.is_multiple_of(20) {
            let bits = random();
            input = MoveInput {
                forward: [1.0, 1.0, 1.0, 0.3, -1.0, 0.0][(bits % 6) as usize],
                right: [0.0, 1.0, -1.0, 0.4][(bits >> 4 & 3) as usize],
                yaw: (bits >> 8 & 0xff) as f32 * 0.0246,
                jump: bits >> 16 & 3 == 0,
                crouch: bits >> 18 & 7 == 0,
                sprint: bits >> 21 & 1 == 0,
            };
        } else {
            // Jump is pressed, not held, so it can fire again.
            input.jump = false;
        }
        player.step(&level, &settings, &input, DT);

        let [x, y, z] = player.position;
        let inside = x.abs() <= 10.0 - 0.35 + 1.0e-3 && z.abs() <= 10.0 - 0.35 + 1.0e-3;
        assert!(inside, "left the room at tick {tick}: {player:?}");
        assert!((-1.0e-3..6.0).contains(&y), "tick {tick}: {player:?}");
        // The player keeps a 1 cm gap from everything; squeezed into a
        // wedge it may give up some of that gap, but never more.
        let height = if player.crouched { 1.1 } else { 1.8 };
        let middle = parry3d::math::Vector::new(x, y + height / 2.0, z);
        let core = parry3d::shape::Capsule::new_y(height / 2.0 - 0.35, 0.35 - SKIN);
        assert!(
            !level.overlaps(&core, middle),
            "inside something solid at tick {tick}: {player:?}"
        );
        furthest = furthest.max(x.hypot(z));
        highest = highest.max(y);
    }
    // It really did get about: across the room and up the stairs or ramp.
    assert!(furthest > 8.0 && highest > 1.5, "{furthest} {highest}");
}

#[test]
fn a_ray_finds_the_first_surface_and_faces_the_looker() {
    let level = level(&[floor(), block([-1.0, 0.0, -6.0], [1.0, 2.0, -5.0])]);
    // Forward at chest height: the near face of the block, 5 m off.
    let hit = level
        .raycast([0.0, 1.0, 0.0], [0.0, 0.0, -3.0], 100.0)
        .unwrap();
    assert!(close(hit.distance, 5.0, 1.0e-4), "{hit:?}");
    assert!(close(hit.point[2], -5.0, 1.0e-4), "{hit:?}");
    assert!(close(hit.normal[2], 1.0, 1.0e-4), "{hit:?}");
    // From under the floor looking up, the floor still faces the looker.
    let hit = level
        .raycast([5.0, -2.0, 5.0], [0.0, 1.0, 0.0], 100.0)
        .unwrap();
    assert!(close(hit.distance, 2.0, 1.0e-4), "{hit:?}");
    assert!(close(hit.normal[1], -1.0, 1.0e-4), "{hit:?}");
}

#[test]
fn a_ray_stops_at_its_reach_and_ignores_nonsense() {
    let level = level(&[floor(), block([-1.0, 0.0, -6.0], [1.0, 2.0, -5.0])]);
    let from = [0.0, 1.0, 0.0];
    assert!(level.raycast(from, [0.0, 0.0, -1.0], 4.9).is_none());
    assert!(level.raycast(from, [0.0, 1.0, 0.0], 100.0).is_none(), "sky");
    assert!(level.raycast(from, [0.0; 3], 100.0).is_none());
    assert!(level.raycast(from, [f32::NAN, 0.0, -1.0], 100.0).is_none());
    assert!(level.raycast(from, [0.0, 0.0, -1.0], f32::NAN).is_none());
    assert!(
        level
            .raycast(from, [0.0, 0.0, -1.0], f32::INFINITY)
            .is_some()
    );
    assert!(
        Level::from_triangles(&[])
            .raycast(from, [0.0, -1.0, 0.0], 9.0)
            .is_none()
    );
}
