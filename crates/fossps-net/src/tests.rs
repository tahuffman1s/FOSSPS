//! A server and its clients talking over a connection built by hand, which
//! can lose packets, hold them back, repeat them and mix up their order.

use fossps_physics::{Level, MoveInput, MoveSettings, Player};

use crate::{
    Client, ClientSettings, Command, MAX_PACKET_BYTES, MAX_PLAYERS, NetError, PlayerId,
    PlayerInput, Received, Server, ServerSettings,
};

const STEP: f32 = 1.0 / 60.0;

type Triangle = [[f32; 3]; 3];

fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> [Triangle; 2] {
    [[a, b, c], [a, c, d]]
}

/// A floor 400 m across, a wall along the right at x = 6 and a stair up
/// to a platform 15 m ahead.
fn arena() -> Level {
    let s = 200.0;
    let mut triangles = Vec::new();
    triangles.extend(quad([-s, 0.0, s], [s, 0.0, s], [s, 0.0, -s], [-s, 0.0, -s]));
    triangles.extend(quad(
        [6.0, 0.0, s],
        [6.0, 0.0, -s],
        [6.0, 3.0, -s],
        [6.0, 3.0, s],
    ));
    triangles.extend(quad(
        [-s, 0.0, -15.0],
        [s, 0.0, -15.0],
        [s, 0.3, -15.0],
        [-s, 0.3, -15.0],
    ));
    triangles.extend(quad(
        [-s, 0.3, -15.0],
        [s, 0.3, -15.0],
        [s, 0.3, -s],
        [-s, 0.3, -s],
    ));
    Level::from_triangles(&triangles)
}

fn spawn(x: f32) -> Player {
    Player::at([x, 0.5, 0.0], &MoveSettings::default())
}

fn walking(forward: f32) -> PlayerInput {
    PlayerInput {
        movement: MoveInput {
            forward,
            ..MoveInput::default()
        },
        ..PlayerInput::default()
    }
}

