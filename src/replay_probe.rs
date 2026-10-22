// Diagnostic build for Replay Takeover from the game's own replays (Vault).
//
// Outside training mode it logs which input hooks fire each frame and what each one sees,
// so a replay can be compared with a normal match. At boot it lists every string in the
// game's read-only data that contains "replay", with the address of exported symbols.
//
// Output: sd:/ultimate/TrainingModpack/replay_probe_symbols.txt
//         sd:/ultimate/TrainingModpack/replay_probe_frames.txt

use std::fmt::Write as _;

use log::info;
use skyline::hooks::{getRegionAddress, Region};
use skyline::nn::ro::LookupSymbol;
use smash::app::{lua_bind::*, BattleObjectModuleAccessor};
use smash::lib::lua_const::*;
use training_mod_sync::*;

use crate::common::filesystem;
use crate::common::input::{ControlModuleInternal, MappedInputs, SomeControllerStruct};
use crate::common::is_training_mode;

const SYMBOLS_PATH: &str = "sd:/ultimate/TrainingModpack/replay_probe_symbols.txt";
const FRAMES_PATH: &str = "sd:/ultimate/TrainingModpack/replay_probe_frames.txt";
const MAX_FRAMES_PER_MATCH: u32 = 20_000; // about 5.5 minutes
const FLUSH_EVERY: u32 = 120; // every 2 seconds, so a match that just ended is mostly on the SD card
const MAX_SYMBOLS: usize = 5_000;
const PLAYERS: usize = 2;

#[derive(Copy, Clone, Default)]
struct FimSample {
    calls: u8,
    raw_buttons: u64,
    raw_lx: f32,
    raw_ly: f32,
    mapped_buttons: u32,
    mapped_lx: i8,
    mapped_ly: i8,
}

#[derive(Copy, Clone, Default)]
struct CpuControlsSample {
    calls: u8,
    controller_index: i32,
    buttons: u32,
    stick_x: f32,
    stick_y: f32,
}

#[derive(Copy, Clone, Default)]
struct FighterSample {
    seen: bool,
    is_cpu: bool,
    button: i32,
    stick_x: f32,
    stick_y: f32,
    status: i32,
    pos_x: f32,
}

#[derive(Copy, Clone, Default)]
struct FrameSample {
    fim: [FimSample; PLAYERS],
    cpu_controls: CpuControlsSample,
    fighters: [FighterSample; PLAYERS],
}

struct Probe {
    frame: u32,
    current: FrameSample,
    text: String,
}

static PROBE: LazyLock<RwLock<Probe>> = LazyLock::new(|| {
    RwLock::new(Probe {
        frame: 0,
        current: FrameSample::default(),
        text: String::with_capacity(64 * 1024),
    })
});

fn active() -> bool {
    unsafe { !is_training_mode() }
}

/// Starts the symbol scan in the background so boot isn't delayed
pub fn init() {
    std::thread::Builder::new()
        .stack_size(0x20000)
        .spawn(scan_symbols)
        .expect("Couldn't create replay probe thread");
}

fn scan_symbols() {
    let (start, end, text_base) = unsafe {
        (
            getRegionAddress(Region::Rodata) as usize,
            getRegionAddress(Region::Data) as usize,
            getRegionAddress(Region::Text) as usize,
        )
    };
    let rodata = unsafe { std::slice::from_raw_parts(start as *const u8, end - start) };
    let mut found: Vec<&[u8]> = Vec::new();
    for needle in [&b"eplay"[..], &b"EPLAY"[..]] {
        for hit in memchr::memmem::find_iter(rodata, needle) {
            let begin = rodata[..hit]
                .iter()
                .rposition(|b| *b == 0)
                .map_or(0, |p| p + 1);
            let Some(len) = rodata[hit..].iter().position(|b| *b == 0) else {
                continue;
            };
            let s = &rodata[begin..hit + len];
            if s.len() <= 400 && s.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
                found.push(s);
            }
        }
    }
    found.sort_unstable();
    found.dedup();

    let mut out = format!(
        "# {} strings containing 'replay' in rodata (text base {:#x})\n",
        found.len(),
        text_base
    );
    for s in found.iter().take(MAX_SYMBOLS) {
        let name = String::from_utf8_lossy(s);
        // Mangled C++ names may be exported symbols with a resolvable address
        if s.starts_with(b"_Z") {
            let mut c_name = s.to_vec();
            c_name.push(0);
            let mut addr: usize = 0;
            unsafe { LookupSymbol(&mut addr, c_name.as_ptr()) };
            if addr != 0 {
                let _ = writeln!(out, "SYM {:#010x} {name}", addr.wrapping_sub(text_base));
                continue;
            }
        }
        let _ = writeln!(out, "STR {name}");
    }
    info!("Replay probe: found {} replay strings", found.len());
    filesystem::write_bytes(SYMBOLS_PATH, out.into_bytes());
}

