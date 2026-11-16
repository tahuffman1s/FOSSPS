//! Playing over a network: the game on a server, and a player joined to one.
//!
//! [`HostedGame`] is the server's side. It opens a UDP port, gives everyone
//! who joins a player, moves the players with the commands they send and
//! tells them all where everybody is, sixty times a second. The game's
//! script hears about each player joining and leaving.
//!
//! [`JoinedGame`] is a player's side. It moves its own player the moment a
//! key is pressed, without waiting for the server, and corrects itself when
//! the server disagrees.
//!
//! A player needs no copy of the game. A server hands its level file and
//! its map to whoever joins ([`HostedGame::share_level`]), before giving
//! them a place; the level is kept on the player's computer
//! ([`crate::cache`]) so it is fetched once.
//!
//! [`LocalGame`] is both in one program, which is what `fossps run` plays:
//! the same code runs alone as runs over the internet.
//!
//! [`find_servers`] looks for the games being served on the local network,
//! so a player can join one without knowing its address.
//!
//! ```no_run
//! use fossps::online::{DEFAULT_PORT, HostedGame};
//! use fossps::server::{ServerConfig, StopSignal, serve};
//! use fossps::ScriptGame;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ServerConfig::default();
//!     let game = ScriptGame::open("my-game", &config.actions)?;
//!     let address = format!("0.0.0.0:{DEFAULT_PORT}");
//!     let mut hosted = HostedGame::open(game, &address, config.tick_rate)?;
//!     // Players walk the game's level, if it has one, and are sent it
//!     // when they join.
//!     if let Some((level, file)) = fossps::level::load_file("my-game".as_ref())? {
//!         hosted = hosted.with_level(&level).share_level(file)?;
//!     }
//!     serve(&config, &mut hosted, &StopSignal::on_ctrl_c()?);
//!     Ok(())
//! }
//! ```

use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fossps_assets::{Marker, Scene};
use fossps_input::{ActionId, ActionMap, InputFrame};
use fossps_mode::{SpawnPoint, Stander, choose_spawn};
use fossps_net::{
    Client, ClientSettings, Closed, HostEvent, HostSettings, LinkEvent, LinkSettings, MAX_PLAYERS,
    MAX_SHARED_BYTES, Offer, PlayerId, PlayerInput, Refusal, RemotePlayer, Server, ServerSettings,
    SocketError, UdpHost, UdpLink, UdpSearch, hash_of,
};
pub use fossps_net::{FoundServer, ShareError, lan_address};
use fossps_physics::{Level, MoveInput, MoveSettings, Player};

use crate::game::{Frame, Game};
use crate::map::{MAP_FILE, Map};
use crate::scaffold::LEVEL_FILE;
use crate::script_game::ScriptGame;

/// The UDP port a server opens and a player joins when none is named.
pub const DEFAULT_PORT: u16 = 24680;

/// Radians turned per count of mouse movement.
pub(crate) const MOUSE_TURN: f32 = 0.002;

/// Radians turned per second with the look stick pushed all the way.
pub(crate) const STICK_TURN: f32 = 3.5;

/// Half the width of the floor players stand on until a game has a level.
const FLOOR_REACH: f32 = 500.0;

/// How long [`find_servers`] is usually given: long enough for every server
/// on a local network to answer, short enough not to feel like waiting.
pub const SEARCH_TIME: Duration = Duration::from_secs(1);

/// How long to wait between looks at the socket while joining.
const JOIN_POLL: Duration = Duration::from_millis(5);

/// What a marker in a level is called to be a place players start: `spawn`,
/// or anything that begins with it, such as `spawn.001` or `Spawn Red`.
const SPAWN_NAME: &str = "spawn";

/// How long a player started at a spawn point is given to land when a level
/// is tried out, in ticks: three seconds, a fall of some 40 m.
const LANDING_TICKS: u32 = 180;

/// A player's first snapshot puts them this near a spawn point, in metres
/// across the ground, if that is where the server started them.
const AT_SPAWN: f32 = 0.5;

/// Which of the files a server offers is the game's level file.
const LEVEL_SLOT: usize = 0;

/// Which of the files a server offers is the game's map, as text.
const MAP_SLOT: usize = 1;

/// The most bytes of map a server offers and a player takes: 1 MiB, far
/// more than any map written by hand or by the editor.
const MOST_MAP_BYTES: usize = 1024 * 1024;

/// How often a player waiting for a level is told how far along it is.
const PROGRESS_EVERY: Duration = Duration::from_secs(2);

/// What a server and its players must agree on to work out the same moves.
///
/// Cheap to copy: the level's shapes are shared, not copied.
#[derive(Debug, Clone)]
struct World {
    level: Arc<Level>,
    /// Where players may start. Never empty.
    spawns: Arc<[SpawnPoint]>,
    /// Which way a player starting at each of `spawns` faces.
    facing: Arc<[f32]>,
    rules: MoveSettings,
    /// Ticks in one second.
    tick_rate: u32,
    /// Seconds in one tick.
    step: f32,
}

impl World {
    /// A flat floor, 1 km on a side, at height zero, with players starting
    /// at its middle.
    fn flat(tick_rate: u32) -> Self {
        let r = FLOOR_REACH;
        let corners = [[-r, 0.0, -r], [r, 0.0, -r], [r, 0.0, r], [-r, 0.0, r]];
        let [a, b, c, d] = corners;
        Self::new(
            Level::from_triangles(&[[a, b, c], [a, c, d]]),
            &[],
            tick_rate,
        )
    }

    /// The world of a game's level: every shape in it is solid, and players
    /// start at its spawn markers, or at its middle if it has none.
    fn from_level(level: &Scene, tick_rate: u32) -> Self {
        let spawns: Vec<&Marker> = spawn_markers(level).collect();
        Self::new(Level::from_scene(level), &spawns, tick_rate)
    }

    fn new(level: Level, markers: &[&Marker], tick_rate: u32) -> Self {
        let mut spawns: Vec<SpawnPoint> = markers
            .iter()
            .map(|marker| SpawnPoint {
                position: position_of(marker),
                team: None,
            })
            .collect();
        let mut facing: Vec<f32> = markers.iter().map(|marker| yaw_of(marker)).collect();
        if spawns.is_empty() {
            spawns.push(SpawnPoint {
                position: [0.0; 3],
                team: None,
            });
            facing.push(0.0);
        }
        Self {
            level: Arc::new(level),
            spawns: spawns.into(),
            facing: facing.into(),
            rules: MoveSettings::default(),
            tick_rate,
            step: 1.0 / tick_rate.max(1) as f32,
        }
    }

    /// The world of a game with `level`, or the flat floor of a game with
    /// none. What is wrong for walking with the level and the spawn points
    /// in `map` is logged.
    fn of(level: Option<&Scene>, map: &Map, tick_rate: u32) -> Self {
        let Some(level) = level else {
            return Self::flat(tick_rate);
        };
        let world = Self::from_level(level, tick_rate);
        for problem in world.problems(level, map) {
            log::warn!("{problem}");
        }
        world
    }

