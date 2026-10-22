mod common;
use common::*;
use training_mod_replay::codec::{HEADER_SIZE, MAGIC, VERSION};

fn sample() -> TestReplay {
    let mut r = Replay::with_slots(60, 3);
    r.set_context([11, -22, 33]);
    r.set_record_slot(1);
    r.request(ReplayMode::Record);
    r.on_state_loaded(-1.0);
    for i in 0..4 {
        frame_with(
            &mut r,
            Pad {
                button: i % 2 == 0,
                x: -(i as i8) * 30,
            },
            Cpu {
                stick_x: i as f32 * -0.25,
                cmd: i32::MIN + i,
            },
            Snap { x: i as f32 * 1.5 },
        );
    }
    r.request(ReplayMode::Record);
    r
}

fn bytes() -> Vec<u8> {
    sample().encode_slot(1).expect("slot 1 encodes")
}

/// Plays a slot completely and returns everything the game would see
fn play_all(r: &mut TestReplay, slot: usize, mirror: f32) -> Vec<(Pad, Cpu, Vec<ReplayEvent>)> {
    assert!(r.set_play_slot(slot));
    assert_eq!(r.request(ReplayMode::Play), (true, None));
    r.on_state_loaded(mirror);
    let mut out = Vec::new();
    while r.state() == ReplayState::Playing {
        let x = -(r.frame() as f32) * 1.5;
        out.push(frame_with(r, NEUTRAL, cpu(0), Snap { x }));
    }
    out
}

#[test]
fn encoded_size_is_exact() {
    assert_eq!(bytes().len(), HEADER_SIZE + 4 * TEST_FRAME_SIZE);
}

#[test]
fn header_layout() {
    let b = bytes();
    assert_eq!(&b[0..4], &MAGIC);
    assert_eq!(b[4], VERSION);
    assert_eq!(u32::from_le_bytes(b[5..9].try_into().unwrap()), 4);
    assert_eq!(f32::from_le_bytes(b[9..13].try_into().unwrap()), -1.0);
    assert_eq!(i32::from_le_bytes(b[13..17].try_into().unwrap()), 11);
    assert_eq!(i32::from_le_bytes(b[17..21].try_into().unwrap()), -22);
    assert_eq!(i32::from_le_bytes(b[21..25].try_into().unwrap()), 33);
}

#[test]
fn round_trip_plays_back_identically() {
    let mut original = sample();
    let mut restored: TestReplay = Replay::with_slots(60, 3);
    restored.set_context([11, -22, 33]);
    assert_eq!(restored.decode_slot(1, &bytes()), Ok(()));
    assert_eq!(restored.slot_len(1), 4);
    // Mirror -1 equals the recording mirror, desync snapshots are mirrored too
    let a = play_all(&mut original, 1, 1.0);
    let b = play_all(&mut restored, 1, 1.0);
    assert_eq!(a, b);
    assert_eq!(a.len(), 4);
}

#[test]
fn round_trip_keeps_context() {
    let mut restored: TestReplay = Replay::with_slots(60, 3);
    restored.decode_slot(0, &bytes()).unwrap();
    restored.set_context([0, 0, 0]);
    restored.set_play_slot(0);
    assert_eq!(
        restored.request(ReplayMode::Play).1,
        Some(ReplayEvent::Rejected(RejectReason::OtherFighters))
    );
}

#[test]
fn empty_slot_encodes_and_restores_as_empty() {
    let r = sample();
    let empty = r.encode_slot(0).unwrap();
    assert_eq!(empty.len(), HEADER_SIZE);
    let mut restored = sample();
    assert_eq!(restored.decode_slot(1, &empty), Ok(()));
    assert_eq!(restored.slot_len(1), 0);
}

#[test]
fn encode_out_of_range_slot_is_none() {
    assert!(sample().encode_slot(3).is_none());
}

#[test]
fn every_truncation_is_rejected() {
    let b = bytes();
    for cut in 0..b.len() {
        let mut r: TestReplay = Replay::with_slots(60, 3);
        let err = r.decode_slot(0, &b[..cut]).unwrap_err();
        assert!(
            matches!(err, DecodeError::Truncated | DecodeError::BadMagic),
            "cut at {cut}: {err:?}"
        );
        assert_eq!(r.slot_len(0), 0, "cut at {cut} must not touch the slot");
    }
}

#[test]
fn bad_magic_is_rejected() {
    let mut b = bytes();
    b[0] = b'X';
    let mut r: TestReplay = Replay::with_slots(60, 3);
    assert_eq!(r.decode_slot(0, &b), Err(DecodeError::BadMagic));
}