/// Input that changes with the tick: turning, strafing into the wall,
/// jumping, crouching and sprinting.
fn busy(tick: u32) -> PlayerInput {
    PlayerInput {
        movement: MoveInput {
            forward: 1.0,
            right: if tick % 120 < 60 { 0.7 } else { -0.4 },
            yaw: (tick as f32 * 0.013).sin() * 0.8,
            jump: tick.is_multiple_of(45),
            crouch: tick % 200 > 170,
            sprint: tick % 90 < 50,
        },
        pitch: (tick as f32 * 0.02).sin() * 0.5,
        fire: tick.is_multiple_of(7),
        ..PlayerInput::default()
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

/// How bad a connection is.
#[derive(Clone, Copy)]
struct Weather {
    /// Packets lost, out of 100.
    lost: u32,
    /// Packets sent twice, out of 100.
    repeated: u32,
    /// The least and most ticks a packet takes. A spread mixes up the order.
    delay: (u32, u32),
}

const CLEAR: Weather = Weather {
    lost: 0,
    repeated: 0,
    delay: (0, 0),
};

const STORM: Weather = Weather {
    lost: 20,
    repeated: 10,
    delay: (2, 6),
};

/// One direction of a connection.
struct Link {
    weather: Weather,
    seed: u64,
    /// Packets on their way, with the tick each arrives.
    flying: Vec<(u32, Vec<u8>)>,
}

impl Link {
    fn new(weather: Weather, seed: u64) -> Self {
        Self {
            weather,
            seed: seed | 1,
            flying: Vec::new(),
        }
    }

    fn random(&mut self, below: u32) -> u32 {
        // xorshift64
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 33) as u32 % below.max(1)
    }

    fn send(&mut self, now: u32, packet: &[u8]) {
        assert!(packet.len() <= MAX_PACKET_BYTES, "{} bytes", packet.len());
        let copies = if self.random(100) < self.weather.lost {
            0
        } else if self.random(100) < self.weather.repeated {
            2
        } else {
            1
        };
        for _ in 0..copies {
            let (least, most) = self.weather.delay;
            let delay = least + self.random(most - least + 1);
            self.flying.push((now + delay, packet.to_vec()));
        }
    }

    /// The packets that arrive at `now`.
    fn arrive(&mut self, now: u32) -> Vec<Vec<u8>> {
        let (here, flying) = std::mem::take(&mut self.flying)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.flying = flying;
        here.into_iter().map(|(_, packet)| packet).collect()
    }
}

/// One player's machine and its connection.
struct Seat {
    id: PlayerId,
    client: Client,
    up: Link,
    down: Link,
    /// What `receive` said, newest last.
    results: Vec<Received>,
}

/// A server, a level and the players' machines.
struct Match {
    level: Level,
    rules: MoveSettings,
    server: Server,
    seats: Vec<Seat>,
    now: u32,
    /// Every snapshot written, for comparing two runs.
    sent: Vec<Vec<u8>>,
}

impl Match {
    fn new(players: usize, weather: Weather) -> Self {
        let mut server = Server::new(STEP, ServerSettings::default());
        let seats = (0..players)
            .map(|seat| {
                let player = spawn(seat as f32 * -2.0);
                Seat {
                    id: server.join(player).unwrap(),
                    client: Client::new(player, STEP, ClientSettings::default()),
                    up: Link::new(weather, 11 + seat as u64 * 7919),
                    down: Link::new(weather, 23 + seat as u64 * 104_729),
                    results: Vec::new(),
                }
            })
            .collect();
        Self {
            level: arena(),
            rules: MoveSettings::default(),
            server,
            seats,
            now: 0,
            sent: Vec::new(),
        }
    }

    fn weather(&mut self, weather: Weather) {
        for seat in &mut self.seats {
            seat.up.weather = weather;
            seat.down.weather = weather;
        }
    }

    /// One tick everywhere, with each seat's input from `input`.
    fn tick(&mut self, input: impl Fn(usize, u32) -> PlayerInput) {
        self.now += 1;
        let mut packet = Vec::new();
        for (number, seat) in self.seats.iter_mut().enumerate() {
            for snapshot in seat.down.arrive(self.now) {
                let result = seat
                    .client
                    .receive(&self.level, &self.rules, &snapshot)
                    .unwrap();
                seat.results.push(result);
            }
            let input = input(number, self.now);
            seat.client
                .predict(&self.level, &self.rules, &input, &mut packet);
            seat.up.send(self.now, &packet);
        }
        for seat in &mut self.seats {
            for commands in seat.up.arrive(self.now) {
                self.server.receive(seat.id, &commands).unwrap();
            }
        }
        self.server.tick(&self.level, &self.rules);
        for seat in &mut self.seats {
            self.server.write_snapshot(seat.id, &mut packet).unwrap();
            seat.down.send(self.now, &packet);
            self.sent.push(packet.clone());
        }
    }

    fn run(&mut self, ticks: u32, input: impl Fn(usize, u32) -> PlayerInput) {
        for _ in 0..ticks {
            self.tick(&input);
        }
    }
}

fn corrections(results: &[Received]) -> usize {
    results
        .iter()
        .filter(|result| matches!(result, Received::Corrected { .. }))
        .count()
}

#[test]
fn a_command_keeps_the_input_to_within_its_rounding() {
    let input = PlayerInput {
        movement: MoveInput {
            forward: 0.5,
            right: -1.0,
            yaw: 1.234,
            jump: true,
            crouch: false,
            sprint: true,
        },
        pitch: -0.7,
        fire: true,
        aim: false,
        reload: true,
    };
    let command = Command::new(9, &input);
    let back = command.input();
    assert_eq!(command.sequence(), 9);
    assert!((back.movement.forward - 0.5).abs() < 0.005);
    assert_eq!(back.movement.right, -1.0);
    assert!((back.movement.yaw - 1.234).abs() < 0.0001);
    assert!((back.pitch + 0.7).abs() < 0.0001);
    assert_eq!(
        (
            back.movement.jump,
            back.movement.crouch,
            back.movement.sprint
        ),
        (true, false, true)
    );
    assert_eq!((back.fire, back.aim, back.reload), (true, false, true));
    // Rounding again changes nothing: both sides run the very same input.
    assert_eq!(Command::new(9, &back), command);
    assert_eq!(command.movement(), back.movement);
}

#[test]
fn a_command_tames_wild_input() {
    let turn = std::f32::consts::TAU;
    let wild = PlayerInput {
        movement: MoveInput {
            forward: 40.0,
            right: f32::NAN,
            yaw: -0.5 - 3.0 * turn,
            ..MoveInput::default()
        },
        pitch: f32::INFINITY,
        ..PlayerInput::default()
    };
    let back = Command::new(1, &wild).input();
    assert_eq!(back.movement.forward, 1.0);
    assert_eq!(back.movement.right, 0.0);
    // A yaw of several turns comes back as the same direction in one turn.
    assert!((back.movement.yaw - (turn - 0.5)).abs() < 0.001);
    assert_eq!(back.pitch, 0.0);
    let up = PlayerInput {
        pitch: 9.0,
        ..PlayerInput::default()
    };
    let limit = std::f32::consts::FRAC_PI_2;
    assert!((Command::new(1, &up).input().pitch - limit).abs() < 1e-6);
}

#[test]
fn on_a_clear_connection_the_server_agrees_with_every_prediction() {
    let mut game = Match::new(1, CLEAR);
    game.run(600, |_, tick| busy(tick));
    let seat = &game.seats[0];
    // The first snapshots correct the client while the server fills its
    // buffer of commands; after that the two never differ.
    // A snapshot written in one tick is read in the next.
    assert_eq!(seat.results.len(), 599);
    assert_eq!(
        corrections(&seat.results[10..]),
        0,
        "{:?}",
        &seat.results[..12]
    );
    assert!(corrections(&seat.results[..10]) > 0);
    // Went somewhere: up the stair and against the wall.
    let player = seat.client.player();
    assert!(player.position[2] < -15.0, "{player:?}");
    assert!(player.position[1] > 0.25, "{player:?}");
    assert_eq!(seat.client.id(), Some(seat.id));
    assert_eq!(seat.client.server_tick(), 599);
    assert_eq!(game.server.tick_number(), 600);
}

#[test]
fn the_player_moves_without_waiting_for_the_server() {
    let slow = Weather {
        delay: (6, 6),
        ..CLEAR
    };
    let mut game = Match::new(1, slow);
    game.run(6, |_, _| walking(1.0));
    // No snapshot has come back yet, and the player has already moved.
    assert!(game.seats[0].results.is_empty());
    assert!(game.seats[0].client.player().position[2] < -0.05);
    game.run(294, |_, _| walking(1.0));
    let seat = &game.seats[0];
    let ahead = game.server.player(seat.id).unwrap().position[2] - seat.client.player().position[2];
    // A command is 6 ticks on its way and waits 1 on the server, at 4.5 m/s.
    let rules = MoveSettings::default();
    let expected = 7.0 * STEP * rules.walk_speed;
    assert!(
        (ahead - expected).abs() < 0.2,
        "{ahead} ahead, not {expected}"
    );
    assert_eq!(corrections(&seat.results[20..]), 0);
}

#[test]
fn on_a_bad_connection_the_client_ends_up_where_the_server_is() {
    let mut game = Match::new(2, STORM);
    game.run(900, |seat, tick| busy(tick + seat as u32 * 31));
    // Let go of everything and let the connection settle.
    game.weather(CLEAR);
    game.run(120, |_, _| PlayerInput::default());
    for seat in &game.seats {
        assert_eq!(seat.results.last(), Some(&Received::Matched));
        let server = game.server.player(seat.id).unwrap();
        assert_eq!(seat.client.player(), server);
        assert_eq!(seat.client.view_position(), server.position);
        // Three commands a packet ride out most of a 20% loss: far fewer
        // corrections than snapshots.
        let corrected = corrections(&seat.results);
        assert!(
            corrected * 4 < seat.results.len(),
            "{corrected} corrections"
        );
        assert!(seat.client.player().position[2] < -15.0);
    }
}

#[test]
fn the_same_packets_give_the_same_match() {
    let run = || {
        let mut game = Match::new(3, STORM);
        game.run(400, |seat, tick| busy(tick * (seat as u32 + 1)));
        let players: Vec<Player> = game.seats.iter().map(|s| *s.client.player()).collect();
        (game.sent, players)
    };
    let (first, second) = (run(), run());
    assert!(first.0 == second.0, "the snapshots differ");
    assert_eq!(first.1, second.1);
}

#[test]
fn a_small_correction_is_glided_over_and_a_big_one_is_shown_at_once() {
    let mut game = Match::new(1, CLEAR);
    game.run(60, |_, _| walking(1.0));
    let id = game.seats[0].id;

    // An explosion pushes the player half a metre sideways on the server.
    game.server.player_mut(id).unwrap().position[0] += 0.5;
    let shown = game.seats[0].client.view_position();
    game.run(2, |_, _| walking(1.0));
    let seat = &game.seats[0];
    let Some(Received::Corrected { distance: pushed }) = seat.results.last().copied() else {
        panic!("not corrected: {:?}", seat.results.last());
    };
    assert!((pushed - 0.5).abs() < 0.01, "{pushed}");
    let server = *game.server.player(id).unwrap();
    // The client is a tick of walking ahead, but level with the push.
    assert!((seat.client.player().position[0] - server.position[0]).abs() < 1e-4);
    // On screen the player has moved two ticks' walk and a little of the push.
    let jump = distance_xz(seat.client.view_position(), shown);
    assert!(jump < 0.25, "moved {jump} on screen in two ticks");
    assert!((seat.client.view_position()[0] - shown[0]).abs() < 0.1);

    game.run(60, |_, _| walking(1.0));
    let seat = &game.seats[0];
    let left = distance(seat.client.view_position(), seat.client.player().position);
    assert!(left < 0.001, "{left} m of the correction still showing");
    assert_eq!(seat.results.last(), Some(&Received::Matched));

    // A respawn 50 m away is not glided over.
    game.server.player_mut(id).unwrap().position[0] -= 50.0;
    game.run(2, |_, _| walking(1.0));
    let seat = &game.seats[0];
    assert!(
        matches!(seat.results.last(), Some(Received::Corrected { distance }) if *distance > 49.0)
    );
    assert_eq!(seat.client.view_position(), seat.client.player().position);
}

fn distance_xz(a: [f32; 3], b: [f32; 3]) -> f32 {
    distance([a[0], 0.0, a[2]], [b[0], 0.0, b[2]])
}

#[test]
fn sending_commands_faster_does_not_move_a_player_faster() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let mut client = Client::new(start, STEP, ClientSettings::default());
    let mut packet = Vec::new();
    for _ in 0..120 {
        // Five commands for every server tick.
        for _ in 0..5 {
            client.predict(&level, &rules, &walking(1.0), &mut packet);
            server.receive(id, &packet).unwrap();
        }
        server.tick(&level, &rules);
    }
    let walked = -server.player(id).unwrap().position[2];
    let fastest = rules.walk_speed * 2.0;
    assert!(walked <= fastest + 0.01, "{walked} m in two seconds");
    assert!(walked > fastest - 1.5, "{walked} m in two seconds");
}

