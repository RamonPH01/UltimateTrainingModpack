mod common;
use common::*;

fn slots() -> TestReplay {
    Replay::with_slots(60, 5)
}

fn play_slot(r: &mut TestReplay, slot: usize) -> Vec<Pad> {
    assert!(r.set_play_slot(slot));
    assert_eq!(r.request(ReplayMode::Play), (true, None));
    r.on_state_loaded(1.0);
    let mut out = Vec::new();
    while r.state() == ReplayState::Playing {
        out.push(frame(r, NEUTRAL, cpu(0)).0);
    }
    out
}

#[test]
fn has_requested_slot_count_and_at_least_one() {
    assert_eq!(slots().slot_count(), 5);
    let r: TestReplay = Replay::with_slots(60, 0);
    assert_eq!(r.slot_count(), 1);
    let r: TestReplay = Replay::new(60);
    assert_eq!(r.slot_count(), 1);
}

#[test]
fn slots_are_independent() {
    let mut r = slots();
    record_into(&mut r, 0, 3, 10);
    record_into(&mut r, 2, 2, 50);
    assert_eq!(r.slot_len(0), 3);
    assert_eq!(r.slot_len(1), 0);
    assert_eq!(r.slot_len(2), 2);
    assert_eq!(play_slot(&mut r, 0), vec![pad(10), pad(11), pad(12)]);
    assert_eq!(play_slot(&mut r, 2), vec![pad(50), pad(51)]);
}

#[test]
fn cpu_track_comes_from_played_slot() {
    let mut r = slots();
    record_into(&mut r, 0, 1, 10);
    record_into(&mut r, 1, 1, 70);
    r.set_play_slot(1);
    r.request(ReplayMode::Play);
    r.on_state_loaded(1.0);
    assert_eq!(frame(&mut r, NEUTRAL, cpu(0)).1, cpu(70));
}

#[test]
fn rerecording_one_slot_keeps_the_others() {
    let mut r = slots();
    record_into(&mut r, 0, 3, 10);
    record_into(&mut r, 1, 3, 20);
    record_into(&mut r, 1, 1, 30);
    assert_eq!(play_slot(&mut r, 0), vec![pad(10), pad(11), pad(12)]);
    assert_eq!(play_slot(&mut r, 1), vec![pad(30)]);
}

#[test]
fn playing_an_empty_slot_is_rejected() {
    let mut r = slots();
    record_into(&mut r, 0, 3, 10);
    assert!(r.set_play_slot(3));
    assert_eq!(
        r.request(ReplayMode::Play),
        (false, Some(ReplayEvent::Rejected(RejectReason::EmptySlot)))
    );
    assert_eq!(r.state(), ReplayState::Idle);
}

#[test]
fn out_of_range_slots_are_refused() {
    let mut r = slots();
    assert!(!r.set_record_slot(5));
    assert!(!r.set_play_slot(99));
    assert_eq!(r.slot_len(99), 0);
}

#[test]
fn slot_selection_is_refused_while_active() {
    let mut r = slots();
    record_into(&mut r, 0, 3, 10);
    r.set_play_slot(0);
    r.request(ReplayMode::Play);
    assert!(!r.set_play_slot(1), "arming");
    assert!(!r.set_record_slot(1), "arming");
    r.on_state_loaded(1.0);
    assert!(!r.set_play_slot(1), "playing");
    assert_eq!(r.slot(), 0);
}

#[test]
fn len_reports_the_current_slot() {
    let mut r = slots();
    record_into(&mut r, 0, 3, 10);
    record_into(&mut r, 1, 5, 10);
    assert_eq!(r.slot(), 1);
    assert_eq!(r.len(), 5);
    play_slot(&mut r, 0);
    assert_eq!(r.slot(), 0);
    assert_eq!(r.len(), 3);
}

#[test]
fn cancelled_recording_keeps_partial_frames_in_its_slot() {
    let mut r = slots();
    r.set_record_slot(4);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    frame(&mut r, pad(1), cpu(1));
    r.cancel();
    assert_eq!(r.slot_len(4), 1);
    assert_eq!(r.slot_len(0), 0);
}

// --- Context: fighters and stage ---

#[test]
fn playback_with_other_fighters_is_rejected() {
    let mut r = slots();
    r.set_context([1, 2, 3]);
    record_into(&mut r, 0, 3, 10);
    r.set_context([1, 9, 3]);
    assert!(r.set_play_slot(0));
    assert_eq!(
        r.request(ReplayMode::Play),
        (
            false,
            Some(ReplayEvent::Rejected(RejectReason::OtherFighters))
        )
    );
    assert_eq!(r.state(), ReplayState::Idle);
}

#[test]
fn playback_with_same_context_is_allowed() {
    let mut r = slots();
    r.set_context([1, 2, 3]);
    record_into(&mut r, 0, 3, 10);
    r.set_context([1, 2, 3]);
    assert_eq!(play_slot(&mut r, 0).len(), 3);
}

#[test]
fn context_is_taken_when_recording_starts() {
    let mut r = slots();
    r.set_context([1, 2, 3]);
    r.request(ReplayMode::Record);
    // Changing context mid-recording must not relabel the recording
    r.set_context([7, 7, 7]);
    r.on_state_loaded(1.0);
    frame(&mut r, pad(1), cpu(1));
    r.request(ReplayMode::Record);
    r.set_context([1, 2, 3]);
    assert_eq!(play_slot(&mut r, 0).len(), 1);
}

#[test]
fn empty_slot_is_reported_before_context() {
    let mut r = slots();
    r.set_context([4, 4, 4]);
    r.set_play_slot(2);
    assert_eq!(
        r.request(ReplayMode::Play).1,
        Some(ReplayEvent::Rejected(RejectReason::EmptySlot))
    );
}

#[test]
fn mirror_is_stored_per_slot() {
    let mut r = slots();
    r.set_record_slot(0);
    r.request(ReplayMode::Record);
    r.on_state_loaded(-1.0);
    frame(&mut r, pad(5), cpu(0));
    r.request(ReplayMode::Record);
    record_into(&mut r, 1, 1, 5);
    // Both played with mirror -1: slot 0 was recorded mirrored, slot 1 not
    r.set_play_slot(0);
    r.request(ReplayMode::Play);
    r.on_state_loaded(-1.0);
    assert_eq!(frame(&mut r, NEUTRAL, cpu(0)).0, pad(5));
    r.set_play_slot(1);
    r.request(ReplayMode::Play);
    r.on_state_loaded(-1.0);
    assert_eq!(frame(&mut r, NEUTRAL, cpu(0)).0, pad(-5));
}

#[test]
fn selected_slots_are_readable_for_messages() {
    let mut r = slots();
    assert_eq!((r.record_slot(), r.play_slot()), (0, 0));
    r.set_record_slot(3);
    r.set_play_slot(4);
    assert_eq!((r.record_slot(), r.play_slot()), (3, 4));
    // A rejected play request reports the requested slot, not the last used one
    assert_eq!(
        r.request(ReplayMode::Play).1,
        Some(ReplayEvent::Rejected(RejectReason::EmptySlot))
    );
    assert_eq!(r.play_slot(), 4);
}
