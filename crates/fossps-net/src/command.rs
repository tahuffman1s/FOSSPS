//! What a player asked for during one tick, in the form that is sent.

use std::f32::consts::{FRAC_PI_2, TAU};

use fossps_physics::MoveInput;

use crate::wire::{COMMANDS, NetError, Reader, Writer};

/// The most commands one packet may carry.
pub(crate) const MOST_COMMANDS: usize = 8;

const JUMP: u16 = 1 << 0;
const CROUCH: u16 = 1 << 1;
const SPRINT: u16 = 1 << 2;
const FIRE: u16 = 1 << 3;
const AIM: u16 = 1 << 4;
const RELOAD: u16 = 1 << 5;

/// Radians in one step of the yaw that is sent: 65 536 steps to a turn.
const YAW_STEP: f32 = TAU / 65_536.0;
/// Radians in one step of the pitch that is sent.
const PITCH_STEP: f32 = FRAC_PI_2 / 32_767.0;

/// Everything a player is asking for during one tick.
///
/// Fill it from the action map once per tick and hand it to
/// [`Client::predict`](crate::Client::predict).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerInput {
    /// Walking, jumping, crouching, sprinting and which way the player
    /// faces.
    pub movement: MoveInput,
    /// Looking up (positive) or down, in radians. Straight up and straight
    /// down are the limits.
    pub pitch: f32,
    /// The trigger is held.
    pub fire: bool,
    /// Aiming down the sights.
    pub aim: bool,
    /// Reload is held.
    pub reload: bool,
}

/// One tick of a player's input, rounded to what fits in eight bytes,
/// and which snapshot the player had on screen when they gave it.
///
/// The client moves its own player with the rounded input, not with what
/// the stick or mouse said exactly, so the server, given the same
/// command, works out the very same move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Command {
    sequence: u32,
    forward: i8,
    right: i8,
    yaw: u16,
    pitch: i16,
    buttons: u16,
    view: View,
}

/// The moment of the server's past a client was showing the other players
/// at: a tick, and how far on towards the next one in 256ths.
///
/// Tick 0 is "nothing yet". Views order by time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct View {
    pub(crate) tick: u32,
    pub(crate) part: u8,
}

impl View {
    /// Exactly at `tick`.
    pub(crate) fn at(tick: u32) -> Self {
        Self { tick, part: 0 }
    }
}

impl Command {
    /// The command for `input`, numbered `sequence`. A client numbers its
    /// commands 1, 2, 3 and so on.
    #[must_use]
    pub fn new(sequence: u32, input: &PlayerInput) -> Self {
        let movement = &input.movement;
        let mut buttons = 0;
        for (held, bit) in [
            (movement.jump, JUMP),
            (movement.crouch, CROUCH),
            (movement.sprint, SPRINT),
            (input.fire, FIRE),
            (input.aim, AIM),
            (input.reload, RELOAD),
        ] {
            if held {
                buttons |= bit;
            }
        }
        Self {
            sequence,
            forward: axis(movement.forward),
            right: axis(movement.right),
            yaw: yaw_steps(movement.yaw),
            pitch: pitch_steps(input.pitch),
            buttons,
            view: View::default(),
        }
    }

    /// This command, given while the other players were shown as of
    /// `view`.
    pub(crate) fn seeing(mut self, view: View) -> Self {
        self.view = view;
        self
    }

    /// What the other players were shown as of when this command was
    /// given.
    pub(crate) fn view(&self) -> View {
        self.view
    }

    /// The number the client gave this command.
    #[must_use]
    pub fn sequence(&self) -> u32 {
        self.sequence
    }

    /// The input this command stands for, after rounding.
    #[must_use]
    pub fn input(&self) -> PlayerInput {
        PlayerInput {
            movement: self.movement(),
            pitch: f32::from(self.pitch) * PITCH_STEP,
            fire: self.has(FIRE),
            aim: self.has(AIM),
            reload: self.has(RELOAD),
        }
    }

    /// The movement half of [`input`](Self::input): what to give
    /// [`Player::step`](fossps_physics::Player::step).
    #[must_use]
    pub fn movement(&self) -> MoveInput {
        MoveInput {
            forward: f32::from(self.forward) / 127.0,
            right: f32::from(self.right) / 127.0,
            yaw: f32::from(self.yaw) * YAW_STEP,
            jump: self.has(JUMP),
            crouch: self.has(CROUCH),
            sprint: self.has(SPRINT),
        }
    }

    fn has(&self, bit: u16) -> bool {
        self.buttons & bit != 0
    }

    /// The yaw and pitch as they are sent.
    pub(crate) fn look(&self) -> (u16, i16) {
        (self.yaw, self.pitch)
    }

    /// This command as a server repeats it when the next one is late: the
    /// player keeps moving the same way, but does not fire or reload.
    pub(crate) fn repeated(mut self) -> Self {
        self.buttons &= !(FIRE | RELOAD);
        self
    }

