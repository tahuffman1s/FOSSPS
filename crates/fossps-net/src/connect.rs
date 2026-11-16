//! Joining, leaving and going silent: the few bytes wrapped around every
//! packet so a server knows which player an address is.
//!
//! Nothing here opens a socket or reads a clock. [`Host`] and [`Link`] are
//! handed each datagram and the time, and say what to send back, so the
//! whole exchange is tested without a network. [`crate::UdpHost`] and
//! [`crate::UdpLink`] put them on real sockets.
//!
//! Joining takes four datagrams:
//!
//! 1. The player asks, in a datagram padded to 1200 bytes.
//! 2. The server answers with a number worked out from the player's
//!    address and a secret, and remembers nothing.
//! 3. The player sends the number back, which proves the address is theirs.
//! 4. The server gives them a place.
//!
//! Until step 3 the server keeps nothing and never sends more bytes than
//! it was sent, so asking in somebody else's name gets that somebody an
//! 18-byte datagram for a 1200-byte one: no use for flooding them.
//! Afterwards every datagram carries the number, so a stranger who only
//! knows a player's address cannot send commands in their name.
//!
//! The answer in step 2 also lists the files the server offers, such as
//! the game's level ([`crate::share`]). A player who lacks one fetches it
//! between steps 2 and 3, before taking a place, asking with the same
//! number so a file is only ever sent to an address that has proved itself.

use std::fmt;
use std::hash::{BuildHasher, RandomState};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::find::{Notice, QUERY};
use crate::share::{Offer, Progress, ShareError, Shelf, Transfer, WINDOW};
use crate::snapshot::{MAX_PLAYERS, PlayerId};
use crate::wire::{MAX_PACKET_BYTES, Reader, VERSION, Writer};

/// A player asks to join. Padded to [`REQUEST_BYTES`].
const REQUEST: u8 = 16;
/// The server's answer to a request: a number to send back.
const CHALLENGE: u8 = 17;
/// The player sends the number back.
const RESPONSE: u8 = 18;
/// The server gives the player a place.
const ACCEPTED: u8 = 19;
/// The server turns the player away.
const REFUSED: u8 = 20;
/// A packet of commands or a snapshot, wrapped.
const DATA: u8 = 21;
/// Either side says it is going.
const GOODBYE: u8 = 22;

/// A player asks for pieces of a file the server offers.
const FETCH: u8 = 24;
/// One piece of a file, server to player.
const PIECE: u8 = 25;

/// Four bytes every request carries, so stray traffic is not answered.
const MAGIC: [u8; 4] = *b"FPSJ";

/// How long a request is. No answer to it is longer, so a server cannot
/// be made to send a third party more than was spent on asking.
const REQUEST_BYTES: usize = MAX_PACKET_BYTES;

/// The bytes in front of a wrapped packet: the kind, the version and the
/// number that proves who sent it.
pub(crate) const DATA_HEADER: usize = 10;

/// The most bytes one datagram holds: a full packet and its wrapping.
pub(crate) const MAX_DATAGRAM_BYTES: usize = MAX_PACKET_BYTES + DATA_HEADER;

/// How many seconds one secret number is worked out for. A number is
/// taken for this long at least and twice this long at most.
const COOKIE_SECONDS: u64 = 8;

/// How old a number may get before a player who is not joined yet asks for
/// a new one. A number is taken for [`COOKIE_SECONDS`] at least, so one
/// lost answer still leaves time for the next.
const COOKIE_REFRESH: Duration = Duration::from_secs(3);

/// How many times a server may change what it offers while one player is
/// joining before the player gives up. A map saved while somebody joins
/// changes it once.
const MOST_CHANGES: u32 = 8;

/// A gap between two looks at a link longer than this is the game being
/// busy, and does not count towards giving up on joining.
const AWAY: Duration = Duration::from_secs(1);

/// How many players who left are remembered, so a late copy of their
/// joining datagram does not put them back.
const REMEMBERED: usize = 64;

