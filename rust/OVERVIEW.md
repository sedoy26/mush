# MUSH Architecture Overview

A terminal-based synthesizer with real-time audio, loop recording, drum sequencer, and multiple visualization modes.

## Project Structure

```
rust/
├── Cargo.toml (workspace root)
├── Cargo.lock
├── crates/
│   ├── mush-core/          # Library: core engine, state, audio
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                 (exports Runtime, App)
│   │       ├── runtime.rs             (main orchestrator, lock-free audio lifecycle)
│   │       ├── audio.rs               (real-time audio engine, sample-rate-aware DSP)
│   │       ├── audio_bridge.rs        (lock-free UI↔Audio sync: arc-swap, rtrb, atomics)
│   │       ├── dsp.rs                 (DSP utilities: SmoothEnvelope, SimpleReverb, PolyBLEP)
│   │       ├── app.rs                 (command handler)
│   │       ├── camera.rs              (ffmpeg webcam + apply_visual_fx post-processing)
│   │       ├── project_io.rs          (file I/O for projects and recordings)
│   │       ├── ports.rs               (MIDI port management)
│   │       ├── commands.rs            (keyboard command dispatch)
│   │       ├── state/                 (application state)
│   │       │   ├── mod.rs             (AppState: main state container)
│   │       │   ├── synth.rs           (dual oscillators, envelope, filters, FX)
│   │       │   ├── drums.rs           (drum sequencer, 6 voices, 20 banks)
│   │       │   ├── looper.rs          (loop recording, overdub, undo stack)
│   │       │   ├── audio.rs           (reactive levels, scope buffers, device info)
│   │       │   ├── midi.rs            (MIDI mappings, learn mode)
│   │       │   ├── ui.rs              (visual modes, VisualFx, themes, settings pages)
│   │       │   └── project.rs         (project save/load data)
│   │       └── visuals/               (pluggable visual effects system)
│   │           ├── mod.rs             (Visual trait, VisualRegistry)
│   │           ├── framebuffer.rs     (Cell, Color, Framebuffer, apply_brightness_fx)
│   │           ├── params.rs          (ParamSpec, ParamValue for tunable params)
│   │           ├── util/              (math, palette, ramp helpers)
│   │           └── [10 effect modules]
│   │               ├── plasma.rs, kaleidoscope.rs, matrix_rain.rs
│   │               ├── fire.rs, tunnel.rs, donut.rs, fireworks.rs
│   │               └── ripples.rs, radio.rs, cube.rs
│   │
│   └── mush-cli/           # Binary: terminal UI, rendering, input handling
│       ├── Cargo.toml
│       └── src/
│           └── main.rs                (single file: ~2500 lines)
│               ├── main() entry point
│               ├── Event loop: poll input, update state, render
│               ├── Keyboard handlers: handle_synth_key, handle_drum_key
│               ├── Settings UI: navigation, value cycling (5 pages: Main, Visuals, Project, SoundDevice, Midi)
│               ├── Rendering: UI panels, synth/drum editors, visualizations
│               └── Visualizers: scope_braille, VisualRegistry effects, camera
```

## High-Level Data Flow

```
┌─────────────────────────────────────────────────────────────┐
│                     mush-cli (UI Process)                    │
├─────────────────────────────────────────────────────────────┤
│  Terminal Input → Keyboard Handler → Command Processing     │
│       ↓                                    ↓                 │
│   Update AppState                 Push AudioParams via      │
│   (UI-owned, no mutex)            arc-swap, commands via    │
│                                   SPSC queue (rtrb)          │
│  AppState → Render Loop → Canvas → Terminal Output          │
│              (60 FPS)                                        │
└─────────────────────────────────────────────────────────────┘
                           ↓ ↑
           ┌───────────────┴─┴───────────────────┐
           │   AudioBridge (lock-free sync)      │
           │   • ArcSwap<AudioParams> for state  │
           │   • SPSC rtrb queue for commands    │
           │   • Atomics for reactive levels     │
           └─────────────────────────────────────┘
                           ↓ ↑
┌─────────────────────────────────────────────────────────────┐
│              mush-core (Audio Thread)                        │
├─────────────────────────────────────────────────────────────┤
│  CPAL Callback (dynamic sample rate, typically 48kHz)       │
│       ↓                                                      │
│  Load AudioParams snapshot (lock-free arc-swap load)        │
│  Pop commands from SPSC queue (non-blocking)                │
│       ↓                                                      │
│  ┌─ Synth Engine ──┐      ┌─ Drum Engine ──┐                │
│  │ • PolyBLEP Oscs │      │ • 6 Drum Voices│                │
│  │ • SmoothEnv ADSR│      │ • Pattern Seq  │                │
│  │ • Filters (LP)  │      │ • 20 Sound Banks│               │
│  │ • FX: Delay,    │      │ • Trigger Decay │               │
│  │   Reverb, Drive │      │                 │               │
│  └─────────────────┘      └─────────────────┘               │
│       ↓                              ↓                       │
│  Mix Synth + Drums + Loop          Apply Master FX          │
│       ↓                                                      │
│  Continuous Soft Limiter           Write to Output Buffer   │
│  (tanh, no threshold)                                        │
│       ↓                                                      │
│  Update Reactive Levels (atomics)  Update Scope Buffers     │
│  (for UI feedback)                 (for visualization)       │
└─────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. **AppState** (`state/mod.rs`)
Central application state container, owned by the UI thread.

```rust
pub struct AppState {
    pub synth: SynthState,           // Oscillators, envelope, filters, FX
    pub drums: DrumState,            // Sequencer, triggers, banks
    pub looper: LoopState,           // Loop buffer, playback state, undo
    pub midi: MidiState,             // Input mappings, learn mode
    pub audio: AudioState,           // Levels, scope buffers, devices
    pub ui: UiState,                 // Visual mode, focus, settings page
    pub project: ProjectState,       // Unsaved changes flag
}
```

**Current Design** (✅ lock-free architecture implemented):
- UI thread exclusively owns `AppState` — no mutex, no polling
- `AudioBridge` mediates UI↔Audio communication via lock-free primitives:
  - `ArcSwap<AudioParams>` for synth/drum/looper parameters (UI→Audio)
  - SPSC ringbuffer (`rtrb`) for commands: note on/off, record start/stop
  - Atomics for reactive levels: master/synth/drums/kick/snare/hat (Audio→UI)
- UI updates `AppState`, then pushes changed params to `AudioBridge`
- Audio callback loads params snapshot (lock-free), never blocks on UI

---

### 1a. **AudioParams** (`audio_bridge.rs`)
The snapshot of parameters sent from UI to audio thread. This is the central type of the lock-free architecture.

```rust
pub struct AudioParams {
    pub synth: SynthState,           // Full synth state (includes UI-only fields — see note)
    pub drums: DrumState,            // Full drum state (pattern, volumes, bank)
    pub looper: LooperParams,        // Subset of loop state for playback
    pub global_recording: bool,      // Whether to write to WAV
}

