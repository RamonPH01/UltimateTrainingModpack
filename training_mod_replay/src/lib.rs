//! Replay Takeover state machine
//!
//! Records the inputs of both fighters starting from a save state, plays them back
//! from the same save state, and lets the player take over their fighter on any frame
//! while the CPU keeps following its recorded track.
//!
//! This crate is platform independent so the logic can be tested on the host.
//! The plugin feeds it with the game's input types via the `ReplayInput` and `Mirror` traits.

/// Inputs that can be flipped horizontally, for save states loaded mirrored
pub trait Mirror {
    fn mirrored(&self) -> Self;
}

/// Player inputs: must know whether they count as "no input" for takeover detection
pub trait ReplayInput: Mirror {
    fn is_neutral(&self) -> bool;
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

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ReplayEvent {
    RecordingStarted,
    RecordingFinished { len: usize },
    PlaybackStarted,
    TakenOver { frame: usize },
    PlaybackFinished,
    Cancelled,
}

pub struct Replay<P: Copy + Default, C: Copy + Default> {
    state: ReplayState,
    frame: usize,
    len: usize,
    max_frames: usize,
    p1_track: Vec<P>,
    cpu_track: Vec<C>,
    takeover_on_input: bool,
    // Player must release all inputs once after playback starts before any input counts as takeover,
    // otherwise still holding the playback combo would take over on frame 0
    takeover_armed: bool,
    takeover_frame: Option<usize>,
    recorded_mirror: f32,
    flip: bool,
}

impl<P: Copy + Default + ReplayInput, C: Copy + Default + Mirror> Replay<P, C> {
    pub fn new(max_frames: usize) -> Self {
        Replay {
            state: ReplayState::Idle,
            frame: 0,
            len: 0,
            max_frames,
            p1_track: vec![P::default(); max_frames],
            cpu_track: vec![C::default(); max_frames],
            takeover_on_input: true,
            takeover_armed: false,
            takeover_frame: None,
            recorded_mirror: 1.0,
            flip: false,
        }
    }

    pub fn state(&self) -> ReplayState {
        self.state
    }

    pub fn frame(&self) -> usize {
        self.frame
    }

    /// Length of the stored recording in frames, 0 if there is none
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
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
        matches!(self.state, ReplayState::Playing | ReplayState::TakenOver)
    }

    pub fn set_takeover_on_input(&mut self, enabled: bool) {
        self.takeover_on_input = enabled;
    }

    /// Recording length used for the next recording. Clamped to the track capacity.
    pub fn set_max_frames(&mut self, max_frames: usize) {
        if self.state == ReplayState::Idle {
            self.max_frames = max_frames.clamp(1, self.p1_track.len());
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
            (_, ReplayMode::Play) if self.len == 0 => (false, None),
            // From Idle, or restarting from Playing/TakenOver to drill the same situation again
            (_, mode) => {
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
                self.p1_track.iter_mut().for_each(|i| *i = P::default());
                self.cpu_track.iter_mut().for_each(|i| *i = C::default());
                self.len = 0;
                self.frame = 0;
                self.recorded_mirror = mirror;
                self.state = ReplayState::Recording;
                Some(ReplayEvent::RecordingStarted)
            }
            ReplayState::Arming(ReplayMode::Play) => {
                self.frame = 0;
                self.flip = mirror != self.recorded_mirror;
                self.takeover_armed = false;
                self.takeover_frame = None;
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
        Some(ReplayEvent::TakenOver { frame: self.frame })
    }

    /// Player 1's live inputs for this frame. Returns the inputs the game should use.
    pub fn on_player_input(&mut self, live: P) -> (P, Option<ReplayEvent>) {
        match self.state {
            ReplayState::Recording => {
                self.p1_track[self.frame] = live;
                (live, None)
            }
            ReplayState::Playing => {
                if self.takeover_on_input {
                    if live.is_neutral() {
                        self.takeover_armed = true;
                    } else if self.takeover_armed {
                        let event = self.take_over();
                        return (live, event);
                    }
                }
                (self.played(self.p1_track[self.frame]), None)
            }
            _ => (live, None),
        }
    }

    /// While recording: the CPU slot of the current frame to write into
    pub fn cpu_frame_mut(&mut self) -> Option<&mut C> {
        if self.state == ReplayState::Recording {
            Some(&mut self.cpu_track[self.frame])
        } else {
            None
        }
    }

    /// While playing back: the CPU inputs the game should use this frame
    pub fn cpu_playback(&self) -> Option<C> {
        if self.drives_cpu() {
            Some(self.played(self.cpu_track[self.frame]))
        } else {
            None
        }
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
                if self.frame >= self.len {
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
        self.len = self.frame;
        self.frame = 0;
        self.state = ReplayState::Idle;
        ReplayEvent::RecordingFinished { len: self.len }
    }

    fn played<T: Mirror + Copy>(&self, input: T) -> T {
        if self.flip {
            input.mirrored()
        } else {
            input
        }
    }
}