const FULL: u8 = 1;
const WRONG_VERSION: u8 = 2;
const STOPPED: u8 = 3;
const KICKED: u8 = 4;
const QUIT: u8 = 5;

/// How a server treats the players joined to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSettings {
    /// A player nothing has been heard from for this long is dropped.
    pub timeout: Duration,
}

impl Default for HostSettings {
    /// Ten seconds of silence drops a player.
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
        }
    }
}

/// How a player's side goes about joining and staying joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkSettings {
    /// How long to wait for an answer before asking to join again.
    pub resend: Duration,
    /// How long to keep asking before giving up.
    pub give_up: Duration,
    /// A server nothing has been heard from for this long is taken to be
    /// gone.
    pub timeout: Duration,
}

impl Default for LinkSettings {
    /// Ask ten times a second for five seconds; ten seconds of silence
    /// ends a joined game.
    fn default() -> Self {
        Self {
            resend: Duration::from_millis(100),
            give_up: Duration::from_secs(5),
            timeout: Duration::from_secs(10),
        }
    }
}

/// Why a player is no longer on a server, as the player's side sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    /// Nothing answered at that address.
    NoAnswer,
    /// The server stopped answering in the middle of a game.
    TimedOut,
    /// Every place on the server is taken.
    Full,
    /// The server runs another version of the engine.
    WrongVersion,
    /// The server was shut down.
    ServerStopped,
    /// The server took this player off.
    Kicked,
    /// This side left.
    Left,
    /// A file the server sent did not arrive as the server described it.
    DamagedFile,
    /// The server's files came too slowly, or kept changing, for the
    /// player ever to be ready to join.
    NeverReady,
    /// This machine ran out of memory for a file the server sent.
    NoMemory,
}

impl fmt::Display for Closed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoAnswer => {
                "no server answered. Check the address and port, that the server is running, \
                 and that its firewall or router lets UDP through on that port."
            }
            Self::TimedOut => {
                "the server stopped answering. It may have crashed or the connection dropped; \
                 try joining again."
            }
            Self::Full => "the server is full. Try again when a player has left.",
            Self::WrongVersion => {
                "the server runs another version of the engine. Update the game on both \
                 sides so the server and the players run the same version."
            }
            Self::ServerStopped => "the server was shut down.",
            Self::Kicked => "the server took you out of the game.",
            Self::Left => "you left the game.",
            Self::DamagedFile => {
                "a file the server sent arrived damaged. Try joining again; if it happens \
                 every time, restart the server."
            }
            Self::NeverReady => {
                "the server's level never finished arriving: it came too slowly or kept \
                 changing. Try joining again; if it happens every time, the connection to \
                 that server is too slow to play on."
            }
            Self::NoMemory => {
                "this computer ran out of memory for the server's level. Close other \
                 programs and try joining again."
            }
        })
    }
}

/// Why a server turns a player away. Passed to
/// [`UdpHost::refuse`](crate::UdpHost::refuse) and
/// [`UdpHost::kick`](crate::UdpHost::kick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Every place is taken. The player sees [`Closed::Full`].
    Full,
    /// The server is shutting down. The player sees
    /// [`Closed::ServerStopped`].
    Stopping,
    /// The server does not want this player. The player sees
    /// [`Closed::Kicked`].
    Kicked,
}

impl Refusal {
    fn byte(self) -> u8 {
        match self {
            Self::Full => FULL,
            Self::Stopping => STOPPED,
            Self::Kicked => KICKED,
        }
    }
}

fn closed_by(reason: u8) -> Closed {
    match reason {
        FULL => Closed::Full,
        WRONG_VERSION => Closed::WrongVersion,
        KICKED => Closed::Kicked,
        _ => Closed::ServerStopped,
    }
}

/// Why a player is no longer on a server, as the server sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    /// The player said goodbye.
    Quit,
    /// Nothing was heard from the player for
    /// [`HostSettings::timeout`].
    TimedOut,
}