pub struct LooperParams {
    pub recording: bool,
    pub playing: bool,
    pub overdub: bool,
    pub play_gain: f32,
    pub playback_speed: f32,
    pub trim_start: f32,             // 0.0–1.0 fraction to skip at start
    pub trim_end: f32,               // 0.0–1.0 fraction to skip at end
}
```

⚠️ **Design Issue**: `SynthState` and `DrumState` are sent wholesale, including UI-only fields like `active_osc` (which oscillator is selected for editing). This means UI navigation triggers param swaps even when audio params are unchanged. Cleaner design: extract `SynthParams`/`DrumParams` containing only audio-relevant fields.

**Design Rules**:
- **Params** (arc-swap): Continuous values sampled each buffer — cutoff, gain, envelope times, LFO rate, drum pattern.
- **Commands** (SPSC queue): Discrete events — note on/off, loop record/play/clear, undo.

⚠️ **Design Issue**: `global_recording: bool` should probably be a command (`StartRecording`/`StopRecording`), not a param. Recording has setup (file open, header write) that fits better as a discrete event than a continuously-sampled value.

**Memory Considerations**:
- `AudioParams` is moderate size (~2-4KB with drum pattern: 6×32 bools + voice params)
- UI swaps a new `Arc<AudioParams>` when any field changes (including UI-only fields due to design issue above)
- Worst case with active editing: ~60 allocs/sec × 4KB = 240KB/sec — still negligible
- Idle state: no allocations (same Arc persists)

**Parameter Smoothing**:
Audio engine maintains internal "current" values that exponentially approach `AudioParams` targets:
- **Pitch/frequency**: Smoothed with ~4ms time constant via `attack_coeff()` (prevents clicks)
- **Envelope**: `SmoothEnvelope` continues from current value on retrigger
- **Filter cutoff**: Currently NOT smoothed — zipper noise on fast sweeps
- **Gain/mix levels**: Currently NOT smoothed — clicks on abrupt changes

**Note on smoothing granularity**: Smoothing is per-sample (inside the buffer loop), not per-buffer. This matters because at 512-sample buffers (10.7ms @ 48kHz), per-buffer smoothing with a 4ms time constant would barely smooth. The code applies `current += (target - current) * coeff` each sample, where `coeff = attack_coeff(time_ms, sample_rate)` produces an exponential smoothing coefficient.

---

### 2. **Runtime** (`runtime.rs`)
Orchestrator that manages lifecycle and resource allocation.

**Responsibilities**:
- Initialize `AppState` and `AudioEngine`
- Start/stop audio stream via CPAL
- Manage audio device selection and enumeration
- Handle MIDI input connections
- Manage camera stream for webcam visuals
- Flush recording to disk on command

**Key Methods**:
- `new()` — Create runtime, init subsystems
- `start_audio()` — Start audio callback thread
- `refresh_audio_devices()` — Enumerate available outputs
- `refresh_midi_devices()` — Enumerate MIDI inputs
- `render_camera_ascii()` — Get rendered webcam frame

---

### 3. **Audio Engine** (`audio.rs`)
Real-time DSP engine running on audio callback thread.

✅ **Dynamic Sample Rate**: Sample rate read from CPAL device config (typically 48kHz). All DSP calculations use proper sample-rate-aware coefficients from `dsp.rs`.

**Components**:

#### Synth (`SynthState` + synthesis)
- **Dual Oscillators**: Independent waveform, level, octave, detune
  - Waveforms: Sine, Triangle, Saw, Square (PolyBLEP anti-aliased), Noise
  - Detune for natural-sounding chords
- **ADSR Envelope**: Modulates amplitude of note (SmoothEnvelope for click-free retrigger)
  - `attack`, `decay`, `sustain`, `release` times
  - Sample-rate-aware coefficients via `decay_coeff()`/`attack_coeff()`
  - Retrigger continues from current value (no clicks)
- **Lowpass Filter**: Resonant filter with cutoff/resonance
  - Driven by LFO or keyboard
- **FX Chain**:
  - **Drive**: Soft saturation (tanh) for warmth
  - **Delay**: Configurable time and feedback (capped at 0.95, up to 2 seconds)
  - **Reverb**: Freeverb-style comb + allpass filter network (`SimpleReverb`)
  - **Warmth/Air/Reverb**: Tone shaping controls
- **LFO**: Modulation source (sine/tri/saw/sq) targeting pitch, filter, or amplitude

#### Drums (`DrumState` + synthesis)
- **6 Drum Voices**: Kick, Snare, Hat, and 3 additional
- **20 Sound Banks**: Vintage drum synth styles (909, 808, TR-808, etc.)
- **32-Step Sequencer**: 16th-note grid
  - Each step can trigger any voice
  - Pattern storage per bank
- **Per-Voice**: Pitch, decay, filter modulation
- **Trigger Decay**: `kick_level *= 0.92` per sample for expressive feel

#### Looper (`LoopState` + buffer)
- **8-Second Loop Buffer**: Mono or mixed stereo
- **Recording**: Captures synth+drums mix
- **Overdub**: Layer new audio over existing loop
- **Playback**: Wrapping read pointer with speed modulation
- **Editing**: 
  - Gain: 0.0–1.0 (volume control)
  - Trim start/end: Skip portions of loop (0.0–0.9 fraction)
  - Speed: 0.25x–4.0x time-stretch
- **Undo Stack**: Stores loop snapshots before overdub

#### Mixing & Output
- **Three Buses**: Synth (0.68x gain), Drums (0.30x gain), Loop (variable gain)
- **Soft Limiter**: Continuous tanh saturation — applied to ALL samples, not conditional
- **Master Gain**: 0.46x to prevent digital clipping
- **Scope Buffer**: Last ~512 samples for UI waveform display
- **Reactive Levels**: Peak detection per bus + drum trigger decay
  - `master`: Overall mix level
  - `synth`, `drums`: Per-bus levels
  - `kick`, `snare`, `hat`: Trigger-based (1.0 → decay)

---

### 3a. **DSP Utilities** (`dsp.rs`)
Shared DSP primitives with proper sample-rate-aware math. Fixes common audio bugs.

**Constants**:
- `DENORMAL_PREVENTION = 1e-24` — Small DC offset added to feedback paths. While this is above the true denormal threshold (~1.18e-38 for f32), it prevents signals from decaying into the denormal range where CPU spikes occur. Alternative: set FTZ/DAZ CPU flags on audio thread.
- `MAX_FEEDBACK = 0.95` — Cap for delay feedback. Reverb combs may use different (higher) internal feedback.

**Functions**:
```rust
// Continuous soft limiter: tanh(x * drive) / tanh(drive) * ceiling
// Applied to ALL samples — no threshold discontinuity
pub fn soft_limit(sample: f32, drive: f32, ceiling: f32) -> f32;