#[test]
fn a_burst_of_late_commands_does_not_leave_the_player_behind_for_good() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let settings = ClientSettings {
        commands_per_packet: 8,
        ..ClientSettings::default()
    };
    let mut client = Client::new(start, STEP, settings);
    let mut packet = Vec::new();
    let firing = PlayerInput {
        fire: true,
        ..walking(1.0)
    };
    // Eight ticks' commands arrive in one go, then one a tick as usual.
    for _ in 0..8 {
        client.predict(&level, &rules, &firing, &mut packet);
        server.tick(&level, &rules);
    }
    server.receive(id, &packet).unwrap();
    for _ in 0..30 {
        client.predict(&level, &rules, &firing, &mut packet);
        server.receive(id, &packet).unwrap();
        server.tick(&level, &rules);
        assert!(server.input(id).unwrap().fire);
    }
    // Only the buffer's two commands are left waiting, not eight.
    server.tick(&level, &rules);
    server.tick(&level, &rules);
    assert!(server.input(id).unwrap().fire);
    server.tick(&level, &rules);
    assert!(
        !server.input(id).unwrap().fire,
        "a command was still waiting"
    );
}

#[test]
fn a_snapshot_from_before_the_first_tick_is_skipped() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let mut client = Client::new(start, STEP, ClientSettings::default());
    let mut packet = Vec::new();
    server.write_snapshot(id, &mut packet).unwrap();
    assert_eq!(
        client.receive(&level, &rules, &packet),
        Ok(Received::Skipped)
    );

    // A client that claims to have read a snapshot from the future is
    // still sent ones it can read.
    client.predict(&level, &rules, &walking(1.0), &mut packet);
    packet[2..6].copy_from_slice(&5_u32.to_le_bytes());
    server.receive(id, &packet).unwrap();
    for _ in 0..8 {
        server.tick(&level, &rules);
        server.write_snapshot(id, &mut packet).unwrap();
        assert_ne!(
            client.receive(&level, &rules, &packet),
            Ok(Received::Skipped)
        );
    }
}