#[test]
fn unknown_version_is_rejected() {
    let mut b = bytes();
    b[4] = VERSION + 1;
    let mut r: TestReplay = Replay::with_slots(60, 3);
    assert_eq!(
        r.decode_slot(0, &b),
        Err(DecodeError::UnsupportedVersion(VERSION + 1))
    );
}

#[test]
fn trailing_bytes_are_rejected() {
    let mut b = bytes();
    b.push(0);
    let mut r: TestReplay = Replay::with_slots(60, 3);
    assert_eq!(r.decode_slot(0, &b), Err(DecodeError::TrailingBytes));
}

#[test]
fn recording_longer_than_capacity_is_rejected() {
    let mut r: TestReplay = Replay::with_slots(3, 1);
    assert_eq!(r.decode_slot(0, &bytes()), Err(DecodeError::TooLong(4)));
}

#[test]
fn huge_length_field_is_rejected_without_allocating() {
    let mut b = bytes();
    b[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut r: TestReplay = Replay::with_slots(60, 1);
    assert_eq!(
        r.decode_slot(0, &b),
        Err(DecodeError::TooLong(u32::MAX as usize))
    );
}

#[test]
fn decode_into_missing_slot_is_rejected() {
    let mut r: TestReplay = Replay::with_slots(60, 1);
    assert_eq!(r.decode_slot(1, &bytes()), Err(DecodeError::SlotOutOfRange));
}

#[test]
fn decode_while_active_is_rejected() {
    let mut r = sample();
    r.set_record_slot(0);
    r.request(ReplayMode::Record);
    assert_eq!(r.decode_slot(2, &bytes()), Err(DecodeError::Busy));
}

#[test]
fn failed_decode_leaves_existing_slot_untouched() {
    let mut r = sample();
    let mut b = bytes();
    b.pop();
    assert_eq!(r.decode_slot(1, &b), Err(DecodeError::Truncated));
    assert_eq!(r.slot_len(1), 4);
    assert_eq!(r.encode_slot(1).unwrap(), bytes());
}

// --- Plugin frame types ---

#[test]
fn cpu_frame_round_trip_is_bit_exact() {
    let f = CpuFrame {
        buttons: 0xDEAD_BEEF,
        stick_x: -0.0,
        stick_y: f32::MIN_POSITIVE,
        clamped_lstick_x: 1.0,
        clamped_lstick_y: -1.0,
        clamped_rstick_x: f32::NAN,
        clamped_rstick_y: 0.123_456_79,
        command_flags: [i32::MIN, -1, 0, i32::MAX],
    };
    let mut out = Vec::new();
    f.encode(&mut out);
    assert_eq!(out.len(), 44);
    let mut input = out.as_slice();
    let d = CpuFrame::decode(&mut input).unwrap();
    assert!(input.is_empty());
    assert_eq!(d.buttons, f.buttons);
    assert_eq!(d.stick_x.to_bits(), f.stick_x.to_bits());
    assert_eq!(d.clamped_rstick_x.to_bits(), f.clamped_rstick_x.to_bits());
    assert_eq!(d.command_flags, f.command_flags);
    assert_eq!(d.clamped_rstick_y, f.clamped_rstick_y);
}

#[test]
fn match_snapshot_round_trip() {
    let m = MatchSnapshot {
        player: FighterSnapshot {
            pos_x: -12.5,
            pos_y: 3.25,
            damage: 47.3,
            status_kind: 0x1e,
        },
        cpu: FighterSnapshot {
            pos_x: 80.0,
            pos_y: -0.5,
            damage: 0.0,
            status_kind: -1,
        },
    };
    let mut out = Vec::new();
    m.encode(&mut out);
    assert_eq!(out.len(), 32);
    let mut input = out.as_slice();
    assert_eq!(MatchSnapshot::decode(&mut input), Some(m));
    assert!(input.is_empty());
}

#[test]
fn frame_types_reject_short_input() {
    let mut out = Vec::new();
    CpuFrame::default().encode(&mut out);
    MatchSnapshot::default().encode(&mut out);
    for cut in 0..44 {
        assert!(
            CpuFrame::decode(&mut &out[..cut]).is_none(),
            "cpu cut {cut}"
        );
    }
    for cut in 0..32 {
        assert!(
            MatchSnapshot::decode(&mut &out[44..44 + cut]).is_none(),
            "snap cut {cut}"
        );
    }
}
