# Handover: Replay Import from the Vault

Branch: `replay-takeover-vault` (based on `main` at `c180d76`)

## Goal

Turn a replay from the game's own replay viewer (Vault) into a Replay Takeover slot.
You can then practice the situation in training mode: the replay plays from a chosen frame,
and you take over your fighter at any moment while the opponent keeps following the replay.

We do not take over inside the Vault itself. Replay files are undocumented, likely encrypted,
and tied to a game version. Instead, the plugin **captures** the inputs and fighter states
while the replay is shown, at normal speed. Playback and takeover then reuse the
training-mode Replay Takeover that already exists on `main`.

## Status at a glance

| Part | Where | State |
|---|---|---|
| Replay Takeover in training mode (record, play, take over, desync detection, 5 SD slots, CPU-side takeover) | `main` | Done, CI green, beta release published. **Not yet tested in-game.** |
| Diagnostic build that logs input hooks outside training mode | this branch, `src/replay_probe.rs` | Done, built. **Waiting for logs from a real Switch.** |
| Start point selection for imported replays | this branch, `training_mod_replay/src/start.rs` | Done, tested against real match data |
| Capture in the Vault (ring buffer, mark combo, write slot) | – | Not started, blocked on the logs |
| Restoring the extended state in training mode | – | Not started |
| Fixed random seed per attempt | – | Not started |

## How the code is laid out

```
training_mod_replay/        platform independent crate, all logic, tested on the host
  src/lib.rs                Replay state machine (record / play / takeover / slots)
  src/frames.rs             CpuFrame, FighterSnapshot, MatchSnapshot (desync check)
  src/codec.rs              binary slot format "UTMR" v1, little-endian
  src/start.rs              NEW: FighterState, MatchState, find_start, MAX_STEP
  tests/                    host tests, see below
  tests/fixtures/fetch.sh   NEW: downloads the real-match fixture (not committed)
  tests/reframed/mod.rs     NEW: parser for that fixture

src/training/replay.rs      plugin glue: hooks -> state machine, notifications, SD slots
src/replay_probe.rs         diagnostic logging, this branch only
```

Rule of thumb: anything that decides something goes into `training_mod_replay` with tests.
The plugin only reads and writes game memory and calls into the crate.

### Hooks the replay feature uses (plugin)

- `handle_final_input_mapping`: the player's inputs (record / play back / take over)
- `set_cpu_controls`: the CPU's inputs (`ControlModuleInternal`)
- `get_command_flag_cat`: CPU command flags (mash options)
- fighter `on_frame`: per-frame snapshots and button combos
- `save_states::request_load` / `mirror_state`: starts every recording and playback from a save state

Idle cost is one atomic load per hook (`ACTIVE`, `PLAYER_CONTROLS_CPU`).

## What this branch adds

### 1. Diagnostic build (`src/replay_probe.rs`)

This code is only active outside training mode.

- **At boot:** scans the game's read-only data for strings containing "replay" and tries
  to resolve mangled C++ names as exported symbols. The result is written to
  `sd:/ultimate/TrainingModpack/replay_probe_symbols.txt`.
- **Every frame of a match or replay:** logs the following, for player slots 0 and 1:
  - which input hooks fired
  - the raw controller and the mapped inputs
  - what each fighter's `ControlModule` sees (buttons, stick)
  - status and position

  The result is appended to `sd:/ultimate/TrainingModpack/replay_probe_frames.txt`, flushed
  every 120 frames, at most 20000 frames per match. A `=== match start ===` header
  separates matches.

The question it answers is **where replay inputs enter the fighters**:

- through final input mapping like live inputs, or
- written straight into `ControlModule`.

That decides how the capture records the player's track.

