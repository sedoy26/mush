# AGENTS.md

## Project

This repository currently contains a single executable script: `synth.sh`.

`synth.sh` bootstraps and runs a terminal-based synthesizer called **MurSynth**. The shell script writes an embedded Python program to `$HOME/.mursynth.py`, ensures a virtualenv exists at `$HOME/.mursynth-venv`, installs `numpy` and `sounddevice` on first run, and then starts the interactive app.

The Python app provides:

- terminal UI with `curses`
- real-time synth audio via `sounddevice`
- simple drum sequencer
- oscilloscope/scope view
- synth FX and filter
- dual-oscillator layered synth playback
- simple audio loop capture / overdub workflow

## Current Progress

Recent work completed in `synth.sh`:

- reduced audible clicks by smoothing pitch and output transitions in the synth engine
- replaced hard clipping in main audio paths with soft limiting for smoother output
- increased audio block size and requested higher stream latency for more stable playback
- reused a shared noise RNG for drum synthesis instead of creating new generators repeatedly
- added a second oscillator with independent waveform, level, octave, and detune
- both oscillators now sound simultaneously, enabling layered bass/lead-style timbres
- added a small loop recorder with record, overdub, play, and clear controls
- updated UI/help/settings to expose oscillator editing and loop status
- added basic xrun counter visibility in the UI for debugging audio stability
- added 10 selectable vintage-style drum sound banks for the sequencer settings panel
- expanded the drum machine to 6 instruments and 32 steps
- added loop overdub undo for stepping back the last recorded layer
- added global mix recording that writes `.wav` files when stopped
- fixed the 32-step sequencer timing so 32 steps run at 16th-note spacing instead of doubled-speed 32nd-note spacing
- changed global mix recording to write numbered `untiteled*.wav` files in the launch directory without overwriting older takes
- added extra synth FX controls for warmth, air, and reverb in the settings panel
- added paged settings support for MIDI input device selection, per-note remapping, and MIDI learn for pad/knob bindings
- added an alternate camera visual mode that live-streams webcam frames via `ffmpeg` inside the synth view when selected in settings
- added a camera FX settings page with multiple audio-reactive webcam rendering styles driven by kick, drums, synth, and master level

## Controls Added Recently

- `1` / `2` select active oscillator for editing
- `z` / `x` change waveform of selected oscillator
- `R` start/stop loop record (replace)
- `T` overdub into existing loop
- `Y` undo the last overdub layer
- `P` toggle loop playback
- `U` clear loop
- `S` opens settings, where the drum sound bank can now be changed with arrow keys
- `G` start/stop global mix recording to a `.wav` file
- `S` settings now also expose warmth / air / reverb amounts with arrow keys
- `S` now opens paged settings for main synth options plus MIDI device, note map, and binding configuration
- main settings now also let the user switch visuals between oscilloscope and camera mode
- camera visuals can now be styled with multiple audio-reactive modes in settings

## Development Notes

- Keep changes focused and minimal; this is a single-file project at the moment.
- The Python source of truth is embedded inside `synth.sh`; do not edit `$HOME/.mursynth.py` directly.
- Prefer preserving the current structure unless there is a strong reason to split the embedded Python into separate files.
- If audio behavior changes, favor fixes at the DSP/state-transition level rather than masking issues in the UI.
- Be careful with anything that can introduce discontinuities between audio blocks; clicks usually come from abrupt state jumps, clipping, xruns, or loop boundary discontinuities.
- Validate shell syntax with `bash -n synth.sh` after edits.
- Validate embedded Python syntax by extracting the heredoc or otherwise checking that the generated Python compiles.
- Avoid adding network-dependent setup changes unless explicitly needed.
- MIDI support may require Python MIDI dependencies; keep any bootstrap install changes minimal and focused.

## Known Technical Areas To Improve

- loop capture is intentionally simple; it is not tempo-synced and does not quantize boundaries
- current synth is still effectively one played note with layered oscillators, not true multitimbral bass+lead on separate note lanes
- there is no persistent save/load for presets, drum patterns, or loops
- xrun/debug reporting is minimal and could be expanded
- audio device selection is not exposed in the UI
- the single-file embedded-Python approach is convenient but harder to maintain as features grow

## Future Development Ideas

- add true two-part performance mode with separate note mappings for bass and lead
- add step or phrase recording for synth notes, not only audio looping
- quantized looper start/stop and BPM-synced loop lengths
- per-oscillator ADSR and pan
- better anti-click handling for loop boundaries and parameter changes
- compressor/limiter or master output headroom controls
- presets and pattern storage
- export captured loops to `.wav`
- optional refactor: move embedded Python into tracked source files while keeping `synth.sh` as launcher

## Notes For Future Agents

- If you touch `synth.sh`, remember that the visible shell file contains the real application logic inside the heredoc.
- When documenting controls, update both the on-screen help and this file if the controls materially change.
- Do not assume the environment has audio output available during automated validation.
- Interactive runtime testing should be described clearly to the user when full audio verification is not possible in the sandbox.
