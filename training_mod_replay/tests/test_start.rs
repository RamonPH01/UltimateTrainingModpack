use std::collections::VecDeque;

use training_mod_replay::*;

const WAIT: i32 = 0;
const WALK: i32 = 1;
const ATTACK: i32 = 39;
const RESTORABLE: [i32; 2] = [WAIT, WALK];

fn fighter(x: f32, status: i32) -> FighterState {
    FighterState {
        pos_x: x,
        pos_y: 0.0,
        lr: 1.0,
        shield: 50.0,
        status_kind: status,
        ..Default::default()
    }
}

fn both(x: f32, player: i32, cpu: i32) -> MatchState {
    MatchState {
        player: fighter(x, player),
        cpu: fighter(-x, cpu),
    }
}

/// Fighters walking 1 unit per frame; `attacks` frames are not restorable for the player
fn history(len: usize, attacks: &[usize]) -> VecDeque<MatchState> {
    (0..len)
        .map(|i| {
            let status = if attacks.contains(&i) { ATTACK } else { WALK };
            both(i as f32, status, WAIT)
        })
        .collect()
}

// --- restorable ---

#[test]
fn listed_status_without_hitstun_is_restorable() {
    assert!(fighter(0.0, WAIT).is_restorable(&RESTORABLE));
    assert!(fighter(0.0, WALK).is_restorable(&RESTORABLE));
}

#[test]
fn unlisted_status_is_not_restorable() {
    assert!(!fighter(0.0, ATTACK).is_restorable(&RESTORABLE));
    assert!(!fighter(0.0, WAIT).is_restorable(&[]));
}

#[test]
fn hitstun_is_not_restorable() {
    // Tumbling down in hitstun can share a status with a free fall
    let mut f = fighter(0.0, WAIT);
    f.hitstun = 0.01;
    assert!(!f.is_restorable(&RESTORABLE));
}

#[test]
fn broken_numbers_are_not_restorable() {
    for field in 0..4 {
        let mut f = fighter(0.0, WAIT);
        match field {
            0 => f.pos_x = f32::NAN,
            1 => f.pos_y = f32::INFINITY,
            2 => f.speed_x = f32::NAN,
            _ => f.speed_y = f32::NAN,
        }
        assert!(!f.is_restorable(&RESTORABLE), "field {field}");
    }
}

#[test]
fn both_fighters_must_be_restorable() {
    assert!(both(0.0, WAIT, WALK).is_restorable(&RESTORABLE));
    assert!(!both(0.0, ATTACK, WAIT).is_restorable(&RESTORABLE));
    assert!(!both(0.0, WAIT, ATTACK).is_restorable(&RESTORABLE));
}

// --- continuity ---

#[test]
fn moving_up_to_max_step_continues() {
    let a = fighter(0.0, WAIT);
    let mut b = fighter(MAX_STEP, WAIT);
    assert!(b.continues(&a));
    b.pos_x = -MAX_STEP;
    assert!(b.continues(&a));
    b = fighter(0.0, WAIT);
    b.pos_y = MAX_STEP;
    assert!(b.continues(&a));
}

#[test]
fn moving_beyond_max_step_is_a_teleport() {
    let a = fighter(0.0, WAIT);
    assert!(!fighter(MAX_STEP + 0.01, WAIT).continues(&a));
    let mut b = fighter(0.0, WAIT);
    b.pos_y = -(MAX_STEP + 0.01);
    assert!(!b.continues(&a));
}

#[test]
fn nan_position_never_continues() {
    let a = fighter(0.0, WAIT);
    assert!(!fighter(f32::NAN, WAIT).continues(&a));
    assert!(!a.continues(&fighter(f32::NAN, WAIT)));
}

#[test]
fn match_continues_only_if_both_do() {
    let a = both(0.0, WAIT, WAIT);
    let mut b = a;
    b.cpu.pos_x = 100.0;
    assert!(!b.continues(&a));
    let mut b = a;
    b.player.pos_y = 100.0;
    assert!(!b.continues(&a));
    assert!(a.continues(&a));
}

// --- find_start ---

#[test]
fn mark_itself_when_restorable() {
    assert_eq!(find_start(&history(10, &[]), 5, 3, &RESTORABLE), Some(5));
}

#[test]
fn nearest_restorable_frame_before_mark() {
    // Player attacks on 6..=8, marks on 8
    assert_eq!(
        find_start(&history(10, &[6, 7, 8]), 8, 5, &RESTORABLE),
        Some(5)
    );
}

#[test]
fn window_is_inclusive() {
    let h = history(10, &[6, 7, 8]);
    assert_eq!(find_start(&h, 8, 3, &RESTORABLE), Some(5));
    assert_eq!(find_start(&h, 8, 2, &RESTORABLE), None);
}