Build: CI run 39832273593, artifact `plugin`
(https://github.com/RamonPH01/UltimateTrainingModpack/actions/runs/39832273593).
Later pushes to this branch produce newer artifacts with the same probe.

Test procedure for whoever has the Switch:

1. Replace `atmosphere/contents/01006A800016E000/romfs/skyline/plugins/libtraining_modpack.nro`.
2. Play a ~20 s normal match vs a CPU with clear actions (3 jumps, 3 smash attacks), then save the replay.
3. Watch it in the Vault. Hold one harmless button for a few seconds and note which one.
4. Start any other match briefly so the end of the log is flushed.
5. Send both txt files.

### 2. Start point selection (`training_mod_replay/src/start.rs`)

A save state only restores what it stores. It cannot resume a move that is halfway done.
So an imported replay must start on a frame where both fighters can be restored exactly.

- `FighterState`: what will be restored.
  - Fields: position, speed, facing (`lr`), damage, shield, hitstun, status, situation (ground/air), jumps used.
  - Implements `Mirror` and `Codec` (44 bytes).
- `is_restorable(statuses)`: true when all of these hold:
  - the status is in the given list
  - no hitstun
  - all numbers are finite
  - The list of statuses is passed in because status IDs are runtime Lua constants in the plugin.
- `continues(prev)`: true when the fighter moved at most `MAX_STEP` on each axis; anything more is a teleport.
- `find_start(history, mark, window, statuses)`:
  - Returns the latest frame at or before `mark`, at most `window` frames back, where both fighters are restorable.
  - It never searches across a teleport.
  - It never picks the first frame of the buffer, because there is no way to tell whether that frame was teleported to.
  - It returns `None` rather than a start that would desync.
  - It works on a `VecDeque`, used as a preallocated ring buffer, and does not allocate.

`MAX_STEP = 20.0` is calibrated on real data:

- The fastest real movement in the fixture is 11 units/frame (Pikachu up special).
- Respawns and state reloads jump 233 units or more.

Test setup restorable list:

```
WAIT, WALK, RUN, SQUAT_WAIT, GUARD, FALL, FALL_AERIAL
```

`DASH`, `LANDING` and `GUARD_ON` are left out on purpose, because they run on a fixed frame count.

## Tests

```sh
cd training_mod_replay
./tests/fixtures/fetch.sh     # once; CI does this in the "Replay Test Fixtures" step
cargo test                    # 125 tests
cargo clippy --all-targets
cargo fmt --check
```

| File | Tests | Covers |
|---|---|---|
| `test_replay.rs` | 27 | state machine: arming, record, play, takeover, cancel |
| `test_codec.rs` | 18 | slot file format, every corrupt-input case |
| `test_slots.rs` | 15 | 5 slots, context check, busy rules |
| `test_cpu_side.rs` | 14 | taking over the CPU instead of the player |
| `test_desync.rs` | 12 | first desync frame, mirror aware |
| `test_frames.rs` | 9 | tolerances, NaN handling |
| `test_start.rs` | 21 | start point selection, synthetic cases, codec, mirror |
| `test_start_reframed.rs` | 7 | start point selection against a real session |
| `test_no_alloc.rs` | 2 | per-frame paths never allocate (counting global allocator) |

### Real-match fixture

`fetch.sh` downloads a training-mode session recorded on a Switch by
[ReFramed](https://github.com/reframedultimate/ReFramed) (Pikachu vs Joker, Final Destination, 1496 frames).

- Source repository: `reframedultimate/ReFramed-server-emulator`.
- Pinned to commit `dcbafcaf`; sha256 `538e3c4e…`.
- The repository has **no license**, so the file is gitignored and never committed.
- The format is documented in `tests/reframed/mod.rs`.
- It contains states only, **no inputs**.
- Speed is derived from the position difference between frames.
- Duplicate rows (hitlag, standing still) are kept as they come.

Once the probe logs from a real Switch exist, they should become a second fixture. They
include inputs, and since they are our own recordings they can be committed.

### Checking the plugin on a Mac

`cargo check` of the plugin fails on macOS with "invalid Mach-O section specifier" from `skyline::main`. Use this workaround:

1. Comment out `#[skyline::main(name = "training_modpack")]` in `src/lib.rs`.
2. Run `cargo check` / `cargo clippy`.
3. Restore the line. Never commit it commented out.

A Linux-target check fails locally on `openssl-sys` (no cross gcc). CI builds the real NRO.

## CI

`.github/workflows/rust.yml`:

- **Pushes to `main`** build and test, then replace the `beta` release.
- **Other branches** only build when started by hand:

  ```sh
  gh workflow run rust.yml -R RamonPH01/UltimateTrainingModpack --ref replay-takeover-vault
  ```

  The `Upload Beta Release` job is skipped there, so the beta release stays untouched.
  The NRO is in the run's `plugin` artifact.
- The upload job needs `permissions: contents: write`, otherwise it fails with a 403 on a fork.

Last run on this branch: 39857207985, all green.

## Next steps

### 1. Read the probe logs (blocked on the Switch tester)

Compare the replay section with the normal match section:

- Does `fim calls` stay > 0 during the replay?
- Do the mapped inputs match the fighter's `ControlModule` values, and do they show the actions played?
- Does the held button show up anywhere? It should not; if it does, live input leaks into the replay.
- In `replay_probe_symbols.txt`, is there a usable exported symbol to detect "replay is playing"?

If neither hook carries the replay inputs, `ControlModule::get_button` / `get_stick_*` per
fighter in `on_frame` is the fallback. The probe logs those too.

### 2. Capture in the Vault (crate first, test first)

A new `Capture` type in `training_mod_replay` would:

- Keep a preallocated `VecDeque` ring buffer of the last N frames (inputs of both fighters plus a `MatchState`).
- On the mark combo, run `find_start` (window 120). If there is no start, emit an event so
  the plugin shows "no clean start point here" and records nothing.
- From the start frame on, move inputs into a slot until `max_frames` or until the combo is pressed again.
- Store the start `MatchState` with the slot. This means slot format **v2**:
  - add the start state after the header
  - keep reading v1
  - test both
- Context: fighter kinds and stage come from the match; training mode rejects mismatches already.

Two things to check on the Switch:

- whether button combos register in the Vault at all. If not, read the controller directly.
- whether the replay viewer's speed controls skip frames. Capture should refuse anything but normal speed.

### 3. Restore the extended state in training mode

The current save states restore position, damage, facing and charge. This step also restores:

- speed (KineticModule)
- shield (`FIGHTER_INSTANCE_WORK_ID_FLOAT_GUARD_SHIELD`)
- jumps used
- situation
- status from the restorable list

Do it only for imported slots; normal save states stay as they are. The desync detection then shows how exact the restore is: the first desync frame is the measure.

ReFramed's plugin is a good reference for which module calls read these values (`src/lib.rs` there).

### 4. Fixed random seed per attempt

Find the game's RNG state (or the function that seeds it) and set the same value on every playback start. Then every attempt in training mode behaves the same, for example items and random moves.

## Known limits

- An imported replay cannot match the original match exactly. Staling (moves weakening when repeated), the original RNG state, projectiles and items are not restored. Short sequences from neutral work best.
- Starting in the middle of an action is refused on purpose (`find_start` returns `None`).
- Training mode first start after installing this fork resets the menu once (new menu fields).

## Conventions on this project

- Test first: stubs with wrong results, red tests, then the implementation. After that, mutate the implementation and confirm tests catch it.
- No allocation on per-frame paths, enforced by `test_no_alloc.rs`.
- Keep tolerances tight (positions 0.01). Derive limits from real data, not guesses.
- Commits end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01HAgC3XfZKQDTrbfKNRddDY
  ```
