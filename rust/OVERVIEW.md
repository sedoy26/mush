# MUSH Rust — architecture overview

This document describes the **Rust** implementation (`mush-core`, `mush-cli`) as it exists in the repo today. It is written for **code review, debugging, and performance analysis**. The legacy **`mu.sh` / embedded Python** launcher is **not** covered here.

---

## 1. Workspace layout

```
rust/
├── Cargo.toml                 # workspace
├── Cargo.lock
├── OVERVIEW.md                # this file
└── crates/
    ├── mush-core/             # library: state, DSP, audio, runtime, project I/O, visuals
    │   └── src/
    │       ├── lib.rs
    │       ├── runtime.rs           # Runtime, project load, MIDI/audio/camera lifecycle
    │       ├── audio.rs             # AudioEngine, CPAL callback, mixing, per-bus FX (~1.6k LOC)
    │       ├── audio_bridge.rs      # AudioParams, AudioCommand, ReactiveState, scope pool
    │       ├── dsp.rs               # envelopes, reverb, soft limit, PolyBLEP, coeffs
    │       ├── camera.rs
    │       ├── input_capture.rs
    │       ├── project_io.rs        # .mush JSON, loop/sample WAV sidecars
    │       ├── state/               # AppState and sub-states (serde for projects)
    │       └── visuals/             # framebuffer visuals + registry
    └── mush-cli/                    # binary: crossterm UI, all keyboard/render (~3.6k LOC in main.rs)
        └── src/main.rs
```

---

## 2. Product behavior (session model)

**Startup (empty app or after `load_project`)** — `runtime.rs`:

- `apply_startup_session_policy` runs on `Runtime::new` (after `AppState::default()`) and again at the end of `load_project` (after WAV merge + `sanitize_app_state_after_load`).
- Effects: `ui.help_open = true`, `ui.settings_open = false`, `drums.running = false`, main **looper** and **sample performance loop** not playing/recording/overdubbing, `global_recording.recording = false`.
- **Intent:** user always dismisses help and explicitly starts the drum sequencer or loop playback. Saved project flags like `drums.running` or `looper.playing` are **not** honored across a full session entry (analysis: do not assume transport state round-trips from disk to sound without user action).

**Default UI** — `state/ui.rs`: `UiState::default()` sets `help_open: true` (matches policy for cold start).

---

## 3. `AppState` (single source of truth on UI thread)

Defined in `state/mod.rs`:

| Field | Role |
|--------|------|
| `synth` | Dual-osc monophonic synth + ADSR + LFO + optional LP filter + `FxState` (flattened) |
| `drums` | 8 patterns × 6 voices × 32 steps, BPM, bank, **chain** playback, per-voice volumes, **`FxState`** |
| `looper` | Main synth loop (metadata; buffer on audio thread) |
| `sample` | Mono sample buffer, trim/gain/speed/root, chromatic playback flags, **performance `LoopState`**, **`FxState`** |
| `midi` | Device, channel maps, learn, bindings |
| `audio` | Device selection, mirrored reactive levels, scope copies, xruns, global WAV path/status |
| `ui` | Theme, visual mode/FX, settings page, **`TabFocus`**, help/settings flags |
| `project` | Target name, file list, dirty flag |

**Threading:** UI thread holds `Arc<Mutex<AppState>>` (`Runtime::state`). The audio callback **does not** lock `AppState`; it reads `AudioParams` via `arc_swap` and commands via `rtrb`.

---

## 4. Lock-free bridge (`audio_bridge.rs`)

### 4.1 `AudioParams` (arc-swapped snapshot)

```text
synth:     SynthState      # full struct (includes UI-only fields — see §10)
drums:     DrumState
looper:    LooperParams    # subset of LoopState for main loop
sample:    SampleParams    # buffer Arc + trim/gain + sample_loop + fx
global_recording: bool     # when true, engine accumulates master out to WAV
```

**Analysis notes:**

- Any edit to nested fields causes the CLI to rebuild and swap a new `Arc<AudioParams>>` — including UI-only changes (e.g. `active_osc`). This is correct but can mean **high swap rate** while tweaking non-audio UI.
- `global_recording` is a bool sampled every block (not a queued command). Recording flush uses `AudioEngine` internal buffer + `flush_recording_if_needed` on the UI side.

### 4.2 `SampleParams`

Cheap clone each frame: `Arc<Vec<f32>>` for PCM, playback params, `LooperParams` for **sample-tab** loop, and **`fx: FxState`** for the sample bus.

`SampleNoteOn` can carry `sample_snapshot: Option<SampleParams>` so the audio thread arms a voice **after** applying the snapshot — avoids racing an empty buffer.

### 4.3 `AudioCommand` (SPSC `rtrb`)

