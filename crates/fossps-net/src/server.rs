//! The server's side: runs every player's commands and writes snapshots.

use fossps_physics::{Level, MoveSettings, Player};

use crate::command::{Command, PlayerInput, View, read_commands};
use crate::snapshot::{
    self, HISTORY, Header, MAX_PLAYERS, PlayerId, RemotePlayer, Seen, World, between, find, place,
};
use crate::wire::NetError;

/// The most commands kept waiting for one player. One that sends faster
/// than the server ticks loses its oldest commands and gains nothing.
const LONGEST_QUEUE: usize = 8;

/// How many more commands than the buffer asks for may wait before the
/// oldest are dropped. Without this a burst of late packets would leave
/// the player that many ticks behind for good.
const SLACK: usize = 2;

/// How many ticks a late player's last command is repeated before the
/// server lets go of their keys for them.
const MOST_REPEATS: u8 = 5;

/// The most ticks a shot can be judged back: what the history holds, less
/// the tick being written and the one after the view.
const MOST_REWIND: u32 = HISTORY as u32 - 2;

/// The numbers that shape how a server treats its players' commands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServerSettings {
    /// How many commands to have waiting before running a player's
    /// commands, at the start and again after running out. More rides out
    /// a rougher connection; each one adds a tick of delay to that
    /// player's moves as others see them. From 1 to 8.
    ///
    /// When more than two over this many are waiting, after a burst of
    /// late packets or from a client running fast, the oldest are dropped
    /// to get back to it.
    pub buffer: usize,
    /// The furthest back in time, in seconds, a shot is judged: see
    /// [`Server::seen_by`]. A player whose view of the others is older
    /// than this has to aim ahead of them by the difference. Longer is
    /// kinder to players on slow connections and harder on those they
    /// shoot, who are hit further behind cover. Zero judges every shot
    /// against where everyone is now. At most 30 ticks.
    pub rewind_limit: f32,
}