pub unsafe fn on_final_input_mapping(
    player_idx: i32,
    out: *const MappedInputs,
    controller_struct: &SomeControllerStruct,
) {
    if !active() || !(0..PLAYERS as i32).contains(&player_idx) {
        return;
    }
    let controller = &*controller_struct.controller;
    let raw: [u8; 8] = {
        let bytes = controller.current_buttons.into_bytes();
        let mut raw = [0u8; 8];
        raw[..bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
        raw
    };
    let mut probe = lock_write(&PROBE);
    let sample = &mut probe.current.fim[player_idx as usize];
    sample.calls = sample.calls.saturating_add(1);
    sample.raw_buttons = u64::from_le_bytes(raw);
    sample.raw_lx = controller.left_stick_x;
    sample.raw_ly = controller.left_stick_y;
    sample.mapped_buttons = (*out).buttons.bits();
    sample.mapped_lx = (*out).lstick_x;
    sample.mapped_ly = (*out).lstick_y;
}

pub unsafe fn on_cpu_controls(controller_data: *const ControlModuleInternal) {
    if !active() {
        return;
    }
    let data = &*controller_data;
    let mut probe = lock_write(&PROBE);
    let sample = &mut probe.current.cpu_controls;
    sample.calls = sample.calls.saturating_add(1);
    sample.controller_index = data.controller_index;
    sample.buttons = data.buttons.bits();
    sample.stick_x = data.stick_x;
    sample.stick_y = data.stick_y;
}

/// A new match: write a header so matches can be told apart in the log
pub unsafe fn on_fighter_init(module_accessor: *mut BattleObjectModuleAccessor) {
    let entry = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry != 0 {
        return;
    }
    let mut probe = lock_write(&PROBE);
    flush(&mut probe);
    probe.frame = 0;
    probe.current = FrameSample::default();
    let header = format!(
        "\n=== match start, training_mode={} ===\n\
         # frame | per player: fim calls, raw buttons, raw stick, mapped buttons, mapped stick \
         | cpu_controls calls/index/buttons/stick | per fighter: cpu?, get_button, stick, status, pos_x\n",
        is_training_mode()
    );
    filesystem::append_bytes(FRAMES_PATH, header.into_bytes());
}

/// Called for every fighter every frame; entry 0 closes the previous frame
pub unsafe fn on_fighter_frame(module_accessor: *mut BattleObjectModuleAccessor) {
    if !active() {
        return;
    }
    let entry = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if !(0..PLAYERS as i32).contains(&entry) {
        return;
    }
    let sample = FighterSample {
        seen: true,
        is_cpu: crate::common::is_operation_cpu(&mut *module_accessor),
        button: ControlModule::get_button(module_accessor),
        stick_x: ControlModule::get_stick_x(module_accessor),
        stick_y: ControlModule::get_stick_y(module_accessor),
        status: StatusModule::status_kind(module_accessor),
        pos_x: PostureModule::pos_x(module_accessor),
    };
    let mut probe = lock_write(&PROBE);
    if entry == 0 && probe.current.fighters[0].seen {
        end_frame(&mut probe);
    }
    probe.current.fighters[entry as usize] = sample;
}

fn end_frame(probe: &mut Probe) {
    probe.frame += 1;
    if probe.frame <= MAX_FRAMES_PER_MATCH {
        let frame = probe.frame;
        let s = probe.current;
        let text = &mut probe.text;
        let _ = write!(text, "{frame}");
        for f in s.fim.iter() {
            let _ = write!(
                text,
                " | {} {:x} {:.2},{:.2} {:x} {},{}",
                f.calls,
                f.raw_buttons,
                f.raw_lx,
                f.raw_ly,
                f.mapped_buttons,
                f.mapped_lx,
                f.mapped_ly
            );
        }
        let c = s.cpu_controls;
        let _ = write!(
            text,
            " | cc {} {} {:x} {:.2},{:.2}",
            c.calls, c.controller_index, c.buttons, c.stick_x, c.stick_y
        );
        for f in s.fighters.iter() {
            let _ = write!(
                text,
                " | {} {:x} {:.2},{:.2} {} {:.1}",
                if f.is_cpu { "cpu" } else { "hum" },
                f.button,
                f.stick_x,
                f.stick_y,
                f.status,
                f.pos_x
            );
        }
        text.push('\n');
        if probe.frame.is_multiple_of(FLUSH_EVERY) {
            flush(probe);
        }
    }
    probe.current = FrameSample::default();
}

fn flush(probe: &mut Probe) {
    if probe.text.is_empty() {
        return;
    }
    let text = std::mem::replace(&mut probe.text, String::with_capacity(64 * 1024));
    filesystem::append_bytes(FRAMES_PATH, text.into_bytes());
}
