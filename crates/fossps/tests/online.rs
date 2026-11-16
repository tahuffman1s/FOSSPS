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
    // Nothing these tests fetch is kept on the computer they run on.
    let cache = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("online-cache-unused");
    let mut joined = JoinedGame::connect(address, &ActionMap::default_fps(), TICK_RATE)
        .unwrap()
        .with_cache(cache);
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
    let mut game = LocalGame::start(new_game("alone"), None, &actions, TICK_RATE).unwrap();
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

#[test]
fn a_served_game_is_found_by_name_without_knowing_its_address() {
    use fossps::online::find_servers;

    let (hosted, address) = host("found");
    let mut hosted = hosted.named("zombie-night");
    let port = hosted.address().unwrap().port();
    let nothing = Input::new(ActionMap::default_fps());

    // The server answers from its tick, so it ticks while the search waits.
    let searching =
        std::thread::spawn(move || find_servers(port, Duration::from_millis(600)).unwrap());
    while !searching.is_finished() {
        hosted.tick(nothing.frame(), STEP);
        std::thread::sleep(Duration::from_millis(2));
    }
    let found = searching.join().unwrap();

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].address.to_string(), address);
    assert_eq!(found[0].name, "zombie-night");
    assert_eq!((found[0].players, found[0].places), (0, 64));

    // And what was found can be joined as it is.
    let joined = join(&mut hosted, &found[0].address.to_string());
    assert!(joined.id().is_some());
}

#[test]
fn an_open_game_is_played_here_and_joined_by_someone_else() {
    use fossps::online::find_servers;

    let port = std::net::UdpSocket::bind("0.0.0.0:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let actions = ActionMap::default_fps();
    let mut local = LocalGame::open(
        new_game("open"),
        None,
        "open-game",
        port,
        &actions,
        TICK_RATE,
    )
    .unwrap();
    assert_eq!(local.hosted().players().count(), 1);
    let nothing = Input::new(actions.clone());

    let searching =
        std::thread::spawn(move || find_servers(port, Duration::from_millis(600)).unwrap());
    while !searching.is_finished() {
        local.tick(nothing.frame(), STEP);
        std::thread::sleep(Duration::from_millis(2));
    }
    let found = searching.join().unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].name.as_str(), found[0].players), ("open-game", 1));

    let mut other =
        JoinedGame::connect(&found[0].address.to_string(), &actions, TICK_RATE).unwrap();
    other
        .wait_until_joined(|| local.tick(nothing.frame(), STEP))
        .unwrap();
    assert_eq!(local.hosted().players().count(), 2);
}

/// The level `fossps new` writes: a 32 m floor with a wall around it at
/// 16 m, and three places to start at each end, 14 m from the middle.
fn starter_level() -> fossps::assets::Scene {
    fossps::level::from_bytes("level.glb", scaffold::STARTER_LEVEL).unwrap()
}

#[test]
fn a_game_with_a_level_starts_at_a_spawn_point_and_its_walls_are_solid() {
    let actions = ActionMap::default_fps();
    let level = starter_level();
    let mut game = LocalGame::start(new_game("level"), Some(&level), &actions, TICK_RATE).unwrap();
    let mut input = Input::new(actions);
    let play = |game: &mut LocalGame, input: &mut Input, ticks: u32| {
        for _ in 0..ticks {
            game.tick(input.tick(), STEP);
            std::thread::sleep(Duration::from_millis(1));
        }
    };

    // Settled at one of the six spawn points, looking at the other end.
    play(&mut game, &mut input, 30);
    let start = game.joined().player().position;
    assert!(start[1].abs() < 0.02, "on the floor: {start:?}");
    assert!((start[2].abs() - 14.0).abs() < 0.01, "{start:?}");
    assert!([0.0, 6.0].contains(&start[0].abs().round()), "{start:?}");
    let yaw = game.joined().yaw();
    let faces_south = (yaw - std::f32::consts::PI).abs() < 0.001;
    assert!(
        if start[2] < 0.0 {
            faces_south
        } else {
            yaw == 0.0
        },
        "at {start:?} facing {yaw}"
    );

    // Backing away from the middle, the wall around the level is 2 m behind.
    // Two seconds of walking would be 9 m on a flat floor.
    input.press(Key::S);
    play(&mut game, &mut input, 120);
    input.release(Key::S);
    play(&mut game, &mut input, 60);

    let here = game.joined().player().position;
    let out = here[2].abs();
    assert!(out > 15.5 && out < 15.7, "stopped by the wall: {here:?}");
    assert!((here[0] - start[0]).abs() < 0.01, "{here:?}");
    assert!(here[1].abs() < 0.02, "on the floor: {here:?}");
    let (_, on_server) = game.hosted().players().next().unwrap();
    assert!(
        distance(on_server.position, here) < 0.001,
        "server {:?}, player {here:?}",
        on_server.position
    );
}

