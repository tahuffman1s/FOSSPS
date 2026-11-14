//! What the server tells a client about one tick, and how it is packed.
//!
//! A snapshot holds the client's own player exactly, so the client can
//! check its prediction to the last digit, and every other player rounded
//! to a few bytes. Other players are sent as what changed since a snapshot
//! the client has said it read, or in full when there is no such snapshot.

use fossps_physics::Player;

use crate::command::{pitch_radians, yaw_radians};
use crate::wire::{NetError, Reader, SNAPSHOT, Writer};

/// The most players one server holds.
///
/// A snapshot of this many players, all of them changed, still fits in
/// one packet.
pub const MAX_PLAYERS: usize = 64;

/// A player's number on the server, from 0 to 63. Given by
/// [`Server::join`](crate::Server::join) and free for the next player
/// after [`Server::leave`](crate::Server::leave).
pub type PlayerId = u8;

/// How many ticks of history each side keeps to work out changes from.
pub(crate) const HISTORY: usize = 32;

/// Steps to a metre in a sent position: about 2 mm each.
const POSITION_STEPS: f32 = 512.0;
/// The furthest a sent position reaches on each axis, in steps (16 km).
const FURTHEST: i32 = (1 << 23) - 1;

const GROUNDED: u8 = 1 << 0;
const CROUCHED: u8 = 1 << 1;
const SPRINTING: u8 = 1 << 2;
const JUMP_HELD: u8 = 1 << 3;

const POSITION: u8 = 1 << 0;
const LOOK: u8 = 1 << 1;
const FLAGS: u8 = 1 << 2;
const EVERYTHING: u8 = POSITION | LOOK | FLAGS;

/// Another player as a client sees them: where they are and which way
/// they look, to the nearest 2 mm and 0.006 degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemotePlayer {
    /// The player's number on the server.
    pub id: PlayerId,
    /// Where the feet are, in metres.
    pub position: [f32; 3],
    /// Which way they face, in radians, turning left from -Z as it grows.
    pub yaw: f32,
    /// Looking up (positive) or down, in radians.
    pub pitch: f32,
    /// Standing on something.
    pub grounded: bool,
    /// Crouched.
    pub crouched: bool,
    /// Sprinting.
    pub sprinting: bool,
}

/// A player as sent: whole numbers, so "has it changed" is an exact
/// question.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Seen {
    position: [i32; 3],
    yaw: u16,
    pitch: i16,
    flags: u8,
}

impl Seen {
    pub(crate) fn of(player: &Player, (yaw, pitch): (u16, i16)) -> Self {
        let mut flags = 0;
        for (set, bit) in [
            (player.grounded, GROUNDED),
            (player.crouched, CROUCHED),
            (player.sprinting, SPRINTING),
        ] {
            if set {
                flags |= bit;
            }
        }
        Self {
            position: player.position.map(|metres| {
                // A position that is not a number is sent as 0; the cast
                // cannot cut anything off after the clamp.
                let steps = libm::roundf(metres * POSITION_STEPS);
                if steps.is_finite() {
                    (steps as i32).clamp(-FURTHEST, FURTHEST)
                } else {
                    0
                }
            }),
            yaw,
            pitch,
            flags,
        }
    }

    pub(crate) fn remote(&self, id: PlayerId) -> RemotePlayer {
        RemotePlayer {
            id,
            // Under 2^23, so the cast is exact.
            position: self.position.map(|steps| steps as f32 / POSITION_STEPS),
            yaw: yaw_radians(self.yaw),
            pitch: pitch_radians(self.pitch),
            grounded: self.flags & GROUNDED != 0,
            crouched: self.flags & CROUCHED != 0,
            sprinting: self.flags & SPRINTING != 0,
        }
    }
}

/// Every player on the server at one tick, as sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct World {
    /// The tick this is, or 0 for "nothing yet": ticks count from 1.
    pub(crate) tick: u32,
    pub(crate) players: [Option<Seen>; MAX_PLAYERS],
}

impl World {
    pub(crate) const EMPTY: Self = Self {
        tick: 0,
        players: [None; MAX_PLAYERS],
    };

    /// The players in it, lowest number first.
    pub(crate) fn remotes(&self) -> impl Iterator<Item = RemotePlayer> + '_ {
        (0..).zip(&self.players).filter_map(|(id, seen)| {
            let seen = seen.as_ref()?;
            Some(seen.remote(id))
        })
    }
}

/// Where the world of `tick` is kept in a history of [`HISTORY`] worlds.
pub(crate) fn place(tick: u32) -> usize {
    tick as usize % HISTORY
}

/// The world of `tick` in `history`, if it is still there.
pub(crate) fn find(history: &[World], tick: u32) -> Option<&World> {
    history
        .get(place(tick))
        .filter(|world| tick != 0 && world.tick == tick)
}

/// The start of a snapshot: everything but the other players.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Header {
    /// The server's tick.
    pub(crate) tick: u32,
    /// The tick the other players are given as changes from, or 0 when
    /// they are given in full.
    pub(crate) baseline: u32,
    /// The number of the client's last command the server has run, or 0.
    pub(crate) last_command: u32,
    /// The client's own number.
    pub(crate) id: PlayerId,
    /// The client's own player after that command.
    pub(crate) player: Player,
}

