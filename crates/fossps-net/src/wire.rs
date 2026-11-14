//! Reading and writing the bytes of a packet.

use std::fmt;

/// The most bytes one packet may hold.
///
/// A packet this size crosses the internet in one piece on almost every
/// connection, VPNs and mobile networks included.
pub const MAX_PACKET_BYTES: usize = 1200;

/// The first byte of a packet of commands, client to server.
pub(crate) const COMMANDS: u8 = 1;
/// The first byte of a snapshot, server to client.
pub(crate) const SNAPSHOT: u8 = 2;

/// The version of the packet layout, the second byte of every packet.
/// It goes up whenever the layout changes.
pub(crate) const VERSION: u8 = 1;

/// Why a packet could not be read, or a player could not be found or added.
///
/// A packet that fails to read is thrown away; the next one makes up for
/// it. None of these stop a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    /// The packet ended before everything in it could be read.
    TooShort,
    /// The packet is longer than [`MAX_PACKET_BYTES`].
    TooLong,
    /// The packet is of another kind: a snapshot handed to a server, or
    /// commands handed to a client.
    WrongKind,
    /// The packet was written by another version of the engine.
    WrongVersion,
    /// There is no player with that number on the server.
    NoSuchPlayer,
    /// Every place on the server is taken.
    ServerFull,
    /// The packet holds a value that cannot be right, such as a position
    /// that is not a number or the same player twice.
    BadValue,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooShort => {
                "the packet ended early. It was cut short on the way or was not sent by this \
                 engine; it is safe to ignore."
            }
            Self::TooLong => {
                "the packet is over 1200 bytes, the most this engine sends. Pass the bytes of \
                 one packet at a time."
            }
            Self::WrongKind => {
                "the packet is of the wrong kind. Give snapshots to `Client::receive` and \
                 commands to `Server::receive`."
            }
            Self::WrongVersion => {
                "the packet comes from another version of the engine. Update the game on \
                 both sides so the server and the players run the same version."
            }
            Self::NoSuchPlayer => {
                "there is no player with that number. Use the number `Server::join` gave, \
                 and stop using it after `Server::leave`."
            }
            Self::ServerFull => {
                "the server is full: all 64 places are taken. Call `Server::leave` for a \
                 player who has gone before adding another."
            }
            Self::BadValue => {
                "the packet holds a value that cannot be right. It was damaged on the way or \
                 was not sent by this engine; it is safe to ignore."
            }
        })
    }
}

impl std::error::Error for NetError {}

/// Reads a packet from the front to the back.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader for a packet whose first byte must be `kind`.
    pub(crate) fn open(bytes: &'a [u8], kind: u8) -> Result<Self, NetError> {
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(NetError::TooLong);
        }
        let mut reader = Self { rest: bytes };
        if reader.u8()? != kind {
            return Err(NetError::WrongKind);
        }
        if reader.u8()? != VERSION {
            return Err(NetError::WrongVersion);
        }
        Ok(reader)
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], NetError> {
        let (head, rest) = self
            .rest
            .split_first_chunk::<N>()
            .ok_or(NetError::TooShort)?;
        self.rest = rest;
        Ok(*head)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, NetError> {
        Ok(self.take::<1>()?[0])
    }

    pub(crate) fn i8(&mut self) -> Result<i8, NetError> {
        Ok(i8::from_le_bytes(self.take()?))
    }

    pub(crate) fn u16(&mut self) -> Result<u16, NetError> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    pub(crate) fn i16(&mut self) -> Result<i16, NetError> {
        Ok(i16::from_le_bytes(self.take()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, NetError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    /// A whole number of 24 bits, from about -8 million to 8 million.
    pub(crate) fn i24(&mut self) -> Result<i32, NetError> {
        let [a, b, c] = self.take()?;
        // The top byte carries the sign.
        Ok(i32::from_le_bytes([a, b, c, 0]) << 8 >> 8)
    }

    /// A number that must be finite.
    pub(crate) fn f32(&mut self) -> Result<f32, NetError> {
        let value = f32::from_le_bytes(self.take()?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(NetError::BadValue)
        }
    }

    pub(crate) fn vector(&mut self) -> Result<[f32; 3], NetError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

/// Writes a packet into a buffer the caller keeps.
#[derive(Debug)]
pub(crate) struct Writer<'a> {
    out: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    /// Empties `out` and starts a packet of `kind` in it.
    pub(crate) fn open(out: &'a mut Vec<u8>, kind: u8) -> Self {
        out.clear();
        out.reserve(MAX_PACKET_BYTES);
        out.push(kind);
        out.push(VERSION);
        Self { out }
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.out.push(value);
    }

    pub(crate) fn i8(&mut self, value: i8) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn i16(&mut self, value: i16) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    /// The low 24 bits of `value`.
    pub(crate) fn i24(&mut self, value: i32) {
        self.out.extend_from_slice(&value.to_le_bytes()[..3]);
    }

    pub(crate) fn f32(&mut self, value: f32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn vector(&mut self, value: [f32; 3]) {
        for part in value {
            self.f32(part);
        }
    }

    /// Bytes written so far, the kind included.
    pub(crate) fn len(&self) -> usize {
        self.out.len()
    }

    /// Changes one byte written earlier, for a count that is only known
    /// once the things counted are written.
    pub(crate) fn set(&mut self, at: usize, value: u8) {
        if let Some(byte) = self.out.get_mut(at) {
            *byte = value;
        }
    }
}
