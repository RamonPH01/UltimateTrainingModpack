mod common;
use common::*;

// --- Start and arming ---

#[test]
fn starts_idle_and_empty() {
    let r: TestReplay = Replay::new(60);
    assert_eq!(r.state(), ReplayState::Idle);
    assert!(r.is_empty());
    assert!(!r.drives_cpu());
}

#[test]
fn play_without_recording_is_rejected() {
    let mut r: TestReplay = Replay::new(60);
    assert_eq!(
        r.request(ReplayMode::Play),
        (false, Some(ReplayEvent::Rejected(RejectReason::EmptySlot)))
    );
    assert_eq!(r.state(), ReplayState::Idle);
}

#[test]
fn record_request_waits_for_save_state_load() {
    let mut r: TestReplay = Replay::new(60);
    assert_eq!(r.request(ReplayMode::Record), (true, None));
    assert_eq!(r.state(), ReplayState::Arming(ReplayMode::Record));
    // Frames while the save state is still loading must not be recorded
    let (p1, _, events) = frame(&mut r, press(), cpu(1));
    assert_eq!(p1, press());
    assert!(events.is_empty());
    assert_eq!(r.frame(), 0);
    r.on_state_loaded(1.0);
    assert_eq!(r.state(), ReplayState::Recording);
}

#[test]
fn state_load_without_request_does_nothing() {
    let mut r: TestReplay = Replay::new(60);
    assert_eq!(r.on_state_loaded(1.0), None);
    assert_eq!(r.state(), ReplayState::Idle);
}

#[test]
fn requests_while_arming_are_ignored() {
    let mut r: TestReplay = Replay::new(60);
    r.request(ReplayMode::Record);
    assert_eq!(r.request(ReplayMode::Play), (false, None));
    assert_eq!(r.request(ReplayMode::Record), (false, None));
    assert_eq!(r.state(), ReplayState::Arming(ReplayMode::Record));
}

// --- Recording ---

#[test]
fn recording_keeps_live_control_for_both_fighters() {
    let mut r: TestReplay = Replay::new(60);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    let (p1, c, _) = frame(&mut r, pad(42), cpu(7));
    assert_eq!(p1, pad(42));
    assert_eq!(c, cpu(7));
    assert!(!r.drives_cpu());
}

#[test]
fn recording_stops_at_max_frames() {
    let mut r: TestReplay = Replay::new(5);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    let mut all = Vec::new();
    for _ in 0..5 {
        all.extend(frame(&mut r, pad(1), cpu(1)).2);
    }
    assert_eq!(all, vec![ReplayEvent::RecordingFinished { len: 5 }]);
    assert_eq!(r.state(), ReplayState::Idle);
    assert_eq!(r.len(), 5);
}

#[test]
fn set_max_frames_is_clamped_to_capacity() {
    let mut r: TestReplay = Replay::new(10);
    r.set_max_frames(1000);
    assert_eq!(r.max_frames(), 10);
    r.set_max_frames(3);
    assert_eq!(r.max_frames(), 3);
    r.set_max_frames(0);
    assert_eq!(r.max_frames(), 1);
}

#[test]
fn set_max_frames_is_ignored_while_active() {
    let mut r: TestReplay = Replay::new(10);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    r.set_max_frames(2);
    assert_eq!(r.max_frames(), 10);
}

#[test]
fn new_recording_replaces_the_old_one() {
    let mut r = recorded(10);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    frame(&mut r, pad(99), cpu(99));
    r.request(ReplayMode::Record);
    assert_eq!(r.len(), 1);
    start_playback(&mut r, 1.0);
    let (p1, c, _) = frame(&mut r, NEUTRAL, cpu(0));
    assert_eq!((p1, c), (pad(99), cpu(99)));
}

#[test]
fn cancel_during_recording_keeps_recorded_frames() {
    let mut r: TestReplay = Replay::new(60);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    frame(&mut r, pad(1), cpu(1));
    frame(&mut r, pad(2), cpu(2));
    assert_eq!(r.cancel(), Some(ReplayEvent::RecordingFinished { len: 2 }));
    assert_eq!(r.len(), 2);
}