// Sample-rate-aware envelope coefficients
pub fn decay_coeff(time_sec: f32, sample_rate: f32) -> f32;  // exp(-1/(t*sr))
pub fn attack_coeff(time_sec: f32, sample_rate: f32) -> f32;

// PolyBLEP anti-aliasing for saw/square oscillators
pub fn poly_blep(t: f32, dt: f32) -> f32;
pub fn saw_blep(phase: f32, dt: f32) -> f32;
pub fn square_blep(phase: f32, dt: f32, pw: f32) -> f32;

// Basic DSP building blocks
pub fn one_pole_lp(input: f32, prev: f32, coeff: f32) -> f32;
pub fn allpass(input: f32, buffer: &mut [f32], idx: usize, feedback: f32) -> f32;
pub fn comb_filter(input: f32, buffer: &mut [f32], idx: usize, feedback: f32, damp: f32, ...) -> f32;
```

**Structs**:

`SmoothEnvelope` — Click-free ADSR with proper retrigger:
- Always continues from current value on note-on (no pop from jumping to 0)
- Sample-rate-aware attack/decay coefficients
- States: Idle, Attack, Decay, Sustain, Release

`SimpleReverb` — Freeverb-style comb + allpass network:
- 8 parallel comb filters (different prime lengths)
- 4 series allpass filters for diffusion
- Denormal prevention in feedback paths
- Configurable room size, damping, wet/dry

---

### 4. **State: Synth** (`state/synth.rs`)
Synth parameter state. **Currently monophonic** — single voice with dual oscillators.

```rust
pub struct SynthState {
    pub oscillators: [OscillatorState; 2],    // OSC 1 & 2 (layered, not polyphonic)
    pub active_osc: usize,                    // Which to edit in UI
    pub pitch: f32,                           // Current note pitch (single voice)
    pub env: EnvelopeState,                   // ADSR times (shared)
    pub filter: FilterState,                  // Cutoff, resonance (shared)
    pub lfo: LFOState,                        // Waveform, rate, target
    pub fx: FXState,                          // Drive, delay, reverb
    pub gate_mode: GateMode,                  // Held (sustained) or Gated (note-off releases)
}

pub enum GateMode {
    Held,   // Note sustains until next note or explicit release
    Gated,  // Note releases immediately when key released
}
```

**Polyphony Status**: The synth is monophonic with retriggering — new notes steal the single voice. True polyphony would require:
- Per-voice state: pitch, envelope position, filter state, oscillator phases
- Voice allocator with stealing policy (oldest/quietest)
- Shared params (ADSR times, filter cutoff) separate from per-voice state

This is out of scope for the current design. The focus is on bass/lead synthesis where monophonic with portamento is typical.

---

### 5. **State: Drums** (`state/drums.rs`)
Drum sequencer state.

```rust
pub struct DrumState {
    pub steps: [[bool; 32]; 6],       // 6 voices × 32 steps
    pub volumes: [f32; 6],            // Per-voice volume (0.0–1.0)
    pub running: bool,                // Sequencer playing?
    pub bpm: f32,                     // Tempo in beats per minute
    pub current_step: usize,          // Current step (0–31)
    pub bank: usize,                  // Sound bank (0–19)
    pub triggers: [bool; 6],          // Pending triggers for this step
}
```

**Timing**: Steps are 16th notes. At 120 BPM, each step = 125ms. The sequencer advances `current_step` based on elapsed samples, calculated from `bpm` and sample rate.

---

### 6. **State: Looper** (`state/looper.rs`)
Loop recorder state.

```rust
pub struct LoopState {
    pub length: usize,         // Samples captured
    pub write_pos: usize,      // Where to write next
    pub read_pos: usize,       // Where to read for playback
    pub recording: bool,       // Currently capturing?
    pub playing: bool,         // Play back?
    pub overdub: bool,         // Layering onto existing?
    pub has_audio: bool,       // Non-empty loop exists?
    pub play_gain: f32,        // Playback volume (default 0.7, range 0.0–1.0)
    pub trim_start: f32,       // 0.0–1.0 fraction to skip at start
    pub trim_end: f32,         // 0.0–1.0 fraction to skip at end
    pub playback_speed: f32,   // 0.25–4.0 time-stretch
    pub undo_stack: Vec<LoopSnapshot>,  // Snapshots for undo
}

pub struct LoopSnapshot {
    pub length: usize,         // Length at snapshot time
}
```

**Loop Buffer Location**: The actual audio buffer (~384k samples @ 48kHz) lives on the **audio thread**, not in `LoopState`. `LoopState` contains metadata; the buffer is owned by `AudioEngine`. The `has_audio` flag is tracked via `ReactiveState.loop_has_audio` atomic so UI knows whether a loop exists without crossing the thread boundary with the buffer itself.

**Default Gain**: `play_gain` defaults to 0.7 (audible). This ensures loaded loops play at reasonable volume. The serde deserializer uses `#[serde(default = "default_play_gain")]` to handle old project files that may have saved 0.0.

**Undo**: Each overdub pushes a `LoopSnapshot` before modifying. Undo restores the previous length. The undo stack is capped at 10 entries (~15MB max) to prevent unbounded memory growth.

---

### 7. **State: MIDI** (`state/midi.rs`)
MIDI input configuration and learning.

```rust
pub struct MidiState {
    pub input_device: usize,           // Selected input
    pub learn_mode: MidiLearnMode,     // What to bind?
    pub note_map: Map<MidiNote → SynthNote>,  // Remapping
    pub bindings: Vec<MidiBinding>,    // Knob/pad assignments
}
```

**MIDI Learn**: Press key, MIDI data updates binding in real-time.

---

### 8. **State: Audio** (`state/audio.rs`)
Audio system state (devices, levels, recording). This is **UI-side state**, copied from AudioBridge atomics.

```rust
pub struct GlobalRecordingState {
    pub recording: bool,           // Currently writing WAV?
    pub last_path: Option<String>, // Path of last recording
    pub last_error: Option<String>,// Error message if failed
}

pub struct ReactiveLevels {
    pub master: f32,           // 0.0–1.0 overall level
    pub synth: f32,            // Synth bus level
    pub drums: f32,            // Drum bus level
    pub kick: f32,             // Kick trigger (1.0 → decay)
    pub snare: f32,
    pub hat: f32,
    pub note: f32,             // Synth note active?
}

pub struct AudioState {
    pub input: AudioDeviceSelection,
    pub output: AudioDeviceSelection,
    pub input_devices: Vec<AudioDeviceInfo>,
    pub output_devices: Vec<AudioDeviceInfo>,
    pub reactive: ReactiveLevels,
    pub recent_scope: Vec<f32>,         // Last 512 samples (master mix)
    pub recent_scope_synth: Vec<f32>,   // Last 512 samples (synth bus)
    pub status: String,                 // Status message
    pub xruns: usize,                   // Underrun counter
    pub global_recording: GlobalRecordingState,
}
```

