use training_mod_replay::*;

fn fighter(x: f32, y: f32, damage: f32, status: i32) -> FighterSnapshot {
    FighterSnapshot {
        pos_x: x,
        pos_y: y,
        damage,
        status_kind: status,
    }
}

const BASE: FighterSnapshot = FighterSnapshot {
    pos_x: 10.0,
    pos_y: 5.0,
    damage: 30.0,
    status_kind: 3,
};

#[test]
fn identical_fighter_does_not_diverge() {
    assert!(!BASE.diverges_from(&BASE));
}

#[test]
fn position_within_tolerance_does_not_diverge() {
    assert!(!fighter(10.005, 5.0, 30.0, 3).diverges_from(&BASE));
    assert!(!fighter(10.0, 4.995, 30.0, 3).diverges_from(&BASE));
}

#[test]
fn position_beyond_tolerance_diverges() {
    assert!(fighter(10.02, 5.0, 30.0, 3).diverges_from(&BASE));
    assert!(fighter(9.98, 5.0, 30.0, 3).diverges_from(&BASE));
    assert!(fighter(10.0, 5.02, 30.0, 3).diverges_from(&BASE));
}

#[test]
fn damage_beyond_tolerance_diverges() {
    assert!(!fighter(10.0, 5.0, 30.005, 3).diverges_from(&BASE));
    assert!(fighter(10.0, 5.0, 31.0, 3).diverges_from(&BASE));
}

#[test]
fn different_status_diverges() {
    assert!(fighter(10.0, 5.0, 30.0, 4).diverges_from(&BASE));
}

#[test]
fn nan_always_diverges() {
    assert!(fighter(f32::NAN, 5.0, 30.0, 3).diverges_from(&BASE));
    assert!(BASE.diverges_from(&fighter(10.0, f32::NAN, 30.0, 3)));
    assert!(fighter(10.0, 5.0, f32::NAN, 3).diverges_from(&BASE));
}

#[test]
fn match_diverges_if_either_fighter_diverges() {
    let ok = MatchSnapshot {
        player: BASE,
        cpu: BASE,
    };
    assert!(!ok.diverges_from(&ok));
    let player_off = MatchSnapshot {
        player: fighter(11.0, 5.0, 30.0, 3),
        cpu: BASE,
    };
    let cpu_off = MatchSnapshot {
        player: BASE,
        cpu: fighter(10.0, 5.0, 30.0, 9),
    };
    assert!(player_off.diverges_from(&ok));
    assert!(cpu_off.diverges_from(&ok));
}

#[test]
fn match_mirror_flips_only_horizontal_positions() {
    let m = MatchSnapshot {
        player: BASE,
        cpu: fighter(-3.0, 1.0, 12.0, 7),
    };
    let mirrored = m.mirrored();
    assert_eq!(mirrored.player, fighter(-10.0, 5.0, 30.0, 3));
    assert_eq!(mirrored.cpu, fighter(3.0, 1.0, 12.0, 7));
    assert_eq!(mirrored.mirrored(), m);
}

#[test]
fn cpu_frame_mirror_flips_only_horizontal_sticks() {
    let f = CpuFrame {
        buttons: 0x15,
        stick_x: 0.5,
        stick_y: -0.25,
        clamped_lstick_x: 0.4,
        clamped_lstick_y: -0.2,
        clamped_rstick_x: -1.0,
        clamped_rstick_y: 0.3,
        command_flags: [1, 2, 3, 4],
    };
    let m = f.mirrored();
    assert_eq!(
        m,
        CpuFrame {
            stick_x: -0.5,
            clamped_lstick_x: -0.4,
            clamped_rstick_x: 1.0,
            ..f
        }
    );
    assert_eq!(m.mirrored(), f);
}
