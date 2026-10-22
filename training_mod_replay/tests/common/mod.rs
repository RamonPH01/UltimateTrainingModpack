#![allow(dead_code)]

pub use training_mod_replay::*;

/// Stand-in for MappedInputs: one button flag and a horizontal stick
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Pad {
    pub button: bool,
    pub x: i8,
}

impl Mirror for Pad {
    fn mirrored(&self) -> Self {
        Pad {
            button: self.button,
            x: self.x.saturating_neg(),
        }
    }
}

impl ReplayInput for Pad {
    fn is_neutral(&self) -> bool {
        !self.button && self.x == 0
    }
}

/// Stand-in for the CPU controller data plus command flags
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Cpu {
    pub stick_x: f32,
    pub cmd: i32,
}

impl Mirror for Cpu {
    fn mirrored(&self) -> Self {
        Cpu {
            stick_x: -self.stick_x,
            cmd: self.cmd,
        }
    }
}

/// Stand-in for the game state: one position, exact comparison
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Snap {
    pub x: f32,
}

impl Mirror for Snap {
    fn mirrored(&self) -> Self {
        Snap { x: -self.x }
    }
}

impl Snapshot for Snap {
    fn diverges_from(&self, recorded: &Self) -> bool {
        self.x != recorded.x
    }
}

pub type TestReplay = Replay<Pad, Cpu, Snap>;

pub const NEUTRAL: Pad = Pad {
    button: false,
    x: 0,
};

pub fn pad(x: i8) -> Pad {
    Pad { button: false, x }
}

pub fn press() -> Pad {
    Pad { button: true, x: 0 }
}

pub fn cpu(n: i32) -> Cpu {
    Cpu {
        stick_x: n as f32 / 10.0,
        cmd: n,
    }
}

/// Runs one game frame in the order of the plugin hooks:
/// frame handler (game state), player input mapping, CPU controls + advance
pub fn frame_with(
    r: &mut TestReplay,
    live_p1: Pad,
    live_cpu: Cpu,
    live_snap: Snap,
) -> (Pad, Cpu, Vec<ReplayEvent>) {
    let mut events = Vec::new();
    if let Some(slot) = r.snapshot_mut() {
        *slot = live_snap;
    } else {
        events.extend(r.check_snapshot(&live_snap));
    }
    let (p1, ev) = r.on_player_input(live_p1);
    events.extend(ev);
    let cpu = if let Some(slot) = r.cpu_frame_mut() {
        *slot = live_cpu;
        live_cpu
    } else {
        r.cpu_playback().unwrap_or(live_cpu)
    };
    events.extend(r.advance());
    (p1, cpu, events)
}

pub fn frame(r: &mut TestReplay, live_p1: Pad, live_cpu: Cpu) -> (Pad, Cpu, Vec<ReplayEvent>) {
    let snap = Snap {
        x: r.frame() as f32,
    };
    frame_with(r, live_p1, live_cpu, snap)
}

/// Records `n` frames where P1 holds stick x = frame+1, the CPU sends cmd = 100+frame
/// and the game state is x = frame
pub fn recorded(n: usize) -> TestReplay {
    let mut r = Replay::new(60);
    assert_eq!(r.request(ReplayMode::Record), (true, None));
    assert_eq!(r.on_state_loaded(1.0), Some(ReplayEvent::RecordingStarted));
    for i in 0..n {
        frame(&mut r, pad(i as i8 + 1), cpu(100 + i as i32));
    }
    assert_eq!(
        r.request(ReplayMode::Record),
        (false, Some(ReplayEvent::RecordingFinished { len: n }))
    );
    r
}

pub fn start_playback(r: &mut TestReplay, mirror: f32) {
    assert_eq!(r.request(ReplayMode::Play), (true, None));
    assert_eq!(r.state(), ReplayState::Arming(ReplayMode::Play));
    assert_eq!(
        r.on_state_loaded(mirror),
        Some(ReplayEvent::PlaybackStarted)
    );
}

impl Codec for Pad {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.button as u8);
        codec::put_i8(out, self.x);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        let button = codec::take::<1>(input)?[0] != 0;
        let x = codec::take_i8(input)?;
        Some(Pad { button, x })
    }
}

impl Codec for Cpu {
    fn encode(&self, out: &mut Vec<u8>) {
        codec::put_f32(out, self.stick_x);
        codec::put_i32(out, self.cmd);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(Cpu {
            stick_x: codec::take_f32(input)?,
            cmd: codec::take_i32(input)?,
        })
    }
}

impl Codec for Snap {
    fn encode(&self, out: &mut Vec<u8>) {
        codec::put_f32(out, self.x);
    }

    fn decode(input: &mut &[u8]) -> Option<Self> {
        Some(Snap {
            x: codec::take_f32(input)?,
        })
    }
}

/// Bytes per frame of the test types: Pad 2 + Cpu 8 + Snap 4
pub const TEST_FRAME_SIZE: usize = 14;

/// Records `n` frames into `slot`, P1 stick x = base + frame
pub fn record_into(r: &mut TestReplay, slot: usize, n: usize, base: i8) {
    assert!(r.set_record_slot(slot));
    assert_eq!(r.request(ReplayMode::Record), (true, None));
    r.on_state_loaded(1.0);
    for i in 0..n {
        frame(r, pad(base + i as i8), cpu(base as i32 + i as i32));
    }
    r.request(ReplayMode::Record);
    assert_eq!(r.slot_len(slot), n);
}