**Data Flow**: `ReactiveLevels` and scope buffers are copied from `AudioBridge` atomics/arc-swaps into `AudioState` by `bridge.sync_to_app_state()` each UI frame. This lets rendering code access them without atomic loads on every access.

**Scope Buffer Selection**: `recent_scope` shows master mix (synth + drums + loop). `recent_scope_synth` shows synth-only. UI setting `scope_show_drums` toggles which buffer the visualizer reads.

**Global Recording**: WAV writing uses a dedicated background thread:
1. Audio thread writes samples to a lock-free ringbuffer (~1 second capacity)
2. Writer thread drains ringbuffer to disk in ~100ms chunks
3. On buffer overflow (disk too slow), samples are dropped (logged but not fatal)
4. On stop, writer thread flushes remaining samples and closes file
5. Errors are reported via `GlobalRecordingState.last_error`

This keeps disk I/O completely off the real-time audio path.

---

### 9. **State: UI** (`state/ui.rs`)
User interface state.

```rust
pub enum VisualMode {
    Scope,          // Unicode braille waveform (built-in)
    Camera,         // Live webcam + FX (built-in, requires ffmpeg)
    // 10 pluggable effects from VisualRegistry:
    Plasma,         // Classic plasma effect
    Kaleidoscope,   // Amplitude-reactive polar geometry
    MatrixRain,     // Falling code rain
    Fire,           // Fire simulation
    Tunnel,         // Tunnel zoom effect
    Donut,          // Spinning 3D torus
    Fireworks,      // Particle fireworks
    Ripples,        // Water ripples
    Radio,          // Radio wave rings
    Cube,           // Rotating 3D wireframe cube
}

pub enum VisualFx {
    Off,            // No audio-reactive post-processing
    KickFlash,      // Kick drum triggers brightness flash
    SynthGlow,      // Synth level adds warm glow
    DrumPunch,      // Overall drum level pulses contrast
    BassScan,       // Kick creates scanline sweep
    EdgePulse,      // Edge detection pulses with beat
    GlitchShift,    // Random color channel shifts on kicks
    GatePoster,     // Posterization depth from gate state
    FireStorm,      // Fire palette shift on drums
    IcePulse,       // Cool blue pulse on snare
    ChromaSplit,    // RGB split based on drum hits
    MatrixBeat,     // Matrix-style color from beat
    WaveRipple,     // Synth level creates radial ripple
    BeatStrobe,     // Master level strobes brightness
    HatSparkle,     // Hi-hat triggers random bright pixels
    SnareBurst,     // Snare creates expanding brightness rings
    BassWobble,     // Kick causes horizontal wave distortion
    LfoSweep,       // LFO phase creates scanning brightness
    EnvelopeFade,   // Envelope value controls overall fade
    DrumGrid,       // Different drums affect different grid regions
    FreqShift,      // Pitch frequency shifts contrast
    ComboReact,     // Complex combination of multiple sources
}

pub enum SettingsPage {
    Main,           // Synth/drum/loop parameters
    Visuals,        // Visual mode, FX style, depth
    Project,        // Save/load projects
    SoundDevice,    // Audio device selection
    Midi,           // MIDI input, mappings, learn mode
}

pub struct UiState {
    pub help_open: bool,
    pub settings_open: bool,
    pub settings_page: SettingsPage,
    pub theme: Theme,
    pub visual_mode: VisualMode,
    pub visual_fx: VisualFx,           // Audio-reactive effect (applies to ALL visuals)
    pub visual_fx_depth: f32,          // Intensity of the visual FX (0.0–1.0, default 0.85)
    pub scope_show_drums: bool,        // Which scope buffer to display
}
```

**Visual FX Architecture**: The `VisualFx` effects are **post-processing filters** applied to the framebuffer after the base visual renders. This means any framebuffer-based visual mode (Camera, Plasma, Donut, etc.) can have audio-reactive effects like KickFlash or SynthGlow overlaid. The `visual_fx_depth` controls intensity (0.0 = no effect, 1.0 = full effect).

**Context-Aware Settings**: The Visuals settings page shows different options based on the current visual mode:
- **Scope**: Visual selector + "Drums scope" toggle (2 rows)
- **All others** (Camera, Plasma, Donut, etc.): Visual selector + FX Style + FX Depth (3 rows)

**Backward Compatibility**: `visual_fx` has serde alias `camera_style` and `visual_fx_depth` has alias `camera_reactivity` for loading old project files.

---

### 10. **CLI** (`mush-cli/src/main.rs`)
Single-file terminal UI (~2500 lines). Handles all rendering and input.

**Event Loop**:
1. **Poll Input** (16ms) — keyboard, MIDI queue, signals
2. **Handle Command** — Update `AppState` based on input
3. **Push Params** — If state changed, build `AudioParams` and swap via `AudioBridge`
4. **Read Reactive** — Load reactive levels from atomics for visual feedback
5. **Render** — Draw UI to canvas buffer
6. **Flush Terminal** — Output to screen

**Key Functions**:

#### Input Handlers
- `handle_synth_key()` — Keyboard controls for synth (notes, effects, record)
- `handle_drum_key()` — Keyboard controls for sequencer
- `parse_command()` — Dispatch KeyCode → Function

#### UI Rendering
- `render_main_ui()` — Synth editor, drum grid, visuals, status
- `render_settings_ui()` — Tabbed settings for synth, MIDI, visuals
- `render_help_overlay()` — Context help

#### Visualization System
- `render_scope_braille()` — Unicode waveform (·▀▄ characters) for Scope mode
- `VisualRegistry` — 10 pluggable visual effects implementing `Visual` trait:
  - Each effect has tunable parameters (`ParamSpec`), tick/render methods
  - Effects render to `Framebuffer` (terminal cell grid with color)
  - Registry manages selection, cycling, reset on switch
- `runtime.render_camera_ascii()` — ffmpeg webcam output for Camera mode
- `apply_visual_fx()` — Audio-reactive post-processing applied after framebuffer render
  - 22 audio-reactive styles plus Off (see `VisualFx` enum)
  - Applies to all framebuffer-based visuals (Camera, Plasma, Donut, etc.)

**Canvas Concept**: 
- Build full screen as `Vec<String>` in memory
- Diff with previous frame to minimize terminal writes
- Uses `crossterm` for colors, cursor positioning

**Two-Layer Rendering**:
- **Canvas layer**: Static UI elements (panels, labels, settings)
- **Overlay layer**: Dynamic visual effects rendered cell-by-cell with proper clipping around UI elements

---

## Audio Processing Pipeline

### Per-Block (Buffer of ~128-512 Samples @ Device Sample Rate)

Sample rate is read from CPAL device config (typically 48kHz on macOS/most devices).