#[test]
fn a_player_whose_commands_stop_coming_carries_on_briefly_then_stands_still() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let mut client = Client::new(start, STEP, ClientSettings::default());
    let mut packet = Vec::new();
    let firing = PlayerInput {
        fire: true,
        ..walking(1.0)
    };
    for _ in 0..60 {
        client.predict(&level, &rules, &firing, &mut packet);
        server.receive(id, &packet).unwrap();
        server.tick(&level, &rules);
    }
    assert!(server.input(id).unwrap().fire);

    // The connection goes quiet. Two commands were still waiting.
    server.tick(&level, &rules);
    server.tick(&level, &rules);
    let before = server.player(id).unwrap().position[2];
    server.tick(&level, &rules);
    let input = server.input(id).unwrap();
    assert_eq!(input.movement.forward, 1.0, "keeps walking");
    assert!(!input.fire, "does not keep firing");
    assert!(server.player(id).unwrap().position[2] < before - 0.05);

    for _ in 0..60 {
        server.tick(&level, &rules);
    }
    assert_eq!(server.input(id).unwrap().movement.forward, 0.0);
    let player = server.player(id).unwrap();
    assert!(distance(player.velocity, [0.0; 3]) < 0.01, "{player:?}");
}

#[test]
fn a_client_sees_the_other_players_to_within_the_rounding() {
    let mut game = Match::new(3, CLEAR);
    game.run(200, |seat, tick| busy(tick + seat as u32 * 50));
    let seen: Vec<_> = game.seats[0].client.others().collect();
    assert_eq!(seen.len(), 2);
    for other in seen {
        assert_ne!(other.id, game.seats[0].id);
        // The newest snapshot read is of the tick before.
        assert_eq!(game.server.player_at(other.id, 199), Some(other));
    }
    // Handed over with no delay, a snapshot shows the others as they are.
    let mut packet = Vec::new();
    game.server.write_snapshot(0, &mut packet).unwrap();
    let (level, rules) = (arena(), MoveSettings::default());
    game.seats[0]
        .client
        .receive(&level, &rules, &packet)
        .unwrap();
    for other in game.seats[0].client.others() {
        let real = game.server.player(other.id).unwrap();
        assert!(distance(other.position, real.position) < 0.002, "{other:?}");
        assert_eq!(other.grounded, real.grounded);
        assert_eq!(other.crouched, real.crouched);
        assert_eq!(other.sprinting, real.sprinting);
        let input = game.server.input(other.id).unwrap();
        assert!((other.yaw.sin() - input.movement.yaw.sin()).abs() < 0.001);
        assert!((other.pitch - input.pitch).abs() < 0.001);
    }
    assert_eq!(game.server.player_at(1, 100), None, "too long ago");
    assert_eq!(game.server.players().count(), 3);

    // Someone leaves; the next snapshot no longer has them.
    let gone = game.seats.pop().unwrap();
    game.server.leave(gone.id);
    game.run(2, |_, _| PlayerInput::default());
    let seen: Vec<_> = game.seats[0]
        .client
        .others()
        .map(|other| other.id)
        .collect();
    assert_eq!(seen, [game.seats[1].id]);
    assert_eq!(game.server.player(gone.id), None);
}

