//! Reads a session recorded on a Switch by ReFramed's server plugin
//! (https://github.com/reframedultimate/ReFramed-server), so tests run against how
//! fighters really move instead of numbers made up for the test.
//!
//! File: records of [delay bytes until 0][len u8][message]. A fighter state message is
//! 30 bytes, big-endian: type 15, frame u32, entry u8, pos x/y f32, damage*50 u16,
//! hitstun*100 u16, shield*200 u16, status u16, motion (5 bytes), hit status u8,
//! stocks u8, flags u8 (bit 1: facing right).

#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::Path;

use training_mod_replay::{FighterState, MatchState};

/// FIGHTER_STATUS_KIND_* values, from ReFramed's data/mappingInfo.json
pub mod status {
    pub const WAIT: i32 = 0;
    pub const WALK: i32 = 1;
    pub const DASH: i32 = 3;
    pub const RUN: i32 = 4;
    pub const JUMP: i32 = 11;
    pub const FALL: i32 = 14;
    pub const FALL_AERIAL: i32 = 15;
    pub const SQUAT_WAIT: i32 = 18;
    pub const LANDING: i32 = 22;
    pub const GUARD_ON: i32 = 27;
    pub const GUARD: i32 = 28;
    pub const ESCAPE_AIR: i32 = 34;
    pub const DAMAGE_AIR: i32 = 72;
    pub const DEAD: i32 = 181;
    pub const REBIRTH: i32 = 182;
    pub const STANDBY: i32 = 470;
}

/// What a save state brings back exactly in this test setup
pub const RESTORABLE: [i32; 7] = [
    status::WAIT,
    status::WALK,
    status::RUN,
    status::SQUAT_WAIT,
    status::GUARD,
    status::FALL,
    status::FALL_AERIAL,
];

pub struct Session {
    pub stage: u16,
    pub fighters: [u8; 2],
    pub frames: VecDeque<MatchState>,
}

const FIGHTER_STATE: u8 = 15;
const TRAINING_RESUME: u8 = 12;

/// Pikachu (player) against Joker (CPU) on Final Destination in training mode:
/// a grab, up-airs, save state reloads and respawns, 1496 frames
pub fn pikachu_joker() -> Session {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reframed_pikachu_joker.raw");
    let bytes = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing, run tests/fixtures/fetch.sh first",
            path.display()
        )
    });
    parse(&bytes)
}

fn be_f32(b: &[u8]) -> f32 {
    f32::from_be_bytes(b.try_into().unwrap())
}

fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes(b.try_into().unwrap())
}

pub fn parse(bytes: &[u8]) -> Session {
    let mut messages = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while bytes[i] != 0 {
            i += 1;
        }
        let len = bytes[i + 1] as usize;
        messages.push(&bytes[i + 2..i + 2 + len]);
        i += 2 + len;
    }

    let start = messages[0];
    assert_eq!(start[0], TRAINING_RESUME, "expected a training session");
    let stage = be_u16(&start[1..3]);
    let fighters = [start[3], start[4]];

    let mut frames = VecDeque::new();
    let mut previous: [Option<FighterState>; 2] = [None, None];
    let mut current: [Option<FighterState>; 2] = [None, None];
    for m in messages.iter().filter(|m| m[0] == FIGHTER_STATE) {
        assert_eq!(m.len(), 30);
        let entry = m[5] as usize;
        let pos_x = be_f32(&m[6..10]);
        let pos_y = be_f32(&m[10..14]);
        // Speed is not recorded; the distance moved since the last frame is the same thing
        let (speed_x, speed_y) =
            previous[entry].map_or((0.0, 0.0), |p| (pos_x - p.pos_x, pos_y - p.pos_y));
        let state = FighterState {
            pos_x,
            pos_y,
            speed_x,
            speed_y,
            lr: if m[29] & 0b10 != 0 { 1.0 } else { -1.0 },
            damage: be_u16(&m[14..16]) as f32 / 50.0,
            hitstun: be_u16(&m[16..18]) as f32 / 100.0,
            shield: be_u16(&m[18..20]) as f32 / 200.0,
            status_kind: be_u16(&m[20..22]) as i32,
            situation_kind: 0,
            jumps_used: 0,
        };
        previous[entry] = Some(state);
        current[entry] = Some(state);
        if let [Some(player), Some(cpu)] = current {
            frames.push_back(MatchState { player, cpu });
            current = [None, None];
        }
    }
    Session {
        stage,
        fighters,
        frames,
    }
}