/// A player who has proved their address and is waiting for a place.
///
/// Answer straight away, before reading the next datagram, with
/// [`UdpHost::accept`](crate::UdpHost::accept) or
/// [`UdpHost::refuse`](crate::UdpHost::refuse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinRequest {
    from: SocketAddr,
    salt: u64,
    session: u64,
}

impl JoinRequest {
    /// The address the player is joining from.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.from
    }
}

/// What a datagram meant to the server's game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Heard {
    /// Somebody new proved their address.
    Join(JoinRequest),
    /// A packet from this player sits after [`DATA_HEADER`] bytes.
    Packet(PlayerId),
    /// This player said goodbye.
    Left(PlayerId),
    /// Somebody who proved their address wants pieces of an offered file.
    Fetch(Fetch),
}

/// A request for pieces that the server may answer: see
/// [`Host::write_piece`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fetch {
    /// The number the player made up, which every piece repeats.
    salt: u64,
    slot: usize,
    /// The first piece wanted.
    first: u32,
}

#[derive(Debug, Clone, Copy)]
struct Peer {
    address: SocketAddr,
    session: u64,
    heard: Duration,
}

/// The server's side: which address is which player.
#[derive(Debug)]
pub(crate) struct Host {
    settings: HostSettings,
    /// The secret the numbers handed to joining players are worked out
    /// with. New every time a server starts.
    secret: RandomState,
    peers: [Option<Peer>; MAX_PLAYERS],
    /// The numbers of players who left, newest overwriting oldest.
    ended: [u64; REMEMBERED],
    next_ended: usize,
    /// What is said to whoever is looking for servers.
    notice: Notice,
    /// The files on offer to players about to join.
    shelf: Shelf,
}

impl Host {
    pub(crate) fn new(settings: HostSettings) -> Self {
        Self {
            settings,
            secret: RandomState::new(),
            peers: [None; MAX_PLAYERS],
            ended: [0; REMEMBERED],
            next_ended: 0,
            notice: Notice::new(),
            shelf: Shelf::default(),
        }
    }

    /// Puts `bytes` on offer in `slot`, in place of what was there.
    pub(crate) fn share(&mut self, slot: usize, bytes: Arc<[u8]>) -> Result<(), ShareError> {
        self.shelf.share(slot, bytes)
    }

    /// Takes what is on offer in `slot` away.
    pub(crate) fn unshare(&mut self, slot: usize) {
        self.shelf.unshare(slot);
    }

    /// Whether `cookie` is the number `from` was handed, not too long ago,
    /// for `salt`: only somebody who receives what is sent to `from` knows
    /// it.
    fn proved(&self, now: Duration, from: SocketAddr, salt: u64, cookie: u64) -> bool {
        let period = now.as_secs() / COOKIE_SECONDS;
        cookie == self.cookie(from, salt, period)
            || period
                .checked_sub(1)
                .is_some_and(|before| cookie == self.cookie(from, salt, before))
    }

    /// Writes one of the pieces `fetch` asked for: the one `ahead` after
    /// the first. Gives `false`, and an empty `out`, when the file has no
    /// such piece or `ahead` is past what one request is answered with.
    pub(crate) fn write_piece(&self, fetch: Fetch, ahead: u32, out: &mut Vec<u8>) -> bool {
        out.clear();
        if ahead >= WINDOW {
            return false;
        }
        let Some(index) = fetch.first.checked_add(ahead) else {
            return false;
        };
        let Some((tag, piece)) = self.shelf.piece(fetch.slot, index) else {
            return false;
        };
        let mut writer = Writer::open(out, PIECE);
        writer.u64(fetch.salt);
        writer.u64(tag);
        writer.u32(index);
        writer.bytes(piece);
        true
    }

    /// Sets the name given to whoever is looking for servers.
    pub(crate) fn set_name(&mut self, name: &str) {
        self.notice.set_name(name);
    }

    fn cookie(&self, from: SocketAddr, salt: u64, period: u64) -> u64 {
        self.secret.hash_one((from, salt, period))
    }

