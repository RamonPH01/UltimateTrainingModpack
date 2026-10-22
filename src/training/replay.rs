// Replay Takeover: record both fighters from a save state, play them back, and take over
// either fighter on any frame while the other one keeps following its recorded track.
// The state machine lives in training_mod_replay so it can be unit tested on the host;
// this module only connects it to the game hooks.
//
// Performance: every hook first checks an atomic flag, so nothing is locked or computed
// while no replay runs. The per-frame path never allocates (see test_no_alloc).

use std::fs;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use log::{info, warn};
use skyline::nn::ui2d::ResColor;
use smash::app::{lua_bind::*, utility, BattleObjectModuleAccessor};
use smash::lib::lua_const::*;

use training_mod_consts::{ReplayTakeoverSide, REPLAY_SLOT_PATHS};
use training_mod_replay::codec::{put_i8, put_u32, take_i8, take_u32};
use training_mod_replay::{
    Codec, Context, CpuFrame, FighterSnapshot, MatchSnapshot, Mirror, RejectReason, Replay,
    ReplayEvent, ReplayInput, ReplayMode, ReplayState, TakeoverSide,
};
use training_mod_sync::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::input::*;
use crate::common::{button_config, filesystem, try_get_module_accessor, MENU};
use crate::training::input_record::{self, STICK_CLAMP_MULTIPLIER, STICK_NEUTRAL};
use crate::training::save_states;
use crate::training::ui::notifications::{clear_notification, color_notification};

const REPLAY_CAPACITY: usize = 600; // Matches the longest RecordingDuration option
const SLOT_COUNT: usize = REPLAY_SLOT_PATHS.len();
const NOTIFICATION_HEADER: &str = "Replay";

const BLACK: ResColor = ResColor {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};
const RED: ResColor = ResColor {
    r: 200,
    g: 8,
    b: 8,
    a: 255,
};
const GREEN: ResColor = ResColor {
    r: 8,
    g: 140,
    b: 8,
    a: 255,
};
const ORANGE: ResColor = ResColor {
    r: 220,
    g: 110,
    b: 0,
    a: 255,
};

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

impl Codec for MappedInputs {
    fn encode(&self, out: &mut Vec<u8>) {
        put_u32(out, self.buttons.bits());
        for v in [self.lstick_x, self.lstick_y, self.rstick_x, self.rstick_y] {
            put_i8(out, v);
        }
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(MappedInputs {
            buttons: Buttons::from_bits_truncate(take_u32(input)?),
            lstick_x: take_i8(input)?,
            lstick_y: take_i8(input)?,
            rstick_x: take_i8(input)?,
            rstick_y: take_i8(input)?,
        })
    }
}

type GameReplay = Replay<MappedInputs, CpuFrame, MatchSnapshot>;

static REPLAY: LazyLock<RwLock<GameReplay>> =
    LazyLock::new(|| RwLock::new(Replay::with_slots(REPLAY_CAPACITY, SLOT_COUNT)));

// Lock-free mirrors of the replay state for the hot paths
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PLAYER_CONTROLS_CPU: AtomicBool = AtomicBool::new(false);