    /// A player about to come into the world. Where players start is one
    /// rule everywhere: at the spawn points in `map` if it has any, else
    /// at the level's spawn markers, else in the middle of the level. Of
    /// those the one furthest from `others` is taken, or one nearly as
    /// good; `seed` settles the choice and should differ from one call to
    /// the next.
    ///
    /// Every spawn point is for anyone, and everyone else counts as an
    /// enemy to keep away from, until teams reach the running game (#171).
    fn spawn_in<'a>(
        &self,
        map: &Map,
        others: impl Iterator<Item = &'a Player>,
        seed: u32,
    ) -> Player {
        let mut standers = [Stander {
            position: [0.0; 3],
            enemy: true,
        }; MAX_PLAYERS];
        let mut count = 0;
        for (stander, other) in standers.iter_mut().zip(others) {
            stander.position = other.position;
            count += 1;
        }
        let placed: Vec<SpawnPoint> = map
            .spawns()
            .map(|spawn| SpawnPoint {
                position: spawn.position,
                team: None,
            })
            .collect();
        let points: &[SpawnPoint] = if placed.is_empty() {
            &self.spawns
        } else {
            &placed
        };
        let chosen = choose_spawn(points, None, &standers[..count], seed)
            .and_then(|point| points.get(point))
            .map_or([0.0; 3], |point| point.position);
        Player::at(chosen, &self.rules)
    }

    /// A player where one may start, before the server has said where this
    /// one does.
    fn first(&self) -> Player {
        let position = self.spawns.first().map_or([0.0; 3], |point| point.position);
        Player::at(position, &self.rules)
    }

    /// Which way a player who has just started at `position` faces, if that
    /// is one of the spawn points: `map`'s if it has any, else the level's.
    fn facing_at(&self, map: &Map, position: [f32; 3]) -> Option<f32> {
        let near =
            |point: [f32; 3]| (point[0] - position[0]).hypot(point[2] - position[2]) <= AT_SPAWN;
        if map.spawns().next().is_some() {
            return map
                .spawns()
                .find(|spawn| near(spawn.position))
                .map(|spawn| spawn.facing.to_radians().rem_euclid(TAU))
                .filter(|yaw| yaw.is_finite());
        }
        self.spawns
            .iter()
            .zip(self.facing.iter())
            .find(|(point, _)| near(point.position))
            .map(|(_, yaw)| *yaw)
    }

    /// What is wrong for walking with `level`, which this world was made
    /// from, and the spawn points in `map`: nothing to stand on, nowhere to
    /// start, or a start that a player falls from. Each is a sentence for
    /// the game's maker.
    fn problems(&self, level: &Scene, map: &Map) -> Vec<String> {
        if self.level.triangle_count() == 0 {
            return vec![format!(
                "{LEVEL_FILE} has no shapes in it, so there is nothing to stand on and players \
                 fall for ever. Add a floor in your 3D tool and export the level again."
            )];
        }
        let mut problems = Vec::new();
        let starts = starts(level, map);
        if starts.is_empty() {
            problems.push(format!(
                "This game has no spawn points, so players start at the level's middle \
                 (0, 0, 0). To choose where they start, run `fossps edit` and use \"Place a \
                 spawn here\", which writes {MAP_FILE}; or add an empty object named `spawn` \
                 to the level in your 3D tool, turned the way players should look, and \
                 export it again."
            ));
        }
        let middle = [("the level's middle".to_owned(), [0.0; 3])];
        let starts = if starts.is_empty() {
            &middle[..]
        } else {
            &starts[..]
        };
        let still = MoveInput::default();
        for (place, position) in starts {
            let mut player = Player::at(*position, &self.rules);
            for _ in 0..LANDING_TICKS {
                player.step(&self.level, &self.rules, &still, self.step);
                if player.grounded {
                    break;
                }
            }
            if player.grounded {
                continue;
            }
            let [x, y, z] = position;
            let seconds = LANDING_TICKS as f32 * self.step;
            problems.push(format!(
                "A player who starts at {place} ({x:.1}, {y:.1}, {z:.1}) falls and has not \
                 landed after {seconds:.0} seconds: there is no floor under it. Move it to \
                 just above a floor, or put a floor under it."
            ));
        }
        problems
    }
}

/// Where players start in a game with `level` and `map`, each with what to
/// call it: the spawn points in the map if it has any, else the level's
/// spawn markers. Empty when the game has neither.
fn starts(level: &Scene, map: &Map) -> Vec<(String, [f32; 3])> {
    let placed: Vec<_> = map
        .spawns()
        .map(|spawn| {
            let name = if spawn.name.is_empty() {
                &spawn.id
            } else {
                &spawn.name
            };
            (
                format!("the spawn point \"{name}\" in {MAP_FILE}"),
                spawn.position,
            )
        })
        .collect();
    if !placed.is_empty() {
        return placed;
    }
    spawn_markers(level)
        .map(|marker| {
            (
                format!(
                    "the spawn point \"{}\" in {LEVEL_FILE}",
                    marker.name.as_deref().unwrap_or_default()
                ),
                position_of(marker),
            )
        })
        .collect()
}

/// The markers in `level` that are places for players to start.
fn spawn_markers(level: &Scene) -> impl Iterator<Item = &Marker> {
    level.markers.iter().filter(|marker| {
        marker.name.as_deref().is_some_and(|name| {
            name.trim_start()
                .get(..SPAWN_NAME.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(SPAWN_NAME))
        })
    })
}

/// Where `marker` stands.
fn position_of(marker: &Marker) -> [f32; 3] {
    let [x, y, z, _] = marker.transform[3];
    [x, y, z]
}

/// Which way `marker` faces across the ground, as a player's yaw: 0 faces
/// -Z and it grows turning left. A marker looks along its own -Z, as a
/// camera in a glTF file does; one pointing straight up or down faces -Z.
fn yaw_of(marker: &Marker) -> f32 {
    let [x, _, z, _] = marker.transform[2];
    // The marker's -Z is (-x, -z); yaw is the turn left from -Z to it.
    if x.is_finite() && z.is_finite() && x.hypot(z) > 1.0e-6 {
        x.atan2(z).rem_euclid(TAU)
    } else {
        0.0
    }
}

/// What trying a level out for walking found: see [`try_walking`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkCheck {
    /// How many places for players to start the game has: the `spawn`
    /// entities in its map file or, when that has none, the level's
    /// markers whose names begin with `spawn`.
    pub spawn_points: usize,
    /// What its maker should change, a sentence each. Empty when a player
    /// can start at every spawn point and land on something.
    pub problems: Vec<String>,
}

