# mush Rust port

`rust/` contains the structured Rust implementation of `mush`.

The original `mu.sh` launcher remains the legacy single-file implementation. The Rust app now mirrors the same major feature surface in a proper multi-module workspace: synth, drums, looper, WAV capture, project save/load, MIDI device routing/remap/bindings, audio device state, scope mode, and camera visuals.

## Architecture

The audio engine uses a **lock-free** architecture to prevent priority inversion:

- **UI Thread**: Owns `AppState`, pushes parameters to audio via `ArcSwap` (atomic pointer swap)
- **Audio Thread**: Reads parameters lock-free, never blocks on UI
- **Communication**: 
  - UI→Audio params: `arc-swap` for atomic snapshots
  - UI→Audio commands: `rtrb` SPSC lock-free queue
  - Audio→UI reactive state: `std::sync::atomic` wrapper types

Sample rate is dynamically set from the CPAL device (48kHz on most modern systems) rather than hardcoded.

## Install without Cargo

See **[GitHub Releases](https://github.com/sedoy26/mush/releases)** for pre-built `mush-cli` (v0.0.1+). Download the artifact for your OS, `chmod +x` on Unix, run from the **repo root** next to `projects/` and `wav/`. Optional: `ffmpeg` for camera mode.

## Startup (from source)

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p mush-cli
```

Release build:

```bash
cargo run --release --manifest-path rust/Cargo.toml -p mush-cli
```

You can also run from inside `rust/`:

```bash
cd rust
cargo run -p mush-cli
```

When launched from the repo root, the Rust app uses the same top-level `projects/` and `wav/` folders as the shell version.

## Current runtime coverage

- terminal UI with keyboard control loop
- realtime synth audio via `cpal`
- dual-oscillator playback with waveform, level, octave, detune, voices, filter, LFO, and FX controls
- 6-voice, 32-step drum sequencer with 20 selectable drum banks
- loop replace, overdub, undo, play, and clear state flow
- global WAV capture into numbered `wav/mush*.wav` files
- project save/load into `.mush` JSON files
- MIDI input open/close, note remap, and learnable pad/knob bindings
- audio device enumeration and default/named input/output selection state
- scope and camera visual modes with reactive camera styles via `ffmpeg`

## Controls

### Global

- `TAB` — switch synth/drum focus
- `S` — open or close settings
- `H` — toggle help
- `q` — quit

### Synth

- `a w s e d f t g y h u j k` — play chromatic notes
- key release — releases note when the terminal reports key-up
- `SPACE` — force note release
- `1` / `2` — select active oscillator
- `z` / `x` — previous/next waveform on active oscillator
- `←` / `→` — base note
- `↑` / `↓` — master volume
- `[` / `]` — attack
- `{` / `}` — release
- `m` — gate mode
- `p` — filter on/off
- `-` / `=` — cutoff
- `_` / `+` — resonance
- `l` — cycle LFO waveform
- `o` — cycle LFO target
- `,` / `.` — LFO rate
- `;` / `'` — LFO depth
- `D` / `F` — drive
- `J` / `K` — delay mix
- `N` / `M` — delay feedback
- `V` / `B` — delay time
- `W` / `A` / `E` — warmth / air / reverb increase

### Loop and recording

- `R` — start loop replace recording
- `T` — start overdub recording
- `Y` — undo last overdub snapshot
- `P` — toggle loop playback
- `U` — clear loop
- `G` — toggle global WAV recording

### Drums

- `TAB` into drum focus first
- arrow keys — move the sequencer cursor
- `SPACE` — toggle current step
- `r` — start/stop sequencer
- `c` — clear current row
- `X` — clear all rows
- `1` / `2` — load built-in starter patterns
- `,` / `.` — BPM -/+ 1
- `<` / `>` — BPM -/+ 5
- `-` / `=` — selected drum voice level

### Settings

- `[` / `]` — previous/next settings page
- `↑` / `↓` — select settings row
- `←` / `→` — adjust selected row value
- `ENTER` / `SPACE` — trigger selected action row
- `MAIN` — voices, active oscillator, waveform, osc level, osc octave, osc detune, visual mode, scope source, drum bank, warmth, air, reverb
- `CAM FX` — camera reactive style and reactivity amount
- `PROJECT` — choose project, open selected, save next
- `SOUND DEVICE` — stored output/input selection and refresh/reopen flow
- `MIDI DEVICE` — enable, device selection, refresh, channel, note input, pad input
- `MIDI NOTE` — edit source note, learn source, edit destination, save map, clear map
- `MIDI MAP` — choose target, arm MIDI learn, clear binding

## Requirements

- Rust toolchain compatible with the workspace lockfile
- macOS terminal with CoreAudio support for `cpal`
- optional MIDI device for external note input
- optional `ffmpeg` for live camera mode

## Architecture

- `crates/mush-core` — state model, commands, runtime, project I/O, and audio engine
- `crates/mush-cli` — interactive terminal frontend

Key modules:

- `rust/crates/mush-core/src/audio.rs` — realtime synth, drums, loop, FX, and WAV capture
- `rust/crates/mush-core/src/runtime.rs` — audio/MIDI bootstrap and shared runtime state
- `rust/crates/mush-core/src/project_io.rs` — `.mush` project persistence and numbered WAV pathing
- `rust/crates/mush-cli/src/main.rs` — keyboard input, rendering, and control mapping

## Worth knowing

- The Rust app uses actual key-release events when the terminal provides them, so note release behavior is cleaner than the old curses timeout approximation.
- Running from the repo root is recommended so saved projects and WAV files stay in the same top-level folders as the shell app.
- The data model already includes the same main subsystems as the legacy app, making continued parity work easier and safer than editing a giant heredoc.
- Camera mode shells out to `ffmpeg` on macOS using `avfoundation`; set `MUSH_CAMERA_DEVICE` if you need a different camera index.

## Validation

```bash
cargo build --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml
```
