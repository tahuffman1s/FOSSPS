//! Files a server hands to players before they join, such as the level.
//!
//! A server puts up to [`MAX_SHARED_FILES`] files on offer, each in a
//! numbered slot ([`crate::UdpHost::share`]). What a slot means is for the
//! game to say; nothing here knows a file's name, so a server cannot name
//! a place on a player's disk. The offer, each file's length and SHA-256
//! hash, rides in the server's answer to a request to join. A player who
//! lacks a file asks for it [`WINDOW`] pieces at a time
//! ([`crate::UdpLink::fetch`]) and only takes a place once it has arrived
//! whole and its hash is right.
//!
//! Both sides distrust the other:
//!
//! - A piece is only sent to an address that has proved itself, with the
//!   same number that joining takes, so nobody can have a file sent to a
//!   third party.
//! - A server sends at most a fixed number of pieces a tick however many
//!   are asked for, and keeps nothing about who is asking.
//! - A player never takes more than the length the offer gave, never more
//!   than [`MAX_SHARED_BYTES`], and throws away a file whose hash is wrong.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::wire::{NetError, Reader, Writer};

/// How many files a server can offer at once.
pub const MAX_SHARED_FILES: usize = 4;

/// The most bytes one offered file may hold: 256 MiB.
pub const MAX_SHARED_BYTES: usize = 256 * 1024 * 1024;

/// How many bytes of a file travel in one datagram. With its wrapping a
/// piece stays under [`crate::MAX_PACKET_BYTES`].
pub(crate) const PIECE_BYTES: usize = 1024;

/// How many pieces one request is answered with. Thirty-two pieces fit in
/// the smallest receive buffer an operating system gives a socket (64 KiB
/// on Windows), so a burst is not dropped on arrival.
pub(crate) const WINDOW: u32 = 32;

/// How long to wait for the pieces asked for before asking again.
const ASK_AGAIN: Duration = Duration::from_millis(250);

/// The SHA-256 hash of `bytes`: what a shared file is known by.
#[must_use]
pub fn hash_of(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// One file a server offers: how long it is and its hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedFile {
    /// How many bytes the file holds.
    pub len: u32,
    /// The file's SHA-256 hash ([`hash_of`]).
    pub hash: [u8; 32],
}

impl SharedFile {
    /// The first eight bytes of the hash, which requests and pieces carry
    /// to say which file they are about.
    pub(crate) fn tag(&self) -> u64 {
        let [a, b, c, d, e, f, g, h, ..] = self.hash;
        u64::from_le_bytes([a, b, c, d, e, f, g, h])
    }

    /// How many pieces the file travels in.
    pub(crate) fn pieces(&self) -> u32 {
        self.len.div_ceil(PIECE_BYTES as u32)
    }
}

/// The files a server offers, by slot. A player is shown it in
/// [`LinkEvent::Offered`](crate::LinkEvent::Offered).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Offer {
    files: [Option<SharedFile>; MAX_SHARED_FILES],
}

impl Offer {
    /// The file in `slot`, if the server offers one there.
    #[must_use]
    pub fn file(&self, slot: usize) -> Option<SharedFile> {
        self.files.get(slot).copied().flatten()
    }

    /// Whether nothing is offered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.iter().all(Option::is_none)
    }

    pub(crate) fn write(&self, writer: &mut Writer<'_>) {
        let mut mask = 0_u8;
        for (slot, file) in self.files.iter().enumerate() {
            if file.is_some() {
                mask |= 1 << slot;
            }
        }
        writer.u8(mask);
        for file in self.files.iter().flatten() {
            writer.u32(file.len);
            writer.bytes(&file.hash);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, NetError> {
        let mask = reader.u8()?;
        if mask >> MAX_SHARED_FILES != 0 {
            return Err(NetError::BadValue);
        }
        let mut offer = Self::default();
        for (slot, place) in offer.files.iter_mut().enumerate() {
            if mask & (1 << slot) == 0 {
                continue;
            }
            let len = reader.u32()?;
            let mut hash = [0; 32];
            for byte in &mut hash {
                *byte = reader.u8()?;
            }
            *place = Some(SharedFile { len, hash });
        }
        Ok(offer)
    }
}

/// Why a file could not be put on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareError {
    /// There is no slot with that number.
    NoSuchSlot,
    /// The file holds nothing.
    Empty,
    /// The file holds more than [`MAX_SHARED_BYTES`].
    TooBig {
        /// How many bytes it holds.
        len: usize,
    },
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchSlot => write!(
                f,
                "there is no slot with that number. A server offers {MAX_SHARED_FILES} files \
                 at most, in slots 0 to {}.",
                MAX_SHARED_FILES - 1
            ),
            Self::Empty => f.write_str(
                "the file is empty, so there is nothing to send. Leave the slot empty \
                 instead (`UdpHost::unshare`).",
            ),
            Self::TooBig { len } => write!(
                f,
                "the file is {} MB, and the most a server sends a joining player is {} MB. \
                 Make the file smaller.",
                len.div_ceil(1024 * 1024),
                MAX_SHARED_BYTES / (1024 * 1024)
            ),
        }
    }
}

impl std::error::Error for ShareError {}

/// The files a server has on offer.
#[derive(Debug, Default)]
pub(crate) struct Shelf {
    files: [Option<Arc<[u8]>>; MAX_SHARED_FILES],
    offer: Offer,
}