/// Tries `level` out for walking, with no window and no server: starts a
/// player at each spawn point, lets them stand for three seconds and says
/// which ones never landed. `fossps check` prints what it finds.
///
/// The spawn points are the `spawn` entities in `map`
/// ([`ScriptGame::map`]) if it has any, else the level's markers whose
/// names begin with `spawn`, else the middle of the level.
#[must_use]
pub fn try_walking(level: &Scene, map: &Map) -> WalkCheck {
    let world = World::from_level(level, 60);
    WalkCheck {
        spawn_points: starts(level, map).len(),
        problems: world.problems(level, map),
    }
}

/// A game being played on a server: its script, its players and the UDP
/// port they join through.
///
/// Hand it to [`serve`](crate::server::serve) (or [`run`](crate::run)): each
/// tick it reads what the players sent, moves them, sends everyone the
/// result and then runs the script's tick.
#[derive(Debug)]
pub struct HostedGame {
    game: ScriptGame,
    host: UdpHost,
    server: Server,
    world: World,
    /// Who each place on the server is to the script.
    names: [Option<fossps_script::PlayerId>; MAX_PLAYERS],
    /// How many times a player has been put at a spawn point.
    spawned: u32,
    packet: Vec<u8>,
    /// The map as players who join are sent it.
    offered_map: Map,
}

impl HostedGame {
    /// Opens `address` for players to join `game`. `address` is an address
    /// of this machine and a port: `0.0.0.0:24680` takes players from
    /// anywhere, `127.0.0.1:0` only from this machine on a port the system
    /// chooses ([`address`](Self::address) says which).
    ///
    /// `tick_rate` must be the rate the game is then ticked at, and the rate
    /// its players tick at.
    ///
    /// # Errors
    ///
    /// When the port cannot be opened, most often because another server
    /// already has it.
    pub fn open(game: ScriptGame, address: &str, tick_rate: u32) -> Result<Self, SocketError> {
        Self::open_in(game, address, World::flat(tick_rate))
    }

    fn open_in(game: ScriptGame, address: &str, world: World) -> Result<Self, SocketError> {
        Ok(Self {
            game,
            host: UdpHost::bind(address, HostSettings::default())?,
            server: Server::new(world.step, ServerSettings::default()),
            world,
            names: [None; MAX_PLAYERS],
            spawned: 0,
            packet: Vec::new(),
            offered_map: Map::default(),
        })
    }

    /// Hands `file` to every player who joins without it: the bytes of the
    /// level file that [`with_level`](Self::with_level) was given, as
    /// [`level::load_file`](crate::level::load_file) reads them. A player
    /// who joins with `fossps join` has no copy of the game, and without
    /// this walks a flat floor and is pulled back at every wall.
    ///
    /// The file is fetched before the player is given a place, and kept on
    /// their computer for next time. The game's map file needs nothing
    /// like this: players are always sent the map as the game has it.
    /// Scripts are not sent; they run on the server only.
    ///
    /// # Errors
    ///
    /// When the file is empty or holds more than 256 MiB, the most a
    /// player is sent.
    pub fn share_level(mut self, file: impl Into<Arc<[u8]>>) -> Result<Self, ShareError> {
        self.host.share(LEVEL_SLOT, file)?;
        Ok(self)
    }

    /// Offers players who join the map as the game now has it, if it has
    /// changed since it was last offered.
    fn offer_map(&mut self) {
        if *self.game.map() == self.offered_map {
            return;
        }
        self.offered_map.clone_from(self.game.map());
        let text = self.offered_map.to_text();
        if self.offered_map.is_empty() {
            self.host.unshare(MAP_SLOT);
        } else if text.len() > MOST_MAP_BYTES {
            self.host.unshare(MAP_SLOT);
            log::warn!(
                "{MAP_FILE} is {} KB, more than the {} KB sent to players who join, so they \
                 are not sent it and start out facing north. Take entities out of it.",
                text.len() / 1024,
                MOST_MAP_BYTES / 1024
            );
        } else if let Err(error) = self.host.share(MAP_SLOT, text.into_bytes()) {
            log::warn!("{MAP_FILE} is not sent to players who join: {error}");
        }
    }

    /// Plays the game in `level`, usually the game's `level.glb` as
    /// [`level::load`](crate::level::load) reads it: its shapes are solid.
    /// Without this, players walk a flat floor at height zero.
    ///
    /// Players start at the `spawn` entities of the game's map file
    /// ([`ScriptGame::map`]). When it has none they start at the level's
    /// markers whose names begin with `spawn` (`spawn`, `spawn.001`,
    /// `Spawn Red`), and with none of those either, at the level's middle.
    ///
    /// Call it before anyone joins. What is wrong with the level for
    /// walking, such as a spawn point with no floor under it, is logged as
    /// a warning ([`try_walking`] gives the same list).
    #[must_use]
    pub fn with_level(mut self, level: &Scene) -> Self {
        self.world = World::of(Some(level), self.game.map(), self.world.tick_rate);
        self
    }

    /// Names the game for players looking for one to join
    /// ([`find_servers`]). The game's folder name is the usual choice.
    #[must_use]
    pub fn named(mut self, name: &str) -> Self {
        self.host.set_name(name);
        self
    }

    /// The address and port players join through, if the system can say.
    #[must_use]
    pub fn address(&self) -> Option<SocketAddr> {
        self.host.local_addr().ok()
    }

    /// The game being played.
    #[must_use]
    pub fn game(&self) -> &ScriptGame {
        &self.game
    }

    /// The game being played, to change it.
    pub fn game_mut(&mut self) -> &mut ScriptGame {
        &mut self.game
    }

    /// Everyone joined: their number on the server and where they are.
    pub fn players(&self) -> impl Iterator<Item = (PlayerId, &Player)> + '_ {
        self.server.players()
    }

    /// Deals with everything players have sent since the last call: lets
    /// in who wants to join, takes commands, and lets go of who left.
    fn listen(&mut self) {
        let Self {
            game,
            host,
            server,
            world,
            names,
            spawned,
            ..
        } = self;
        while let Some(event) = host.poll() {
            match event {
                HostEvent::Join(request) => {
                    *spawned = spawned.wrapping_add(1);
                    let standing = server.players().map(|(_, player)| player);
                    let player = world.spawn_in(game.map(), standing, *spawned);
                    match server.join(player) {
                        Ok(id) => {
                            host.accept(request, id);
                            let name = format!("Player {}", u32::from(id) + 1);
                            log::info!("{name} joined from {}.", request.address());
                            if let Some(slot) = names.get_mut(usize::from(id)) {
                                *slot = Some(game.host_mut().player_joined(&name));
                            }
                        }
                        Err(_) => host.refuse(request, Refusal::Full),
                    }
                }
                // A packet that cannot be read is one more lost packet.
                HostEvent::Packet(id, commands) => drop(server.receive(id, commands)),
                HostEvent::Left(id, why) => {
                    server.leave(id);
                    log::info!("Player {} left ({why:?}).", u32::from(id) + 1);
                    if let Some(name) = names.get_mut(usize::from(id)).and_then(Option::take) {
                        game.host_mut().player_left(name);
                    }
                }
            }
        }
    }

    /// Puts everyone the match has just brought into the world, at the
    /// start of a round or after a death, at one of the map's spawn points.
    fn place_spawned(&mut self) {
        // Taken out first: placing reads the map, which the game also holds.
        let waiting: Vec<fossps_script::PlayerId> = self.game.host_mut().take_spawns().collect();
        for name in waiting {
            let place = (0..)
                .zip(self.names.iter())
                .find(|(_, held)| **held == Some(name));
            let Some((id, _)) = place else {
                continue;
            };
            self.spawned = self.spawned.wrapping_add(1);
            // Everyone but the one being placed is to be kept away from.
            let standing = self
                .server
                .players()
                .filter(|(other, _)| *other != id)
                .map(|(_, player)| player);
            let placed = self.world.spawn_in(self.game.map(), standing, self.spawned);
            if let Some(player) = self.server.player_mut(id) {
                *player = placed;
            }
        }
    }
}

