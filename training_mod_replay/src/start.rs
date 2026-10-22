//! Picking where a replay captured outside training mode can start in training mode
//!
//! A save state only brings back what it stores, not a move that is halfway done.
//! So an imported replay starts on a frame where both fighters are in a state that
//! can be restored exactly (standing, walking, shielding, falling, ...), and the
//! stretch from there to the frame the player picked must not cross a respawn.

use std::collections::VecDeque;

use crate::frames::within;
use crate::Mirror;

/// Farthest a fighter moves in one frame without teleporting, in game units.
/// Measured on a real match: the fastest movement was 11 (Pikachu's up special),
/// respawns and save state loads jumped at least 233.
pub const MAX_STEP: f32 = 20.0;

/// Everything restored when an imported replay starts
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct FighterState {
    pub pos_x: f32,
    pub pos_y: f32,
    pub speed_x: f32,
    pub speed_y: f32,
    pub lr: f32,
    pub damage: f32,
    pub shield: f32,
    pub hitstun: f32,
    pub status_kind: i32,
    pub situation_kind: i32,
    pub jumps_used: i32,
}

impl FighterState {
    /// In a state from `statuses` (the ones a save state brings back exactly)
    /// and free to act
    pub fn is_restorable(&self, statuses: &[i32]) -> bool {
        statuses.contains(&self.status_kind)
            && self.hitstun <= 0.0
            && [self.pos_x, self.pos_y, self.speed_x, self.speed_y]
                .iter()
                .all(|v| v.is_finite())
    }

    /// Moved from `prev` without teleporting (respawn, save state load)
    pub fn continues(&self, prev: &Self) -> bool {
        within(self.pos_x, prev.pos_x, MAX_STEP) && within(self.pos_y, prev.pos_y, MAX_STEP)
    }
}

impl Mirror for FighterState {
    fn mirrored(&self) -> Self {
        FighterState {
            pos_x: -self.pos_x,
            speed_x: -self.speed_x,
            lr: -self.lr,
            ..*self
        }
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct MatchState {
    pub player: FighterState,
    pub cpu: FighterState,
}

impl MatchState {
    pub fn is_restorable(&self, statuses: &[i32]) -> bool {
        self.player.is_restorable(statuses) && self.cpu.is_restorable(statuses)
    }

    pub fn continues(&self, prev: &Self) -> bool {
        self.player.continues(&prev.player) && self.cpu.continues(&prev.cpu)
    }
}

impl Mirror for MatchState {
    fn mirrored(&self) -> Self {
        MatchState {
            player: self.player.mirrored(),
            cpu: self.cpu.mirrored(),
        }
    }
}

/// The latest frame at or before `mark`, at most `window` frames back, where both
/// fighters can be restored and nothing teleports between it and `mark`.
/// The first frame of `history` is never picked: without the frame before it
/// there is no telling whether it was just teleported to.
pub fn find_start(
    history: &VecDeque<MatchState>,
    mark: usize,
    window: usize,
    statuses: &[i32],
) -> Option<usize> {
    if mark >= history.len() {
        return None;
    }
    let earliest = mark.saturating_sub(window).max(1);
    // Walk back from the mark; a teleport on the way cuts off everything before it
    let mut i = mark;
    while i >= earliest {
        if !history[i].continues(&history[i - 1]) {
            return None;
        }
        if history[i].is_restorable(statuses) {
            return Some(i);
        }
        i -= 1;
    }
    None
}
