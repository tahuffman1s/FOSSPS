//! The netcode on real UDP sockets.
//!
//! A server holds one [`UdpHost`], a player one [`UdpLink`]. Neither ever
//! waits: [`UdpHost::poll`] and [`UdpLink::poll`] hand over what has
//! arrived and come straight back, so both fit in a game's tick.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Instant;

use crate::connect::{
    Closed, DATA_HEADER, Gone, Heard, Host, HostSettings, JoinRequest, Link, LinkHeard,
    LinkSettings, MAX_DATAGRAM_BYTES, Refusal,
};
use crate::snapshot::PlayerId;

/// How many times a goodbye is sent, since any one copy may be lost.
const GOODBYES: usize = 3;

/// How many socket errors one `poll` shrugs off before coming back.
const MOST_ERRORS: usize = 16;

/// How many datagrams are read before `poll` says "nothing more" whatever
/// is still waiting, so a flood of rubbish cannot hold up the tick. The
/// rest is read on the next tick. A full server's players send 64 a tick.
const MOST_DATAGRAMS: usize = 1024;

/// Room for one datagram and one byte more, so one that is too long is
/// seen to be too long instead of being cut to fit.
const INBOX_BYTES: usize = MAX_DATAGRAM_BYTES + 1;

/// Why a socket could not be opened.
#[derive(Debug)]
pub struct SocketError {
    hosting: bool,
    address: String,
    source: io::Error,
}

impl fmt::Display for SocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            hosting,
            address,
            source,
        } = self;
        if *hosting {
            write!(
                f,
                "could not open `{address}` for players to join: {source}. Another server may \
                 already be using that port; stop it or choose another port."
            )
        } else {
            write!(
                f,
                "could not find a server at `{address}`: {source}. Write the address with its \
                 port, like `203.0.113.5:27015` or `example.com:27015`."
            )
        }
    }
}