#[test]
fn only_what_changed_is_sent_once_a_snapshot_has_been_read() {
    let mut game = Match::new(8, CLEAR);
    // Everyone stands still but the first player.
    let input = |seat: usize, _| {
        if seat == 0 {
            walking(1.0)
        } else {
            PlayerInput::default()
        }
    };
    game.tick(input);
    let whole = game.sent[0].len();
    game.run(120, input);
    let changes = game.sent[game.sent.len() - 8].len();
    // Seven others whole are 16 bytes each; unchanged they are 2 each.
    assert_eq!(
        whole - changes,
        7 * 14,
        "{whole} bytes whole, {changes} as changes"
    );
    // The others, who see the walker move, are sent the walker's position.
    assert_eq!(game.sent[game.sent.len() - 1].len(), changes + 9);
    assert_eq!(game.seats[0].client.others().count(), 7);
}

#[test]
fn a_client_that_hears_nothing_for_a_while_picks_up_again() {
    let mut game = Match::new(2, CLEAR);
    game.run(60, |_, tick| busy(tick));
    // Nothing reaches the first player for a second and a half: longer
    // than either side remembers snapshots.
    game.seats[0].down.weather.lost = 100;
    game.run(90, |_, tick| busy(tick));
    assert_eq!(game.seats[0].client.server_tick(), 60);
    game.seats[0].down.weather.lost = 0;
    game.run(30, |_, tick| busy(tick));
    let seat = &game.seats[0];
    assert_eq!(seat.client.server_tick(), 179);
    assert_eq!(seat.results.last(), Some(&Received::Matched));
    let other = seat.client.others().next().unwrap();
    assert_eq!(game.server.player_at(other.id, 179), Some(other));
}

