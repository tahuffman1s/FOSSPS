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
//! [`LocalGame`] is both in one program, which is what `fossps run` plays:
//! the same code runs alone as runs over the internet.
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
//!     serve(&config, &mut hosted, &StopSignal::on_ctrl_c()?);
//!     Ok(())
//! }
//! ```

use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use fossps_input::{ActionId, ActionMap, InputFrame};
use fossps_net::{
    Client, ClientSettings, Closed, HostEvent, HostSettings, LinkEvent, LinkSettings, MAX_PLAYERS,
    PlayerId, PlayerInput, Refusal, RemotePlayer, Server, ServerSettings, SocketError, UdpHost,
    UdpLink,
};
use fossps_physics::{Level, MoveInput, MoveSettings, Player};

use crate::game::{Frame, Game};
use crate::script_game::ScriptGame;

/// The UDP port a server opens and a player joins when none is named.
pub const DEFAULT_PORT: u16 = 24680;

/// Radians turned per count of mouse movement.
const MOUSE_TURN: f32 = 0.002;

/// Radians turned per second with the look stick pushed all the way.
const STICK_TURN: f32 = 3.5;

/// Half the width of the floor players stand on until a game has a level.
const FLOOR_REACH: f32 = 500.0;

/// How long to wait between looks at the socket while joining.
const JOIN_POLL: Duration = Duration::from_millis(5);

/// How many ticks pass between changes to the window's title.
const TITLE_EVERY: u32 = 15;

/// What a server and its players must agree on to work out the same moves.
#[derive(Debug, Clone)]
struct World {
    level: Level,
    rules: MoveSettings,
    /// Seconds in one tick.
    step: f32,
}

impl World {
    /// A flat floor, 1 km on a side, at height zero.
    fn flat(tick_rate: u32) -> Self {
        let r = FLOOR_REACH;
        let corners = [[-r, 0.0, -r], [r, 0.0, -r], [r, 0.0, r], [-r, 0.0, r]];
        let [a, b, c, d] = corners;
        Self {
            level: Level::from_triangles(&[[a, b, c], [a, c, d]]),
            rules: MoveSettings::default(),
            step: 1.0 / tick_rate.max(1) as f32,
        }
    }

    /// A player where everyone comes into the world.
    fn spawn(&self) -> Player {
        Player::at([0.0, 0.0, 0.0], &self.rules)
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
    packet: Vec<u8>,
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
        let world = World::flat(tick_rate);
        Ok(Self {
            game,
            host: UdpHost::bind(address, HostSettings::default())?,
            server: Server::new(world.step, ServerSettings::default()),
            world,
            names: [None; MAX_PLAYERS],
            packet: Vec::new(),
        })
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
            ..
        } = self;
        while let Some(event) = host.poll() {
            match event {
                HostEvent::Join(request) => match server.join(world.spawn()) {
                    Ok(id) => {
                        host.accept(request, id);
                        let name = format!("Player {}", u32::from(id) + 1);
                        log::info!("{name} joined from {}.", request.address());
                        if let Some(slot) = names.get_mut(usize::from(id)) {
                            *slot = Some(game.host_mut().player_joined(&name));
                        }
                    }
                    Err(_) => host.refuse(request, Refusal::Full),
                },
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
    /// start of a round or after a death, back where players start.
    fn place_spawned(&mut self) {
        let Self {
            game,
            server,
            world,
            names,
            ..
        } = self;
        for spawned in game.host_mut().take_spawns() {
            let place = (0..)
                .zip(names.iter())
                .find(|(_, name)| **name == Some(spawned));
            if let Some(player) = place.and_then(|(id, _)| server.player_mut(id)) {
                *player = world.spawn();
            }
        }
    }
}

impl Game for HostedGame {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.listen();
        self.server.tick(&self.world.level, &self.world.rules);
        for id in self.host.players() {
            if self.server.write_snapshot(id, &mut self.packet).is_ok() {
                self.host.send(id, &self.packet);
            }
        }
        self.game.tick(input, step);
        self.place_spawned();
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
            Self::Closed { address, why } => write!(f, "The game at {address} ended: {why}"),
        }
    }
}