impl Game for HostedGame {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.offer_map();
        self.listen();
        // Before anyone is told where they are: a player the match brought
        // into the world, on joining or in the script's last tick, is at
        // their spawn point in the first snapshot that shows them there.
        self.place_spawned();
        self.server.tick(&self.world.level, &self.world.rules);
        for id in self.host.players() {
            if self.server.write_snapshot(id, &mut self.packet).is_ok() {
                self.host.send(id, &self.packet);
            }
        }
        self.game.tick(input, step);
    }

    /// The script's own presenting, such as its rumble, which is for the
    /// player at this computer.
    fn frame(&mut self, frame: &mut Frame) {
        self.game.frame(frame);
    }
}

/// The actions that move a player, looked up once. An action the map does
/// not have reads as not pressed.
#[derive(Debug, Clone, Copy)]
struct Controls {
    walk: Option<ActionId>,
    look: Option<ActionId>,
    jump: Option<ActionId>,
    crouch: Option<ActionId>,
    sprint: Option<ActionId>,
    fire: Option<ActionId>,
    aim: Option<ActionId>,
    reload: Option<ActionId>,
}

impl Controls {
    fn new(actions: &ActionMap) -> Self {
        let find = |name| actions.action(name).ok();
        Self {
            walk: find("move"),
            look: find("look"),
            jump: find("jump"),
            crouch: find("crouch"),
            sprint: find("sprint"),
            fire: find("fire"),
            aim: find("aim"),
            reload: find("reload"),
        }
    }
}

/// Why a player could not join a server, or is no longer on it.
#[derive(Debug)]
pub enum JoinError {
    /// The address could not be read, or its name is not known.
    Address {
        /// The address that was given, with its port.
        address: String,
        /// What went wrong.
        error: SocketError,
    },
    /// The server did not let the player in, or the stay has ended.
    Closed {
        /// The address that was joined.
        address: String,
        /// What happened.
        why: Closed,
    },
    /// The level the server plays could not be used on this computer.
    Level {
        /// The address that was joined.
        address: String,
        /// What is wrong with the level, as a sentence or more.
        problem: String,
    },
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address { address, error } => {
                write!(f, "Could not join: there is no server address `{address}`")?;
                if let Some(cause) = std::error::Error::source(error) {
                    write!(f, " ({cause})")?;
                }
                write!(
                    f,
                    ". Write the server's address like `192.168.1.20` or `example.com`. \
                     The port can be left out when the server uses {DEFAULT_PORT}, the usual \
                     one; otherwise add it after a colon, like `192.168.1.20:7000`."
                )
            }
            Self::Closed {
                address,
                why: Closed::NoAnswer,
            } => write!(
                f,
                "Could not join {address}: no server answered. Check that the server is \
                 running (`fossps serve`), that the address and port are right, and that \
                 the server's firewall and router let UDP (not TCP) through on that port."
            ),
            // Only a player who has not joined yet is sent files.
            Self::Closed {
                address,
                why: why @ Closed::DamagedFile,
            } => write!(f, "Could not join {address}: {why}"),
            Self::Closed { address, why } => write!(f, "The game at {address} ended: {why}"),
            Self::Level { address, problem } => {
                write!(f, "Could not join {address}: {problem}")
            }
        }
    }
}

impl std::error::Error for JoinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Address { error, .. } => Some(error),
            Self::Closed { .. } | Self::Level { .. } => None,
        }
    }
}

/// Why a player's stay is over.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    /// The server or the network ended it.
    Closed(Closed),
    /// The server's level could not be used; this is what is wrong with it.
    Level(String),
}

/// What a player takes from a server before joining: the level and the map.
#[derive(Debug)]
struct Arrivals {
    /// Whether to fetch what the server offers. A player in the server's
    /// own program has it all already.
    wanted: bool,
    /// Where fetched levels are kept, if anywhere.
    cache: Option<PathBuf>,
    /// The hash of the level in hand, fetched or given.
    level_held: Option<[u8; 32]>,
    /// The hash of the map in hand.
    map_held: Option<[u8; 32]>,
    /// The caller gave the level itself, so the server's is not fetched.
    level_given: bool,
    /// The caller gave the map itself.
    map_given: bool,
    /// What the server last said it offers.
    offer: Offer,
    /// A level that has arrived, until the game takes it to walk.
    level: Option<Scene>,
    /// A map that has arrived, until the game takes it.
    map: Option<Map>,
    /// When the player was last told how the fetch is going.
    told: Instant,
    /// What went wrong, if the server's level cannot be used.
    problem: Option<String>,
}

impl Arrivals {
    fn new(wanted: bool) -> Self {
        Self {
            wanted,
            cache: crate::cache::folder(),
            level_held: None,
            map_held: None,
            level_given: false,
            map_given: false,
            offer: Offer::default(),
            level: None,
            map: None,
            told: Instant::now(),
            problem: None,
        }
    }

    /// The server said what it offers. Gives the slot to fetch first, or
    /// `None` when everything wanted is in hand.
    fn offered(&mut self, offer: Offer) -> Option<usize> {
        self.offer = offer;
        self.next()
    }

    /// The slot to fetch next, or `None` when everything wanted is in hand
    /// or something has gone wrong.
    fn next(&mut self) -> Option<usize> {
        if !self.wanted || self.problem.is_some() {
            return None;
        }
        if let Some(file) = self.offer.file(LEVEL_SLOT)
            && !self.level_given
            && self.level_held != Some(file.hash)
        {
            if file.len as usize > MAX_SHARED_BYTES {
                self.problem = Some(format!(
                    "its level is {} MB, and the most a server can send a player is {} MB. \
                     Ask whoever runs the server to make the level smaller.",
                    file.len / (1024 * 1024),
                    MAX_SHARED_BYTES / (1024 * 1024)
                ));
                return None;
            }
            let kept = self
                .cache
                .as_deref()
                .and_then(|folder| crate::cache::find(folder, file));
            let Some(kept) = kept else {
                log::info!(
                    "Getting the level from the server ({})...",
                    size_text(u64::from(file.len))
                );
                self.told = Instant::now();
                return Some(LEVEL_SLOT);
            };
            log::info!("This computer already has the server's level from an earlier visit.");
            self.take_level(&kept, file.hash);
            if self.problem.is_some() {
                return None;
            }
        }
        if let Some(file) = self.offer.file(MAP_SLOT)
            && !self.map_given
            && self.map_held != Some(file.hash)
        {
            if file.len as usize <= MOST_MAP_BYTES {
                return Some(MAP_SLOT);
            }
            // Only which way spawn points face is lost.
            self.map_held = Some(file.hash);
        }
        None
    }