    fn find(&self, from: SocketAddr, session: u64) -> Option<PlayerId> {
        (0..).zip(&self.peers).find_map(|(id, peer)| {
            peer.filter(|peer| peer.address == from && peer.session == session)
                .map(|_| id)
        })
    }

    fn forget(&mut self, id: PlayerId) -> Option<Peer> {
        let peer = self.peers.get_mut(usize::from(id))?.take()?;
        if let Some(place) = self.ended.get_mut(self.next_ended) {
            *place = peer.session;
        }
        self.next_ended = (self.next_ended + 1) % REMEMBERED;
        Some(peer)
    }

    /// Reads one datagram that came from `from`. Afterwards, if `reply`
    /// is not empty, send it back to `from`.
    pub(crate) fn receive(
        &mut self,
        now: Duration,
        from: SocketAddr,
        datagram: &[u8],
        reply: &mut Vec<u8>,
    ) -> Option<Heard> {
        reply.clear();
        if datagram.len() > MAX_DATAGRAM_BYTES {
            return None;
        }
        let mut reader = Reader::raw(datagram);
        let kind = reader.u8().ok()?;
        let version = reader.u8().ok()?;
        if kind == QUERY {
            self.notice.answer(datagram, self.players().count(), reply);
            return None;
        }
        if kind == REQUEST {
            // Shorter than the answer could be used to flood someone else.
            if datagram.len() < REQUEST_BYTES
                || [reader.u8(), reader.u8(), reader.u8(), reader.u8()] != MAGIC.map(Ok)
            {
                return None;
            }
            let salt = reader.u64().ok()?;
            if version == VERSION {
                let mut writer = Writer::open(reply, CHALLENGE);
                writer.u64(salt);
                writer.u64(self.cookie(from, salt, now.as_secs() / COOKIE_SECONDS));
                // Still far shorter than the request it answers.
                self.shelf.offer().write(&mut writer);
            } else {
                write_refused(reply, salt, WRONG_VERSION);
            }
            return None;
        }
        if version != VERSION {
            return None;
        }
        match kind {
            RESPONSE => {
                let salt = reader.u64().ok()?;
                let session = reader.u64().ok()?;
                if !self.proved(now, from, salt, session) || self.ended.contains(&session) {
                    return None;
                }
                if let Some(id) = self.find(from, session) {
                    // The answer was lost on the way; say it again.
                    write_accepted(reply, session, id);
                    return None;
                }
                // One address holds one place; a second try from it waits
                // until the first has left or gone silent.
                if self.peers.iter().flatten().any(|peer| peer.address == from) {
                    return None;
                }
                Some(Heard::Join(JoinRequest {
                    from,
                    salt,
                    session,
                }))
            }
            DATA => {
                let id = self.find(from, reader.u64().ok()?)?;
                if let Some(Some(peer)) = self.peers.get_mut(usize::from(id)) {
                    peer.heard = now;
                }
                // A datagram with nothing in it only says "still here".
                (!reader.rest().is_empty()).then_some(Heard::Packet(id))
            }
            GOODBYE => {
                let id = self.find(from, reader.u64().ok()?)?;
                self.forget(id);
                Some(Heard::Left(id))
            }
            FETCH => {
                let salt = reader.u64().ok()?;
                let cookie = reader.u64().ok()?;
                let tag = reader.u64().ok()?;
                let first = reader.u32().ok()?;
                if !self.proved(now, from, salt, cookie) {
                    return None;
                }
                let slot = self.shelf.find(tag, first)?;
                Some(Heard::Fetch(Fetch { salt, slot, first }))
            }
            _ => None,
        }
    }

    /// Gives the player behind `request` the place `id` and writes the
    /// datagram that tells them so. Whoever had that place is forgotten.
    pub(crate) fn accept(
        &mut self,
        now: Duration,
        request: JoinRequest,
        id: PlayerId,
        reply: &mut Vec<u8>,
    ) {
        reply.clear();
        self.forget(id);
        if let Some(place) = self.peers.get_mut(usize::from(id)) {
            *place = Some(Peer {
                address: request.from,
                session: request.session,
                heard: now,
            });
            write_accepted(reply, request.session, id);
        }
    }