#[test]
fn play_request_during_recording_is_ignored() {
    let mut r: TestReplay = Replay::new(60);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    assert_eq!(r.request(ReplayMode::Play), (false, None));
    assert_eq!(r.state(), ReplayState::Recording);
}

// --- Playback ---

#[test]
fn playback_reproduces_both_tracks_frame_by_frame() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    assert!(r.drives_cpu());
    for i in 0..10 {
        // Live inputs differ from the recording; playback must win
        let (p1, c, events) = frame(&mut r, NEUTRAL, cpu(-1));
        assert_eq!(p1, pad(i as i8 + 1), "P1 frame {i}");
        assert_eq!(c, cpu(100 + i), "CPU frame {i}");
        if i < 9 {
            assert!(events.is_empty());
        } else {
            assert_eq!(events, vec![ReplayEvent::PlaybackFinished]);
        }
    }
    assert_eq!(r.state(), ReplayState::Idle);
    assert!(!r.drives_cpu());
}

#[test]
fn playback_can_be_repeated() {
    let mut r = recorded(3);
    for _ in 0..2 {
        start_playback(&mut r, 1.0);
        for i in 0..3 {
            let (p1, _, _) = frame(&mut r, NEUTRAL, cpu(0));
            assert_eq!(p1, pad(i + 1));
        }
    }
}

#[test]
fn play_request_during_playback_restarts() {
    let mut r = recorded(5);
    start_playback(&mut r, 1.0);
    frame(&mut r, NEUTRAL, cpu(0));
    frame(&mut r, NEUTRAL, cpu(0));
    start_playback(&mut r, 1.0);
    assert_eq!(r.frame(), 0);
    assert_eq!(frame(&mut r, NEUTRAL, cpu(0)).0, pad(1));
}

#[test]
fn cancel_during_playback_returns_to_idle() {
    let mut r = recorded(5);
    start_playback(&mut r, 1.0);
    frame(&mut r, NEUTRAL, cpu(0));
    assert_eq!(r.cancel(), Some(ReplayEvent::Cancelled));
    assert_eq!(r.state(), ReplayState::Idle);
    assert_eq!(r.len(), 5, "recording survives a cancel");
    let (p1, c, _) = frame(&mut r, pad(7), cpu(7));
    assert_eq!((p1, c), (pad(7), cpu(7)));
}

#[test]
fn cancel_when_idle_does_nothing() {
    let mut r: TestReplay = Replay::new(60);
    assert_eq!(r.cancel(), None);
}

// --- Takeover ---

#[test]
fn any_input_takes_over_player_one_on_that_frame() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    for _ in 0..4 {
        frame(&mut r, NEUTRAL, cpu(0));
    }
    let (p1, c, events) = frame(&mut r, press(), cpu(-1));
    assert_eq!(
        p1,
        press(),
        "live input is used on the takeover frame itself"
    );
    assert_eq!(c, cpu(104), "CPU still follows its track");
    assert_eq!(events, vec![ReplayEvent::TakenOver { frame: 4 }]);
    assert_eq!(r.state(), ReplayState::TakenOver);
    assert_eq!(r.takeover_frame(), Some(4));
}

#[test]
fn after_takeover_player_is_live_and_cpu_finishes_track() {
    let mut r = recorded(6);
    start_playback(&mut r, 1.0);
    frame(&mut r, NEUTRAL, cpu(0));
    frame(&mut r, press(), cpu(0));
    for i in 2..6 {
        // Neutral input after takeover must stay neutral, not fall back to the recording
        let (p1, c, _) = frame(&mut r, NEUTRAL, cpu(-1));
        assert_eq!(p1, NEUTRAL);
        assert_eq!(c, cpu(100 + i));
    }
    assert_eq!(r.state(), ReplayState::Idle);
    // CPU is back to live control
    assert_eq!(frame(&mut r, NEUTRAL, cpu(-1)).1, cpu(-1));
}

