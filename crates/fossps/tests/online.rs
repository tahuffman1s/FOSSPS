//! A server and players joined to it over this machine's own network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use fossps::input::{ActionMap, Input, Key};
use fossps::online::{HostedGame, JoinedGame, LocalGame};
use fossps::{Game, ScriptGame, scaffold};

const TICK_RATE: u32 = 60;
const STEP: f32 = 1.0 / 60.0;

/// A new game in a folder of its own.
fn new_game(test: &str) -> ScriptGame {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("online-{test}"));
    let _ = std::fs::remove_dir_all(&folder);
    scaffold::new_game(&folder).unwrap();
    ScriptGame::open(&folder, &ActionMap::default_fps()).unwrap()
}

/// A server for a new game on a port of the system's choosing, and the
/// address to join it at.
fn host(test: &str) -> (HostedGame, String) {
    let hosted = HostedGame::open(new_game(test), "127.0.0.1:0", TICK_RATE).unwrap();
    let address = hosted.address().unwrap().to_string();
    (hosted, address)
}

fn join(hosted: &mut HostedGame, address: &str) -> JoinedGame {
    let nothing = Input::new(ActionMap::default_fps());
    let mut joined = JoinedGame::connect(address, &ActionMap::default_fps(), TICK_RATE).unwrap();
    joined
        .wait_until_joined(|| hosted.tick(nothing.frame(), STEP))
        .unwrap();
    joined
}

/// Runs `ticks` ticks of the server and each player, with each player's
/// own keys. A short wait lets this machine carry the packets across.
fn play(hosted: &mut HostedGame, players: &mut [(&mut JoinedGame, &mut Input)], ticks: u32) {
    let nothing = Input::new(ActionMap::default_fps());
    for _ in 0..ticks {
        for (joined, input) in players.iter_mut() {
            joined.tick(input.tick(), STEP);
        }
        std::thread::sleep(Duration::from_millis(1));
        hosted.tick(nothing.frame(), STEP);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

#[test]
fn a_joined_player_walks_at_once_and_the_server_agrees() {
    let (mut hosted, address) = host("walk");
    let mut joined = join(&mut hosted, &address);
    let mut input = Input::new(ActionMap::default_fps());
    assert_eq!(hosted.game().host().mode().players().len(), 1);

    // Settle onto the floor, then hold W.
    play(&mut hosted, &mut [(&mut joined, &mut input)], 30);
    let before = joined.player().position;
    input.press(Key::W);
    joined.tick(input.tick(), STEP);
    assert!(
        joined.player().position[2] < before[2],
        "the player's own side moves them in the tick the key goes down"
    );

    play(&mut hosted, &mut [(&mut joined, &mut input)], 60);
    input.release(Key::W);
    play(&mut hosted, &mut [(&mut joined, &mut input)], 60);

    let here = joined.player().position;
    assert!(here[2] < -2.0, "walked forward, along -Z: {here:?}");
    assert!(here[1].abs() < 0.01, "on the floor: {here:?}");
    let (_, on_server) = hosted.players().next().unwrap();
    assert!(
        distance(on_server.position, here) < 0.001,
        "server {:?}, player {here:?}",
        on_server.position
    );
    assert!(joined.closed().is_none());
}

#[test]
fn players_see_each_other_and_the_script_hears_them_come_and_go() {
    let (mut hosted, address) = host("two");
    let mut first = join(&mut hosted, &address);
    let mut second = join(&mut hosted, &address);
    let (mut still, mut walking) = (
        Input::new(ActionMap::default_fps()),
        Input::new(ActionMap::default_fps()),
    );
    assert_ne!(first.id(), second.id());
    assert_eq!(hosted.game().host().mode().players().len(), 2);

    walking.press(Key::D);
    play(
        &mut hosted,
        &mut [(&mut first, &mut still), (&mut second, &mut walking)],
        90,
    );

    let seen: Vec<_> = first.others().collect();
    assert_eq!(seen.len(), 1);
    assert_eq!(Some(seen[0].id), second.id());
    assert!(
        distance(seen[0].position, second.player().position) < 1.0,
        "the first player sees the second near where they are: {:?} and {:?}",
        seen[0].position,
        second.player().position
    );
    assert!(seen[0].position[0] > 1.0, "{:?}", seen[0].position);

    drop(second);
    play(&mut hosted, &mut [(&mut first, &mut still)], 10);
    assert_eq!(hosted.players().count(), 1);
    assert_eq!(hosted.game().host().mode().players().len(), 1);
    assert_eq!(first.others().count(), 0);
}

#[test]
fn a_player_is_told_when_the_server_stops() {
    let (mut hosted, address) = host("stop");
    let mut joined = join(&mut hosted, &address);
    let mut input = Input::new(ActionMap::default_fps());

    drop(hosted);
    for _ in 0..20 {
        joined.tick(input.tick(), STEP);
        std::thread::sleep(Duration::from_millis(1));
    }

    let said = joined.closed().unwrap().to_string();
    assert_eq!(
        said,
        format!("The game at {address} ended: the server was shut down.")
    );
}

#[test]
fn a_game_played_alone_goes_through_the_same_netcode() {
    let actions = ActionMap::default_fps();
    let mut game = LocalGame::start(new_game("alone"), &actions, TICK_RATE).unwrap();
    let mut input = Input::new(actions);

    input.press(Key::W);
    for _ in 0..120 {
        game.tick(input.tick(), STEP);
        std::thread::sleep(Duration::from_millis(1));
    }

    let here = game.joined().player().position;
    assert!(here[2] < -3.0, "{here:?}");
    assert_eq!(game.hosted().players().count(), 1);
    assert_eq!(game.hosted().game().host().mode().players().len(), 1);
    assert!(game.joined().closed().is_none());
}
