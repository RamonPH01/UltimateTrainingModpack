//! The per-frame path runs inside the game's input hooks. It must never allocate:
//! an allocation can take a lock in the system allocator and cost a frame.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use training_mod_replay::*;

struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(|c| c.get()) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.with(|c| c.get()) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn count_allocations(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(|a| a.get())
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Pad(i8);

impl Mirror for Pad {
    fn mirrored(&self) -> Self {
        Pad(self.0.saturating_neg())
    }
}

impl ReplayInput for Pad {
    fn is_neutral(&self) -> bool {
        self.0 == 0
    }
}

/// One frame exactly as the plugin hooks drive it, using the real plugin data types
fn run_frame(r: &mut Replay<Pad, CpuFrame, MatchSnapshot>, live: Pad, events: &mut usize) {
    let snap = MatchSnapshot::default();
    if let Some(s) = r.snapshot_mut() {
        *s = snap;
    } else if r.check_snapshot(&snap).is_some() {
        *events += 1;
    }
    let (out, ev) = r.on_player_input(live);
    std::hint::black_box(out);
    *events += ev.is_some() as usize;
    if let Some(c) = r.cpu_frame_mut() {
        c.stick_x = 0.5;
        c.command_flags[0] = 1;
    } else {
        std::hint::black_box(r.cpu_playback());
        std::hint::black_box(r.cpu_takeover_input());
    }
    *events += r.advance().is_some() as usize;
}

#[test]
fn record_playback_and_takeover_never_allocate() {
    let mut r: Replay<Pad, CpuFrame, MatchSnapshot> = Replay::with_slots(600, 5);
    let mut events = 0;
    let allocations = count_allocations(|| {
        for side in [TakeoverSide::Player, TakeoverSide::Cpu] {
            for slot in 0..5 {
                r.set_context([1, 2, 3]);
                r.set_takeover_side(side);
                r.set_record_slot(slot);
                r.request(ReplayMode::Record);
                r.on_state_loaded(1.0);
                for i in 0..600 {
                    run_frame(&mut r, Pad((i % 100) as i8), &mut events);
                }
                r.set_play_slot(slot);
                r.request(ReplayMode::Play);
                r.on_state_loaded(-1.0);
                for i in 0..600 {
                    // neutral, then takeover halfway through
                    let live = if i < 300 { Pad(0) } else { Pad(1) };
                    run_frame(&mut r, live, &mut events);
                }
            }
        }
        r.cancel();
    });
    assert_eq!(allocations, 0);
    // Sanity: the loop really recorded, played and took over
    assert!(events >= 2 * 5 * 3, "events: {events}");
}

#[test]
fn ring_buffer_and_start_search_never_allocate() {
    // The capture keeps the last frames in a preallocated ring buffer and searches it
    // when the player marks a frame
    const LEN: usize = 600;
    let restorable = [0, 1];
    let mut history = std::collections::VecDeque::with_capacity(LEN);
    let mut found = 0;
    let allocations = count_allocations(|| {
        for i in 0..3 * LEN {
            if history.len() == LEN {
                history.pop_front();
            }
            let mut s = MatchState::default();
            s.player.pos_x = (i % 50) as f32 * 0.5;
            s.player.status_kind = if i % 7 == 0 { 0 } else { 39 };
            history.push_back(s);
            found += find_start(&history, history.len() - 1, 120, &restorable).is_some() as usize;
        }
    });
    assert_eq!(allocations, 0);
    assert!(found > 0);
}