    /// Writes the datagram that turns the player behind `request` away.
    pub(crate) fn refuse(request: JoinRequest, why: Refusal, reply: &mut Vec<u8>) {
        write_refused(reply, request.salt, why.byte());
    }

    /// Takes one player who has been silent too long off the list, if
    /// there is one. Call until it gives `None`.
    pub(crate) fn timed_out(&mut self, now: Duration) -> Option<PlayerId> {
        let timeout = self.settings.timeout;
        let id = (0..).zip(&self.peers).find_map(|(id, peer)| {
            peer.filter(|peer| now.saturating_sub(peer.heard) > timeout)
                .map(|_| id)
        })?;
        self.forget(id);
        Some(id)
    }

    /// Wraps `packet` for the player `id` and says where to send it.
    pub(crate) fn wrap(
        &self,
        id: PlayerId,
        packet: &[u8],
        out: &mut Vec<u8>,
    ) -> Option<SocketAddr> {
        let peer = self.peers.get(usize::from(id)).copied().flatten()?;
        let mut writer = Writer::open(out, DATA);
        writer.u64(peer.session);
        writer.bytes(packet);
        Some(peer.address)
    }

    /// Forgets the player `id`, writes the datagram that tells them why
    /// and says where to send it.
    pub(crate) fn drop_player(
        &mut self,
        id: PlayerId,
        why: Refusal,
        out: &mut Vec<u8>,
    ) -> Option<SocketAddr> {
        let peer = self.forget(id)?;
        write_goodbye(out, peer.session, why.byte());
        Some(peer.address)
    }

    /// The address of the player `id`.
    pub(crate) fn address(&self, id: PlayerId) -> Option<SocketAddr> {
        Some(self.peers.get(usize::from(id)).copied().flatten()?.address)
    }

    /// The numbers of the players joined, lowest first.
    pub(crate) fn players(&self) -> impl Iterator<Item = PlayerId> + '_ {
        (0..)
            .zip(&self.peers)
            .filter_map(|(id, peer)| peer.map(|_| id))
    }
}

fn write_accepted(out: &mut Vec<u8>, session: u64, id: PlayerId) {
    let mut writer = Writer::open(out, ACCEPTED);
    writer.u64(session);
    writer.u8(id);
}

fn write_refused(out: &mut Vec<u8>, salt: u64, reason: u8) {
    let mut writer = Writer::open(out, REFUSED);
    writer.u64(salt);
    writer.u8(reason);
}

fn write_goodbye(out: &mut Vec<u8>, session: u64, reason: u8) {
    let mut writer = Writer::open(out, GOODBYE);
    writer.u64(session);
    writer.u8(reason);
}

/// What a datagram meant to the player's game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkHeard {
    /// The server gave this player a place.
    Joined(PlayerId),
    /// The server said which files it offers: see [`Link::offer`]. The
    /// player joins next unless a file is fetched first.
    Offered,
    /// The file in this slot arrived whole: see [`Link::fetched`]. The
    /// player joins next unless another file is fetched first.
    Fetched(usize),
    /// A packet from the server sits after [`DATA_HEADER`] bytes.
    Packet,
    /// The server turned this player away or said goodbye.
    Closed(Closed),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Asking,
    Answering {
        cookie: u64,
    },
    /// Fetching the file in `Link::transfer` before answering.
    Fetching {
        cookie: u64,
    },
    Joined {
        session: u64,
        id: PlayerId,
    },
    Closed(Closed),
}

