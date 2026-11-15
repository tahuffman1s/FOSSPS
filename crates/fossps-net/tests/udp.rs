//! The netcode over real UDP sockets on this machine: in one process, and
//! between two.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::UdpSocket;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use fossps_net::{
    Client, ClientSettings, Closed, Gone, HostEvent, HostSettings, LinkEvent, LinkSettings,
    PlayerId, PlayerInput, Received, Refusal, Server, ServerSettings, UdpHost, UdpLink,
};
use fossps_physics::{Level, MoveInput, MoveSettings, Player};

const STEP: f32 = 1.0 / 60.0;
const TICK: Duration = Duration::from_millis(2);
/// Far longer than anything here takes; only a broken test waits it out.
const PATIENCE: Duration = Duration::from_secs(20);

fn floor() -> Level {
    let s = 200.0;
    Level::from_triangles(&[
        [[-s, 0.0, s], [s, 0.0, s], [s, 0.0, -s]],
        [[-s, 0.0, s], [s, 0.0, -s], [-s, 0.0, -s]],
    ])
}

fn spawn() -> Player {
    Player::at([0.0, 0.5, 0.0], &MoveSettings::default())
}

fn forward() -> PlayerInput {
    PlayerInput {
        movement: MoveInput {
            forward: 1.0,
            ..MoveInput::default()
        },
        ..PlayerInput::default()
    }
}

/// A whole server: the socket, the players and the level they walk.
struct Hosted {
    host: UdpHost,
    server: Server,
    level: Level,
    rules: MoveSettings,
    packet: Vec<u8>,
    places: usize,
    joined: Vec<PlayerId>,
    left: Vec<(PlayerId, Gone)>,
}

impl Hosted {
    fn new(settings: HostSettings) -> Self {
        Self {
            host: UdpHost::bind("127.0.0.1:0", settings).unwrap(),
            server: Server::new(STEP, ServerSettings::default()),
            level: floor(),
            rules: MoveSettings::default(),
            packet: Vec::new(),
            places: usize::MAX,
            joined: Vec::new(),
            left: Vec::new(),
        }
    }

    fn address(&self) -> String {
        self.host.local_addr().unwrap().to_string()
    }

    fn tick(&mut self) {
        while let Some(event) = self.host.poll() {
            match event {
                HostEvent::Join(request) => {
                    if self.server.players().count() >= self.places {
                        self.host.refuse(request, Refusal::Full);
                    } else {
                        let id = self.server.join(spawn()).unwrap();
                        self.host.accept(request, id);
                        self.joined.push(id);
                    }
                }
                HostEvent::Packet(id, commands) => drop(self.server.receive(id, commands)),
                HostEvent::Left(id, why) => {
                    self.server.leave(id);
                    self.left.push((id, why));
                }
            }
        }
        self.server.tick(&self.level, &self.rules);
        for id in self.host.players() {
            self.server.write_snapshot(id, &mut self.packet).unwrap();
            self.host.send(id, &self.packet);
        }
    }
}

/// A whole player: the socket and the prediction.
struct Joining {
    link: UdpLink,
    client: Client,
    level: Level,
    rules: MoveSettings,
    packet: Vec<u8>,
    matched: usize,
    closed: Option<Closed>,
}

impl Joining {
    fn new(address: &str, settings: LinkSettings) -> Self {
        Self {
            link: UdpLink::connect(address, settings).unwrap(),
            client: Client::new(spawn(), STEP, ClientSettings::default()),
            level: floor(),
            rules: MoveSettings::default(),
            packet: Vec::new(),
            matched: 0,
            closed: None,
        }
    }

    fn tick(&mut self) {
        while let Some(event) = self.link.poll() {
            match event {
                LinkEvent::Joined(_) => {}
                LinkEvent::Packet(snapshot) => {
                    if self.client.receive(&self.level, &self.rules, snapshot)
                        == Ok(Received::Matched)
                    {
                        self.matched += 1;
                    }
                }
                LinkEvent::Closed(why) => self.closed = Some(why),
            }
        }
        if self.link.id().is_some() {
            self.client
                .predict(&self.level, &self.rules, &forward(), &mut self.packet);
            self.link.send(&self.packet);
        }
    }
}

/// Runs `tick` every couple of milliseconds until `done` says so.
fn until(what: &str, mut tick: impl FnMut() -> bool) {
    let started = Instant::now();
    while !tick() {
        assert!(started.elapsed() < PATIENCE, "gave up waiting for {what}");
        sleep(TICK);
    }
}