#[test]
fn players_who_join_a_level_start_apart_and_one_without_it_is_put_right() {
    let level = starter_level();
    let hosted = HostedGame::open(new_game("apart"), "127.0.0.1:0", TICK_RATE).unwrap();
    let mut hosted = hosted.with_level(&level);
    let address = hosted.address().unwrap().to_string();
    // The first has the level; the second joined with no copy of the game.
    let nothing = Input::new(ActionMap::default_fps());
    let mut first = JoinedGame::connect(&address, &ActionMap::default_fps(), TICK_RATE)
        .unwrap()
        .with_level(&level);
    first
        .wait_until_joined(|| hosted.tick(nothing.frame(), STEP))
        .unwrap();
    let mut second = join(&mut hosted, &address);
    let (mut still, mut walking) = (
        Input::new(ActionMap::default_fps()),
        Input::new(ActionMap::default_fps()),
    );
    play(
        &mut hosted,
        &mut [(&mut first, &mut still), (&mut second, &mut walking)],
        30,
    );

    // The second player is put at the other end from the first.
    let (a, b) = (first.player().position, second.player().position);
    assert!((a[2].abs() - 14.0).abs() < 0.01, "{a:?}");
    assert!((b[2] + a[2]).abs() < 0.01, "{a:?} and {b:?}");

    // Without the level the second player looks north and thinks the way
    // is clear; the server knows where the walls are and has the last word.
    walking.press(if b[2] < 0.0 { Key::W } else { Key::S });
    play(
        &mut hosted,
        &mut [(&mut first, &mut still), (&mut second, &mut walking)],
        150,
    );
    walking.release(Key::W);
    walking.release(Key::S);
    play(
        &mut hosted,
        &mut [(&mut first, &mut still), (&mut second, &mut walking)],
        60,
    );
    let here = second.player().position;
    assert!(here[2].abs() < 16.0, "kept inside the wall: {here:?}");
    let on_server = hosted
        .players()
        .find(|(id, _)| Some(*id) == second.id())
        .map(|(_, player)| player.position)
        .unwrap();
    assert!(
        distance(on_server, here) < 0.05,
        "server {on_server:?}, player {here:?}"
    );
}

/// A map file with one spawn point at `position`.
fn map_with_spawn(position: [f32; 3]) -> String {
    let [x, y, z] = position;
    format!(
        "version = 1\n\n[[entity]]\nid = \"only\"\ntype = \"spawn\"\n\
         position = [{x:?}, {y:?}, {z:?}]\n"
    )
}