    /// This command with everything let go, still looking the same way.
    pub(crate) fn idle(self) -> Self {
        Self {
            forward: 0,
            right: 0,
            buttons: 0,
            ..self
        }
    }

    fn write(&self, writer: &mut Writer<'_>) {
        writer.i8(self.forward);
        writer.i8(self.right);
        writer.u16(self.yaw);
        writer.i16(self.pitch);
        writer.u16(self.buttons);
        writer.u32(self.view.tick);
        writer.u8(self.view.part);
    }

    fn read(reader: &mut Reader<'_>, sequence: u32) -> Result<Self, NetError> {
        let command = Self {
            sequence,
            forward: reader.i8()?,
            right: reader.i8()?,
            yaw: reader.u16()?,
            pitch: reader.i16()?,
            buttons: reader.u16()?,
            view: View {
                tick: reader.u32()?,
                part: reader.u8()?,
            },
        };
        // -128 and -32768 are never sent; they would be a push or a look
        // slightly past the limit.
        if command.forward == i8::MIN || command.right == i8::MIN || command.pitch == i16::MIN {
            return Err(NetError::BadValue);
        }
        Ok(command)
    }
}

/// A stick axis as a whole number from -127 to 127.
fn axis(value: f32) -> i8 {
    if value.is_finite() {
        // In range after the clamp, so the cast cannot cut anything off.
        libm::roundf(value.clamp(-1.0, 1.0) * 127.0) as i8
    } else {
        0
    }
}

/// An angle in radians as 65 536ths of a turn, whichever way and however
/// many times round it has gone.
pub(crate) fn yaw_steps(yaw: f32) -> u16 {
    if !yaw.is_finite() {
        return 0;
    }
    let steps = libm::roundf(yaw / YAW_STEP) as i64;
    // 0 to 65 535 after the remainder.
    steps.rem_euclid(65_536) as u16
}

fn pitch_steps(pitch: f32) -> i16 {
    if pitch.is_finite() {
        libm::roundf(pitch.clamp(-FRAC_PI_2, FRAC_PI_2) / PITCH_STEP) as i16
    } else {
        0
    }
}

/// The angle a sent yaw stands for, in radians.
pub(crate) fn yaw_radians(steps: u16) -> f32 {
    f32::from(steps) * YAW_STEP
}

/// The angle a sent pitch stands for, in radians.
pub(crate) fn pitch_radians(steps: i16) -> f32 {
    f32::from(steps) * PITCH_STEP
}

/// What a packet of commands carries besides the commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandsHeader {
    /// The tick of the newest snapshot the client has read, or 0.
    pub(crate) snapshot: u32,
}

/// Writes a packet of `commands`, newest first with numbers going down by
/// one, into `out`.
pub(crate) fn write_commands(
    out: &mut Vec<u8>,
    snapshot: u32,
    commands: impl Iterator<Item = Command>,
) {
    let mut writer = Writer::open(out, COMMANDS);
    writer.u32(snapshot);
    let newest_at = writer.len();
    writer.u32(0);
    let count_at = writer.len();
    writer.u8(0);
    let mut count = 0_u8;
    for command in commands.take(MOST_COMMANDS) {
        if count == 0 {
            for (place, byte) in command.sequence.to_le_bytes().into_iter().enumerate() {
                writer.set(newest_at + place, byte);
            }
        }
        command.write(&mut writer);
        count += 1;
    }
    writer.set(count_at, count);
}

/// Reads a packet of commands, calling `each` with them oldest first.
pub(crate) fn read_commands(
    bytes: &[u8],
    mut each: impl FnMut(Command),
) -> Result<CommandsHeader, NetError> {
    let mut reader = Reader::open(bytes, COMMANDS)?;
    let snapshot = reader.u32()?;
    let newest = reader.u32()?;
    let count = usize::from(reader.u8()?);
    if count > MOST_COMMANDS {
        return Err(NetError::BadValue);
    }
    let mut commands = [Command::default(); MOST_COMMANDS];
    for (back, slot) in (0_u32..).zip(commands.iter_mut().take(count)) {
        // Commands are numbered from 1, so there is none before the first.
        let sequence = newest
            .checked_sub(back)
            .filter(|&sequence| sequence > 0)
            .ok_or(NetError::BadValue)?;
        *slot = Command::read(&mut reader, sequence)?;
        // Nobody can have been looking at a snapshot newer than the
        // newest they had read when the packet left. A view part of the
        // way past that snapshot is refused too: there is nothing after
        // it to be shown part of the way towards.
        if slot.view > View::at(snapshot) {
            return Err(NetError::BadValue);
        }
    }
    for command in commands.iter().take(count).rev() {
        each(*command);
    }
    Ok(CommandsHeader { snapshot })
}