```
┌─────────────────────── Audio Callback ──────────────────────┐
│             (runs ~100-400 times/second, device-dependent)   │
├────────────────────────────────────────────────────────────┤
│                                                              │
│  0. LOAD PARAMS (lock-free arc-swap)                         │
│     └─ AudioParams snapshot for this buffer                  │
│     └─ Pop commands from SPSC queue                          │
│                                                              │
│  1. Read CURRENT PARAMS (from arc-swap snapshot)              │
│     ├─ Synth pitch, filter, env params                      │
│     ├─ Drum pattern (current step)                          │
│     └─ Loop playback position                               │
│                                                              │
│  2. GENERATE SYNTH AUDIO                                     │
│     ├─ Oscillators 1+2 → PolyBLEP waveforms (anti-aliased)  │
│     ├─ LFO modulation → apply to pitch/filter               │
│     ├─ SmoothEnvelope ADSR → multiply amplitude             │
│     ├─ Lowpass filter → resonant cutoff                     │
│     └─ Output → synth_out[]                                 │
│                                                              │
│  3. GENERATE DRUM AUDIO                                      │
│     ├─ Check drum pattern for step triggers                 │
│     ├─ Per-triggered voice: decay envelope                  │
│     ├─ Per-voice tone (sine sweep or filtered noise)        │
│     └─ Output → drum_out[]                                  │
│                                                              │
│  4. LOOP RECORDING / PLAYBACK                                │
│     ├─ If recording or overdub: write synth+drum mix        │
│     ├─ Advance loop_write position                          │
│     ├─ If playing: read from loop at playback_speed         │
│     └─ Apply gain and trim to loop_out[]                    │
│                                                              │
│  5. MIX BUSES                                                │
│     ├─ out = synth_out[] × 0.68                             │
│     ├─    + drum_out[]  × 0.30                              │
│     ├─    + loop_out[]  × loop_gain                         │
│     └─ Apply Master FX (drive, delay, SimpleReverb)         │
│                                                              │
│  6. LIMITING & OUTPUT                                        │
│     ├─ Continuous soft limiter on ALL samples               │
│     │   (tanh saturation, no threshold discontinuity)       │
│     ├─ Master gain × 0.46 → write to buffer                 │
│     └─ Report any xruns                                     │
│                                                              │
│  7. UPDATE REACTIVE STATE (atomics)                          │
│     ├─ Push samples to scope buffers                         │
│     ├─ Write reactive levels via atomics (peak + decay)     │
│     ├─ Update drum_step and drum_triggers atomics           │
│     └─ UI reads on next 60Hz tick (lock-free)               │
│                                                              │
└────────────────────────────────────────────────────────────┘
```

---

## Threading Model

### Three-Thread Architecture

**Thread 1: UI (Main)**
- Runs event loop (16ms polling)
- Exclusively owns `AppState` — no mutex needed for UI reads
- Handles keyboard/MIDI input → updates AppState
- Pushes params to audio via `AudioBridge` (lock-free)
- Reads reactive levels from atomics for visual feedback
- Renders UI every frame
- Status: Latency-tolerant (60 Hz refresh)

**Thread 2: Audio (Spawned via CPAL)**
- Runs audio callback at device sample rate (~21µs per sample @ 48kHz)
- Loads `AudioParams` snapshot via `arc_swap` (lock-free, non-blocking)
- Pops commands from SPSC queue (non-blocking)
- Generates DSP output using `dsp.rs` utilities
- Writes reactive levels to atomics for UI
- Status: Hard real-time (any delay = audio glitch)

**Thread 3: WAV Writer (Spawned on Recording Start)**
- Drains samples from audio thread's ringbuffer
- Writes to disk in ~100ms chunks
- Isolated from real-time path — disk I/O doesn't block audio
- Terminates when recording stops

### Lock-Free Synchronization (With Caveats)

The audio callback **never blocks waiting for the UI thread**. Most communication uses lock-free or non-blocking mechanisms. However, there are two mutex wrappers to be aware of:

| Direction | Mechanism | Data | Crate |
|-----------|-----------|------|-------|
| UI→Audio | `ArcSwap<AudioParams>` | Synth/drum/looper params snapshot | `arc-swap` |
| UI→Audio | SPSC ringbuffer | Commands: note on/off, record, etc. | `rtrb` |
| Audio→UI | Atomics | Reactive levels, drum step position | `std::sync::atomic` |
| Audio→UI | `ScopeBufferPool` (triple buffering) | Scope buffers (512 samples × 2) | `arc-swap` |

**AudioBridge** (`audio_bridge.rs`):
```rust
pub struct AudioBridge {
    pub params: ArcSwap<AudioParams>,           // UI swaps, audio loads
    pub reactive: Arc<ReactiveState>,           // Atomics for levels
    pub scope_pool: ScopeBufferPool,            // Triple-buffered scope data
}
// Command channel created separately via create_command_channel()
// Producer owned by Runtime (UI thread), Consumer moved into audio callback
```

**ScopeBufferPool** uses triple buffering with ArcSwap:
```rust
pub struct ScopeBufferPool {
    buffers: [ArcSwap<Vec<f32>>; 3],       // Pre-allocated, rotated
    buffers_synth: [ArcSwap<Vec<f32>>; 3],
    write_idx: AtomicUsize,                 // Current write buffer
}
```

### Mutex Usage (Important Caveats)

**1. Command Consumer Mutex (Audio Thread Only)**:
```rust
// In audio callback closure:
let command_rx = Arc::new(Mutex::new(command_rx));
```
The SPSC consumer is wrapped in `Mutex` for interior mutability (CPAL's `Fn` callback can't use `&mut self`). However, this mutex is **only ever accessed from the audio callback thread** — there is never contention. Uncontested mutex acquisition uses a fast path (no syscall), costing a handful of atomics — negligible but nonzero. This is not architecturally pure "lock-free" but is practically equivalent since no thread ever waits.

**Note**: If CPAL's callback accepted `FnMut` and allowed owned state, the mutex could be eliminated entirely by owning the consumer in the closure. Worth revisiting if CPAL's API changes.

**2. AudioEngine Mutex (try_lock for Export/Import)**:
```rust
// In SharedAudio:
engine: Arc<Mutex<AudioEngine>>,
```
The audio engine uses `try_lock()` for project save/load operations (export loop buffer, import state). Audio callback uses `try_lock()` which either succeeds immediately or returns `None` — **non-blocking** but not strictly lock-free. During project save/load (~10-50ms), the audio callback may fail to acquire the lock and output silence. **⚠️ This is audible and problematic for live performance.** See Technical Debt for fix options.

**3. Command Producer Mutex (UI Thread Only)**:
```rust
// In Runtime:
command_tx: Mutex<Option<rtrb::Producer<AudioCommand>>>,
```
Used for interior mutability (`push` requires `&mut`). Only accessed from UI thread — no contention with audio.

### Why This Architecture Works

