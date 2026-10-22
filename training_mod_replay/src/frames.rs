//! Concrete per-frame data of the plugin, kept here so it can be tested on the host

use crate::{Mirror, Snapshot};

/// Positions may differ by this much before a frame counts as desynced.
/// Identical inputs from an identical state give bit-identical positions,
/// so anything above float noise is a real drift.
pub const POSITION_TOLERANCE: f32 = 0.01;
pub const DAMAGE_TOLERANCE: f32 = 0.01;

/// Everything the CPU decides on in one frame: the controller state written by the AI
/// (or by input playback) and the command flags, which is where mash options act
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct CpuFrame {
    pub buttons: u32,
    pub stick_x: f32,
    pub stick_y: f32,
    pub clamped_lstick_x: f32,
    pub clamped_lstick_y: f32,
    pub clamped_rstick_x: f32,
    pub clamped_rstick_y: f32,
    pub command_flags: [i32; 4],
}

impl Mirror for CpuFrame {
    fn mirrored(&self) -> Self {
        CpuFrame {
            stick_x: -self.stick_x,
            clamped_lstick_x: -self.clamped_lstick_x,
            clamped_rstick_x: -self.clamped_rstick_x,
            ..*self
        }
    }
}

/// NaN never counts as "within tolerance"
pub(crate) fn within(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct FighterSnapshot {
    pub pos_x: f32,
    pub pos_y: f32,
    pub damage: f32,
    pub status_kind: i32,
}

impl FighterSnapshot {
    pub fn diverges_from(&self, recorded: &Self) -> bool {
        self.status_kind != recorded.status_kind
            || !within(self.pos_x, recorded.pos_x, POSITION_TOLERANCE)
            || !within(self.pos_y, recorded.pos_y, POSITION_TOLERANCE)
            || !within(self.damage, recorded.damage, DAMAGE_TOLERANCE)
    }

    fn mirrored(&self) -> Self {
        FighterSnapshot {
            pos_x: -self.pos_x,
            ..*self
        }
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct MatchSnapshot {
    pub player: FighterSnapshot,
    pub cpu: FighterSnapshot,
}

impl Mirror for MatchSnapshot {
    fn mirrored(&self) -> Self {
        MatchSnapshot {
            player: self.player.mirrored(),
            cpu: self.cpu.mirrored(),
        }
    }
}

impl Snapshot for MatchSnapshot {
    fn diverges_from(&self, recorded: &Self) -> bool {
        self.player.diverges_from(&recorded.player) || self.cpu.diverges_from(&recorded.cpu)
    }
}