impl Default for ServerSettings {
    /// Two commands waiting (33 ms at 60 ticks per second), and shots
    /// judged up to a fifth of a second back.
    fn default() -> Self {
        Self {
            buffer: 2,
            rewind_limit: 0.2,
        }
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
    /// What the player had on screen when they gave the last command run.
    /// It never goes back in time.
    view: View,
    /// The player as written into the last tick's world.
    shown: Option<Seen>,
    /// The first tick whose record of this player may be shot at: the
    /// one they joined in, or the one they were last put somewhere new.
    since: u32,
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
    /// The most ticks a shot is judged back.
    rewind: u32,
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
            rewind: {
                let ticks = libm::roundf(settings.rewind_limit / step);
                // Not a number, or below zero, is no rewinding at all; the
                // cast cannot cut anything off after the clamp.
                if ticks > 0.0 {
                    ticks.min(MOST_REWIND as f32) as u32
                } else {
                    0
                }
            },
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
        let since = self.tick.saturating_add(1);
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
            view: View::default(),
            shown: None,
            since,
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
        let tick = self.tick;
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
        // A client cannot have read a snapshot that is not written yet.
        slot.read = slot.read.max(header.snapshot.min(tick));
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
            if slot.waiting > self.buffer + SLACK {
                // Too far behind: skip to one more than the buffer, so
                // that the buffer is what is left after this tick.
                let dropped = slot.waiting - self.buffer - 1;
                slot.queue.copy_within(dropped.., 0);
                slot.waiting -= dropped;
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
                slot.view = slot.view.max(command.view());
                command
            };
            slot.player
                .step(level, settings, &slot.ran.movement(), self.step);
            let now = Seen::of(&slot.player, slot.ran.look());
            if slot.shown.is_some_and(|before| now.jumped_from(&before)) {
                // Respawned or teleported: nobody may shoot them where
                // they were before.
                slot.since = self.tick;
            }
            slot.shown = Some(now);
            *seen = Some(now);
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
    /// A snapshot written before the first tick says nothing yet, and a
    /// client skips it.
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

    /// Everyone but the player `id`, where `id` saw them when giving the
    /// command that ran this tick: what to shoot at when `id` fires.
    ///
    /// This is lag compensation. A player sees the others a little in the
    /// past, so a shot aimed right at someone is judged against where
    /// that someone was on the shooter's screen, not where they have got
    /// to since. Build this tick's targets for `id`'s shots from these;
    /// leave out anyone who is dead.
    ///
    /// The shooter is not moved back: use [`player`](Self::player) for
    /// where the shot starts. Nobody is put back further than
    /// [`ServerSettings::rewind_limit`], nor to before they joined,
    /// respawned or were teleported (moved over 2 m in one tick).
    /// Things that take time to arrive, such as rockets and grenades, fly
    /// through the present: step them against [`players`](Self::players).
    ///
    /// Empty if there is no player `id`.
    ///
    /// ```
    /// use fossps_net::{Server, ServerSettings};
    /// use fossps_physics::{Level, MoveSettings, Player};
    /// use fossps_weapons::{hitscan, Target, WeaponSettings};
    ///
    /// # fn main() -> Result<(), fossps_net::NetError> {
    /// let level = Level::from_triangles(&[]);
    /// let rules = MoveSettings::default();
    /// let mut server = Server::new(1.0 / 60.0, ServerSettings::default());
    /// let shooter = server.join(Player::at([0.0, 0.0, 0.0], &rules))?;
    /// let enemy = server.join(Player::at([0.0, 0.0, -10.0], &rules))?;
    /// server.tick(&level, &rules);
    ///
    /// // When the shooter's weapon fires this tick:
    /// let targets: Vec<Target> = server
    ///     .seen_by(shooter)
    ///     .map(|other| Target {
    ///         id: other.id.into(),
    ///         feet: other.position,
    ///         height: if other.crouched { rules.crouched_height } else { rules.standing_height },
    ///         radius: rules.radius,
    ///     })
    ///     .collect();
    /// let eyes = server.player(shooter).map(Player::eye_position).unwrap_or_default();
    /// let hit = hitscan(&level, &targets, eyes, [0.0, 0.0, -1.0], &WeaponSettings::default(), None);
    /// assert_eq!(hit.and_then(|hit| hit.target()), Some(enemy.into()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn seen_by(&self, id: PlayerId) -> impl Iterator<Item = RemotePlayer> + '_ {
        let view = self.slot(id).ok().map(|slot| self.rewound(slot.view));
        (0..).zip(&self.slots).filter_map(move |(other, slot)| {
            let (view, slot) = (view?, slot.as_ref()?);
            (other != id).then(|| self.at(other, slot, view))
        })
    }

    /// How far back in time the player `id`'s shots are judged this tick,
    /// in seconds: what [`seen_by`](Self::seen_by) goes by, though someone
    /// who has just joined or respawned is put back less far. Zero if
    /// there is no such player.
    #[must_use]
    pub fn rewind(&self, id: PlayerId) -> f32 {
        self.slot(id).map_or(0.0, |slot| {
            let view = self.rewound(slot.view);
            let ticks = self.tick.saturating_sub(view.tick);
            // At most 32 ticks, so the cast is exact.
            (ticks as f32 - f32::from(view.part) / 256.0) * self.step
        })
    }

    /// `view` held to what a shot may be judged against: no older than the
    /// limit and no newer than now.
    fn rewound(&self, view: View) -> View {
        let now = View::at(self.tick);
        if view.tick == 0 {
            // No snapshot read yet: nothing was on screen to aim at.
            return now;
        }
        view.clamp(View::at(self.tick.saturating_sub(self.rewind)), now)
    }

    /// The player `id`, in `slot`, as they were at `view`.
    fn at(&self, id: PlayerId, slot: &Slot, view: View) -> RemotePlayer {
        let view = view.max(View::at(slot.since));
        let record = |tick| {
            find(&self.history, tick).and_then(|world| world.players.get(usize::from(id))?.as_ref())
        };
        let now = Seen::of(&slot.player, slot.ran.look());
        if slot.shown.is_some_and(|shown| now.jumped_from(&shown)) {
            // Put somewhere new since the last tick.
            return now.remote(id);
        }
        let Some(from) = record(view.tick) else {
            // Joined since the last tick.
            return now.remote(id);
        };
        match record(view.tick.saturating_add(1)) {
            Some(to) if view.part > 0 => between(from, to, view.part, id),
            _ => from.remote(id),
        }
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