#[test]
fn two_players_join_walk_and_see_each_other() {
    let mut hosted = Hosted::new(HostSettings::default());
    let address = hosted.address();
    let mut players = [
        Joining::new(&address, LinkSettings::default()),
        Joining::new(&address, LinkSettings::default()),
    ];
    until("both players to walk a metre", || {
        hosted.tick();
        for player in &mut players {
            player.tick();
        }
        players.iter().all(|player| {
            player.matched > 10
                && player.client.others().count() == 1
                && player.client.player().position[2] < -1.0
        })
    });
    for player in &players {
        assert_eq!(player.client.id(), player.link.id());
        assert_eq!(player.closed, None);
    }
    assert_ne!(players[0].link.id(), players[1].link.id());
    assert_eq!(hosted.joined.len(), 2);
    for (id, on_server) in hosted.server.players() {
        assert!(on_server.position[2] < -0.5, "player {id} never moved");
    }
}

#[test]
fn leaving_is_heard_and_the_place_is_given_to_the_next_player() {
    let mut hosted = Hosted::new(HostSettings::default());
    let address = hosted.address();
    let mut first = Joining::new(&address, LinkSettings::default());
    until("the first player to join", || {
        hosted.tick();
        first.tick();
        first.link.id().is_some()
    });
    let id = first.link.id().unwrap();
    drop(first);
    until("the server to hear the goodbye", || {
        hosted.tick();
        !hosted.left.is_empty()
    });
    assert_eq!(hosted.left, [(id, Gone::Quit)]);
    assert_eq!(hosted.server.players().count(), 0);

    let mut second = Joining::new(&address, LinkSettings::default());
    until("the second player to join", || {
        hosted.tick();
        second.tick();
        second.link.id().is_some()
    });
    assert_eq!(second.link.id(), Some(id));
}

#[test]
fn a_player_who_goes_silent_is_dropped() {
    let mut hosted = Hosted::new(HostSettings {
        timeout: Duration::from_millis(200),
    });
    let mut player = Joining::new(&hosted.address(), LinkSettings::default());
    until("the player to join", || {
        hosted.tick();
        player.tick();
        player.link.id().is_some()
    });
    // The player's machine freezes: nothing is sent, not even a goodbye.
    until("the server to give up on the player", || {
        hosted.tick();
        !hosted.left.is_empty()
    });
    assert_eq!(hosted.left, [(0, Gone::TimedOut)]);
    assert_eq!(hosted.host.players().count(), 0);
}

#[test]
fn a_server_that_stops_tells_its_players() {
    let mut hosted = Hosted::new(HostSettings::default());
    let mut player = Joining::new(&hosted.address(), LinkSettings::default());
    until("the player to join", || {
        hosted.tick();
        player.tick();
        player.link.id().is_some()
    });
    drop(hosted);
    until("the player to hear the server stopped", || {
        player.tick();
        player.closed.is_some()
    });
    assert_eq!(player.closed, Some(Closed::ServerStopped));
    assert_eq!(player.link.id(), None);
}

#[test]
fn a_server_that_dies_without_a_word_is_given_up_on() {
    let mut hosted = Hosted::new(HostSettings::default());
    let settings = LinkSettings {
        timeout: Duration::from_millis(200),
        ..LinkSettings::default()
    };
    let mut player = Joining::new(&hosted.address(), settings);
    until("the player to join", || {
        hosted.tick();
        player.tick();
        player.link.id().is_some()
    });
    // The server freezes: it is still there but says nothing.
    until("the player to give up on the server", || {
        player.tick();
        player.closed.is_some()
    });
    assert_eq!(player.closed, Some(Closed::TimedOut));
}

#[test]
fn a_full_server_turns_the_next_player_away_with_a_reason() {
    let mut hosted = Hosted::new(HostSettings::default());
    hosted.places = 1;
    let address = hosted.address();
    let mut first = Joining::new(&address, LinkSettings::default());
    until("the first player to join", || {
        hosted.tick();
        first.tick();
        first.link.id().is_some()
    });
    let mut second = Joining::new(&address, LinkSettings::default());
    until("the second player to be turned away", || {
        hosted.tick();
        second.tick();
        second.closed.is_some()
    });
    assert_eq!(second.closed, Some(Closed::Full));
    assert_eq!(hosted.server.players().count(), 1);
}

#[test]
fn joining_where_nothing_listens_gives_up_and_says_what_to_check() {
    // A port that was open a moment ago and is closed now.
    let address = {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.local_addr().unwrap().to_string()
    };
    let settings = LinkSettings {
        give_up: Duration::from_millis(300),
        ..LinkSettings::default()
    };
    let mut player = Joining::new(&address, settings);
    until("the player to give up", || {
        player.tick();
        player.closed.is_some()
    });
    assert_eq!(player.closed, Some(Closed::NoAnswer));
    assert!(Closed::NoAnswer.to_string().contains("firewall"));
}

#[test]
fn a_flood_of_rubbish_does_not_hold_up_the_tick() {
    let mut hosted = Hosted::new(HostSettings::default());
    let flood = UdpSocket::bind("127.0.0.1:0").unwrap();
    let to = hosted.host.local_addr().unwrap();
    // More than one tick reads; the system may drop some, which is fine.
    for _ in 0..1500 {
        let _ = flood.send_to(b"rubbish", to);
    }
    sleep(Duration::from_millis(50));
    assert!(hosted.host.poll().is_none());
    let mut player = Joining::new(&hosted.address(), LinkSettings::default());
    until("a player to join after the flood", || {
        hosted.tick();
        player.tick();
        player.link.id().is_some()
    });
}