/// The player's side of one stay on a server.
#[derive(Debug)]
pub(crate) struct Link {
    settings: LinkSettings,
    /// A number made up for this stay. The server repeats it, so answers
    /// meant for an earlier try, or made up by a stranger, are not
    /// believed.
    salt: u64,
    stage: Stage,
    started: Duration,
    /// When to ask or answer next.
    send_at: Duration,
    /// When the server was last heard from.
    heard: Duration,
    /// When the number in hand was given.
    cookie_at: Duration,
    /// When [`update`](Self::update) was last called.
    updated: Duration,
    /// How many times the server has changed its offer since asking began.
    changes: u32,
    /// What the server offers, as it last said.
    offer: Offer,
    /// The file being fetched, or the one that last arrived.
    transfer: Option<Transfer>,
}

impl Link {
    /// A link that starts asking to join at `now`.
    pub(crate) fn new(now: Duration, settings: LinkSettings) -> Self {
        Self {
            settings,
            salt: RandomState::new().hash_one(0_u8),
            stage: Stage::Asking,
            started: now,
            send_at: now,
            heard: now,
            cookie_at: now,
            updated: now,
            changes: 0,
            offer: Offer::default(),
            transfer: None,
        }
    }

    /// The files the server offers, as it last said.
    pub(crate) fn offer(&self) -> &Offer {
        &self.offer
    }

    /// Fetches the file the server offers in `slot` before joining. Call
    /// it straight after [`LinkHeard::Offered`] or [`LinkHeard::Fetched`].
    /// Gives `false`, and fetches nothing, when it is too late for that,
    /// the slot is empty or the file is longer than
    /// [`MAX_SHARED_BYTES`](crate::MAX_SHARED_BYTES).
    pub(crate) fn fetch(&mut self, now: Duration, slot: usize) -> bool {
        let Stage::Answering { cookie } = self.stage else {
            return false;
        };
        let Some(transfer) = self
            .offer
            .file(slot)
            .and_then(|file| Transfer::start(now, slot, file))
        else {
            return false;
        };
        self.transfer = Some(transfer);
        self.stage = Stage::Fetching { cookie };
        true
    }

    /// The file that last arrived whole, until the next is fetched.
    pub(crate) fn fetched(&self) -> Option<&[u8]> {
        match self.stage {
            Stage::Fetching { .. } => None,
            _ => self.transfer.as_ref().map(Transfer::bytes),
        }
    }

    /// Lets go of the file that last arrived, to free its memory.
    pub(crate) fn forget_fetched(&mut self) {
        if !matches!(self.stage, Stage::Fetching { .. }) {
            self.transfer = None;
        }
    }

    /// How much of the file being fetched has arrived, and how long it is,
    /// in bytes.
    pub(crate) fn progress(&self) -> Option<(u64, u64)> {
        let Stage::Fetching { .. } = self.stage else {
            return None;
        };
        let transfer = self.transfer.as_ref()?;
        Some((transfer.fetched(), u64::from(transfer.file().len)))
    }

    /// Takes the server's answer to a request: a number and its offer.
    fn challenged(&mut self, now: Duration, reader: &mut Reader<'_>) -> Option<LinkHeard> {
        let cookie = reader.u64().ok()?;
        let offer = Offer::read(reader).ok()?;
        self.cookie_at = now;
        let changed = offer != self.offer;
        self.offer = offer;
        match self.stage {
            Stage::Asking => {
                self.stage = Stage::Answering { cookie };
                self.send_at = now;
                (!offer.is_empty()).then_some(LinkHeard::Offered)
            }
            Stage::Fetching { .. } => {
                let wanted = self
                    .transfer
                    .as_ref()
                    .map(|transfer| (transfer.slot, transfer.file()));
                if wanted.is_some_and(|(slot, file)| offer.file(slot) == Some(file)) {
                    self.stage = Stage::Fetching { cookie };
                    return None;
                }
                // The server has put another file there: start over with
                // what it offers now.
                self.transfer = None;
                self.stage = Stage::Answering { cookie };
                self.send_at = now;
                self.started = now;
                Some(self.offer_changed())
            }
            Stage::Answering { .. } => {
                self.stage = Stage::Answering { cookie };
                self.send_at = now;
                changed.then(|| self.offer_changed())
            }
            Stage::Joined { .. } | Stage::Closed(_) => None,
        }
    }