- `arc_swap::ArcSwap` uses atomic pointer operations — load/store are wait-free
- `rtrb` SPSC queue is lock-free by design (single producer, single consumer)
- Atomics with `Relaxed` ordering are sufficient for level meters (eventual consistency OK)
- Audio thread never **waits** on UI thread — if queue is empty, proceed; if swap hasn't happened, use previous params
- The only blocking operations are export/import (try_lock) which cause brief silence, not deadlock

---

## Keyboard Command Dispatch

```
Keyboard Input (crossterm event loop)
        ↓
Focus Context (Synth or Drums?)
        ↓
┌─────────────────────────────────────┐
│ handle_synth_key (if Synth focus)   │
├─────────────────────────────────────┤
│ a-k (note keys)  → synth.pitch      │
│ 1/2              → active_osc       │
│ z/x              → waveform         │
│ R                → record toggle    │
│ T                → overdub          │
│ P                → play toggle      │
│ U                → clear loop       │
│ I/O              → loop gain        │
│ Shift+2/3/4/5/6/7 → trim/speed     │
│ S                → settings mode    │
│ H                → help toggle      │
│ TAB              → switch focus     │
└─────────────────────────────────────┘
        ↓
Update AppState
```

---

## Visual Modes

### Scope (Braille)
- **Input**: Last ~512 samples from scope buffer
- **Buffer Selection**: `recent_scope` (master mix) or `recent_scope_synth` (synth only), controlled by `scope_show_drums` setting
- **Rendering**: Unicode braille (·▀▄▓█) character grid, downsampled to fit terminal width
- **Update**: Every frame (60 Hz)
- **Latency**: 1–2 frames (~16ms)

### Camera
- **Input**: Webcam stream via ffmpeg subprocess
- **Processing**: Resize to terminal, map grayscale to ASCII ramp
- **Post-Processing**: Visual FX can be applied (same as other framebuffer visuals)
- **Update**: Every 500ms (limited by ffmpeg overhead)

### Pluggable Effects (10 total)

All pluggable effects implement the `Visual` trait and render to a `Framebuffer`:

| Effect | Description | Audio Reactivity |
|--------|-------------|------------------|
| **Plasma** | Classic sine-based plasma | Colors shift with master level |
| **Kaleidoscope** | Polar geometry with mirroring | Amplitude drives visualization size |
| **MatrixRain** | Falling code rain | Rain speed varies with drums |
| **Fire** | Fire simulation | Intensity from master level |
| **Tunnel** | Zoom tunnel effect | Speed from reactive levels |
| **Donut** | Spinning 3D torus | Rotation speed modulated by master + kick + snare |
| **Fireworks** | Particle explosions | Spawns on kick/snare triggers |
| **Ripples** | Water ripples | Spawns across full visual area |
| **Radio** | Radio wave rings | Emits on beat |
| **Cube** | Rotating 3D wireframe | Speed and depth from master level |

### Visual FX Post-Processing

All framebuffer-based visuals (Camera + 10 pluggable effects) can have audio-reactive post-processing:

- **Application**: After `visual.render(fb)`, UI calls `apply_visual_fx(fb, visual_fx, depth, reactive)`
- **Implementation**: `camera.rs::apply_visual_fx()` — originally for camera, now public for all visuals
- **Effects**: 22 audio-reactive styles plus Off (see `VisualFx` enum)
- **Modulation Sources**: kick, snare, hat triggers; synth/drum/master levels; LFO phase; envelope value; pitch frequency
- **Performance**: Single pass over framebuffer cells, minimal cost

---

## MIDI Integration

**Note**: MIDI input only. Output (notes, CC, clock) is not implemented.

### MIDI Input Flow
```
MIDI Device → midir Input → Port Callback Queue
        ↓
Main Event Loop (every 16ms)
        ↓
Process Queued MIDI Messages
        ↓
Check MIDI Learn Mode
├─ If learning: Add binding
└─ Else: Apply learned mapping
        ↓
Dispatch to Target
├─ Note On → Play Synth Note
├─ CC → Update Synth/Drum Parameter
└─ Program Change → Load Bank/Pattern
        ↓
Update AppState
```

**MIDI Learn**: 
1. User enters Learn Mode (press specific key)
2. Program highlights "waiting for MIDI"
3. User sends MIDI CC or Note
4. System stores binding with target parameter
5. Future MIDI on that channel/CC updates that parameter

**Architecture Note**: MIDI is processed on the UI thread (every 16ms poll). Bindings are stored in `MidiState` (UI-side). When a CC comes in, the UI translates it into a parameter change, updates `AppState`, and swaps `AudioParams`. The audio thread never sees raw MIDI — it only sees final parameter values. This keeps MIDI complexity out of the real-time path.

---

## File I/O

### Project Save/Load
- **Format**: JSON (serde_json)
- **Contents**: Synth state, drum patterns, loop, MIDI mappings
- **Location**: `~/.mush/projects/`

### Recording Export
- **Format**: WAV (device sample rate, typically 48kHz, 16-bit mono or stereo)
- **Trigger**: Stop global recording (G key)
- **Filename**: `mush_001.wav`, `mush_002.wav`, etc. (auto-numbered)
- **Location**: `~/.mush/wav/` or `./wav/` (relative to working dir)

### Webcam Frames
- **Source**: ffmpeg subprocess (AVFoundation on macOS)
- **Handling**: Raw frames sent to CameraRuntime for ASCII rendering

---

## Performance Notes

### Audio Latency
CPAL uses platform defaults; actual latency varies by device and driver:
- **macOS CoreAudio**: Typically ~10-20ms round-trip (low latency is default)
- **Linux ALSA/PulseAudio**: ~20-100ms depending on driver configuration
- **Measurement**: Not currently exposed in UI; would require CPAL latency query

### ✅ Dynamic Sample Rate (Implemented)

Sample rate is read from CPAL device config at stream creation (typically 48kHz on macOS/most devices).

**What's now correct**:
- `sample_rate: f32` stored on `AudioEngine` from `StreamConfig`
- Filter coefficients use `cutoff_hz / sample_rate`
- Envelope coefficients use `decay_coeff(time, sample_rate)` and `attack_coeff(time, sample_rate)`
- LFO phase increments calculated with actual sample rate
- Loop buffer sized for actual device rate

### Buffer Sizes
- **Audio Block**: ~128–512 samples (device-dependent)
- **Scope Buffer**: 512 samples (~10ms at 48kHz, enough for ~1 cycle of A2/110Hz)
- **UI Poll**: 16ms (60 Hz refresh)
- **Loop Buffer**: 8 seconds × device sample rate (~384,000 samples @ 48kHz)

### CPU Usage
CPU is typically dominated by terminal rendering, not DSP. Profile before assuming audio is the bottleneck.

**If profiling shows high CPU**: Use `cargo flamegraph` (Linux/macOS) or Instruments (macOS) to identify hot spots. Common culprits:
- Terminal I/O at 60Hz with full-screen diff updates
- Visualization calculations (especially complex effects like kaleidoscope, cube, donut)
- Unnecessary param swaps when UI-only state changes

