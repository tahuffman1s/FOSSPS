//! The server's side: runs every player's commands and writes snapshots.

use fossps_physics::{Level, MoveSettings, Player};

use crate::command::{Command, PlayerInput, read_commands};
use crate::snapshot::{
    self, HISTORY, Header, MAX_PLAYERS, PlayerId, RemotePlayer, Seen, World, find, place,
};
use crate::wire::NetError;

/// The most commands kept waiting for one player. One that sends faster
/// than the server ticks loses its oldest commands and gains nothing.
const LONGEST_QUEUE: usize = 8;

/// How many ticks a late player's last command is repeated before the
/// server lets go of their keys for them.
const MOST_REPEATS: u8 = 5;

/// The numbers that shape how a server treats its players' commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSettings {
    /// How many commands to have waiting before running a player's
    /// commands, at the start and again after running out. More rides out
    /// a rougher connection; each one adds a tick of delay to that
    /// player's moves as others see them. From 1 to 8.
    pub buffer: usize,
}

impl Default for ServerSettings {
    /// Two commands waiting: 33 ms at 60 ticks per second.
    fn default() -> Self {
        Self { buffer: 2 }
    }
}

/// One player's place on the server.
#[derive(Debug, Clone, Copy)]
struct Slot {
    player: Player,
    /// Commands waiting to run, oldest first.
    queue: [Command; LONGEST_QUEUE],
    waiting: usize,
    /// The highest command number received.
    newest: u32,
    /// The number of the last command run, or 0.
    last_command: u32,
    /// The command that moved the player this tick.
    ran: Command,
    /// Ticks in a row with no command of the player's own to run.
    repeats: u8,
    /// Holding commands back until the buffer is full again.
    filling: bool,
    /// The tick of the newest snapshot the client has said it read, or 0.
    read: u32,
}

/// The server's half of the netcode: it owns every player, moves each one
/// by the commands their client sends, and writes each client a snapshot
/// of the result.
///
/// Once per tick: hand every packet that arrived to
/// [`receive`](Self::receive), call [`tick`](Self::tick), then send each
/// player what [`write_snapshot`](Self::write_snapshot) writes.
#[derive(Debug, Clone)]
pub struct Server {
    step: f32,
    buffer: usize,
    tick: u32,
    slots: Vec<Option<Slot>>,
    history: Vec<World>,
}

impl Server {
    /// A server with nobody on it whose ticks are `step` seconds long.
    /// Every client must be made with the same `step`.
    #[must_use]
    pub fn new(step: f32, settings: ServerSettings) -> Self {
        Self {
            step,
            buffer: settings.buffer.clamp(1, LONGEST_QUEUE),
            tick: 0,
            slots: vec![None; MAX_PLAYERS],
            history: vec![World::EMPTY; HISTORY],
        }
    }

    /// Adds a player and gives back their number.
    ///
    /// # Errors
    ///
    /// [`NetError::ServerFull`] when all 64 places are taken.
    pub fn join(&mut self, player: Player) -> Result<PlayerId, NetError> {
        let (id, slot) = (0..)
            .zip(&mut self.slots)
            .find(|(_, slot)| slot.is_none())
            .ok_or(NetError::ServerFull)?;
        *slot = Some(Slot {
            player,
            queue: [Command::default(); LONGEST_QUEUE],
            waiting: 0,
            newest: 0,
            last_command: 0,
            ran: Command::default(),
            repeats: 0,
            filling: true,
            read: 0,
        });
        Ok(id)
    }

    /// Takes a player off the server. Their number is free for the next
    /// player to join. Does nothing if there is no such player.
    pub fn leave(&mut self, id: PlayerId) {
        if let Some(slot) = self.slots.get_mut(usize::from(id)) {
            *slot = None;
        }
    }

    /// Takes in a packet of commands from the player `id`.
    ///
    /// Commands that came before, or came out of order, are dropped; each
    /// packet repeats the last few, so a lost packet is usually made up
    /// for by the next.
    ///
    /// # Errors
    ///
    /// [`NetError::NoSuchPlayer`] if nobody has that number, and the other
    /// [`NetError`]s if the packet cannot be read. A packet that fails
    /// changes nothing.
    pub fn receive(&mut self, id: PlayerId, packet: &[u8]) -> Result<(), NetError> {
        let slot = self.slot_mut(id)?;
        // Read into a copy, so a packet that fails halfway leaves no trace.
        let mut queue = slot.queue;
        let mut waiting = slot.waiting;
        let mut newest = slot.newest;
        let header = read_commands(packet, |command| {
            if command.sequence() <= newest {
                return;
            }
            newest = command.sequence();
            if waiting == LONGEST_QUEUE {
                queue.copy_within(1.., 0);
                waiting -= 1;
            }
            if let Some(place) = queue.get_mut(waiting) {
                *place = command;
                waiting += 1;
            }
        })?;
        slot.queue = queue;
        slot.waiting = waiting;
        slot.newest = newest;
        slot.read = slot.read.max(header.snapshot);
        Ok(())
    }