impl std::error::Error for JoinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Address { error, .. } => Some(error),
            Self::Closed { .. } => None,
        }
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
    closed: Option<Closed>,
    packet: Vec<u8>,
    /// The window's title without where the player is; empty to leave the
    /// title alone.
    title: String,
    ticks: u32,
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
        let address = with_port(address);
        let world = World::flat(tick_rate);
        Ok(Self {
            link: match UdpLink::connect(&address, LinkSettings::default()) {
                Ok(link) => link,
                Err(error) => return Err(JoinError::Address { address, error }),
            },
            client: Client::new(world.spawn(), world.step, ClientSettings::default()),
            world,
            controls: Controls::new(actions),
            address,
            yaw: 0.0,
            pitch: 0.0,
            closed: None,
            packet: Vec::new(),
            title: String::new(),
            ticks: 0,
        })
    }

    /// Shows where the player is in the window's title, after `title`.
    /// A stand-in for seeing the level, until levels are drawn.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Waits until the server has let this player in, for five seconds at
    /// most, and gives the player's number. `meanwhile` is called over and
    /// over while waiting; a server in the same program must be given its
    /// turn there.
    ///
    /// # Errors
    ///
    /// When nothing answers or the server turns the player away.
    pub fn wait_until_joined(
        &mut self,
        mut meanwhile: impl FnMut(),
    ) -> Result<PlayerId, JoinError> {
        loop {
            self.listen();
            if let Some(why) = self.closed {
                return Err(self.error(why));
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
        self.closed.map(|why| self.error(why))
    }

    fn error(&self, why: Closed) -> JoinError {
        JoinError::Closed {
            address: self.address.clone(),
            why,
        }
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
        let Self {
            link,
            client,
            world,
            closed,
            ..
        } = self;
        while let Some(event) = link.poll() {
            match event {
                LinkEvent::Joined(_) => {}
                // A snapshot that cannot be read is one more lost packet.
                LinkEvent::Packet(snapshot) => {
                    let _ = client.receive(&world.level, &world.rules, snapshot);
                }
                LinkEvent::Closed(why) => *closed = Some(why),
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
        if self.closed.is_some() || self.link.id().is_none() {
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
        self.ticks = self.ticks.wrapping_add(1);
    }

    fn frame(&mut self, frame: &mut Frame) {
        if self.closed.is_some() {
            frame.close();
        } else if !self.title.is_empty() && self.ticks >= TITLE_EVERY {
            self.ticks = 0;
            let [x, y, z] = self.client.view_position();
            frame.set_title(format!(
                "{} | you are at {x:.1}, {y:.1}, {z:.1}",
                self.title
            ));
        }
    }
}

/// A game played alone: a server and one player joined to it, in one
/// program, talking over this machine's own network. Nobody else can join.
#[derive(Debug)]
pub struct LocalGame {
    hosted: HostedGame,
    joined: JoinedGame,
}

impl LocalGame {
    /// Starts `game` and joins it. `actions` and `tick_rate` must be the
    /// ones the game is then run with.
    ///
    /// # Errors
    ///
    /// When this machine will not open a port for the game, or the game
    /// cannot be joined through it; neither is expected.
    pub fn start(
        game: ScriptGame,
        actions: &ActionMap,
        tick_rate: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut hosted = HostedGame::open(game, "127.0.0.1:0", tick_rate)?;
        let address = hosted.host.local_addr()?;
        let mut joined = JoinedGame::connect(&address.to_string(), actions, tick_rate)?;
        joined.wait_until_joined(|| hosted.listen())?;
        Ok(Self { hosted, joined })
    }

    /// Shows where the player is in the window's title, after `title`.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.joined = self.joined.with_title(title);
        self
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
        self.joined.frame(frame);
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