    /// The server changed its offer after first making it. A server that
    /// keeps doing so is given up on.
    fn offer_changed(&mut self) -> LinkHeard {
        self.changes += 1;
        if self.changes > MOST_CHANGES {
            self.transfer = None;
            self.stage = Stage::Closed(Closed::NeverReady);
            return LinkHeard::Closed(Closed::NeverReady);
        }
        LinkHeard::Offered
    }

    /// Takes one piece of the file being fetched.
    fn piece(&mut self, now: Duration, reader: &mut Reader<'_>) -> Option<LinkHeard> {
        let tag = reader.u64().ok()?;
        let index = reader.u32().ok()?;
        let Stage::Fetching { cookie } = self.stage else {
            return None;
        };
        let transfer = self.transfer.as_mut()?;
        match transfer.take(now, tag, index, reader.rest()) {
            Progress::Nothing | Progress::More => None,
            Progress::Whole => {
                let slot = transfer.slot;
                self.stage = Stage::Answering { cookie };
                self.send_at = now;
                // Giving up is counted from here, not from before the fetch.
                self.started = now;
                Some(LinkHeard::Fetched(slot))
            }
            ended @ (Progress::Damaged | Progress::NoRoom) => {
                let why = if ended == Progress::Damaged {
                    Closed::DamagedFile
                } else {
                    Closed::NoMemory
                };
                self.transfer = None;
                self.stage = Stage::Closed(why);
                Some(LinkHeard::Closed(why))
            }
        }
    }

