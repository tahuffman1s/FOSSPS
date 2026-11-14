//! Whole matches played by hand.

use crate::{
    Event, Match, Phase, PlayerId, Rules, RulesError, SpawnPoint, Stander, TeamId, Winner,
    choose_spawn,
};

const STEP: f32 = 1.0 / 60.0;
const SECOND: u32 = 60;

fn teams() -> Rules {
    Rules {
        name: "Team Deathmatch".to_owned(),
        teams: vec!["Red".to_owned(), "Blue".to_owned()],
        ..Rules::default()
    }
}

fn run(game: &mut Match, ticks: u32) {
    for _ in 0..ticks {
        game.tick(STEP);
    }
}

fn events(game: &mut Match) -> Vec<Event> {
    std::iter::from_fn(|| game.next_event()).collect()
}

/// A team match with two players, past its warm-up, with nothing left to read.
fn playing() -> (Match, PlayerId, PlayerId) {
    let mut game = Match::new(teams()).unwrap();
    let ann = game.join("Ann");
    let bo = game.join("Bo");
    run(&mut game, 5 * SECOND);
    assert_eq!(game.phase(), Phase::Playing);
    events(&mut game);
    (game, ann, bo)
}

const RED: TeamId = TeamId(0);
const BLUE: TeamId = TeamId(1);

#[test]
fn an_empty_match_waits() {
    let mut game = Match::new(Rules::default()).unwrap();
    run(&mut game, 10 * SECOND);
    assert_eq!(game.phase(), Phase::Waiting);
    assert_eq!(game.round(), 0);
    assert_eq!(game.time_left(), None);
    assert_eq!(events(&mut game), []);
}

#[test]
fn the_first_player_starts_the_warm_up_and_is_spawned() {
    let mut game = Match::new(teams()).unwrap();
    let ann = game.join("Ann");
    assert_eq!(game.phase(), Phase::Warmup);
    assert_eq!(game.round(), 1);
    assert_eq!(game.time_left(), Some(5.0));
    assert_eq!(events(&mut game), [Event::Joined(ann), Event::Spawned(ann)]);
    assert!(game.player(ann).unwrap().alive);
}

#[test]
fn the_warm_up_lasts_exactly_as_long_as_the_rules_say() {
    for rate in [30u32, 60, 100, 128, 144] {
        let mut game = Match::new(teams()).unwrap();
        game.join("Ann");
        let step = 1.0 / rate as f32;
        for _ in 0..5 * rate - 1 {
            game.tick(step);
        }
        assert_eq!(game.phase(), Phase::Warmup, "at {rate} ticks a second");
        game.tick(step);
        assert_eq!(game.phase(), Phase::Playing, "at {rate} ticks a second");
    }
}

#[test]
fn the_round_starts_with_clean_scores_and_everyone_at_a_spawn_point() {
    let mut game = Match::new(teams()).unwrap();
    let ann = game.join("Ann");
    let bo = game.join("Bo");
    assert!(game.kill(Some(ann), bo));
    assert_eq!(game.player(ann).unwrap().kills, 1);
    assert_eq!(game.teams()[0].score, 0, "nothing scores in the warm-up");
    events(&mut game);

    run(&mut game, 5 * SECOND);
    assert_eq!(
        events(&mut game),
        [
            Event::Spawned(bo),
            Event::RoundStarted,
            Event::Spawned(ann),
            Event::Spawned(bo)
        ],
        "Bo came back after 3 s, then the round started"
    );
    assert_eq!(game.player(ann).unwrap().kills, 0);
    assert_eq!(game.player(bo).unwrap().deaths, 0);
    assert_eq!(game.time_left(), Some(600.0));
}