### Optimization Techniques
- Minimal allocations in audio callback (pre-allocated buffers)
- Scope buffer: `ScopeBufferPool` with triple buffering — 3 pre-allocated Vecs rotate, bounded allocation
- Delay/reverb: ring buffers, wrap-around indexing

### Lock-Free Audio Thread (With Caveats)

Audio callback uses no **blocking** locks:
- Parameters loaded via `arc_swap` (atomic pointer swap)
- Commands popped from `rtrb` SPSC queue (lock-free)
- Reactive levels written via atomics (no contention)
- Command consumer mutex is audio-thread-only (never contested)
- Engine mutex uses `try_lock()` — non-blocking but causes brief silence during project save/load

See Threading Model section for detailed architecture.

---

## Error Handling

### Audio Errors
- **Xrun (underrun)**: Counter incremented, status logged
- **Device Lost**: Audio stream stops, user notified
- **Format Mismatch**: Fall back to supported format
- **MIDI Error**: Port closes gracefully, input disabled

### File I/O Errors
- **Missing Project**: Show error, revert to defaults
- **Write Failure**: Log to status bar, user can retry
- **Permission Denied**: Show error, suggest alternate location

### Panic Handling
Audio callback is written to avoid panics:
- Saturating arithmetic for level calculations
- Bounds-checked indexing on all buffers
- NaN/Inf guards on user-facing values
- Debug builds use `debug_assert!` for invariants
- Release builds rely on correctness, not `catch_unwind`

**Why not catch_unwind**: CPAL callbacks cross FFI boundaries (CoreAudio/ALSA/WASAPI). Unwinding across FFI is undefined behavior. Additionally, panic unwinding allocates (backtrace, message formatting) which is not real-time safe. The correct approach is defensive coding that cannot panic.

---

## Key Design Decisions

### 1. Single-Threaded UI, Audio on Separate Thread
- **Pro**: Responsive UI, real-time audio, clear separation
- **Con**: Requires careful thread synchronization
- **Status**: ✅ Correct approach

### 2. Lock-Free State Synchronization (Mostly Implemented)
- **Design**: UI owns `AppState`, `AudioBridge` provides lock-free sync
- **Mechanisms**: `arc_swap` for params, `rtrb` SPSC for commands, atomics for levels
- **Result**: Audio callback never **waits** on UI thread
- **Caveat**: Command consumer has mutex wrapper (audio-only, never contested). Engine mutex uses try_lock for export/import (causes brief silence during project save/load).
- **Status**: ✅ Practical lock-free — no priority inversion or deadlocks

### 3. Canvas Rendering (Build Full Screen, Diff Output)
- **Pro**: Decouples logic from terminal, easy to test
- **Con**: Memory for full screen buffer every frame
- **Status**: ✅ Good tradeoff

### 4. Soft Limiting (Tanh) Instead of Hard Clipping
- **Pro**: No audible clicks, natural saturation
- **Con**: Slight CPU cost
- **Status**: ✅ Audio quality priority

### 5. Loop Recording at Master Level, Not Per-Voice
- **Pro**: Simple UI, natural-sounding
- **Con**: Can't edit individual layers post-recording
- **Status**: ✅ Intentional constraint

### 6. Drum Sequencer on 32 Steps, 16th-Note Grid
- **Pro**: Fine-grained control, real-time feel
- **Con**: More keys to manage
- **Status**: ✅ Good balance

### 7. ✅ Dynamic Sample Rate (Implemented)
- **Design**: Sample rate read from CPAL device config at stream creation
- **Implementation**: All DSP coefficients use `dsp::decay_coeff(time, sample_rate)` etc.
- **Result**: Correct filter cutoffs, delay times, LFO rates on 44.1/48/96kHz devices
- **Status**: ✅ Production-ready

---

## Future Extensibility

### Possible Additions
1. **Step/Phrase Recording**: Record synth note sequences (not audio)
2. **Sampler**: Load and trigger samples (not just synth)
3. **Preset Management**: Save/load synth + drum + MIDI state bundles
4. **OSC Control**: Network-based parameter control
5. **Plugin Support**: VST/AU integration (complex, out of scope currently)
6. **Multiplayer**: Multi-client sequencing over network
7. **MIDI Clock Sync**: Sync drum sequencer to external clock (receive), send clock out
8. **MIDI Output**: Send note/CC data to external gear
9. **Tempo-Synced Loop**: Loop length in bars, auto-adjusts to BPM

### Architecture Readiness
- **Module Boundaries**: Clear (state/ vs audio.rs vs app.rs vs dsp.rs)
- **State Extension**: New `AppState` fields integrate easily
- **Audio Path**: New generators (sampler) fit after synth/drums
- **UI Path**: New settings pages / key bindings easily added
- **Concurrency**: Practical lock-free architecture in place — extending is safe (see Threading Model for caveats)

---

## Technical Debt Summary

### ✅ Resolved Issues

#### 1. Priority Inversion in Audio Thread — FIXED

**Was**: `Arc<Mutex<AppState>>` caused blocking in real-time audio callback, leading to xruns under load.

**Fix**: Implemented lock-free architecture:
- `ArcSwap<AudioParams>` for synth/drum/looper parameters
- `rtrb` SPSC queue for commands (note on/off, record start/stop)
- Atomics for reactive levels (master, synth, drums, kick, snare, hat)
- Audio callback has no blocking mutex operations. Command consumer uses a mutex wrapper that is only accessed from the audio thread (no contention). Engine uses `try_lock()` for export/import, which is non-blocking. See Threading Model for full caveats.

#### 2. Hardcoded Sample Rate — FIXED

**Was**: `const SAMPLE_RATE: f32 = 44_100.0` baked into DSP, causing wrong filter/delay/LFO/ADSR on 48kHz devices.

**Fix**: Dynamic sample rate from CPAL:
- `sample_rate: f32` stored on `AudioEngine` from `StreamConfig`
- All DSP coefficients use `dsp::decay_coeff(time, sample_rate)`, etc.
- Loop buffer sized for actual device rate

#### 3. DSP Correctness Issues — FIXED

**Was**: Multiple DSP bugs causing clicks and artifacts:
- Soft limiter had threshold discontinuity (conditional application)
- Envelope retrigger jumped from 0, not current value
- Saw/square oscillators had aliasing on high notes
- Reverb was comb-only (metallic sounding)
- Magic constants instead of sample-rate-aware math
- No denormal prevention in feedback paths

**Fix**: New `dsp.rs` module with proper implementations:
- Continuous soft limiter: `tanh(x * drive) / tanh(drive) * ceiling`
- `SmoothEnvelope`: continues from current value on retrigger
- PolyBLEP anti-aliasing for saw/square waveforms
- `SimpleReverb`: comb + allpass Freeverb-style network
- `decay_coeff()` / `attack_coeff()` with proper `exp(-1/(tau*sr))`
- `DENORMAL_PREVENTION = 1e-24` DC offset in feedback paths
- `MAX_FEEDBACK = 0.95` for delay (reverb combs may use higher values internally)