Discrete events: main loop record/overdub/stop/clear/restore/gain/speed, `TriggerDrum`, sample note on/off, sample-loop record/play/clear/restore.

### 4.4 `ReactiveState` (atomics)

Peak-ish levels (`master`, `synth`, `drums`, per-voice triggers, `note`, `env`, …), loop **and** sample-loop length/positions/has_audio, **drum sequencer** `drum_step`, `chain_position`, `playing_pattern`, `xruns`.

CLI periodically copies these into `AppState.audio` for rendering without hammering atomics in draw code.

### 4.5 Scope buffers

`ScopeBufferPool`: triple-buffered `ArcSwap<Vec<f32>>` for master and synth-styled buses — audio writes, UI reads latest completed buffer.

---

## 5. Audio engine pipeline (`audio.rs`)

### 5.1 Order per callback block

1. Load `AudioParams` (lock-free).
2. Drain `AudioCommand`s (mutex on consumer **only inside callback** — no cross-thread contention; see code comments).
3. **`render_synth`** → dry oscillator + filter path (per `SynthState`).
4. **`render_loop`** → main looper: **replace/overdub capture** writes **`synth_out` only** (dry synth, not drums/sample); **playback** reads `loop_buf` and is summed with `synth_out` into the synth bus before FX (see `render_loop`).
5. **`render_sample`** → chromatic poly voices using `sample_params` + `SampleNoteOn`/`Off` state.
6. **`render_sample_loop`** → sample performance loop layered on live sample bus.
7. **`render_drums`** → pattern sequencer at BPM; bank synthesis.
8. **Per-bus FX** (`FxBusScratch` × 3): `wet_synth`, `wet_sample`, `wet_drums` — each has own delay line, warmth/air state, `SimpleReverb`.
9. Sum wet buses → **continuous** `dsp::soft_limit` → `OUTPUT_GAIN` → hard clamp `±0.98`.
10. `update_recording` if `global_recording`.
11. Push scope / reactive atomics.

### 5.2 Mix constants (tuning / analysis)

Current literals in `audio.rs`:

- `SYNTH_BUS_GAIN` / `SAMPLE_BUS_GAIN` ≈ **0.55** (after synth volume smoothing on synth+loop bus).
- `DRUM_BUS_GAIN` ≈ **0.28** (before drum FX).
- `LIMIT_DRIVE`, `LIMIT_CEILING`, `OUTPUT_GAIN` — see file; master curve is **not** a hard clip threshold.

### 5.3 Dynamic sample rate

CPAL device config sets `AudioEngine.sample_rate`; buffers (delay lines, loop buffers) resized/rebuilt in `set_sample_rate`.

### 5.4 `FxState` (`state/synth.rs`)

Shared **Copy** struct used on **synth**, **sample**, and **drums**: `drive`, `delay_mix`, `delay_feedback`, `delay_time`, `warmth`, `air`, `reverb`. Defaults via `FxState::bus_defaults()` (includes non-zero delay feedback/time defaults for sensible tail).

**Not per-bus:** the resonant **synth LP filter** (`filter_on`, `cutoff`, `resonance`, LFO→filter) lives only in `render_synth`. Sample and drum buses get **FX chain only**, not that multimode LP unless extended later.

### 5.5 Smoothing (zipper / clicks)

- **Synth master gain:** per-sample exponential toward `synth.volume` (~5 ms).
- **Synth filter cutoff:** smoothed cutoff coefficient (~5 ms) before one-pole LP.
- **FX chains:** independent state per bus prevents cross-bus delay/reverb bleed.

---

## 6. Synth (`state/synth.rs` + `render_synth`)

- **Monophonic** keyboard + MIDI; dual oscillators layered; `voices` is **unison spread**, not polyphony.
- **GateMode:** Trigger vs Hold.
- **LFO:** waveform, rate, depth, target (pitch / volume / filter).
- **FX:** flattened `FxState` on `SynthState` (serde aliases preserved for older keys).

---

## 7. Drums (`state/drums.rs` + `render_drums`)

- **6** voices, **32** steps, **8** patterns, **`NUM_DRUM_BANKS`** static banks (`get_bank`).
- **Chain:** `chain: Vec<usize>`, `chain_mode`, `chain_position` — UI/help call this “chain” (ordered pattern playback).
- **`FxState`** on `DrumState` for post-synthesis bus FX.
- `triggers`: session-only one-shots; cleared on save sanitize.

**Reactive:** `playing_pattern` / `chain_position` exposed for UI scope of chain state.

---

## 8. Loops (`state/looper.rs`)

- **Main loop:** `LoopState` in `AppState.looper` — WAV sidecar per project name; audio buffer in `AudioEngine`.
- **Sample loop:** `SampleState.performance_loop` — separate buffer path; same R/T/Y/P/U semantics on **Sample** tab in CLI.