#[test]
fn players_are_shared_out_over_the_teams() {
    let mut game = Match::new(teams()).unwrap();
    let ids: Vec<_> = ["a", "b", "c", "d", "e"]
        .into_iter()
        .map(|name| game.join(name))
        .collect();
    let team = |game: &Match, id| game.player(id).unwrap().team;
    assert_eq!(
        ids.iter().map(|&id| team(&game, id)).collect::<Vec<_>>(),
        [Some(RED), Some(BLUE), Some(RED), Some(BLUE), Some(RED)]
    );
    // Two reds leave: the next two players even the teams out again.
    game.leave(ids[0]);
    game.leave(ids[2]);
    let f = game.join("f");
    let g = game.join("g");
    assert_eq!([team(&game, f), team(&game, g)], [Some(RED), Some(RED)]);
}

#[test]
fn with_equal_teams_a_new_player_joins_the_one_that_is_losing() {
    let (mut game, _, bo) = playing();
    game.add_team_score(RED, 3);
    let cy = game.join("Cy");
    let di = game.join("Di");
    assert_eq!(game.player(cy).unwrap().team, Some(BLUE));
    assert_eq!(game.player(di).unwrap().team, Some(RED));
    assert_eq!(game.player(bo).unwrap().team, Some(BLUE));
}

#[test]
fn a_kill_scores_for_the_killer_and_their_team() {
    let (mut game, ann, bo) = playing();
    assert!(game.kill(Some(ann), bo));
    let killer = game.player(ann).unwrap();
    assert_eq!((killer.kills, killer.score), (1, 1));
    let victim = game.player(bo).unwrap();
    assert_eq!((victim.deaths, victim.alive), (1, false));
    assert_eq!(victim.respawn_in, Some(3.0));
    assert_eq!(game.teams()[0].score, 1);
    assert_eq!(game.teams()[1].score, 0);
    assert_eq!(
        events(&mut game),
        [Event::Killed {
            killer: Some(ann),
            victim: bo
        }]
    );
}

#[test]
fn killing_a_team_mate_or_yourself_scores_nothing() {
    let (mut game, ann, _) = playing();
    let cy = game.join("Cy");
    assert_eq!(game.player(cy).unwrap().team, Some(RED));
    assert!(game.kill(Some(ann), cy));
    assert_eq!(game.teams()[0].score, 0);
    assert_eq!(game.player(ann).unwrap().score, 0);

    events(&mut game);
    assert!(game.kill(Some(ann), ann));
    assert_eq!(
        events(&mut game),
        [Event::Killed {
            killer: None,
            victim: ann
        }],
        "killing yourself is dying alone"
    );
    assert_eq!(game.player(ann).unwrap().kills, 1, "only Cy");
}

#[test]
fn kill_points_can_be_changed_or_switched_off() {
    let mut rules = teams();
    rules.kill_points = 0;
    rules.warmup = 0.0;
    let mut game = Match::new(rules.clone()).unwrap();
    let ann = game.join("Ann");
    let bo = game.join("Bo");
    game.kill(Some(ann), bo);
    assert_eq!(game.teams()[0].score, 0);

    rules.kill_points = 5;
    game.set_rules(rules).unwrap();
    game.kill(Some(bo), ann);
    assert_eq!(game.teams()[1].score, 5);
}

#[test]
fn the_dead_cannot_be_killed_again() {
    let (mut game, ann, bo) = playing();
    assert!(game.kill(Some(ann), bo));
    assert!(!game.kill(Some(ann), bo));
    assert!(!game.kill(Some(ann), PlayerId(99)));
    assert_eq!(game.teams()[0].score, 1);
}

#[test]
fn the_dead_come_back_after_the_respawn_delay() {
    let (mut game, ann, bo) = playing();
    game.kill(Some(ann), bo);
    events(&mut game);
    run(&mut game, 3 * SECOND - 1);
    assert!(!game.player(bo).unwrap().alive);
    assert_eq!(events(&mut game), []);
    game.tick(STEP);
    assert!(game.player(bo).unwrap().alive);
    assert_eq!(game.player(bo).unwrap().respawn_in, None);
    assert_eq!(events(&mut game), [Event::Spawned(bo)]);
}