#[test]
fn old_and_repeated_snapshots_are_skipped() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let mut client = Client::new(start, STEP, ClientSettings::default());
    let (mut commands, mut first, mut second) = (Vec::new(), Vec::new(), Vec::new());
    client.predict(&level, &rules, &walking(1.0), &mut commands);
    server.receive(id, &commands).unwrap();
    // The same packet again changes nothing.
    server.receive(id, &commands).unwrap();
    server.tick(&level, &rules);
    server.write_snapshot(id, &mut first).unwrap();
    server.tick(&level, &rules);
    server.write_snapshot(id, &mut second).unwrap();

    assert_ne!(
        client.receive(&level, &rules, &second),
        Ok(Received::Skipped)
    );
    let player = *client.player();
    assert_eq!(
        client.receive(&level, &rules, &first),
        Ok(Received::Skipped)
    );
    assert_eq!(
        client.receive(&level, &rules, &second),
        Ok(Received::Skipped)
    );
    assert_eq!(*client.player(), player);
    assert_eq!(client.server_tick(), 2);
}

#[test]
fn a_full_server_fits_its_snapshot_in_one_packet() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let mut clients = Vec::new();
    for seat in 0..MAX_PLAYERS {
        let start = spawn(seat as f32 * -1.5);
        let id = server.join(start).unwrap();
        assert_eq!(usize::from(id), seat);
        clients.push(Client::new(start, STEP, ClientSettings::default()));
    }
    assert_eq!(server.join(spawn(0.0)), Err(NetError::ServerFull));

    let mut packet = Vec::new();
    let mut largest = 0;
    for tick in 0..30 {
        for (id, client) in (0..).zip(&mut clients) {
            // Everyone moves and turns, so everything changes every tick.
            client.predict(&level, &rules, &busy(tick * 3 + u32::from(id)), &mut packet);
            server.receive(id, &packet).unwrap();
        }
        server.tick(&level, &rules);
        for (id, client) in (0..).zip(&mut clients) {
            server.write_snapshot(id, &mut packet).unwrap();
            largest = largest.max(packet.len());
            client.receive(&level, &rules, &packet).unwrap();
        }
    }
    assert!(largest <= MAX_PACKET_BYTES, "{largest} bytes");
    assert!(largest > 900, "{largest} bytes: was everyone in it?");
    assert_eq!(clients[0].others().count(), MAX_PLAYERS - 1);

    server.leave(5);
    assert_eq!(server.join(spawn(0.0)), Ok(5));
}