#[test]
fn players_start_at_the_maps_spawn_point_and_a_saved_map_is_picked_up() {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("online-map");
    let _ = std::fs::remove_dir_all(&folder);
    scaffold::new_game(&folder).unwrap();
    let map_file = folder.join(fossps::map::MAP_FILE);
    let first = [10.0, 0.0, -6.0];
    std::fs::write(&map_file, map_with_spawn(first)).unwrap();

    let game = ScriptGame::open(&folder, &ActionMap::default_fps()).unwrap();
    assert_eq!(game.map().spawns().count(), 1);
    let mut hosted = HostedGame::open(game, "127.0.0.1:0", TICK_RATE).unwrap();
    let address = hosted.address().unwrap().to_string();

    // The server puts the player at the spawn point, and the player's own
    // side, which started in the middle, is brought there.
    let mut one = join(&mut hosted, &address);
    let mut still = Input::new(ActionMap::default_fps());
    play(&mut hosted, &mut [(&mut one, &mut still)], 30);
    let (_, on_server) = hosted.players().next().unwrap();
    assert!(distance(on_server.position, first) < 0.01, "{on_server:?}");
    assert!(
        distance(one.player().position, first) < 0.01,
        "{:?}",
        one.player().position
    );

    // The map is saved with the spawn point somewhere else while the game
    // runs. The game notices, and the next player comes in there.
    let second = [-20.0, 0.0, 8.0];
    std::fs::write(&map_file, map_with_spawn(second)).unwrap();
    let moved = |hosted: &HostedGame| {
        hosted
            .game()
            .map()
            .spawns()
            .any(|spawn| spawn.position == second)
    };
    for _ in 0..2000 {
        if moved(&hosted) {
            break;
        }
        play(&mut hosted, &mut [(&mut one, &mut still)], 1);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(moved(&hosted), "the saved map was never picked up");
    let mut two = join(&mut hosted, &address);
    let mut also_still = Input::new(ActionMap::default_fps());
    play(
        &mut hosted,
        &mut [(&mut one, &mut still), (&mut two, &mut also_still)],
        30,
    );
    let id = two.id().unwrap();
    let (_, on_server) = hosted.players().find(|(other, _)| *other == id).unwrap();
    assert!(distance(on_server.position, second) < 0.01, "{on_server:?}");
    // Whoever was already playing stays where they are.
    assert!(distance(one.player().position, first) < 0.01);

    // A save with a mistake in it changes nothing: the map that worked stays.
    std::fs::write(&map_file, "version = 1\n[[entity]]\nid = 3\n").unwrap();
    for _ in 0..2000 {
        if hosted.game().map_error().is_some() {
            break;
        }
        play(&mut hosted, &mut [], 1);
        std::thread::sleep(Duration::from_millis(5));
    }
    let said = hosted.game().map_error().unwrap().to_string();
    assert!(said.contains("line 3: An id has to be text"), "{said}");
    assert!(moved(&hosted));
}
/// An empty folder for levels fetched in `test` to be kept in.
fn cache_for(test: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("online-cache-{test}"));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

#[test]
fn a_player_with_no_copy_of_the_game_is_sent_its_level_and_walks_it() {
    let level = starter_level();
    let mut hosted = HostedGame::open(new_game("sent"), "127.0.0.1:0", TICK_RATE)
        .unwrap()
        .with_level(&level)
        .share_level(scaffold::STARTER_LEVEL)
        .unwrap();
    let address = hosted.address().unwrap().to_string();
    let cache = cache_for("sent");
    let nothing = Input::new(ActionMap::default_fps());

    // All this player has is the server's address.
    let mut joined = JoinedGame::connect(&address, &ActionMap::default_fps(), TICK_RATE)
        .unwrap()
        .with_cache(&cache);
    joined
        .wait_until_joined(|| hosted.tick(nothing.frame(), STEP))
        .unwrap();
    // The level to draw is the server's, and it is kept for next time.
    let sent = joined.take_level().expect("the server's level");
    assert_eq!(sent.instances.len(), level.instances.len());
    assert!(joined.take_level().is_none());
    let kept: Vec<_> = std::fs::read_dir(&cache).unwrap().flatten().collect();
    assert_eq!(kept.len(), 1);
    let path = kept[0].path();
    assert_eq!(std::fs::read(&path).unwrap(), scaffold::STARTER_LEVEL);

    // Started at a spawn point of the level, looking at the other end.
    let mut input = Input::new(ActionMap::default_fps());
    play(&mut hosted, &mut [(&mut joined, &mut input)], 30);
    let start = joined.player().position;
    assert!((start[2].abs() - 14.0).abs() < 0.01, "{start:?}");
    let yaw = joined.yaw();
    assert!(
        if start[2] < 0.0 {
            (yaw - std::f32::consts::PI).abs() < 0.001
        } else {
            yaw == 0.0
        },
        "at {start:?} facing {yaw}"
    );

    // Backing into the wall 2 m behind, the player stops at it on their
    // own side, where the server has them too: nothing to put right.
    input.press(Key::S);
    play(&mut hosted, &mut [(&mut joined, &mut input)], 120);
    input.release(Key::S);
    play(&mut hosted, &mut [(&mut joined, &mut input)], 60);
    let here = joined.player().position;
    let out = here[2].abs();
    assert!(out > 15.5 && out < 15.7, "stopped by the wall: {here:?}");
    let (_, on_server) = hosted.players().next().unwrap();
    assert!(
        distance(on_server.position, here) < 0.001,
        "server {:?}, player {here:?}",
        on_server.position
    );
    drop(joined);
    play(&mut hosted, &mut [], 5);

    // The next visit uses the kept file and leaves it as it is.
    let written = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut again = JoinedGame::connect(&address, &ActionMap::default_fps(), TICK_RATE)
        .unwrap()
        .with_cache(&cache);
    again
        .wait_until_joined(|| hosted.tick(nothing.frame(), STEP))
        .unwrap();
    assert!(again.take_level().is_some());
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
    assert!(std::fs::metadata(&path).unwrap().modified().unwrap() >= written);
    drop(again);
    play(&mut hosted, &mut [], 5);

    // A kept file that is no longer what the server offers is not
    // trusted: the level is fetched afresh.
    let mut changed = scaffold::STARTER_LEVEL.to_vec();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    let mut afresh = JoinedGame::connect(&address, &ActionMap::default_fps(), TICK_RATE)
        .unwrap()
        .with_cache(&cache);
    afresh
        .wait_until_joined(|| hosted.tick(nothing.frame(), STEP))
        .unwrap();
    assert!(afresh.take_level().is_some());
    assert_eq!(std::fs::read(&path).unwrap(), scaffold::STARTER_LEVEL);
}

#[test]
fn a_player_is_sent_the_map_and_faces_the_way_its_spawn_point_does() {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("online-sent-map");
    let _ = std::fs::remove_dir_all(&folder);
    scaffold::new_game(&folder).unwrap();
    let at = [3.0, 0.0, -2.0];
    let map = format!("{}facing = 90.0\n", map_with_spawn(at));
    std::fs::write(folder.join(fossps::map::MAP_FILE), map).unwrap();
    let game = ScriptGame::open(&folder, &ActionMap::default_fps()).unwrap();
    assert_eq!(game.map().spawns().count(), 1);
    let mut hosted = HostedGame::open(game, "127.0.0.1:0", TICK_RATE).unwrap();
    let address = hosted.address().unwrap().to_string();

    let mut joined = join(&mut hosted, &address);
    let mut still = Input::new(ActionMap::default_fps());
    play(&mut hosted, &mut [(&mut joined, &mut still)], 30);
    let here = joined.player().position;
    let (_, on_server) = hosted.players().next().unwrap();
    // Dropped on the floor, a player settles within a centimetre or two.
    assert!(distance(here, at) < 0.05, "{here:?}");
    assert!(distance(here, on_server.position) < 0.001, "{here:?}");
    let yaw = joined.yaw();
    assert!(
        (yaw - std::f32::consts::FRAC_PI_2).abs() < 0.001,
        "facing {yaw}"
    );
}

#[test]
fn a_server_whose_level_cannot_be_read_is_not_joined() {
    let mut hosted = HostedGame::open(new_game("unreadable"), "127.0.0.1:0", TICK_RATE)
        .unwrap()
        .share_level(b"this is not a level".to_vec())
        .unwrap();
    let address = hosted.address().unwrap().to_string();
    let cache = cache_for("unreadable");
    let nothing = Input::new(ActionMap::default_fps());
    let mut joined = JoinedGame::connect(&address, &ActionMap::default_fps(), TICK_RATE)
        .unwrap()
        .with_cache(&cache);
    let error = joined
        .wait_until_joined(|| hosted.tick(nothing.frame(), STEP))
        .unwrap_err();
    let said = error.to_string();
    assert!(
        said.starts_with(&format!(
            "Could not join {address}: the level it plays could not"
        )),
        "{said}"
    );
    assert!(said.contains("tell whoever runs the server"), "{said}");
    // No place was taken, and nothing unreadable was kept.
    for _ in 0..10 {
        hosted.tick(nothing.frame(), STEP);
    }
    assert_eq!(hosted.players().count(), 0);
    assert!(!cache.exists());
    assert!(joined.take_level().is_none());
}

#[test]
fn a_level_file_with_nothing_in_it_is_not_shared_and_says_why() {
    let hosted = HostedGame::open(new_game("empty-level"), "127.0.0.1:0", TICK_RATE).unwrap();
    let error = hosted.share_level(Vec::new()).unwrap_err();
    assert!(error.to_string().contains("empty"), "{error}");
}
