mod common;
use common::*;

fn cpu_side(n: usize) -> TestReplay {
    let mut r = recorded(n);
    assert!(r.set_takeover_side(TakeoverSide::Cpu));
    r
}

/// Runs a frame like the plugin: the CPU hook uses the player's input after a CPU takeover
fn frame_cpu(
    r: &mut TestReplay,
    live_p1: Pad,
    live_ai: Cpu,
) -> (Pad, Option<Pad>, Cpu, Vec<ReplayEvent>) {
    let mut events = Vec::new();
    let snap = Snap {
        x: r.frame() as f32,
    };
    if r.snapshot_mut().is_none() {
        events.extend(r.check_snapshot(&snap));
    }
    let (p1, ev) = r.on_player_input(live_p1);
    events.extend(ev);
    let routed = r.cpu_takeover_input();
    let cpu_out = r.cpu_playback().unwrap_or(live_ai);
    events.extend(r.advance());
    (p1, routed, cpu_out, events)
}

#[test]
fn default_side_is_player() {
    let r = recorded(3);
    assert_eq!(r.takeover_side(), TakeoverSide::Player);
}

#[test]
fn side_can_be_changed_while_idle() {
    let mut r = recorded(3);
    assert!(r.set_takeover_side(TakeoverSide::Cpu));
    assert_eq!(r.takeover_side(), TakeoverSide::Cpu);
    assert!(r.set_takeover_side(TakeoverSide::Player));
    assert_eq!(r.takeover_side(), TakeoverSide::Player);
}

#[test]
fn side_change_is_refused_while_active() {
    let mut r = recorded(3);
    start_playback(&mut r, 1.0);
    assert!(!r.set_takeover_side(TakeoverSide::Cpu));
    assert_eq!(r.takeover_side(), TakeoverSide::Player);
}

#[test]
fn before_takeover_both_tracks_play() {
    let mut r = cpu_side(5);
    start_playback(&mut r, 1.0);
    let (p1, routed, c, events) = frame_cpu(&mut r, NEUTRAL, cpu(-1));
    assert_eq!((p1, routed, c), (pad(1), None, cpu(100)));
    assert!(events.is_empty());
    assert!(!r.player_controls_cpu());
}

#[test]
fn takeover_routes_live_input_to_cpu_on_the_same_frame() {
    let mut r = cpu_side(10);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    let (p1, routed, _, events) = frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!(events, vec![ReplayEvent::TakenOver { frame: 1 }]);
    assert_eq!(p1, pad(2), "player 1 keeps following its track");
    assert_eq!(
        routed,
        Some(press()),
        "CPU gets the input that triggered the takeover"
    );
    assert_eq!(r.state(), ReplayState::TakenOver);
    assert!(r.player_controls_cpu());
    assert!(!r.drives_cpu());
}

#[test]
fn after_takeover_cpu_follows_the_controller_including_neutral() {
    let mut r = cpu_side(10);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    frame_cpu(&mut r, press(), cpu(-1));
    for i in 2..6 {
        let (p1, routed, c, _) = frame_cpu(&mut r, NEUTRAL, cpu(-1));
        assert_eq!(p1, pad(i + 1));
        assert_eq!(routed, Some(NEUTRAL));
        assert_eq!(c, cpu(-1), "no recorded CPU inputs after takeover");
    }
    let (_, routed, _, _) = frame_cpu(&mut r, pad(-40), cpu(-1));
    assert_eq!(routed, Some(pad(-40)));
}

#[test]
fn held_combo_does_not_take_over_the_cpu() {
    let mut r = cpu_side(10);
    start_playback(&mut r, 1.0);
    let (_, routed, c, events) = frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!((routed, c), (None, cpu(100)));
    assert!(events.is_empty());
}

#[test]
fn track_end_hands_everything_back() {
    let mut r = cpu_side(3);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    frame_cpu(&mut r, press(), cpu(-1));
    let (_, _, _, events) = frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!(events, vec![ReplayEvent::PlaybackFinished]);
    assert_eq!(r.state(), ReplayState::Idle);
    assert!(!r.player_controls_cpu());
    assert_eq!(r.cpu_takeover_input(), None);
    let (p1, routed, c, _) = frame_cpu(&mut r, pad(9), cpu(-1));
    assert_eq!((p1, routed, c), (pad(9), None, cpu(-1)));
}

#[test]
fn cancel_hands_everything_back() {
    let mut r = cpu_side(10);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    frame_cpu(&mut r, press(), cpu(-1));
    r.cancel();
    assert!(!r.player_controls_cpu());
    assert_eq!(r.cpu_takeover_input(), None);
}

#[test]
fn restart_after_cpu_takeover_plays_both_tracks_again() {
    let mut r = cpu_side(10);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    frame_cpu(&mut r, press(), cpu(-1));
    start_playback(&mut r, 1.0);
    assert!(!r.player_controls_cpu());
    let (p1, routed, c, _) = frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!((p1, routed, c), (pad(1), None, cpu(100)));
}

#[test]
fn mirrored_cpu_takeover_keeps_controller_input_unmirrored() {
    let mut r = cpu_side(10);
    start_playback(&mut r, -1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    let (p1, routed, _, _) = frame_cpu(&mut r, pad(30), cpu(-1));
    assert_eq!(p1, pad(-2), "recorded track is mirrored");
    assert_eq!(routed, Some(pad(30)), "live stick is not");
}

#[test]
fn no_desync_after_cpu_takeover() {
    let mut r = cpu_side(10);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!(r.check_snapshot(&Snap { x: 999.0 }), None);
}

#[test]
fn player_side_never_routes_to_cpu() {
    let mut r = recorded(10);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, NEUTRAL, cpu(-1));
    let (p1, routed, c, _) = frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!((p1, routed, c), (press(), None, cpu(101)));
    assert!(!r.player_controls_cpu());
}

#[test]
fn combo_only_trigger_takes_over_the_cpu() {
    let mut r = cpu_side(10);
    r.set_takeover_on_input(false);
    start_playback(&mut r, 1.0);
    frame_cpu(&mut r, press(), cpu(-1));
    assert_eq!(r.take_over(), Some(ReplayEvent::TakenOver { frame: 1 }));
    let (p1, routed, _, _) = frame_cpu(&mut r, pad(7), cpu(-1));
    assert_eq!((p1, routed), (pad(2), Some(pad(7))));
}