#[test]
fn never_crosses_a_teleport() {
    // Respawn between 3 and 4: frames 0..=3 are restorable but on the other side
    let mut h = history(10, &[4, 5, 6, 7]);
    for (i, f) in h.iter_mut().enumerate().skip(4) {
        f.player.pos_x = 200.0 + i as f32;
    }
    assert_eq!(find_start(&h, 7, 9, &RESTORABLE), None);
}

#[test]
fn frame_right_after_a_teleport_is_not_picked() {
    // Frame 4 is where the fighter landed after the teleport: it may be restorable,
    // but the inputs before it belong to the other side of the teleport
    let mut h = history(10, &[]);
    for (i, f) in h.iter_mut().enumerate().skip(4) {
        f.player.pos_x = 200.0 + i as f32;
    }
    assert_eq!(find_start(&h, 4, 9, &RESTORABLE), None);
    assert_eq!(find_start(&h, 6, 9, &RESTORABLE), Some(6));
    assert_eq!(find_start(&h, 3, 9, &RESTORABLE), Some(3));
}

#[test]
fn first_frame_is_never_picked() {
    let h = history(10, &[1, 2]);
    assert_eq!(find_start(&h, 2, 9, &RESTORABLE), None);
    assert_eq!(find_start(&h, 0, 9, &RESTORABLE), None);
}

#[test]
fn mark_past_the_end_finds_nothing() {
    let h = history(10, &[]);
    assert_eq!(find_start(&h, 10, 9, &RESTORABLE), None);
    assert_eq!(find_start(&VecDeque::new(), 0, 9, &RESTORABLE), None);
}

#[test]
fn works_on_a_wrapped_ring_buffer() {
    // The capture keeps a fixed size buffer: old frames drop off the front
    let mut h = VecDeque::with_capacity(8);
    for i in 0..20 {
        if h.len() == 8 {
            h.pop_front();
        }
        h.push_back(both(i as f32, if i >= 17 { ATTACK } else { WALK }, WAIT));
    }
    // Frame 16 of the stream is index 4 in the buffer
    assert_eq!(find_start(&h, 7, 7, &RESTORABLE), Some(4));
}

// --- mirror and codec ---

#[test]
fn mirror_flips_horizontal_values_only() {
    let f = FighterState {
        pos_x: 3.0,
        pos_y: 4.0,
        speed_x: 1.5,
        speed_y: -2.0,
        lr: 1.0,
        damage: 12.0,
        shield: 40.0,
        hitstun: 0.0,
        status_kind: 4,
        situation_kind: 2,
        jumps_used: 1,
    };
    let m = f.mirrored();
    assert_eq!(
        m,
        FighterState {
            pos_x: -3.0,
            speed_x: -1.5,
            lr: -1.0,
            ..f
        }
    );
    assert_eq!(m.mirrored(), f);
}

#[test]
fn match_state_mirrors_both() {
    let s = both(5.0, WAIT, WALK);
    let m = s.mirrored();
    assert_eq!(m.player.pos_x, -5.0);
    assert_eq!(m.cpu.pos_x, 5.0);
    assert_eq!(m.player.lr, -1.0);
}

fn sample() -> MatchState {
    MatchState {
        player: FighterState {
            pos_x: -12.5,
            pos_y: 30.25,
            speed_x: 1.75,
            speed_y: -0.5,
            lr: -1.0,
            damage: 47.3,
            shield: 38.0,
            hitstun: 0.0,
            status_kind: 14,
            situation_kind: 2,
            jumps_used: 1,
        },
        cpu: FighterState {
            pos_x: 40.0,
            pos_y: 0.0,
            speed_x: 0.0,
            speed_y: 0.0,
            lr: 1.0,
            damage: 0.0,
            shield: 50.0,
            hitstun: 12.5,
            status_kind: 72,
            situation_kind: 0,
            jumps_used: 2,
        },
    }
}

#[test]
fn state_roundtrips_through_codec() {
    let mut bytes = Vec::new();
    sample().encode(&mut bytes);
    assert_eq!(bytes.len(), 2 * 44);
    let mut input = bytes.as_slice();
    assert_eq!(MatchState::decode(&mut input), Some(sample()));
    assert!(input.is_empty());
}

#[test]
fn truncated_state_does_not_decode() {
    let mut bytes = Vec::new();
    sample().encode(&mut bytes);
    for len in 0..bytes.len() {
        let mut input = &bytes[..len];
        assert_eq!(MatchState::decode(&mut input), None, "len {len}");
    }
}