fn sync_flags(replay: &GameReplay) {
    ACTIVE.store(replay.is_active(), Ordering::Relaxed);
    PLAYER_CONTROLS_CPU.store(replay.player_controls_cpu(), Ordering::Relaxed);
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// After a CPU-side takeover the player's controller drives the CPU
pub fn player_controls_cpu() -> bool {
    PLAYER_CONTROLS_CPU.load(Ordering::Relaxed)
}

/// Restores saved slots at boot, off the game thread
pub fn load_from_file() {
    let mut replay = lock_write(&REPLAY);
    for (slot, path) in REPLAY_SLOT_PATHS.iter().enumerate() {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        match replay.decode_slot(slot, &bytes) {
            Ok(()) => info!(
                "Loaded replay slot {} ({} frames)",
                slot + 1,
                replay.slot_len(slot)
            ),
            Err(e) => warn!("Ignoring replay slot file {path}: {e:?}"),
        }
    }
}

fn save_slot(replay: &GameReplay, slot: usize) {
    if let Some(bytes) = replay.encode_slot(slot) {
        filesystem::write_bytes(REPLAY_SLOT_PATHS[slot], bytes);
    }
}

unsafe fn snapshot(module_accessor: *mut BattleObjectModuleAccessor) -> FighterSnapshot {
    FighterSnapshot {
        pos_x: PostureModule::pos_x(module_accessor),
        pos_y: PostureModule::pos_y(module_accessor),
        damage: DamageModule::damage(module_accessor, 0),
        status_kind: StatusModule::status_kind(module_accessor),
    }
}

unsafe fn context(
    player: *mut BattleObjectModuleAccessor,
    cpu: *mut BattleObjectModuleAccessor,
) -> Context {
    [
        utility::get_kind(&mut *player),
        utility::get_kind(&mut *cpu),
        save_states::stage_id(),
    ]
}

fn takeover_side() -> TakeoverSide {
    if read(&MENU).replay_takeover_side == ReplayTakeoverSide::CPU {
        TakeoverSide::Cpu
    } else {
        TakeoverSide::Player
    }
}

/// Once per frame, from the CPU's frame handler: starts recording or playback,
/// records or checks the game state, and handles the button combos.
pub unsafe fn handle_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    let Some(player) = try_get_module_accessor(FighterId::Player) else {
        return;
    };
    let Some(cpu) = try_get_module_accessor(FighterId::CPU) else {
        return;
    };
    // Ice Climbers: only run for the CPU's main fighter, not for Nana
    if !ptr::eq(cpu as *const _, module_accessor as *const _) {
        return;
    }

    let record = button_config::combo_passes(button_config::ButtonCombo::ReplayRecord);
    let play = !record && button_config::combo_passes(button_config::ButtonCombo::ReplayPlay);
    if !record && !play && !is_active() {
        return;
    }

    let mut replay = lock_write(&REPLAY);
    let mut event = None;

    // The load was requested on an earlier frame, so is_loading() already went true
    if matches!(replay.state(), ReplayState::Arming(_)) && !save_states::is_loading() {
        event = replay.on_state_loaded(save_states::mirror_state());
    }

    if replay.snapshot_mut().is_some() || replay.state() == ReplayState::Playing {
        let live = MatchSnapshot {
            player: snapshot(player),
            cpu: snapshot(cpu),
        };
        if let Some(slot) = replay.snapshot_mut() {
            *slot = live;
        } else if let Some(desync) = replay.check_snapshot(&live) {
            event = Some(desync);
        }
    }

    if record || play {
        let mode = if record {
            ReplayMode::Record
        } else {
            ReplayMode::Play
        };
        let busy = !input_record::is_idle() || save_states::is_loading();
        if read(&MENU).save_state_enable == OnOff::OFF {
            notify_text("Enable Save States first", 120, BLACK);
        } else if !busy && prepare_request(&mut replay, mode, player, cpu) {
            let (should_load, request_event) = replay.request(mode);
            event = request_event.or(event);
            if should_load {
                save_states::request_load();
            }
        }
    }

    sync_flags(&replay);
    if let Some(ReplayEvent::RecordingFinished { .. }) = event {
        save_slot(&replay, replay.slot());
    }
    let slot = match event {
        Some(ReplayEvent::Rejected(_)) => replay.play_slot(),
        _ => replay.slot(),
    };
    let len = replay.len();
    drop(replay);
    show(event, slot, len);
}

/// Applies menu settings before a request. Returns false if the request can't be made.
unsafe fn prepare_request(
    replay: &mut GameReplay,
    mode: ReplayMode,
    player: *mut BattleObjectModuleAccessor,
    cpu: *mut BattleObjectModuleAccessor,
) -> bool {
    // A running replay can still be stopped or restarted; settings apply to the next one
    if replay.is_active() {
        return true;
    }
    let menu = read(&MENU);
    replay.set_context(context(player, cpu));
    replay.set_max_frames(menu.replay_duration.into_frames());
    replay.set_takeover_on_input(menu.replay_takeover_on_input == OnOff::ON);
    replay.set_takeover_side(takeover_side());
    match mode {
        ReplayMode::Record => {
            replay.set_record_slot(menu.replay_record_slot.into_idx().unwrap_or(0));
        }
        ReplayMode::Play => {
            let Some(slot) = menu.replay_play_slots.get_random().into_idx() else {
                notify_text("Choose a Replay Play Slot in the menu", 120, BLACK);
                return false;
            };
            replay.set_play_slot(slot);
        }
    }
    true
}

/// Manual save state load, death autoload or anything else that resets the situation
pub fn cancel() {
    if !is_active() {
        return;
    }
    let mut replay = lock_write(&REPLAY);
    let event = replay.cancel();
    sync_flags(&replay);
    if let Some(ReplayEvent::RecordingFinished { .. }) = event {
        save_slot(&replay, replay.slot());
    }
    let (slot, len) = (replay.slot(), replay.len());
    drop(replay);
    show(event, slot, len);
}

/// Player 1's final inputs. Runs last in the input mapping hook.
pub unsafe fn handle_final_input_mapping(player_idx: i32, out: *mut MappedInputs) {
    if player_idx != 0 || !is_active() {
        return;
    }
    let mut replay = lock_write(&REPLAY);
    let (inputs, event) = replay.on_player_input(*out);
    sync_flags(&replay);
    let (slot, len) = (replay.slot(), replay.len());
    drop(replay);
    *out = inputs;
    show(event, slot, len);
}