#[test]
fn held_combo_after_load_does_not_take_over() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    // Player is still holding the playback combo when the state finishes loading
    let (p1, _, events) = frame(&mut r, press(), cpu(0));
    assert_eq!(p1, pad(1));
    assert!(events.is_empty());
    let (p1, _, _) = frame(&mut r, press(), cpu(0));
    assert_eq!(p1, pad(2));
    assert_eq!(r.state(), ReplayState::Playing);
    // Released once, the next press takes over
    frame(&mut r, NEUTRAL, cpu(0));
    let (_, _, events) = frame(&mut r, press(), cpu(0));
    assert_eq!(events, vec![ReplayEvent::TakenOver { frame: 3 }]);
}

#[test]
fn takeover_rearms_on_every_playback() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    frame(&mut r, NEUTRAL, cpu(0));
    frame(&mut r, press(), cpu(0));
    assert_eq!(r.state(), ReplayState::TakenOver);
    // Restart while holding the button: must not take over immediately
    start_playback(&mut r, 1.0);
    assert_eq!(r.takeover_frame(), None);
    let (p1, _, events) = frame(&mut r, press(), cpu(0));
    assert_eq!(p1, pad(1));
    assert!(events.is_empty());
}

#[test]
fn combo_only_trigger_ignores_inputs() {
    let mut r = recorded(10);
    r.set_takeover_on_input(false);
    start_playback(&mut r, 1.0);
    frame(&mut r, NEUTRAL, cpu(0));
    let (p1, _, events) = frame(&mut r, press(), cpu(0));
    assert_eq!(p1, pad(2));
    assert!(events.is_empty());
    assert_eq!(r.take_over(), Some(ReplayEvent::TakenOver { frame: 2 }));
    assert_eq!(frame(&mut r, press(), cpu(0)).0, press());
}

#[test]
fn take_over_outside_playback_does_nothing() {
    let mut r = recorded(3);
    assert_eq!(r.take_over(), None);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    assert_eq!(r.take_over(), None);
    assert_eq!(r.state(), ReplayState::Recording);
}

#[test]
fn takeover_on_last_frame_still_finishes() {
    let mut r = recorded(2);
    start_playback(&mut r, 1.0);
    frame(&mut r, NEUTRAL, cpu(0));
    let (_, _, events) = frame(&mut r, press(), cpu(0));
    assert_eq!(
        events,
        vec![
            ReplayEvent::TakenOver { frame: 1 },
            ReplayEvent::PlaybackFinished
        ]
    );
    assert_eq!(r.state(), ReplayState::Idle);
}

// --- Mirroring ---

#[test]
fn mirrored_load_flips_both_tracks() {
    let mut r = recorded(3);
    start_playback(&mut r, -1.0);
    let (p1, c, _) = frame(&mut r, NEUTRAL, cpu(0));
    assert_eq!(p1, pad(-1));
    assert_eq!(
        c,
        Cpu {
            stick_x: -10.0,
            cmd: 100
        }
    );
}

#[test]
fn same_mirror_as_recording_does_not_flip() {
    let mut r: TestReplay = Replay::new(60);
    r.request(ReplayMode::Record);
    r.on_state_loaded(-1.0);
    frame(&mut r, pad(5), cpu(5));
    r.request(ReplayMode::Record);
    start_playback(&mut r, -1.0);
    assert_eq!(frame(&mut r, NEUTRAL, cpu(0)).0, pad(5));
}

#[test]
fn mirror_handles_extreme_stick_values() {
    let mut r: TestReplay = Replay::new(60);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    frame(&mut r, pad(i8::MIN), cpu(0));
    r.request(ReplayMode::Record);
    start_playback(&mut r, -1.0);
    assert_eq!(frame(&mut r, NEUTRAL, cpu(0)).0, pad(i8::MAX));
}
