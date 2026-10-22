//! Compact little-endian binary format for saving replay slots to the SD card

use crate::frames::{CpuFrame, FighterSnapshot, MatchSnapshot};
use crate::start::{FighterState, MatchState};

pub const MAGIC: [u8; 4] = *b"UTMR";
pub const VERSION: u8 = 1;
/// magic + version + len (u32) + mirror (f32) + context (3 x i32)
pub const HEADER_SIZE: usize = 4 + 1 + 4 + 4 + 12;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum DecodeError {
    BadMagic,
    UnsupportedVersion(u8),
    /// Recording is longer than the slot capacity
    TooLong(usize),
    Truncated,
    TrailingBytes,
    SlotOutOfRange,
    /// Slots can only be replaced while no replay runs
    Busy,
}

pub trait Codec: Sized {
    fn encode(&self, out: &mut Vec<u8>);
    fn decode(input: &mut &[u8]) -> Option<Self>;
}

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_i8(out: &mut Vec<u8>, v: i8) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Reads N bytes and advances the input, or leaves it as is if too short
pub fn take<const N: usize>(input: &mut &[u8]) -> Option<[u8; N]> {
    let (head, rest) = input.split_first_chunk::<N>()?;
    *input = rest;
    Some(*head)
}

pub fn take_u32(input: &mut &[u8]) -> Option<u32> {
    take::<4>(input).map(u32::from_le_bytes)
}

pub fn take_i32(input: &mut &[u8]) -> Option<i32> {
    take::<4>(input).map(i32::from_le_bytes)
}

pub fn take_f32(input: &mut &[u8]) -> Option<f32> {
    take::<4>(input).map(f32::from_le_bytes)
}

pub fn take_i8(input: &mut &[u8]) -> Option<i8> {
    take::<1>(input).map(i8::from_le_bytes)
}

impl Codec for CpuFrame {
    fn encode(&self, out: &mut Vec<u8>) {
        put_u32(out, self.buttons);
        for v in [
            self.stick_x,
            self.stick_y,
            self.clamped_lstick_x,
            self.clamped_lstick_y,
            self.clamped_rstick_x,
            self.clamped_rstick_y,
        ] {
            put_f32(out, v);
        }
        self.command_flags.iter().for_each(|f| put_i32(out, *f));
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        let mut command_flags = [0; 4];
        let frame = CpuFrame {
            buttons: take_u32(input)?,
            stick_x: take_f32(input)?,
            stick_y: take_f32(input)?,
            clamped_lstick_x: take_f32(input)?,
            clamped_lstick_y: take_f32(input)?,
            clamped_rstick_x: take_f32(input)?,
            clamped_rstick_y: take_f32(input)?,
            command_flags: {
                for f in command_flags.iter_mut() {
                    *f = take_i32(input)?;
                }
                command_flags
            },
        };
        Some(frame)
    }
}

impl Codec for FighterSnapshot {
    fn encode(&self, out: &mut Vec<u8>) {
        put_f32(out, self.pos_x);
        put_f32(out, self.pos_y);
        put_f32(out, self.damage);
        put_i32(out, self.status_kind);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(FighterSnapshot {
            pos_x: take_f32(input)?,
            pos_y: take_f32(input)?,
            damage: take_f32(input)?,
            status_kind: take_i32(input)?,
        })
    }
}

impl Codec for MatchSnapshot {
    fn encode(&self, out: &mut Vec<u8>) {
        self.player.encode(out);
        self.cpu.encode(out);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(MatchSnapshot {
            player: FighterSnapshot::decode(input)?,
            cpu: FighterSnapshot::decode(input)?,
        })
    }
}

impl Codec for FighterState {
    fn encode(&self, out: &mut Vec<u8>) {
        for v in [
            self.pos_x,
            self.pos_y,
            self.speed_x,
            self.speed_y,
            self.lr,
            self.damage,
            self.shield,
            self.hitstun,
        ] {
            put_f32(out, v);
        }
        put_i32(out, self.status_kind);
        put_i32(out, self.situation_kind);
        put_i32(out, self.jumps_used);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(FighterState {
            pos_x: take_f32(input)?,
            pos_y: take_f32(input)?,
            speed_x: take_f32(input)?,
            speed_y: take_f32(input)?,
            lr: take_f32(input)?,
            damage: take_f32(input)?,
            shield: take_f32(input)?,
            hitstun: take_f32(input)?,
            status_kind: take_i32(input)?,
            situation_kind: take_i32(input)?,
            jumps_used: take_i32(input)?,
        })
    }
}

impl Codec for MatchState {
    fn encode(&self, out: &mut Vec<u8>) {
        self.player.encode(out);
        self.cpu.encode(out);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(MatchState {
            player: FighterState::decode(input)?,
            cpu: FighterState::decode(input)?,
        })
    }
}