/// CPU controller data after the AI and input playback wrote it. Ends the replay frame.
pub unsafe fn handle_cpu_controls(controller_data: *mut ControlModuleInternal) {
    if !is_active() {
        return;
    }
    let mut replay = lock_write(&REPLAY);
    let data = &mut *controller_data;
    if let Some(frame) = replay.cpu_frame_mut() {
        frame.buttons = data.buttons.bits();
        frame.stick_x = data.stick_x;
        frame.stick_y = data.stick_y;
        frame.clamped_lstick_x = data.clamped_lstick_x;
        frame.clamped_lstick_y = data.clamped_lstick_y;
        frame.clamped_rstick_x = data.clamped_rstick_x;
        frame.clamped_rstick_y = data.clamped_rstick_y;
    } else if let Some(frame) = replay.cpu_playback() {
        data.buttons = Buttons::from_bits_truncate(frame.buttons);
        data.stick_x = frame.stick_x;
        data.stick_y = frame.stick_y;
        data.clamped_lstick_x = frame.clamped_lstick_x;
        data.clamped_lstick_y = frame.clamped_lstick_y;
        data.clamped_rstick_x = frame.clamped_rstick_x;
        data.clamped_rstick_y = frame.clamped_rstick_y;
    } else if let Some(inputs) = replay.cpu_takeover_input() {
        apply_controller(data, &inputs);
    }
    let event = replay.advance();
    sync_flags(&replay);
    let (slot, len) = (replay.slot(), replay.len());
    drop(replay);
    show(event, slot, len);
}

/// Player inputs onto the CPU's controller, the same conversion input recording uses
fn apply_controller(data: &mut ControlModuleInternal, inputs: &MappedInputs) {
    let clamp = |v: i8| {
        let c = ((v as f32) * STICK_CLAMP_MULTIPLIER).clamp(-1.0, 1.0);
        if c.abs() >= STICK_NEUTRAL {
            c
        } else {
            0.0
        }
    };
    data.buttons = inputs.buttons;
    data.stick_x = (inputs.lstick_x as f32) / (i8::MAX as f32);
    data.stick_y = (inputs.lstick_y as f32) / (i8::MAX as f32);
    data.clamped_lstick_x = clamp(inputs.lstick_x);
    data.clamped_lstick_y = clamp(inputs.lstick_y);
    data.clamped_rstick_x = clamp(inputs.rstick_x);
    data.clamped_rstick_y = clamp(inputs.rstick_y);
}

/// Command flags of the CPU. `original` is the game's own value, `flag` includes mash and
/// throw options. Recorded so mash-driven actions replay exactly; after a CPU-side takeover
/// only the player's own inputs count.
pub fn handle_command_flag_cat(
    module_accessor: &BattleObjectModuleAccessor,
    category: i32,
    original: i32,
    flag: i32,
) -> i32 {
    if !is_active() {
        return flag;
    }
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
    let is_cpu = try_get_module_accessor(FighterId::CPU)
        .is_some_and(|cpu| ptr::eq(cpu as *const _, module_accessor as *const _));
    if !is_cpu {
        return flag;
    }
    if player_controls_cpu() {
        return original;
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

fn notify_text(message: &str, length: u32, color: ResColor) {
    clear_notification(NOTIFICATION_HEADER);
    color_notification(
        NOTIFICATION_HEADER.to_string(),
        message.to_string(),
        length,
        color,
    );
}

/// Only called on events, so formatting here never runs on an ordinary frame
fn show(event: Option<ReplayEvent>, slot: usize, len: usize) {
    let Some(event) = event else {
        return;
    };
    let slot = slot + 1;
    let (message, length, color) = match event {
        ReplayEvent::RecordingStarted => (
            format!("Recording slot {slot}"),
            read(&MENU).replay_duration.into_frames() as u32,
            RED,
        ),
        ReplayEvent::RecordingFinished { len } => {
            (format!("Saved slot {slot}: {len} frames"), 120, BLACK)
        }
        ReplayEvent::PlaybackStarted => {
            let who = if takeover_side() == TakeoverSide::Cpu {
                "the CPU"
            } else {
                "your fighter"
            };
            (
                format!("Slot {slot} - press anything to take over {who}"),
                len as u32,
                BLACK,
            )
        }
        ReplayEvent::TakenOver { frame } => (format!("Takeover on frame {frame}"), 120, GREEN),
        ReplayEvent::Desync { frame } => (format!("Desync on frame {frame}"), 180, ORANGE),
        ReplayEvent::PlaybackFinished => ("Playback finished".to_string(), 60, BLACK),
        ReplayEvent::Cancelled => ("Stopped".to_string(), 60, BLACK),
        ReplayEvent::Rejected(RejectReason::EmptySlot) => {
            (format!("Slot {slot} is empty"), 120, BLACK)
        }
        ReplayEvent::Rejected(RejectReason::OtherFighters) => (
            format!("Slot {slot} was recorded with other fighters or stage"),
            150,
            ORANGE,
        ),
    };
    clear_notification(NOTIFICATION_HEADER);
    color_notification(NOTIFICATION_HEADER.to_string(), message, length, color);
}