#[test]
fn respawn_skips_the_wait() {
    let (mut game, ann, bo) = playing();
    game.kill(Some(ann), bo);
    events(&mut game);
    assert!(game.respawn(bo));
    assert_eq!(events(&mut game), [Event::Spawned(bo)]);
    run(&mut game, 4 * SECOND);
    assert_eq!(events(&mut game), [], "and they do not spawn a second time");
}

#[test]
fn reaching_the_score_to_win_ends_the_round_at_once() {
    let (mut game, ann, bo) = playing();
    game.add_team_score(RED, 49);
    assert_eq!(game.phase(), Phase::Playing);
    game.kill(Some(ann), bo);
    assert_eq!(game.phase(), Phase::Over);
    assert_eq!(
        events(&mut game).last(),
        Some(&Event::RoundEnded {
            winner: Some(Winner::Team(RED))
        })
    );
    assert_eq!(game.time_left(), Some(10.0));
    assert!(!game.add_team_score(BLUE, 100), "the final score stands");
    assert_eq!(game.teams()[1].score, 0);
    assert_eq!(game.player(bo).unwrap().respawn_in, None);
}

#[test]
fn two_sides_reaching_the_score_together_is_won_by_the_higher_or_drawn() {
    let (mut game, _, _) = playing();
    game.add_team_score(RED, 49);
    game.add_team_score(BLUE, 49);
    let mut rules = teams();
    rules.score_to_win = 40;
    game.set_rules(rules).unwrap();
    assert_eq!(events(&mut game), [Event::RoundEnded { winner: None }]);

    let (mut game, _, _) = playing();
    game.add_team_score(BLUE, 45);
    game.add_team_score(RED, 60);
    assert_eq!(
        events(&mut game),
        [Event::RoundEnded {
            winner: Some(Winner::Team(RED))
        }]
    );
}

#[test]
fn when_time_runs_out_the_highest_score_wins() {
    let (mut game, _, _) = playing();
    game.add_team_score(BLUE, 2);
    game.add_team_score(RED, 1);
    run(&mut game, 600 * SECOND - 1);
    assert_eq!(game.phase(), Phase::Playing);
    game.tick(STEP);
    assert_eq!(game.phase(), Phase::Over);
    assert_eq!(
        events(&mut game),
        [Event::RoundEnded {
            winner: Some(Winner::Team(BLUE))
        }]
    );
}

#[test]
fn equal_scores_when_time_runs_out_are_a_draw() {
    let (mut game, _, _) = playing();
    run(&mut game, 600 * SECOND);
    assert_eq!(events(&mut game), [Event::RoundEnded { winner: None }]);
}

#[test]
fn a_round_with_no_time_limit_only_ends_on_score() {
    let mut rules = teams();
    rules.time_limit = 0.0;
    rules.warmup = 0.0;
    let mut game = Match::new(rules).unwrap();
    game.join("Ann");
    run(&mut game, 3600 * SECOND);
    assert_eq!(game.phase(), Phase::Playing);
    assert_eq!(game.time_left(), None);
}

#[test]
fn after_the_intermission_the_next_round_warms_up() {
    let (mut game, ann, bo) = playing();
    game.kill(Some(ann), bo);
    assert!(game.finish(None));
    assert!(!game.finish(None), "a round ends once");
    events(&mut game);
    run(&mut game, 10 * SECOND);
    assert_eq!(game.phase(), Phase::Warmup);
    assert_eq!(game.round(), 2);
    assert_eq!(
        events(&mut game),
        [Event::Spawned(bo)],
        "whoever was dead comes back for the warm-up"
    );
    assert_eq!(game.teams()[0].score, 0);
    run(&mut game, 5 * SECOND);
    assert_eq!(game.phase(), Phase::Playing);
}

#[test]
fn someone_who_joins_during_the_intermission_waits_for_the_next_round() {
    let (mut game, _, _) = playing();
    game.finish(None);
    let cy = game.join("Cy");
    assert!(!game.player(cy).unwrap().alive);
    assert!(!game.respawn(cy));
    run(&mut game, 10 * SECOND);
    assert!(game.player(cy).unwrap().alive);
}

