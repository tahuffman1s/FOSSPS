//! The client's side: moves its own player at once, and puts right what
//! the server sees differently.

use fossps_physics::{Level, MoveSettings, Player};

use crate::command::{Command, MOST_COMMANDS, PlayerInput, View, write_commands};
use crate::snapshot::{
    HISTORY, PlayerId, RemotePlayer, World, find, place, read_header, read_players,
};
use crate::wire::NetError;

/// How many of its own moves a client remembers: just over two seconds at
/// 60 ticks per second. With no word from the server for longer than
/// that, the next snapshot is simply taken as it is.
const MOVES: usize = 128;

/// The numbers that shape a client's side of the connection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientSettings {
    /// How many commands go in each packet: the new one and the ones
    /// before it again, in case their packets were lost. From 1 to 8.
    pub commands_per_packet: usize,
    /// How long, in seconds, a correction takes to shrink to about a
    /// third on screen. Zero shows every correction at once.
    pub smoothing_time: f32,
    /// A correction longer than this, in metres, is shown at once rather
    /// than glided over: a respawn or a teleport.
    pub snap_distance: f32,
}

impl Default for ClientSettings {
    /// Three commands a packet, corrections eased over a tenth of a
    /// second, and anything past two metres shown at once.
    fn default() -> Self {
        Self {
            commands_per_packet: 3,
            smoothing_time: 0.1,
            snap_distance: 2.0,
        }
    }
}

/// What [`Client::receive`] did with a snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Received {
    /// The server agrees with what the client predicted. Nothing changed.
    Matched,
    /// The server saw something else. The player was put where the server
    /// has them and the moves made since were made again from there.
    Corrected {
        /// How far the player ended up from where they were predicted
        /// to be, in metres.
        distance: f32,
    },
    /// The snapshot is older than one already read, or builds on one this
    /// client never read. It was ignored; nothing is wrong.
    Skipped,
}

/// One command and where it left the player.
#[derive(Debug, Clone, Copy)]
struct Move {
    command: Command,
    after: Player,
}

/// The client's half of the netcode: it moves the player the moment a
/// button is pressed, without waiting for the server, and corrects
/// itself when a snapshot disagrees.
///
/// Once per tick: hand every snapshot that arrived to
/// [`receive`](Self::receive), then call [`predict`](Self::predict) and
/// send the packet it writes.
///
/// A client is good for one stay on one server. Make a new one to join
/// again or when the server starts afresh: snapshots from a server that
/// has gone back to tick 1 would all be skipped as old.
#[derive(Debug, Clone)]
pub struct Client {
    step: f32,
    commands_per_packet: usize,
    /// What is left of the correction on screen after one tick.
    kept_per_tick: f32,
    snap_distance: f32,
    id: Option<PlayerId>,
    player: Player,
    /// The number of the newest command.
    sequence: u32,
    /// The last [`MOVES`] moves, each at its number's remainder.
    moves: Vec<Move>,
    /// The tick of the newest snapshot read, or 0.
    newest: u32,
    worlds: Vec<World>,
    /// Where a snapshot is read before it is known to be whole.
    reading: Box<World>,
    /// From where the player is to where they are shown.
    offset: [f32; 3],
}

impl Client {
    /// A client whose player starts as `player` and whose ticks are `step`
    /// seconds long: the same `step` as the server's, and the same
    /// `player` the server was given in [`Server::join`](crate::Server::join).
    #[must_use]
    pub fn new(player: Player, step: f32, settings: ClientSettings) -> Self {
        let smoothing = settings.smoothing_time;
        Self {
            step,
            commands_per_packet: settings.commands_per_packet.clamp(1, MOST_COMMANDS),
            kept_per_tick: if smoothing.is_finite() && smoothing > 0.0 && step.is_finite() {
                libm::expf(-step.max(0.0) / smoothing)
            } else {
                0.0
            },
            snap_distance: settings.snap_distance,
            id: None,
            player,
            sequence: 0,
            moves: vec![
                Move {
                    command: Command::default(),
                    after: player,
                };
                MOVES
            ],
            newest: 0,
            worlds: vec![World::EMPTY; HISTORY],
            reading: Box::new(World::EMPTY),
            offset: [0.0; 3],
        }
    }