#[test]
fn a_name_with_both_kinds_of_address_is_joined_over_ipv4() {
    let mut hosted = Hosted::new(HostSettings::default());
    let port = hosted.host.local_addr().unwrap().port();
    let mut player = Joining::new(&format!("localhost:{port}"), LinkSettings::default());
    until("a player to join by name", || {
        hosted.tick();
        player.tick();
        player.link.id().is_some()
    });
}

#[test]
fn a_port_in_use_and_an_unreadable_address_say_what_to_do() {
    let hosted = Hosted::new(HostSettings::default());
    let taken = hosted.address();
    let error = UdpHost::bind(&taken, HostSettings::default()).unwrap_err();
    let text = error.to_string();
    assert!(text.contains(&taken), "{text}");
    assert!(text.contains("choose another port"), "{text}");

    let error = UdpLink::connect("not an address", LinkSettings::default()).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("`not an address`"), "{text}");
    assert!(text.contains("203.0.113.5:27015"), "{text}");
}

/// The name of the variable that turns [`the_server_process`] on.
const ROLE: &str = "FOSSPS_NET_TEST_SERVER";

/// The other half of [`two_processes_exchange_commands_and_snapshots`]:
/// run as a test on its own it does nothing.
#[test]
fn the_server_process() {
    if std::env::var_os(ROLE).is_none() {
        return;
    }
    let mut hosted = Hosted::new(HostSettings::default());
    println!("PORT {}", hosted.host.local_addr().unwrap().port());
    let mut furthest = 0.0_f32;
    until("a player to join, walk and leave", || {
        hosted.tick();
        for (_, player) in hosted.server.players() {
            furthest = furthest.min(player.position[2]);
        }
        !hosted.left.is_empty()
    });
    assert_eq!(hosted.left, [(0, Gone::Quit)]);
    assert!(furthest < -1.0, "the player only got to {furthest}");
}

#[test]
fn two_processes_exchange_commands_and_snapshots() {
    /// Stops the other process if this one fails halfway.
    struct Stopped(Child);
    impl Drop for Stopped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let mut server = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "the_server_process", "--nocapture"])
        .env(ROLE, "1")
        .stdout(Stdio::piped())
        .spawn()
        .map(Stopped)
        .unwrap();
    // Kept open to the end, so the other process can go on printing.
    let mut said = BufReader::new(server.0.stdout.take().unwrap()).lines();
    let port = said
        .by_ref()
        // The test runner may print on the same line first.
        .find_map(|line| line.unwrap().split("PORT ").nth(1).map(str::to_owned))
        .expect("the server process never said its port");

    let mut player = Joining::new(&format!("127.0.0.1:{port}"), LinkSettings::default());
    until(
        "the player to walk a metre with the server agreeing",
        || {
            player.tick();
            assert_eq!(player.closed, None);
            player.matched > 10 && player.client.player().position[2] < -1.5
        },
    );
    assert_eq!(player.client.id(), Some(0));
    drop(player);
    said.for_each(drop);
    assert!(server.0.wait().unwrap().success());
}

/// Polls `search` for half a second and gives what it found.
fn found_by(
    search: &mut fossps_net::UdpSearch,
    mut meanwhile: impl FnMut(),
) -> Vec<fossps_net::FoundServer> {
    let mut found = Vec::new();
    while search.elapsed() < Duration::from_millis(500) {
        meanwhile();
        while let Some(server) = search.poll() {
            found.push(server);
        }
        sleep(TICK);
    }
    found
}

#[test]
fn a_search_finds_the_server_on_this_machine_with_its_name_and_players() {
    let mut hosted = Hosted::new(HostSettings::default());
    hosted.host.set_name("zombie-night");
    let address = hosted.host.local_addr().unwrap();
    let mut joined = Joining::new(&address.to_string(), LinkSettings::default());
    let deadline = Instant::now() + PATIENCE;
    while joined.link.id().is_none() {
        assert!(Instant::now() < deadline, "never joined");
        joined.tick();
        hosted.tick();
        sleep(TICK);
    }

    let mut search = fossps_net::UdpSearch::start(address.port()).unwrap();
    let found = found_by(&mut search, || {
        joined.tick();
        hosted.tick();
    });

    assert_eq!(found.len(), 1, "{found:?}");
    let server = &found[0];
    assert_eq!(server.address, address);
    assert_eq!(server.name, "zombie-night");
    assert_eq!((server.players, server.places), (1, 64));
    assert!(server.same_version);
}

#[test]
fn a_search_with_no_server_finds_nothing() {
    let free = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut search = fossps_net::UdpSearch::start(free).unwrap();
    assert_eq!(found_by(&mut search, || {}), Vec::new());
}
