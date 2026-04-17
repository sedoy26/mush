# mush

`mush` is a terminal synthesizer and drum machine launched by `mu.sh`.

It runs as a single shell script that writes an embedded Python app to your home directory, creates a local virtualenv on first run, installs the required Python packages, and starts an interactive curses-based instrument.

## Features

- dual-oscillator synth with waveform, level, octave, and detune control
- realtime audio output with delay, warmth, air, reverb, filter, and limiting
- 6-voice, 32-step drum sequencer with multiple drum sound banks
- audio looper with record, overdub, undo, play, and clear
- global stereo WAV recording
- oscilloscope and camera-based visual modes
- MIDI device input, note remapping, and learnable pad/knob bindings
- audio output selection and stored audio input preference for future use
- project save/load for synth, drum, UI, audio, and MIDI settings

## Files and folders

- `mu.sh` — launcher and embedded app source of truth
- `wav/` — saved mix recordings such as `untiteled.wav`
- `projects/` — saved project files using the `.mush` extension
- `README.md` — project overview and usage notes

Generated files in `wav/` and `projects/` are gitignored by default, except for `wav/demo.wav` and `projects/demo.mush`.

## Running

```bash
./mu.sh
```

On first launch, the script creates:

- `$HOME/.mush-venv` for Python dependencies
- `$HOME/.mush.py` for the generated runtime script

The app expects a working terminal and an available audio output device. Camera mode also expects `ffmpeg` with camera capture support.

## Main controls

- `TAB` — switch between synth focus and drum focus
- `S` — open settings
- `H` — show help
- `q` — quit

### Synth

- `a w s e d f t g y h u j k` — play notes
- `SPACE` — release held note
- `1` / `2` — select oscillator
- `z` / `x` — change selected oscillator waveform
- `↑` / `↓` — master volume
- `←` / `→` — root note
- `[` / `]` — attack
- `{` / `}` — release
- `m` — gate/free mode
- `p` — filter on/off
- `-` / `=` — cutoff
- `_` / `+` — resonance
- `l` — LFO wave
- `o` — LFO target
- `,` / `.` — LFO rate
- `;` / `'` — LFO depth
- `D` / `F` — drive
- `J` / `K` — delay mix
- `N` / `M` — delay feedback
- `V` / `B` — delay time

### Loop and recording

- `R` — record loop (replace)
- `T` — overdub loop
- `Y` — undo last overdub layer
- `P` — toggle loop playback
- `U` — clear loop
- `G` — start/stop global WAV recording

### Drums

- `TAB` into drum focus first
- `←` / `→` / `↑` / `↓` — move sequencer cursor
- `SPACE` — toggle current step
- `r` — start/stop sequencer
- `c` — clear current row with confirmation
- `X` — clear all rows with confirmation
- `1` / `2` — load built-in patterns
- `,` / `.` / `<` / `>` — BPM adjustments
- `-` / `=` — selected drum voice level

## Settings pages

The settings UI currently includes:

- `MAIN` — synth, visuals, drum bank, oscillator, and FX settings
- `CAM FX` — camera visual style and reactivity
- `SOUND DEVICE` — audio output selection and stored input preference
- `PROJECT` — save/load `.mush` project files
- `MIDI DEVICE` — MIDI input selection and routing
- `MIDI NOTE` — note remapping
- `MIDI MAP` — learnable pad/knob bindings

## Project saves

Project files store the current working setup, including:

- synth parameters and oscillator settings
- drum pattern, BPM, bank, and per-voice levels
- UI visual settings
- selected audio input/output names
- MIDI device settings, remaps, and bindings

They do not currently store recorded loop audio.

## Validation

When editing `mu.sh`, useful checks are:

```bash
bash -n mu.sh
python3 - <<'PY'
from pathlib import Path
text = Path('mu.sh').read_text()
start = text.index("<< 'PYEOF'\n") + len("<< 'PYEOF'\n")
end = text.index("\nPYEOF", start)
compile(text[start:end], 'embedded_app.py', 'exec')
print('ok')
PY
```

## Notes

- audio input selection is saved for future expansion, but the current app does not process live input yet
- the looper is free-running and not tempo-quantized
- the whole app currently lives inside the `mu.sh` heredoc for convenience