### Verification Checklist

- [x] No blocking mutex acquisitions in audio callback (try_lock only)
- [x] Lock-free param updates via `arc_swap`
- [x] Lock-free commands via `rtrb` SPSC queue (consumer mutex is audio-only, never contested)
- [x] Sample rate read from device config
- [x] Filter coefficients are sample-rate-aware
- [x] ADSR times scale correctly with sample rate
- [x] Continuous soft limiter (no threshold discontinuity)
- [x] Envelope retrigger continues from current value
- [x] Anti-aliased saw/square via PolyBLEP
- [x] Proper comb+allpass reverb
- [x] Denormal prevention in feedback paths
- [x] Feedback clamped to prevent oscillation
- [x] Filter cutoff smoothing (prevents zipper noise)
- [x] Gain smoothing (prevents zipper noise)
- [x] Undo stack bounded at 10 entries (~15MB max)
- [ ] Scope buffer uses true zero-allocation (currently rotates 3 small Vecs — acceptable but not optimal)

### Remaining Technical Debt

**High priority** (code cleanup):
- **Extract `SynthParams` / `DrumParams`**: Currently `SynthState` and `DrumState` are sent wholesale to audio, including UI-only fields like `active_osc`. Separating audio-relevant params into dedicated types would (a) type-enforce the boundary, (b) reduce unnecessary swaps on UI navigation.
- **Move `global_recording` to commands**: Starting/stopping WAV recording is a discrete event, not a continuous param. Better as `AudioCommand::StartRecording` / `StopRecording`. (Note: current polling approach works correctly via `was_recording` transition detection.)
- **True zero-allocation scope buffer**: `ScopeBufferPool` currently allocates Vecs when rotating buffers. Use `triple_buffer` crate or raw pointer manipulation for true zero-allocation.

**Live performance issue** (audible during use):
- **Engine mutex during export/import**: Audio callback uses `try_lock()` on engine during project save/load. This causes ~20-50ms of silence — audible and unacceptable for live performance. Potential fixes:
  1. **ArcSwap snapshot**: Export-relevant state (loop buffer) accessible via atomic snapshot, not by locking engine
  2. **UI-side copy**: Audio thread pushes loop buffer to UI via bridge when recording stops; UI serializes its own copy
  3. **Separate loop export**: Project save excludes loop buffer (instant, no audio impact); loop export is explicit user action

### ✅ Recently Fixed

- **Filter cutoff and gain smoothing**: Added `current_cutoff` and `current_gain` to audio engine with per-sample exponential smoothing (~5ms time constant). Fast knob sweeps no longer produce zipper noise.
- **Undo stack bounded**: `LoopState::begin_overdub()` now caps `undo_stack` at 10 entries (~15MB max), dropping oldest on push. Prevents unbounded memory growth in extended sessions.
- **File rename completed**: `audio_new.rs` → `audio.rs`, `runtime_new.rs` → `runtime.rs`. Old mutex-based files deleted.
- **Command channel split**: Producer and consumer are now owned separately (UI and audio threads respectively). `AudioBridge` no longer stores them. This removes cross-thread mutex contention.
- **Scope buffer pool**: `ScopeBufferPool` with triple buffering replaces unbounded ArcSwap<Vec> allocation. 3 pre-allocated buffers rotate.
- **Loop play_gain default**: Changed from 0.0 to 0.7. Old projects with saved 0.0 gain are fixed on load via `#[serde(default = "default_play_gain")]`. Runtime also checks and sets gain to 0.7 if below 0.1 after importing loop data.
- **Visual FX renamed and generalized**: `CameraReactiveStyle` → `VisualFx`, now applies to all framebuffer-based visuals (not just camera). Settings page renamed from "CameraFx" to "Visuals".
- **Context-aware Visuals settings**: Settings page now shows different options based on visual mode — Scope shows drums toggle, all others show FX Style + FX Depth.
- **Ripples spawn distribution**: Ripples effect now tracks actual framebuffer dimensions and spawns ripples across the full visual area, not just upper-left corner.

**Low priority** (does not affect audio quality or stability):
- **Loop is not tempo-synced**: Loop length is fixed at 8 seconds wall-clock time, regardless of drum tempo. For loops that align with the drum pattern, set tempo so N bars = 8 seconds (e.g., 4 bars at 120 BPM = 8s, 4 bars at 110 BPM = 8.73s which will clip). This is a deliberate simplification, not a bug.
- No persistent save/load for presets
- Audio device selection not exposed in UI (uses system default)
- Camera visual mode requires `ffmpeg` installed
- **MIDI output not implemented**: Input only. No note/CC output, no clock output.
- **MIDI clock sync not implemented**: Drum sequencer runs on internal clock. External sync is a common feature request — see Future Extensibility.

---

## Debugging Tips

### Audio Issues
- Check `state.audio.xruns` (underruns indicate CPU overload)
- Use `state.audio.status` for device/stream errors
- Monitor `reactive` levels to verify DSP output
- Reduce polyphony if CPU-bound

### UI Issues
- Check terminal size constraints (minimum ~80×24)
- Verify `canvas` not panicking on bounds checks
- Trace focus state and settings page transitions

### MIDI Issues
- Verify device is in `input_devices` list
- Check `midi.learn_mode` state during binding
- Inspect `bindings` for correct target parameters

### Performance
- Profile with `perf` (Linux) or Instruments (macOS)
- Check audio callback time with xrun counter
- Monitor heap allocations with `valgrind` or equivalent

---

## References

### Current Dependencies
- **CPAL** (audio I/O): https://github.com/RustAudio/cpal
- **Crossterm** (terminal UI): https://github.com/crossterm-rs/crossterm
- **Midir** (MIDI input): https://github.com/Boddlnagg/midir
- **Serde** (serialization): https://serde.rs/
- **Parking Lot** (fast mutex): https://docs.rs/parking_lot/ — UI-side only (project save, device enumeration)
- **arc-swap** (atomic Arc swapping): https://docs.rs/arc-swap/ — UI→Audio params
- **rtrb** (real-time safe SPSC ringbuffer): https://docs.rs/rtrb/ — UI→Audio commands

### Additional Lock-Free Resources (Not Currently Used)
- **triple_buffer** (wait-free triple buffering): https://docs.rs/triple_buffer/ — could replace ScopeBufferPool
- **ringbuf** (alternative SPSC queue): https://docs.rs/ringbuf/
- **atomic_float** (atomic f32/f64): https://docs.rs/atomic_float/

---

**Last Updated**: April 23, 2026
**Version**: Rust rewrite (mush-core + mush-cli) — practical lock-free architecture
