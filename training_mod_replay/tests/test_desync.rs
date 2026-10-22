mod common;
use common::*;

fn snap(x: f32) -> Snap {
    Snap { x }
}

/// The recording's game state is x = frame (see common::frame)
fn play_frame(r: &mut TestReplay, live_x: f32) -> Vec<ReplayEvent> {
    frame_with(r, NEUTRAL, cpu(0), snap(live_x)).2
}

#[test]
fn matching_playback_reports_no_desync() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    for i in 0..10 {
        let events = play_frame(&mut r, i as f32);
        assert!(!events
            .iter()
            .any(|e| matches!(e, ReplayEvent::Desync { .. })));
    }
    assert_eq!(r.desync_frame(), None);
}

#[test]
fn reports_exact_first_diverging_frame() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    for i in 0..3 {
        assert!(play_frame(&mut r, i as f32).is_empty());
    }
    assert_eq!(
        play_frame(&mut r, 99.0),
        vec![ReplayEvent::Desync { frame: 3 }]
    );
    assert_eq!(r.desync_frame(), Some(3));
}

#[test]
fn desync_is_reported_only_once_per_playback() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    play_frame(&mut r, 99.0);
    for _ in 1..9 {
        assert!(play_frame(&mut r, 99.0).is_empty());
    }
    assert_eq!(r.desync_frame(), Some(0));
}

#[test]
fn desync_on_frame_zero_is_detected() {
    let mut r = recorded(5);
    start_playback(&mut r, 1.0);
    assert_eq!(
        play_frame(&mut r, 0.5),
        vec![ReplayEvent::Desync { frame: 0 }]
    );
}

#[test]
fn desync_does_not_stop_playback() {
    let mut r = recorded(5);
    start_playback(&mut r, 1.0);
    play_frame(&mut r, 99.0);
    assert_eq!(r.state(), ReplayState::Playing);
    let (p1, c, _) = frame_with(&mut r, NEUTRAL, cpu(0), snap(99.0));
    assert_eq!((p1, c), (pad(2), cpu(101)));
}

#[test]
fn no_desync_after_takeover() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    play_frame(&mut r, 0.0);
    frame_with(&mut r, press(), cpu(0), snap(1.0));
    assert_eq!(r.state(), ReplayState::TakenOver);
    // The player now plays differently, so the state is expected to diverge
    for _ in 2..10 {
        assert!(!frame_with(&mut r, press(), cpu(0), snap(99.0))
            .2
            .iter()
            .any(|e| matches!(e, ReplayEvent::Desync { .. })));
    }
    assert_eq!(r.desync_frame(), None);
}

#[test]
fn mirrored_playback_compares_against_mirrored_recording() {
    let mut r = recorded(5);
    start_playback(&mut r, -1.0);
    for i in 0..5 {
        let events = play_frame(&mut r, -(i as f32));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ReplayEvent::Desync { .. })),
            "frame {i}"
        );
    }
    assert_eq!(r.desync_frame(), None);
}

#[test]
fn mirrored_playback_with_unmirrored_state_desyncs() {
    let mut r = recorded(5);
    start_playback(&mut r, -1.0);
    play_frame(&mut r, 0.0); // 0 mirrored is 0
    assert_eq!(
        play_frame(&mut r, 1.0),
        vec![ReplayEvent::Desync { frame: 1 }]
    );
}

#[test]
fn new_playback_resets_desync() {
    let mut r = recorded(5);
    start_playback(&mut r, 1.0);
    play_frame(&mut r, 99.0);
    assert_eq!(r.desync_frame(), Some(0));
    start_playback(&mut r, 1.0);
    assert_eq!(r.desync_frame(), None);
    assert!(play_frame(&mut r, 0.0).is_empty());
}

#[test]
fn snapshots_are_only_written_while_recording() {
    let mut r = recorded(5);
    assert!(r.snapshot_mut().is_none());
    start_playback(&mut r, 1.0);
    assert!(r.snapshot_mut().is_none());
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    assert!(r.snapshot_mut().is_some());
}

#[test]
fn check_outside_playback_does_nothing() {
    let mut r = recorded(5);
    assert_eq!(r.check_snapshot(&snap(99.0)), None);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    assert_eq!(r.check_snapshot(&snap(99.0)), None);
}

#[test]
fn rerecording_replaces_snapshots() {
    let mut r = recorded(5);
    r.request(ReplayMode::Record);
    r.on_state_loaded(1.0);
    frame_with(&mut r, pad(1), cpu(1), snap(42.0));
    r.request(ReplayMode::Record);
    start_playback(&mut r, 1.0);
    assert_eq!(
        play_frame(&mut r, 42.0),
        vec![ReplayEvent::PlaybackFinished]
    );
    assert_eq!(r.desync_frame(), None);
}