impl std::error::Error for SocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Reads the next datagram, or `None` when nothing more is waiting.
///
/// Errors other than "nothing waiting" are stepped over: Windows reports
/// that an earlier datagram found nobody listening as an error on the
/// next read, and that must not stop the others being read.
fn next_datagram<T>(budget: &mut Budget, mut read: impl FnMut() -> io::Result<T>) -> Option<T> {
    while budget.datagrams < MOST_DATAGRAMS && budget.errors <= MOST_ERRORS {
        match read() {
            Ok(read) => {
                budget.datagrams += 1;
                return Some(read);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return None,
            Err(_) => budget.errors += 1,
        }
    }
    None
}

/// What has been read since `poll` last said "nothing more".
#[derive(Debug, Default)]
struct Budget {
    datagrams: usize,
    errors: usize,
}

/// Something a server's game should act on, from [`UdpHost::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEvent<'a> {
    /// Somebody wants to join. Answer before the next `poll` with
    /// [`UdpHost::accept`] or [`UdpHost::refuse`]. One address and port
    /// holds one place at a time; how many places one machine may take
    /// (see [`JoinRequest::address`]) is for the game to decide.
    Join(JoinRequest),
    /// A packet of commands from a player, for
    /// [`Server::receive`](crate::Server::receive).
    Packet(PlayerId, &'a [u8]),
    /// A player is gone. Call [`Server::leave`](crate::Server::leave).
    Left(PlayerId, Gone),
}

/// A server's UDP socket and the players joined through it.
///
/// ```no_run
/// use fossps_net::{HostEvent, HostSettings, Refusal, Server, ServerSettings, UdpHost};
/// use fossps_physics::{Level, MoveSettings, Player};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let level = Level::from_triangles(&[]);
/// let rules = MoveSettings::default();
/// let mut server = Server::new(1.0 / 60.0, ServerSettings::default());
/// let mut host = UdpHost::bind("0.0.0.0:27015", HostSettings::default())?;
/// let mut packet = Vec::new();
///
/// loop {
///     // Once per tick:
///     while let Some(event) = host.poll() {
///         match event {
///             HostEvent::Join(request) => {
///                 match server.join(Player::at([0.0, 1.0, 0.0], &rules)) {
///                     Ok(id) => host.accept(request, id),
///                     Err(_) => host.refuse(request, Refusal::Full),
///                 }
///             }
///             // A packet that cannot be read is thrown away.
///             HostEvent::Packet(id, commands) => drop(server.receive(id, commands)),
///             HostEvent::Left(id, _) => server.leave(id),
///         }
///     }
///     server.tick(&level, &rules);
///     for id in host.players() {
///         server.write_snapshot(id, &mut packet)?;
///         host.send(id, &packet);
///     }
/// #   break;
/// }
/// # Ok(())
/// # }
/// ```
///
/// Dropping the host tells every player the server has stopped.
#[derive(Debug)]
pub struct UdpHost {
    socket: UdpSocket,
    host: Host,
    started: Instant,
    inbox: Box<[u8]>,
    outbox: Vec<u8>,
    budget: Budget,
}

impl UdpHost {
    /// Opens a socket for players to join through. `address` is an
    /// address of this machine and a port: `0.0.0.0:27015` takes players
    /// from anywhere, `127.0.0.1:27015` only from this machine, and port
    /// `0` lets the system choose one ([`local_addr`](Self::local_addr)
    /// says which).
    ///
    /// # Errors
    ///
    /// When the address cannot be read or the port cannot be opened,
    /// most often because another program has it.
    pub fn bind(address: &str, settings: HostSettings) -> Result<Self, SocketError> {
        let open = || {
            let socket = UdpSocket::bind(address)?;
            socket.set_nonblocking(true)?;
            Ok(socket)
        };
        Ok(Self {
            socket: open().map_err(|source| SocketError {
                hosting: true,
                address: address.to_owned(),
                source,
            })?,
            host: Host::new(settings),
            started: Instant::now(),
            inbox: vec![0; INBOX_BYTES].into(),
            outbox: Vec::with_capacity(MAX_DATAGRAM_BYTES),
            budget: Budget::default(),
        })
    }

    /// The address and port the socket is open on.
    ///
    /// # Errors
    ///
    /// When the system cannot say, which it always can for an open
    /// socket.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The next thing the game should act on, or `None` when everything
    /// that has arrived is dealt with. Never waits. Call it in a loop
    /// once per tick.
    pub fn poll(&mut self) -> Option<HostEvent<'_>> {
        let now = self.started.elapsed();
        while let Some((len, from)) =
            next_datagram(&mut self.budget, || self.socket.recv_from(&mut self.inbox))
        {
            let datagram = self.inbox.get(..len).unwrap_or_default();
            let heard = self.host.receive(now, from, datagram, &mut self.outbox);
            if !self.outbox.is_empty() {
                // A datagram that cannot be sent is one more lost one.
                let _ = self.socket.send_to(&self.outbox, from);
            }
            match heard {
                Some(Heard::Join(request)) => return Some(HostEvent::Join(request)),
                Some(Heard::Packet(id)) => {
                    let packet = self.inbox.get(DATA_HEADER..len).unwrap_or_default();
                    return Some(HostEvent::Packet(id, packet));
                }
                Some(Heard::Left(id)) => return Some(HostEvent::Left(id, Gone::Quit)),
                None => {}
            }
        }
        if let Some(id) = self.host.timed_out(now) {
            return Some(HostEvent::Left(id, Gone::TimedOut));
        }
        self.budget = Budget::default();
        None
    }

    /// Gives the player behind `request` the place `id`, the number
    /// [`Server::join`](crate::Server::join) gave, and tells them.
    pub fn accept(&mut self, request: JoinRequest, id: PlayerId) {
        self.host
            .accept(self.started.elapsed(), request, id, &mut self.outbox);
        if !self.outbox.is_empty() {
            let _ = self.socket.send_to(&self.outbox, request.address());
        }
    }

    /// Turns the player behind `request` away and tells them why.
    pub fn refuse(&mut self, request: JoinRequest, why: Refusal) {
        Host::refuse(request, why, &mut self.outbox);
        let _ = self.socket.send_to(&self.outbox, request.address());
    }

    /// Sends a snapshot to the player `id`. Does nothing if nobody has
    /// that place.
    pub fn send(&mut self, id: PlayerId, packet: &[u8]) {
        if let Some(to) = self.host.wrap(id, packet, &mut self.outbox) {
            let _ = self.socket.send_to(&self.outbox, to);
        }
    }

    /// Takes the player `id` off and tells them why. The game still has
    /// to call [`Server::leave`](crate::Server::leave); no
    /// [`HostEvent::Left`] follows.
    pub fn kick(&mut self, id: PlayerId, why: Refusal) {
        if let Some(to) = self.host.drop_player(id, why, &mut self.outbox) {
            for _ in 0..GOODBYES {
                let _ = self.socket.send_to(&self.outbox, to);
            }
        }
    }

    /// The numbers of the players joined, lowest first.
    pub fn players(&self) -> impl Iterator<Item = PlayerId> + use<> {
        let mut joined = 0_u64;
        for id in self.host.players() {
            joined |= 1 << id;
        }
        (0..64).filter(move |id| joined & (1 << id) != 0)
    }

    /// The address the player `id` joined from.
    #[must_use]
    pub fn address(&self, id: PlayerId) -> Option<SocketAddr> {
        self.host.address(id)
    }
}

impl Drop for UdpHost {
    fn drop(&mut self) {
        for id in self.players() {
            self.kick(id, Refusal::Stopping);
        }
    }
}

/// Something a player's game should act on, from [`UdpLink::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent<'a> {
    /// The server gave this player a place, with this number.
    Joined(PlayerId),
    /// A snapshot from the server, for
    /// [`Client::receive`](crate::Client::receive).
    Packet(&'a [u8]),
    /// The game is over for this player; show them why. Nothing more
    /// comes from this link.
    Closed(Closed),
}