impl Shelf {
    pub(crate) fn share(&mut self, slot: usize, bytes: Arc<[u8]>) -> Result<(), ShareError> {
        if bytes.is_empty() {
            return Err(ShareError::Empty);
        }
        let len = u32::try_from(bytes.len())
            .ok()
            .filter(|_| bytes.len() <= MAX_SHARED_BYTES)
            .ok_or(ShareError::TooBig { len: bytes.len() })?;
        let (Some(file), Some(offered)) =
            (self.files.get_mut(slot), self.offer.files.get_mut(slot))
        else {
            return Err(ShareError::NoSuchSlot);
        };
        *offered = Some(SharedFile {
            len,
            hash: hash_of(&bytes),
        });
        *file = Some(bytes);
        Ok(())
    }

    pub(crate) fn unshare(&mut self, slot: usize) {
        if let (Some(file), Some(offered)) =
            (self.files.get_mut(slot), self.offer.files.get_mut(slot))
        {
            *file = None;
            *offered = None;
        }
    }

    pub(crate) fn offer(&self) -> &Offer {
        &self.offer
    }

    /// The slot of the file with `tag` that has a piece numbered `first`.
    pub(crate) fn find(&self, tag: u64, first: u32) -> Option<usize> {
        self.offer
            .files
            .iter()
            .position(|file| file.is_some_and(|file| file.tag() == tag && first < file.pieces()))
    }

    /// The tag of the file in `slot` and its piece numbered `index`.
    pub(crate) fn piece(&self, slot: usize, index: u32) -> Option<(u64, &[u8])> {
        let tag = self.offer.file(slot)?.tag();
        let bytes = self.files.get(slot)?.as_deref()?;
        let piece = bytes.chunks(PIECE_BYTES).nth(index as usize)?;
        Some((tag, piece))
    }
}

/// What a piece that arrived did for a file on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Progress {
    /// Not a piece of this file, or one already held.
    Nothing,
    /// One more piece is held.
    More,
    /// Every piece is held and the hash is right.
    Whole,
    /// Every piece is held and the hash is wrong.
    Damaged,
}

/// A file on its way to a player.
#[derive(Debug)]
pub(crate) struct Transfer {
    pub(crate) slot: usize,
    file: SharedFile,
    bytes: Vec<u8>,
    /// One bit a piece: whether it is held.
    held: Vec<u64>,
    /// How many pieces are held.
    count: u32,
    /// The lowest piece not held.
    next: u32,
    /// The piece last asked from.
    asked: u32,
    /// When to ask next.
    ask_at: Duration,
    /// When a piece last arrived, or the fetch began.
    pub(crate) heard: Duration,
}

impl Transfer {
    /// Starts fetching `file`, or `None` when it is empty or too long to
    /// take.
    pub(crate) fn start(now: Duration, slot: usize, file: SharedFile) -> Option<Self> {
        let len = file.len as usize;
        if len == 0 || len > MAX_SHARED_BYTES {
            return None;
        }
        Some(Self {
            slot,
            file,
            // The system hands zeroed memory over as it is touched, so a
            // long file that never arrives costs little.
            bytes: vec![0; len],
            held: vec![0; (file.pieces() as usize).div_ceil(64)],
            count: 0,
            next: 0,
            asked: 0,
            ask_at: now,
            heard: now,
        })
    }

    pub(crate) fn file(&self) -> SharedFile {
        self.file
    }

    /// The file, which is whole once [`take`](Self::take) said
    /// [`Progress::Whole`].
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// How many bytes have arrived.
    pub(crate) fn fetched(&self) -> u64 {
        (u64::from(self.count) * PIECE_BYTES as u64).min(u64::from(self.file.len))
    }

    fn holds(&self, index: u32) -> bool {
        self.held
            .get((index / 64) as usize)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }

    /// Takes in the piece numbered `index` of the file with `tag`.
    pub(crate) fn take(&mut self, now: Duration, tag: u64, index: u32, piece: &[u8]) -> Progress {
        let pieces = self.file.pieces();
        if tag != self.file.tag() || index >= pieces || self.holds(index) {
            return Progress::Nothing;
        }
        let start = index as usize * PIECE_BYTES;
        // Every piece is full but the last, which holds what is left.
        let Some(place) = self
            .bytes
            .get_mut(start..(start + PIECE_BYTES).min(self.file.len as usize))
            .filter(|place| place.len() == piece.len())
        else {
            return Progress::Nothing;
        };
        place.copy_from_slice(piece);
        if let Some(word) = self.held.get_mut((index / 64) as usize) {
            *word |= 1 << (index % 64);
        }
        self.count += 1;
        self.heard = now;
        while self.next < pieces && self.holds(self.next) {
            self.next += 1;
        }
        if self.count == pieces {
            return if hash_of(&self.bytes) == self.file.hash {
                Progress::Whole
            } else {
                Progress::Damaged
            };
        }
        // Everything asked for is here: ask for more without waiting.
        if self.next >= self.asked.saturating_add(WINDOW) {
            self.ask_at = now;
        }
        Progress::More
    }

    /// The piece to ask from, when it is time to ask.
    pub(crate) fn ask(&mut self, now: Duration) -> Option<(u64, u32)> {
        if now < self.ask_at {
            return None;
        }
        self.ask_at = now.saturating_add(ASK_AGAIN);
        self.asked = self.next;
        Some((self.file.tag(), self.next))
    }
}