#[test]
fn every_player_for_themselves_is_won_by_a_player() {
    let mut game = Match::new(Rules {
        score_to_win: 2,
        warmup: 0.0,
        ..Rules::default()
    })
    .unwrap();
    let ann = game.join("Ann");
    let bo = game.join("Bo");
    assert_eq!(game.player(ann).unwrap().team, None);
    assert_eq!(game.teams(), []);
    game.kill(Some(ann), bo);
    assert_eq!(game.leader(), Some(Winner::Player(ann)));
    game.respawn(bo);
    game.kill(Some(ann), bo);
    assert_eq!(
        events(&mut game).last(),
        Some(&Event::RoundEnded {
            winner: Some(Winner::Player(ann))
        })
    );
}

#[test]
fn leaving_is_reported_with_how_the_player_stood() {
    let (mut game, ann, bo) = playing();
    game.kill(Some(ann), bo);
    events(&mut game);
    assert!(game.leave(ann));
    assert!(!game.leave(ann));
    let left = events(&mut game);
    let [Event::Left(player)] = left.as_slice() else {
        panic!("expected one Left event, got {left:?}");
    };
    assert_eq!((player.name.as_str(), player.kills), ("Ann", 1));
    assert_eq!(game.player(ann), None);
    assert_eq!(game.teams()[0].score, 1, "the team keeps what they scored");
}

#[test]
fn when_everyone_leaves_the_match_starts_over() {
    let (mut game, ann, bo) = playing();
    game.add_team_score(RED, 7);
    game.leave(ann);
    game.leave(bo);
    assert_eq!(game.phase(), Phase::Waiting);
    assert_eq!(game.teams()[0].score, 0);
    let cy = game.join("Cy");
    assert_eq!(game.round(), 1);
    assert_eq!(game.phase(), Phase::Warmup);
    assert_ne!(cy, ann, "numbers are never handed out twice");
    assert_ne!(cy, bo);
}

#[test]
fn new_numbers_apply_at_once_and_keep_the_score() {
    let (mut game, _, _) = playing();
    game.add_team_score(RED, 10);
    let mut rules = teams();
    rules.time_limit = 60.0;
    game.set_rules(rules.clone()).unwrap();
    assert_eq!(game.teams()[0].score, 10);
    assert_eq!(game.time_left(), Some(60.0));

    rules.score_to_win = 10;
    game.set_rules(rules).unwrap();
    assert_eq!(game.phase(), Phase::Over, "Red already has 10");
}

#[test]
fn new_teams_start_the_match_again_with_the_same_players() {
    let (mut game, ann, bo) = playing();
    game.add_team_score(RED, 10);
    events(&mut game);
    let mut rules = teams();
    rules.teams = vec!["Green".to_owned(), "Gold".to_owned(), "Grey".to_owned()];
    game.set_rules(rules).unwrap();
    assert_eq!(game.phase(), Phase::Warmup);
    assert_eq!(game.round(), 1);
    assert_eq!(game.teams().len(), 3);
    assert!(game.teams().iter().all(|team| team.score == 0));
    assert_eq!(game.player(ann).unwrap().team, Some(TeamId(0)));
    assert_eq!(game.player(bo).unwrap().team, Some(TeamId(1)));
    assert_eq!(game.team_named("gold"), Some(TeamId(1)));
}