    /// Moves every player through `level` by one tick.
    ///
    /// Each player's oldest waiting command is run: one per tick, however
    /// many were sent. A player whose commands are late keeps doing what
    /// they last did (without firing) for a few ticks, then stands still.
    pub fn tick(&mut self, level: &Level, settings: &MoveSettings) {
        self.tick = self.tick.saturating_add(1);
        let world = self.history.get_mut(place(self.tick));
        let mut seen = [None; MAX_PLAYERS];
        for (slot, seen) in self.slots.iter_mut().zip(&mut seen) {
            let Some(slot) = slot else { continue };
            if slot.filling && slot.waiting >= self.buffer {
                slot.filling = false;
            }
            if slot.waiting == 0 {
                slot.filling = true;
            }
            slot.ran = if slot.filling {
                slot.repeats = slot.repeats.saturating_add(1);
                if slot.repeats <= MOST_REPEATS {
                    slot.ran.repeated()
                } else {
                    slot.ran.idle()
                }
            } else {
                let command = slot.queue[0];
                slot.queue.copy_within(1.., 0);
                slot.waiting -= 1;
                slot.repeats = 0;
                slot.last_command = command.sequence();
                command
            };
            slot.player
                .step(level, settings, &slot.ran.movement(), self.step);
            *seen = Some(Seen::of(&slot.player, slot.ran.look()));
        }
        if let Some(world) = world {
            *world = World {
                tick: self.tick,
                players: seen,
            };
        }
    }

    /// Writes the snapshot of the last tick for the player `id` into
    /// `packet`, replacing what was in it. Send it as it is; it is never
    /// over [`MAX_PACKET_BYTES`](crate::MAX_PACKET_BYTES).
    ///
    /// Call it after [`tick`](Self::tick), every tick or every second tick.
    ///
    /// # Errors
    ///
    /// [`NetError::NoSuchPlayer`] if nobody has that number.
    pub fn write_snapshot(&self, id: PlayerId, packet: &mut Vec<u8>) -> Result<(), NetError> {
        let slot = self.slot(id)?;
        let header = Header {
            tick: self.tick,
            baseline: 0,
            last_command: slot.last_command,
            id,
            player: slot.player,
        };
        let world = find(&self.history, self.tick).unwrap_or(&World::EMPTY);
        let baseline = find(&self.history, slot.read).filter(|world| world.tick < self.tick);
        snapshot::write(packet, &header, world, baseline);
        Ok(())
    }

    /// How many ticks have been run.
    #[must_use]
    pub fn tick_number(&self) -> u32 {
        self.tick
    }

    /// The player `id`, if there is one.
    #[must_use]
    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.slot(id).ok().map(|slot| &slot.player)
    }

    /// The player `id`, to move or change them: a respawn, a teleport, a
    /// push from an explosion. Their client is corrected by the next
    /// snapshot.
    #[must_use]
    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.slot_mut(id).ok().map(|slot| &mut slot.player)
    }

    /// Everyone on the server, lowest number first.
    pub fn players(&self) -> impl Iterator<Item = (PlayerId, &Player)> + '_ {
        (0..)
            .zip(&self.slots)
            .filter_map(|(id, slot)| Some((id, &slot.as_ref()?.player)))
    }

    /// The input that moved the player `id` during the last tick: what to
    /// give their weapon.
    #[must_use]
    pub fn input(&self, id: PlayerId) -> Option<PlayerInput> {
        self.slot(id).ok().map(|slot| slot.ran.input())
    }

    /// The player `id` as clients were told they were at `tick`, for as
    /// long as the server remembers it: the last 32 ticks.
    #[must_use]
    pub fn player_at(&self, id: PlayerId, tick: u32) -> Option<RemotePlayer> {
        let seen = find(&self.history, tick)?.players.get(usize::from(id))?;
        seen.as_ref().map(|seen| seen.remote(id))
    }

    fn slot(&self, id: PlayerId) -> Result<&Slot, NetError> {
        self.slots
            .get(usize::from(id))
            .and_then(Option::as_ref)
            .ok_or(NetError::NoSuchPlayer)
    }

    fn slot_mut(&mut self, id: PlayerId) -> Result<&mut Slot, NetError> {
        self.slots
            .get_mut(usize::from(id))
            .and_then(Option::as_mut)
            .ok_or(NetError::NoSuchPlayer)
    }
}