    /// The file in `slot` arrived. Gives the slot to fetch next, as
    /// [`next`](Self::next) does.
    fn fetched(&mut self, slot: usize, file: &[u8]) -> Option<usize> {
        let hash = hash_of(file);
        if slot == LEVEL_SLOT {
            self.take_level(file, hash);
            if self.problem.is_none()
                && let Some(folder) = &self.cache
            {
                match crate::cache::keep(folder, file) {
                    Ok(path) => log::info!(
                        "Got the level. It is kept in {} so it is not fetched again.",
                        path.display()
                    ),
                    // The game is played all the same.
                    Err(error) => log::warn!(
                        "Got the level, but could not keep it in {} ({error}), so it will be \
                         fetched again next time.",
                        folder.display()
                    ),
                }
            }
        } else if slot == MAP_SLOT {
            self.map_held = Some(hash);
            // What a server sends is read as any stranger's file: a map
            // that does not read is done without.
            let read = std::str::from_utf8(file)
                .map_err(|error| error.to_string())
                .and_then(|text| Map::parse(MAP_FILE, text).map_err(|error| error.to_string()));
            match read {
                Ok(map) => self.map = Some(map),
                Err(error) => log::warn!(
                    "The server's map could not be read, so you may start out facing the \
                     wrong way: {error}"
                ),
            }
        }
        self.next()
    }

    /// Reads `file` as the level the server plays.
    fn take_level(&mut self, file: &[u8], hash: [u8; 32]) {
        match crate::level::from_bytes("the server's level", file) {
            Ok(level) => {
                self.level_held = Some(hash);
                self.level = Some(level);
            }
            Err(error) => {
                self.problem = Some(format!(
                    "the level it plays could not be read on this computer.\n{error}\nIf the \
                     server runs a newer FOSSPS than this, update this copy; otherwise tell \
                     whoever runs the server that its level does not load."
                ));
            }
        }
    }

    /// Tells the player how far along a fetch is, every couple of seconds.
    fn tell(&mut self, progress: Option<(u64, u64)>) {
        let Some((fetched, whole)) = progress else {
            return;
        };
        if self.told.elapsed() >= PROGRESS_EVERY {
            self.told = Instant::now();
            log::info!("{} of {} so far...", size_text(fetched), size_text(whole));
        }
    }
}