    /// Moves the player by one tick of `input` right away, and writes the
    /// packet to send to the server into `packet`, replacing what was in
    /// it.
    ///
    /// Returns the command that was run: the input after rounding.
    pub fn predict(
        &mut self,
        level: &Level,
        settings: &MoveSettings,
        input: &PlayerInput,
        packet: &mut Vec<u8>,
    ) -> Command {
        self.sequence = self.sequence.saturating_add(1);
        // The other players are shown as of the newest snapshot, so that
        // is what the player was aiming at.
        let command = Command::new(self.sequence, input).seeing(View::at(self.newest));
        self.player
            .step(level, settings, &command.movement(), self.step);
        if let Some(slot) = self.moves.get_mut(place_of(self.sequence)) {
            *slot = Move {
                command,
                after: self.player,
            };
        }
        self.offset = self.offset.map(|part| part * self.kept_per_tick);

        let moves = &self.moves;
        let newest = self.sequence;
        let commands = (0..self.commands_per_packet as u32).map_while(|back| {
            let sequence = newest.checked_sub(back).filter(|&sequence| sequence > 0)?;
            moves
                .get(place_of(sequence))
                .map(|made| made.command)
                .filter(|command| command.sequence() == sequence)
        });
        write_commands(packet, self.newest, commands);
        command
    }

    /// Takes in a snapshot from the server.
    ///
    /// If the server has the player somewhere other than where this
    /// client predicted for the same command, the player is put where the
    /// server says and every command since is run again from there.
    ///
    /// # Errors
    ///
    /// A [`NetError`] if the packet cannot be read. A packet that fails
    /// changes nothing.
    pub fn receive(
        &mut self,
        level: &Level,
        settings: &MoveSettings,
        packet: &[u8],
    ) -> Result<Received, NetError> {
        let (header, mut reader) = read_header(packet)?;
        if header.tick <= self.newest {
            return Ok(Received::Skipped);
        }
        if header.last_command > self.sequence {
            // The server cannot have run a command that was never sent.
            return Err(NetError::BadValue);
        }
        let baseline = if header.baseline == 0 {
            None
        } else {
            match find(&self.worlds, header.baseline) {
                Some(world) => Some(world),
                None => return Ok(Received::Skipped),
            }
        };
        read_players(&mut reader, &header, baseline, &mut self.reading)?;
        if let Some(world) = self.worlds.get_mut(place(header.tick)) {
            *world = *self.reading;
        }
        self.newest = header.tick;
        self.id = Some(header.id);
        Ok(self.reconcile(level, settings, header.last_command, header.player))
    }

    /// Compares what the server says the player was after command
    /// `sequence` with what was predicted, and replays from there if the
    /// two differ.
    fn reconcile(
        &mut self,
        level: &Level,
        settings: &MoveSettings,
        sequence: u32,
        server: Player,
    ) -> Received {
        let predicted = self
            .moves
            .get(place_of(sequence))
            .filter(|made| made.command.sequence() == sequence)
            .map(|made| made.after);
        if predicted == Some(server) {
            return Received::Matched;
        }

        let before = self.player.position;
        let mut player = server;
        if predicted.is_some() {
            if let Some(made) = self.moves.get_mut(place_of(sequence)) {
                made.after = player;
            }
            for later in sequence.saturating_add(1)..=self.sequence {
                let Some(made) = self.moves.get_mut(place_of(later)) else {
                    break;
                };
                player.step(level, settings, &made.command.movement(), self.step);
                made.after = player;
            }
        }
        // With no record of that command (more than two seconds without a
        // snapshot) there is nothing to replay: take the server's word.
        self.player = player;

        let moved = [0, 1, 2].map(|axis| before[axis] - player.position[axis]);
        let distance = moved.iter().map(|part| part * part).sum::<f32>().sqrt();
        let shown = [0, 1, 2].map(|axis| self.offset[axis] + moved[axis]);
        let shown_far = shown.iter().map(|part| part * part).sum::<f32>().sqrt();
        // A distance that is not a number counts as too far.
        self.offset = if shown_far.is_finite() && shown_far <= self.snap_distance {
            shown
        } else {
            [0.0; 3]
        };
        Received::Corrected { distance }
    }

    /// The player as predicted: where the simulation has them now.
    #[must_use]
    pub fn player(&self) -> &Player {
        &self.player
    }

    /// Where to show the player's feet. After a correction this glides
    /// from where they were shown to where they are, instead of jumping.
    #[must_use]
    pub fn view_position(&self) -> [f32; 3] {
        [0, 1, 2].map(|axis| self.player.position[axis] + self.offset[axis])
    }

    /// The other players as of the newest snapshot, lowest number first.
    pub fn others(&self) -> impl Iterator<Item = RemotePlayer> + '_ {
        find(&self.worlds, self.newest)
            .into_iter()
            .flat_map(World::remotes)
    }

    /// This player's number on the server, once a snapshot has said it.
    #[must_use]
    pub fn id(&self) -> Option<PlayerId> {
        self.id
    }

    /// The server tick of the newest snapshot read, or 0 before the first.
    #[must_use]
    pub fn server_tick(&self) -> u32 {
        self.newest
    }

    /// The number of the newest command: how many times
    /// [`predict`](Self::predict) has been called.
    #[must_use]
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
}

/// Where the move numbered `sequence` is kept.
fn place_of(sequence: u32) -> usize {
    sequence as usize % MOVES
}
