//! Replay Takeover state machine
//!
//! Records the inputs of both fighters starting from a save state, plays them back
//! from the same save state, and lets the player take over their fighter on any frame
//! while the CPU keeps following its recorded track.
//!
//! This crate is platform independent so the logic can be tested on the host.
//! The plugin feeds it with the game's input types via the `ReplayInput` and `Mirror` traits.

pub mod codec;
mod frames;
pub use codec::{Codec, DecodeError};
pub use frames::{CpuFrame, FighterSnapshot, MatchSnapshot};

/// Identifies what a recording was made with: player fighter kind, CPU fighter kind, stage id.
/// Playing a slot back with a different context makes no sense and is rejected.
pub type Context = [i32; 3];

/// Inputs that can be flipped horizontally, for save states loaded mirrored
pub trait Mirror {
    fn mirrored(&self) -> Self;
}

/// Player inputs: must know whether they count as "no input" for takeover detection
pub trait ReplayInput: Mirror {
    fn is_neutral(&self) -> bool;
}

/// Per-frame game state recorded alongside the inputs to detect when playback drifts off
pub trait Snapshot: Mirror {
    fn diverges_from(&self, recorded: &Self) -> bool;
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ReplayMode {
    Record,
    Play,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ReplayState {
    Idle,
    /// Waiting for the save state load to finish
    Arming(ReplayMode),
    Recording,
    Playing,
    /// Player 1 is controlled live, the CPU still follows its track
    TakenOver,
}

/// Which fighter the player takes over during playback
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum TakeoverSide {
    /// Player 1 goes live, the CPU keeps following its track
    Player,
    /// The CPU goes live on the player's controller, player 1 keeps following its track
    Cpu,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum RejectReason {
    EmptySlot,
    OtherFighters,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ReplayEvent {
    Rejected(RejectReason),
    RecordingStarted,
    RecordingFinished {
        len: usize,
    },
    PlaybackStarted,
    TakenOver {
        frame: usize,
    },
    /// Playback no longer matches the recording, first diverging frame
    Desync {
        frame: usize,
    },
    PlaybackFinished,
    Cancelled,
}

struct Track<P, C, S> {
    p1: Vec<P>,
    cpu: Vec<C>,
    snapshots: Vec<S>,
    len: usize,
    mirror: f32,
    context: Context,
}

impl<P: Copy + Default, C: Copy + Default, S: Copy + Default> Track<P, C, S> {
    fn new(capacity: usize) -> Self {
        Track {
            p1: vec![P::default(); capacity],
            cpu: vec![C::default(); capacity],
            snapshots: vec![S::default(); capacity],
            len: 0,
            mirror: 1.0,
            context: [0; 3],
        }
    }
}

pub struct Replay<P: Copy + Default, C: Copy + Default, S: Copy + Default> {
    state: ReplayState,
    frame: usize,
    capacity: usize,
    max_frames: usize,
    tracks: Vec<Track<P, C, S>>,
    // Slot used by the running or last replay
    slot: usize,
    record_slot: usize,
    play_slot: usize,
    context: Context,
    // Context at the time recording was requested
    recording_context: Context,
    desync_frame: Option<usize>,
    takeover_on_input: bool,
    // Player must release all inputs once after playback starts before any input counts as takeover,
    // otherwise still holding the playback combo would take over on frame 0
    takeover_armed: bool,
    takeover_frame: Option<usize>,
    flip: bool,
    takeover_side: TakeoverSide,
    // Player's live inputs after a CPU-side takeover
    cpu_live: P,
}

impl<P, C, S> Replay<P, C, S>
where
    P: Copy + Default + ReplayInput,
    C: Copy + Default + Mirror,
    S: Copy + Default + Snapshot,
{
    /// Single slot replay, `capacity` frames long
    pub fn new(capacity: usize) -> Self {
        Self::with_slots(capacity, 1)
    }

    /// All memory is allocated here; recording and playback never allocate
    pub fn with_slots(capacity: usize, slot_count: usize) -> Self {
        let capacity = capacity.max(1);
        Replay {
            state: ReplayState::Idle,
            frame: 0,
            capacity,
            max_frames: capacity,
            tracks: (0..slot_count.max(1))
                .map(|_| Track::new(capacity))
                .collect(),
            slot: 0,
            record_slot: 0,
            play_slot: 0,
            context: [0; 3],
            recording_context: [0; 3],
            desync_frame: None,
            takeover_on_input: true,
            takeover_armed: false,
            takeover_frame: None,
            flip: false,
            takeover_side: TakeoverSide::Player,
            cpu_live: P::default(),
        }
    }

    pub fn state(&self) -> ReplayState {
        self.state
    }

    pub fn frame(&self) -> usize {
        self.frame
    }

    /// Length of the recording in the current slot, 0 if there is none
    pub fn len(&self) -> usize {
        self.tracks[self.slot].len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn slot(&self) -> usize {
        self.slot
    }

    pub fn record_slot(&self) -> usize {
        self.record_slot
    }

    pub fn play_slot(&self) -> usize {
        self.play_slot
    }

    pub fn slot_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn slot_len(&self, slot: usize) -> usize {
        self.tracks.get(slot).map_or(0, |t| t.len)
    }

    /// Slot the next recording goes into. Ignored while a replay runs or if out of range.
    pub fn set_record_slot(&mut self, slot: usize) -> bool {
        if self.is_active() || slot >= self.tracks.len() {
            return false;
        }
        self.record_slot = slot;
        true
    }

    /// Slot the next playback comes from. Ignored while a replay runs or if out of range.
    pub fn set_play_slot(&mut self, slot: usize) -> bool {
        if self.is_active() || slot >= self.tracks.len() {
            return false;
        }
        self.play_slot = slot;
        true
    }

    /// Fighters and stage currently loaded
    pub fn set_context(&mut self, context: Context) {
        self.context = context;
    }

    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    pub fn takeover_frame(&self) -> Option<usize> {
        self.takeover_frame
    }

    pub fn is_active(&self) -> bool {
        self.state != ReplayState::Idle
    }

    /// True while the CPU must follow the recorded track instead of AI and training options
    pub fn drives_cpu(&self) -> bool {
        match self.state {
            ReplayState::Playing => true,
            ReplayState::TakenOver => self.takeover_side == TakeoverSide::Player,
            _ => false,
        }
    }

    pub fn takeover_side(&self) -> TakeoverSide {
        self.takeover_side
    }

    /// Ignored while a replay runs
    pub fn set_takeover_side(&mut self, side: TakeoverSide) -> bool {
        if self.is_active() {
            return false;
        }
        self.takeover_side = side;
        true
    }

    /// True after a CPU-side takeover: the player's controller drives the CPU
    pub fn player_controls_cpu(&self) -> bool {
        self.state == ReplayState::TakenOver && self.takeover_side == TakeoverSide::Cpu
    }

    /// After a CPU-side takeover: the player's live inputs of this frame, for the CPU
    pub fn cpu_takeover_input(&self) -> Option<P> {
        self.player_controls_cpu().then_some(self.cpu_live)
    }

    pub fn set_takeover_on_input(&mut self, enabled: bool) {
        self.takeover_on_input = enabled;
    }

    /// Recording length used for the next recording. Clamped to the track capacity.
    pub fn set_max_frames(&mut self, max_frames: usize) {
        if self.state == ReplayState::Idle {
            self.max_frames = max_frames.clamp(1, self.capacity);
        }
    }

    /// Button combo pressed. Returns true if the caller must load the save state now.
    pub fn request(&mut self, mode: ReplayMode) -> (bool, Option<ReplayEvent>) {
        match (self.state, mode) {
            // Pressing record again ends the recording early
            (ReplayState::Recording, ReplayMode::Record) => (false, Some(self.finish_recording())),
            (ReplayState::Recording, ReplayMode::Play) | (ReplayState::Arming(_), _) => {
                (false, None)
            }
            (_, ReplayMode::Play) if self.tracks[self.play_slot].len == 0 => {
                (false, Some(ReplayEvent::Rejected(RejectReason::EmptySlot)))
            }
            (_, ReplayMode::Play) if self.tracks[self.play_slot].context != self.context => (
                false,
                Some(ReplayEvent::Rejected(RejectReason::OtherFighters)),
            ),
            // From Idle, or restarting from Playing/TakenOver to drill the same situation again
            (_, mode) => {
                self.recording_context = self.context;
                self.slot = match mode {
                    ReplayMode::Record => self.record_slot,
                    ReplayMode::Play => self.play_slot,
                };
                self.state = ReplayState::Arming(mode);
                self.takeover_frame = None;
                (true, None)
            }
        }
    }

    /// The save state finished loading. `mirror` is 1.0 or -1.0 depending on save state mirroring.
    pub fn on_state_loaded(&mut self, mirror: f32) -> Option<ReplayEvent> {
        match self.state {
            ReplayState::Arming(ReplayMode::Record) => {
                let track = &mut self.tracks[self.slot];
                track.p1.fill(P::default());
                track.cpu.fill(C::default());
                track.snapshots.fill(S::default());
                track.len = 0;
                track.mirror = mirror;
                track.context = self.recording_context;
                self.frame = 0;
                self.state = ReplayState::Recording;
                Some(ReplayEvent::RecordingStarted)
            }
            ReplayState::Arming(ReplayMode::Play) => {
                self.frame = 0;
                self.flip = mirror != self.tracks[self.slot].mirror;
                self.takeover_armed = false;
                self.takeover_frame = None;
                self.desync_frame = None;
                self.state = ReplayState::Playing;
                Some(ReplayEvent::PlaybackStarted)
            }
            _ => None,
        }
    }

    /// Manual save state load or menu interaction. Keeps a finished recording.
    pub fn cancel(&mut self) -> Option<ReplayEvent> {
        match self.state {
            ReplayState::Idle => None,
            // A cancelled recording keeps what was recorded so far
            ReplayState::Recording => Some(self.finish_recording()),
            _ => {
                self.state = ReplayState::Idle;
                self.frame = 0;
                Some(ReplayEvent::Cancelled)
            }
        }
    }

    /// Takeover without input, for the "combo only" trigger setting
    pub fn take_over(&mut self) -> Option<ReplayEvent> {
        if self.state != ReplayState::Playing {
            return None;
        }
        self.state = ReplayState::TakenOver;
        self.takeover_frame = Some(self.frame);
        self.cpu_live = P::default();
        Some(ReplayEvent::TakenOver { frame: self.frame })
    }

    /// Player 1's live inputs for this frame. Returns the inputs the game should use.
    pub fn on_player_input(&mut self, live: P) -> (P, Option<ReplayEvent>) {
        match self.state {
            ReplayState::Recording => {
                self.tracks[self.slot].p1[self.frame] = live;
                (live, None)
            }
            ReplayState::Playing => {
                if self.takeover_on_input {
                    if live.is_neutral() {
                        self.takeover_armed = true;
                    } else if self.takeover_armed {
                        let event = self.take_over();
                        return (self.after_takeover(live), event);
                    }
                }
                (self.played(self.tracks[self.slot].p1[self.frame]), None)
            }
            ReplayState::TakenOver => (self.after_takeover(live), None),
            _ => (live, None),
        }
    }

    /// While recording: the CPU slot of the current frame to write into
    pub fn cpu_frame_mut(&mut self) -> Option<&mut C> {
        if self.state == ReplayState::Recording {
            Some(&mut self.tracks[self.slot].cpu[self.frame])
        } else {
            None
        }
    }

    /// While playing back: the CPU inputs the game should use this frame
    pub fn cpu_playback(&self) -> Option<C> {
        if self.drives_cpu() {
            Some(self.played(self.tracks[self.slot].cpu[self.frame]))
        } else {
            None
        }
    }

    /// First frame on which the current or last playback diverged
    pub fn desync_frame(&self) -> Option<usize> {
        self.desync_frame
    }

    /// While recording: the game state slot of the current frame to write into
    pub fn snapshot_mut(&mut self) -> Option<&mut S> {
        if self.state == ReplayState::Recording {
            Some(&mut self.tracks[self.slot].snapshots[self.frame])
        } else {
            None
        }
    }

    /// While playing back: compares the live game state with the recording.
    /// Reports only the first diverging frame of a playback.
    pub fn check_snapshot(&mut self, live: &S) -> Option<ReplayEvent> {
        // After a takeover the state is expected to diverge
        if self.state != ReplayState::Playing || self.desync_frame.is_some() {
            return None;
        }
        let recorded = self.played(self.tracks[self.slot].snapshots[self.frame]);
        if live.diverges_from(&recorded) {
            self.desync_frame = Some(self.frame);
            return Some(ReplayEvent::Desync { frame: self.frame });
        }
        None
    }

    /// End of a game frame. Called once per frame after the CPU controls are applied.
    pub fn advance(&mut self) -> Option<ReplayEvent> {
        match self.state {
            ReplayState::Recording => {
                self.frame += 1;
                if self.frame >= self.max_frames {
                    return Some(self.finish_recording());
                }
                None
            }
            ReplayState::Playing | ReplayState::TakenOver => {
                self.frame += 1;
                if self.frame >= self.tracks[self.slot].len {
                    self.state = ReplayState::Idle;
                    self.frame = 0;
                    return Some(ReplayEvent::PlaybackFinished);
                }
                None
            }
            _ => None,
        }
    }

    fn finish_recording(&mut self) -> ReplayEvent {
        let len = self.frame;
        self.tracks[self.slot].len = len;
        self.frame = 0;
        self.state = ReplayState::Idle;
        ReplayEvent::RecordingFinished { len }
    }

    /// Serializes one slot for saving to the SD card
    pub fn encode_slot(&self, slot: usize) -> Option<Vec<u8>>
    where
        P: Codec,
        C: Codec,
        S: Codec,
    {
        let track = self.tracks.get(slot)?;
        let mut out = Vec::with_capacity(codec::HEADER_SIZE + track.len * 64);
        out.extend_from_slice(&codec::MAGIC);
        out.push(codec::VERSION);
        codec::put_u32(&mut out, track.len as u32);
        codec::put_f32(&mut out, track.mirror);
        track
            .context
            .iter()
            .for_each(|c| codec::put_i32(&mut out, *c));
        for i in 0..track.len {
            track.p1[i].encode(&mut out);
            track.cpu[i].encode(&mut out);
            track.snapshots[i].encode(&mut out);
        }
        Some(out)
    }

    /// Restores one slot from saved bytes. Leaves the slot untouched on any error.
    pub fn decode_slot(&mut self, slot: usize, bytes: &[u8]) -> Result<(), DecodeError>
    where
        P: Codec,
        C: Codec,
        S: Codec,
    {
        use DecodeError::*;
        if slot >= self.tracks.len() {
            return Err(SlotOutOfRange);
        }
        if self.is_active() {
            return Err(Busy);
        }
        let mut input = bytes;
        let magic = codec::take::<4>(&mut input).ok_or(Truncated)?;
        if magic != codec::MAGIC {
            return Err(BadMagic);
        }
        let version = codec::take::<1>(&mut input).ok_or(Truncated)?[0];
        if version != codec::VERSION {
            return Err(UnsupportedVersion(version));
        }
        let len = codec::take_u32(&mut input).ok_or(Truncated)? as usize;
        // Checked before reading frames so a corrupt length never allocates
        if len > self.capacity {
            return Err(TooLong(len));
        }
        let mirror = codec::take_f32(&mut input).ok_or(Truncated)?;
        let mut context = [0; 3];
        for c in context.iter_mut() {
            *c = codec::take_i32(&mut input).ok_or(Truncated)?;
        }
        // Decode into scratch first so a broken file leaves the slot untouched
        let mut frames = Vec::with_capacity(len);
        for _ in 0..len {
            let p1 = P::decode(&mut input).ok_or(Truncated)?;
            let cpu = C::decode(&mut input).ok_or(Truncated)?;
            let snapshot = S::decode(&mut input).ok_or(Truncated)?;
            frames.push((p1, cpu, snapshot));
        }
        if !input.is_empty() {
            return Err(TrailingBytes);
        }
        let track = &mut self.tracks[slot];
        for (i, (p1, cpu, snapshot)) in frames.into_iter().enumerate() {
            track.p1[i] = p1;
            track.cpu[i] = cpu;
            track.snapshots[i] = snapshot;
        }
        track.len = len;
        track.mirror = mirror;
        track.context = context;
        Ok(())
    }

    /// Player 1's inputs once taken over: live, or the track if the player took the CPU
    fn after_takeover(&mut self, live: P) -> P {
        match self.takeover_side {
            TakeoverSide::Player => live,
            TakeoverSide::Cpu => {
                self.cpu_live = live;
                self.played(self.tracks[self.slot].p1[self.frame])
            }
        }
    }

    fn played<T: Mirror + Copy>(&self, input: T) -> T {
        if self.flip {
            input.mirrored()
        } else {
            input
        }
    }
}