/// `bytes` as a person reads a file's size: `812 KB`, `12.3 MB`.
fn size_text(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A player's side of a game on a server.
///
/// Each tick it reads the server's news, moves its own player with what is
/// pressed without waiting for the server, and sends the server what it did.
#[derive(Debug)]
pub struct JoinedGame {
    link: UdpLink,
    client: Client,
    world: World,
    controls: Controls,
    address: String,
    yaw: f32,
    pitch: f32,
    closed: Option<Ended>,
    packet: Vec<u8>,
    /// The game's map as this side has it, for which way spawn points face.
    map: Map,
    /// The player has been turned the way their spawn point faces.
    faced: bool,
    /// What is fetched from the server before joining.
    arrivals: Arrivals,
    /// The level the server sent, until it is taken to be drawn.
    level: Option<Scene>,
}

impl JoinedGame {
    /// Starts joining the server at `address`: `203.0.113.5`,
    /// `example.com:24680` or `[2001:db8::1]`. Without a port,
    /// [`DEFAULT_PORT`] is used.
    ///
    /// The player is moved by the actions in `actions` called `move`,
    /// `look`, `jump`, `crouch`, `sprint`, `fire`, `aim` and `reload`.
    /// `tick_rate` must be the server's.
    ///
    /// Whether the server answered is known later: call
    /// [`wait_until_joined`](Self::wait_until_joined), or keep ticking and
    /// look at [`closed`](Self::closed).
    ///
    /// # Errors
    ///
    /// When the address cannot be read or its name is not known.
    pub fn connect(address: &str, actions: &ActionMap, tick_rate: u32) -> Result<Self, JoinError> {
        Self::connect_in(address, actions, World::flat(tick_rate), true)
    }

    fn connect_in(
        address: &str,
        actions: &ActionMap,
        world: World,
        fetches: bool,
    ) -> Result<Self, JoinError> {
        let address = with_port(address);
        Ok(Self {
            link: match UdpLink::connect(&address, LinkSettings::default()) {
                Ok(link) => link,
                Err(error) => return Err(JoinError::Address { address, error }),
            },
            client: Client::new(world.first(), world.step, ClientSettings::default()),
            world,
            controls: Controls::new(actions),
            address,
            yaw: 0.0,
            pitch: 0.0,
            closed: None,
            packet: Vec::new(),
            map: Map::default(),
            faced: false,
            arrivals: Arrivals::new(fetches),
            level: None,
        })
    }

    /// Keeps levels fetched from servers in `folder` instead of the usual
    /// place ([`cache::folder`](crate::cache::folder)).
    ///
    /// Call it before the first tick.
    #[must_use]
    pub fn with_cache(mut self, folder: impl Into<PathBuf>) -> Self {
        self.arrivals.cache = Some(folder.into());
        self
    }

    /// The level the server sent, to draw: put it in
    /// [`Config::level`](crate::Config). `None` if the server plays on the
    /// flat floor or shares no level, if the level was given with
    /// [`with_level`](Self::with_level), or once it has been taken.
    ///
    /// It is there by the time
    /// [`wait_until_joined`](Self::wait_until_joined) comes back. It is
    /// what the server chose to send, read with the same care as any file
    /// from a stranger.
    pub fn take_level(&mut self) -> Option<Scene> {
        self.level.take()
    }

    /// Gives this side the game's map file as the server has it
    /// ([`ScriptGame::map`]), so the player starts out looking the way
    /// their spawn point faces. Without it a player started at one of the
    /// map's spawn points looks north, along -Z.
    ///
    /// A player given no map takes the one the server sends, so this is
    /// only for a program that has the game already.
    ///
    /// Call it before the first tick.
    #[must_use]
    pub fn with_map(mut self, map: Map) -> Self {
        self.map = map;
        self.arrivals.map_given = true;
        self
    }

    /// Walks `level`, which must be the level the server plays
    /// ([`HostedGame::with_level`]): this side works out the player's moves
    /// against it without waiting for the server. Without this, or with a
    /// different level, the player still ends up where the server says, but
    /// is pulled back each time the two disagree, at a wall for one.
    ///
    /// A player given no level takes the one the server sends
    /// ([`HostedGame::share_level`]), so this is only for a program that
    /// has the game already; the server's level is then not fetched.
    ///
    /// Call it before the first tick.
    #[must_use]
    pub fn with_level(mut self, level: &Scene) -> Self {
        self.world = World::from_level(level, self.world.tick_rate);
        self.arrivals.level_given = true;
        self
    }

    /// Waits until the server has let this player in and gives the
    /// player's number. A server that does not answer is given up on after
    /// five seconds. One that shares its level
    /// ([`HostedGame::share_level`]) is waited on for as long as the level
    /// keeps arriving, which is logged as it goes; a level this computer
    /// has from an earlier visit is not fetched again. `meanwhile` is
    /// called over and over while waiting; a server in the same program
    /// must be given its turn there.
    ///
    /// # Errors
    ///
    /// When nothing answers, the server turns the player away, or the
    /// server's level does not arrive whole or cannot be read.
    pub fn wait_until_joined(
        &mut self,
        mut meanwhile: impl FnMut(),
    ) -> Result<PlayerId, JoinError> {
        loop {
            self.listen();
            if let Some(error) = self.closed() {
                return Err(error);
            }
            if let Some(id) = self.link.id() {
                return Ok(id);
            }
            meanwhile();
            std::thread::sleep(JOIN_POLL);
        }
    }

    /// Why this player is no longer on the server, once that is so. The
    /// game goes on being ticked but nothing moves.
    #[must_use]
    pub fn closed(&self) -> Option<JoinError> {
        let address = self.address.clone();
        Some(match self.closed.as_ref()? {
            Ended::Closed(why) => JoinError::Closed { address, why: *why },
            Ended::Level(problem) => JoinError::Level {
                address,
                problem: problem.clone(),
            },
        })
    }

    /// This player's number on the server, once joined.
    #[must_use]
    pub fn id(&self) -> Option<PlayerId> {
        self.link.id()
    }

    /// The player as this side has worked out, ahead of the server's word.
    #[must_use]
    pub fn player(&self) -> &Player {
        self.client.player()
    }

    /// Where to put the camera: the player's eyes, with corrections from
    /// the server eased in rather than jumped to.
    #[must_use]
    pub fn eye_position(&self) -> [f32; 3] {
        let [x, y, z] = self.client.view_position();
        [x, y + self.client.player().eye_height, z]
    }

    /// Which way the player faces, in radians, turning left from -Z as it
    /// grows.
    #[must_use]
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// How far the player looks up (positive) or down, in radians.
    #[must_use]
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// The other players as the server last told of them.
    pub fn others(&self) -> impl Iterator<Item = RemotePlayer> + '_ {
        self.client.others()
    }

    /// Takes in everything the server has sent since the last call.
    fn listen(&mut self) {
        while let Some(event) = self.link.poll() {
            let fetch = match event {
                LinkEvent::Joined(_) => None,
                LinkEvent::Offered(offer) => self.arrivals.offered(offer),
                LinkEvent::Fetched(slot, file) => self.arrivals.fetched(slot, file),
                // A snapshot that cannot be read is one more lost packet.
                LinkEvent::Packet(snapshot) => {
                    let _ = self
                        .client
                        .receive(&self.world.level, &self.world.rules, snapshot);
                    None
                }
                LinkEvent::Closed(why) => {
                    self.closed.get_or_insert(Ended::Closed(why));
                    None
                }
            };
            // What has arrived is taken up before a place is asked for.
            if let Some(level) = self.arrivals.level.take() {
                self.world = World::from_level(&level, self.world.tick_rate);
                self.level = Some(level);
            }
            if let Some(map) = self.arrivals.map.take() {
                self.map = map;
            }
            if let Some(problem) = self.arrivals.problem.take() {
                // No place is taken on a server whose level cannot be used.
                self.arrivals.wanted = false;
                self.closed.get_or_insert(Ended::Level(problem));
                self.link.leave();
            } else if let Some(slot) = fetch {
                self.link.fetch(slot);
            }
        }
        self.arrivals.tell(self.link.progress());
        // The server's first word says where this player starts: look the
        // way that spawn point faces.
        if !self.faced && self.client.server_tick() > 0 {
            self.faced = true;
            let at = self.client.player().position;
            if let Some(yaw) = self.world.facing_at(&self.map, at) {
                self.yaw = yaw;
            }
        }
    }

    /// What the player is asking for this tick. Turns the view as well.
    fn read(&mut self, input: &InputFrame, step: f32) -> PlayerInput {
        let held = |action: Option<ActionId>| action.is_some_and(|action| input.pressed(action));
        let controls = self.controls;
        let [right, forward] = controls.walk.map_or([0.0; 2], |walk| input.axis2d(walk));
        if let Some(look) = controls.look {
            let (mouse, stick) = (input.delta(look), input.axis2d(look));
            // Yaw grows turning left; the mouse and stick report right.
            let turn = mouse[0] * MOUSE_TURN + stick[0] * STICK_TURN * step;
            let tilt = mouse[1] * MOUSE_TURN + stick[1] * STICK_TURN * step;
            if turn.is_finite() && tilt.is_finite() {
                self.yaw = (self.yaw - turn).rem_euclid(TAU);
                self.pitch = (self.pitch + tilt).clamp(-FRAC_PI_2, FRAC_PI_2);
            }
        }
        PlayerInput {
            movement: MoveInput {
                forward,
                right,
                yaw: self.yaw,
                jump: held(controls.jump),
                crouch: held(controls.crouch),
                sprint: held(controls.sprint),
            },
            pitch: self.pitch,
            fire: held(controls.fire),
            aim: held(controls.aim),
            reload: held(controls.reload),
        }
    }
}

impl Game for JoinedGame {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.listen();
        // Nothing moves until the server has said where this player starts:
        // a step taken from a guessed place would be taken back.
        if self.closed.is_some() || self.link.id().is_none() || !self.faced {
            return;
        }
        let asked = self.read(input, step);
        self.client.predict(
            &self.world.level,
            &self.world.rules,
            &asked,
            &mut self.packet,
        );
        self.link.send(&self.packet);
    }

    fn frame(&mut self, frame: &mut Frame) {
        if self.closed.is_some() {
            frame.close();
            return;
        }
        frame.look_from(self.eye_position(), self.yaw, self.pitch);
    }
}

/// A server and one player joined to it, in one program, talking over this
/// machine's own network. [`start`](Self::start) is a game played alone;
/// [`open`](Self::open) lets other players join it too.
#[derive(Debug)]
pub struct LocalGame {
    hosted: HostedGame,
    joined: JoinedGame,
}

impl LocalGame {
    /// Starts `game` and joins it. The player walks `level`, usually the
    /// game's `level.glb` as [`level::load`](crate::level::load) reads it
    /// (see [`HostedGame::with_level`]), or a flat floor with `None`.
    /// `actions` and `tick_rate` must be the ones the game is then run with.
    ///
    /// # Errors
    ///
    /// When this machine will not open a port for the game, or the game
    /// cannot be joined through it; neither is expected.
    pub fn start(
        game: ScriptGame,
        level: Option<&Scene>,
        actions: &ActionMap,
        tick_rate: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let world = World::of(level, game.map(), tick_rate);
        Self::join_own(HostedGame::open_in(game, "127.0.0.1:0", world)?, actions)
    }

