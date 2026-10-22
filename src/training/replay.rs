// Replay Takeover: record both fighters from a save state, play them back, and take over
// player 1 on any frame while the CPU keeps following its recorded track.
// The state machine lives in training_mod_replay so it can be unit tested on the host;
// this module only connects it to the game hooks.

use std::ptr;

use skyline::nn::ui2d::ResColor;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use training_mod_replay::{Mirror, Replay, ReplayEvent, ReplayInput, ReplayMode, ReplayState};
use training_mod_sync::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::input::*;
use crate::common::{button_config, try_get_module_accessor, MENU};
use crate::training::input_record;
use crate::training::save_states;
use crate::training::ui::notifications::{clear_notification, color_notification, notification};

const REPLAY_CAPACITY: usize = 600; // Matches the longest RecordingDuration option
const COMMAND_CATEGORY_COUNT: usize = 4;
const NOTIFICATION_HEADER: &str = "Replay";

/// Everything the CPU decides on in one frame: the controller state written by the AI
/// (or by input playback) and the command flags, which is where mash options act
#[derive(Copy, Clone, Default)]
pub struct CpuFrame {
    buttons: Buttons,
    stick_x: f32,
    stick_y: f32,
    clamped_lstick_x: f32,
    clamped_lstick_y: f32,
    clamped_rstick_x: f32,
    clamped_rstick_y: f32,
    command_flags: [i32; COMMAND_CATEGORY_COUNT],
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

impl Mirror for MappedInputs {
    fn mirrored(&self) -> Self {
        MappedInputs {
            lstick_x: self.lstick_x.saturating_neg(),
            rstick_x: self.rstick_x.saturating_neg(),
            ..*self
        }
    }
}

impl ReplayInput for MappedInputs {
    fn is_neutral(&self) -> bool {
        input_record::is_mapped_input_neutral(self)
    }
}

static REPLAY: LazyLock<RwLock<Replay<MappedInputs, CpuFrame>>> =
    LazyLock::new(|| RwLock::new(Replay::new(REPLAY_CAPACITY)));

pub fn is_active() -> bool {
    lock_read(&REPLAY).is_active()
}

fn is_cpu(module_accessor: &BattleObjectModuleAccessor) -> bool {
    try_get_module_accessor(FighterId::CPU)
        .is_some_and(|cpu| ptr::eq(cpu as *const _, module_accessor as *const _))
}

/// Once per frame, from the CPU's frame handler. Starts recording or playback.
pub unsafe fn handle_frame() {
    let mut replay = lock_write(&REPLAY);

    // The load was requested on an earlier frame, so is_loading() already went true
    if matches!(replay.state(), ReplayState::Arming(_)) && !save_states::is_loading() {
        let event = replay.on_state_loaded(save_states::mirror_state());
        drop(replay);
        show(event);
        return;
    }

    let mode = if button_config::combo_passes(button_config::ButtonCombo::ReplayRecord) {
        ReplayMode::Record
    } else if button_config::combo_passes(button_config::ButtonCombo::ReplayPlay) {
        ReplayMode::Play
    } else {
        return;
    };

    if read(&MENU).save_state_enable == OnOff::OFF {
        notification(
            NOTIFICATION_HEADER.to_string(),
            "Enable Save States first".to_string(),
            120,
        );
        return;
    }
    if !input_record::is_idle() || save_states::is_loading() {
        return;
    }

    replay.set_max_frames(read(&MENU).replay_duration.into_frames());
    replay.set_takeover_on_input(read(&MENU).replay_takeover_on_input == OnOff::ON);
    let (should_load, event) = replay.request(mode);
    drop(replay);
    if should_load {
        save_states::request_load();
    } else if mode == ReplayMode::Play && event.is_none() && lock_read(&REPLAY).is_empty() {
        notification(
            NOTIFICATION_HEADER.to_string(),
            "Nothing recorded yet".to_string(),
            120,
        );
    }
    show(event);
}

/// Manual save state load, death autoload or anything else that resets the situation
pub fn cancel() {
    let event = lock_write(&REPLAY).cancel();
    show(event);
}

/// Player 1's final inputs. Runs last in the input mapping hook.
pub unsafe fn handle_final_input_mapping(player_idx: i32, out: *mut MappedInputs) {
    if player_idx != 0 {
        return;
    }
    let mut replay = lock_write(&REPLAY);
    if !replay.is_active() {
        return;
    }
    let (inputs, event) = replay.on_player_input(*out);
    drop(replay);
    *out = inputs;
    show(event);
}

/// CPU controller data after the AI and input playback wrote it. Ends the replay frame.
pub unsafe fn handle_cpu_controls(controller_data: *mut ControlModuleInternal) {
    let mut replay = lock_write(&REPLAY);
    if !replay.is_active() {
        return;
    }
    let data = &mut *controller_data;
    if let Some(frame) = replay.cpu_frame_mut() {
        frame.buttons = data.buttons;
        frame.stick_x = data.stick_x;
        frame.stick_y = data.stick_y;
        frame.clamped_lstick_x = data.clamped_lstick_x;
        frame.clamped_lstick_y = data.clamped_lstick_y;
        frame.clamped_rstick_x = data.clamped_rstick_x;
        frame.clamped_rstick_y = data.clamped_rstick_y;
    } else if let Some(frame) = replay.cpu_playback() {
        data.buttons = frame.buttons;
        data.stick_x = frame.stick_x;
        data.stick_y = frame.stick_y;
        data.clamped_lstick_x = frame.clamped_lstick_x;
        data.clamped_lstick_y = frame.clamped_lstick_y;
        data.clamped_rstick_x = frame.clamped_rstick_x;
        data.clamped_rstick_y = frame.clamped_rstick_y;
    }
    let event = replay.advance();
    drop(replay);
    show(event);
}

/// Command flags of the CPU, after mash and throw options were applied.
/// Recorded so that mash-driven actions replay exactly, and replayed instead of new rolls.
pub fn handle_command_flag_cat(
    module_accessor: &BattleObjectModuleAccessor,
    category: i32,
    flag: i32,
) -> i32 {
    let Some(idx) = [
        FIGHTER_PAD_COMMAND_CATEGORY1,
        FIGHTER_PAD_COMMAND_CATEGORY2,
        FIGHTER_PAD_COMMAND_CATEGORY3,
        FIGHTER_PAD_COMMAND_CATEGORY4,
    ]
    .iter()
    .position(|c| *c == category) else {
        return flag;
    };
    if !is_cpu(module_accessor) {
        return flag;
    }
    let mut replay = lock_write(&REPLAY);
    if let Some(frame) = replay.cpu_frame_mut() {
        frame.command_flags[idx] = flag;
        flag
    } else if let Some(frame) = replay.cpu_playback() {
        frame.command_flags[idx]
    } else {
        flag
    }
}

fn show(event: Option<ReplayEvent>) {
    let Some(event) = event else {
        return;
    };
    clear_notification(NOTIFICATION_HEADER);
    let (message, length, color) = match event {
        ReplayEvent::RecordingStarted => (
            "Recording".to_string(),
            read(&MENU).replay_duration.into_frames() as u32,
            ResColor {
                r: 200,
                g: 8,
                b: 8,
                a: 255,
            },
        ),
        ReplayEvent::RecordingFinished { len } => (
            format!("Recorded {len} frames"),
            120,
            ResColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        ReplayEvent::PlaybackStarted => (
            "Playback - press anything to take over".to_string(),
            lock_read(&REPLAY).len() as u32,
            ResColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        ReplayEvent::TakenOver { frame } => (
            format!("Takeover on frame {frame}"),
            120,
            ResColor {
                r: 8,
                g: 140,
                b: 8,
                a: 255,
            },
        ),
        ReplayEvent::PlaybackFinished => (
            "Playback finished".to_string(),
            60,
            ResColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        ReplayEvent::Cancelled => (
            "Stopped".to_string(),
            60,
            ResColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
    };
    color_notification(NOTIFICATION_HEADER.to_string(), message, length, color);
}