/// Writes the snapshot of `world` for the player `header.id` into `out`.
/// Other players are written as changes from `baseline` when there is one.
pub(crate) fn write(out: &mut Vec<u8>, header: &Header, world: &World, baseline: Option<&World>) {
    let mut writer = Writer::open(out, SNAPSHOT);
    writer.u32(header.tick);
    writer.u32(baseline.map_or(0, |world| world.tick));
    writer.u32(header.last_command);
    writer.u8(header.id);

    let player = &header.player;
    writer.vector(player.position);
    writer.vector(player.velocity);
    writer.f32(player.eye_height);
    writer.vector(player.ground_normal);
    let mut flags = 0;
    for (set, bit) in [
        (player.grounded, GROUNDED),
        (player.crouched, CROUCHED),
        (player.sprinting, SPRINTING),
        (player.jump_held, JUMP_HELD),
    ] {
        if set {
            flags |= bit;
        }
    }
    writer.u8(flags);

    let count_at = writer.len();
    writer.u8(0);
    let mut count = 0_u8;
    for (id, seen) in (0..).zip(&world.players) {
        let Some(seen) = seen else { continue };
        if id == header.id {
            continue;
        }
        let before = baseline.and_then(|world| world.players.get(usize::from(id))?.as_ref());
        let mask = before.map_or(EVERYTHING, |before| {
            let mut mask = 0;
            if seen.position != before.position {
                mask |= POSITION;
            }
            if (seen.yaw, seen.pitch) != (before.yaw, before.pitch) {
                mask |= LOOK;
            }
            if seen.flags != before.flags {
                mask |= FLAGS;
            }
            mask
        });
        writer.u8(id);
        writer.u8(mask);
        if mask & POSITION != 0 {
            for steps in seen.position {
                writer.i24(steps);
            }
        }
        if mask & LOOK != 0 {
            writer.u16(seen.yaw);
            writer.i16(seen.pitch);
        }
        if mask & FLAGS != 0 {
            writer.u8(seen.flags);
        }
        count += 1;
    }
    writer.set(count_at, count);
}

/// Reads the start of a snapshot, leaving `reader` at the other players.
pub(crate) fn read_header<'a>(bytes: &'a [u8]) -> Result<(Header, Reader<'a>), NetError> {
    let mut reader = Reader::open(bytes, SNAPSHOT)?;
    let tick = reader.u32()?;
    let baseline = reader.u32()?;
    let last_command = reader.u32()?;
    let id = reader.u8()?;
    // Tick 0 is a server that has not ticked yet; the client skips it.
    if (baseline != 0 && baseline >= tick) || usize::from(id) >= MAX_PLAYERS {
        return Err(NetError::BadValue);
    }
    let position = reader.vector()?;
    let velocity = reader.vector()?;
    let eye_height = reader.f32()?;
    let ground_normal = reader.vector()?;
    let flags = reader.u8()?;
    let header = Header {
        tick,
        baseline,
        last_command,
        id,
        player: Player {
            position,
            velocity,
            grounded: flags & GROUNDED != 0,
            crouched: flags & CROUCHED != 0,
            sprinting: flags & SPRINTING != 0,
            eye_height,
            ground_normal,
            jump_held: flags & JUMP_HELD != 0,
        },
    };
    Ok((header, reader))
}

/// Reads the other players of the snapshot `header` starts into `world`.
/// `baseline` is the world the changes are from, when the header names one.
///
/// On an error `world` is left half filled and must not be used.
pub(crate) fn read_players(
    reader: &mut Reader<'_>,
    header: &Header,
    baseline: Option<&World>,
    world: &mut World,
) -> Result<(), NetError> {
    *world = World::EMPTY;
    world.tick = header.tick;
    let count = reader.u8()?;
    let mut lowest = 0_usize;
    for _ in 0..count {
        let id = usize::from(reader.u8()?);
        let mask = reader.u8()?;
        // Players come lowest number first, so none comes twice.
        if id < lowest || id == usize::from(header.id) || mask & !EVERYTHING != 0 {
            return Err(NetError::BadValue);
        }
        lowest = id + 1;
        let before = baseline.and_then(|world| world.players.get(id)?.as_ref());
        let mut seen = match before {
            Some(before) => *before,
            // A player the client has not seen must come whole.
            None if mask == EVERYTHING => Seen::default(),
            None => return Err(NetError::BadValue),
        };
        if mask & POSITION != 0 {
            seen.position = [reader.i24()?, reader.i24()?, reader.i24()?];
        }
        if mask & LOOK != 0 {
            seen.yaw = reader.u16()?;
            seen.pitch = reader.i16()?;
        }
        if mask & FLAGS != 0 {
            seen.flags = reader.u8()?;
        }
        *world.players.get_mut(id).ok_or(NetError::BadValue)? = Some(seen);
    }
    Ok(())
}