#[test]
fn rules_that_cannot_be_played_are_refused_and_say_why() {
    let bad = |change: fn(&mut Rules)| {
        let mut rules = teams();
        change(&mut rules);
        Match::new(rules).unwrap_err()
    };
    assert_eq!(
        bad(|r| r.teams = vec!["Red".to_owned()]),
        RulesError::OneTeam
    );
    assert_eq!(
        bad(|r| r.teams = vec!["Red".to_owned(), "red".to_owned()]),
        RulesError::RepeatedTeam {
            name: "red".to_owned()
        }
    );
    assert!(matches!(
        bad(|r| r.teams = (0..9).map(|n| n.to_string()).collect()),
        RulesError::TooManyTeams { count: 9 }
    ));
    assert!(matches!(
        bad(|r| r.teams[1] = "  ".to_owned()),
        RulesError::EmptyName { .. }
    ));
    assert!(matches!(
        bad(|r| r.name = "x".repeat(33)),
        RulesError::LongName { .. }
    ));
    for change in [
        (|r| r.time_limit = -1.0) as fn(&mut Rules),
        |r| r.respawn_delay = f32::NAN,
        |r| r.warmup = f32::INFINITY,
        |r| r.intermission = 1e9,
    ] {
        let error = bad(change);
        assert!(matches!(error, RulesError::BadTime { .. }), "{error}");
    }

    let (mut game, _, _) = playing();
    let mut rules = teams();
    rules.time_limit = -1.0;
    assert!(game.set_rules(rules).is_err());
    assert_eq!(game.rules().time_limit, 600.0, "nothing changed");
}

#[test]
fn the_same_calls_give_the_same_match() {
    let play = || {
        let (mut game, ann, bo) = playing();
        for tick in 0..2000u32 {
            if tick % 97 == 0 {
                game.kill(Some(ann), bo);
            }
            if tick % 131 == 0 {
                game.kill(Some(bo), ann);
            }
            game.tick(STEP);
        }
        (events(&mut game), game.teams().to_vec(), game.time_left())
    };
    assert_eq!(play(), play());
}

fn point(x: f32, team: Option<TeamId>) -> SpawnPoint {
    SpawnPoint {
        position: [x, 0.0, 0.0],
        team,
    }
}

fn enemy(x: f32) -> Stander {
    Stander {
        position: [x, 0.0, 0.0],
        enemy: true,
    }
}

fn friend(x: f32) -> Stander {
    Stander {
        position: [x, 0.0, 0.0],
        enemy: false,
    }
}

#[test]
fn a_map_with_no_spawn_points_has_no_answer() {
    assert_eq!(choose_spawn(&[], None, &[], 1), None);
}

#[test]
fn players_spawn_away_from_enemies() {
    let points = [point(0.0, None), point(10.0, None), point(30.0, None)];
    for seed in 0..50 {
        assert_eq!(
            choose_spawn(&points, None, &[enemy(1.5)], seed),
            Some(2),
            "28.5 m from the enemy beats 8.5 m and 1.5 m"
        );
    }
}

#[test]
fn players_spawn_on_their_own_teams_points() {
    let points = [
        point(0.0, Some(RED)),
        point(50.0, Some(BLUE)),
        point(100.0, Some(BLUE)),
    ];
    for seed in 0..50 {
        // Red's only point, though an enemy is nearer to it than to Blue's.
        assert_eq!(
            choose_spawn(&points, Some(RED), &[enemy(5.0)], seed),
            Some(0)
        );
    }
    // A map with no points for this team still spawns them somewhere.
    assert!(choose_spawn(&points, Some(TeamId(2)), &[], 7).is_some());
}

#[test]
fn nobody_spawns_inside_another_player_while_a_point_is_free() {
    let points = [point(0.0, None), point(2.0, None)];
    for seed in 0..50 {
        assert_eq!(choose_spawn(&points, None, &[friend(0.2)], seed), Some(1));
    }
    // With every point taken, one is still chosen.
    assert!(choose_spawn(&points, None, &[friend(0.2), friend(2.2)], 3).is_some());
}

#[test]
fn equally_good_points_take_turns() {
    let points: Vec<_> = (0..6).map(|n| point(n as f32 * 100.0, None)).collect();
    let mut used = [false; 6];
    for seed in 0..200 {
        let index = choose_spawn(&points, None, &[], seed).unwrap();
        assert_eq!(choose_spawn(&points, None, &[], seed), Some(index));
        used[index] = true;
    }
    assert_eq!(used, [true; 6]);
}