#[test]
fn packets_of_the_wrong_sort_are_refused_with_a_reason() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings::default());
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let mut client = Client::new(start, STEP, ClientSettings::default());
    let (mut commands, mut snapshot) = (Vec::new(), Vec::new());
    client.predict(&level, &rules, &walking(1.0), &mut commands);
    server.receive(id, &commands).unwrap();
    server.tick(&level, &rules);
    server.write_snapshot(id, &mut snapshot).unwrap();

    assert_eq!(server.receive(id, &snapshot), Err(NetError::WrongKind));
    assert_eq!(
        client.receive(&level, &rules, &commands),
        Err(NetError::WrongKind)
    );
    assert_eq!(server.receive(id, &[]), Err(NetError::TooShort));
    assert_eq!(
        server.receive(id, &vec![1; MAX_PACKET_BYTES + 1]),
        Err(NetError::TooLong)
    );
    assert_eq!(server.receive(9, &commands), Err(NetError::NoSuchPlayer));
    assert_eq!(
        server.write_snapshot(9, &mut Vec::new()),
        Err(NetError::NoSuchPlayer)
    );

    let mut newer = commands.clone();
    newer[1] += 1;
    assert_eq!(server.receive(id, &newer), Err(NetError::WrongVersion));
    let mut newer = snapshot.clone();
    newer[1] += 1;
    assert_eq!(
        client.receive(&level, &rules, &newer),
        Err(NetError::WrongVersion)
    );

    // A position that is not a number.
    let mut broken = snapshot.clone();
    broken[15..19].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        client.receive(&level, &rules, &broken),
        Err(NetError::BadValue)
    );
    // A snapshot that claims a command the client never sent.
    let mut early = snapshot.clone();
    early[10..14].copy_from_slice(&77_u32.to_le_bytes());
    assert_eq!(
        client.receive(&level, &rules, &early),
        Err(NetError::BadValue)
    );

    for error in [
        NetError::TooShort,
        NetError::TooLong,
        NetError::WrongKind,
        NetError::WrongVersion,
        NetError::NoSuchPlayer,
        NetError::ServerFull,
        NetError::BadValue,
    ] {
        assert!(error.to_string().len() > 40, "{error:?} says what to do");
    }
}

#[test]
fn damaged_packets_never_do_worse_than_an_error() {
    let mut game = Match::new(4, CLEAR);
    game.run(40, |seat, tick| busy(tick + seat as u32 * 9));
    let level = arena();
    let rules = MoveSettings::default();
    let snapshot = game.sent.last().unwrap().clone();
    let mut commands = Vec::new();
    game.seats[0]
        .client
        .predict(&level, &rules, &busy(3), &mut commands);

    let mut noise = Link::new(CLEAR, 99);
    let try_both = |packet: &[u8]| {
        let mut server = game.server.clone();
        let _ = server.receive(0, packet);
        server.tick(&level, &rules);
        let mut client = game.seats[3].client.clone();
        if client.receive(&level, &rules, packet).is_ok() {
            let mut out = Vec::new();
            client.predict(&level, &rules, &busy(1), &mut out);
            assert!(client.player().position.iter().all(|part| part.is_finite()));
        }
    };
    for packet in [&snapshot, &commands] {
        // Cut short at every length.
        for length in 0..packet.len() {
            try_both(&packet[..length]);
        }
        // Every byte changed, one at a time, to a few other values.
        for at in 0..packet.len() {
            for _ in 0..6 {
                let mut damaged = packet.clone();
                damaged[at] = noise.random(256) as u8;
                try_both(&damaged);
            }
        }
    }
    // Bytes that were never a packet.
    for _ in 0..2_000 {
        let length = noise.random(200) as usize;
        let mut junk: Vec<u8> = (0..length).map(|_| noise.random(256) as u8).collect();
        if let Some(kind) = junk.first_mut() {
            *kind = 1 + *kind % 2;
        }
        if let Some(version) = junk.get_mut(1) {
            *version = 1;
        }
        try_both(&junk);
    }
}

#[test]
fn a_cut_short_packet_changes_nothing() {
    let level = arena();
    let rules = MoveSettings::default();
    let mut server = Server::new(STEP, ServerSettings { buffer: 1 });
    let start = spawn(0.0);
    let id = server.join(start).unwrap();
    let mut client = Client::new(start, STEP, ClientSettings::default());
    let mut packet = Vec::new();
    for _ in 0..3 {
        client.predict(&level, &rules, &walking(1.0), &mut packet);
    }
    // Three commands, the last cut off: none of them is taken.
    assert_eq!(
        server.receive(id, &packet[..packet.len() - 1]),
        Err(NetError::TooShort)
    );
    server.tick(&level, &rules);
    assert_eq!(server.input(id).unwrap().movement.forward, 0.0);
    server.receive(id, &packet).unwrap();
    server.tick(&level, &rules);
    assert_eq!(server.input(id).unwrap().movement.forward, 1.0);
}
