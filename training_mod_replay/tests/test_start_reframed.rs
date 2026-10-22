//! Start point selection against a session recorded on a real Switch.
//! Needs tests/fixtures/fetch.sh to have run.

mod reframed;

use reframed::{status, RESTORABLE};
use training_mod_replay::*;

/// Two seconds, as the capture will search
const WINDOW: usize = 120;

fn is_respawn(s: i32) -> bool {
    matches!(s, status::DEAD | status::STANDBY | status::REBIRTH)
}

#[test]
fn fixture_is_the_expected_session() {
    let session = reframed::pikachu_joker();
    assert_eq!(session.frames.len(), 1496);
    // Final Destination, Pikachu, Joker
    assert_eq!(session.stage, 3);
    assert_eq!(session.fighters, [8, 82]);
}

#[test]
fn real_movement_never_counts_as_teleport() {
    let frames = reframed::pikachu_joker().frames;
    for i in 1..frames.len() {
        let (a, b) = (&frames[i - 1], &frames[i]);
        for (prev, cur) in [(a.player, b.player), (a.cpu, b.cpu)] {
            let respawn = is_respawn(prev.status_kind) || is_respawn(cur.status_kind);
            assert!(
                respawn || cur.continues(&prev),
                "frame {i}: status {} -> {} moved ({}, {}) -> ({}, {})",
                prev.status_kind,
                cur.status_kind,
                prev.pos_x,
                prev.pos_y,
                cur.pos_x,
                cur.pos_y
            );
        }
    }
}

#[test]
fn every_reload_is_a_teleport() {
    // Each save state reload goes through DEAD; position jumps around it
    let frames = reframed::pikachu_joker().frames;
    let reloads = (1..frames.len())
        .filter(|&i| {
            frames[i].player.status_kind == status::DEAD
                && frames[i - 1].player.status_kind != status::DEAD
        })
        .count();
    let teleports = (1..frames.len())
        .filter(|&i| !frames[i].player.continues(&frames[i - 1].player))
        .count();
    assert!(reloads >= 5, "only {reloads} reloads in the session");
    assert!(
        teleports >= reloads,
        "{teleports} teleports for {reloads} reloads"
    );
}

/// Whatever frame gets marked, the start found must be safe to restore
#[test]
#[allow(clippy::needless_range_loop)] // compares each frame with the one before
fn every_start_found_is_restorable_and_reaches_mark_without_teleport() {
    let frames = reframed::pikachu_joker().frames;
    let mut found = 0;
    for mark in 0..frames.len() {
        let Some(start) = find_start(&frames, mark, WINDOW, &RESTORABLE) else {
            continue;
        };
        found += 1;
        assert!(
            start >= 1 && start <= mark && mark - start <= WINDOW,
            "mark {mark}"
        );
        assert!(frames[start].is_restorable(&RESTORABLE), "mark {mark}");
        for i in start..=mark {
            assert!(
                frames[i].continues(&frames[i - 1]),
                "mark {mark}: teleport at {i}"
            );
            assert!(!is_respawn(frames[i].player.status_kind), "mark {mark}");
            assert!(!is_respawn(frames[i].cpu.status_kind), "mark {mark}");
        }
        // Nothing between start and mark would have been a better start
        for i in start + 1..=mark {
            assert!(
                !frames[i].is_restorable(&RESTORABLE),
                "mark {mark}: {i} was closer"
            );
        }
    }
    assert!(found > 100, "only {found} marks had a start");
}

#[test]
fn grab_from_standing_starts_on_the_last_standing_frame() {
    // Both stand until the player grabs on frame 36
    let frames = reframed::pikachu_joker().frames;
    assert_eq!(frames[35].player.status_kind, status::WAIT);
    assert_ne!(frames[36].player.status_kind, status::WAIT);
    assert_eq!(find_start(&frames, 36, WINDOW, &RESTORABLE), Some(35));
    assert_eq!(find_start(&frames, 20, WINDOW, &RESTORABLE), Some(20));
}

#[test]
fn combo_after_a_reload_into_shield_has_no_clean_start() {
    // After each reload the player starts in shield startup and goes straight into
    // a grab: there is no frame a save state could bring back, and the frames before
    // are on the far side of the reload. Better no start than one that desyncs.
    let frames = reframed::pikachu_joker().frames;
    assert_eq!(frames[44].player.status_kind, status::GUARD_ON);
    for mark in [50, 100, 150, 200] {
        assert_eq!(
            find_start(&frames, mark, WINDOW, &RESTORABLE),
            None,
            "mark {mark}"
        );
    }
}

#[test]
fn falling_player_and_standing_cpu_is_a_start() {
    // After the respawn around frame 880 the player falls while Joker stands
    let frames = reframed::pikachu_joker().frames;
    let start = find_start(&frames, 939, WINDOW, &RESTORABLE).expect("a start");
    assert_eq!(frames[start].player.status_kind, status::FALL);
    assert_eq!(frames[start].cpu.status_kind, status::WAIT);
    assert_eq!(start, 938);
}