Undo snapshots capped in `LoopState` implementation (see source).

---

## 9. Sample instrument (`state/sample.rs`)

- Mono buffer + trim + chromatic formula from `root_midi` + keyboard offsets.
- **Input record** flag is UI-only for capture workflow (sanitized on load/save).
- **`FxState`** persisted with project.

---

## 10. CLI (`mush-cli/src/main.rs`) — analysis hooks

- **Tabs:** `TabFocus::{Synth, Drums, Sample}` — changes QWERTY/MIDI routing and which **FX bus** Shift+DF… / WAE edits (`try_shift_fx_keys` targets `synth.fx`, `sample.fx`, or `drums.fx`).
- **Chromatic row:** `key_to_offset` lowercase; **Shift+** on reserved letters skips chromatic so FX keys work on terminals that send `Char('d')+Shift`.
- **Filter keys:** plain `-` / `=` → cutoff; `_` / `+` **or** `Shift+-` / `Shift+=` → resonance (Kitty-style shifted key reporting).
- **Help overlay:** built from structured rows; tagline string is CLI-specific (“Terminal based music production center” in help paint).
- **SONG panel:** pattern row, **CHAIN** row (was “list” in older UI), chain string, compact hints (`\ |`, `Del clr`, `\ chain`, etc.).
- **Settings:** paged (Main, Visuals, Project, Sound, MIDI, …) — see `settings_row_count` / `SettingsPage` in `state/ui.rs`.

---

## 11. Runtime operations (`runtime.rs`)

- **`sanitize_app_state_for_disk`:** clears held notes, key state, recording flags, drum triggers, clear_request flags before JSON write.
- **`sanitize_app_state_after_load`:** same class of ephemeral flags after deserialize + WAV import.
- **`load_project`:** merges JSON + optional loop WAV + sample WAV + sample-loop WAV; forces looper/sample-loop **playing false** where documented in code; then startup policy.
- **MIDI:** optional `midir` connection; sample notes can go through `SegQueue` to audio.
- **Camera:** `CameraRuntime` for visual modes that need ffmpeg.

---

## 12. Projects (`project_io.rs` + `state/project.rs`)

- JSON snapshot of `AppState` subset via `ProjectData` / `snapshot_project`.
- Large audio: **sidecar** `.wav` files for main loop, sample buffer, sample loop paths derived from project name and `base_dir`.

---

## 13. DSP module (`dsp.rs`)

Central place for: `SmoothEnvelope`, `SimpleReverb`, `soft_limit`, PolyBLEP saw/square, `attack_coeff` / `decay_coeff`, `DENORMAL_PREVENTION`, `MAX_FEEDBACK`. Unit tests live alongside.

---

## 14. Visuals (`visuals/`)

Framebuffer-based effects registered in `VisualRegistry`; params typed via `ParamValue`. Camera path uses `camera.rs` + optional ffmpeg. Visual **post-FX** (`VisualFx`) applied after base render — intensity from `visual_fx_depth`.

---

## 15. Known limitations & analysis checklist

| Area | Note |
|------|------|
| **Param snapshot size** | `SynthState` + `DrumState` full patterns in every `AudioParams` — fine for CPU, watch **allocation rate** on UI if every keypress rebuilds Arc. |
| **Polyphony** | Synth is mono; sample playback is **poly** with fixed max voices in engine. |
| **Filter scope** | Only synth path has multimode resonant LP + LFO routing; bus “tone” for sample/drums is **FxState**-driven. |
| **Transport persistence** | Session policy **overrides** playing/running on load — document for users and tests. |
| **xruns** | Incremented on stream error callback; surfaced in UI/reactive. |
| **Mutex on `AudioEngine`** | `try_lock` in callback; lock held for full render. UI uses same mutex for loop import/export — risk of glitch if UI holds lock long. |
| **Tests** | `mush-core` has DSP/visual tests; full audio path needs device for integration. |

---

## 16. Quick file → responsibility map

| File | Responsibility |
|------|----------------|
| `audio.rs` | CPAL stream, `AudioEngine`, mixing, FX buses, drums synth, sample voices, loops, recording WAV from master |
| `audio_bridge.rs` | `AudioParams`, commands, reactive atomics, scope pool, `bridge.update_params` |
| `runtime.rs` | Lifecycle, load/save hooks, sanitizers, **startup policy**, MIDI, camera |
| `state/*.rs` | Serde shapes + defaults for UI and projects |
| `mush-cli/main.rs` | Input routing by tab, render layout, help/settings overlays |

---

*Last aligned with crate sources in-repo (per-bus FX, chain UI, session/help policy, filter key behavior). Regenerate or diff against `git` when making large behavioral changes.*