/// A player's UDP socket and their stay on one server.
///
/// ```no_run
/// use fossps_net::{Client, ClientSettings, LinkEvent, LinkSettings, PlayerInput, UdpLink};
/// use fossps_physics::{Level, MoveSettings, Player};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let level = Level::from_triangles(&[]);
/// let rules = MoveSettings::default();
/// let spawn = Player::at([0.0, 1.0, 0.0], &rules);
/// let mut client = Client::new(spawn, 1.0 / 60.0, ClientSettings::default());
/// let mut link = UdpLink::connect("127.0.0.1:27015", LinkSettings::default())?;
/// let mut packet = Vec::new();
///
/// loop {
///     // Once per tick:
///     while let Some(event) = link.poll() {
///         match event {
///             LinkEvent::Joined(_) => println!("joined"),
///             // A packet that cannot be read is thrown away.
///             LinkEvent::Packet(snapshot) => drop(client.receive(&level, &rules, snapshot)),
///             LinkEvent::Closed(why) => {
///                 println!("{why}");
///                 return Ok(());
///             }
///         }
///     }
///     if link.id().is_some() {
///         client.predict(&level, &rules, &PlayerInput::default(), &mut packet);
///         link.send(&packet);
///     }
/// #   break;
/// }
/// # Ok(())
/// # }
/// ```
///
/// Dropping the link tells the server this player has left.
#[derive(Debug)]
pub struct UdpLink {
    socket: UdpSocket,
    link: Link,
    started: Instant,
    inbox: Box<[u8]>,
    outbox: Vec<u8>,
    budget: Budget,
}

impl UdpLink {
    /// Starts joining the server at `address`, an address or a name and
    /// a port: `203.0.113.5:27015` or `example.com:27015`. A name with
    /// both kinds of address is joined over IPv4. Looking a name
    /// up may take a moment; nothing else here waits. Whether the server
    /// answered is told later, by [`poll`](Self::poll).
    ///
    /// # Errors
    ///
    /// When the address cannot be read or the name is not known.
    pub fn connect(address: &str, settings: LinkSettings) -> Result<Self, SocketError> {
        let open = || {
            // A name such as `localhost` often has an IPv6 address listed
            // first, and a server opened on `0.0.0.0` does not hear it.
            let mut found = address.to_socket_addrs()?.peekable();
            let first = found.peek().copied();
            let server = found.find(SocketAddr::is_ipv4).or(first).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "the name has no address")
            })?;
            let socket = if server.is_ipv4() {
                UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            } else {
                UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0))
            }?;
            // Only datagrams from the server are handed over after this.
            socket.connect(server)?;
            socket.set_nonblocking(true)?;
            Ok(socket)
        };
        let started = Instant::now();
        Ok(Self {
            socket: open().map_err(|source| SocketError {
                hosting: false,
                address: address.to_owned(),
                source,
            })?,
            link: Link::new(started.elapsed(), settings),
            started,
            inbox: vec![0; INBOX_BYTES].into(),
            outbox: Vec::with_capacity(MAX_DATAGRAM_BYTES),
            budget: Budget::default(),
        })
    }

    /// The next thing the game should act on, or `None` when everything
    /// that has arrived is dealt with. Never waits. Call it in a loop
    /// once per tick, joined or not: it is also what keeps asking the
    /// server for a place.
    pub fn poll(&mut self) -> Option<LinkEvent<'_>> {
        let now = self.started.elapsed();
        while let Some(len) = next_datagram(&mut self.budget, || self.socket.recv(&mut self.inbox))
        {
            let datagram = self.inbox.get(..len).unwrap_or_default();
            match self.link.receive(now, datagram) {
                Some(LinkHeard::Joined(id)) => return Some(LinkEvent::Joined(id)),
                Some(LinkHeard::Packet) => {
                    let packet = self.inbox.get(DATA_HEADER..len).unwrap_or_default();
                    return Some(LinkEvent::Packet(packet));
                }
                Some(LinkHeard::Closed(why)) => return Some(LinkEvent::Closed(why)),
                None => {}
            }
        }
        let closed = self.link.update(now, &mut self.outbox);
        if !self.outbox.is_empty() {
            let _ = self.socket.send(&self.outbox);
        }
        if closed.is_none() {
            self.budget = Budget::default();
        }
        closed.map(LinkEvent::Closed)
    }

    /// Sends a packet of commands to the server. Does nothing until
    /// joined. Send one every tick: a server that hears nothing for a
    /// while takes the player to be gone.
    pub fn send(&mut self, packet: &[u8]) {
        if self.link.wrap(packet, &mut self.outbox) {
            let _ = self.socket.send(&self.outbox);
        }
    }

    /// Leaves the server and tells it so. Dropping the link does the
    /// same.
    pub fn leave(&mut self) {
        if self.link.leave(&mut self.outbox) {
            for _ in 0..GOODBYES {
                let _ = self.socket.send(&self.outbox);
            }
        }
    }

    /// This player's number on the server, once joined and until the
    /// stay ends.
    #[must_use]
    pub fn id(&self) -> Option<PlayerId> {
        self.link.id()
    }

    /// Why the stay ended, once it has.
    #[must_use]
    pub fn closed(&self) -> Option<Closed> {
        self.link.closed()
    }

    /// The address and port this side's socket is open on.
    ///
    /// # Errors
    ///
    /// When the system cannot say, which it always can for an open
    /// socket.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Drop for UdpLink {
    fn drop(&mut self) {
        self.leave();
    }
}