    /// Starts `game` for other players to join on the UDP port `port`, and
    /// joins it. `name` is what players looking for a game are shown, and
    /// `level` is as for [`start`](Self::start). `actions` and `tick_rate`
    /// must be the ones the game is then run with.
    ///
    /// # Errors
    ///
    /// When the port cannot be opened, most often because another server
    /// already has it, or the game cannot be joined through it.
    pub fn open(
        game: ScriptGame,
        level: Option<&Scene>,
        name: &str,
        port: u16,
        actions: &ActionMap,
        tick_rate: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let world = World::of(level, game.map(), tick_rate);
        let hosted = HostedGame::open_in(game, &format!("0.0.0.0:{port}"), world)?.named(name);
        Self::join_own(hosted, actions)
    }

    /// Joins `hosted` from this program, over this machine's own network,
    /// walking the very level the server has.
    fn join_own(
        mut hosted: HostedGame,
        actions: &ActionMap,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let port = hosted.host.local_addr()?.port();
        let address = format!("127.0.0.1:{port}");
        let mut joined = JoinedGame::connect_in(&address, actions, hosted.world.clone(), false)?
            .with_map(hosted.game.map().clone());
        joined.wait_until_joined(|| hosted.listen())?;
        Ok(Self { hosted, joined })
    }

    /// Hands `file`, the bytes of the level file the game is played in, to
    /// every player who joins without it: see
    /// [`HostedGame::share_level`].
    ///
    /// # Errors
    ///
    /// When the file is empty or holds more than 256 MiB, the most a
    /// player is sent.
    pub fn share_level(mut self, file: impl Into<Arc<[u8]>>) -> Result<Self, ShareError> {
        self.hosted = self.hosted.share_level(file)?;
        Ok(self)
    }

    /// The server's side.
    #[must_use]
    pub fn hosted(&self) -> &HostedGame {
        &self.hosted
    }

    /// The player's side.
    #[must_use]
    pub fn joined(&self) -> &JoinedGame {
        &self.joined
    }
}

impl Game for LocalGame {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.hosted.tick(input, step);
        self.joined.tick(input, step);
    }

    fn frame(&mut self, frame: &mut Frame) {
        self.hosted.frame(frame);
        self.joined.frame(frame);
    }
}

/// Looks for `wait` (usually [`SEARCH_TIME`]) for games being served on
/// this computer and its local network on the UDP port `port` (usually
/// [`DEFAULT_PORT`]), and gives the ones that answered, first to answer
/// first. A game on this computer is found at `127.0.0.1`.
///
/// Only games reached without a router in between are found; one
/// elsewhere on the internet is joined by its address.
///
/// # Errors
///
/// When this machine will not open a socket to search with.
pub fn find_servers(port: u16, wait: Duration) -> Result<Vec<FoundServer>, SocketError> {
    let mut search = UdpSearch::start(port)?;
    let mut found = Vec::new();
    loop {
        while let Some(server) = search.poll() {
            found.push(server);
        }
        if search.elapsed() >= wait {
            return Ok(found);
        }
        std::thread::sleep(JOIN_POLL);
    }
}