    /// The place the server gave, once it has.
    pub(crate) fn id(&self) -> Option<PlayerId> {
        match self.stage {
            Stage::Joined { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Why the stay ended, once it has.
    pub(crate) fn closed(&self) -> Option<Closed> {
        match self.stage {
            Stage::Closed(why) => Some(why),
            _ => None,
        }
    }

    /// Reads one datagram from the server.
    pub(crate) fn receive(&mut self, now: Duration, datagram: &[u8]) -> Option<LinkHeard> {
        if datagram.len() > MAX_DATAGRAM_BYTES {
            return None;
        }
        let mut reader = Reader::raw(datagram);
        let kind = reader.u8().ok()?;
        let version = reader.u8().ok()?;
        let number = reader.u64().ok()?;
        match (kind, self.stage) {
            // A refusal is read whatever its version: a server of another
            // version has no other way to say so.
            (REFUSED, Stage::Asking | Stage::Answering { .. } | Stage::Fetching { .. })
                if number == self.salt =>
            {
                let why = closed_by(reader.u8().ok()?);
                self.stage = Stage::Closed(why);
                Some(LinkHeard::Closed(why))
            }
            _ if version != VERSION => None,
            (CHALLENGE, Stage::Asking | Stage::Answering { .. } | Stage::Fetching { .. })
                if number == self.salt =>
            {
                self.challenged(now, &mut reader)
            }
            (PIECE, Stage::Fetching { .. }) if number == self.salt => self.piece(now, &mut reader),
            (ACCEPTED, Stage::Answering { cookie }) if number == cookie => {
                let id = reader.u8().ok()?;
                if usize::from(id) >= MAX_PLAYERS {
                    return None;
                }
                self.stage = Stage::Joined {
                    session: cookie,
                    id,
                };
                self.heard = now;
                Some(LinkHeard::Joined(id))
            }
            (DATA, Stage::Joined { session, .. }) if number == session => {
                self.heard = now;
                (!reader.rest().is_empty()).then_some(LinkHeard::Packet)
            }
            (GOODBYE, Stage::Joined { session, .. }) if number == session => {
                let why = closed_by(reader.u8().ok()?);
                self.stage = Stage::Closed(why);
                Some(LinkHeard::Closed(why))
            }
            _ => None,
        }
    }

    /// Call often, at least once a tick. Afterwards, if `out` is not
    /// empty, send it to the server. Says so when waiting has gone on
    /// too long.
    pub(crate) fn update(&mut self, now: Duration, out: &mut Vec<u8>) -> Option<Closed> {
        out.clear();
        // Time the game spent elsewhere once the server had answered, such
        // as reading a level that has just arrived, is not time the server
        // kept this player waiting.
        let away = now.saturating_sub(self.updated);
        self.updated = now;
        if away > AWAY && matches!(self.stage, Stage::Answering { .. }) {
            self.started = self.started.saturating_add(away);
        }
        let why = match self.stage {
            Stage::Asking | Stage::Answering { .. } => {
                if now.saturating_sub(self.started) <= self.settings.give_up {
                    if now >= self.send_at {
                        self.send_at = now.saturating_add(self.settings.resend);
                        // A number that has been held a while, as after a
                        // long fetch, may no longer be taken: ask for a
                        // new one before answering with it.
                        if matches!(self.stage, Stage::Answering { .. })
                            && now.saturating_sub(self.cookie_at) > COOKIE_REFRESH
                        {
                            self.write_request(out);
                        } else {
                            self.write_handshake(out);
                        }
                    }
                    return None;
                }
                Closed::NoAnswer
            }
            Stage::Fetching { cookie } => {
                let Some(transfer) = self.transfer.as_mut() else {
                    self.stage = Stage::Answering { cookie };
                    return None;
                };
                if transfer.overdue(now) {
                    Closed::NeverReady
                } else if now.saturating_sub(transfer.heard) <= self.settings.timeout {
                    if now.saturating_sub(self.cookie_at) > COOKIE_REFRESH && now >= self.send_at {
                        // Asked for no more often than joining is.
                        self.send_at = now.saturating_add(self.settings.resend);
                        self.write_request(out);
                    } else if let Some((tag, first)) = transfer.ask(now) {
                        let mut writer = Writer::open(out, FETCH);
                        writer.u64(self.salt);
                        writer.u64(cookie);
                        writer.u64(tag);
                        writer.u32(first);
                    }
                    return None;
                } else {
                    Closed::TimedOut
                }
            }
            Stage::Joined { .. } => {
                if now.saturating_sub(self.heard) <= self.settings.timeout {
                    return None;
                }
                Closed::TimedOut
            }
            Stage::Closed(_) => return None,
        };
        self.stage = Stage::Closed(why);
        Some(why)
    }

    fn write_request(&self, out: &mut Vec<u8>) {
        let mut writer = Writer::open(out, REQUEST);
        writer.bytes(&MAGIC);
        writer.u64(self.salt);
        writer.pad_to(REQUEST_BYTES);
    }

    fn write_handshake(&self, out: &mut Vec<u8>) {
        match self.stage {
            Stage::Asking => self.write_request(out),
            Stage::Answering { cookie } => {
                let mut writer = Writer::open(out, RESPONSE);
                writer.u64(self.salt);
                writer.u64(cookie);
            }
            Stage::Fetching { .. } | Stage::Joined { .. } | Stage::Closed(_) => {}
        }
    }

    /// Wraps `packet` for the server. Gives `false`, and an empty `out`,
    /// when not joined.
    pub(crate) fn wrap(&self, packet: &[u8], out: &mut Vec<u8>) -> bool {
        out.clear();
        let Stage::Joined { session, .. } = self.stage else {
            return false;
        };
        let mut writer = Writer::open(out, DATA);
        writer.u64(session);
        writer.bytes(packet);
        true
    }

    /// Ends the stay. Gives `true` when `out` holds a goodbye to send.
    pub(crate) fn leave(&mut self, out: &mut Vec<u8>) -> bool {
        out.clear();
        let stage = self.stage;
        if !matches!(stage, Stage::Closed(_)) {
            self.stage = Stage::Closed(Closed::Left);
        }
        let Stage::Joined { session, .. } = stage else {
            return false;
        };
        write_goodbye(out, session, QUIT);
        true
    }
}