/// `address` with [`DEFAULT_PORT`] added if it names no port.
///
/// `203.0.113.5`, `example.com`, `[2001:db8::1]` and a bare `2001:db8::1`
/// have no port; `203.0.113.5:7000` and `[2001:db8::1]:7000` do.
#[must_use]
pub fn with_port(address: &str) -> String {
    let address = address.trim();
    if address.parse::<SocketAddr>().is_ok() {
        return address.to_owned();
    }
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => format!("{ip}:{DEFAULT_PORT}"),
        Ok(IpAddr::V6(ip)) => format!("[{ip}]:{DEFAULT_PORT}"),
        // A name with a port, or something only the lookup can judge.
        Err(_) if address.contains(':') && !address.ends_with(']') => address.to_owned(),
        Err(_) => format!("{address}:{DEFAULT_PORT}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_played_alone_rumbles_the_players_gamepad() {
        use fossps_input::{GamepadButton, Input, Rumble};

        let actions = ActionMap::default_fps();
        let script = ScriptGame::from_source(
            r#"game.onTick(function()
                if game.input.justPressed("fire") then game.input.rumble(0.2, 0.6, 0.1) end
            end)"#,
            &actions,
        );
        let mut game = LocalGame::start(script, None, &actions, 60).unwrap();
        let mut input = Input::new(actions);
        let asked = |game: &mut LocalGame, input: &mut Input| {
            game.tick(input.tick(), 1.0 / 60.0);
            let mut frame = Frame::new(0.0);
            game.frame(&mut frame);
            frame.rumble_requested()
        };

        assert_eq!(asked(&mut game, &mut input), None);
        input.press(GamepadButton::RightTrigger);
        assert_eq!(
            asked(&mut game, &mut input),
            Some(Rumble::new(0.2, 0.6, 0.1))
        );
        assert_eq!(asked(&mut game, &mut input), None);
    }

    /// A marker called `name` at `position`, turned `yaw` radians to the left.
    fn marker(name: &str, position: [f32; 3], yaw: f32) -> Marker {
        let (sin, cos) = yaw.sin_cos();
        let [x, y, z] = position;
        Marker {
            name: Some(name.to_owned()),
            transform: [
                [cos, 0.0, -sin, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [sin, 0.0, cos, 0.0],
                [x, y, z, 1.0],
            ],
            extras: None,
        }
    }

    /// A level that is a floor 20 m on a side at height zero, with `markers`.
    fn floor_with(markers: Vec<Marker>) -> Scene {
        use fossps_assets::{Instance, Mesh, MeshPart};

        let mut scene = Scene::default();
        scene.meshes.push(Mesh {
            name: None,
            parts: vec![MeshPart {
                positions: vec![
                    [-10.0, 0.0, -10.0],
                    [10.0, 0.0, -10.0],
                    [10.0, 0.0, 10.0],
                    [-10.0, 0.0, 10.0],
                ],
                indices: vec![0, 2, 1, 0, 3, 2],
                ..MeshPart::default()
            }],
        });
        scene.instances.push(Instance {
            mesh: 0,
            transform: marker("", [0.0; 3], 0.0).transform,
            mirrored: false,
            name: None,
            extras: None,
        });
        scene.markers = markers;
        scene
    }

    #[test]
    fn markers_whose_names_begin_with_spawn_are_where_players_start() {
        let level = floor_with(vec![
            marker("Camera Spot", [1.0, 0.0, 1.0], 0.0),
            marker("spawn", [2.0, 0.0, 3.0], 0.0),
            marker("Spawn Red", [-4.0, 0.5, 0.0], FRAC_PI_2),
            marker("respawn timer", [5.0, 0.0, 5.0], 0.0),
            marker("spawn.001", [0.0, 0.0, -6.0], TAU / 2.0),
        ]);
        let world = World::from_level(&level, 60);

        let places: Vec<_> = world.spawns.iter().map(|point| point.position).collect();
        assert_eq!(
            places,
            [[2.0, 0.0, 3.0], [-4.0, 0.5, 0.0], [0.0, 0.0, -6.0]]
        );
        // A player looks the way the marker is turned: left is west (-X).
        assert_eq!(
            world.facing_at(&Map::default(), [2.0, 0.01, 3.1]),
            Some(0.0)
        );
        let west = world.facing_at(&Map::default(), [-4.0, 0.5, 0.0]).unwrap();
        assert!((west - FRAC_PI_2).abs() < 1.0e-6, "{west}");
        let south = world.facing_at(&Map::default(), [0.0, 0.0, -6.0]).unwrap();
        assert!((south - TAU / 2.0).abs() < 1.0e-6, "{south}");
        assert_eq!(world.facing_at(&Map::default(), [7.0, 0.0, 7.0]), None);
        assert_eq!(try_walking(&level, &Map::default()).spawn_points, 3);
        assert_eq!(
            try_walking(&level, &Map::default()).problems,
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_new_player_is_put_at_a_spawn_point_away_from_the_others() {
        let level = floor_with(vec![
            marker("spawn", [-9.0, 0.0, 0.0], 0.0),
            marker("spawn.001", [9.0, 0.0, 0.0], 0.0),
        ]);
        let world = World::from_level(&level, 60);
        for seed in 0..20 {
            let first = world.spawn_in(&Map::default(), std::iter::empty(), seed);
            assert_eq!(first.position[0].abs(), 9.0);
            let second = world.spawn_in(&Map::default(), std::iter::once(&first), seed);
            assert_eq!(second.position[0], -first.position[0], "seed {seed}");
            // The same question has the same answer.
            assert_eq!(
                world.spawn_in(&Map::default(), std::iter::once(&first), seed),
                second
            );
        }
    }

    #[test]
    fn a_level_with_no_spawn_points_starts_players_at_its_middle_and_says_so() {
        let level = floor_with(vec![marker("Camera Spot", [1.0, 0.0, 1.0], 0.0)]);
        let world = World::from_level(&level, 60);
        assert_eq!(
            world
                .spawn_in(&Map::default(), std::iter::empty(), 7)
                .position,
            [0.0; 3]
        );

        let walked = try_walking(&level, &Map::default());
        assert_eq!(walked.spawn_points, 0);
        assert_eq!(walked.problems.len(), 1, "{:?}", walked.problems);
        let said = &walked.problems[0];
        assert!(
            said.starts_with(
                "This game has no spawn points, so players start at the level's middle"
            ),
            "{said}"
        );
        assert!(said.contains("an empty object named `spawn`"), "{said}");
    }

    #[test]
    fn a_spawn_point_with_no_floor_under_it_is_named() {
        let level = floor_with(vec![
            marker("spawn", [0.0, 2.0, 0.0], 0.0),
            marker("spawn.ledge", [30.0, 0.0, 0.5], 0.0),
        ]);
        let walked = try_walking(&level, &Map::default());
        assert_eq!(walked.problems.len(), 1, "{:?}", walked.problems);
        let said = &walked.problems[0];
        assert!(
            said.starts_with(
                "A player who starts at the spawn point \"spawn.ledge\" in level.glb \
                 (30.0, 0.0, 0.5) falls and has not landed after 3 seconds"
            ),
            "{said}"
        );
        assert!(said.contains("put a floor under it"), "{said}");
    }

    #[test]
    fn spawn_points_in_the_map_file_are_used_instead_of_the_levels_markers() {
        let level = floor_with(vec![marker("spawn", [-9.0, 0.0, 0.0], 0.0)]);
        let world = World::from_level(&level, 60);
        let map = Map::parse(
            "map.toml",
            "version = 1\n\n\
             [[entity]]\nid = \"aaaa\"\ntype = \"spawn\"\nname = \"Blue spawn\"\n\
             position = [5.0, 0.0, 5.0]\nfacing = 90.0\n\n\
             [[entity]]\nid = \"bbbb\"\ntype = \"spawn\"\nposition = [50.0, 0.0, 0.0]\n\n\
             [[entity]]\nid = \"cccc\"\ntype = \"zone\"\nposition = [1.0, 0.0, 1.0]\n",
        )
        .unwrap();

        // With an empty map the level's marker is where players start.
        let empty = Map::default();
        assert_eq!(
            world.spawn_in(&empty, std::iter::empty(), 3).position,
            [-9.0, 0.0, 0.0]
        );
        assert_eq!(world.facing_at(&empty, [-9.0, 0.0, 0.0]), Some(0.0));

        // With spawn points in the map, only they are.
        for seed in 0..20 {
            let x = world.spawn_in(&map, std::iter::empty(), seed).position[0];
            assert!(x == 5.0 || x == 50.0, "{x}");
        }
        let west = world.facing_at(&map, [5.0, 0.0, 5.2]).unwrap();
        assert!((west - FRAC_PI_2).abs() < 1.0e-6, "{west}");
        assert_eq!(world.facing_at(&map, [-9.0, 0.0, 0.0]), None);

        // The walk is tried from the map's points, and the one off the
        // floor is named by its file; one with no name goes by its id.
        let walked = try_walking(&level, &map);
        assert_eq!(walked.spawn_points, 2);
        assert_eq!(walked.problems.len(), 1, "{:?}", walked.problems);
        assert!(
            walked.problems[0].starts_with(
                "A player who starts at the spawn point \"bbbb\" in map.toml (50.0, 0.0, 0.0) \
                 falls"
            ),
            "{}",
            walked.problems[0]
        );
    }

    #[test]
    fn a_level_with_nothing_in_it_has_nothing_to_stand_on() {
        let mut level = Scene::default();
        level.markers.push(marker("spawn", [0.0; 3], 0.0));
        let walked = try_walking(&level, &Map::default());
        assert_eq!(walked.problems.len(), 1, "{:?}", walked.problems);
        assert!(
            walked.problems[0].starts_with("level.glb has no shapes in it"),
            "{}",
            walked.problems[0]
        );
    }

    #[test]
    fn an_address_without_a_port_gets_the_usual_one() {
        for (written, meant) in [
            ("203.0.113.5", "203.0.113.5:24680"),
            (" 203.0.113.5:7000 ", "203.0.113.5:7000"),
            ("example.com", "example.com:24680"),
            ("example.com:7000", "example.com:7000"),
            ("localhost", "localhost:24680"),
            ("::1", "[::1]:24680"),
            ("[::1]", "[::1]:24680"),
            ("[2001:db8::1]:7000", "[2001:db8::1]:7000"),
        ] {
            assert_eq!(with_port(written), meant, "{written}");
        }
    }
}
